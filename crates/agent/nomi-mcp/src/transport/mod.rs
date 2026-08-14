pub mod sse;
pub mod stdio;
pub mod streamable_http;

use async_trait::async_trait;
use std::time::Duration;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// Keep remote MCP transports bounded even if a caller accidentally bypasses
/// the manager deadline. Connection setup is intentionally shorter than a
/// complete coding-tool request; the manager owns the latter's configurable
/// wall-clock deadline.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn bounded_http_client() -> Result<reqwest::Client, McpError> {
    reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_TIMEOUT)
        .build()
        .map_err(|error| McpError::Transport(format!("failed to build bounded MCP HTTP client: {error}")))
}

/// Find the next SSE event boundary (blank line) in `buf`, returning
/// `(offset, delimiter_len)` for the earliest match.
///
/// Per the SSE spec an event is terminated by a blank line, which may be framed
/// with LF (`\n\n`), CRLF (`\r\n\r\n`), or bare CR (`\r\r`). The MCP spec and
/// most servers use `\n\n`, but some MCP servers behind new-api / one-api style
/// proxies emit `\r\n\r\n`; matching only `\n\n` there finds no event boundary,
/// parses zero events, and yields a silent connection failure ("No endpoint
/// event received" / "SSE stream ended without JSON-RPC response").
///
/// Returns the boundary with the smallest offset; if only a partial delimiter
/// sits at the end of the buffer (e.g. a chunk split mid-`\r\n\r\n`), none match
/// yet and the caller waits for more bytes — same as the original `\n\n` logic.
/// Mirrors `find_sse_event_boundary` in `nomi-providers/src/anthropic_shared.rs`.
pub(crate) fn find_sse_event_boundary(buf: &str) -> Option<(usize, usize)> {
    [
        buf.find("\r\n\r\n").map(|i| (i, 4)),
        buf.find("\n\n").map(|i| (i, 2)),
        buf.find("\r\r").map(|i| (i, 2)),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|&(offset, _)| offset)
}

/// Transport abstraction for MCP communication
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request and receive the response
    async fn request(&self, req: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError>;

    /// Abort the currently outstanding request after the manager's bounded
    /// request deadline expires.  Implementations must make the transport safe
    /// for a later request (for example, a stdio response must not be left in a
    /// pipe where it could be mistaken for the next request's response).
    ///
    /// HTTP transports normally rely on cancellation of the request future;
    /// the default keeps those implementations source-compatible.
    async fn abort_request(&self) -> Result<(), McpError> {
        Ok(())
    }

    /// Send a notification (no response expected)
    async fn notify(&self, req: &JsonRpcRequest) -> Result<(), McpError>;

    /// Close the transport
    async fn close(&self) -> Result<(), McpError>;
}

/// Errors from MCP transport and protocol
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP request timed out after {timeout_ms}ms")]
    RequestTimeout { timeout_ms: u64 },

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },

    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("Tool not found: {server}/{tool}")]
    ToolNotFound { server: String, tool: String },

    #[error("Initialization failed: {0}")]
    InitFailed(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::find_sse_event_boundary;

    #[test]
    fn lf_framing() {
        assert_eq!(find_sse_event_boundary("a\n\nb"), Some((1, 2)));
    }

    #[test]
    fn crlf_framing() {
        // new-api / one-api proxies frame SSE events with CRLF.
        assert_eq!(find_sse_event_boundary("a\r\n\r\nb"), Some((1, 4)));
    }

    #[test]
    fn bare_cr_framing() {
        assert_eq!(find_sse_event_boundary("a\r\rb"), Some((1, 2)));
    }

    #[test]
    fn earliest_boundary_wins() {
        // A CRLF boundary at offset 1 must beat an LF boundary later in the buffer.
        assert_eq!(find_sse_event_boundary("a\r\n\r\nb\n\nc"), Some((1, 4)));
    }

    #[test]
    fn partial_delimiter_waits() {
        // A chunk split mid-CRLF must not match yet.
        assert_eq!(find_sse_event_boundary("data: {}\r\n\r"), None);
        assert_eq!(find_sse_event_boundary("data: {}\n"), None);
    }
}
