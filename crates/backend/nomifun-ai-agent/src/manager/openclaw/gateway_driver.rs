//! Shared session driver for the local and remote OpenClaw gateway managers.
//!
//! Both `OpenClawAgentManager` and `RemoteAgentManager` speak the same v4
//! gateway protocol; only construction, teardown and connection-status
//! reporting differ. The turn/event driver core — run/turn identity
//! validation, event routing, chat.send runId binding, and session
//! resolution — lives here exactly once so concurrency fixes cannot drift
//! between the two copies.

use std::collections::HashMap;

use nomifun_common::{AppError, Confirmation, ConversationStatus, ErrorChain};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast, watch};
use tracing::{debug, info, warn};

use crate::protocol::events::AgentStreamEvent;
use crate::runtime_state::{AgentRuntimeState, AgentRuntimeTurn};
use crate::types::{SendMessageData, inject_runtime_preset_context};

use super::connection::OpenClawConnection;
use super::event_mapper::{
    TextFallbackState, drain_events_for_run, is_openclaw_turn_event, map_openclaw_event, openclaw_event_run_id,
};
use super::protocol::{
    ChatSendParams, EventFrame, SessionsResetParams, SessionsResetResponse, SessionsResolveParams,
    SessionsResolveResponse,
};
use super::teardown::{GatewayRunTurn, GatewayTeardownTarget};

/// Internal mutable session state shared by the gateway managers.
pub(crate) struct GatewayState {
    pub(crate) session_key: Option<String>,
    pub(crate) confirmations: Vec<Confirmation>,
    pub(crate) has_messages: bool,
    pub(crate) active_run_id: Option<String>,
    pub(crate) turn_generation: u64,
    pub(crate) runtime_turn: Option<AgentRuntimeTurn>,
    pub(crate) pending_run_events: Vec<EventFrame>,
    pub(crate) approval_memory: HashMap<String, bool>,
}

impl GatewayState {
    /// Fresh state for a new runtime activation.
    ///
    /// `has_messages` is scoped to this runtime activation, not the durable
    /// remote session. The first local prompt validates a resumed key and
    /// replays immutable preset context exactly once. This also repairs
    /// sessions created by older builds that dropped it.
    pub(crate) fn new(resume_session_key: Option<String>) -> Self {
        Self {
            session_key: resume_session_key,
            confirmations: Vec::new(),
            has_messages: false,
            active_run_id: None,
            turn_generation: 0,
            runtime_turn: None,
            pending_run_events: Vec::new(),
            approval_memory: HashMap::new(),
        }
    }
}

pub(crate) fn gateway_turn_is_current(state: &GatewayState, gateway_turn: &GatewayRunTurn) -> bool {
    state.active_run_id.as_deref() == Some(gateway_turn.run_id.as_str())
        && state.turn_generation == gateway_turn.turn_generation
        && state.runtime_turn == Some(gateway_turn.runtime_turn)
}

pub(crate) fn teardown_target_from_state(
    state: &GatewayState,
    label: &str,
) -> Result<Option<GatewayTeardownTarget>, AppError> {
    match (state.runtime_turn, state.active_run_id.as_ref()) {
        (None, None) => Ok(None),
        (None, Some(run_id)) => Err(AppError::Internal(format!(
            "{label} lifecycle invariant violated: run {run_id} has no runtime turn"
        ))),
        (Some(runtime_turn), run_id) => {
            let session_key = state.session_key.clone().ok_or_else(|| {
                AppError::Conflict(format!(
                    "{label} has an admitted turn but no session key; chat.abort cannot identify it"
                ))
            })?;
            Ok(Some(GatewayTeardownTarget {
                session_key,
                run_id: run_id.cloned(),
                turn_generation: state.turn_generation,
                runtime_turn,
            }))
        }
    }
}

pub(crate) fn admit_gateway_turn(state: &mut GatewayState, runtime_turn: AgentRuntimeTurn) -> bool {
    let is_first = !state.has_messages;
    state.active_run_id = None;
    state.turn_generation = state.turn_generation.wrapping_add(1);
    state.runtime_turn = Some(runtime_turn);
    state.pending_run_events.clear();
    is_first
}

