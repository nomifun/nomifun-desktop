use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use tokio::sync::oneshot;

use super::{McpError, McpTransport, find_sse_event_boundary};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// SSE transport: connects to an SSE endpoint for server→client events,
/// sends requests via POST to the endpoint URL received from the SSE stream
pub struct SseTransport {
    client: reqwest::Client,
    /// The POST endpoint URL (received from the SSE stream's "endpoint" event)
    post_url: String,
    headers: HeaderMap,
    /// Pending request-response channels, keyed by JSON-RPC id
    pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    /// Handle to the background SSE listener task
    _listener: tokio::task::JoinHandle<()>,
}

impl SseTransport {
    /// Connect to an SSE MCP server
    pub async fn connect(url: &str, headers: &HashMap<String, String>) -> Result<Self, McpError> {
        let mut header_map = HeaderMap::new();
        for (k, v) in headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| McpError::Transport(format!("Invalid header name '{}': {}", k, e)))?;
            let value = HeaderValue::from_str(v)
                .map_err(|e| McpError::Transport(format!("Invalid header value '{}': {}", v, e)))?;
            header_map.insert(name, value);
        }

        let client = super::bounded_http_client()?;

        // GET the SSE endpoint to establish the event stream
        let response = client
            .get(url)
            .headers(header_map.clone())
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("SSE connection failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(McpError::Transport(format!(
                "SSE connection returned status: {}",
                response.status()
            )));
        }

        let pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>> =
            Arc::new(StdMutex::new(HashMap::new()));

        // Parse the SSE stream to find the endpoint URL
        // The server sends an "endpoint" event with the POST URL
        let base_url = extract_base_url(url);
        let mut bytes_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut post_url: Option<String> = None;

        use futures::StreamExt;
        // Read initial events to get the endpoint URL
        while let Some(chunk) = bytes_stream.next().await {
            let chunk = chunk.map_err(|e| McpError::Transport(format!("SSE read error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Parse SSE events from buffer. Events may be framed with LF, CRLF
            // (new-api / one-api proxies), or bare CR — see find_sse_event_boundary.
            while let Some((event_end, delim_len)) = find_sse_event_boundary(&buffer) {
                let event_block = buffer[..event_end].to_string();
                buffer = buffer[event_end + delim_len..].to_string();

                let (event_type, event_data) = parse_sse_event(&event_block);

                if event_type == "endpoint" {
                    // The endpoint might be relative or absolute
                    let endpoint = if event_data.starts_with("http") {
                        event_data.clone()
                    } else {
                        format!("{}{}", base_url, event_data)
                    };
                    post_url = Some(endpoint);
                    break;
                }
            }

            if post_url.is_some() {
                break;
            }
        }

        let post_url = post_url
            .ok_or_else(|| McpError::Transport("No endpoint event received from SSE".into()))?;

        // Spawn background task to listen for SSE responses
        let pending_clone = pending.clone();
        let listener = tokio::spawn(async move {
            let mut buf = buffer; // carry over remaining buffer
            while let Some(chunk) = bytes_stream.next().await {
                let Ok(chunk) = chunk else { break };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some((event_end, delim_len)) = find_sse_event_boundary(&buf) {
                    let event_block = buf[..event_end].to_string();
                    buf = buf[event_end + delim_len..].to_string();

                    let (event_type, event_data) = parse_sse_event(&event_block);

                    if (event_type == "message" || event_type.is_empty())
                        && let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&event_data)
                        && let Some(id) = response.id
                    {
                        let mut map = pending_clone
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(sender) = map.remove(&id) {
                            let _ = sender.send(response);
                        }
                    }
                }
            }
            // A closed or failed SSE listener can no longer deliver any
            // correlated response. Drop every sender so callers fail
            // immediately instead of waiting for the manager deadline.
            pending_clone
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
        });

        Ok(Self {
            client,
            post_url,
            headers: header_map,
            pending,
            _listener: listener,
        })
    }

}

/// Removes an unanswered request from the correlation map even when the
/// caller is cancelled by the manager deadline.  A `tokio::sync::Mutex` cannot
/// be used here because `Drop` cannot await it.
struct PendingRequestGuard {
    pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
    request_id: u64,
    armed: bool,
}

