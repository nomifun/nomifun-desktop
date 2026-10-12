//! Managed, bounded Office rendering. Requests are owner-scoped leases; render
//! jobs are shared only by immutable content + engine + renderer parameters.
//! No document listener, global package installation or ambient executable lookup.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use async_trait::async_trait;
use nomi_process_runtime::ChildProcessBuilder;
use nomifun_api_types::{
    OfficePreviewDocType, OfficePreviewError, OfficePreviewResponse, OfficePreviewState,
    PREVIEW_CAPABILITY_BYTES, is_preview_capability,
};
use nomifun_common::{AppError, validate_uuidv7};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{OnceCell, OwnedSemaphorePermit, Semaphore, watch},
};
use tokio_util::sync::CancellationToken;

const RENDER_PARAMETERS: &str = "static-html-v1;no-resident;all-pages";
const MAX_ZIP_ENTRIES: usize = 20_000;
const MAX_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const STDERR_BYTES: usize = 8192;
const TOMBSTONE_TTL: Duration = Duration::from_secs(60);
const MAX_TOMBSTONES: usize = 4096;

#[derive(Clone, Debug)]
pub struct OfficeEngine {
    pub executable_path: PathBuf,
    pub version: String,
    pub digest: String,
}

/// Only supply verified engines here. Production uses the canonical managed
/// tool supplier; injection keeps process/cancellation tests independent of downloads.
#[async_trait]
pub trait OfficeEngineSupplier: Send + Sync {
    async fn resolve(&self) -> Result<OfficeEngine, OfficePreviewError>;
}

pub struct ManagedOfficeEngineSupplier;
#[async_trait]
impl OfficeEngineSupplier for ManagedOfficeEngineSupplier {
    async fn resolve(&self) -> Result<OfficeEngine, OfficePreviewError> {
        let tool = nomifun_runtime::resolve_tool(nomifun_runtime::ToolId::OfficeCli)
            .await
            .map_err(|error| {
                tracing::error!(%error, "managed Office preview engine admission failed");
                failure(
                    "ENGINE_UNAVAILABLE",
                    "The managed Office preview component is unavailable or incompatible.",
                    false,
                )
            })?;
        Ok(OfficeEngine {
            executable_path: tool.executable_path,
            version: tool.version,
            digest: tool.digest,
        })
    }
}

#[derive(Clone)]
pub struct PreviewLimits {
    pub max_requests: usize,
    pub render_concurrency: usize,
    pub max_source_bytes: u64,
    pub max_html_bytes: u64,
    pub cache_bytes: u64,
    pub cache_entries: usize,
    pub lease_ttl: Duration,
    pub render_timeout: Duration,
}
impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            max_requests: 256,
            render_concurrency: 1,
            max_source_bytes: 64 * 1024 * 1024,
            max_html_bytes: 64 * 1024 * 1024,
            cache_bytes: 256 * 1024 * 1024,
            cache_entries: 128,
            lease_ttl: Duration::from_secs(2 * 60 * 60),
            render_timeout: Duration::from_secs(90),
        }
    }
}

struct Lease {
    request_id: String,
    source: Option<(PathBuf, OfficePreviewDocType)>,
    capability: String,
    cancelled: CancellationToken,
    response: Mutex<OfficePreviewResponse>,
    artifact: Mutex<Option<PathBuf>>,
    touched: Mutex<Instant>,
}
impl Lease {
    fn response(&self) -> OfficePreviewResponse {
        self.response.lock().unwrap().clone()
    }
    fn fail(&self, error: OfficePreviewError) {
        let mut response = self.response.lock().unwrap();
        *self.artifact.lock().unwrap() = None;
        response.state = OfficePreviewState::Error;
        response.url = None;
        response.capability = None;
        response.error = Some(error);
    }
}

#[derive(Clone)]
enum Progress {
    Queued,
    Rendering,
    Ready(PathBuf),
    Error(OfficePreviewError),
}
struct Job {
    subscribers: AtomicUsize,
    cancelled: CancellationToken,
    progress: watch::Sender<Progress>,
}
struct Interest(Arc<Job>);
impl Drop for Interest {
    fn drop(&mut self) {
        if self.0.subscribers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.cancelled.cancel();
        }
    }
}
#[derive(Default)]
struct Registry {
    leases: HashMap<(String, String), Arc<Lease>>,
    capabilities: HashMap<String, (String, String)>,
    jobs: HashMap<String, Arc<Job>>,
    tombstones: HashMap<(String, String), Instant>,
}
struct SourceVersion {
    metadata: std::fs::Metadata,
    digest: String,
    touched: Instant,
}
struct Shared {
    root: PathBuf,
    supplier: Arc<dyn OfficeEngineSupplier>,
    supplier_slots: Semaphore,
    limits: PreviewLimits,
    registry: Mutex<Registry>,
    source_versions: Mutex<HashMap<PathBuf, SourceVersion>>,
    render_slots: Semaphore,
    prepare_slots: Arc<Semaphore>,
    cache_gate: tokio::sync::Mutex<()>,
    initialized: OnceCell<()>,
    expiry_started: AtomicBool,
    shutdown: CancellationToken,
}

