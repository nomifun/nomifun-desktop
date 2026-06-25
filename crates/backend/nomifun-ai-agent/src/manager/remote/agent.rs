use std::collections::HashMap;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use nomifun_common::{
    AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, ErrorChain, RemoteAgentStatus, TimestampMs,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

use crate::agent_runtime::AgentRuntime;
use crate::protocol::events::AgentStreamEvent;
use crate::protocol::send_error::AgentSendError;
use crate::types::SendMessageData;

/// Internal mutable state for the Remote agent.
struct RemoteState {
    session_key: Option<String>,
    confirmations: Vec<Confirmation>,
    has_messages: bool,
    approval_memory: HashMap<String, bool>,
    connection_status: RemoteAgentStatus,
}

/// Configuration for connecting to a remote agent.
#[derive(Debug, Clone)]
pub struct RemoteAgentConfig {
    pub remote_agent_id: String,
    pub url: String,
    pub auth_type: String,
    pub auth_token: Option<String>,
    pub allow_insecure: bool,
}

/// Manages a Remote Agent via WebSocket connection.
///
/// Remote agents communicate over WebSocket, reusing the OpenClaw Gateway
/// connection protocol. The Rust implementation owns the WebSocket connection
/// directly (no CLI subprocess).
pub struct RemoteAgentManager {
    runtime: AgentRuntime,
    remote_config: RemoteAgentConfig,
    state: RwLock<RemoteState>,
    /// WebSocket sink for sending messages, wrapped in Mutex for concurrency.
    ws_sink: Mutex<
        Option<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
                Message,
            >,
        >,
    >,
    /// Handle to the WebSocket reader task.
    _reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RemoteAgentManager {
    /// Create a new Remote agent by establishing a WebSocket connection.
    pub async fn new(
        conversation_id: String,
        workspace: String,
        remote_config: RemoteAgentConfig,
    ) -> Result<Self, AppError> {
        let runtime = AgentRuntime::new(conversation_id, workspace, 256);

        let manager = Self {
            runtime,
            remote_config,
            state: RwLock::new(RemoteState {
                session_key: None,
                confirmations: Vec::new(),
                has_messages: false,
                approval_memory: HashMap::new(),
                connection_status: RemoteAgentStatus::Unknown,
            }),
            ws_sink: Mutex::new(None),
            _reader_handle: Mutex::new(None),
        };

        Ok(manager)
    }

    /// Connect to the remote WebSocket endpoint and start the reader task.
    pub async fn connect(self: &Arc<Self>) -> Result<(), AppError> {
        let url = &self.remote_config.url;

        let (ws_stream, _response) = tokio_tungstenite::connect_async(url).await.map_err(|e| {
            error!(url = url, error = %ErrorChain(&e), "Failed to connect to remote agent");
            AppError::Internal(format!("WebSocket connection failed: {e}"))
        })?;

        info!(
            conversation_id = %self.runtime.conversation_id(),
            url = url,
            "Connected to remote agent"
        );

        let (sink, stream) = ws_stream.split();

        // Store the sink for sending messages
        *self.ws_sink.lock().await = Some(sink);

        // Update connection status
        {
            let mut state = self.state.write().await;
            state.connection_status = RemoteAgentStatus::Connected;
        }

        // Start reader task
        let this = Arc::clone(self);
        let reader_handle = tokio::spawn(async move {
            this.run_ws_reader(stream).await;
        });

        *self._reader_handle.lock().await = Some(reader_handle);

        Ok(())
    }

    /// Read messages from the WebSocket and process them.
    async fn run_ws_reader(
        self: Arc<Self>,
        mut stream: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        >,
    ) {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    self.runtime.bump_activity();
                    match serde_json::from_str::<Value>(&text) {
                        Ok(raw_json) => self.handle_raw_event(raw_json).await,
                        Err(e) => {
                            debug!(
                                conversation_id = %self.runtime.conversation_id(),
                                error = %ErrorChain(&e),
                                "Non-JSON WebSocket message, skipping"
                            );
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    debug!(
                        conversation_id = %self.runtime.conversation_id(),
                        "Remote WebSocket closed"
                    );
                    break;
                }
                Err(e) => {
                    warn!(
                        conversation_id = %self.runtime.conversation_id(),
                        error = %ErrorChain(&e),
                        "WebSocket read error"
                    );
                    break;
                }
                _ => {} // Ignore ping/pong/binary
            }
        }

        // Connection closed — update connection_status and ensure terminal agent status.
        {
            let mut state = self.state.write().await;
            state.connection_status = RemoteAgentStatus::Error;
        }
        if self.runtime.status() == Some(ConversationStatus::Running) {
            self.runtime.transition_to(ConversationStatus::Finished);
        }
    }

    async fn handle_raw_event(&self, raw: Value) {
        let stream_event = match serde_json::from_value::<AgentStreamEvent>(raw.clone()) {
            Ok(event) => event,
            Err(_) => {
                debug!(
                    conversation_id = %self.runtime.conversation_id(),
                    "Unrecognized remote event, skipping"
                );
                return;
            }
        };

        self.update_state_from_event(&stream_event).await;
        self.runtime.emit(stream_event);
    }

    async fn update_state_from_event(&self, event: &AgentStreamEvent) {
        match event {
            AgentStreamEvent::Start(data) => {
                self.runtime.transition_to(ConversationStatus::Running);
                if let Some(ref sid) = data.session_id {
                    let mut state = self.state.write().await;
                    state.session_key = Some(sid.clone());
                }
            }
            AgentStreamEvent::Finish(data) => {
                self.runtime.transition_to(ConversationStatus::Finished);
                if let Some(ref sid) = data.session_id {
                    let mut state = self.state.write().await;
                    state.session_key = Some(sid.clone());
                }
            }
            AgentStreamEvent::Error(_) => {
                self.runtime.transition_to(ConversationStatus::Finished);
            }
            AgentStreamEvent::AcpPermission(data) => {
                if let Some(conf) = data.as_confirmation() {
                    let mut guard = self.state.write().await;
                    if let Some(existing) = guard.confirmations.iter_mut().find(|c| c.call_id == conf.call_id) {
                        *existing = conf;
                    } else {
                        guard.confirmations.push(conf);
                    }
                }
            }
            _ => {}
        }
    }

    /// Send a JSON message over the WebSocket.
    async fn ws_send(&self, payload: &Value) -> Result<(), AppError> {
        let text = serde_json::to_string(payload)
            .map_err(|e| AppError::Internal(format!("Failed to serialize WebSocket message: {e}")))?;

        let mut guard = self.ws_sink.lock().await;
        let sink = guard
            .as_mut()
            .ok_or_else(|| AppError::Internal("WebSocket not connected".into()))?;

        sink.send(Message::Text(text.into())).await.map_err(|e| {
            error!(
                conversation_id = %self.runtime.conversation_id(),
                error = %ErrorChain(&e),
                "Failed to send WebSocket message"
            );
            AppError::Internal(format!("WebSocket send failed: {e}"))
        })
    }

    /// Get the connection status.
    pub async fn connection_status(&self) -> RemoteAgentStatus {
        self.state.read().await.connection_status
    }
}

use crate::shared_kernel::approval_key;

#[async_trait::async_trait]
impl crate::agent_task::IAgentTask for RemoteAgentManager {
    fn agent_type(&self) -> AgentType {
        AgentType::Remote
    }

    fn conversation_id(&self) -> &str {
        self.runtime.conversation_id()
    }

    fn workspace(&self) -> &str {
        self.runtime.workspace()
    }

    fn status(&self) -> Option<ConversationStatus> {
        self.runtime.status()
    }

    fn last_activity_at(&self) -> TimestampMs {
        self.runtime.last_activity_at()
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.runtime.subscribe()
    }

    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        self.runtime.bump_activity();

        let is_first = {
            let mut state = self.state.write().await;
            let first = !state.has_messages;
            state.has_messages = true;
            first
        };
        self.runtime.transition_to(ConversationStatus::Running);

        if is_first {
            // First message: create new session via sessionsReset
            let payload = json!({
                "type": "sessionsReset",
                "data": {
                    "conversationId": self.runtime.conversation_id(),
                    "message": data.content,
                    "msgId": data.msg_id,
                }
            });
            match self.ws_send(&payload).await {
                Ok(()) => Ok(()),
                Err(err) => {
                    error!(
                        conversation_id = %self.runtime.conversation_id(),
                        error = %ErrorChain(&err),
                        "Remote send_message failed, emitting Error"
                    );
                    let send_error = AgentSendError::from_app_error(err);
                    self.runtime.emit_error_data(send_error.stream_error().clone());
                    Err(send_error)
                }
            }
        } else {
            // Subsequent messages: try to resume session
            let session_key = self.state.read().await.session_key.clone();
            let mut payload = json!({
                "type": "sendMessage",
                "data": {
                    "message": data.content,
                    "msgId": data.msg_id,
                }
            });
            if let Some(ref key) = session_key {
                payload["data"]["sessionKey"] = json!(key);
            }
            if !data.files.is_empty() {
                payload["data"]["files"] = json!(data.files);
            }
            match self.ws_send(&payload).await {
                Ok(()) => Ok(()),
                Err(err) => {
                    error!(
                        conversation_id = %self.runtime.conversation_id(),
                        error = %ErrorChain(&err),
                        "Remote send_message failed, emitting Error"
                    );
                    let send_error = AgentSendError::from_app_error(err);
                    self.runtime.emit_error_data(send_error.stream_error().clone());
                    Err(send_error)
                }
            }
        }
    }

    async fn cancel(&self) -> Result<(), AppError> {
        if self.ws_sink.lock().await.is_none() {
            return Err(AppError::Conflict("WebSocket not connected; nothing to cancel".into()));
        }
        let payload = json!({ "type": "session/cancel", "data": {} });
        self.ws_send(&payload).await?;

        let mut state = self.state.write().await;
        state.confirmations.clear();
        Ok(())
    }

    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            "Killing Remote agent"
        );

        // Drop the WebSocket sink to close the connection.
        // We can't move the Mutex into a spawned task, so we clear it inline
        // using try_lock (non-blocking). If the lock is held, the connection
        // will close when the holder drops it.
        if let Ok(mut guard) = self.ws_sink.try_lock() {
            *guard = None;
        }

        Ok(())
    }
}

