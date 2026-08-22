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

/// Canonical media kind captured with every ordered task input. It is part of
/// the durable retry snapshot rather than something reconstructed later from a
/// mutable asset row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreationInputKind {
    Image,
    Video,
    Audio,
    Text,
}

impl CreationInputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Text => "text",
        }
    }

    pub fn matches_mime(self, mime: &str) -> bool {
        let essence = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match self {
            Self::Image => essence.starts_with("image/"),
            Self::Video => essence.starts_with("video/"),
            Self::Audio => essence.starts_with("audio/"),
            Self::Text => essence.starts_with("text/"),
        }
    }
}

/// Stable aggregate discriminator for generation outside an individual canvas
/// node. It is part of task ownership and therefore immutable after creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandaloneWorkbenchKind {
    Image,
    Video,
    Audio,
}

impl StandaloneWorkbenchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            _ => return None,
        })
    }

    pub fn accepts_capability(self, capability: MediaCapability) -> bool {
        match self {
            Self::Image => matches!(
                capability,
                MediaCapability::T2i | MediaCapability::I2i | MediaCapability::Inpaint
            ),
            Self::Video => matches!(
                capability,
                MediaCapability::T2v | MediaCapability::I2v | MediaCapability::V2v
            ),
            Self::Audio => capability == MediaCapability::Tts,
        }
    }
}

/// One ordered input binding to a task (contract §3.3 `inputs[]`). `kind` and
/// `role` are both explicit so history/retry never guesses from a URL, file
/// extension, or a later asset-library state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreationInput {
    pub asset_id: String,
    pub kind: CreationInputKind,
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

    /// Attach an HTTP status (for `provider_error`s carrying a remote status).
    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    /// No adapter is registered / routed for the requested capability.
    pub fn adapter_unavailable() -> Self {
        Self::new(
            "adapter_unavailable",
            "no media provider adapter is available for this provider/capability",
        )
    }

    /// A remote provider call failed (network / non-2xx / parse).
    pub fn provider_error(message: impl Into<String>) -> Self {
        Self::new("provider_error", message)
    }

    /// The engine's own wiring is incomplete (no resolver / sink / source).
    pub fn config(message: impl Into<String>) -> Self {
        Self::new("config", message)
    }

    /// The task exceeded the poll deadline.
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("timeout", message)
    }
}

/// Map the invoke layer's typed error onto the creation task error vocabulary
/// (the `error.kind` strings persisted on failed task rows):
/// - `UnsupportedTask` → `unsupported_capability` (untagged model / task gate);
/// - `InvalidParams` → `invalid_params`;
/// - `Timeout` → `timeout`;
/// - `Config` / `MissingConnection` → `config` (local wiring / catalog);
/// - `NoAdapter` → `adapter_unavailable` (matches the old routing semantic);
/// - everything upstream-shaped (`Auth`/`ProviderError`/`JobFailed`/`Network`/
///   `ParseError`/`RateLimited`/`QuotaExhausted`/`ContentPolicy`/`NotPollable`)
///   → `provider_error`.
///
/// `http_status` is transferred verbatim so 4xx/5xx classification survives.
impl From<nomifun_model_invoke::InvokeError> for CreationError {
    fn from(e: nomifun_model_invoke::InvokeError) -> Self {
        use nomifun_model_invoke::InvokeErrorKind as K;
        let kind = match e.kind {
            K::UnsupportedTask => "unsupported_capability",
            K::InvalidParams => "invalid_params",
            K::Timeout => "timeout",
            K::Config | K::MissingConnection => "config",
            // A document body means the configured address is wrong, which the
            // operator fixes in provider settings — not an upstream fault.
            K::NonApiResponse => "config",
            K::NoAdapter => "adapter_unavailable",
            K::Auth
            | K::ProviderError
            | K::JobFailed
            | K::Network
            | K::ParseError
            | K::RateLimited
            | K::QuotaExhausted
            | K::ContentPolicy
            | K::NotPollable => "provider_error",
        };
        Self { kind: kind.to_string(), message: e.message, http_status: e.http_status }
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaCapability, StandaloneWorkbenchKind};

    #[test]
    fn standalone_workbench_kind_accepts_only_its_media_family() {
        assert!(StandaloneWorkbenchKind::Image.accepts_capability(MediaCapability::T2i));
        assert!(StandaloneWorkbenchKind::Image.accepts_capability(MediaCapability::I2i));
        assert!(StandaloneWorkbenchKind::Image.accepts_capability(MediaCapability::Inpaint));
        assert!(!StandaloneWorkbenchKind::Image.accepts_capability(MediaCapability::T2v));
        assert!(StandaloneWorkbenchKind::Video.accepts_capability(MediaCapability::T2v));
        assert!(StandaloneWorkbenchKind::Video.accepts_capability(MediaCapability::I2v));
        assert!(StandaloneWorkbenchKind::Video.accepts_capability(MediaCapability::V2v));
        assert!(!StandaloneWorkbenchKind::Video.accepts_capability(MediaCapability::Tts));
        assert!(StandaloneWorkbenchKind::Audio.accepts_capability(MediaCapability::Tts));
        assert!(!StandaloneWorkbenchKind::Audio.accepts_capability(MediaCapability::T2i));
        assert!(!StandaloneWorkbenchKind::Audio.accepts_capability(MediaCapability::Text));
    }
}
