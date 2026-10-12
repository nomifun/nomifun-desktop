//! Product-owned tools are admitted once from trusted embeds or explicit development paths.
use crate::progress::{ToolPreparationFailure, ToolPreparationPhase, ToolPreparationSnapshot};
use crate::{cache, embed, extract};
use nomi_process_runtime::ChildProcessBuilder;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

#[path = "../build_support.rs"]
pub(crate) mod contract;

/// Read-only supply catalogue, including references to existing domain-owned manifests.
pub fn component_inventory() -> serde_json::Value {
    let mut inventory: serde_json::Value =
        serde_json::from_str(contract::MANIFEST).expect("valid component inventory");
    inventory["component_notices"] =
        serde_json::from_str(contract::NOTICE_MANIFEST).expect("valid component notice inventory");
    inventory
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    Bun,
    Node,
    #[serde(rename = "officecli")]
    OfficeCli,
}
impl ToolId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bun => "bun",
            Self::Node => "node",
            Self::OfficeCli => "officecli",
        }
    }
    pub fn override_variable(self) -> &'static str {
        match self {
            Self::Bun => "NOMIFUN_BUN_PATH",
            Self::Node => "NOMIFUN_NODE_PATH",
            Self::OfficeCli => "NOMIFUN_OFFICECLI_PATH",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTool {
    pub executable_path: PathBuf,
    pub version: String,
    /// SHA-256 of the actual admitted executable, including explicit development overrides.
    pub digest: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error(
        "approved {tool} component is not bundled; development requires an explicit {override_variable} path"
    )]
    Unavailable {
        tool: &'static str,
        override_variable: &'static str,
    },
    #[error("invalid {tool} development override: {reason}")]
    InvalidOverride { tool: &'static str, reason: String },
    #[error("component extraction failed: {0}")]
    Extract(#[from] extract::ExtractError),
    #[error("component I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("{tool} identity or capability verification failed: {reason}")]
    Verification { tool: &'static str, reason: String },
    #[error("{tool} component changed after admission; restart after repairing the component")]
    Changed { tool: &'static str },
    #[error("approved component has not been prepared")]
    NotPrepared,
    #[error("component preparation was cancelled")]
    Cancelled,
    #[error("component download failed: {0}")]
    Download(String),
    #[error("component download exceeds the approved size limit")]
    TooLarge,
    #[error("component asset is invalid: {0}")]
    InvalidAsset(String),
    #[error("component process cleanup could not be proven: {0}")]
    Cleanup(String),
    #[error("{0}")]
    Shared(Arc<ToolError>),
    #[error("component resolver worker failed: {0}")]
    Worker(String),
}

struct Admitted {
    tool: VerifiedTool,
    fingerprint: FileFingerprint,
}

#[derive(Debug, PartialEq, Eq)]
struct FileFingerprint {
    size: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64),
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    attributes: u32,
}

fn fingerprint(path: &Path) -> std::io::Result<FileFingerprint> {
    let metadata = std::fs::metadata(path)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;
    Ok(FileFingerprint {
        size: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        inode: (metadata.dev(), metadata.ino()),
        #[cfg(unix)]
        changed: (metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(unix)]
        mode: metadata.mode(),
        #[cfg(windows)]
        creation_time: metadata.creation_time(),
        #[cfg(windows)]
        attributes: metadata.file_attributes(),
    })
}
struct PreparationAttempt {
    cancel: CancellationToken,
    result: OnceLock<Result<VerifiedTool, Arc<ToolError>>>,
    completed: Notify,
}

struct ToolSlot {
    admitted: OnceLock<Admitted>,
    active: Mutex<Option<Arc<PreparationAttempt>>>,
    progress: watch::Sender<ToolPreparationSnapshot>,
}

static BUN: OnceLock<ToolSlot> = OnceLock::new();
static NODE: OnceLock<ToolSlot> = OnceLock::new();
static OFFICE: OnceLock<ToolSlot> = OnceLock::new();

fn slot(id: ToolId) -> &'static ToolSlot {
    let slot = match id {
        ToolId::Bun => &BUN,
        ToolId::Node => &NODE,
        ToolId::OfficeCli => &OFFICE,
    };
    slot.get_or_init(|| {
        let spec = specification(id);
        let (progress, _) = watch::channel(ToolPreparationSnapshot {
            tool: id,
            version: spec.version,
            revision: 0,
            phase: ToolPreparationPhase::Idle,
            received_bytes: 0,
            total_bytes: None,
            error: None,
        });
        ToolSlot {
            admitted: OnceLock::new(),
            active: Mutex::new(None),
            progress,
        }
    })
}

pub(crate) fn specification(id: ToolId) -> contract::ComponentSpec {
    contract::manifest()
        .components
        .into_iter()
        .find(|spec| spec.id == id.as_str())
        .expect("approved tool declaration")
}

pub fn tool_preparation_snapshot(id: ToolId) -> ToolPreparationSnapshot {
    slot(id).progress.borrow().clone()
}

pub fn subscribe_tool_preparation(id: ToolId) -> watch::Receiver<ToolPreparationSnapshot> {
    slot(id).progress.subscribe()
}

/// Begin one shared supplier operation without queuing workers for duplicate callers.
pub fn begin_tool_preparation(id: ToolId) -> ToolPreparationSnapshot {
    // This acknowledgment performs no filesystem work, even for a warm cache.
    // Fingerprint checks happen in the same supplier worker before Ready.
    if let Err(error) = start_attempt(id) {
        publish_failure(id, &error);
    }
    tool_preparation_snapshot(id)
}

/// Await exactly the shared supplier operation started by begin/another consumer.
pub async fn resolve_tool(id: ToolId) -> Result<VerifiedTool, ToolError> {
    if let Some(tool) = cached_tool(id) {
        return tool;
    }
    let attempt = start_attempt(id)?;
    loop {
        let notified = attempt.completed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if let Some(result) = attempt.result.get() {
            return result.clone().map_err(ToolError::Shared);
        }
        notified.await;
    }
}

pub async fn prepare_tool(id: ToolId) -> Result<VerifiedTool, ToolError> {
    resolve_tool(id).await
}

/// Global cancellation is reserved for the host exiting; observers/consumer timeouts
/// merely stop waiting and never revoke the shared producer for other consumers.
pub fn cancel_tool_preparation(id: ToolId) {
    if let Ok(active) = slot(id).active.lock()
        && let Some(attempt) = active.as_ref()
    {
        attempt.cancel.cancel();
    }
}

fn start_attempt(id: ToolId) -> Result<Arc<PreparationAttempt>, ToolError> {
    let mut active = slot(id)
        .active
        .lock()
        .map_err(|_| ToolError::Worker("component controller lock poisoned".into()))?;
    if let Some(attempt) = active.as_ref() {
        return Ok(Arc::clone(attempt));
    }
    let attempt = Arc::new(PreparationAttempt {
        cancel: CancellationToken::new(),
        result: OnceLock::new(),
        completed: Notify::new(),
    });
    *active = Some(Arc::clone(&attempt));
    slot(id).progress.send_modify(|snapshot| {
        snapshot.phase = ToolPreparationPhase::Checking;
        snapshot.received_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.error = None;
        snapshot.revision += 1;
    });
    let worker = Arc::clone(&attempt);
    drop(active);
    let spawned = std::thread::Builder::new()
        .name(format!("prepare-{}", id.as_str()))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(admit_tool(id, &worker.cancel))
            }))
            .unwrap_or_else(|_| {
                Err(ToolError::Cleanup(
                    "component supplier panicked before cleanup proof".into(),
                ))
            });
            finish_attempt(id, &worker, result);
        });
    if let Err(error) = spawned {
        finish_attempt(id, &attempt, Err(ToolError::Worker(error.to_string())));
    }
    Ok(attempt)
}

