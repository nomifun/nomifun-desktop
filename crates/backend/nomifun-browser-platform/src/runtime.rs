//! Browser Resource host ports. No native handle or debugging endpoint is wire data.

use std::{
    ffi::OsStr,
    fs::Metadata,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::run_guard::{NativeInputGate, RunAdmissionError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkspaceError {
    #[error("The browser observation is out of date; observe the page again.")]
    StaleObservation,
    #[error("The target is not currently visible, enabled, stable, or reachable.")]
    NotActionable,
    #[error(
        "The browser action stopped after input was sent; observe the page before deciding what to do next."
    )]
    ActionInterrupted,
    #[error("A browser input is waiting for a website dialog response. Respond to that dialog before issuing another browser action.")]
    DialogPending,
    #[error("The requested browser action is not supported by this runtime.")]
    UnsupportedAction,
    #[error("The browser observation exceeds its size limit.")]
    ObservationLimit,
    #[error("The native browser is unavailable on this host.")]
    NativeUnavailable,
    #[error("The AgentSession browser resource has been closed.")]
    WorkspaceClosed,
    #[error("The selected browser provider does not match this resource.")]
    ProviderChanged,
    #[error("The Browser grant or resource binding does not authorize this action.")]
    ActionDenied,
    #[error("The browser tab no longer exists.")]
    TabNotFound,
    #[error("The browser tab limit has been reached.")]
    TabLimit,
    #[error("The browser target belongs to an older page or runtime.")]
    StaleTarget,
    #[error("The browser URL is not allowed.")]
    InvalidUrl,
    #[error("Upload files must be regular files inside the authorized workspace; links and special paths are not accepted.")]
    UploadPathDenied,
    #[error("The browser upload file count or byte limit has been reached.")]
    UploadLimit,
    #[error("The browser download exceeds the task file or byte limit.")]
    DownloadLimit,
    #[error("The browser download cannot be published safely into the authorized workspace.")]
    DownloadDenied,
    #[error("The native browser command failed.")]
    NativeCommandFailed,
    #[error("The Browser profile cleanup request is not an exact frozen binding set.")]
    ProfileCleanupInvalid,
    #[error("Persistent Browser profile cleanup is not configured on this host.")]
    ProfileCleanupUnavailable,
    #[error("The persistent Browser profile could not be deleted safely.")]
    ProfileCleanupFailed,
    #[error(transparent)]
    Admission(#[from] RunAdmissionError),
}

impl WorkspaceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleObservation => "BROWSER_STALE_OBSERVATION",
            Self::NotActionable => "BROWSER_NOT_ACTIONABLE",
            Self::ActionInterrupted => "BROWSER_ACTION_INTERRUPTED",
            Self::DialogPending => "BROWSER_DIALOG_PENDING",
            Self::UnsupportedAction => "BROWSER_UNSUPPORTED_ACTION",
            Self::ObservationLimit => "BROWSER_OBSERVATION_LIMIT",
            Self::NativeUnavailable => "BROWSER_NATIVE_SURFACE_UNAVAILABLE",
            Self::WorkspaceClosed => "BROWSER_RESOURCE_CLOSED",
            Self::ProviderChanged => "BROWSER_PROVIDER_CHANGED",
            Self::ActionDenied => "BROWSER_ACTION_DENIED",
            Self::TabNotFound => "BROWSER_TAB_NOT_FOUND",
            Self::TabLimit => "BROWSER_TAB_LIMIT",
            Self::StaleTarget => "BROWSER_STALE_TARGET",
            Self::InvalidUrl => "BROWSER_INVALID_URL",
            Self::UploadPathDenied => "BROWSER_UPLOAD_PATH_DENIED",
            Self::UploadLimit => "BROWSER_UPLOAD_LIMIT",
            Self::DownloadLimit => "BROWSER_DOWNLOAD_LIMIT",
            Self::DownloadDenied => "BROWSER_DOWNLOAD_DENIED",
            Self::NativeCommandFailed => "BROWSER_NATIVE_COMMAND_FAILED",
            Self::ProfileCleanupInvalid => "BROWSER_PROFILE_CLEANUP_INVALID",
            Self::ProfileCleanupUnavailable => "BROWSER_PROFILE_CLEANUP_UNAVAILABLE",
            Self::ProfileCleanupFailed => "BROWSER_PROFILE_CLEANUP_FAILED",
            Self::Admission(error) => match error {
                RunAdmissionError::Busy => "BROWSER_RUN_BUSY",
                RunAdmissionError::StaleRun => "BROWSER_STALE_RUN",
                RunAdmissionError::Cancelled => "BROWSER_OPERATION_CANCELLED",
                RunAdmissionError::InputGateFailed => "BROWSER_INPUT_GATE_FAILED",
                RunAdmissionError::UserInputLocked => "BROWSER_USER_INPUT_LOCKED",
                RunAdmissionError::WorkerFailed => "BROWSER_WORKER_FAILED",
            },
        }
    }
}

