//! Explicit legacy HTTP+SSE transport. No reconnect, Last-Event-ID replay,
//! cross-origin endpoint routing, or synthetic remote shutdown acknowledgment.
use super::{
    JsonRpcResponse, MAX_RESPONSE_BYTES, McpOwnerError, McpSession, stream, validate_http_endpoint,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};

pub(super) const PROTOCOL_VERSION: &str = "2024-11-05";

pub(super) struct LegacyStream {
    response: reqwest::Response,
    decoder: stream::EventDecoder,
    pending: Vec<u8>,
    cursor: usize,
    received: usize,
}

impl LegacyStream {
    pub(super) async fn connect(
        client: &reqwest::Client,
        endpoint: &str,
        headers: &HeaderMap,
    ) -> Result<(Self, String), McpOwnerError> {
        let base = validate_http_endpoint(endpoint)?;
        // The message POST opens another connection. Like Codex's same-origin
        // routing policy, do not trust rebinding of plaintext domain names.
        if base.scheme() == "http"
            && base.host_str().is_some_and(|host| {
                host != "localhost"
                    && host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_err()
            })
        {
            return Err(McpOwnerError::invalid_binding(
                "Legacy SSE for non-localhost domain names requires HTTPS",
            ));
        }
        let mut headers = headers.clone();
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        headers.remove(CONTENT_TYPE);
        let response = client
            .get(base.clone())
            .headers(headers)
            .send()
            .await
            .map_err(|_| McpOwnerError::connection_failed("MCP legacy SSE connection failed"))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(McpOwnerError::credential_required(response.headers()));
        }
        if response.status() != reqwest::StatusCode::OK || response.url() != &base {
            return Err(McpOwnerError::connection_failed(
                "MCP legacy SSE requires HTTP 200 without redirects",
            ));
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if !mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("text/event-stream")
        {
            return Err(McpOwnerError::protocol_failed(
                "MCP legacy SSE requires text/event-stream",
            ));
        }
        let mut stream = Self {
            response,
            decoder: Default::default(),
            pending: Vec::new(),
            cursor: 0,
            received: 0,
        };
        let event = stream.next().await?;
        if event.kind != "endpoint"
            || event.data.is_empty()
            || event.data.len() > 8192
            || event.data.trim() != event.data
            || event.data.chars().any(char::is_control)
        {
            return Err(McpOwnerError::protocol_failed(
                "MCP legacy SSE must first announce one bounded message endpoint",
            ));
        }
        let target = base
            .join(&event.data)
            .map_err(|_| McpOwnerError::invalid_binding("MCP message endpoint is invalid"))?;
        validate_http_endpoint(target.as_str())?;
        if target.origin() != base.origin() {
            return Err(McpOwnerError::invalid_binding(
                "MCP message endpoint must remain on the configured origin",
            ));
        }
        Ok((stream, target.to_string()))
    }

    async fn next(&mut self) -> Result<stream::Event, McpOwnerError> {
        loop {
            while self.cursor < self.pending.len() {
                let byte = self.pending[self.cursor];
                self.cursor += 1;
                if let Some(event) = self.decoder.push(byte)? {
                    return Ok(event);
                }
            }
            self.pending.clear();
            self.cursor = 0;
            let chunk = self
                .response
                .chunk()
                .await
                .map_err(|_| {
                    McpOwnerError::connection_failed("MCP legacy event stream could not be read")
                })?
                .ok_or_else(|| {
                    McpOwnerError::protocol_failed(
                        "MCP legacy event stream ended before the correlated response",
                    )
                })?;
            self.received = self.received.saturating_add(chunk.len());
            if self.received > MAX_RESPONSE_BYTES {
                return Err(McpOwnerError::protocol_failed(
                    "MCP legacy stream exceeds the transaction byte limit",
                ));
            }
            self.pending.extend_from_slice(&chunk);
        }
    }
}

pub(super) async fn read_response(
    session: &mut McpSession,
    expected: u64,
) -> Result<JsonRpcResponse, McpOwnerError> {
    loop {
        let event = session
            .legacy
            .as_mut()
            .ok_or_else(|| McpOwnerError::protocol_failed("MCP legacy stream is unavailable"))?
            .next()
            .await?;
        // A second endpoint announcement cannot change routing mid-transaction.
        if !event.kind.is_empty() && event.kind != "message" {
            return Err(McpOwnerError::protocol_failed(
                "MCP legacy stream changed endpoint or returned an unsupported event",
            ));
        }
        if let Some(result) = stream::process_message(&event.data, expected, session).await? {
            return Ok(result);
        }
    }
}

/// Legacy POST acknowledgment is not the JSON-RPC result. Some servers send
/// a short textual body (e.g. Accepted); consume it without interpreting it.
pub(super) async fn accept_ack(mut response: reqwest::Response) -> Result<(), McpOwnerError> {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(McpOwnerError::credential_required(response.headers()));
    }
    if response.status() != reqwest::StatusCode::ACCEPTED {
        return Err(McpOwnerError::protocol_failed(
            "MCP legacy message endpoint must acknowledge with HTTP 202",
        ));
    }
    let mut bytes = 0usize;
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        McpOwnerError::connection_failed("MCP legacy acknowledgment could not be read")
    })? {
        bytes = bytes.saturating_add(chunk.len());
        if bytes > 16 * 1024 {
            return Err(McpOwnerError::protocol_failed(
                "MCP legacy acknowledgment exceeds its byte limit",
            ));
        }
    }
    Ok(())
}
