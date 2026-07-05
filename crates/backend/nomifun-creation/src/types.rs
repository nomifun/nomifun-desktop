//! Shared value types for the generation engine (contract §6 `types.rs`).

use serde::{Deserialize, Serialize};

/// The media operation a task performs. Wire values are the lowercase codes
/// from contract §3.3 (`t2i|i2i|inpaint|t2v|i2v|v2v|tts|text`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCapability {
    /// text → image
    T2i,
    /// image → image
    I2i,
    /// masked local repaint
    Inpaint,
    /// text → video
    T2v,
    /// image → video
    I2v,
    /// video → video
    V2v,
    /// text → speech
    Tts,
    /// LLM text
    Text,
}

impl MediaCapability {
    /// The canonical wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::T2i => "t2i",
            Self::I2i => "i2i",
            Self::Inpaint => "inpaint",
            Self::T2v => "t2v",
            Self::I2v => "i2v",
            Self::V2v => "v2v",
            Self::Tts => "tts",
            Self::Text => "text",
        }
    }

    /// Parse a wire capability string.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "t2i" => Self::T2i,
            "i2i" => Self::I2i,
            "inpaint" => Self::Inpaint,
            "t2v" => Self::T2v,
            "i2v" => Self::I2v,
            "v2v" => Self::V2v,
            "tts" => Self::Tts,
            "text" => Self::Text,
            _ => return None,
        })
    }
}

/// The task lifecycle state (contract §3.3 `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// True for states with no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }
}

/// One input reference to a task (contract §3.3 `inputs[]`). `role` is a free
/// string (`reference|mask|first_frame|last_frame|video|audio`) an adapter
/// interprets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationInput {
    pub asset_id: String,
    pub role: String,
}

/// A structured error stored on a failed task (`error` JSON column).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationError {
    /// Stable machine code, e.g. `adapter_unavailable`, `provider_error`, `timeout`.
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub http_status: Option<u16>,
}

impl CreationError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind: kind.into(), message: message.into(), http_status: None }
    }

    /// The M0 placeholder: no adapter is wired yet.
    pub fn adapter_unavailable() -> Self {
        Self::new(
            "adapter_unavailable",
            "no media provider adapter is available yet (M0 skeleton); generation lands in M2",
        )
    }
}