impl RemoteAgentManager {
    pub fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let _ = crate::agent_task::IAgentTask::kill(self, reason);
        Box::pin(std::future::ready(()))
    }
}

/// Remote-specific operations reached through `AgentInstance::Remote(..)`.
impl RemoteAgentManager {
    pub fn confirm(&self, _msg_id: &str, call_id: &str, _data: Value, always_allow: bool) -> Result<(), AppError> {
        if let Ok(mut state) = self.state.try_write() {
            if always_allow && let Some(conf) = state.confirmations.iter().find(|c| c.call_id == call_id) {
                let key = approval_key(conf.action.as_deref(), conf.command_type.as_deref());
                state.approval_memory.insert(key, true);
            }
            state.confirmations.retain(|c| c.call_id != call_id);
        }

        // WebSocket send for confirmation will be fully wired in Phase 6.15 integration
        // via a command channel that avoids &self lifetime issues in spawned tasks.
        warn!(
            conversation_id = %self.runtime.conversation_id(),
            call_id = call_id,
            "Remote agent confirm: WebSocket send deferred to integration phase"
        );

        Ok(())
    }

    pub fn get_confirmations(&self) -> Vec<Confirmation> {
        self.state
            .try_read()
            .map(|g| g.confirmations.clone())
            .unwrap_or_default()
    }

