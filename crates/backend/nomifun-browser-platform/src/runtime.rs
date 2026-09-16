//! Browser Workspace v2 host ports. No native handle or debugging endpoint is wire data.

use std::{path::PathBuf, sync::Arc};

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
    #[error("The conversation browser has been closed.")]
    WorkspaceClosed,
    #[error("The selected browser provider does not match this workspace.")]
    ProviderChanged,
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
            Self::WorkspaceClosed => "BROWSER_WORKSPACE_CLOSED",
            Self::ProviderChanged => "BROWSER_PROVIDER_CHANGED",
            Self::TabNotFound => "BROWSER_TAB_NOT_FOUND",
            Self::TabLimit => "BROWSER_TAB_LIMIT",
            Self::StaleTarget => "BROWSER_STALE_TARGET",
            Self::InvalidUrl => "BROWSER_INVALID_URL",
            Self::UploadPathDenied => "BROWSER_UPLOAD_PATH_DENIED",
            Self::UploadLimit => "BROWSER_UPLOAD_LIMIT",
            Self::DownloadLimit => "BROWSER_DOWNLOAD_LIMIT",
            Self::DownloadDenied => "BROWSER_DOWNLOAD_DENIED",
            Self::NativeCommandFailed => "BROWSER_NATIVE_COMMAND_FAILED",
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

/// Supplied by the authenticated application composition, never from tool JSON.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrowserWorkspaceKey {
    pub user_id: String,
    pub conversation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserProfile {
    /// Application-owned, canonical v2 directory. Browser page input cannot select it.
    Persistent(PathBuf),
    Ephemeral,
}

impl BrowserProfile {
    /// Identity comes from the authenticated host, never a page or tool argument.
    /// A project path is not a browser identity: conversations never share data.
    /// No old profile is read or migrated, and temporary work stays ephemeral.
    pub fn for_conversation(
        data_dir: &std::path::Path,
        key: &BrowserWorkspaceKey,
        temporary: bool,
    ) -> Self {
        use sha2::{Digest, Sha256};
        if temporary {
            return Self::Ephemeral;
        }
        let mut digest = Sha256::new();
        digest.update(b"nomifun.browser.conversation-profile.v1\0");
        for value in [&key.user_id, &key.conversation_id] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
        Self::Persistent(
            data_dir.join("browser-v2").join("conversations")
                .join(format!("{:x}", digest.finalize())),
        )
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    fn profile(user: &str, conversation: &str) -> BrowserProfile {
        BrowserProfile::for_conversation(std::path::Path::new("owned-data"), &BrowserWorkspaceKey {
            user_id: user.into(), conversation_id: conversation.into(),
        }, false)
    }

    #[test]
    fn conversation_profile_is_stable_and_isolates_users_and_conversations() {
        assert_eq!(profile("alice", "one"), profile("alice", "one"));
        assert_ne!(profile("alice", "one"), profile("alice", "two"));
        assert_ne!(profile("alice", "one"), profile("bob", "one"));
        assert_ne!(profile("ab", "c"), profile("a", "bc"));
    }

    #[test]
    fn conversation_profile_is_opaque_and_never_selects_a_legacy_directory() {
        let BrowserProfile::Persistent(path) = profile("../用户", "C:\\outside/../../secret") else {
            panic!("persistent conversation");
        };
        assert_eq!(path.parent().unwrap(), std::path::Path::new("owned-data/browser-v2/conversations"));
        let hash = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn temporary_conversation_never_selects_persistent_storage() {
        let key = BrowserWorkspaceKey { user_id: "alice".into(), conversation_id: "one".into() };
        assert_eq!(BrowserProfile::for_conversation(std::path::Path::new("owned-data"), &key, true), BrowserProfile::Ephemeral);
    }
}

#[derive(Clone, Debug)]
pub struct CreateBrowserRuntime {
    pub key: BrowserWorkspaceKey,
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
    /// User-only, confirmed conversation-wide site data removal; closes pages.
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