pub struct OfficePreviewService {
    shared: Arc<Shared>,
}
impl OfficePreviewService {
    pub fn new(data_dir: &Path) -> Self {
        Self::with_supplier(
            data_dir,
            Arc::new(ManagedOfficeEngineSupplier),
            PreviewLimits::default(),
        )
    }
    pub fn with_supplier(
        data_dir: &Path,
        supplier: Arc<dyn OfficeEngineSupplier>,
        limits: PreviewLimits,
    ) -> Self {
        // The data root may be newly configured or reached through a platform
        // alias such as /var. Store one canonical cache authority before work.
        let _ = std::fs::create_dir_all(data_dir);
        let service = Self {
            shared: Arc::new(Shared {
                root: std::fs::canonicalize(data_dir)
                    .unwrap_or_else(|_| data_dir.to_path_buf())
                    .join("office-preview-cache"),
                render_slots: Semaphore::new(limits.render_concurrency.max(1)),
                prepare_slots: Arc::new(Semaphore::new(2)),
                supplier,
                supplier_slots: Semaphore::new(2),
                limits,
                registry: Mutex::new(Registry::default()),
                source_versions: Mutex::new(HashMap::new()),
                cache_gate: tokio::sync::Mutex::new(()),
                initialized: OnceCell::new(),
                expiry_started: AtomicBool::new(false),
                shutdown: CancellationToken::new(),
            }),
        };
        start_expiry(&service.shared);
        service
    }

    pub fn start(
        &self,
        owner: &str,
        request_id: &str,
        source: PathBuf,
        doc_type: OfficePreviewDocType,
    ) -> Result<OfficePreviewResponse, AppError> {
        validate_request_id(request_id)?;
        start_expiry(&self.shared);
        let key = (owner.to_owned(), request_id.to_owned());
        let mut registry = self.shared.registry.lock().unwrap();
        prune_leases(&mut registry, self.shared.limits.lease_ttl);
        if registry.tombstones.contains_key(&key) {
            return Ok(cancelled_response(request_id));
        }
        if let Some(existing) = registry.leases.get(&key) {
            if existing
                .source
                .as_ref()
                .is_some_and(|value| value != &(source, doc_type))
            {
                return Err(AppError::Conflict(
                    "Preview request_id is already bound to another document.".into(),
                ));
            }
            *existing.touched.lock().unwrap() = Instant::now();
            return Ok(existing.response());
        }
        if registry.leases.len() >= self.shared.limits.max_requests {
            return Err(AppError::RateLimited);
        }
        let capability = mint_capability()?;
        let lease = Arc::new(Lease {
            request_id: request_id.to_owned(),
            source: Some((source, doc_type)),
            capability: capability.clone(),
            cancelled: CancellationToken::new(),
            response: Mutex::new(OfficePreviewResponse {
                request_id: request_id.to_owned(),
                state: OfficePreviewState::Queued,
                url: None,
                capability: None,
                error: None,
            }),
            artifact: Mutex::new(None),
            touched: Mutex::new(Instant::now()),
        });
        registry.capabilities.insert(capability, key.clone());
        registry.leases.insert(key, lease.clone());
        let initial = lease.response();
        drop(registry);
        let shared = self.shared.clone();
        tokio::spawn(async move {
            if let Err(error) = drive_request(shared, lease.clone()).await {
                if !lease.cancelled.is_cancelled() {
                    tracing::warn!(request_id = %lease.request_id, code = %error.code, retryable = error.retryable, "Office preview request failed");
                    lease.fail(error);
                }
            }
        });
        Ok(initial)
    }

    pub fn status(&self, owner: &str, request_id: &str) -> Result<OfficePreviewResponse, AppError> {
        validate_request_id(request_id)?;
        let mut registry = self.shared.registry.lock().unwrap();
        prune_leases(&mut registry, self.shared.limits.lease_ttl);
        let key = (owner.to_owned(), request_id.to_owned());
        if registry.tombstones.contains_key(&key) {
            return Ok(cancelled_response(request_id));
        }
        let lease = registry
            .leases
            .get(&key)
            .ok_or_else(|| AppError::NotFound("Preview request not found.".into()))?;
        *lease.touched.lock().unwrap() = Instant::now();
        Ok(lease.response())
    }

    /// A bounded cancellation tombstone also handles a stop that reaches the
    /// server before its start. IDs cannot resurrect stopped work.
    pub fn stop(&self, owner: &str, request_id: &str) -> Result<(), AppError> {
        validate_request_id(request_id)?;
        let key = (owner.to_owned(), request_id.to_owned());
        let mut registry = self.shared.registry.lock().unwrap();
        prune_leases(&mut registry, self.shared.limits.lease_ttl);
        if let Some(lease) = registry.leases.remove(&key) {
            lease.cancelled.cancel();
            lease.fail(cancelled_error());
            registry.capabilities.remove(&lease.capability);
        }
        registry.tombstones.insert(key, Instant::now());
        if registry.tombstones.len() > MAX_TOMBSTONES {
            if let Some(oldest) = registry
                .tombstones
                .iter()
                .min_by_key(|(_, created)| **created)
                .map(|(key, _)| key.clone())
            {
                registry.tombstones.remove(&oldest);
            }
        }
        Ok(())
    }