    /// Clear the conversation context ("release model context"): forget the
    /// remote session key and pending confirmations and re-arm
    /// `has_messages = false` so the next `send_message` takes the
    /// `sessionsReset` branch, creating a brand-new remote session with no
    /// history.
    pub async fn clear_context(&self) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            "Clearing Remote context"
        );
        let mut state = self.state.write().await;
        state.session_key = None;
        state.has_messages = false;
        state.confirmations.clear();
        Ok(())
    }

    pub fn check_approval(&self, action: &str, command_type: Option<&str>) -> bool {
        self.state
            .try_read()
            .map(|g| {
                let key = approval_key(Some(action), command_type);
                g.approval_memory.get(&key).copied().unwrap_or(false)
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_key_formats_correctly() {
        assert_eq!(approval_key(Some("exec"), Some("curl")), "exec:curl");
        assert_eq!(approval_key(Some("exec"), None), "exec");
        assert_eq!(approval_key(None, None), "");
    }

    #[test]
    fn remote_agent_config_clone() {
        let config = RemoteAgentConfig {
            remote_agent_id: "ra-1".into(),
            url: "wss://example.com".into(),
            auth_type: "bearer".into(),
            auth_token: Some("token".into()),
            allow_insecure: false,
        };
        let cloned = config.clone();
        assert_eq!(cloned.remote_agent_id, "ra-1");
        assert_eq!(cloned.url, "wss://example.com");
    }
}
