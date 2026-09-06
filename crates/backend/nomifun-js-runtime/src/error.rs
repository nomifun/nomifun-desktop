use std::path::PathBuf;

use thiserror::Error;

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
    #[error("Node Runtime executable identity mismatch: requested {requested}, observed {observed}")]
    ExecutableIdentityMismatch { requested: String, observed: String },
    #[error("Node Runtime selection has no pending candidate")]
    NoPendingCandidate,
    #[error("Node Runtime switch already has a pending candidate")]
    SwitchAlreadyPending,
    #[error("Node Runtime validation does not match the pending candidate")]
    CandidateMismatch,
    #[error("Node Runtime candidate has not completed validation")]
    ValidationRequired,
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
    #[error("managed Node archive digest mismatch: expected {expected}, observed {observed}")]
    ArchiveDigestMismatch { expected: String, observed: String },
    #[error("managed Node archive is invalid: {0}")]
    InvalidArchive(String),
    #[error("managed Node filesystem operation failed at {path}: {reason}")]
    ManagedFilesystem { path: PathBuf, reason: String },
}