fn finish_attempt(
    id: ToolId,
    attempt: &Arc<PreparationAttempt>,
    result: Result<VerifiedTool, ToolError>,
) {
    let fenced = result.as_ref().err().is_some_and(cleanup_unproven);
    match &result {
        Ok(tool) => slot(id).progress.send_modify(|snapshot| {
            snapshot.phase = ToolPreparationPhase::Ready;
            snapshot.version = tool.version.clone();
            snapshot.error = None;
            snapshot.revision += 1;
        }),
        Err(error) => publish_failure(id, error),
    }
    let _ = attempt.result.set(result.map_err(Arc::new));
    if !fenced
        && let Ok(mut active) = slot(id).active.lock()
        && active
            .as_ref()
            .is_some_and(|value| Arc::ptr_eq(value, attempt))
    {
        *active = None;
    }
    attempt.completed.notify_waiters();
}

fn cleanup_unproven(error: &ToolError) -> bool {
    match error {
        ToolError::Cleanup(_) => true,
        ToolError::Shared(error) => cleanup_unproven(error),
        _ => false,
    }
}

pub(crate) fn report_progress(
    id: ToolId,
    phase: ToolPreparationPhase,
    received: u64,
    total: Option<u64>,
) {
    slot(id).progress.send_modify(|snapshot| {
        snapshot.phase = phase;
        snapshot.received_bytes = received;
        snapshot.total_bytes = total;
        snapshot.error = None;
        snapshot.revision += 1;
    });
}