    /// Capability access only reads the already published immutable result.
    /// A new request must pass source-path authorization even on cache hits.
    pub async fn artifact_reader(
        &self,
        capability: &str,
        request_id: &str,
        nonce: &str,
    ) -> Result<impl AsyncRead + Unpin + Send + 'static, AppError> {
        validate_request_id(request_id)?;
        validate_request_id(nonce)?;
        if !is_preview_capability(capability) {
            return Err(AppError::Forbidden("Invalid preview capability.".into()));
        }
        let (lease, artifact, source) = {
            let mut registry = self.shared.registry.lock().unwrap();
            prune_leases(&mut registry, self.shared.limits.lease_ttl);
            let key = registry
                .capabilities
                .get(capability)
                .ok_or_else(|| AppError::Forbidden("Preview capability is inactive.".into()))?;
            let lease = registry
                .leases
                .get(key)
                .cloned()
                .ok_or_else(|| AppError::Forbidden("Preview capability is inactive.".into()))?;
            if lease.cancelled.is_cancelled() || lease.request_id != request_id {
                return Err(AppError::Forbidden(
                    "Preview capability is inactive or belongs to another request.".into(),
                ));
            }
            let artifact = lease
                .artifact
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| AppError::NotFound("Preview is not ready.".into()))?;
            let source = lease.source.as_ref().unwrap().0.clone();
            *lease.touched.lock().unwrap() = Instant::now();
            (lease, artifact, source)
        };
        // The grant cannot outlive source-path access or follow a replaced symlink.
        let actual = tokio::fs::canonicalize(&source)
            .await
            .map_err(|_| AppError::Forbidden("Preview source is no longer accessible.".into()))?;
        if actual != source {
            return Err(AppError::Forbidden(
                "Preview source identity changed.".into(),
            ));
        }
        let _source_handle = tokio::fs::File::open(&source)
            .await
            .map_err(|_| AppError::Forbidden("Preview source is no longer readable.".into()))?;
        let (file, size) = open_bounded(&artifact, self.shared.limits.max_html_bytes)
            .await
            .map_err(|_| {
                AppError::NotFound(
                    "Preview artifact is unavailable or exceeds its resource limit.".into(),
                )
            })?;
        if lease.cancelled.is_cancelled() {
            return Err(AppError::Forbidden(
                "Preview capability is inactive.".into(),
            ));
        }
        // HTTP streams this reader in small chunks instead of buffering a whole
        // HTML document for each client, including slow or concurrent readers.
        Ok(file.take(size).chain(std::io::Cursor::new(inject_readiness(
            Vec::new(),
            capability,
        ))))
    }

    pub async fn read_artifact(
        &self,
        capability: &str,
        request_id: &str,
        nonce: &str,
    ) -> Result<Vec<u8>, AppError> {
        let mut reader = self.artifact_reader(capability, request_id, nonce).await?;
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| AppError::Internal("Preview artifact could not be read.".into()))?;
        Ok(bytes)
    }
}
impl Drop for OfficePreviewService {
    fn drop(&mut self) {
        self.shared.shutdown.cancel();
        let registry = self.shared.registry.lock().unwrap();
        for lease in registry.leases.values() {
            lease.cancelled.cancel();
        }
        for job in registry.jobs.values() {
            job.cancelled.cancel();
        }
    }
}

fn validate_request_id(request_id: &str) -> Result<(), AppError> {
    validate_uuidv7(request_id)
        .map(|_| ())
        .map_err(|_| AppError::BadRequest("request_id must be a canonical UUIDv7.".into()))
}
fn mint_capability() -> Result<String, AppError> {
    let mut bytes = [0_u8; PREVIEW_CAPABILITY_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| AppError::Internal("Could not create a preview capability.".into()))?;
    Ok(hex::encode(bytes))
}
fn failure(code: &str, message: &str, retryable: bool) -> OfficePreviewError {
    OfficePreviewError {
        code: code.into(),
        message: message.into(),
        retryable,
    }
}
fn cancelled_error() -> OfficePreviewError {
    failure("CANCELLED", "Preview request was cancelled.", false)
}
fn cancelled_response(request_id: &str) -> OfficePreviewResponse {
    OfficePreviewResponse {
        request_id: request_id.into(),
        state: OfficePreviewState::Error,
        url: None,
        capability: None,
        error: Some(cancelled_error()),
    }
}
fn io_failure(error: std::io::Error) -> OfficePreviewError {
    tracing::warn!(%error, "Office preview I/O failed");
    failure(
        "FILE_UNAVAILABLE",
        "The document or preview cache could not be read or written.",
        true,
    )
}
fn prune_leases(registry: &mut Registry, ttl: Duration) {
    registry
        .tombstones
        .retain(|_, created| created.elapsed() < TOMBSTONE_TTL);
    registry.leases.retain(|_, lease| {
        if lease.touched.lock().unwrap().elapsed() >= ttl {
            lease.cancelled.cancel();
            false
        } else {
            true
        }
    });
    registry
        .capabilities
        .retain(|_, key| registry.leases.contains_key(key));
    registry
        .jobs
        .retain(|_, job| job.subscribers.load(Ordering::Acquire) > 0);
}

