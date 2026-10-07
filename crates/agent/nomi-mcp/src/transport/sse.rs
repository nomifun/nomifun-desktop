use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use tokio::sync::oneshot;

use super::{McpError, McpTransport, find_sse_event_boundary, http_headers, parse_sse_event};
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

type PendingResponses = Arc<StdMutex<Option<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>>;

/// SSE transport: connects to an SSE endpoint for server→client events,
/// sends requests via POST to the endpoint URL received from the SSE stream
pub struct SseTransport {
    client: reqwest::Client,
    /// The POST endpoint URL (received from the SSE stream's "endpoint" event)
    post_url: String,
    headers: HeaderMap,
    /// Pending request-response channels, keyed by JSON-RPC id
    pending: PendingResponses,
    /// Handle to the background SSE listener task
    _listener: tokio::task::JoinHandle<()>,
}

impl SseTransport {
    /// Connect to an SSE MCP server
    pub async fn connect(url: &str, headers: &HashMap<String, String>) -> Result<Self, McpError> {
        let header_map = http_headers(headers)?;

        let client = super::bounded_http_client()?;

        // GET the SSE endpoint to establish the event stream
        let response = client
            .get(url)
            .headers(header_map.clone())
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("SSE connection failed: {}", e.without_url())))?;

        let response = super::check_http_status(response)?;

        let pending: PendingResponses = Arc::new(StdMutex::new(Some(HashMap::new())));

        // Parse the SSE stream to find the endpoint URL
        // The server sends an "endpoint" event with the POST URL
        let base_url = response.url().clone();
        let mut bytes_stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let mut post_url: Option<String> = None;

        use futures::StreamExt;
        // Read initial events to get the endpoint URL
        while let Some(chunk) = bytes_stream.next().await {
            let chunk = chunk.map_err(|e| McpError::Transport(format!("SSE read error: {}", e.without_url())))?;
            buffer.extend_from_slice(&chunk);

            // Parse SSE events from buffer. Events may be framed with LF, CRLF
            // (new-api / one-api proxies), or bare CR — see find_sse_event_boundary.
            while let Some((event_end, delim_len)) = find_sse_event_boundary(&buffer) {
                let event_block = String::from_utf8_lossy(&buffer[..event_end]).into_owned();
                buffer.drain(..event_end + delim_len);

                let (event_type, event_data) = parse_sse_event(&event_block);

                if event_type == "endpoint" {
                    // The endpoint might be relative or absolute
                    let endpoint = base_url.join(&event_data)
                        .map_err(|_| McpError::Transport("Invalid SSE endpoint URL".into()))?;
                    if endpoint.origin() != base_url.origin()
                        || !endpoint.username().is_empty() || endpoint.password().is_some()
                    {
                        return Err(McpError::Transport("SSE endpoint must stay on the configured origin".into()));
                    }
                    post_url = Some(endpoint.to_string());
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
            loop {
                while let Some((event_end, delim_len)) = find_sse_event_boundary(&buf) {
                    let event_block = String::from_utf8_lossy(&buf[..event_end]).into_owned();
                    buf.drain(..event_end + delim_len);

                    let (event_type, event_data) = parse_sse_event(&event_block);

                    if (event_type == "message" || event_type.is_empty())
                        && let Ok(response) = serde_json::from_str::<JsonRpcResponse>(&event_data)
                        && let Some(id) = response.id
                    {
                        let mut map = pending_clone
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(sender) = map.as_mut().and_then(|map| map.remove(&id)) {
                            let _ = sender.send(response);
                        }
                    }
                }
                let Some(Ok(chunk)) = bytes_stream.next().await else { break };
                buf.extend_from_slice(&chunk);
            }
            // A closed or failed SSE listener can no longer deliver any
            // correlated response. Drop every sender so callers fail
            // immediately instead of waiting for the manager deadline.
            pending_clone
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
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

impl Drop for SseTransport {
    fn drop(&mut self) {
        self._listener.abort();
        self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    }
}

/// Removes an unanswered request from the correlation map even when the
/// caller is cancelled by the manager deadline.  A `tokio::sync::Mutex` cannot
/// be used here because `Drop` cannot await it.
struct PendingRequestGuard {
    pending: PendingResponses,
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
                .as_mut().map(|map| map.remove(&self.request_id));
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
            let map = map.as_mut().ok_or_else(|| McpError::Transport("SSE listener is closed".into()))?;
            if map.contains_key(&req_id) {
                return Err(McpError::Transport("Duplicate in-flight JSON-RPC request id".into()));
            }
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
            .map_err(|e| McpError::Transport(format!("POST request failed: {}", e.without_url())))?;

        super::check_http_status(response)?;

        // Wait for response from SSE stream
        let rpc_response = rx
            .await
            .map_err(|_| McpError::Transport("Response channel closed unexpectedly".into()))?;
        pending_guard.disarm();
        if rpc_response.jsonrpc != "2.0" {
            return Err(McpError::Transport("Invalid JSON-RPC response version".into()));
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
        if self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_none() {
            return Err(McpError::Transport("SSE listener is closed".into()));
        }
        let body = serde_json::to_string(req)
            .map_err(|e| McpError::Transport(format!("JSON serialize error: {}", e)))?;

        self.client
            .post(&self.post_url)
            .headers(self.headers.clone())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("Notification POST failed: {}", e.without_url())))
            .and_then(super::check_http_status)?;

        Ok(())
    }

    async fn close(&self) -> Result<(), McpError> {
        self._listener.abort();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        Ok(())
    }
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
                .is_none()
        );
        transport.close().await.unwrap();
        server.await.unwrap();
    }
}
