use std::path::PathBuf;

use thiserror::Error;

pub const ERR_RUNTIME_INVALID_INPUT: &str = "JAVASCRIPT_RUNTIME_INVALID_INPUT";
pub const ERR_RUNTIME_NOT_FOUND: &str = "JAVASCRIPT_RUNTIME_NOT_FOUND";
pub const ERR_RUNTIME_STALE: &str = "JAVASCRIPT_RUNTIME_STALE";
pub const ERR_RUNTIME_BUSY: &str = "JAVASCRIPT_RUNTIME_BUSY";
pub const ERR_RUNTIME_NOT_COVERED: &str = "JAVASCRIPT_RUNTIME_NOT_COVERED";
pub const ERR_RUNTIME_FOUNDATION_FAILED: &str =
    "JAVASCRIPT_RUNTIME_FOUNDATION_FAILED";
pub const ERR_RUNTIME_NON_RECOMMENDED_CONFIRMATION: &str =
    "JAVASCRIPT_RUNTIME_NON_RECOMMENDED_CONFIRMATION_REQUIRED";
pub const ERR_RUNTIME_DOWNLOAD_OFFER_STALE: &str =
    "JAVASCRIPT_RUNTIME_DOWNLOAD_OFFER_STALE";
pub const ERR_RUNTIME_DOWNLOAD_RUNNING: &str =
    "JAVASCRIPT_RUNTIME_DOWNLOAD_RUNNING";
pub const ERR_RUNTIME_UNAVAILABLE: &str = "JAVASCRIPT_RUNTIME_UNAVAILABLE";
pub const ERR_RUNTIME_INTERNAL: &str = "JAVASCRIPT_RUNTIME_INTERNAL";

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RuntimeSelectionStoreError {
    #[error("Runtime selection revision conflict: {0}")]
    Conflict(String),
    #[error("persisted Runtime selection is invalid: {0}")]
    Corrupt(String),
    #[error("Runtime selection store is unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Error)]
pub enum JavaScriptRuntimeError {
    #[error("Node Runtime path is not absolute: {0}")]
    RelativePath(PathBuf),
    #[error("Node Runtime executable is unavailable: {path}: {reason}")]
    ExecutableUnavailable { path: PathBuf, reason: String },
    #[error("Node Runtime probe timed out: {0}")]
    ProbeTimeout(PathBuf),
    #[error("Node Runtime probe returned invalid output: {0}")]
    InvalidProbeOutput(String),
    #[error(
        "Node Runtime executable identity mismatch: requested {requested}, observed {observed}"
    )]
    ExecutableIdentityMismatch { requested: String, observed: String },
    #[error("Node Runtime selection has no pending candidate")]
    NoPendingCandidate,
    #[error("Node Runtime switch already has a pending candidate")]
    SwitchAlreadyPending,
    #[error("Node Runtime validation does not match the pending candidate")]
    CandidateMismatch,
    #[error("Node Runtime candidate has not completed validation")]
    ValidationRequired,
    #[error("Node Runtime candidate is not in the server probe inventory")]
    CandidateUnavailable,
    #[error("Node Runtime candidate changed after it was probed")]
    CandidateStale,
    #[error(
        "Node Runtime selection revision changed: expected {expected}, observed {observed}"
    )]
    SelectionRevisionConflict { expected: u64, observed: u64 },
    #[error("Node Runtime selected identity does not match the current selection")]
    SelectedRuntimeMismatch,
    #[error("Node Runtime requires one-time non-recommended confirmation")]
    NonRecommendedConfirmationRequired,
    #[error("Node Runtime switch is busy: {0}")]
    SwitchBusy(String),
    #[error("Node Runtime switch is not covered by the current coordinator: {0}")]
    SwitchNotCovered(String),
    #[error("Node Runtime foundation validation failed: {0}")]
    FoundationValidationFailed(String),
    #[error("managed Node download offer changed")]
    DownloadOfferStale,
    #[error("managed Node download is already running")]
    DownloadAlreadyRunning,
    #[error("Node Runtime selection persistence failed: {0}")]
    SelectionStore(#[from] RuntimeSelectionStoreError),
    #[error("Node Runtime contract is invalid: {0}")]
    Contract(String),
    #[error("managed Node download approval is invalid: {0}")]
    InvalidDownloadApproval(String),
    #[error("managed Node is not implemented for Runtime target {0}")]
    ManagedTargetUnsupported(String),
    #[error("official Node release metadata is unavailable: {0}")]
    ReleaseMetadata(String),
    #[error("official Node release has no archive for {0}")]
    ReleaseArchiveUnavailable(String),
    #[error("managed Node download exceeds {limit_bytes} bytes")]
    DownloadTooLarge { limit_bytes: u64 },
    #[error(
        "managed Node archive digest mismatch: expected {expected}, observed {observed}"
    )]
    ArchiveDigestMismatch { expected: String, observed: String },
    #[error("managed Node archive is invalid: {0}")]
    InvalidArchive(String),
    #[error("managed Node filesystem operation failed at {path}: {reason}")]
    ManagedFilesystem { path: PathBuf, reason: String },
}

impl JavaScriptRuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RelativePath(_)
            | Self::InvalidProbeOutput(_)
            | Self::InvalidDownloadApproval(_)
            | Self::Contract(_) => ERR_RUNTIME_INVALID_INPUT,
            Self::CandidateUnavailable | Self::NoPendingCandidate => {
                ERR_RUNTIME_NOT_FOUND
            }
            Self::SelectionRevisionConflict { .. }
            | Self::SelectedRuntimeMismatch
            | Self::CandidateMismatch
            | Self::CandidateStale
            | Self::SwitchAlreadyPending
            | Self::ValidationRequired
            | Self::SelectionStore(RuntimeSelectionStoreError::Conflict(_)) => {
                ERR_RUNTIME_STALE
            }
            Self::SwitchBusy(_) => ERR_RUNTIME_BUSY,
            Self::SwitchNotCovered(_) => ERR_RUNTIME_NOT_COVERED,
            Self::FoundationValidationFailed(_) => {
                ERR_RUNTIME_FOUNDATION_FAILED
            }
            Self::NonRecommendedConfirmationRequired => {
                ERR_RUNTIME_NON_RECOMMENDED_CONFIRMATION
            }
            Self::DownloadOfferStale => ERR_RUNTIME_DOWNLOAD_OFFER_STALE,
            Self::DownloadAlreadyRunning => ERR_RUNTIME_DOWNLOAD_RUNNING,
            Self::ExecutableUnavailable { .. }
            | Self::ProbeTimeout(_)
            | Self::ExecutableIdentityMismatch { .. }
            | Self::ManagedTargetUnsupported(_)
            | Self::ReleaseMetadata(_)
            | Self::ReleaseArchiveUnavailable(_)
            | Self::DownloadTooLarge { .. }
            | Self::ArchiveDigestMismatch { .. }
            | Self::InvalidArchive(_)
            | Self::ManagedFilesystem { .. }
            | Self::SelectionStore(RuntimeSelectionStoreError::Unavailable(_)) => {
                ERR_RUNTIME_UNAVAILABLE
            }
            Self::SelectionStore(RuntimeSelectionStoreError::Corrupt(_)) => {
                ERR_RUNTIME_INTERNAL
            }
        }
    }
}
