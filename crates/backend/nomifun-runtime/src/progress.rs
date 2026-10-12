//! Ephemeral preparation observations published by the component supplier itself.
use crate::ToolId;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPreparationPhase {
    Idle,
    Checking,
    Downloading,
    Extracting,
    Verifying,
    Ready,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolPreparationFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolPreparationSnapshot {
    pub tool: ToolId,
    pub version: String,
    pub revision: u64,
    pub phase: ToolPreparationPhase,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<ToolPreparationFailure>,
}