fn publish_failure(id: ToolId, error: &ToolError) {
    let failure = preparation_failure(error);
    slot(id).progress.send_modify(|snapshot| {
        snapshot.phase = ToolPreparationPhase::Error;
        snapshot.error = Some(failure);
        snapshot.revision += 1;
    });
}

fn preparation_failure(error: &ToolError) -> ToolPreparationFailure {
    let (code, message, retryable) = match error {
        ToolError::Shared(error) => return preparation_failure(error),
        ToolError::Cancelled => (
            "PREPARATION_CANCELLED",
            "Component preparation was cancelled.",
            true,
        ),
        ToolError::Download(_) => (
            "COMPONENT_DOWNLOAD_FAILED",
            "The approved component could not be downloaded. Check your connection and try again.",
            true,
        ),
        ToolError::TooLarge => (
            "COMPONENT_SIZE_LIMIT",
            "The component exceeds its preparation size limit.",
            false,
        ),
        ToolError::InvalidAsset(_) => (
            "COMPONENT_ASSET_INVALID",
            "The component does not match the approved artifact.",
            true,
        ),
        ToolError::Verification { .. } | ToolError::InvalidOverride { .. } => (
            "COMPONENT_VERIFICATION_FAILED",
            "The component failed identity or capability verification.",
            false,
        ),
        ToolError::Changed { .. } => (
            "COMPONENT_CHANGED",
            "The prepared executable changed. Repair it and restart the application.",
            false,
        ),
        ToolError::Cleanup(_) => (
            "COMPONENT_CLEANUP_UNPROVEN",
            "The component process could not be confirmed stopped.",
            false,
        ),
        ToolError::Unavailable { .. } | ToolError::NotPrepared => (
            "COMPONENT_UNAVAILABLE",
            "The required component is not available.",
            true,
        ),
        ToolError::Extract(_) | ToolError::Io(_) => (
            "COMPONENT_CACHE_FAILED",
            "The managed component cache could not be prepared. Check disk space and permissions.",
            true,
        ),
        ToolError::Worker(_) => (
            "COMPONENT_PREPARATION_FAILED",
            "The component supplier could not complete preparation.",
            true,
        ),
    };
    ToolPreparationFailure {
        code: code.into(),
        message: message.into(),
        retryable,
    }
}

async fn admit_tool(id: ToolId, cancel: &CancellationToken) -> Result<VerifiedTool, ToolError> {
    if cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    if let Some(tool) = cached_tool(id) {
        return tool;
    }
    let (path, expected_version, digest) = supply_tool(id, cancel).await?;
    report_progress(id, ToolPreparationPhase::Verifying, 0, None);
    let version = verify(id, &path, expected_version.as_deref(), cancel).await?;
    let before_hash = fingerprint(&path)?;
    let after = hash(&path).await?;
    let fingerprint = fingerprint(&path)?;
    if after != digest || fingerprint != before_hash {
        return Err(ToolError::Changed { tool: id.as_str() });
    }
    if cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    let admitted = Admitted {
        tool: VerifiedTool {
            executable_path: path,
            version,
            digest,
        },
        fingerprint,
    };
    let tool = admitted.tool.clone();
    slot(id)
        .admitted
        .set(admitted)
        .map_err(|_| ToolError::Worker("component committed twice".into()))?;
    Ok(tool)
}

/// Pure synchronous discovery only reads an admission already verified in this process.
/// It never probes, downloads, scans an archive, or waits for an active preparation.
pub(crate) fn cached_tool(id: ToolId) -> Option<Result<VerifiedTool, ToolError>> {
    if let Err(error) = cached_admission_permit(id)? {
        return Some(Err(error));
    }
    let admitted = slot(id).admitted.get()?;
    // Filesystem work runs without the preparation-control guard, so begin/cancel
    // remain independent of a slow volume. Recheck the in-memory fence afterward.
    let validated = validate_admission(id, admitted);
    if let Err(error) = cached_admission_permit(id)? {
        return Some(Err(error));
    }
    Some(validated)
}

