use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use nomifun_common::{AgentKillReason, AgentType, AppError, Confirmation, ConversationStatus, ErrorChain, TimestampMs};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{debug, error, info, warn};

use crate::runtime_state::{AgentRuntimeState, AgentRuntimeTurn};
use crate::capability::cli_process::CliAgentProcess;
use crate::manager::process_registry::register_session_process;
use crate::protocol::events::AgentStreamEvent;
use crate::protocol::send_error::AgentSendError;
use crate::types::SendMessageData;
use nomifun_api_types::OpenClawBuildExtra;

use super::config::load_openclaw_config;
use super::connection::{AuthConfig, OpenClawConnection};
use super::device_identity::load_or_create_identity;
use super::event_mapper::{
    TextFallbackState, drain_events_for_run, is_openclaw_turn_event, map_openclaw_event, openclaw_event_run_id,
};
use super::protocol::{
    ChatAbortParams, ChatSendParams, EventFrame, SessionsResetParams, SessionsResetResponse,
    SessionsResolveParams, SessionsResolveResponse, normalize_ws_url,
};

mod confirmations;
mod spawn_helpers;

use spawn_helpers::{build_spawn_config, is_port_listening, wait_for_gateway_ready};

pub const DEFAULT_GATEWAY_PORT: u16 = 18789;