fn start_expiry(shared: &Arc<Shared>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if shared.expiry_started.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = Arc::downgrade(shared);
    let cancellation = shared.shutdown.clone();
    let period =
        (shared.limits.lease_ttl / 2).clamp(Duration::from_millis(10), Duration::from_secs(60));
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(period);
        loop {
            tokio::select! { biased; _ = cancellation.cancelled() => break, _ = interval.tick() => {} }
            let Some(shared) = weak.upgrade() else { break; };
            prune_leases(&mut shared.registry.lock().unwrap(), shared.limits.lease_ttl);
        }
    });
}

struct Staging {
    path: PathBuf,
    admission: Option<OwnedSemaphorePermit>,
}
impl Drop for Staging {
    fn drop(&mut self) {
        let path = self.path.clone();
        let admission = self.admission.take();
        // Retain snapshot admission through render and disposable-file cleanup.
        tokio::spawn(async move {
            let _ = tokio::fs::remove_dir_all(path).await;
            drop(admission);
        });
    }
}

enum Prepared {
    Cached(PathBuf),
    Render(Staging, PathBuf, OfficeEngine, String),
}
fn artifact_key(content_digest: &str, engine: &OfficeEngine, extension: &str) -> String {
    let mut hash = Sha256::new();
    for value in [
        content_digest,
        engine.digest.as_str(),
        engine.version.as_str(),
        extension,
        RENDER_PARAMETERS,
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hex::encode(hash.finalize())
}
fn publish_lease(lease: &Lease, path: PathBuf) {
    let mut response = lease.response.lock().unwrap();
    if lease.cancelled.is_cancelled() {
        return;
    }
    *lease.artifact.lock().unwrap() = Some(path);
    response.state = OfficePreviewState::Ready;
    response.url = Some(format!(
        "/api/office-preview-artifacts/{}/",
        lease.capability
    ));
    response.capability = Some(lease.capability.clone());
}
async fn drive_request(shared: Arc<Shared>, lease: Arc<Lease>) -> Result<(), OfficePreviewError> {
    let prepare = async {
        shared
            .initialized
            .get_or_try_init(|| async {
                tokio::fs::create_dir_all(&shared.root).await?;
                if tokio::fs::canonicalize(&shared.root).await? != shared.root {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "preview cache authority changed",
                    ));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    tokio::fs::set_permissions(
                        &shared.root,
                        std::fs::Permissions::from_mode(0o700),
                    )
                    .await?;
                }
                tokio::fs::create_dir_all(shared.root.join("artifacts")).await?;
                for path in [&shared.root, &shared.root.join("artifacts")] {
                    if tokio::fs::canonicalize(path).await? != *path {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "preview cache authority changed",
                        ));
                    }
                }
                // Disposable copies from a prior application exit never become inputs.
                match tokio::fs::remove_dir_all(shared.root.join("staging")).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                tokio::fs::create_dir_all(shared.root.join("staging")).await?;
                Ok::<(), std::io::Error>(())
            })
            .await
            .map_err(io_failure)?;
        let (source, doc_type) = lease.source.as_ref().unwrap();
        let doc_type = *doc_type;
        let extension = expected_extension(doc_type);
        if source
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case(extension))
        {
            return Err(failure(
                "FORMAT_UNSUPPORTED",
                "This file format does not support built-in preview. Open it with a system application.",
                false,
            ));
        }
        let supplier_slot = shared
            .supplier_slots
            .acquire()
            .await
            .map_err(|_| cancelled_error())?;
        let engine = tokio::time::timeout(Duration::from_secs(20), shared.supplier.resolve())
            .await
            .map_err(|_| {
                failure(
                    "PREPARATION_TIMEOUT",
                    "The preview component took too long to prepare.",
                    true,
                )
            })??;
        drop(supplier_slot);
        // An unchanged, previously snapshotted file can hit the artifact cache
        // immediately. The fingerprint includes native identity and change time;
        // a new version still goes through the quiet-window copy and ZIP check.
        let current = tokio::fs::File::open(source)
            .await
            .map_err(io_failure)?
            .metadata()
            .await
            .map_err(io_failure)?;
        let known_digest = shared
            .source_versions
            .lock()
            .unwrap()
            .get_mut(source)
            .filter(|record| cfg!(unix) && same_file_state(&record.metadata, &current))
            .map(|record| {
                record.touched = Instant::now();
                record.digest.clone()
            });
        if let Some(digest) = known_digest {
            if tokio::fs::canonicalize(source).await.map_err(io_failure)? == *source {
                let key = artifact_key(&digest, &engine, extension);
                let _gate = shared.cache_gate.lock().await;
                if let Some(path) = cached_artifact(&shared, &key).await {
                    publish_lease(&lease, path.clone());
                    return Ok(Prepared::Cached(path));
                }
            }
        }
        let admission = shared
            .prepare_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| cancelled_error())?;
        let staging = Staging {
            path: shared.root.join("staging").join(&lease.capability),
            admission: Some(admission),
        };
        tokio::fs::create_dir_all(&staging.path)
            .await
            .map_err(io_failure)?;
        let copy = staging.path.join(format!("source.{extension}"));
        let (content_digest, metadata) = tokio::time::timeout(
            Duration::from_secs(10),
            stable_copy(source, &copy, shared.limits.max_source_bytes),
        )
        .await
        .map_err(|_| {
            failure(
                "PREPARATION_TIMEOUT",
                "The document took too long to snapshot.",
                true,
            )
        })??;
        let zip_copy = copy.clone();
        tokio::task::spawn_blocking(move || validate_document(&zip_copy, doc_type))
            .await
            .map_err(|_| {
                failure(
                    "DOCUMENT_INVALID",
                    "The document could not be inspected.",
                    true,
                )
            })??;
        let key = artifact_key(&content_digest, &engine, extension);
        let mut versions = shared.source_versions.lock().unwrap();
        if versions.len() >= shared.limits.max_requests {
            if let Some(oldest) = versions
                .iter()
                .min_by_key(|(_, record)| record.touched)
                .map(|(path, _)| path.clone())
            {
                versions.remove(&oldest);
            }
        }
        versions.insert(
            source.clone(),
            SourceVersion {
                metadata,
                digest: content_digest,
                touched: Instant::now(),
            },
        );
        Ok::<_, OfficePreviewError>(Prepared::Render(staging, copy, engine, key))
    };
    let prepared = tokio::select! {
        biased;
        _ = lease.cancelled.cancelled() => return Ok(()),
        result = prepare => result?,
    };
    if lease.cancelled.is_cancelled() {
        return Ok(());
    }
    let (staging, source_copy, engine, key) = match prepared {
        Prepared::Cached(path) => {
            publish_lease(&lease, path);
            return Ok(());
        }
        Prepared::Render(staging, copy, engine, key) => (staging, copy, engine, key),
    };
    let (job, created) = {
        let mut registry = shared.registry.lock().unwrap();
        let existing = registry
            .jobs
            .get(&key)
            .filter(|job| !job.cancelled.is_cancelled())
            .cloned();
        let created = existing.is_none();
        let job = existing.unwrap_or_else(|| {
            Arc::new(Job {
                subscribers: AtomicUsize::new(0),
                cancelled: CancellationToken::new(),
                progress: watch::channel(Progress::Queued).0,
            })
        });
        job.subscribers.fetch_add(1, Ordering::AcqRel);
        registry.jobs.insert(key.clone(), job.clone());
        (job, created)
    };
    let _interest = Interest(job.clone());
    let mut receiver = job.progress.subscribe();
    if created {
        let worker_shared = shared.clone();
        let worker_job = job.clone();
        tokio::spawn(async move {
            let result = generate_artifact(
                worker_shared.clone(),
                worker_job.clone(),
                &key,
                &staging.path,
                &source_copy,
                &engine,
            )
            .await;
            worker_job.progress.send_replace(match result {
                Ok(path) => Progress::Ready(path),
                Err(error) => Progress::Error(error),
            });
            drop(staging);
        });
    } else {
        drop(staging);
    }
    loop {
        if lease.cancelled.is_cancelled() {
            return Ok(());
        }
        let progress = receiver.borrow().clone();
        match progress {
            Progress::Queued => {}
            Progress::Rendering => {
                lease.response.lock().unwrap().state = OfficePreviewState::Rendering
            }
            Progress::Ready(path) => {
                publish_lease(&lease, path);
                return Ok(());
            }
            Progress::Error(error) => return Err(error),
        }
        tokio::select! { biased; _ = lease.cancelled.cancelled() => return Ok(()), _ = receiver.changed() => {} }
    }
}
fn expected_extension(doc_type: OfficePreviewDocType) -> &'static str {
    match doc_type {
        OfficePreviewDocType::Word => "docx",
        OfficePreviewDocType::Excel => "xlsx",
        OfficePreviewDocType::Ppt => "pptx",
    }
}