fn cached_admission_permit(id: ToolId) -> Option<Result<(), ToolError>> {
    let active = match slot(id).active.try_lock() {
        Ok(active) => active,
        Err(std::sync::TryLockError::WouldBlock) => return None,
        Err(std::sync::TryLockError::Poisoned(_)) => {
            return Some(Err(ToolError::Cleanup(
                "component controller ownership is unproved".into(),
            )));
        }
    };
    if let Some(Err(error)) = active.as_ref().and_then(|attempt| attempt.result.get())
        && cleanup_unproven(error)
    {
        return Some(Err(ToolError::Shared(Arc::clone(error))));
    }
    Some(Ok(()))
}

fn validate_admission(id: ToolId, admitted: &Admitted) -> Result<VerifiedTool, ToolError> {
    if fingerprint(&admitted.tool.executable_path)? != admitted.fingerprint {
        return Err(ToolError::Changed { tool: id.as_str() });
    }
    Ok(admitted.tool.clone())
}

async fn supply_tool(
    id: ToolId,
    cancel: &CancellationToken,
) -> Result<(PathBuf, Option<String>, String), ToolError> {
    if cfg!(debug_assertions)
        && let Some(raw) = std::env::var_os(id.override_variable())
    {
        let path = development_path(id, PathBuf::from(raw))?;
        let digest = hash(&path).await?;
        return Ok((path, None, digest));
    }
    let spec = specification(id);
    let asset = spec
        .asset(
            env!("NOMIFUN_RUNTIME_TARGET"),
            if id == ToolId::Bun {
                env!("NOMIFUN_RUNTIME_BUN_VARIANT")
            } else {
                "default"
            },
        )
        .ok_or_else(|| ToolError::InvalidAsset("no approved platform asset".into()))?
        .clone();
    if spec.delivery == contract::ComponentDelivery::Download {
        return crate::download::supply(id, &spec, &asset, embed::identity(id), cancel).await;
    }
    let payload = embed::embedded(id).ok_or(ToolError::Unavailable {
        tool: id.as_str(),
        override_variable: id.override_variable(),
    })?;
    let root = cache::runtime_root().ok_or_else(|| ToolError::Verification {
        tool: id.as_str(),
        reason: "no runtime cache directory is available".into(),
    })?;
    let dir = root.join(format!(
        "{}-{}-{}",
        id.as_str(),
        payload.version,
        payload.sha256
    ));
    report_progress(id, ToolPreparationPhase::Extracting, 0, None);
    let provenance = provenance_bytes(&spec, &asset, payload.sha256);
    let path = tokio::task::spawn_blocking(move || {
        let mut files = vec![crate::notices::PublicationFile {
            filename: "PROVENANCE.json",
            contents: &provenance,
        }];
        files.extend(crate::notices::files(id).iter().copied());
        extract::extract_into(
            &dir,
            payload.executable,
            payload.blob,
            payload.sha256,
            payload.version,
            false,
            &files,
        )
    })
    .await
    .map_err(|error| ToolError::Worker(error.to_string()))??;
    Ok((
        dunce::canonicalize(path)?,
        Some(payload.version.into()),
        payload.sha256.into(),
    ))
}

pub(crate) fn provenance_bytes(
    spec: &contract::ComponentSpec,
    asset: &contract::ComponentAsset,
    digest: &str,
) -> Vec<u8> {
    let notices = contract::notice_manifest()
        .components
        .into_iter()
        .find(|notice| notice.id == spec.id)
        .unwrap();
    serde_json::to_vec_pretty(&serde_json::json!({
        "component":spec.id,"version":spec.version,"source":spec.source,"package":spec.package,
        "delivery":spec.delivery,"artifact_url":asset.url,"artifact_sha256":asset.sha256,"executable_sha256":digest,
        "notices":notices.files.into_iter().map(|file| serde_json::json!({"filename":file.filename,"source":file.source,"sha256":file.sha256})).collect::<Vec<_>>()
    })).expect("valid component provenance")
}

