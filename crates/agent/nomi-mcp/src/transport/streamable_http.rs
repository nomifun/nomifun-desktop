use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use tokio::sync::Mutex;

use super::{McpError, McpTransport, find_sse_event_boundary, http_headers, parse_sse_event};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// Streamable HTTP transport: uses HTTP POST for both requests and responses
/// Supports optional SSE streaming for server responses
pub struct StreamableHttpTransport {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    session_id: Mutex<Option<String>>,
}

impl StreamableHttpTransport {
    /// Create a new Streamable HTTP transport
    pub async fn connect(url: &str, headers: &HashMap<String, String>) -> Result<Self, McpError> {
        let header_map = http_headers(headers)?;

        Ok(Self {
            client: super::bounded_http_client()?,
            url: url.to_string(),
            headers: header_map,
            session_id: Mutex::new(None),
        })
    }

    /// Build request with session ID header if available
    async fn build_request(&self, body: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(&self.url)
            .headers(self.headers.clone())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");

        if let Some(sid) = self.session_id.lock().await.as_ref() {
            req = req.header("Mcp-Session-Id", sid.as_str());
        }

        req.body(body.to_string())
    }

    /// Parse response based on content type
    async fn parse_response(
        &self,
        response: reqwest::Response,
        request_id: u64,
    ) -> Result<JsonRpcResponse, McpError> {
        // Capture session ID from response headers
        if let Some(sid) = response.headers().get("mcp-session-id")
            && let Ok(sid_str) = sid.to_str()
        {
            *self.session_id.lock().await = Some(sid_str.to_string());
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // SSE response: parse events to find the JSON-RPC response
            self.parse_sse_response(response, request_id).await
        } else {
            // Direct JSON response
            let text = response
                .text()
                .await
                .map_err(|e| McpError::Transport(format!("Read response body failed: {}", e.without_url())))?;
            serde_json::from_str(&text).map_err(|e| {
                McpError::Transport(format!("Invalid JSON-RPC response at line {}, column {}", e.line(), e.column()))
            })
        }
    }

    /// Parse an SSE stream response to extract JSON-RPC response
    async fn parse_sse_response(
        &self,
        response: reqwest::Response,
        request_id: u64,
    ) -> Result<JsonRpcResponse, McpError> {
        use futures::StreamExt;

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| McpError::Transport(format!("SSE read error: {}", e.without_url())))?;
            buffer.extend_from_slice(&chunk);

            // Parse SSE events. Events may be framed with LF, CRLF (new-api /
            // one-api proxies), or bare CR — see find_sse_event_boundary.
            while let Some((event_end, delim_len)) = find_sse_event_boundary(&buffer) {
                let event_block = String::from_utf8_lossy(&buffer[..event_end]).into_owned();
                buffer.drain(..event_end + delim_len);

                let (_, data) = parse_sse_event(&event_block);
                if !data.is_empty()
                    && let Ok(rpc_response) = serde_json::from_str::<JsonRpcResponse>(&data)
                    && rpc_response.id == Some(request_id)
                {
                    return Ok(rpc_response);
                }
            }
        }

        Err(McpError::Transport(
            "SSE stream ended without JSON-RPC response".into(),
        ))
    }
}

#[async_trait]
impl McpTransport for StreamableHttpTransport {
    async fn request(&self, req: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        let request_id = req.id.ok_or_else(|| McpError::Transport("Request must have an id".into()))?;
        let body = serde_json::to_string(req)
            .map_err(|e| McpError::Transport(format!("JSON serialize error: {}", e)))?;

        let http_req = self.build_request(&body).await;
        let response = http_req
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("HTTP request failed: {}", e.without_url())))?;

        let response = super::check_http_status(response)?;

        let rpc_response = self.parse_response(response, request_id).await?;
        if rpc_response.id != Some(request_id) || rpc_response.jsonrpc != "2.0" {
            return Err(McpError::Transport("Invalid JSON-RPC response version or request id".into()));
        }

        if let Some(err) = &rpc_response.error {
            return Err(McpError::JsonRpc {
                code: err.code,
                message: err.message.clone(),
            });
        }

        Ok(rpc_response)
    }

    async fn notify(&self, req: &JsonRpcRequest) -> Result<(), McpError> {
        let body = serde_json::to_string(req)
            .map_err(|e| McpError::Transport(format!("JSON serialize error: {}", e)))?;

        let http_req = self.build_request(&body).await;
        http_req
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("Notification request failed: {}", e.without_url())))
            .and_then(super::check_http_status)?;

        Ok(())
    }

    async fn close(&self) -> Result<(), McpError> {
        // No persistent connection to close for HTTP
        Ok(())
    }
}