async fn stable_copy(
    source: &Path,
    target: &Path,
    limit: u64,
) -> Result<(String, std::fs::Metadata), OfficePreviewError> {
    // Wait for a quiet write window, then read a bounded snapshot through one
    // handle and verify that both its metadata and path identity stayed stable.
    for _ in 0..4 {
        let actual = tokio::fs::canonicalize(source).await.map_err(io_failure)?;
        if actual != source {
            return Err(failure(
                "FILE_CHANGED",
                "The document path changed while preparing preview.",
                true,
            ));
        }
        let before = tokio::fs::metadata(source).await.map_err(io_failure)?;
        if !before.is_file() || before.len() > limit {
            return Err(failure(
                "RESOURCE_LIMIT",
                "The document exceeds the built-in preview size limit.",
                false,
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut input = tokio::fs::File::open(source).await.map_err(io_failure)?;
        let opened = input.metadata().await.map_err(io_failure)?;
        if !same_file_state(&before, &opened) {
            continue;
        }
        let mut output = tokio::fs::File::create(target).await.map_err(io_failure)?;
        let mut hash = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer).await.map_err(io_failure)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > limit {
                return Err(failure(
                    "RESOURCE_LIMIT",
                    "The document exceeds the built-in preview size limit.",
                    false,
                ));
            }
            hash.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .await
                .map_err(io_failure)?;
        }
        output.flush().await.map_err(io_failure)?;
        let after = input.metadata().await.map_err(io_failure)?;
        let path_after = tokio::fs::metadata(source).await.map_err(io_failure)?;
        if same_file_state(&opened, &after)
            && same_file_state(&opened, &path_after)
            && total == opened.len()
            && tokio::fs::canonicalize(source).await.map_err(io_failure)? == source
        {
            return Ok((hex::encode(hash.finalize()), opened));
        }
    }
    Err(failure(
        "FILE_CHANGED",
        "The document is still being written. Try again when it is saved.",
        true,
    ))
}
fn same_file_state(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev()
            || left.ino() != right.ino()
            || left.ctime() != right.ctime()
            || left.ctime_nsec() != right.ctime_nsec()
        {
            return false;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if left.creation_time() != right.creation_time()
            || left.file_attributes() != right.file_attributes()
        {
            return false;
        }
    }
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}
fn validate_document(
    path: &Path,
    doc_type: OfficePreviewDocType,
) -> Result<(), OfficePreviewError> {
    let file = std::fs::File::open(path).map_err(io_failure)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| {
        failure(
            "DOCUMENT_INVALID",
            "The document is incomplete, encrypted, or damaged.",
            true,
        )
    })?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(failure(
            "RESOURCE_LIMIT",
            "The document contains too many embedded parts.",
            false,
        ));
    }
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| {
            failure(
                "DOCUMENT_INVALID",
                "The document contains an unreadable embedded part.",
                false,
            )
        })?;
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            return Err(failure(
                "RESOURCE_LIMIT",
                "The document's expanded content exceeds the preview limit.",
                false,
            ));
        }
    }
    let part = match doc_type {
        OfficePreviewDocType::Word => "word/document.xml",
        OfficePreviewDocType::Excel => "xl/workbook.xml",
        OfficePreviewDocType::Ppt => "ppt/presentation.xml",
    };
    if archive.by_name("[Content_Types].xml").is_err() || archive.by_name(part).is_err() {
        return Err(failure(
            "DOCUMENT_INVALID",
            "The file does not contain the expected Office document structure.",
            false,
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    key: String,
    html_digest: String,
    html_bytes: u64,
}
async fn cached_artifact(shared: &Shared, key: &str) -> Option<PathBuf> {
    let directory = shared.root.join("artifacts").join(key);
    let directory_type = tokio::fs::symlink_metadata(&directory).await.ok()?;
    if !directory_type.is_dir() {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(
        &read_bounded(&directory.join("manifest.json"), 4096)
            .await
            .ok()?,
    )
    .ok()?;
    if manifest.key != key || manifest.html_bytes > shared.limits.max_html_bytes {
        return None;
    }
    let path = directory.join("index.html");
    let metadata = tokio::fs::symlink_metadata(&path).await.ok()?;
    if !metadata.is_file() || metadata.len() != manifest.html_bytes {
        return None;
    }
    let html = read_bounded(&path, shared.limits.max_html_bytes)
        .await
        .ok()?;
    if hex::encode(Sha256::digest(&html)) != manifest.html_digest || !valid_html(&html) {
        return None;
    }
    let _ = tokio::fs::write(directory.join("access"), []).await;
    Some(path)
}
async fn generate_artifact(
    shared: Arc<Shared>,
    job: Arc<Job>,
    key: &str,
    staging: &Path,
    source: &Path,
    engine: &OfficeEngine,
) -> Result<PathBuf, OfficePreviewError> {
    {
        let _gate = shared.cache_gate.lock().await;
        if let Some(path) = cached_artifact(&shared, key).await {
            job.progress.send_replace(Progress::Ready(path.clone()));
            return Ok(path);
        }
    }
    let _permit = tokio::select! { biased; _ = job.cancelled.cancelled() => return Err(cancelled_error()), permit = shared.render_slots.acquire() => permit.map_err(|_| cancelled_error())? };
    job.progress.send_replace(Progress::Rendering);
    let html_path = staging.join("index.html");
    render_html(
        engine,
        source,
        &html_path,
        &job.cancelled,
        shared.limits.render_timeout,
        shared.limits.max_html_bytes,
    )
    .await?;
    let html = read_bounded(&html_path, shared.limits.max_html_bytes)
        .await
        .map_err(io_failure)?;
    if !valid_html(&html) {
        return Err(failure(
            "OUTPUT_INVALID",
            "The preview component did not produce a valid HTML document.",
            false,
        ));
    }
    let manifest = Manifest {
        key: key.to_owned(),
        html_digest: hex::encode(Sha256::digest(&html)),
        html_bytes: html.len() as u64,
    };
    // Only publication/eviction is serialized; rendering never holds this lock.
    let _gate = shared.cache_gate.lock().await;
    if job.cancelled.is_cancelled() {
        return Err(cancelled_error());
    }
    if let Some(path) = cached_artifact(&shared, key).await {
        job.progress.send_replace(Progress::Ready(path.clone()));
        return Ok(path);
    }
    prune_cache(&shared, html.len() as u64).await?;
    let publish = staging.join("publish");
    tokio::fs::create_dir(&publish).await.map_err(io_failure)?;
    tokio::fs::rename(&html_path, publish.join("index.html"))
        .await
        .map_err(io_failure)?;
    tokio::fs::write(
        publish.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .await
    .map_err(io_failure)?;
    tokio::fs::write(publish.join("access"), [])
        .await
        .map_err(io_failure)?;
    let directory = shared.root.join("artifacts").join(key);
    if tokio::fs::try_exists(&directory)
        .await
        .map_err(io_failure)?
    {
        tokio::fs::remove_dir_all(&directory)
            .await
            .map_err(io_failure)?;
    }
    tokio::fs::rename(publish, &directory)
        .await
        .map_err(io_failure)?;
    let path = directory.join("index.html");
    job.progress.send_replace(Progress::Ready(path.clone()));
    Ok(path)
}

async fn prune_cache(shared: &Shared, incoming_bytes: u64) -> Result<(), OfficePreviewError> {
    let pinned: HashSet<PathBuf> = {
        let registry = shared.registry.lock().unwrap();
        let mut paths: HashSet<PathBuf> = registry
            .leases
            .values()
            .filter_map(|lease| {
                lease
                    .artifact
                    .lock()
                    .unwrap()
                    .clone()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
            .collect();
        for job in registry
            .jobs
            .values()
            .filter(|job| job.subscribers.load(Ordering::Acquire) > 0)
        {
            if let Progress::Ready(path) = &*job.progress.borrow() {
                if let Some(parent) = path.parent() {
                    paths.insert(parent.to_path_buf());
                }
            }
        }
        paths
    };
    let mut entries = Vec::new();
    let mut total = 0_u64;
    let mut directories = tokio::fs::read_dir(shared.root.join("artifacts"))
        .await
        .map_err(io_failure)?;
    while let Some(entry) = directories.next_entry().await.map_err(io_failure)? {
        if !entry.file_type().await.map_err(io_failure)?.is_dir() {
            continue;
        }
        let bytes = tokio::fs::metadata(entry.path().join("index.html"))
            .await
            .map(|value| value.len())
            .unwrap_or(0);
        let accessed = tokio::fs::metadata(entry.path().join("access"))
            .await
            .ok()
            .and_then(|value| value.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        total = total.saturating_add(bytes);
        entries.push((entry.path(), bytes, accessed));
    }
    entries.sort_by_key(|entry| entry.2);
    let mut count = entries.len();
    for (path, bytes, _) in entries {
        if total.saturating_add(incoming_bytes) <= shared.limits.cache_bytes
            && count < shared.limits.cache_entries
        {
            break;
        }
        if !pinned.contains(&path) {
            tokio::fs::remove_dir_all(&path).await.map_err(io_failure)?;
            total = total.saturating_sub(bytes);
            count -= 1;
        }
    }
    if total.saturating_add(incoming_bytes) > shared.limits.cache_bytes
        || count >= shared.limits.cache_entries
    {
        return Err(failure(
            "CACHE_BUSY",
            "The preview cache is full of open documents. Close another preview and retry.",
            true,
        ));
    }
    Ok(())
}

/// Capture bounded diagnostics and stream HTML to disk. Every outcome seals
/// the managed process tree, including early exit, overflow, timeout and cancel.
pub async fn render_html(
    engine: &OfficeEngine,
    source: &Path,
    output: &Path,
    cancellation: &CancellationToken,
    timeout: Duration,
    max_bytes: u64,
) -> Result<(), OfficePreviewError> {
    let mut builder = ChildProcessBuilder::clean_cli(&engine.executable_path);
    builder
        .arg("view")
        .arg(source)
        .arg("html")
        .env("OFFICECLI_NO_AUTO_RESIDENT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(source.parent().unwrap_or(Path::new(".")));
    let mut process = builder.spawn_managed().map_err(|error| {
        tracing::error!(%error, "Office renderer spawn failed");
        failure(
            "RENDER_FAILED",
            "The preview component could not start.",
            false,
        )
    })?;
    let stdout = process.stdout.take().unwrap();
    let stderr = process.stderr.take().unwrap();
    let output_path = output.to_owned();
    let mut stdout_task =
        tokio::spawn(async move { stream_html(stdout, &output_path, max_bytes).await });
    let stderr_task = tokio::spawn(async move { bounded_stderr(stderr).await });
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut stdout_result = None;
    let result = loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break Err(cancelled_error()),
            _ = &mut deadline => break Err(failure("RENDER_TIMEOUT", "The document took too long to render.", true)),
            output = &mut stdout_task, if stdout_result.is_none() => {
                let output = output.unwrap_or_else(|_| Err(failure("RENDER_FAILED", "The preview output stream could not be read.", true)));
                if let Err(error) = output { stdout_result = Some(Ok(())); break Err(error); }
                stdout_result = Some(output);
            },
            status = process.wait() => match status {
                Ok(status) if status.success() => break Ok(()),
                Ok(status) => {
                    tracing::warn!(exit_code = ?status.code(), "Office renderer exited unsuccessfully");
                    break Err(failure("RENDER_FAILED", "The preview component exited before producing a document.", false));
                },
                Err(error) => break Err(io_failure(error)),
            },
        }
    };
    if let Err(error) = process.shutdown().await {
        stdout_task.abort();
        stderr_task.abort();
        tracing::error!(%error, "Office renderer process tree cleanup not proven");
        return Err(failure(
            "CLEANUP_FAILED",
            "The preview process could not be stopped safely.",
            false,
        ));
    }
    let stdout_result = match stdout_result {
        Some(result) => result,
        None => stdout_task.await.map_err(|_| {
            failure(
                "RENDER_FAILED",
                "The preview output stream could not be read.",
                true,
            )
        })?,
    };
    let diagnostics = stderr_task.await.unwrap_or_default();
    let mut diagnostics = nomi_redact::redact_secrets(&diagnostics).into_owned();
    if let Some(capability) = source
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| is_preview_capability(name))
    {
        diagnostics = diagnostics.replace(capability, "[preview-request]");
    }
    if let Err(error) = &result {
        tracing::warn!(code = %error.code, engine_version = %engine.version, stderr = %diagnostics, "Office rendering failed");
    }
    result?;
    stdout_result
}

async fn stream_html(
    mut stream: impl AsyncRead + Unpin,
    output: &Path,
    limit: u64,
) -> Result<(), OfficePreviewError> {
    let mut file = tokio::fs::File::create(output).await.map_err(io_failure)?;
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = stream.read(&mut buffer).await.map_err(io_failure)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err(failure(
                "RESOURCE_LIMIT",
                "The generated preview exceeds the display size limit.",
                false,
            ));
        }
        file.write_all(&buffer[..count]).await.map_err(io_failure)?;
    }
    file.flush().await.map_err(io_failure)
}
async fn bounded_stderr(mut stream: impl AsyncRead + Unpin) -> String {
    let mut tail = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let Ok(count) = stream.read(&mut buffer).await else {
            break;
        };
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..count]);
        if tail.len() > STDERR_BYTES {
            tail.drain(..tail.len() - STDERR_BYTES);
        }
    }
    // Logs stay single-line and bounded; stdout/document contents never enter them.
    String::from_utf8_lossy(&tail).replace(['\n', '\r'], " ")
}
async fn open_bounded(path: &Path, limit: u64) -> Result<(tokio::fs::File, u64), std::io::Error> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if !metadata.is_file() || metadata.len() > limit || tokio::fs::canonicalize(path).await? != path
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "preview artifact identity or size invalid",
        ));
    }
    let file = tokio::fs::File::open(path).await?;
    if !same_file_state(&metadata, &file.metadata().await?) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "preview artifact changed",
        ));
    }
    Ok((file, metadata.len()))
}
async fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, std::io::Error> {
    let (file, size) = open_bounded(path, limit).await?;
    let mut reader = file.take(limit.saturating_add(1));
    let mut bytes = Vec::with_capacity(size as usize);
    reader.read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "preview artifact too large",
        ));
    }
    Ok(bytes)
}