/// Canonical identity of one Browser Resource. It is supplied by the
/// authenticated AgentSession owner, never by model or page input.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrowserResourceKey {
    pub principal_id: String,
    pub agent_session_id: String,
    pub resource_binding_id: String,
}

impl BrowserResourceKey {
    pub(crate) fn validate_profile_identity(&self) -> Result<(), WorkspaceError> {
        for value in [
            self.principal_id.as_str(),
            self.agent_session_id.as_str(),
            self.resource_binding_id.as_str(),
        ] {
            if !is_bounded_profile_identity(value) {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserProfilePersistence {
    Persistent,
    Ephemeral,
}

/// Frozen managed-Browser binding identity supplied by the authenticated
/// AgentSession owner during deletion. It contains no caller-selected path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserProfileBinding {
    resource_binding_id: String,
    persistence: BrowserProfilePersistence,
}

impl BrowserProfileBinding {
    pub fn new(
        resource_binding_id: impl Into<String>,
        persistence: BrowserProfilePersistence,
    ) -> Result<Self, WorkspaceError> {
        let resource_binding_id = resource_binding_id.into();
        if !is_bounded_profile_identity(&resource_binding_id) {
            return Err(WorkspaceError::ProfileCleanupInvalid);
        }
        Ok(Self {
            resource_binding_id,
            persistence,
        })
    }

    pub fn persistent(resource_binding_id: impl Into<String>) -> Result<Self, WorkspaceError> {
        Self::new(resource_binding_id, BrowserProfilePersistence::Persistent)
    }

    pub fn ephemeral(resource_binding_id: impl Into<String>) -> Result<Self, WorkspaceError> {
        Self::new(resource_binding_id, BrowserProfilePersistence::Ephemeral)
    }

    pub fn resource_binding_id(&self) -> &str {
        &self.resource_binding_id
    }

    pub const fn persistence(&self) -> BrowserProfilePersistence {
        self.persistence
    }
}

pub(crate) fn is_bounded_profile_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserProfile {
    /// Application-owned, canonical v2 directory. Browser page input cannot select it.
    Persistent(PathBuf),
    Ephemeral,
}

impl BrowserProfile {
    /// Identity comes from the authenticated host, never a page or tool argument.
    /// A project path is not a browser identity: AgentSessions never share data.
    /// No old profile is read or migrated, and temporary work stays ephemeral.
    pub fn for_agent_session(
        data_dir: &std::path::Path,
        key: &BrowserResourceKey,
        ephemeral: bool,
    ) -> Self {
        use sha2::{Digest, Sha256};
        if ephemeral {
            return Self::Ephemeral;
        }
        let mut digest = Sha256::new();
        digest.update(b"nomifun.browser.agent-session-profile.v1\0");
        for value in [
            &key.principal_id,
            &key.agent_session_id,
            &key.resource_binding_id,
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        Self::Persistent(
            data_dir.join("browser-v3").join("agent-sessions")
                .join(format!("{:x}", digest.finalize())),
        )
    }
}

/// Host-owned root used only to derive and delete canonical Browser profiles.
/// Deletion APIs accept this opaque owner once at service composition and
/// never accept a model-, route-, or caller-supplied filesystem path.
#[derive(Clone, Debug)]
pub struct BrowserProfileStore {
    data_dir: PathBuf,
}

impl BrowserProfileStore {
    pub fn new(data_dir: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let data_dir = data_dir.into();
        let metadata = std::fs::symlink_metadata(&data_dir)
            .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
        if !metadata.is_dir() || path_is_link_or_reparse(&metadata) {
            return Err(WorkspaceError::ProfileCleanupUnavailable);
        }
        let data_dir = std::fs::canonicalize(data_dir)
            .map_err(|_| WorkspaceError::ProfileCleanupUnavailable)?;
        Ok(Self { data_dir })
    }

    pub fn profile_for(
        &self,
        key: &BrowserResourceKey,
        persistence: BrowserProfilePersistence,
    ) -> Result<BrowserProfile, WorkspaceError> {
        key.validate_profile_identity()?;
        Ok(BrowserProfile::for_agent_session(
            &self.data_dir,
            key,
            persistence == BrowserProfilePersistence::Ephemeral,
        ))
    }

    pub(crate) fn delete_persistent_profile(
        &self,
        key: &BrowserResourceKey,
    ) -> Result<(), WorkspaceError> {
        let BrowserProfile::Persistent(profile) =
            self.profile_for(key, BrowserProfilePersistence::Persistent)?
        else {
            unreachable!("persistent policy always derives a persistent profile")
        };
        delete_exact_profile_tree(&self.data_dir, &profile)
            .map_err(|_| WorkspaceError::ProfileCleanupFailed)
    }
}

fn delete_exact_profile_tree(data_dir: &Path, profile: &Path) -> io::Result<()> {
    let expected_root = data_dir.join("browser-v3").join("agent-sessions");
    if profile.parent() != Some(expected_root.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Browser profile is outside the canonical profile root",
        ));
    }
    let profile_name = profile.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Browser profile has no identity")
    })?;
    let Some(browser_root) = plain_child(data_dir, OsStr::new("browser-v3"))? else {
        return Ok(());
    };
    let Some(session_root) =
        plain_child(&browser_root, OsStr::new("agent-sessions"))?
    else {
        return Ok(());
    };
    let Some(profile) = plain_child(&session_root, profile_name)? else {
        return Ok(());
    };
    validate_plain_profile_tree(&profile)?;
    std::fs::remove_dir_all(&profile)?;
    match std::fs::symlink_metadata(&profile) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::Other,
            "Browser profile still exists after deletion",
        )),
        Err(error) => Err(error),
    }
}

