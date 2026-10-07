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
    /// Newly composed music, distinct from speech synthesis.
    Music,
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
            Self::Music => "music",
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
            "music" => Self::Music,
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

/// Server-owned reference captured from one ordinary conversation attachment.
/// `file_index` preserves order among the original files, including non-images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationCreationReference {
    pub asset_id: String,
    pub file_name: String,
    pub file_index: usize,
    pub kind: CreationInputKind,
}

/// A structured error stored on a failed task (`error` JSON column).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationError {
    /// Stable machine code, e.g. `adapter_unavailable`, `provider_error`, `timeout`.
    pub kind: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub http_status: Option<u16>,
    /// Approved provider evidence uses the same contract as conversation errors.
    #[serde(default, rename = "providerDiagnostic", skip_serializing_if = "Option::is_none")]
    pub provider_diagnostic: Option<nomifun_api_types::ModelFailureDiagnostic>,
}

impl CreationError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self { kind: kind.into(), message: message.into(), http_status: None, provider_diagnostic: None }
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
/// `http_status` and approved typed diagnostics are transferred verbatim. The
/// human message is never parsed into diagnostic evidence.
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
        Self {
            kind: kind.to_string(), message: e.message, http_status: e.http_status,
            provider_diagnostic: e.diagnostic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{ModelFailureDiagnostic, ModelFailureReason};
    use nomifun_model_invoke::{InvokeError, InvokeErrorKind};

    #[test]
    fn invoke_error_conversion_preserves_approved_diagnostic_and_existing_error_semantics() {
        let mut diagnostic = ModelFailureDiagnostic::new(ModelFailureReason::InvalidKey);
        diagnostic.http_status = Some(401);
        diagnostic.provider_code = Some("invalid_api_key".into());
        diagnostic.provider_param = Some("headers.authorization".into());
        diagnostic.provider_id = Some("actual-provider".into());
        diagnostic.model_name = Some("actual-media-model".into());
        diagnostic.request_id = Some("safe-request-id".into());
        for (invoke_kind, creation_kind) in [
            (InvokeErrorKind::UnsupportedTask, "unsupported_capability"),
            (InvokeErrorKind::InvalidParams, "invalid_params"),
            (InvokeErrorKind::Timeout, "timeout"),
            (InvokeErrorKind::Config, "config"),
            (InvokeErrorKind::MissingConnection, "config"),
            (InvokeErrorKind::NonApiResponse, "config"),
            (InvokeErrorKind::NoAdapter, "adapter_unavailable"),
            (InvokeErrorKind::Auth, "provider_error"),
            (InvokeErrorKind::ProviderError, "provider_error"),
            (InvokeErrorKind::JobFailed, "provider_error"),
            (InvokeErrorKind::Network, "provider_error"),
            (InvokeErrorKind::ParseError, "provider_error"),
            (InvokeErrorKind::RateLimited, "provider_error"),
            (InvokeErrorKind::QuotaExhausted, "provider_error"),
            (InvokeErrorKind::ContentPolicy, "provider_error"),
            (InvokeErrorKind::NotPollable, "provider_error"),
        ] {
            let mut source = InvokeError::new(invoke_kind, "Safe fixed model failure").with_http_status(401);
            source.diagnostic = Some(diagnostic.clone());
            let error = CreationError::from(source);
            assert_eq!(error.kind, creation_kind);
            assert_eq!(error.message, "Safe fixed model failure");
            assert_eq!(error.http_status, Some(401));
            assert_eq!(error.provider_diagnostic.as_ref(), Some(&diagnostic));
            let value = serde_json::to_value(&error).unwrap();
            assert_eq!(value["providerDiagnostic"], serde_json::to_value(&diagnostic).unwrap());
            assert!(value.get("provider_diagnostic").is_none());
            assert_eq!(serde_json::from_value::<CreationError>(value).unwrap().provider_diagnostic, Some(diagnostic.clone()));
        }
    }

    #[test]
    fn creation_error_without_typed_evidence_does_not_guess_from_message_or_status() {
        let old: CreationError = serde_json::from_value(serde_json::json!({
            "kind":"provider_error", "message":"existing diagnostic", "http_status":503,
        })).unwrap();
        assert!(old.provider_diagnostic.is_none());
        let source = InvokeError::provider(401, "balance insufficient; subscription expired; invalid api key");
        let error = CreationError::from(source);
        assert_eq!(error.kind, "provider_error");
        assert_eq!(error.http_status, Some(401));
        assert!(error.provider_diagnostic.is_none());
        assert!(serde_json::to_value(error).unwrap().get("providerDiagnostic").is_none());
        assert!(CreationError::new("timeout", "Safe local deadline").provider_diagnostic.is_none());
    }
}