impl PendingRequestGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        if self.armed {
            self.pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.request_id);
        }
    }
}

#[async_trait]
impl McpTransport for SseTransport {
    async fn request(&self, req: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
        let req_id = req
            .id
            .ok_or_else(|| McpError::Transport("Request must have an id".into()))?;

        // Set up response channel before sending
        let (tx, rx) = oneshot::channel::<JsonRpcResponse>();
        {
            let mut map = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            map.insert(req_id, tx);
        }
        let mut pending_guard = PendingRequestGuard {
            pending: Arc::clone(&self.pending),
            request_id: req_id,
            armed: true,
        };

        // POST the request
        let body = serde_json::to_string(req)
            .map_err(|e| McpError::Transport(format!("JSON serialize error: {}", e)))?;

        let response = self
            .client
            .post(&self.post_url)
            .headers(self.headers.clone())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("POST request failed: {}", e)))?;

        if !response.status().is_success() {
            // Clean up pending
            self.pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&req_id);
            return Err(McpError::Transport(format!(
                "POST returned status: {}",
                response.status()
            )));
        }

        // Wait for response from SSE stream
        let rpc_response = rx
            .await
            .map_err(|_| McpError::Transport("Response channel closed unexpectedly".into()))?;
        pending_guard.disarm();

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

        self.client
            .post(&self.post_url)
            .headers(self.headers.clone())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("Notification POST failed: {}", e)))?;

        Ok(())
    }

    async fn close(&self) -> Result<(), McpError> {
        self._listener.abort();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        Ok(())
    }
}

/// Parse a single SSE event block into (event_type, data)
fn parse_sse_event(block: &str) -> (String, String) {
    let mut event_type = String::new();
    let mut data_lines = Vec::new();

    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim().to_string());
        }
    }

    (event_type, data_lines.join("\n"))
}

/// Extract base URL (scheme + host + port) from a full URL
fn extract_base_url(url: &str) -> String {
    // Find the position after "://"
    if let Some(scheme_end) = url.find("://") {
        let rest = &url[scheme_end + 3..];
        if let Some(path_start) = rest.find('/') {
            return url[..scheme_end + 3 + path_start].to_string();
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    async fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP peer closed before sending request headers");
            request.extend_from_slice(&buffer[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP peer closed before sending request body");
            request.extend_from_slice(&buffer[..read]);
        }
        request
    }

    #[tokio::test]
    async fn listener_eof_drops_pending_response_without_waiting_for_manager_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sse_stream, _) = listener.accept().await.unwrap();
            let sse_request = read_http_request(&mut sse_stream).await;
            assert!(String::from_utf8_lossy(&sse_request).starts_with("GET /sse "));
            let endpoint_event = b"event: endpoint\ndata: /message\n\n";
            sse_stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                         Cache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\
                         Connection: keep-alive\r\n\r\n{:X}\r\n",
                        endpoint_event.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            sse_stream.write_all(endpoint_event).await.unwrap();
            sse_stream.write_all(b"\r\n").await.unwrap();
            sse_stream.flush().await.unwrap();

            let (mut post_stream, _) = listener.accept().await.unwrap();
            let post_request = read_http_request(&mut post_stream).await;
            assert!(String::from_utf8_lossy(&post_request).starts_with("POST /message "));
            post_stream
                .write_all(
                    b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            post_stream.flush().await.unwrap();

            // No response event will ever arrive. Closing the listener stream
            // must synchronously drain correlation senders and wake the call.
            drop(sse_stream);
        });

        let transport = SseTransport::connect(
            &format!("http://{address}/sse"),
            &HashMap::new(),
        )
        .await
        .unwrap();
        let request = JsonRpcRequest::new(
            42,
            "tools/call",
            Some(serde_json::json!({"name": "silent", "arguments": {}})),
        );
        let error = tokio::time::timeout(Duration::from_secs(1), transport.request(&request))
            .await
            .expect("SSE listener EOF must wake the pending request immediately")
            .expect_err("the fixture closes before publishing a response");
        assert!(
            error.to_string().contains("Response channel closed"),
            "unexpected SSE EOF error: {error}"
        );
        assert!(
            transport
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
        transport.close().await.unwrap();
        server.await.unwrap();
    }
}
