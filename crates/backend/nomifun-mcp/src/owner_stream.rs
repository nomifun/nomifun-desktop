//! Bounded HTTP SSE framing and shared JSON-RPC server-message handling for
//! HTTP, legacy SSE and stdio. Server traffic never grants a platform capability.
use super::{
    JsonRpcResponse, MAX_RESPONSE_BYTES, McpOwnerError, McpSession, decode_rpc_response,
    ensure_response_id,
};
use serde_json::Value;

pub(super) async fn read_response(
    mut response: reqwest::Response,
    expected: u64,
    session: &mut McpSession,
) -> Result<JsonRpcResponse, McpOwnerError> {
    let mut decoder = EventDecoder::default();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| McpOwnerError::connection_failed("MCP event stream could not be read"))?
    {
        session.traffic.observe(chunk.len(), false)?;
        for byte in chunk {
            let Some(event) = decoder.push(byte)? else {
                continue;
            };
            if !event.kind.is_empty() && event.kind != "message" {
                return Err(McpOwnerError::protocol_failed(
                    "MCP stream contains an unsupported event type",
                ));
            }
            if let Some(result) = process_message(&event.data, expected, session).await? {
                return Ok(result);
            }
        }
        tokio::task::yield_now().await;
    }
    Err(McpOwnerError::protocol_failed(
        "MCP event stream ended without its correlated response",
    ))
}

/// Shared protocol envelope handling for streaming HTTP and stdio transports.
/// Requests from a server cannot expand the advertised client capabilities.
pub(super) async fn process_message(
    data: &str,
    expected: u64,
    session: &mut McpSession,
) -> Result<Option<JsonRpcResponse>, McpOwnerError> {
    session.traffic.observe(0, true)?;
    // An immediately-ready notification stream must not starve host control.
    if session.traffic.messages.is_multiple_of(64) { tokio::task::yield_now().await; }
    let value: Value = serde_json::from_str(data)
        .map_err(|_| McpOwnerError::protocol_failed("MCP event is not JSON-RPC"))?;
    let envelope = value
        .as_object()
        .ok_or_else(|| McpOwnerError::protocol_failed("MCP event must be a JSON-RPC object"))?;
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpOwnerError::protocol_failed(
            "MCP event has an invalid protocol version",
        ));
    }
    if let Some(method_value) = value.get("method") {
        let method = method_value
            .as_str()
            .filter(|method| !method.is_empty() && method.len() <= 1024)
            .ok_or_else(|| McpOwnerError::protocol_failed("MCP message has an invalid method"))?;
        if envelope
            .keys()
            .any(|key| !matches!(key.as_str(), "jsonrpc" | "id" | "method" | "params"))
        {
            return Err(McpOwnerError::protocol_failed(
                "MCP request or notification envelope is malformed",
            ));
        }
        if envelope.contains_key("id") {
            // Same endpoint/credential/session and host request control. The
            // reply path accepts only an empty acknowledgment, not a
            // second stream of arbitrary server requests.
            session.reply_to_server(&value).await?;
        } else {
            // A server may only cancel requests it previously sent to this
            // client. It cannot cancel our outgoing call, even with the same
            // numeric ID. We handle server requests synchronously, so there is
            // no cancellable server-origin task here. Invalid cancels are ignored.
            if method == "notifications/cancelled" { return Ok(None); }
            if value
                .get("params")
                .is_some_and(|params| !params.is_object())
            {
                return Err(McpOwnerError::protocol_failed(
                    "MCP notification params are malformed",
                ));
            }
            if method == "notifications/tools/list_changed" {
                // The checked catalog cannot be mutated underneath a
                // frozen invocation. No refresh/retry after effects.
                return Err(McpOwnerError::new(
                    "MCP_CATALOG_CHANGED",
                    "MCP catalog changed during the frozen invocation",
                ));
            }
            // Other bounded notifications are untrusted observations.
            // They do not rewrite tools, workspace roots or authority.
        }
        return Ok(None);
    }
    let result = decode_rpc_response(value)?;
    ensure_response_id(&result, expected)?;
    // A correlated event ends the request; HTTP EOF need not follow.
    Ok(Some(result))
}

#[derive(Debug)]
pub(super) struct Event {
    pub(super) kind: String,
    pub(super) data: String,
}

/// Incremental line framing: CR terminates a line immediately; a subsequent LF
/// is swallowed even across HTTP chunks. Never rescan a growing partial event.
#[derive(Default)]
pub(super) struct EventDecoder {
    line: Vec<u8>,
    skip_lf: bool,
    first_line_seen: bool,
    event_type: String,
    data: String,
    has_data: bool,
}

impl EventDecoder {
    pub(super) fn push(&mut self, byte: u8) -> Result<Option<Event>, McpOwnerError> {
        if std::mem::take(&mut self.skip_lf) && byte == b'\n' {
            return Ok(None);
        }
        if byte != b'\r' && byte != b'\n' {
            if self.line.len() >= MAX_RESPONSE_BYTES {
                return Err(McpOwnerError::protocol_failed("MCP SSE line exceeds its byte limit"));
            }
            self.line.push(byte);
            return Ok(None);
        }
        self.skip_lf = byte == b'\r';
        let bytes = std::mem::take(&mut self.line);
        let line = std::str::from_utf8(&bytes)
            .map_err(|_| McpOwnerError::protocol_failed("MCP event stream is not UTF-8"))?;
        let line = if !std::mem::replace(&mut self.first_line_seen, true) {
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            let kind = std::mem::take(&mut self.event_type);
            let has_data = std::mem::take(&mut self.has_data);
            let data = std::mem::take(&mut self.data);
            if !has_data {
                return Ok(None);
            }
            return Ok(Some(Event { kind, data }));
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self.data.len().saturating_add(value.len()).saturating_add(usize::from(self.has_data)) > MAX_RESPONSE_BYTES {
                    return Err(McpOwnerError::protocol_failed("MCP SSE message exceeds its byte limit"));
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.has_data = true;
                self.data.push_str(value);
            }
            "event" => {
                self.event_type = value.to_owned();
            }
            // Comments, event IDs and retry hints cannot authorize reconnect,
            // replay or legacy SSE endpoint redirection.
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_events_have_no_lifetime_event_count() {
        let mut decoder = EventDecoder::default();
        for _ in 0..10_000 {
            for byte in b": heartbeat\r\n\r\n" { assert!(decoder.push(*byte).unwrap().is_none()); }
        }
        let mut event = None;
        for byte in b"data: {}\n\n" { if let Some(value) = decoder.push(*byte).unwrap() { event = Some(value); } }
        assert_eq!(event.unwrap().data,"{}");
    }

    #[test]
    fn an_unterminated_single_line_is_bounded() {
        let mut decoder = EventDecoder::default();
        for _ in 0..MAX_RESPONSE_BYTES { decoder.push(b'x').unwrap(); }
        assert_eq!(decoder.push(b'x').unwrap_err().code(),"MCP_PROTOCOL_ERROR");
    }
}
