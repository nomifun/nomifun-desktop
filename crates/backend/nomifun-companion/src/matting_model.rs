//! Backend-owned persistent MODNet supply. The shared manifest pins the exact
//! artifact; bytes are verified before serving or atomically publishing them.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nomifun_common::AppError;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock, watch};

/// Host-owned admission and join boundary for this model supply only. The
/// existing model lock still serializes acquisition; this guard also covers
/// callers waiting for that lock and the original cache readers/publisher.
pub(crate) struct ModelLifecycle {
    closing: watch::Sender<bool>,
    active: Arc<RwLock<()>>,
}

type ModelActivity = Arc<OwnedRwLockReadGuard<()>>;

impl Default for ModelLifecycle {
    fn default() -> Self {
        Self { closing: watch::channel(false).0, active: Arc::new(RwLock::new(())) }
    }
}

fn shutdown_error() -> AppError {
    AppError::Conflict("Companion matting model supply is shutting down".into())
}

impl ModelLifecycle {
    pub(crate) fn request_shutdown(&self) {
        self.closing.send_replace(true);
    }

    pub(crate) async fn shutdown(&self) {
        self.request_shutdown();
        let _quiescent = self.active.write().await;
    }

    fn ensure_open(&self) -> Result<(), AppError> {
        if *self.closing.borrow() { Err(shutdown_error()) } else { Ok(()) }
    }

    async fn cancelled(&self) {
        let mut closing = self.closing.subscribe();
        while !*closing.borrow_and_update() {
            if closing.changed().await.is_err() { break; }
        }
    }

    async fn admit(&self) -> Result<ModelActivity, AppError> {
        self.ensure_open()?;
        let active = tokio::select! {
            biased;
            _ = self.cancelled() => return Err(shutdown_error()),
            active = self.active.clone().read_owned() => active,
        };
        self.ensure_open()?;
        Ok(Arc::new(active))
    }

    async fn acquisition_lock<'a>(&self, lock: &'a Mutex<()>) -> Result<tokio::sync::MutexGuard<'a, ()>, AppError> {
        let guard = tokio::select! {
            biased;
            _ = self.cancelled() => return Err(shutdown_error()),
            guard = lock.lock() => guard,
        };
        self.ensure_open()?;
        Ok(guard)
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ModelManifest {
    id: String,
    filename: String,
    size: u64,
    sha256: String,
    urls: Vec<String>,
}

fn manifest() -> &'static ModelManifest {
    static MANIFEST: OnceLock<ModelManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("../matting-model.json"))
            .expect("the bundled matting model manifest must be valid")
    })
}

/// The route checks the requested identity before any disk read or download.
pub fn approved_sha256() -> &'static str {
    &manifest().sha256
}

#[derive(Clone, Copy)]
struct DownloadTimeouts {
    connect: Duration,
    headers: Duration,
    read_idle: Duration,
}

const DOWNLOAD_TIMEOUTS: DownloadTimeouts = DownloadTimeouts {
    connect: Duration::from_secs(15),
    headers: Duration::from_secs(30),
    read_idle: Duration::from_secs(30),
};

/// Return verified bytes from the persistent cache, acquiring them if absent
/// or corrupt. Existing `modnet.onnx` files are reusable only after exact size
/// and SHA256 verification. The returned buffer is the one that was verified,
/// avoiding a second file read racing a replacement. Concurrent callers share
/// one successful acquisition through the lock and subsequent cache recheck.
pub(crate) async fn model_bytes(
    models_dir: &Path, lock: &Mutex<()>, lifecycle: &ModelLifecycle,
) -> Result<Vec<u8>, AppError> {
    model_bytes_from_upstreams(
        models_dir, lock, lifecycle, manifest(), reqwest::Client::builder(), DOWNLOAD_TIMEOUTS,
    ).await
}