fn plain_child(parent: &Path, name: &OsStr) -> io::Result<Option<PathBuf>> {
    require_plain_directory(parent)?;
    let path = parent.join(name);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !path_is_link_or_reparse(&metadata) => {
            Ok(Some(path))
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Browser profile storage is not a plain directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn require_plain_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || path_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Browser profile storage is not a plain directory",
        ));
    }
    Ok(())
}

fn validate_plain_profile_tree(path: &Path) -> io::Result<()> {
    require_plain_directory(path)?;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if path_is_link_or_reparse(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Browser profile contains a link or reparse point",
            ));
        }
        if metadata.is_dir() {
            validate_plain_profile_tree(&entry.path())?;
        } else if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Browser profile contains a special filesystem entry",
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn path_is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn path_is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn profile(principal: &str, session: &str, binding: &str) -> BrowserProfile {
        BrowserProfile::for_agent_session(std::path::Path::new("owned-data"), &BrowserResourceKey {
            principal_id: principal.into(),
            agent_session_id: session.into(),
            resource_binding_id: binding.into(),
        }, false)
    }

    #[test]
    fn agent_session_profile_is_stable_and_isolates_principals_sessions_and_bindings() {
        assert_eq!(profile("alice", "one", "binding"), profile("alice", "one", "binding"));
        assert_ne!(profile("alice", "one", "binding"), profile("alice", "two", "binding"));
        assert_ne!(profile("alice", "one", "binding"), profile("bob", "one", "binding"));
        assert_ne!(profile("alice", "one", "binding-a"), profile("alice", "one", "binding-b"));
        assert_ne!(profile("ab", "c", "d"), profile("a", "bc", "d"));
    }

    #[test]
    fn agent_session_profile_is_opaque_and_never_selects_a_legacy_directory() {
        let BrowserProfile::Persistent(path) = profile("../用户", "C:\\outside/../../secret", "binding") else {
            panic!("persistent AgentSession");
        };
        assert_eq!(path.parent().unwrap(), std::path::Path::new("owned-data/browser-v3/agent-sessions"));
        let hash = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn temporary_agent_session_never_selects_persistent_storage() {
        let key = BrowserResourceKey {
            principal_id: "alice".into(),
            agent_session_id: "one".into(),
            resource_binding_id: "binding".into(),
        };
        assert_eq!(BrowserProfile::for_agent_session(std::path::Path::new("owned-data"), &key, true), BrowserProfile::Ephemeral);
    }
}