const OPENCLAW_KILL_GRACE_MS: u64 = 1000;
pub(super) const GATEWAY_READY_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const GATEWAY_READY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const STOP_FINISH_FALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct OpenClawState {
    pub(super) session_key: Option<String>,
    pub(super) confirmations: Vec<Confirmation>,
    pub(super) has_messages: bool,
    pub(super) active_run_id: Option<String>,
    pub(super) turn_generation: u64,
    pub(super) runtime_turn: Option<AgentRuntimeTurn>,
    pending_run_events: Vec<EventFrame>,
    pub(super) approval_memory: HashMap<String, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GatewayRunTurn {
    run_id: String,
    runtime_turn: AgentRuntimeTurn,
}

fn gateway_turn_is_current(state: &OpenClawState, gateway_turn: &GatewayRunTurn) -> bool {
    state.active_run_id.as_deref() == Some(gateway_turn.run_id.as_str())
        && state.runtime_turn == Some(gateway_turn.runtime_turn)
}

fn admit_gateway_turn(state: &mut OpenClawState, runtime_turn: AgentRuntimeTurn) -> bool {
    let is_first = !state.has_messages;
    state.active_run_id = None;
    state.turn_generation = state.turn_generation.wrapping_add(1);
    state.runtime_turn = Some(runtime_turn);
    state.pending_run_events.clear();
    is_first
}

fn abandon_gateway_turn(state: &mut OpenClawState, runtime_turn: AgentRuntimeTurn) {
    if state.runtime_turn == Some(runtime_turn) {
        state.active_run_id = None;
        state.runtime_turn = None;
        state.pending_run_events.clear();
    }
}

async fn map_event_for_gateway_turn(
    state: &RwLock<OpenClawState>,
    text_state: &Mutex<TextFallbackState>,
    event_frame: &EventFrame,
    gateway_turn: &GatewayRunTurn,
) -> Option<Vec<AgentStreamEvent>> {
    // The read guard is intentionally held across mapper mutation. New-turn
    // admission requires the write guard before it resets `text_state`, which
    // makes check+map and reset one linearized order.
    let state = state.read().await;
    if !gateway_turn_is_current(&state, gateway_turn) {
        return None;
    }
    let session_key = state.session_key.clone();
    let mut text_state = text_state.lock().await;
    Some(map_openclaw_event(
        event_frame,
        &mut text_state,
        session_key.as_deref(),
    ))
}

pub struct OpenClawAgentManager {
    runtime: AgentRuntimeState,
    config: OpenClawBuildExtra,
    gateway_process: Option<Arc<CliAgentProcess>>,
    pub(super) connection: Arc<OpenClawConnection>,
    pub(super) state: Arc<RwLock<OpenClawState>>,
    text_state: Mutex<TextFallbackState>,
}

impl OpenClawAgentManager {
    pub async fn new(
        conversation_id: String,
        workspace: String,
        config: OpenClawBuildExtra,
        resume_session_key: Option<String>,
        data_dir: std::path::PathBuf,
    ) -> Result<Self, AppError> {
        let file_config = load_openclaw_config();

        let host = config.gateway.host.as_deref().unwrap_or("127.0.0.1");
        let port = config
            .gateway
            .port
            .or_else(|| {
                file_config
                    .as_ref()
                    .and_then(|c| c.gateway.as_ref())
                    .and_then(|g| g.port)
            })
            .unwrap_or(DEFAULT_GATEWAY_PORT);

        let gateway_process = if !config.gateway.use_external_gateway {
            let cli_path = config
                .gateway
                .cli_path
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("OpenClaw CLI path is required".into()))?;

            if !is_port_listening(host, port).await {
                let spawn_config = build_spawn_config(cli_path, &workspace, &config.gateway);
                let command_preview = spawn_config.command.display().to_string();
                let process = Arc::new(CliAgentProcess::spawn(spawn_config).await?);
                register_session_process(
                    &data_dir,
                    Arc::clone(&process),
                    conversation_id.clone(),
                    AgentType::OpenclawGateway,
                    None,
                    Some(command_preview),
                )?;

                wait_for_gateway_ready(host, port).await?;

                info!(
                    conversation_id = %conversation_id,
                    port = port,
                    "OpenClaw gateway subprocess ready"
                );

                Some(process)
            } else {
                debug!(port = port, "OpenClaw gateway already listening, skipping spawn");
                None
            }
        } else {
            None
        };

        let ws_url = normalize_ws_url(host, port);

        let identity = load_or_create_identity(None)?;

        let shared_token = config
            .gateway
            .token
            .clone()
            .or_else(|| super::config::get_gateway_auth_token(file_config.as_ref()));
        let device_token =
            super::device_auth_store::load_device_auth_token(&identity.device_id, "operator").map(|entry| entry.token);
        let password = config
            .gateway
            .password
            .clone()
            .or_else(|| super::config::get_gateway_auth_password(file_config.as_ref()));

        let auth = if shared_token.is_some() || device_token.is_some() || password.is_some() {
            Some(AuthConfig {
                token: shared_token,
                device_token,
                password,
            })
        } else {
            None
        };

        let (connection, hello) = OpenClawConnection::connect(&ws_url, auth, &identity)
            .await
            .inspect_err(|e| {
                error!(
                    conversation_id = %conversation_id,
                    url = %ws_url,
                    error = %ErrorChain(e),
                    "Failed to connect to OpenClaw gateway"
                );
            })?;

        if let Some(ref device_token) = hello.auth.device_token
        {
            super::device_auth_store::store_device_auth_token(
                &identity.device_id,
                &hello.auth.role,
                device_token,
                &hello.auth.scopes,
            );
        }

        info!(
            conversation_id = %conversation_id,
            url = %ws_url,
            "Connected to OpenClaw gateway via WebSocket"
        );

        let has_resume_key = resume_session_key.is_some();
        if has_resume_key {
            info!(
                conversation_id = %conversation_id,
                "Resuming OpenClaw session with stored session key"
            );
        }

        let runtime = AgentRuntimeState::new(conversation_id, workspace, 256);

        let manager = Self {
            runtime,
            config,
            gateway_process,
            connection: Arc::clone(&connection),
            state: Arc::new(RwLock::new(OpenClawState {
                session_key: resume_session_key,
                confirmations: Vec::new(),
                has_messages: has_resume_key,
                active_run_id: None,
                turn_generation: 0,
                runtime_turn: None,
                pending_run_events: Vec::new(),
                approval_memory: HashMap::new(),
            })),
            text_state: Mutex::new(TextFallbackState::new()),
        };

        Ok(manager)
    }

    pub fn start_event_relay(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.run_event_relay().await;
        });
    }

    async fn run_event_relay(self: Arc<Self>) {
        let mut event_rx = self.connection.subscribe_events();
        let mut close_rx = self.connection.subscribe_close();

        loop {
            tokio::select! {
                event = event_rx.recv() => match event {
                    Ok(event_frame) => {
                        self.runtime.bump_activity();
                        self.route_event_frame(event_frame).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            conversation_id = %self.runtime.conversation_id(),
                            lagged = n,
                            "OpenClaw event relay lagged"
                        );
                        self.runtime.emit_stream_broken(format!(
                            "OpenClaw event relay lost {n} buffered event(s)"
                        ));
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = close_rx.recv() => break,
            }
        }

        if self.runtime.status() == Some(ConversationStatus::Running) {
            self.runtime.emit_stream_broken("OpenClaw connection closed");
        } else {
            self.runtime.mark_transport_broken();
        }
    }

    async fn route_event_frame(&self, event_frame: EventFrame) {
        let gateway_turn = if is_openclaw_turn_event(&event_frame) {
            let Some(event_run_id) = openclaw_event_run_id(&event_frame).map(str::to_owned) else {
                warn!(
                    conversation_id = %self.runtime.conversation_id(),
                    event = %event_frame.event,
                    "Dropping turn-scoped OpenClaw event without runId"
                );
                return;
            };
            let mut state = self.state.write().await;
            match (state.active_run_id.as_deref(), state.runtime_turn) {
                (Some(active_run_id), Some(runtime_turn)) if active_run_id == event_run_id => {
                    Some(GatewayRunTurn {
                        run_id: event_run_id,
                        runtime_turn,
                    })
                }
                (Some(active_run_id), _) => {
                    debug!(
                        conversation_id = %self.runtime.conversation_id(),
                        %event_run_id,
                        %active_run_id,
                        "Dropping delayed OpenClaw event from another run"
                    );
                    return;
                }
                (None, Some(_)) if self.runtime.status() == Some(ConversationStatus::Running) =>
                {
                    const MAX_PENDING_RUN_EVENTS: usize = 256;
                    if state.pending_run_events.len() < MAX_PENDING_RUN_EVENTS {
                        state.pending_run_events.push(event_frame);
                    } else {
                        drop(state);
                        self.runtime.emit_stream_broken(
                            "OpenClaw produced too many events before acknowledging chat.send",
                        );
                    }
                    return;
                }
                (None, _) => return,
            }
        } else {
            None
        };
        self.process_event_frame(event_frame, gateway_turn).await;
    }

    async fn process_event_frame(&self, event_frame: EventFrame, gateway_turn: Option<GatewayRunTurn>) {
        let stream_events = if let Some(gateway_turn) = gateway_turn.as_ref() {
            // Keep the run/token validation guard across mutation of the
            // shared mapper state. A new turn needs this state write lock
            // before resetting TextFallbackState, so an old frame can finish
            // mapping before that reset or be rejected after it—never write
            // into the new turn between check and map.
            let Some(events) = map_event_for_gateway_turn(
                &self.state,
                &self.text_state,
                &event_frame,
                gateway_turn,
            )
            .await
            else {
                return;
            };
            events
        } else {
            let session_key = self.state.read().await.session_key.clone();
            let mut text_state = self.text_state.lock().await;
            map_openclaw_event(&event_frame, &mut text_state, session_key.as_deref())
        };

        for stream_event in stream_events {
            self.update_state_from_event(&stream_event, gateway_turn.as_ref()).await;
            if !matches!(stream_event, AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)) {
                if let Some(gateway_turn) = gateway_turn.as_ref() {
                    self.runtime.emit_for_turn(gateway_turn.runtime_turn, stream_event);
                } else {
                    self.runtime.emit(stream_event);
                }
            }
        }
    }

    async fn bind_run_to_active_turn(&self, runtime_turn: AgentRuntimeTurn, run_id: String) -> bool {
        let pending = {
            let mut state = self.state.write().await;
            if state.runtime_turn != Some(runtime_turn) {
                return false;
            }
            // Lock order is always manager state -> text mapper state. Anchor
            // the mapper before making active_run_id visible to the relay.
            self.text_state.lock().await.current_run_id = Some(run_id.clone());
            state.active_run_id = Some(run_id.clone());
            state.has_messages = true;
            drain_events_for_run(&mut state.pending_run_events, &run_id)
        };
        for event in pending {
            self.process_event_frame(
                event,
                Some(GatewayRunTurn {
                    run_id: run_id.clone(),
                    runtime_turn,
                }),
            )
            .await;
        }
        true
    }

    async fn update_state_from_event(&self, event: &AgentStreamEvent, gateway_turn: Option<&GatewayRunTurn>) {
        match event {
            AgentStreamEvent::Start(data) => {
                if let (Some(gateway_turn), Some(sid)) = (gateway_turn, data.session_id.as_ref()) {
                    let mut state = self.state.write().await;
                    if state.active_run_id.as_deref() == Some(gateway_turn.run_id.as_str())
                        && state.runtime_turn == Some(gateway_turn.runtime_turn)
                    {
                        state.session_key = Some(sid.clone());
                    }
                }
            }
            AgentStreamEvent::Finish(data) => {
                let Some(gateway_turn) = gateway_turn else { return };
                let mut state = self.state.write().await;
                let is_same_run = state.active_run_id.as_deref() == Some(gateway_turn.run_id.as_str())
                    && state.runtime_turn == Some(gateway_turn.runtime_turn);
                if is_same_run {
                    state.active_run_id = None;
                    state.runtime_turn = None;
                    if let Some(ref sid) = data.session_id {
                        state.session_key = Some(sid.clone());
                    }
                }
                drop(state);
                self.runtime.emit_finish_for_turn(
                    gateway_turn.runtime_turn,
                    data.session_id.clone(),
                    data.stop_reason,
                );
            }
            AgentStreamEvent::Error(data) => {
                let Some(gateway_turn) = gateway_turn else { return };
                let mut state = self.state.write().await;
                if state.active_run_id.as_deref() == Some(gateway_turn.run_id.as_str())
                    && state.runtime_turn == Some(gateway_turn.runtime_turn)
                {
                    state.active_run_id = None;
                    state.runtime_turn = None;
                }
                drop(state);
                self.runtime
                    .emit_error_data_for_turn(gateway_turn.runtime_turn, data.clone());
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

    async fn do_send_message(
        &self,
        is_first: bool,
        runtime_turn: AgentRuntimeTurn,
        data: SendMessageData,
    ) -> Result<(), AppError> {
        if is_first {
            self.resolve_session().await?;
        }

        let session_key = self
            .state
            .read()
            .await
            .session_key
            .clone()
            .ok_or_else(|| AppError::Internal("No active session key".into()))?;

        let params = ChatSendParams {
            session_key,
            message: data.content,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            attachments: if data.files.is_empty() {
                None
            } else {
                Some(data.files.into_iter().map(|f| json!(f)).collect())
            },
        };

        let response = self
            .connection
            .request::<Value>("chat.send", serde_json::to_value(params).unwrap_or_default())
            .await?;
        let active_run_id = response
            .get("runId")
            .or_else(|| response.get("run_id"))
            .and_then(Value::as_str)
            .filter(|run_id| !run_id.trim().is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(|| AppError::BadGateway("OpenClaw chat.send returned no runId".into()))?;
        self.bind_run_to_active_turn(runtime_turn, active_run_id).await;

        Ok(())
    }

    /// Resolve gateway session: try to resume an existing session first,
    /// then fall back to creating a new one via sessions.reset.
    async fn resolve_session(&self) -> Result<(), AppError> {
        let resume_key = self.state.read().await.session_key.clone();

        if let Some(ref key) = resume_key {
            match self
                .connection
                .request::<SessionsResolveResponse>(
                    "sessions.resolve",
                    serde_json::to_value(SessionsResolveParams { key: key.clone() }).unwrap_or_default(),
                )
                .await
            {
                Ok(resp) => {
                    if resp.ok == Some(false) {
                        warn!(
                            conversation_id = %self.runtime.conversation_id(),
                            "OpenClaw sessions.resolve reported a missing session, falling back to sessions.reset"
                        );
                    } else if let Some(resolved_key) = resp.key {
                        self.state.write().await.session_key = Some(resolved_key.clone());
                        info!(
                            conversation_id = %self.runtime.conversation_id(),
                            session_key = %resolved_key,
                            "Resumed OpenClaw session via sessions.resolve"
                        );
                        return Ok(());
                    } else {
                        warn!(
                            conversation_id = %self.runtime.conversation_id(),
                            "OpenClaw sessions.resolve returned no key, falling back to sessions.reset"
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        conversation_id = %self.runtime.conversation_id(),
                        error = %ErrorChain(&e),
                        "Failed to resume OpenClaw session, falling back to sessions.reset"
                    );
                }
            }
        }

        let resp: SessionsResetResponse = self
            .connection
            .request(
                "sessions.reset",
                serde_json::to_value(SessionsResetParams {
                    key: self.runtime.conversation_id().to_owned(),
                    reason: "new".into(),
                })
                .unwrap_or_default(),
            )
            .await?;

        let entry_session_id = resp
            .entry
            .as_ref()
            .and_then(|entry| entry.get("sessionId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let key = resp
            .key
            .or(resp.session_id)
            .or(entry_session_id)
            .ok_or_else(|| AppError::Internal("OpenClaw sessions.reset returned no session key".into()))?;
        self.state.write().await.session_key = Some(key);

        Ok(())
    }

    /// Clear the conversation context ("release model context"): forget the
    /// gateway session key and pending confirmations so the next send is
    /// treated as a first message — `resolve_session` then falls straight to
    /// `sessions.reset`, allocating a brand-new gateway session with no
    /// history. Robust even when the gateway is momentarily disconnected: the
    /// reset happens lazily on the next send.
    pub async fn clear_context(&self) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            "Clearing OpenClaw context"
        );
        let mut state = self.state.write().await;
        state.session_key = None;
        state.has_messages = false;
        state.active_run_id = None;
        state.runtime_turn = None;
        state.pending_run_events.clear();
        state.turn_generation = state.turn_generation.wrapping_add(1);
        state.confirmations.clear();
        Ok(())
    }

    pub async fn get_diagnostics(&self) -> Value {
        let state = self.state.read().await;
        let host = self.config.gateway.host.as_deref().unwrap_or("127.0.0.1");
        let port = self.config.gateway.port.unwrap_or(DEFAULT_GATEWAY_PORT);

        json!({
            "workspace": self.runtime.workspace(),
            "backend": serde_json::to_value(&self.config.backend).unwrap_or_default(),
            "agentName": self.config.agent_name,
            "cliPath": self.config.gateway.cli_path,
            "gatewayHost": host,
            "gatewayPort": port,
            "conversationId": self.runtime.conversation_id(),
            "isConnected": self.connection.is_connected(),
            "hasActiveSession": state.session_key.is_some(),
            "sessionKey": state.session_key,
        })
    }
}

#[cfg(test)]
mod turn_lifecycle_tests {
    use super::*;

    fn state_for_turn(turn: AgentRuntimeTurn, run_id: Option<&str>) -> OpenClawState {
        OpenClawState {
            session_key: Some("session-1".into()),
            confirmations: Vec::new(),
            has_messages: run_id.is_some(),
            active_run_id: run_id.map(str::to_owned),
            turn_generation: 1,
            runtime_turn: Some(turn),
            pending_run_events: Vec::new(),
            approval_memory: HashMap::new(),
        }
    }

    #[test]
    fn first_send_failure_does_not_poison_next_send_admission() {
        let runtime = AgentRuntimeState::new("openclaw-first-send", "/workspace", 8);
        let first_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let mut state = state_for_turn(first_turn, None);
        state.has_messages = false;

        assert!(admit_gateway_turn(&mut state, first_turn));
        assert!(!state.has_messages, "admission alone must not claim a successful message");
        abandon_gateway_turn(&mut state, first_turn);

        let second_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        assert!(
            admit_gateway_turn(&mut state, second_turn),
            "a failed first chat.send must retry session resolution on the next turn"
        );
    }

    #[tokio::test]
    async fn old_frame_mapping_is_linearized_before_new_turn_text_reset() {
        let runtime = AgentRuntimeState::new("openclaw-map-order", "/workspace", 8);
        let old_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let state = Arc::new(RwLock::new(state_for_turn(old_turn, Some("run-old"))));
        let text_state = Arc::new(Mutex::new(TextFallbackState::new()));
        let held_text = text_state.lock().await;
        let old_binding = GatewayRunTurn {
            run_id: "run-old".into(),
            runtime_turn: old_turn,
        };
        let old_frame = EventFrame {
            event: "chat".into(),
            payload: Some(json!({
                "runId": "run-old",
                "state": "delta",
                "deltaText": "stale"
            })),
            seq: None,
        };

        let state_for_old = Arc::clone(&state);
        let text_for_old = Arc::clone(&text_state);
        let old_mapper = tokio::spawn(async move {
            map_event_for_gateway_turn(&state_for_old, &text_for_old, &old_frame, &old_binding).await
        });

        // Wait until the mapper has acquired the state read guard and is
        // blocked on the text mutex we hold.
        let mut mapper_holds_state = false;
        for _ in 0..100 {
            if state.try_write().is_err() {
                mapper_holds_state = true;
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(mapper_holds_state, "old mapper never reached its linearization guard");

        let new_turn = runtime.reset_for_new_turn(ConversationStatus::Running);
        let state_for_new = Arc::clone(&state);
        let text_for_new = Arc::clone(&text_state);
        let new_admission = tokio::spawn(async move {
            let mut state = state_for_new.write().await;
            admit_gateway_turn(&mut state, new_turn);
            let mut text = text_for_new.lock().await;
            text.reset_for_new_turn();
            text.current_run_id = Some("run-new".into());
        });

        drop(held_text);
        assert!(old_mapper.await.unwrap().is_some());
        new_admission.await.unwrap();

        let text = text_state.lock().await;
        assert_eq!(text.current_run_id.as_deref(), Some("run-new"));
        assert!(text.accumulated_text.is_empty(), "old run text leaked past the new-turn reset");
    }
}

#[async_trait::async_trait]
impl crate::runtime_handle::AgentRuntimeControl for OpenClawAgentManager {
    fn agent_type(&self) -> AgentType {
        AgentType::OpenclawGateway
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

    fn is_transport_healthy(&self) -> bool {
        self.runtime.is_transport_healthy()
    }

    fn last_activity_at(&self) -> TimestampMs {
        self.runtime.last_activity_at()
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.runtime.subscribe()
    }

    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        self.runtime.bump_activity();
        if !self.runtime.is_transport_healthy() {
            return Err(AgentSendError::stream_broken(
                "OpenClaw's permanent connection relay is no longer running",
            ));
        }

        let runtime_turn = self.runtime.reset_for_new_turn(ConversationStatus::Running);
        let is_first = {
            let mut state = self.state.write().await;
            admit_gateway_turn(&mut state, runtime_turn)
        };
        if !self.runtime.is_transport_healthy() {
            let error = AgentSendError::stream_broken(
                "OpenClaw's connection relay stopped during turn admission",
            );
            let mut state = self.state.write().await;
            abandon_gateway_turn(&mut state, runtime_turn);
            drop(state);
            self.runtime
                .emit_error_data_for_turn(runtime_turn, error.stream_error().clone());
            return Err(error);
        }

        {
            let mut text_state = self.text_state.lock().await;
            text_state.reset_for_new_turn();
        }

        match self.do_send_message(is_first, runtime_turn, data).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut state = self.state.write().await;
                abandon_gateway_turn(&mut state, runtime_turn);
                drop(state);
                error!(
                    conversation_id = %self.runtime.conversation_id(),
                    error = %ErrorChain(&err),
                    "OpenClaw send_message failed, emitting terminal Error"
                );
                let send_error = AgentSendError::from_app_error(err);
                self.runtime
                    .emit_error_data_for_turn(runtime_turn, send_error.stream_error().clone());
                Err(send_error)
            }
        }
    }

    async fn cancel(&self) -> Result<(), AppError> {
        let (session_key, run_id, turn_generation, runtime_turn) = {
            let state = self.state.read().await;
            (
                state.session_key.clone(),
                state.active_run_id.clone(),
                state.turn_generation,
                state.runtime_turn,
            )
        };
        if let Some(ref key) = session_key {
            let params = ChatAbortParams {
                session_key: key.clone(),
                run_id,
            };
            let _ = self
                .connection
                .request::<Value>("chat.abort", serde_json::to_value(params).unwrap_or_default())
                .await;
        }

        {
            let mut state = self.state.write().await;
            state.confirmations.clear();
        }

        let runtime = self.runtime.clone();
        let state = Arc::clone(&self.state);
        let conversation_id = self.runtime.conversation_id().to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(STOP_FINISH_FALLBACK_TIMEOUT).await;
            let is_same_turn = {
                let mut state = state.write().await;
                let matches = state.turn_generation == turn_generation && state.runtime_turn == runtime_turn;
                if matches {
                    state.active_run_id = None;
                    state.runtime_turn = None;
                    state.pending_run_events.clear();
                }
                matches
            };
            let needs_fallback = is_same_turn && runtime.status() == Some(ConversationStatus::Running);
            if needs_fallback {
                warn!(
                    conversation_id = %conversation_id,
                    "Gateway did not send abort event within timeout, emitting fallback Finish"
                );
                if let Some(runtime_turn) = runtime_turn {
                    runtime.emit_finish_for_turn(
                        runtime_turn,
                        None,
                        Some(crate::protocol::events::TurnStopReason::Cancelled),
                    );
                }
            }
        });

        Ok(())
    }

    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            "Killing OpenClaw agent"
        );

        let connection = Arc::clone(&self.connection);
        tokio::spawn(async move {
            connection.close().await;
        });

        if let Some(ref process) = self.gateway_process {
            let process = Arc::clone(process);
            let grace = Duration::from_millis(OPENCLAW_KILL_GRACE_MS);
            tokio::spawn(async move {
                if let Err(e) = process.kill(grace).await {
                    error!(error = %ErrorChain(&e), "Failed to kill OpenClaw gateway process");
                }
            });
        }

        if reason == Some(AgentKillReason::UserCancelled) {
            if let Ok(state) = self.state.try_read()
                && let Some(runtime_turn) = state.runtime_turn
            {
                self.runtime.emit_finish_for_turn(
                    runtime_turn,
                    None,
                    Some(crate::protocol::events::TurnStopReason::Cancelled),
                );
            }
        } else if self.runtime.status() == Some(ConversationStatus::Running) {
            self.runtime.emit_error(format!("OpenClaw agent was terminated ({reason:?})"));
        }

        Ok(())
    }
}

impl OpenClawAgentManager {
    pub fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        info!(
            conversation_id = %self.runtime.conversation_id(),
            ?reason,
            "Killing OpenClaw agent and waiting for shutdown"
        );
        let connection = Arc::clone(&self.connection);
        let process = self.gateway_process.clone();
        let grace = Duration::from_millis(OPENCLAW_KILL_GRACE_MS);
        Box::pin(async move {
            connection.close().await;
            if let Some(process) = process {
                let _ = process.kill(grace).await;
            }
        })
    }
}