fn development_path(id: ToolId, path: PathBuf) -> Result<PathBuf, ToolError> {
    if !path.is_absolute() || !path.is_file() {
        return Err(ToolError::InvalidOverride {
            tool: id.as_str(),
            reason: "path must be an absolute executable file".into(),
        });
    }
    dunce::canonicalize(path).map_err(|error| ToolError::InvalidOverride {
        tool: id.as_str(),
        reason: error.to_string(),
    })
}

async fn hash(path: &Path) -> Result<String, ToolError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || extract::sha256_file(&path))
        .await
        .map_err(|error| ToolError::Worker(error.to_string()))?
        .map_err(ToolError::Io)
}

#[derive(Deserialize)]
struct EngineIdentity {
    name: Option<String>,
    node: Option<String>,
    bun: Option<String>,
    #[serde(rename = "execPath")]
    exec_path: String,
}
const ENGINE_SCRIPT: &str = "process.stdout.write(JSON.stringify({name:process.release&&process.release.name,node:process.versions&&process.versions.node,bun:process.versions&&process.versions.bun,execPath:process.execPath}))";

async fn verify(
    id: ToolId,
    path: &Path,
    expected: Option<&str>,
    cancel: &CancellationToken,
) -> Result<String, ToolError> {
    let failure = |reason: String| ToolError::Verification {
        tool: id.as_str(),
        reason,
    };
    let observed = match id {
        ToolId::Bun | ToolId::Node => {
            let output = probe(id, path, &["-e", ENGINE_SCRIPT], cancel).await?;
            let identity: EngineIdentity =
                serde_json::from_slice(&output).map_err(|error| failure(error.to_string()))?;
            let version = engine_version(id, &identity).map_err(failure)?;
            if dunce::canonicalize(&identity.exec_path)? != path {
                return Err(failure(
                    "engine reported a different executable path".into(),
                ));
            }
            version
        }
        ToolId::OfficeCli => {
            let bytes = probe(id, path, &["--version"], cancel).await?;
            let version = String::from_utf8_lossy(&bytes).trim().to_string();
            // Exercise the required protocol, rather than trusting a name/help string.
            // Temporary files are private and never touch user documents.
            let temp = tempfile::tempdir()?;
            let source = temp.path().join("capability.docx");
            let output = temp.path().join("capability.html");
            probe(
                id,
                path,
                &[
                    "create",
                    source
                        .to_str()
                        .ok_or_else(|| failure("temporary path is not UTF-8".into()))?,
                ],
                cancel,
            )
            .await?;
            probe(
                id,
                path,
                &[
                    "view",
                    source.to_str().unwrap(),
                    "html",
                    "-o",
                    output.to_str().unwrap(),
                ],
                cancel,
            )
            .await?;
            let html = std::fs::read_to_string(output)?;
            if !html.to_ascii_lowercase().contains("<html") {
                return Err(failure(
                    "view html did not generate an HTML document".into(),
                ));
            }
            version
        }
    };
    let parsed = Version::parse(observed.trim_start_matches('v'))
        .map_err(|error| failure(format!("invalid version: {error}")))?;
    let spec = contract::manifest()
        .components
        .into_iter()
        .find(|spec| spec.id == id.as_str())
        .unwrap();
    if let Some(expected) = expected {
        if parsed.to_string() != expected {
            return Err(failure(format!(
                "expected version {expected}, observed {parsed}"
            )));
        }
    } else if !VersionReq::parse(&spec.development_version_requirement)
        .unwrap()
        .matches(&parsed)
    {
        return Err(failure(format!(
            "version {parsed} does not satisfy {}",
            spec.development_version_requirement
        )));
    }
    Ok(parsed.to_string())
}

fn engine_version(id: ToolId, identity: &EngineIdentity) -> Result<String, String> {
    match id {
        ToolId::Node if identity.name.as_deref() == Some("node") && identity.bun.is_none() => {
            identity
                .node
                .clone()
                .ok_or_else(|| "genuine Node version is missing".into())
        }
        ToolId::Bun => identity
            .bun
            .clone()
            .ok_or_else(|| "Bun engine identity is missing".into()),
        _ => Err(
            "the executable is not the required genuine engine (Bun cannot serve as Node)".into(),
        ),
    }
}

