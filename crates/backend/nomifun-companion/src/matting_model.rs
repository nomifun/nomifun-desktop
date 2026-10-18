//! Backend-owned persistent MODNet supply. The shared manifest pins the exact
//! artifact; bytes are verified before serving or atomically publishing them.

use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use nomifun_common::AppError;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

#[derive(Debug, Deserialize)]
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
pub async fn model_bytes(models_dir: &Path, lock: &Mutex<()>) -> Result<Vec<u8>, AppError> {
    model_bytes_from_upstreams(
        models_dir, lock, manifest(), reqwest::Client::builder(), DOWNLOAD_TIMEOUTS,
    ).await
}

async fn model_bytes_from_upstreams(
    models_dir: &Path,
    lock: &Mutex<()>,
    model: &ModelManifest,
    client_builder: reqwest::ClientBuilder,
    timeouts: DownloadTimeouts,
) -> Result<Vec<u8>, AppError> {
    let path = models_dir.join(&model.filename);
    if let Some(bytes) = read_verified_cache(&path, model).await? {
        return Ok(bytes);
    }
    let _guard = lock.lock().await;
    if let Some(bytes) = read_verified_cache(&path, model).await? {
        return Ok(bytes);
    }
    tokio::fs::create_dir_all(models_dir).await
        .map_err(|error| AppError::Internal(format!("create models dir: {error}")))?;
    let client = client_builder.connect_timeout(timeouts.connect).build()
        .map_err(|error| AppError::Internal(format!("build model http client: {error}")))?;
    let mut errors = Vec::new();
    for url in &model.urls {
        match download_one(&client, url, model, timeouts).await {
            Ok(bytes) => {
                write_atomic(&path, &bytes).await?;
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

async fn read_verified_cache(path: &Path, model: &ModelManifest) -> Result<Option<Vec<u8>>, AppError> {
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AppError::Internal(format!("open matting model cache: {error}"))),
    };
    let meta = file.metadata().await
        .map_err(|error| AppError::Internal(format!("stat matting model cache: {error}")))?;
    if !meta.is_file() || meta.len() != model.size {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(model.size as usize);
    // Bound the read even if another process appends to the opened file.
    file.take(model.size + 1).read_to_end(&mut bytes).await
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
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || {
        let parent = path.parent()
            .ok_or_else(|| AppError::Internal("matting model path has no parent".into()))?;
        let mut temp = tempfile::Builder::new().prefix(".modnet.onnx.tmp.").tempfile_in(parent)
            .map_err(|error| AppError::Internal(format!("create model tempfile: {error}")))?;
        temp.write_all(&bytes).and_then(|_| temp.as_file_mut().sync_all())
            .map_err(|error| AppError::Internal(format!("write model tempfile: {error}")))?;
        temp.persist(&path)
            .map_err(|error| AppError::Internal(format!("commit model: {}", error.error)))?;
        crate::fsio::sync_dir(parent)
            .map_err(|error| AppError::Internal(format!("fsync models dir: {error}")))
    }).await.map_err(|error| AppError::Internal(format!("model write task failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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
            dir, lock, model, reqwest::Client::builder().no_proxy(), TEST_TIMEOUTS,
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
}