pub(crate) fn abandon_gateway_turn(state: &mut GatewayState, runtime_turn: AgentRuntimeTurn) {
    if state.runtime_turn == Some(runtime_turn) {
        state.active_run_id = None;
        state.runtime_turn = None;
        state.pending_run_events.clear();
    }
}

pub(crate) async fn map_event_for_gateway_turn(
    state: &RwLock<GatewayState>,
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

/// Access to the pieces of a gateway manager the shared driver operates on.
///
/// `label()` distinguishes the two managers in logs and error strings
/// ("OpenClaw" vs "Remote OpenClaw").
pub(crate) trait GatewayCore: Send + Sync {
    fn runtime(&self) -> &AgentRuntimeState;
    fn connection(&self) -> &OpenClawConnection;
    fn state(&self) -> &RwLock<GatewayState>;
    fn text_state(&self) -> &Mutex<TextFallbackState>;
    fn terminal_proof_tx(&self) -> &watch::Sender<Option<GatewayRunTurn>>;
    fn label(&self) -> &'static str;
    /// Immutable conversation preset replayed with the first prompt of this
    /// runtime activation.
    fn preset_context(&self) -> Option<&str>;
}

/// Run the permanent event relay loop until the connection closes or the
/// event channel is lost. The caller applies its manager-specific epilogue
/// (e.g. remote connection-status bookkeeping) before `mark_relay_closed`.
pub(crate) async fn relay_events<C: GatewayCore>(core: &C) {
    let mut event_rx = core.connection().subscribe_events();
    let mut close_rx = core.connection().subscribe_close();
    loop {
        tokio::select! {
            event = event_rx.recv() => match event {
                Ok(event_frame) => {
                    core.runtime().bump_activity();
                    route_event_frame(core, event_frame).await;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(
                        conversation_id = %core.runtime().conversation_id(),
                        lagged = n,
                        "{} event relay lagged",
                        core.label()
                    );
                    core.runtime().emit_stream_broken(format!(
                        "{} event relay lost {n} buffered event(s)",
                        core.label()
                    ));
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = close_rx.recv() => break,
        }
    }
}

pub(crate) fn mark_relay_closed<C: GatewayCore>(core: &C) {
    if core.runtime().status() == Some(ConversationStatus::Running) {
        core.runtime()
            .emit_stream_broken(format!("{} connection closed", core.label()));
    } else {
        core.runtime().mark_transport_broken();
    }
}

pub(crate) async fn route_event_frame<C: GatewayCore>(core: &C, event_frame: EventFrame) {
    let gateway_turn = if is_openclaw_turn_event(&event_frame) {
        let Some(event_run_id) = openclaw_event_run_id(&event_frame).map(str::to_owned) else {
            warn!(
                conversation_id = %core.runtime().conversation_id(),
                event = %event_frame.event,
                "Dropping turn-scoped {} event without runId",
                core.label()
            );
            return;
        };
        let mut state = core.state().write().await;
        match (state.active_run_id.as_deref(), state.runtime_turn) {
            (Some(active_run_id), Some(runtime_turn)) if active_run_id == event_run_id => {
                Some(GatewayRunTurn {
                    run_id: event_run_id,
                    turn_generation: state.turn_generation,
                    runtime_turn,
                })
            }
            (Some(active_run_id), _) => {
                debug!(
                    conversation_id = %core.runtime().conversation_id(),
                    %event_run_id,
                    %active_run_id,
                    "Dropping delayed {} event from another run",
                    core.label()
                );
                return;
            }
            (None, Some(_)) if core.runtime().status() == Some(ConversationStatus::Running) => {
                const MAX_PENDING_RUN_EVENTS: usize = 256;
                if state.pending_run_events.len() < MAX_PENDING_RUN_EVENTS {
                    state.pending_run_events.push(event_frame);
                } else {
                    drop(state);
                    core.runtime().emit_stream_broken(format!(
                        "{} produced too many events before acknowledging chat.send",
                        core.label()
                    ));
                }
                return;
            }
            (None, _) => return,
        }
    } else {
        None
    };
    process_event_frame(core, event_frame, gateway_turn).await;
}

pub(crate) async fn process_event_frame<C: GatewayCore>(
    core: &C,
    event_frame: EventFrame,
    gateway_turn: Option<GatewayRunTurn>,
) {
    let stream_events = if let Some(gateway_turn) = gateway_turn.as_ref() {
        // Keep the run/token validation guard across mutation of the
        // shared mapper state. A new turn needs this state write lock
        // before resetting TextFallbackState, so an old frame can finish
        // mapping before that reset or be rejected after it—never write
        // into the new turn between check and map.
        let Some(events) =
            map_event_for_gateway_turn(core.state(), core.text_state(), &event_frame, gateway_turn).await
        else {
            return;
        };
        events
    } else {
        let session_key = core.state().read().await.session_key.clone();
        let mut text_state = core.text_state().lock().await;
        map_openclaw_event(&event_frame, &mut text_state, session_key.as_deref())
    };

    for stream_event in stream_events {
        update_state_from_event(core, &stream_event, gateway_turn.as_ref()).await;
        if !matches!(stream_event, AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)) {
            if let Some(gateway_turn) = gateway_turn.as_ref() {
                core.runtime().emit_for_turn(gateway_turn.runtime_turn, stream_event);
            } else {
                core.runtime().emit(stream_event);
            }
        }
    }
}

pub(crate) async fn bind_run_to_active_turn<C: GatewayCore>(
    core: &C,
    runtime_turn: AgentRuntimeTurn,
    run_id: String,
) -> bool {
    let (pending, turn_generation) = {
        let mut state = core.state().write().await;
        if state.runtime_turn != Some(runtime_turn) {
            return false;
        }
        let turn_generation = state.turn_generation;
        // Lock order is always manager state -> text mapper state. Anchor
        // the mapper before making active_run_id visible to the relay.
        core.text_state().lock().await.current_run_id = Some(run_id.clone());
        state.active_run_id = Some(run_id.clone());
        state.has_messages = true;
        (
            drain_events_for_run(&mut state.pending_run_events, &run_id),
            turn_generation,
        )
    };
    for event in pending {
        process_event_frame(
            core,
            event,
            Some(GatewayRunTurn {
                run_id: run_id.clone(),
                turn_generation,
                runtime_turn,
            }),
        )
        .await;
    }
    true
}

pub(crate) async fn update_state_from_event<C: GatewayCore>(
    core: &C,
    event: &AgentStreamEvent,
    gateway_turn: Option<&GatewayRunTurn>,
) {
    match event {
        AgentStreamEvent::Start(data) => {
            if let (Some(gateway_turn), Some(sid)) = (gateway_turn, data.session_id.as_ref()) {
                let mut state = core.state().write().await;
                if gateway_turn_is_current(&state, gateway_turn) {
                    state.session_key = Some(sid.clone());
                }
            }
        }
        AgentStreamEvent::Finish(data) => {
            let Some(gateway_turn) = gateway_turn else { return };
            let mut state = core.state().write().await;
            let is_same_run = gateway_turn_is_current(&state, gateway_turn);
            if is_same_run {
                state.active_run_id = None;
                state.runtime_turn = None;
                if let Some(ref sid) = data.session_id {
                    state.session_key = Some(sid.clone());
                }
            }
            drop(state);
            if is_same_run {
                core.terminal_proof_tx().send_replace(Some(gateway_turn.clone()));
            }
            core.runtime().emit_finish_for_turn(
                gateway_turn.runtime_turn,
                data.session_id.clone(),
                data.stop_reason,
            );
        }
        AgentStreamEvent::Error(data) => {
            let Some(gateway_turn) = gateway_turn else { return };
            let mut state = core.state().write().await;
            let is_same_run = gateway_turn_is_current(&state, gateway_turn);
            if is_same_run {
                state.active_run_id = None;
                state.runtime_turn = None;
            }
            drop(state);
            if is_same_run {
                core.terminal_proof_tx().send_replace(Some(gateway_turn.clone()));
            }
            core.runtime()
                .emit_error_data_for_turn(gateway_turn.runtime_turn, data.clone());
        }
        AgentStreamEvent::AcpPermission(data) => {
            if let Some(conf) = data.as_confirmation() {
                let mut state = core.state().write().await;
                if let Some(existing) = state.confirmations.iter_mut().find(|c| c.call_id == conf.call_id) {
                    *existing = conf;
                } else {
                    state.confirmations.push(conf);
                }
            }
        }
        _ => {}
    }
}

/// Resolve the gateway session: try to resume an existing session first,
/// then fall back to creating a new one via sessions.reset.
pub(crate) async fn resolve_session<C: GatewayCore>(core: &C) -> Result<(), AppError> {
    let resume_key = core.state().read().await.session_key.clone();

    if let Some(ref key) = resume_key {
        match core
            .connection()
            .request::<SessionsResolveResponse>(
                "sessions.resolve",
                serde_json::to_value(SessionsResolveParams { key: key.clone() }).unwrap_or_default(),
            )
            .await
        {
            Ok(resp) => {
                if resp.ok == Some(false) {
                    warn!(
                        conversation_id = %core.runtime().conversation_id(),
                        "{} sessions.resolve reported a missing session, falling back to sessions.reset",
                        core.label()
                    );
                } else if let Some(resolved_key) = resp.key {
                    core.state().write().await.session_key = Some(resolved_key.clone());
                    info!(
                        conversation_id = %core.runtime().conversation_id(),
                        session_key = %resolved_key,
                        "Resumed {} session via sessions.resolve",
                        core.label()
                    );
                    return Ok(());
                } else {
                    warn!(
                        conversation_id = %core.runtime().conversation_id(),
                        "{} sessions.resolve returned no key, falling back to sessions.reset",
                        core.label()
                    );
                }
            }
            Err(e) => {
                warn!(
                    conversation_id = %core.runtime().conversation_id(),
                    error = %ErrorChain(&e),
                    "Failed to resume {} session, falling back to sessions.reset",
                    core.label()
                );
            }
        }
    }

    let resp: SessionsResetResponse = core
        .connection()
        .request(
            "sessions.reset",
            serde_json::to_value(SessionsResetParams {
                key: core.runtime().conversation_id().to_owned(),
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
        .ok_or_else(|| {
            AppError::Internal(format!("{} sessions.reset returned no session key", core.label()))
        })?;
    core.state().write().await.session_key = Some(key);

    Ok(())
}

/// Send one prompt through chat.send and bind the returned runId to the
/// admitted turn (draining any events buffered before the acknowledgement).
pub(crate) async fn send_chat_message<C: GatewayCore>(
    core: &C,
    is_first: bool,
    runtime_turn: AgentRuntimeTurn,
    mut data: SendMessageData,
) -> Result<(), AppError> {
    if is_first {
        resolve_session(core).await?;
    }
    data.content = inject_runtime_preset_context(data.content, core.preset_context(), is_first);

    let session_key = core
        .state()
        .read()
        .await
        .session_key
        .clone()
        .ok_or_else(|| AppError::Internal(format!("{} did not return a session key", core.label())))?;

    let params = ChatSendParams {
        session_key,
        message: data.content,
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        attachments: if data.files.is_empty() {
            None
        } else {
            Some(data.files.into_iter().map(|file| json!(file)).collect())
        },
    };

    let response = core
        .connection()
        .request::<Value>("chat.send", serde_json::to_value(params).unwrap_or_default())
        .await?;
    let active_run_id = response
        .get("runId")
        .or_else(|| response.get("run_id"))
        .and_then(Value::as_str)
        .filter(|run_id| !run_id.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::BadGateway(format!("{} chat.send returned no runId", core.label())))?;
    bind_run_to_active_turn(core, runtime_turn, active_run_id).await;

    Ok(())
}