async fn model_bytes_from_upstreams(
    models_dir: &Path,
    lock: &Mutex<()>,
    lifecycle: &ModelLifecycle,
    model: &ModelManifest,
    client_builder: reqwest::ClientBuilder,
    timeouts: DownloadTimeouts,
) -> Result<Vec<u8>, AppError> {
    let activity = lifecycle.admit().await?;
    let path = models_dir.join(&model.filename);
    if let Some(bytes) = read_verified_cache(&path, model, lifecycle, activity.clone()).await? {
        lifecycle.ensure_open()?;
        return Ok(bytes);
    }
    let _guard = lifecycle.acquisition_lock(lock).await?;
    if let Some(bytes) = read_verified_cache(&path, model, lifecycle, activity.clone()).await? {
        lifecycle.ensure_open()?;
        return Ok(bytes);
    }
    let client = client_builder.connect_timeout(timeouts.connect).build()
        .map_err(|error| AppError::Internal(format!("build model http client: {error}")))?;
    let mut errors = Vec::new();
    for url in &model.urls {
        let downloaded = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(shutdown_error()),
            result = download_one(&client, url, model, timeouts) => result,
        };
        // Shutdown is a host cancellation, never an upstream failure authorizing
        // fallback. A verified acquisition may enter publication only while open.
        lifecycle.ensure_open()?;
        match downloaded {
            Ok(bytes) => {
                write_atomic(&path, &bytes, lifecycle, activity.clone()).await?;
                tracing::info!(model = %model.id, url, bytes = bytes.len(), "verified matting model cached");
                return Ok(bytes);
            }
            Err(error) => {
                tracing::warn!(model = %model.id, url, %error, "matting model upstream failed; trying next");
                errors.push(format!("{url}: {error}"));
            }
        }
    }
    Err(AppError::Internal(format!(
        "无法获取抠图模型(所有上游均失败): {}", errors.join("; ")
    )))
}

async fn read_verified_cache(
    path: &Path, model: &ModelManifest, lifecycle: &ModelLifecycle, activity: ModelActivity,
) -> Result<Option<Vec<u8>>, AppError> {
    let path = path.to_path_buf();
    let model = model.clone();
    join_owned_io(lifecycle, activity, move || {
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(AppError::Internal(format!("open matting model cache: {error}"))),
        };
        read_open_verified_cache(file, &model)
    }).await
}