fn valid_html(bytes: &[u8]) -> bool {
    let Ok(html) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lower = html.to_ascii_lowercase();
    lower.contains("<html") && lower.contains("<body") && lower.contains("</html>")
}
fn inject_readiness(mut html: Vec<u8>, capability: &str) -> Vec<u8> {
    let script = format!(
        r#"<style>.file-title{{display:none!important}}</style><script>(()=>{{
const q=new URLSearchParams(location.search),fields={{capability:'{capability}',request_id:q.get('request_id'),nonce:q.get('nonce')}};
const send=(type,extra={{}})=>parent.postMessage({{type,...fields,...extra}},'*');
const pane=()=>document.querySelector('.main')||document.querySelector('.sheet-content.active .table-wrapper')||document.scrollingElement;
const position=()=>{{const p=pane(),sheet=document.querySelector('.sheet-content.active')?.dataset.sheet;return {{x:p?.scrollLeft||0,y:p?.scrollTop||0,...(sheet!==undefined?{{sheet:Number(sheet)}}:{{}})}}}};
addEventListener('click',e=>{{if(e.target?.closest?.('.sheet-tab'))requestAnimationFrame(()=>send('nomifun-office-preview-scroll',position()));if(e.target?.closest?.('a[href]'))e.preventDefault()}},true);
let pending=false;addEventListener('scroll',e=>{{if(e.target!==document&&e.target!==pane())return;if(!pending){{pending=true;requestAnimationFrame(()=>{{pending=false;send('nomifun-office-preview-scroll',position())}})}}}},{{passive:true,capture:true}});
addEventListener('message',e=>{{const d=e.data;if(e.source!==parent||d?.type!=='nomifun-office-preview-restore-scroll'||d.capability!==fields.capability||d.request_id!==fields.request_id||d.nonce!==fields.nonce||!Number.isFinite(d.x)||!Number.isFinite(d.y)||d.x<0||d.y<0)return;if(Number.isInteger(d.sheet)&&d.sheet>=0&&d.sheet<=4096&&document.querySelector('.sheet-content[data-sheet="'+d.sheet+'"]')&&typeof switchSheet==='function')switchSheet(d.sheet);const p=pane();if(p)p.scrollTo({{left:d.x,top:d.y,behavior:'instant'}})}});
const ready=async()=>{{await document.fonts.ready;await Promise.all(Array.from(document.images).map(i=>i.complete?Promise.resolve():new Promise(r=>{{i.addEventListener('load',r,{{once:true}});i.addEventListener('error',r,{{once:true}})}})));requestAnimationFrame(()=>requestAnimationFrame(()=>send('nomifun-office-preview-ready')))}};
if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',ready,{{once:true}});else ready()
}})();</script>"#
    );
    html.extend_from_slice(script.as_bytes());
    html
}
