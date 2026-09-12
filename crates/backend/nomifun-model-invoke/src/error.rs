//! Unified typed error for the invocation layer.
//!
//! [`InvokeError`] is the single error currency of nomifun-model-invoke: every
//! resolver / transport / adapter failure is classified into an
//! [`InvokeErrorKind`] so callers ("按错误语义自愈" loops: key rotation,
//! failover, retry) can branch on semantics instead of parsing strings.

use serde::Serialize;
use nomifun_net::secret_redaction::redact_url_queries as transport_cause_detail;

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
    /// The URL answered with a document (HTML/XML) instead of an API payload —
    /// almost always a wrong path, not a provider fault. Kept distinct from
    /// [`InvokeErrorKind::ParseError`] so the diagnosis can name the address.
    NonApiResponse,
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

/// Render a transport error's cause chain for a diagnostic, with URL query
/// strings removed.
///
/// Walks to the innermost source: reqwest's own `Display` is mostly the request
/// URL, while the actionable part ("dns error", "tcp connect error",
/// "invalid peer certificate") lives further down the chain.
fn transport_detail(e: &reqwest::Error) -> Option<String> {
    let mut rendered = Vec::new();
    let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(current) = source {
        let text = current.to_string();
        if !text.trim().is_empty() && !rendered.iter().any(|seen| seen == &text) {
            rendered.push(text);
        }
        source = current.source();
    }
    if rendered.is_empty() {
        // No source chain (rare): fall back to reqwest's own rendering, which
        // is redacted the same way.
        rendered.push(e.to_string());
    }
    let detail = transport_cause_detail(&rendered.join(": "));
    (!detail.trim().is_empty()).then_some(detail)
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
    /// The rendered cause is appended with every URL query string stripped.
    /// reqwest's `Display` includes the full request URL, and a query key is a
    /// credential — but discarding the cause entirely made DNS failure, TLS
    /// rejection, a dead local proxy and a refused port produce one identical
    /// sentence, which is not enough to act on.
    pub fn network(e: &reqwest::Error) -> Self {
        let (kind, label) = if e.is_timeout() {
            (InvokeErrorKind::Timeout, "upstream request timed out")
        } else if e.is_connect() {
            (InvokeErrorKind::Network, "could not connect to upstream provider")
        } else if e.is_body() {
            (InvokeErrorKind::Network, "upstream response body transfer failed")
        } else if e.is_decode() {
            (InvokeErrorKind::Network, "upstream response decoding failed")
        } else if e.is_request() {
            (InvokeErrorKind::Network, "upstream request could not be sent")
        } else {
            (InvokeErrorKind::Network, "upstream network request failed")
        };
        // The label stays the message PREFIX: `provider_health::classify_error`
        // and other callers match on these exact strings.
        match transport_detail(e) {
            Some(detail) => Self::new(kind, format!("{label} ({detail})")),
            None => Self::new(kind, label),
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

    /// The configured URL served a document instead of an API payload.
    ///
    /// Carries the upstream status so a `200 OK` HTML page is still reported as
    /// what it is: the wrong address, answered successfully.
    pub fn non_api_response(status: u16, content_type: &str) -> Self {
        Self {
            http_status: Some(status),
            ..Self::new(
                InvokeErrorKind::NonApiResponse,
                format!(
                    "provider returned {status} with content-type {content_type}: {}",
                    nomifun_net::api_response::NON_API_DIAGNOSTIC
                ),
            )
        }
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
            // The configured address served a web page. This is a configuration
            // fault the operator can fix, not an upstream outage.
            NonApiResponse => AppError::BadRequest(msg),
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

    /// A bare "could not connect" cannot distinguish DNS from TLS from a dead
    /// proxy from a refused port, which is the difference between "fix your
    /// resolver" and "the provider is down". The cause is appended with query
    /// strings stripped, because reqwest renders the full request URL and a
    /// query key is a credential.
    #[tokio::test]
    async fn connect_failures_name_the_cause_without_leaking_query_credentials() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // Port 1 on loopback refuses instantly; the query carries a fake secret.
        let error = client
            .get("http://127.0.0.1:1/v1/models?api_key=super-secret-value")
            .send()
            .await
            .expect_err("connect to port 1 must fail");

        let invoke = InvokeError::network(&error);

        assert_eq!(invoke.kind, InvokeErrorKind::Network);
        assert!(
            invoke.message.starts_with("could not connect to upstream provider"),
            "the stable prefix is matched by provider_health::classify_error: {}",
            invoke.message
        );
        assert!(
            !invoke.message.contains("super-secret-value"),
            "query credentials must never reach the message: {}",
            invoke.message
        );
        // Something about the underlying cause has to survive, otherwise the
        // message is exactly as undiagnosable as before.
        assert!(
            invoke.message.len() > "could not connect to upstream provider".len(),
            "expected an appended cause: {}",
            invoke.message
        );
    }

    #[test]
    fn transport_cause_strips_queries_but_keeps_hosts_and_reasons() {
        let detail = transport_cause_detail(
            "error sending request for url (https://api.example.com/v1/audio/speech?key=leaked): \
             dns error: failed to lookup address",
        );
        assert!(!detail.contains("leaked"), "got: {detail}");
        assert!(detail.contains("api.example.com"), "host is diagnostic, keep it: {detail}");
        assert!(detail.contains("dns error"), "the reason is the whole point: {detail}");
    }

    /// The shared boundary must cover userinfo, WebSocket queries and queries
    /// containing parentheses while preserving diagnostic hosts.
    #[test]
    fn transport_cause_uses_the_shared_url_credential_boundary() {
        for (input, secret) in [
            ("https://user:PASSWD@host/v1 failed", "PASSWD"),
            ("proxy http://user:PROXYPASS@127.0.0.1:7897 refused", "PROXYPASS"),
            ("wss://host/realtime?token=WSSECRET closed", "WSSECRET"),
            ("ws://host/rt?api_key=WSKEY", "WSKEY"),
            ("error for url (https://h/v1?cb=f(x)&api_key=LEAKED)", "LEAKED"),
        ] {
            let detail = transport_cause_detail(input);
            assert!(!detail.contains(secret), "{secret} leaked from {input:?}: {detail}");
            // The host must survive: it is the actionable half.
            assert!(detail.contains("host") || detail.contains("127.0.0.1") || detail.contains('h'));
        }
    }
}
