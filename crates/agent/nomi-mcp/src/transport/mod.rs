pub mod sse;
pub mod stdio;
pub mod streamable_http;

#[cfg(test)]
mod http_audit_tests;

use async_trait::async_trait;
use std::time::Duration;
use std::collections::HashMap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// Keep remote MCP transports bounded even if a caller accidentally bypasses
/// the manager deadline. Connection setup is intentionally shorter than a
/// complete coding-tool request; the manager owns the latter's configurable
/// wall-clock deadline.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn bounded_http_client() -> Result<reqwest::Client, McpError> {
    reqwest::Client::builder()
        // Configured headers may carry custom credentials, not just Authorization.
        // Follow only same-origin redirects; cross-origin endpoints need explicit config.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().first().is_some_and(|first| first.origin() == attempt.url().origin())
                && attempt.previous().len() < 10
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_TIMEOUT)
        .build()
        .map_err(|error| McpError::Transport(format!("failed to build bounded MCP HTTP client: {error}")))
}

pub(crate) fn check_http_status(response: reqwest::Response) -> Result<reqwest::Response, McpError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(McpError::Transport(format!("HTTP request returned status: {}", response.status())))
    }
}

/// Shared header validation must never echo configured credential values.
pub(crate) fn http_headers(headers: &HashMap<String, String>) -> Result<HeaderMap, McpError> {
    let mut parsed = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| McpError::Transport(format!("Invalid HTTP header name: {error}")))?;
        let mut value = HeaderValue::from_str(value)
            .map_err(|error| McpError::Transport(format!("Invalid HTTP header value: {error}")))?;
        value.set_sensitive(true);
        parsed.insert(name, value);
    }
    Ok(parsed)
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
pub(crate) fn find_sse_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    [b"\r\n\r\n".as_slice(), b"\n\n", b"\r\r"]
    .map(|delimiter| buf.windows(delimiter.len()).position(|w| w == delimiter).map(|i| (i, delimiter.len())))
    .into_iter()
    .flatten()
    .min_by_key(|&(offset, _)| offset)
}

/// Parse a single SSE event block into (event_type, data)
pub(crate) fn parse_sse_event(block: &str) -> (String, String) {
    let mut event_type = String::new();
    let mut data_lines = Vec::new();

    for line in block.split(['\r', '\n']) {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = value.strip_prefix(' ').unwrap_or(value).to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        }
    }

    (event_type, data_lines.join("\n"))
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
        assert_eq!(find_sse_event_boundary(b"a\n\nb"), Some((1, 2)));
    }

    #[test]
    fn crlf_framing() {
        // new-api / one-api proxies frame SSE events with CRLF.
        assert_eq!(find_sse_event_boundary(b"a\r\n\r\nb"), Some((1, 4)));
    }

    #[test]
    fn bare_cr_framing() {
        assert_eq!(find_sse_event_boundary(b"a\r\rb"), Some((1, 2)));
    }

    #[test]
    fn earliest_boundary_wins() {
        // A CRLF boundary at offset 1 must beat an LF boundary later in the buffer.
        assert_eq!(find_sse_event_boundary(b"a\r\n\r\nb\n\nc"), Some((1, 4)));
    }

    #[test]
    fn partial_delimiter_waits() {
        // A chunk split mid-CRLF must not match yet.
        assert_eq!(find_sse_event_boundary(b"data: {}\r\n\r"), None);
        assert_eq!(find_sse_event_boundary(b"data: {}\n"), None);
    }
}
