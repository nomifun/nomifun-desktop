//! Invocation failures retain their original kind across HTTP and Agent boundaries.
use serde::{Deserialize, Serialize};

/// Machine-readable classification of an invocation failure.
/// Wire values are snake_case (serialized into API error payloads/logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelInvocationFailureKind {
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
    /// [`ModelInvocationFailureKind::ParseError`] so the diagnosis can name the address.
    NonApiResponse,
    /// Poll was requested on an adapter/job that cannot be polled.
    NotPollable,
    /// Local provider/connection configuration is incomplete or invalid.
    Config,
}