async fn probe(
    id: ToolId,
    path: &Path,
    args: &[&str],
    cancel: &CancellationToken,
) -> Result<Vec<u8>, ToolError> {
    if cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    let mut command = ChildProcessBuilder::clean_cli(path);
    command
        .args(args)
        .env("OFFICECLI_NO_AUTO_RESIDENT", "1")
        .env_remove("NODE_OPTIONS")
        .env_remove("BUN_OPTIONS");
    let mut child = command.spawn_managed()?;
    let stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| ToolError::Worker("probe stdout missing".into()))?;
    let stderr = child
        .child_mut()
        .stderr
        .take()
        .ok_or_else(|| ToolError::Worker("probe stderr missing".into()))?;
    let (overflow, mut exceeded) = tokio::sync::mpsc::unbounded_channel();
    let mut stdout = tokio::spawn(read_probe_output(stdout, overflow.clone()));
    let mut stderr = tokio::spawn(read_probe_output(stderr, overflow));
    let timeout = tokio::time::sleep(Duration::from_secs(20));
    tokio::pin!(timeout);
    let mut alerts_open = true;
    let outcome = loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break Err(ToolError::Cancelled),
            _ = &mut timeout => break Err(ToolError::Verification { tool:id.as_str(), reason:"capability probe timed out".into() }),
            alert = exceeded.recv(), if alerts_open => match alert {
                Some(()) => break Err(ToolError::TooLarge), None => alerts_open = false,
            },
            status = child.child_mut().wait() => break status.map_err(ToolError::Io),
        }
    };
    // Never publish cancellation or an exit until the shared lifecycle owner
    // has proved whole-tree shutdown. Its existing relay retains failed proof.
    if let Err(error) = child.shutdown().await {
        stdout.abort();
        stderr.abort();
        let _ = (&mut stdout).await;
        let _ = (&mut stderr).await;
        return Err(ToolError::Cleanup(error.to_string()));
    }
    let stdout = stdout
        .await
        .map_err(|error| ToolError::Worker(error.to_string()))??;
    let stderr = stderr
        .await
        .map_err(|error| ToolError::Worker(error.to_string()))??;
    let status = outcome?;
    if !status.success() {
        return Err(ToolError::Verification {
            tool: id.as_str(),
            reason: format!(
                "process exited with {status}: {}",
                String::from_utf8_lossy(&stderr)
                    .chars()
                    .take(1024)
                    .collect::<String>()
            ),
        });
    }
    Ok(stdout)
}