#[derive(Clone, Debug)]
pub struct CreateBrowserRuntime {
    pub key: BrowserResourceKey,
    pub runtime_generation: u64,
    pub profile: BrowserProfile,
    /// New tabs inherit the runtime gate before they are made visible.
    pub user_input_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserTabTarget {
    pub tab_id: String,
    pub runtime_generation: u64,
    pub document_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTabLifecycle {
    Loading,
    Ready,
    Failed,
    Crashed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserTabSnapshot {
    pub target: BrowserTabTarget,
    pub title: String,
    pub url: String,
    pub lifecycle: BrowserTabLifecycle,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub blocked_permissions: Vec<String>,
    pub permission_requests: Vec<BrowserPermissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_dialog: Option<BrowserDialog>,
    /// Agent-only observation data, not a renderer dashboard payload.
    #[serde(skip_serializing)]
    pub diagnostics: BrowserDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserPermissionRequest {
    pub request_id: String,
    pub kind: String,
    pub origin: String,
}

/// Bounded, untrusted page output; never a protocol object or execution handle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BrowserDiagnostics {
    pub entries: Vec<BrowserDiagnostic>,
    pub dropped: u64,
    /// Collection lost its native lifecycle proof; retained entries are only
    /// partial history, not evidence that the current page has no errors.
    pub unavailable: bool,
}
impl BrowserDiagnostics {
    pub fn clear_page(&mut self) {
        self.entries.clear();
        self.dropped=0;
        // Navigation does not recreate a failed native event subscription.
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserDiagnostic {
    pub id: u64,
    pub kind: String,
    pub level: String,
    pub message: String,
    pub source_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserRuntimeSnapshot {
    pub runtime_generation: u64,
    pub revision: u64,
    pub active_tab_id: Option<String>,
    pub tabs: Vec<BrowserTabSnapshot>,
    pub downloads: Vec<BrowserDownloadSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserDownloadState { Choosing, InProgress, Cancelling, Completed, Cancelled, Failed }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserDownloadSnapshot {
    pub id: String,
    pub tab_id: String,
    pub filename: String,
    pub state: BrowserDownloadState,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub can_cancel: bool,
}

/// The same typed commands serve the human toolbar and the Agent's tab actions.
/// Invocation authority lives outside this value in BrowserRunCoordinator.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserTabCommand {
    Create {
        url: String,
    },
    Activate {
        target: BrowserTabTarget,
    },
    Close {
        target: BrowserTabTarget,
    },
    /// User-only close of this runtime's pages, not its Profile or process owner.
    CloseAll { runtime_generation: u64 },
    /// User-only OS Downloads folder handoff. The host resolves the path.
    OpenDownloads { runtime_generation: u64 },
    /// User-only, confirmed AgentSession-resource site data removal; closes pages.
    ClearSiteData { runtime_generation: u64 },
    Navigate {
        target: BrowserTabTarget,
        url: String,
    },
    Back {
        target: BrowserTabTarget,
    },
    Forward {
        target: BrowserTabTarget,
    },
    Reload {
        target: BrowserTabTarget,
    },
    StopLoading {
        target: BrowserTabTarget,
    },
    /// Explicit user handoff of the current native URL; never an Agent action.
    OpenExternal { target: BrowserTabTarget },
    /// User-only cancellation of an owned download; no destination path input.
    CancelDownload { target: BrowserTabTarget, download_id: String },
    /// Human-only decision for an exact pending native request. Never an Agent capability.
    Permission {
        target: BrowserTabTarget,
        request_id: String,
        allow: bool,
    },
    /// Ordinary website dialog, not a website permission or an Agent takeover.
    /// Agent replies use BrowserAutomationPort under their current run guard.
    Dialog {
        target: BrowserTabTarget,
        request_id: String,
        accept: bool,
        text: Option<String>,
    },
}

impl BrowserTabCommand {
    pub fn target(&self) -> Option<&BrowserTabTarget> {
        match self {
            Self::Create { .. } | Self::CloseAll { .. } | Self::OpenDownloads { .. } | Self::ClearSiteData { .. } => None,
            Self::Activate { target }
            | Self::Close { target }
            | Self::Navigate { target, .. }
            | Self::Back { target }
            | Self::Forward { target }
            | Self::Reload { target }
            | Self::StopLoading { target }
            | Self::OpenExternal { target }
            | Self::CancelDownload { target, .. }
            | Self::Permission { target, .. }
            | Self::Dialog { target, .. } => Some(target),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSurfaceBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BrowserSurfaceBounds {
    pub fn is_valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            && self.x >= 0.0
            && self.y >= 0.0
            && self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 32_768.0
            && self.y + self.height <= 32_768.0
    }
}

#[async_trait]
pub trait BrowserRuntimeFactory: Send + Sync {
    async fn create(
        &self,
        request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError>;

    /// Stop process-wide native browser infrastructure after every runtime
    /// created by this factory has acknowledged destruction.
    ///
    /// Most hosts do not need a separate process-wide shutdown phase. macOS
    /// CEF does: `cef_shutdown` must run on the application event thread after
    /// the last child NSView has closed and before the desktop process exits.
    async fn shutdown(&self) -> Result<(), WorkspaceError> {
        Ok(())
    }
}

#[async_trait]
pub trait BrowserRuntime: NativeInputGate + Send + Sync {
    fn changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        None
    }
    fn automation(&self) -> Option<&dyn BrowserAutomationPort> {
        None
    }
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort>;
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError>;
    /// Must await native command settlement, even if cancellation arrives.
    async fn execute(
        &self,
        command: BrowserTabCommand,
        cancel: CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError>;
    /// Closing acknowledges native destruction, not just a posted close request.
    async fn close(&self) -> Result<(), WorkspaceError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserElementRef {
    pub target: BrowserTabTarget,
    pub observation_generation: u64,
    pub ref_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserElement {
    pub reference: BrowserElementRef,
    pub role: String,
    pub name: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserObservation {
    pub target: BrowserTabTarget,
    pub observation_generation: u64,
    pub content: String,
    pub elements: Vec<BrowserElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_dialog: Option<BrowserDialog>,
    /// Explicitly report frame coverage; never imply that an unobserved frame was inspected.
    pub unobserved_frames: usize,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

fn single_click() -> u8 {
    1
}

// Text input deliberately has no Debug implementation: typed secrets must not
// reach diagnostics through derived action logging.
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAction {
    Click {
        element: BrowserElementRef,
        #[serde(default)]
        button: BrowserMouseButton,
        #[serde(default = "single_click")]
        click_count: u8,
    },
    Hover {
        element: BrowserElementRef,
    },
    Type {
        element: BrowserElementRef,
        text: String,
    },
    Press {
        element: BrowserElementRef,
        keys: String,
    },
    Select {
        element: BrowserElementRef,
        /// Exact accessible option labels; an empty set clears a multi-select.
        labels: Vec<String>,
    },
    Scroll {
        element: BrowserElementRef,
        delta_x: f64,
        delta_y: f64,
    },
    Drag {
        from: BrowserElementRef,
        to: BrowserElementRef,
    },
}

impl BrowserAction {
    pub fn click(element: BrowserElementRef) -> Self {
        Self::Click {
            element,
            button: BrowserMouseButton::Left,
            click_count: 1,
        }
    }

    pub fn element(&self) -> &BrowserElementRef {
        match self {
            Self::Click { element, .. }
            | Self::Hover { element }
            | Self::Type { element, .. }
            | Self::Press { element, .. }
            | Self::Select { element, .. }
            | Self::Scroll { element, .. } => element,
            Self::Drag { from, .. } => from,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionFidelity {
    BrowserInput,
    BrowserProtocol,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserActionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<crate::downloads::BrowserDownloadArtifact>,
    pub target: BrowserTabTarget,
    pub interaction_fidelity: InteractionFidelity,
    #[serde(flatten)]
    pub outcome: BrowserActionOutcome,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrowserActionOutcome {
    Completed,
    /// A dialog reply completed a retained developer evaluation, not an input.
    EvaluationResult { evaluation: BrowserEvaluationResult },
    /// The input still belongs to the native host; this is not completion.
    AwaitingDialog { dialog: BrowserDialog },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserDialogKind { Alert, Confirm, Prompt, BeforeUnload }

/// Bounded untrusted page content, never instructions or execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserDialog {
    pub request_id: String,
    pub target: BrowserTabTarget,
    pub kind: BrowserDialogKind,
    pub message: String,
    pub default_text: String,
    pub origin: String,
    pub text_truncated: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserDialogReply {
    pub target: BrowserTabTarget,
    pub request_id: String,
    pub accept: bool,
    pub text: Option<String>,
}

/// One explicit viewport capture for Agent observation, never the UI surface.
#[derive(Clone)]
pub struct BrowserScreenshot {
    pub target: BrowserTabTarget,
    pub width: u32,
    pub height: u32,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub png_base64: String,
}

/// Explicit developer script execution, never a browser-input success receipt.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserEvaluation {
    pub target: BrowserTabTarget,
    pub expression: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserEvaluationResult {
    pub target: BrowserTabTarget,
    pub execution_kind: &'static str,
    #[serde(flatten)]
    pub outcome: BrowserEvaluationOutcome,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrowserEvaluationOutcome {
    Completed { value: serde_json::Value },
    ScriptError { message: String },
    AwaitingDialog { dialog: BrowserDialog },
}

#[async_trait]
pub trait BrowserAutomationPort: Send + Sync {
    async fn evaluate(&self, _request: BrowserEvaluation, _cancel: CancellationToken)
        -> Result<BrowserEvaluationResult, WorkspaceError> { Err(WorkspaceError::UnsupportedAction) }
    async fn download(&self, _element: BrowserElementRef, _file: Arc<crate::downloads::PreparedBrowserDownload>, _cancel: CancellationToken)
        -> Result<BrowserActionResult, WorkspaceError> { Err(WorkspaceError::UnsupportedAction) }
    async fn respond_dialog(&self, _reply: BrowserDialogReply, _cancel: CancellationToken)
        -> Result<BrowserActionResult, WorkspaceError> { Err(WorkspaceError::UnsupportedAction) }
    async fn upload(
        &self,
        _element: BrowserElementRef,
        _files: Arc<crate::uploads::PreparedBrowserUpload>,
        _cancel: CancellationToken,
    ) -> Result<BrowserActionResult, WorkspaceError> { Err(WorkspaceError::UnsupportedAction) }
    async fn screenshot(&self, tab_id: Option<String>, cancel: CancellationToken) -> Result<BrowserScreenshot, WorkspaceError>;
    async fn observe(
        &self,
        tab_id: Option<String>,
        cancel: CancellationToken,
    ) -> Result<BrowserObservation, WorkspaceError>;
    async fn act(
        &self,
        action: BrowserAction,
        cancel: CancellationToken,
    ) -> Result<BrowserActionResult, WorkspaceError>;
}

#[async_trait]
pub trait BrowserNativeSurfacePort: Send + Sync {
    /// Layout-only cancellation; it never cancels Agent input. Implementations
    /// must recheck it before native UI mutations, including queued UI dispatch.
    async fn set_surface(
        &self,
        bounds: BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
    ) -> Result<(), WorkspaceError>;
}