fn read_open_verified_cache(file: std::fs::File, model: &ModelManifest) -> Result<Option<Vec<u8>>, AppError> {
    let meta = file.metadata()
        .map_err(|error| AppError::Internal(format!("stat matting model cache: {error}")))?;
    if !meta.is_file() || meta.len() != model.size {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(model.size as usize);
    // Bound the read even if another process appends to the opened file.
    file.take(model.size + 1).read_to_end(&mut bytes)
        .map_err(|error| AppError::Internal(format!("read matting model cache: {error}")))?;
    if bytes.len() as u64 != model.size || hex::encode(Sha256::digest(&bytes)) != model.sha256 {
        tracing::warn!(model = %model.id, "matting model cache failed integrity verification; reacquiring");
        return Ok(None);
    }
    Ok(Some(bytes))
}

async fn download_one(
    client: &reqwest::Client,
    url: &str,
    model: &ModelManifest,
    timeouts: DownloadTimeouts,
) -> Result<Vec<u8>, String> {
    let mut response = tokio::time::timeout(timeouts.headers, client.get(url).send())
        .await.map_err(|_| "response headers timed out".to_owned())?
        .map_err(|error| error.without_url().to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    if let Some(size) = response.content_length()
        && size != model.size
    {
        return Err(format!("model size mismatch: expected {}, declared {size}", model.size));
    }
    let mut bytes = Vec::with_capacity(model.size as usize);
    let mut digest = Sha256::new();
    loop {
        // Every chunk resets the idle deadline. Continuously progressing slow
        // transfers have no total deadline; stalled sources permit fallback.
        let chunk = tokio::time::timeout(timeouts.read_idle, response.chunk())
            .await.map_err(|_| "model download read idle timed out".to_owned())?
            .map_err(|error| error.without_url().to_string())?;
        let Some(chunk) = chunk else { break; };
        if chunk.len() as u64 > model.size.saturating_sub(bytes.len() as u64) {
            return Err(format!("model exceeds approved size {}", model.size));
        }
        digest.update(&chunk);
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() as u64 != model.size {
        return Err(format!("model size mismatch: expected {}, received {}", model.size, bytes.len()));
    }
    if hex::encode(digest.finalize()) != model.sha256 {
        return Err("model SHA256 mismatch".into());
    }
    Ok(bytes)
}

/// Invalid downloads cannot replace the old cache or expose partial bytes.
async fn write_atomic(
    path: &Path, bytes: &[u8], lifecycle: &ModelLifecycle, activity: ModelActivity,
) -> Result<(), AppError> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    join_owned_io(lifecycle, activity, move || publish_atomic(&path, &bytes)).await
}

async fn join_owned_io<T: Send + 'static>(
    lifecycle: &ModelLifecycle,
    activity: ModelActivity,
    operation: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let task = {
        // This brief token read serializes cache I/O registration with the
        // synchronous shutdown fence; it never spans an await or native I/O.
        let closing = lifecycle.closing.borrow();
        if *closing { return Err(shutdown_error()); }
        tokio::task::spawn_blocking(move || {
            // Keep the original activity guard in the native operation as well as
            // its async caller. Losing an HTTP waiter cannot release the join
            // boundary while this reader/writer still owns its original file.
            let _activity = activity;
            operation()
        })
    };
    task.await.map_err(|error| AppError::Internal(format!("model cache I/O task failed: {error}")))?
}

fn publish_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path.parent()
        .ok_or_else(|| AppError::Internal("matting model path has no parent".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| AppError::Internal(format!("create models dir: {error}")))?;
    let mut temp = tempfile::Builder::new().prefix(".modnet.onnx.tmp.").tempfile_in(parent)
        .map_err(|error| AppError::Internal(format!("create model tempfile: {error}")))?;
    temp.write_all(bytes).and_then(|_| temp.as_file_mut().sync_all())
        .map_err(|error| AppError::Internal(format!("write model tempfile: {error}")))?;
    temp.persist(path)
        .map_err(|error| AppError::Internal(format!("commit model: {}", error.error)))?;
    crate::fsio::sync_dir(parent)
        .map_err(|error| AppError::Internal(format!("fsync models dir: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    const TEST_TIMEOUTS: DownloadTimeouts = DownloadTimeouts {
        connect: Duration::from_secs(1),
        headers: Duration::from_millis(150),
        read_idle: Duration::from_millis(150),
    };

    struct Reply {
        status: u16,
        declared_size: Option<usize>,
        headers_delay: Duration,
        chunks: Vec<(Duration, Vec<u8>)>,
    }

    impl Reply {
        fn ok(bytes: &[u8]) -> Self {
            Self {
                status: 200, declared_size: Some(bytes.len()), headers_delay: Duration::ZERO,
                chunks: vec![(Duration::ZERO, bytes.to_vec())],
            }
        }
    }

    async fn serve(replies: Vec<Reply>) -> (Vec<String>, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let urls = (0..replies.len()).map(|index| format!("http://{address}/{index}")).collect();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for (index, reply) in replies.into_iter().enumerate() {
                let (stream, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                handlers.push(tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    assert_eq!(line.trim_end(), format!("GET /{index} HTTP/1.1"));
                    loop {
                        line.clear();
                        assert!(reader.read_line(&mut line).await.unwrap() > 0);
                        if line == "\r\n" { break; }
                    }
                    tokio::time::sleep(reply.headers_delay).await;
                    let length = reply.declared_size.map(|size| format!("Content-Length: {size}\r\n"))
                        .unwrap_or_default();
                    let headers = format!("HTTP/1.1 {} fixture\r\n{length}Connection: close\r\n\r\n", reply.status);
                    if reader.get_mut().write_all(headers.as_bytes()).await.is_err() { return; }
                    for (delay, bytes) in reply.chunks {
                        tokio::time::sleep(delay).await;
                        if reader.get_mut().write_all(&bytes).await.is_err() { return; }
                    }
                    let _ = reader.get_mut().shutdown().await;
                }));
            }
            for handler in handlers { handler.await.unwrap(); }
        });
        (urls, requests, server)
    }

    fn fixture(bytes: &[u8], urls: Vec<String>) -> ModelManifest {
        ModelManifest {
            id: "verified-test-model".into(), filename: "modnet.onnx".into(),
            size: bytes.len() as u64, sha256: hex::encode(Sha256::digest(bytes)), urls,
        }
    }

    async fn acquire(dir: &Path, lock: &Mutex<()>, model: &ModelManifest) -> Result<Vec<u8>, AppError> {
        tokio::time::timeout(Duration::from_secs(5), model_bytes_from_upstreams(
            dir, lock, &ModelLifecycle::default(), model, reqwest::Client::builder().no_proxy(), TEST_TIMEOUTS,
        )).await.expect("model acquisition must settle")
    }

    async fn finished(server: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
    }

    #[test]
    fn bundled_manifest_pins_the_approved_model() {
        let model = manifest();
        assert_eq!(model.id, "modnet-fp32-07c308cf0fc7");
        assert_eq!(model.filename, "modnet.onnx");
        assert_eq!(model.size, 25_888_640);
        assert_eq!(model.sha256, "07c308cf0fc7e6e8b2065a12ed7fc07e1de8febb7dc7839d7b7f15dd66584df9");
        assert_eq!(model.urls.len(), 2);
        for url in &model.urls {
            assert!(url.contains("/resolve/fa2fa546052fba4c08921230a26cc69a333fca12/onnx/model.onnx"));
            assert!(url.starts_with("https://"));
        }
    }

    #[tokio::test]
    async fn verified_existing_cache_needs_no_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let model = fixture(bytes, Vec::new());
        std::fs::write(dir.path().join(&model.filename), bytes).unwrap();
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
    }

    #[tokio::test]
    async fn same_length_corrupt_cache_is_reacquired() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let (urls, requests, server) = serve(vec![Reply::ok(bytes)]).await;
        let model = fixture(bytes, urls);
        let path = dir.path().join(&model.filename);
        std::fs::write(&path, vec![0; bytes.len()]).unwrap();
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        finished(server).await;
    }

    #[tokio::test]
    async fn incorrect_download_digest_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let (urls, requests, server) = serve(vec![Reply::ok(&vec![0; bytes.len()]), Reply::ok(bytes)]).await;
        let model = fixture(bytes, urls);
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert_eq!(std::fs::read(dir.path().join(&model.filename)).unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        finished(server).await;
    }

    #[tokio::test]
    async fn truncated_declared_and_unframed_transfers_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let mut truncated = Reply::ok(&bytes[..5]);
        truncated.declared_size = Some(bytes.len());
        let mut unframed = Reply::ok(&bytes[..5]);
        unframed.declared_size = None;
        let (urls, requests, server) = serve(vec![truncated, unframed, Reply::ok(bytes)]).await;
        let model = fixture(bytes, urls);
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        finished(server).await;
    }

    #[tokio::test]
    async fn oversized_declared_and_unframed_transfers_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let oversized = [bytes.as_slice(), b"extra"].concat();
        let mut unframed = Reply::ok(&oversized);
        unframed.declared_size = None;
        let (urls, requests, server) = serve(vec![Reply::ok(&oversized), unframed, Reply::ok(bytes)]).await;
        let model = fixture(bytes, urls);
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        finished(server).await;
    }

    #[tokio::test]
    async fn response_headers_and_body_idle_timeouts_allow_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let mut headers_idle = Reply::ok(bytes);
        headers_idle.headers_delay = Duration::from_millis(450);
        let mut body_idle = Reply::ok(bytes);
        body_idle.chunks = vec![
            (Duration::ZERO, bytes[..5].to_vec()),
            (Duration::from_millis(450), bytes[5..].to_vec()),
        ];
        let (urls, requests, server) = serve(vec![headers_idle, body_idle, Reply::ok(bytes)]).await;
        let model = fixture(bytes, urls);
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 3);
        finished(server).await;
    }

    #[tokio::test]
    async fn continuously_progressing_download_can_exceed_idle_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let mut slow = Reply::ok(bytes);
        slow.chunks = bytes.chunks(3).map(|chunk| (Duration::from_millis(50), chunk.to_vec())).collect();
        let (urls, _, server) = serve(vec![slow]).await;
        let model = fixture(bytes, urls);
        let started = std::time::Instant::now();
        assert_eq!(acquire(dir.path(), &Mutex::new(()), &model).await.unwrap(), bytes);
        assert!(started.elapsed() > TEST_TIMEOUTS.read_idle);
        finished(server).await;
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_successful_download() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let mut reply = Reply::ok(bytes);
        reply.headers_delay = Duration::from_millis(50);
        let (urls, requests, server) = serve(vec![reply]).await;
        let model = fixture(bytes, urls);
        let lock = Mutex::new(());
        let results = tokio::join!(
            acquire(dir.path(), &lock, &model), acquire(dir.path(), &lock, &model),
            acquire(dir.path(), &lock, &model), acquire(dir.path(), &lock, &model),
        );
        for result in [results.0, results.1, results.2, results.3] {
            assert_eq!(result.unwrap(), bytes);
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        finished(server).await;
    }

    #[tokio::test]
    async fn failed_upstreams_preserve_cache_and_next_request_can_retry() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let corrupt = b"corrupt cache";
        let (urls, requests, server) = serve(vec![Reply::ok(&vec![0; bytes.len()]), Reply::ok(bytes)]).await;
        let mut model = fixture(bytes, urls);
        let fallback = model.urls.pop().unwrap();
        let path = dir.path().join(&model.filename);
        std::fs::write(&path, corrupt).unwrap();
        let lock = Mutex::new(());
        let error = acquire(dir.path(), &lock, &model).await.unwrap_err();
        assert!(error.to_string().contains("SHA256 mismatch"));
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no partial cache is published");
        model.urls = vec![fallback];
        assert_eq!(acquire(dir.path(), &lock, &model).await.unwrap(), bytes);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        finished(server).await;
    }

    async fn blocked_upstream(
        body_started: bool, size: usize,
    ) -> (Vec<String>, tokio::sync::oneshot::Receiver<()>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let urls = vec![format!("http://{address}/primary"), format!("http://{address}/fallback")];
        let (entered, observed) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(line.trim_end(), "GET /primary HTTP/1.1");
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).await.unwrap() > 0);
                if line == "\r\n" { break; }
            }
            if body_started {
                reader.get_mut().write_all(format!(
                    "HTTP/1.1 200 fixture\r\nContent-Length: {size}\r\nConnection: close\r\n\r\na"
                ).as_bytes()).await.unwrap();
            }
            entered.send(()).unwrap();
            // Cancelling the original request must release its transport; the
            // server never sends the missing headers/body or an idle timeout.
            let mut rest = Vec::new();
            let eof = tokio::time::timeout(Duration::from_secs(3), reader.read_to_end(&mut rest)).await
                .expect("cancelled model request must close the original connection");
            if let Err(error) = eof {
                assert!(matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted));
            }
            assert!(rest.is_empty(), "shutdown must not send a fallback request on the original connection");
            let listener = listener.into_std().unwrap();
            assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
                "shutdown must not open the fallback upstream");
        });
        (urls, observed, server)
    }

    async fn cancels_blocked_download_without_fallback(body_started: bool) {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let (urls, observed, server) = blocked_upstream(body_started, bytes.len()).await;
        let model = Arc::new(fixture(bytes, urls));
        let lifecycle = Arc::new(ModelLifecycle::default());
        let lock = Arc::new(Mutex::new(()));
        let request_dir = dir.path().to_path_buf();
        let request_lifecycle = lifecycle.clone();
        let request_model = model.clone();
        let request = tokio::spawn(async move {
            model_bytes_from_upstreams(&request_dir, &lock, &request_lifecycle, &request_model,
                reqwest::Client::builder().no_proxy(), DOWNLOAD_TIMEOUTS).await
        });
        tokio::time::timeout(Duration::from_secs(3), observed).await.unwrap().unwrap();
        lifecycle.request_shutdown();
        let error = tokio::time::timeout(Duration::from_secs(3), request).await.unwrap().unwrap().unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref detail) if detail.contains("shutting down")));
        tokio::time::timeout(Duration::from_secs(3), lifecycle.shutdown()).await.unwrap();
        finished(server).await;
        assert!(!dir.path().join(&model.filename).exists(), "cancelled acquisition cannot publish partial cache");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn shutdown_cancels_response_headers_without_fallback() {
        cancels_blocked_download_without_fallback(false).await;
    }

    #[tokio::test]
    async fn shutdown_cancels_body_without_fallback() {
        cancels_blocked_download_without_fallback(true).await;
    }

    #[tokio::test]
    async fn shutdown_withdraws_model_lock_waiter_before_current_acquisition_finishes() {
        let lifecycle = ModelLifecycle::default();
        let lock = Mutex::new(());
        let current = lock.lock().await;
        let activity = lifecycle.admit().await.unwrap();
        let mut queued = Box::pin(lifecycle.acquisition_lock(&lock));
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(queued.as_mut(), context).is_pending(),
        )).await);
        lifecycle.request_shutdown();
        let error = tokio::time::timeout(Duration::from_secs(1), queued).await.unwrap().unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref detail) if detail.contains("shutting down")));
        assert!(lock.try_lock().is_err(), "the first acquisition is still owned, not forcibly unlocked");
        drop(activity);
        tokio::time::timeout(Duration::from_secs(1), lifecycle.shutdown()).await.unwrap();
        drop(current);
    }

    #[tokio::test]
    async fn shutdown_rejects_even_verified_cache_and_repeated_shutdown_is_safe() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let model = fixture(bytes, Vec::new());
        let path = dir.path().join(&model.filename);
        std::fs::write(&path, bytes).unwrap();
        let lifecycle = ModelLifecycle::default();
        lifecycle.request_shutdown();
        lifecycle.shutdown().await;
        lifecycle.shutdown().await;
        let error = model_bytes_from_upstreams(dir.path(), &Mutex::new(()), &lifecycle, &model,
            reqwest::Client::builder().no_proxy(), TEST_TIMEOUTS).await.unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref detail) if detail.contains("shutting down")));
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[tokio::test]
    async fn shutdown_joins_original_atomic_publication_after_its_waiter_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("modnet.onnx");
        std::fs::write(&path, b"previous cache").unwrap();
        let lifecycle = Arc::new(ModelLifecycle::default());
        let activity = lifecycle.admit().await.unwrap();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let publish_path = path.clone();
        let publishing_lifecycle = lifecycle.clone();
        let publisher = tokio::spawn(async move {
            join_owned_io(&publishing_lifecycle, activity, move || {
                started.send(()).unwrap();
                gate.recv().unwrap();
                publish_atomic(&publish_path, b"verified replacement")
            }).await
        });
        tokio::time::timeout(Duration::from_secs(3), observed).await.unwrap().unwrap();
        lifecycle.request_shutdown();
        let mut shutdown = Box::pin(lifecycle.shutdown());
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(shutdown.as_mut(), context).is_pending(),
        )).await);
        publisher.abort();
        assert!(publisher.await.unwrap_err().is_cancelled());
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(shutdown.as_mut(), context).is_pending(),
        )).await, "dropping the HTTP waiter cannot release the native writer's activity guard");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous cache");
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), shutdown).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"verified replacement");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "publication leaves no temporary cache");
    }

    #[tokio::test]
    async fn shutdown_joins_open_cache_reader_after_its_waiter_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let model = fixture(bytes, Vec::new());
        let path = dir.path().join(&model.filename);
        std::fs::write(&path, bytes).unwrap();
        let lifecycle = Arc::new(ModelLifecycle::default());
        let activity = lifecycle.admit().await.unwrap();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let (completed, result) = tokio::sync::oneshot::channel();
        let reading_lifecycle = lifecycle.clone();
        let reader = tokio::spawn(async move {
            join_owned_io(&reading_lifecycle, activity, move || {
                // Capture the real opened object before the barrier. Metadata,
                // bounded reading, digest verification and FD release remain
                // this same owned operation even if its HTTP waiter disappears.
                let file = std::fs::File::open(&path).unwrap();
                assert_eq!(file.metadata().unwrap().len(), model.size);
                started.send(()).unwrap();
                gate.recv().unwrap();
                let read = read_open_verified_cache(file, &model);
                completed.send(read.as_ref().unwrap().as_ref().unwrap().clone()).unwrap();
                read
            }).await
        });
        tokio::time::timeout(Duration::from_secs(3), observed).await.unwrap().unwrap();
        lifecycle.request_shutdown();
        let mut shutdown = Box::pin(lifecycle.shutdown());
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(shutdown.as_mut(), context).is_pending(),
        )).await);
        reader.abort();
        assert!(reader.await.unwrap_err().is_cancelled());
        assert!(std::future::poll_fn(|context| std::task::Poll::Ready(
            std::future::Future::poll(shutdown.as_mut(), context).is_pending(),
        )).await, "dropping the HTTP waiter cannot release the open cache reader's activity guard");
        release.send(()).unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(3), result).await.unwrap().unwrap(), bytes);
        tokio::time::timeout(Duration::from_secs(3), shutdown).await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn shutdown_rejects_new_cache_io_even_with_an_old_activity_guard() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"approved model fixture";
        let model = fixture(bytes, Vec::new());
        let path = dir.path().join(&model.filename);
        std::fs::write(&path, bytes).unwrap();
        let lifecycle = ModelLifecycle::default();
        let activity = lifecycle.admit().await.unwrap();
        lifecycle.request_shutdown();
        let error = read_verified_cache(&path, &model, &lifecycle, activity.clone()).await.unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref detail) if detail.contains("shutting down")));
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let error = join_owned_io(&lifecycle, activity, move || {
            observed.fetch_add(1, Ordering::SeqCst);
            let file = std::fs::File::open(&path).unwrap();
            read_open_verified_cache(file, &model)
        }).await.unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref detail) if detail.contains("shutting down")));
        assert_eq!(attempts.load(Ordering::SeqCst), 0, "the closed owner cannot register another cache read");
        tokio::time::timeout(Duration::from_secs(1), lifecycle.shutdown()).await.unwrap();
    }
}
