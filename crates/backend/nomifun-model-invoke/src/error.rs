//! Unified typed error for the invocation layer.
//!
//! [`InvokeError`] is the single error currency of nomifun-model-invoke: every
//! resolver / transport / adapter failure is classified into an
//! [`InvokeErrorKind`] so callers ("按错误语义自愈" loops: key rotation,
//! failover, retry) can branch on semantics instead of parsing strings.

use serde::Serialize;

/// Machine-readable classification of an invocation failure.
/// Wire values are snake_case (serialized into API error payloads/logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeErrorKind {
    /// Upstream rejected our credentials (401/403).
    Auth,
    /// Upstream rate limit (429); `retry_after_ms` may carry the parsed Retry-After.
    RateLimited,
    /// Account/billing quota exhausted (distinct from a transient 429).
    QuotaExhausted,
    /// The model does not declare the requested task.
    UnsupportedTask,
    /// No registered adapter serves the resolved (protocol, task).
    NoAdapter,
    /// The connection profile a route requires is not configured.
    MissingConnection,
    /// Caller-supplied parameters are malformed (also 400/422 upstream).
    InvalidParams,
    /// Provider refused the content on policy grounds.
    ContentPolicy,
    /// Provider-side failure (5xx or unclassified non-2xx).
    ProviderError,
    /// A remote async job reached a terminal failure state (reported by the
    /// provider's job-status endpoint, not an HTTP-level error).
    JobFailed,
    /// Transport-level failure (DNS/connect/TLS/read).
    Network,
    /// The request timed out.
    Timeout,
    /// Provider response could not be understood.
    ParseError,
    /// Poll was requested on an adapter/job that cannot be polled.
    NotPollable,
    /// Local provider/connection configuration is incomplete or invalid.
    Config,
}

/// The invocation-layer error: a kind + human-readable message, optionally
/// annotated with the upstream HTTP status and a parsed Retry-After hint.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct InvokeError {
    pub kind: InvokeErrorKind,
    pub message: String,
    pub http_status: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub(crate) catalog_failure: bool,
}

impl InvokeError {
    /// Build an error of `kind` with no HTTP status / retry hint.
    pub fn new(kind: InvokeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: None,
            retry_after_ms: None,
            catalog_failure: false,
        }
    }

    /// A [`InvokeErrorKind::ProviderError`] carrying the upstream HTTP status.
    pub fn provider(status: u16, message: impl Into<String>) -> Self {
        Self { http_status: Some(status), ..Self::new(InvokeErrorKind::ProviderError, message) }
    }

    /// Attach an upstream HTTP status while preserving the caller's semantic
    /// error kind. Protocol-specific handshakes use this for status-aware
    /// classification without exposing internal error bookkeeping fields.
    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    /// A local-configuration error ([`InvokeErrorKind::Config`]).
    pub fn config(msg: impl Into<String>) -> Self {
        Self::new(InvokeErrorKind::Config, msg)
    }

    /// A local catalog/repository read failure. It remains `Config` for legacy
    /// invoke callers, while capability discovery can distinguish an internal
    /// database fault from a genuinely incomplete candidate configuration.
    pub(crate) fn catalog(msg: impl Into<String>) -> Self {
        Self {
            catalog_failure: true,
            ..Self::config(msg)
        }
    }

    /// Whether this error originated from a failed catalog/repository read.
    pub fn is_catalog_failure(&self) -> bool {
        self.catalog_failure
    }

    /// Classify a reqwest transport error: timeout → [`InvokeErrorKind::Timeout`],
    /// anything else → [`InvokeErrorKind::Network`].
    ///
    /// The message intentionally excludes the source error because reqwest may
    /// render a complete request URL containing query-key credentials.
    pub fn network(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            Self::new(InvokeErrorKind::Timeout, "upstream request timed out")
        } else if e.is_connect() {
            Self::new(InvokeErrorKind::Network, "could not connect to upstream provider")
        } else if e.is_body() {
            Self::new(InvokeErrorKind::Network, "upstream response body transfer failed")
        } else if e.is_decode() {
            Self::new(InvokeErrorKind::Network, "upstream response decoding failed")
        } else if e.is_request() {
            Self::new(InvokeErrorKind::Network, "upstream request could not be sent")
        } else {
            Self::new(InvokeErrorKind::Network, "upstream network request failed")
        }
    }

    /// Map a `Response::json` failure without copying `reqwest::Error`'s
    /// display text. The source may carry the complete response URL (including
    /// query-key credentials), so only a URL-free operation label is retained.
    pub fn response_json(context: &str, e: &reqwest::Error) -> Self {
        if e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() {
            Self::network(e)
        } else {
            Self::parse(format!("{context}: upstream response was not valid JSON"))
        }
    }

    /// A response-parsing error ([`InvokeErrorKind::ParseError`]).
    pub fn parse(msg: impl Into<String>) -> Self {
        Self::new(InvokeErrorKind::ParseError, msg)
    }

    /// The default `ProtocolAdapter::poll` failure ([`InvokeErrorKind::NotPollable`]).
    pub fn not_pollable() -> Self {
        Self::new(InvokeErrorKind::NotPollable, "this adapter does not support polling")
    }

    /// Preserve machine-readable classification while removing exact runtime
    /// credential representations from an adapter/provider diagnostic.
    pub(crate) fn redacted(
        mut self,
        redactor: &nomifun_net::secret_redaction::SecretRedactor,
    ) -> Self {
        self.message = redactor.redact(&self.message);
        self
    }
}

