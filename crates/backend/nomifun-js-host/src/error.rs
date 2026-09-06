use nomifun_agent_contracts::{CanonicalErrorCode, CorrelationId};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum JavaScriptHostError {
    #[error("JavaScript Host configuration is invalid: {0}")]
    InvalidConfiguration(String),
    #[error("JavaScript Host contract rejected the value: {0}")]
    Contract(String),
    #[error("Node Runtime verification failed: {0}")]
    RuntimeVerification(String),
    #[error("failed to start JavaScript Host: {0}")]
    Spawn(String),
    #[error("JavaScript Host Hello timed out")]
    HelloTimeout,
    #[error("JavaScript Host Hello was rejected: {0}")]
    HelloRejected(String),
    #[error("JavaScript Host generation {generation} failed: {reason}")]
    HostFailure { generation: u64, reason: String },
    #[error("JavaScript Host generation {generation} is stopping")]
    HostStopping { generation: u64 },
    #[error(
        "JavaScript Host generation mismatch: expected {expected}, observed {observed}"
    )]
    GenerationMismatch { expected: u64, observed: u64 },
    #[error("JavaScript Host generation {generation} is not quiescent")]
    NotQuiescent { generation: u64 },
    #[error("Plugin Mount {0} is not resident in the active Host generation")]
    MountNotResident(String),
    #[error("Plugin contribution target does not match the resident Mount")]
    TargetMismatch,
    #[error("immutable Plugin module is invalid: {0}")]
    InvalidModule(String),
    #[error(
        "JavaScript Host request {request_id:?} failed with {code:?}: {message}"
    )]
    RequestFailed {
        request_id: CorrelationId,
        code: CanonicalErrorCode,
        message: String,
        retryable: bool,
    },
    #[error("JavaScript Host request channel closed before completion")]
    RequestChannelClosed,
}