async fn read_probe_output<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    overflow: tokio::sync::mpsc::UnboundedSender<()>,
) -> Result<Vec<u8>, ToolError> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(bytes);
        }
        if bytes.len() + read > 64 * 1024 {
            let _ = overflow.send(());
            return Err(ToolError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn real_embedded_or_explicit_components_pass_their_required_capability() {
        let real_supply = std::env::var("NOMIFUN_REAL_SUPPLY_TEST").as_deref() == Ok("1");
        if real_supply && let Some(root) = std::env::var_os("NOMIFUN_REAL_SUPPLY_TEST_ROOT") {
            crate::init(root);
        }
        for id in [ToolId::Bun, ToolId::Node, ToolId::OfficeCli] {
            if !real_supply
                && embed::embedded(id).is_none()
                && std::env::var_os(id.override_variable()).is_none()
            {
                continue;
            }
            let first = resolve_tool(id).await.unwrap();
            assert!(first.executable_path.is_absolute());
            assert_eq!(first.digest.len(), 64);
            assert_eq!(first, resolve_tool(id).await.unwrap());
        }
    }
    #[test]
    fn bun_node_alias_is_rejected_even_with_node_name_and_version() {
        let identity = EngineIdentity {
            name: Some("node".into()),
            node: Some("24.3.0".into()),
            bun: Some("1.3.14".into()),
            exec_path: "/fixture/node".into(),
        };
        assert!(engine_version(ToolId::Node, &identity).is_err());
        assert_eq!(engine_version(ToolId::Bun, &identity).unwrap(), "1.3.14");
    }
    #[test]
    fn development_override_never_uses_relative_or_missing_paths() {
        assert!(development_path(ToolId::Node, PathBuf::from("node")).is_err());
        assert!(development_path(ToolId::OfficeCli, PathBuf::from("/missing/officecli")).is_err());
    }

    #[test]
    fn preparation_failures_never_echo_transport_credentials_in_observations() {
        let failure = preparation_failure(&ToolError::Download(
            "proxy https://user:secret@example.invalid/private".into(),
        ));
        assert_eq!(failure.code, "COMPONENT_DOWNLOAD_FAILED");
        assert!(failure.retryable);
        assert!(!failure.message.contains("secret"));
        assert!(!failure.message.contains("example.invalid"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn preparation_is_single_flight_retries_after_cancel_and_joins_process_cleanup() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(fixture) = std::env::var_os("NOMIFUN_PREPARATION_CONTROL_TEST_DIR") {
            let directory = dunce::canonicalize(fixture).unwrap();
            let path = directory.join("bun");
            let count = directory.join("count");
            let pid = directory.join("pid");
            let identity =
                serde_json::json!({"name":"node","node":"24.3.0","bun":"1.3.14","execPath":path});
            let script = |sleep: &str| {
                format!(
                    "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nprintf '.' >> '{}'\n/bin/sleep {sleep}\nprintf '%s' '{}'\n",
                    pid.display(),
                    count.display(),
                    identity
                )
            };
            std::fs::write(&path, script("30")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let initial = tool_preparation_snapshot(ToolId::Bun);
            let mut updates = subscribe_tool_preparation(ToolId::Bun);
            let started = begin_tool_preparation(ToolId::Bun);
            assert_eq!(started.phase, ToolPreparationPhase::Checking);
            assert!(started.revision > initial.revision);
            for _ in 0..20 {
                begin_tool_preparation(ToolId::Bun);
            }
            let waiter = tokio::spawn(prepare_tool(ToolId::Bun));
            tokio::time::timeout(Duration::from_secs(5), async {
                while !pid.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            cancel_tool_preparation(ToolId::Bun);
            assert!(
                tokio::time::timeout(Duration::from_secs(5), waiter)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
            let failed = tool_preparation_snapshot(ToolId::Bun);
            assert_eq!(failed.phase, ToolPreparationPhase::Error);
            assert_eq!(failed.error.unwrap().code, "PREPARATION_CANCELLED");
            assert_eq!(std::fs::read(&count).unwrap().len(), 1);
            let pid_value = std::fs::read_to_string(&pid).unwrap();
            assert!(
                !std::process::Command::new("/bin/kill")
                    .args(["-0", &pid_value])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            std::fs::write(&path, script("0.3")).unwrap();
            let abandoned = tokio::spawn(prepare_tool(ToolId::Bun));
            tokio::time::timeout(Duration::from_secs(5), async {
                while std::fs::read(&count).unwrap().len() < 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            abandoned.abort(); // A consumer timeout must not revoke the shared producer.
            let (first, second) =
                tokio::join!(prepare_tool(ToolId::Bun), prepare_tool(ToolId::Bun));
            assert_eq!(first.unwrap(), second.unwrap());
            assert_eq!(std::fs::read(&count).unwrap().len(), 2);
            assert_eq!(
                tool_preparation_snapshot(ToolId::Bun).phase,
                ToolPreparationPhase::Ready
            );
            assert!(updates.has_changed().unwrap());
            assert!(updates.borrow_and_update().revision > started.revision);
            let args = vec!["--version".into()];
            let asynchronous = crate::prepare_managed_command("bunx", &args)
                .await
                .unwrap()
                .unwrap();
            let cached = crate::resolve_cached_managed_command("bunx", &args)
                .unwrap()
                .unwrap();
            assert_eq!(asynchronous, cached);
            assert_eq!(cached.args, ["x", "--version"]);
            // An unproved cleanup fences even a prior admission until restart.
            let fence = Arc::new(PreparationAttempt {
                cancel: CancellationToken::new(),
                result: OnceLock::new(),
                completed: Notify::new(),
            });
            *slot(ToolId::Bun).active.lock().unwrap() = Some(Arc::clone(&fence));
            finish_attempt(
                ToolId::Bun,
                &fence,
                Err(ToolError::Shared(Arc::new(ToolError::Cleanup(
                    "unproved fixture cleanup".into(),
                )))),
            );
            let failure = tool_preparation_snapshot(ToolId::Bun);
            assert_eq!(failure.error.unwrap().code, "COMPONENT_CLEANUP_UNPROVEN");
            for _ in 0..10 {
                assert_eq!(
                    begin_tool_preparation(ToolId::Bun).phase,
                    ToolPreparationPhase::Error
                );
            }
            assert!(prepare_tool(ToolId::Bun).await.is_err());
            assert!(crate::resolve_cached_managed_command("bunx", &args).is_err());
            assert_eq!(std::fs::read(&count).unwrap().len(), 2);
            assert!(Arc::ptr_eq(
                slot(ToolId::Bun).active.lock().unwrap().as_ref().unwrap(),
                &fence
            ));
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "managed::tests::preparation_is_single_flight_retries_after_cancel_and_joins_process_cleanup"])
            .env("NOMIFUN_PREPARATION_CONTROL_TEST_DIR", directory.path()).env("NOMIFUN_BUN_PATH", directory.path().join("bun"))
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    #[test]
    fn changed_cached_executable_is_rejected_even_if_size_and_mtime_are_restored() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("node");
        std::fs::write(&path, b"approved").unwrap();
        let admitted = Admitted {
            fingerprint: fingerprint(&path).unwrap(),
            tool: VerifiedTool {
                executable_path: path.clone(),
                version: "22.22.0".into(),
                digest: extract::sha256_file(&path).unwrap(),
            },
        };
        assert!(validate_admission(ToolId::Node, &admitted).is_ok());
        std::fs::write(&path, b"modified").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(admitted.fingerprint.modified.unwrap())
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            admitted.fingerprint.size
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().ok(),
            admitted.fingerprint.modified
        );
        assert!(matches!(
            validate_admission(ToolId::Node, &admitted),
            Err(ToolError::Changed { .. })
        ));
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn cached_bun_lookup_never_prepares_or_waits_for_an_active_producer() {
        use std::os::unix::fs::PermissionsExt;
        if let Some(fixture) = std::env::var_os("NOMIFUN_BUN_ADMISSION_TEST_DIR") {
            let path = PathBuf::from(fixture).join("bun");
            assert!(matches!(crate::resolve_bun(), Err(ToolError::NotPrepared)));
            assert_eq!(
                tool_preparation_snapshot(ToolId::Bun).phase,
                ToolPreparationPhase::Idle
            );
            assert!(resolve_tool(ToolId::Bun).await.is_err());
            let canonical = dunce::canonicalize(path.parent().unwrap())
                .unwrap()
                .join("bun");
            let identity = serde_json::json!({"name":"node","node":"24.3.0","bun":"1.3.14","execPath":canonical});
            std::fs::write(
                &path,
                format!("#!/bin/sh\n/bin/sleep 0.2\nprintf '%s' '{}'\n", identity),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let started = begin_tool_preparation(ToolId::Bun);
            assert_eq!(started.phase, ToolPreparationPhase::Checking);
            let before = std::time::Instant::now();
            assert!(matches!(crate::resolve_bun(), Err(ToolError::NotPrepared)));
            assert!(before.elapsed() < Duration::from_millis(100));
            let tool = prepare_tool(ToolId::Bun).await.unwrap();
            assert_eq!(crate::resolve_bun().unwrap(), tool.executable_path);
            assert_eq!(
                tool_preparation_snapshot(ToolId::Bun).phase,
                ToolPreparationPhase::Ready
            );
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "managed::tests::cached_bun_lookup_never_prepares_or_waits_for_an_active_producer",
            ])
            .env("NOMIFUN_BUN_ADMISSION_TEST_DIR", temp.path())
            .env("NOMIFUN_BUN_PATH", temp.path().join("bun"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn wrong_same_named_office_tool_fails_required_capability() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("officecli");
        std::fs::write(&path, "#!/bin/sh\nif [ \"$1\" = '--version' ]; then echo 1.0.156; else echo 'officecli TUI requires a TTY' >&2; exit 1; fi\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            verify(ToolId::OfficeCli, &path, None, &CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("TUI requires a TTY")
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_override_reports_actual_engine_and_version() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("genuine-node-fixture");
        let canonical = dunce::canonicalize(tmp.path())
            .unwrap()
            .join("genuine-node-fixture");
        let identity = serde_json::json!({"name":"node","node":"22.22.0","execPath":canonical});
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s' '{}'\n", identity)).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let resolved = development_path(ToolId::Node, path).unwrap();
        assert_eq!(
            verify(ToolId::Node, &resolved, None, &CancellationToken::new())
                .await
                .unwrap(),
            "22.22.0"
        );
        assert_eq!(hash(&resolved).await.unwrap().len(), 64);
    }
}