impl From<InvokeError> for nomifun_common::AppError {
    fn from(e: InvokeError) -> Self {
        use InvokeErrorKind::*;
        use nomifun_common::AppError;
        let msg = e.to_string();
        match e.kind {
            // Upstream rejected our stored credentials — an operator problem, surfaced as a gateway failure.
            Auth => AppError::BadGateway(msg),
            // The provider itself failed (5xx / unclassified non-2xx).
            ProviderError => AppError::BadGateway(msg),
            // The remote async job reached a terminal failure state.
            JobFailed => AppError::BadGateway(msg),
            // Transport-level failure reaching the provider.
            Network => AppError::BadGateway(msg),
            // Upstream account quota exhausted — not the client's request, not retryable now.
            QuotaExhausted => AppError::BadGateway(msg),
            // The provider answered something we could not understand.
            ParseError => AppError::BadGateway(msg),
            // The model does not declare the requested task — a bad client request.
            UnsupportedTask => AppError::BadRequest(msg),
            // Caller-supplied parameters are malformed.
            InvalidParams => AppError::BadRequest(msg),
            // The connection profile this route needs is not configured for the provider.
            MissingConnection => AppError::BadRequest(msg),
            // Provider/connection configuration is incomplete or invalid.
            Config => AppError::BadRequest(msg),
            // Poll was requested for a job that cannot be polled.
            NotPollable => AppError::BadRequest(msg),
            // Preserve timeout semantics end to end.
            Timeout => AppError::Timeout(msg),
            // Preserve 429 semantics end to end (AppError::RateLimited carries no message).
            RateLimited => AppError::RateLimited,
            // Valid request refused by the provider's content policy.
            ContentPolicy => AppError::UnprocessableEntity(msg),
            // Server-side wiring gap: a route resolved to a protocol nothing registered.
            NoAdapter => AppError::Internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use nomifun_common::AppError;

    use super::*;

    #[test]
    fn new_has_no_status_or_retry() {
        let e = InvokeError::new(InvokeErrorKind::Auth, "denied");
        assert_eq!(e.kind, InvokeErrorKind::Auth);
        assert_eq!(e.message, "denied");
        assert_eq!(e.http_status, None);
        assert_eq!(e.retry_after_ms, None);
        assert!(!e.is_catalog_failure());
    }

    #[test]
    fn catalog_failure_marker_is_typed_but_keeps_legacy_config_kind() {
        let error = InvokeError::catalog("database unavailable");
        assert_eq!(error.kind, InvokeErrorKind::Config);
        assert!(error.is_catalog_failure());
    }

    #[test]
    fn provider_carries_status() {
        let e = InvokeError::provider(502, "upstream broke");
        assert_eq!(e.kind, InvokeErrorKind::ProviderError);
        assert_eq!(e.http_status, Some(502));
        assert_eq!(e.retry_after_ms, None);
        assert_eq!(e.message, "upstream broke");
    }

    #[test]
    fn shorthand_constructors_map_kinds() {
        assert_eq!(InvokeError::config("x").kind, InvokeErrorKind::Config);
        assert_eq!(InvokeError::parse("x").kind, InvokeErrorKind::ParseError);
        let np = InvokeError::not_pollable();
        assert_eq!(np.kind, InvokeErrorKind::NotPollable);
        assert!(!np.message.is_empty());
    }

    #[test]
    fn display_folds_kind_and_message() {
        let e = InvokeError::new(InvokeErrorKind::RateLimited, "slow down");
        assert_eq!(e.to_string(), "RateLimited: slow down");
    }

    #[test]
    fn kind_serializes_snake_case() {
        for (kind, wire) in [
            (InvokeErrorKind::Auth, "\"auth\""),
            (InvokeErrorKind::RateLimited, "\"rate_limited\""),
            (InvokeErrorKind::QuotaExhausted, "\"quota_exhausted\""),
            (InvokeErrorKind::UnsupportedTask, "\"unsupported_task\""),
            (InvokeErrorKind::NoAdapter, "\"no_adapter\""),
            (InvokeErrorKind::MissingConnection, "\"missing_connection\""),
            (InvokeErrorKind::InvalidParams, "\"invalid_params\""),
            (InvokeErrorKind::ContentPolicy, "\"content_policy\""),
            (InvokeErrorKind::ProviderError, "\"provider_error\""),
            (InvokeErrorKind::JobFailed, "\"job_failed\""),
            (InvokeErrorKind::Network, "\"network\""),
            (InvokeErrorKind::Timeout, "\"timeout\""),
            (InvokeErrorKind::ParseError, "\"parse_error\""),
            (InvokeErrorKind::NotPollable, "\"not_pollable\""),
            (InvokeErrorKind::Config, "\"config\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), wire, "kind {kind:?}");
        }
    }

    #[test]
    fn app_error_mapping_covers_every_kind() {
        use InvokeErrorKind::*;
        type Check = fn(&AppError) -> bool;
        let cases: Vec<(InvokeErrorKind, Check)> = vec![
            (Auth, |a| matches!(a, AppError::BadGateway(_))),
            (ProviderError, |a| matches!(a, AppError::BadGateway(_))),
            (JobFailed, |a| matches!(a, AppError::BadGateway(_))),
            (Network, |a| matches!(a, AppError::BadGateway(_))),
            (QuotaExhausted, |a| matches!(a, AppError::BadGateway(_))),
            (ParseError, |a| matches!(a, AppError::BadGateway(_))),
            (UnsupportedTask, |a| matches!(a, AppError::BadRequest(_))),
            (InvalidParams, |a| matches!(a, AppError::BadRequest(_))),
            (MissingConnection, |a| matches!(a, AppError::BadRequest(_))),
            (Config, |a| matches!(a, AppError::BadRequest(_))),
            (NotPollable, |a| matches!(a, AppError::BadRequest(_))),
            (Timeout, |a| matches!(a, AppError::Timeout(_))),
            (RateLimited, |a| matches!(a, AppError::RateLimited)),
            (ContentPolicy, |a| matches!(a, AppError::UnprocessableEntity(_))),
            (NoAdapter, |a| matches!(a, AppError::Internal(_))),
        ];
        for (kind, check) in cases {
            let app: AppError = InvokeError::new(kind, "boom").into();
            assert!(check(&app), "kind {kind:?} mapped to unexpected {app:?}");
        }
    }

    #[test]
    fn app_error_mapping_preserves_message() {
        let app: AppError = InvokeError::new(InvokeErrorKind::ProviderError, "upstream exploded").into();
        assert!(app.to_string().contains("upstream exploded"), "got: {app}");
    }
}
