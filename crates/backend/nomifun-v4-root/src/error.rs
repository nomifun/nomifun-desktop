use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FreshV4RootError {
    #[error("invalid Fresh-v4 root: {0}")]
    InvalidRoot(String),
    #[error("Fresh-v4 contract mismatch: {0}")]
    Contract(String),
    #[error("Fresh-v4 recovery state is inconsistent: {0}")]
    State(String),
    #[error("Fresh-v4 quiesce failed: {0}")]
    Quiesce(String),
    #[error("Fresh-v4 fault injection at {0}")]
    Fault(String),
    #[error("{operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Fresh-v4 JSON contract error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Fresh-v4 canonical digest error: {0}")]
    Digest(#[from] nomifun_agent_contracts::CanonicalDigestError),
    #[error("Fresh-v4 SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
}

impl FreshV4RootError {
    pub(crate) fn io(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}
