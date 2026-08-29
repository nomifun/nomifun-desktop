use nomifun_agent_contracts::{CanonicalDigestError, CanonicalErrorCode, SESSION_DELETED};
use thiserror::Error;

pub const INVALID_SESSION_EVENT: &str = "INVALID_SESSION_EVENT";
pub const SESSION_NOT_FOUND: &str = "SESSION_NOT_FOUND";
pub const IDEMPOTENCY_CONFLICT: &str = "IDEMPOTENCY_CONFLICT";
pub const INVALID_SESSION: &str = "INVALID_SESSION";
pub const INVALID_PAYLOAD: &str = "INVALID_PAYLOAD";

#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("canonical digest error: {0}")]
    Digest(#[from] CanonicalDigestError),
    #[error("{code:?}: {message}")]
    Contract {
        code: CanonicalErrorCode,
        message: String,
    },
    #[error("{SESSION_NOT_FOUND}: {0}")]
    NotFound(String),
    #[error("{SESSION_DELETED}: {0}")]
    Deleted(String),
    #[error("{IDEMPOTENCY_CONFLICT}: {0}")]
    IdempotencyConflict(String),
    #[error("{INVALID_SESSION_EVENT}: {0}")]
    InvalidEvent(String),
    #[error("{INVALID_PAYLOAD}: {0}")]
    InvalidPayload(String),
    #[error("{INVALID_SESSION}: {0}")]
    InvalidSession(String),
    #[error(
        "runtime sequence gap for {runtime_binding_id}: committed={committed_producer_seq}, expected={expected}, actual={actual}"
    )]
    RuntimeSequenceGap {
        runtime_binding_id: String,
        committed_producer_seq: u64,
        expected: u64,
        actual: u64,
    },
    #[error("session conflict: {0}")]
    Conflict(String),
    #[error("event registry error: {0}")]
    Registry(String),
}

impl SessionStoreError {
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Deleted(_) => Some(SESSION_DELETED),
            Self::NotFound(_) => Some(SESSION_NOT_FOUND),
            Self::IdempotencyConflict(_) => Some(IDEMPOTENCY_CONFLICT),
            Self::InvalidEvent(_) => Some(INVALID_SESSION_EVENT),
            Self::InvalidPayload(_) => Some(INVALID_PAYLOAD),
            Self::InvalidSession(_) => Some(INVALID_SESSION),
            Self::Contract { code, .. } => Some(match code.as_ref() {
                "SESSION_DELETED" => SESSION_DELETED,
                "SNAPSHOT_EXECUTOR_UNAVAILABLE" => "SNAPSHOT_EXECUTOR_UNAVAILABLE",
                _ => "CONTRACT_ERROR",
            }),
            _ => None,
        }
    }

    pub fn is_session_deleted(&self) -> bool {
        matches!(self, Self::Deleted(_))
    }
}
