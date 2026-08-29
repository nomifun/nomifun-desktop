use std::io;

use nomifun_agent_contracts::CheckpointDiscardReason;
use nomifun_agent_contracts::SnapshotContractMismatch;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime protocol violation: {0}")]
    Protocol(String),

    #[error("runtime RPC method is not allowlisted: {0}")]
    RpcNotAllowed(String),

    #[error("runtime hello rejected: {0}")]
    HelloRejected(String),

    #[error("runtime release manifest rejected: {0}")]
    ReleaseManifest(String),

    #[error("runtime process error: {0}")]
    Process(#[source] io::Error),

    #[error("runtime RPC error {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },

    #[error("runtime session is disposed")]
    Disposed,

    #[error("runtime session already exists")]
    SessionAlreadyExists,

    #[error("runtime session was not found")]
    SessionNotFound,

    #[error("runtime operation timed out: {0}")]
    Timeout(String),

    #[error("runtime checkpoint was discarded: {0:?}")]
    CheckpointDiscarded(Vec<CheckpointDiscardReason>),

    #[error("snapshot executor is unavailable")]
    SnapshotExecutorUnavailable(Vec<SnapshotContractMismatch>),

    #[error("native action ACK rejected: {0}")]
    NativeActionAck(String),

    #[error("native action has already been committed: {0}")]
    NativeActionAlreadyCommitted(String),

    #[error("credential handoff failed: {0}")]
    Credential(String),

    #[error("runtime JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<io::Error> for RuntimeError {
    fn from(value: io::Error) -> Self {
        Self::Process(value)
    }
}
