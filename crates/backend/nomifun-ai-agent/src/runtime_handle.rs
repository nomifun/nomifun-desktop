//! Public control contract and handle for a live Agent runtime.
//!
//! `AgentRuntimeControl` captures **only** the operations that every agent type
//! implements identically and that the generic runtime registry, idle scanner,
//! message-flow code actually needs. Anything that is type-specific
//! (session modes, session keys, model switching, config options, pending
//! confirmation lists, approval memory, etc.) lives as **inherent** methods on
//! each concrete `XxxAgentManager`
//! and is reached through the `AgentRuntimeHandle` enum — forcing every callsite
//! to say out loud which agent type it is addressing.
//!
//! This replaces the former downcast-based manager abstraction with an explicit,
//! closed set of runtime variants.
use std::sync::Arc;

use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use tokio::sync::broadcast;

use crate::manager::nomi::NomiAgentManager;
use crate::protocol::events::AgentStreamEvent;
use crate::protocol::send_error::AgentSendError;
use crate::types::SendMessageData;

use nomifun_api_types::{
    GetModelInfoResponse, SideQuestionRequest, SideQuestionResponse, SlashCommandItem,
};

/// Where a trusted host resource notification was queued.
///
/// Both dispositions are non-turn-creating. `ActiveTurn` means a Nomi turn was
/// running when the notice entered its dedicated system-resource inbox;
/// `NextModelCall` means the runtime was idle and will expose the notice in the
/// top-level system context of its next real model request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemResourceNoticeDelivery {
    ActiveTurn,
    NextModelCall,
}

#[cfg(any(test, feature = "test-support"))]
use nomifun_common::Confirmation;

/// Minimal public surface every agent type implements identically.
///
/// Object-safe by construction (no generic methods, no `Self` by value).
/// Used by generic lifecycle code (runtime registry, idle scanner, stream
/// fan-out) that genuinely does not care which agent type it is dealing
/// with. For type-specific operations, match on [`AgentRuntimeHandle`] and
/// call the concrete manager's inherent methods.
#[async_trait::async_trait]
pub trait AgentRuntimeControl: Send + Sync {
    /// The type of Agent this runtime controls.
    fn agent_type(&self) -> AgentType;

    /// Conversation ID this runtime is bound to.
    fn conversation_id(&self) -> &str;

    /// Working directory for this agent session.
    fn workspace(&self) -> &str;

    /// Current conversation status. `None` if the agent has not
    /// transitioned into a known status yet.
    fn status(&self) -> Option<ConversationStatus>;

    /// Whether the runtime still has a functioning event transport. Required
    /// for every manager so a newly added/reworked backend cannot silently
    /// inherit "healthy" after its process or relay has been quarantined.
    fn is_transport_healthy(&self) -> bool;

    /// Timestamp (ms) of the last activity (message send, event received).
    fn last_activity_at(&self) -> TimestampMs;

    /// Mark lifecycle admission for a new turn as activity.
    ///
    /// Production runtimes override this so idle cleanup cannot observe an old
    /// `Finished` timestamp in the gap between registry admission and
    /// `send_message` resetting the runtime to `Running`. The no-op default
    /// preserves compatibility for lightweight external/test implementations.
    fn touch_activity(&self) {}

    /// Subscribe to the agent's stream event channel.
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent>;

    /// Send a user message to the agent. Returns once the agent has
    /// accepted the turn; actual streaming proceeds on the broadcast
    /// channel returned by [`Self::subscribe`].
    async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError>;

    /// Stop the current streaming response without killing the agent.
    async fn cancel(&self) -> Result<(), AppError>;

    /// Terminate the agent process.
    ///
    /// - `reason: Some(IdleTimeout)` — idle cleanup
    /// - `reason: None` — explicit user/system kill
    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError>;
}

/// Extended trait used exclusively by the `AgentRuntimeHandle::Mock` variant so
/// tests can inject richer fake behaviour (pending confirmations, approval
/// memory, fake session keys, etc.) without polluting the production
/// `AgentRuntimeControl` contract with trait-level defaults that would be lies for
/// at least one concrete manager.
///
/// Every method has a sensible identity-style default so simple mocks only
/// need to implement the `AgentRuntimeControl` methods and pick up nothing for
/// free.
#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
pub trait MockAgentRuntime: AgentRuntimeControl {
    fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        Box::pin(std::future::ready(self.kill(reason)))
    }

    fn get_confirmations(&self) -> Vec<Confirmation> {
        Vec::new()
    }
    fn requires_turn_boundary_recycle(&self) -> bool {
        false
    }
    fn check_approval(&self, _action: &str, _command_type: Option<&str>) -> bool {
        false
    }
    /// Mid-turn steering. Mirrors `AgentRuntimeHandle::steer`: `Ok(true)` = queued
    /// into a live turn, `Ok(false)` = no live turn (caller sends normally).
    /// Defaults to `Ok(false)` so simple mocks report "not steerable"; tests
    /// that exercise the steering path override this.
    fn steer(&self, _text: String) -> Result<bool, AppError> {
        Ok(false)
    }
    fn notify_system_resource(
        &self,
        _notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        Err(AppError::BadRequest(
            "System resource notifications are not supported for this mock".into(),
        ))
    }
    fn confirm(
        &self,
        _msg_id: &str,
        _call_id: &str,
        _data: serde_json::Value,
        _always_allow: bool,
    ) -> Result<(), AppError> {
        Ok(())
    }
    async fn mode(&self) -> Result<nomifun_api_types::AgentModeResponse, AppError> {
        Ok(nomifun_api_types::AgentModeResponse {
            mode: "default".into(),
            initialized: false,
        })
    }
    async fn set_mode(&self, _mode: &str) -> Result<(), AppError> {
        Err(AppError::BadRequest(
            "Mode switching is not supported for this mock".into(),
        ))
    }
    async fn get_model(&self) -> Result<GetModelInfoResponse, AppError> {
        Ok(GetModelInfoResponse { model_info: None })
    }
    async fn set_model(&self, _model_id: &str) -> Result<(), AppError> {
        Err(AppError::BadRequest(
            "Model switching is not supported for this mock".into(),
        ))
    }
    async fn get_slash_commands(&self) -> Result<Vec<SlashCommandItem>, AppError> {
        Ok(Vec::new())
    }
    async fn handle_side_question(&self, _req: SideQuestionRequest) -> Result<SideQuestionResponse, AppError> {
        Ok(SideQuestionResponse {
            status: "unsupported".into(),
            answer: None,
        })
    }
}

/// Concrete, closed-set dispatcher for the five agent variants.
///
/// Every generic path holds an `AgentRuntimeHandle` (not `Arc<dyn AgentRuntimeControl>`):
/// this gives us the common `AgentRuntimeControl` surface via [`Self::as_runtime`]
/// **and** lets type-specific routes recover the concrete manager with a
/// single `match` — no `as_any` / `downcast_ref` anywhere. Adding a new
/// agent type means adding a new variant here; every `match` in the
/// codebase then fails to compile until it explicitly handles the new
/// type, which is the compile-time pressure we want.
#[derive(Clone)]
pub enum AgentRuntimeHandle {
    Nomi(Arc<NomiAgentManager>),
    /// Test-only trait-object escape hatch used by downstream crates
    /// (conversation/cron/requirement/app tests) to inject fake agents without
    /// spinning up a real CLI or WebSocket connection. Gated behind
    /// `#[cfg(any(test, feature = "test-support"))]`: production builds
    /// never see this variant, so every `match` in release code can
    /// rely on the closed set of real runtime variants. The trait object is
    /// [`MockAgentRuntime`] (extends `AgentRuntimeControl`) so mocks can also override
    /// the enum-level helpers — `get_confirmations`, `check_approval`,
    /// `confirm`, `get_session_key`, `get_mode`, `set_mode`.
    #[cfg(any(test, feature = "test-support"))]
    Mock(Arc<dyn MockAgentRuntime>),
}

impl AgentRuntimeHandle {
    /// Common `AgentRuntimeControl` view, regardless of variant.
    pub fn as_runtime(&self) -> &dyn AgentRuntimeControl {
        match self {
            Self::Nomi(m) => m.as_ref(),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.as_ref(),
        }
    }

    // ── Convenience forwarders ───────────────────────────────────────
    //
    // These stay in the final API (not a migration crutch): they turn
    // `runtime.agent_type()` into a direct vtable-free call on the
    // concrete `Arc<XxxManager>`, and they keep callsites terse.

    /// The type of Agent this runtime controls.
    pub fn agent_type(&self) -> AgentType {
        self.as_runtime().agent_type()
    }

    /// Conversation ID this runtime is bound to.
    pub fn conversation_id(&self) -> &str {
        self.as_runtime().conversation_id()
    }

    /// Working directory for this agent session.
    pub fn workspace(&self) -> &str {
        self.as_runtime().workspace()
    }

    /// Current conversation status.
    pub fn status(&self) -> Option<ConversationStatus> {
        self.as_runtime().status()
    }

    pub fn is_transport_healthy(&self) -> bool {
        self.as_runtime().is_transport_healthy()
    }

    /// Whether this exact process must be retired before another explicit turn
    /// can be admitted. A protocol without per-frame turn identity has its
    /// emission authority permanently closed by a terminal boundary, even
    /// though the transport itself is still healthy.
    pub fn requires_turn_boundary_recycle(&self) -> bool {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.requires_turn_boundary_recycle(),
            Self::Nomi(_) => false,
        }
    }

    /// Timestamp (ms) of the last activity.
    pub fn last_activity_at(&self) -> TimestampMs {
        self.as_runtime().last_activity_at()
    }

    /// Mark turn admission as runtime activity.
    pub fn touch_activity(&self) {
        self.as_runtime().touch_activity();
    }

    /// Subscribe to the stream event channel.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.as_runtime().subscribe()
    }

    /// Send a user message to the agent.
    pub async fn send_message(&self, data: SendMessageData) -> Result<(), AgentSendError> {
        self.as_runtime().send_message(data).await
    }

    /// Cancel the current streaming response without killing the agent.
    pub async fn cancel(&self) -> Result<(), AppError> {
        self.as_runtime().cancel().await
    }

    /// Terminate the agent process.
    pub fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.as_runtime().kill(reason)
    }

    /// Terminate the agent process and return a future that resolves when the
    /// underlying OS process has exited.
    pub fn kill_and_wait(
        &self,
        reason: Option<AgentKillReason>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send>> {
        match self {
            Self::Nomi(m) => m.kill_and_wait(reason),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.kill_and_wait(reason),
        }
    }

    // ── Cross-variant semi-specific helpers ──────────────────────────
    //
    // These fan out to inherent methods on concrete managers. Variants
    // that don't support the operation return a sensible zero-value
    // rather than an error: "no pending confirmations" and "no session
    // key" are honest statements about those variants.

    /// Pending confirmation items for this runtime.
    ///
    /// Nomi maintains an inline confirmation list.
    pub fn get_confirmations(&self) -> Vec<nomifun_common::Confirmation> {
        match self {
            Self::Nomi(m) => m.get_confirmations(),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.get_confirmations(),
        }
    }

    /// Submit a confirmation response for a pending tool call.
    pub fn confirm(
        &self,
        msg_id: &str,
        call_id: &str,
        data: serde_json::Value,
        always_allow: bool,
    ) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => m.confirm(msg_id, call_id, data, always_allow),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.confirm(msg_id, call_id, data, always_allow),
        }
    }

    /// Check whether an action is auto-approved in this session.
    pub fn check_approval(&self, action: &str, command_type: Option<&str>) -> bool {
        match self {
            Self::Nomi(m) => m.check_approval(action, command_type),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.check_approval(action, command_type),
        }
    }

    /// Get the current session mode.
    pub async fn get_mode(&self) -> Result<nomifun_api_types::AgentModeResponse, AppError> {
        match self {
            Self::Nomi(m) => m.mode().await,
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.mode().await,
        }
    }

    /// Set the session mode.
    pub async fn set_mode(&self, mode: &str) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => m.set_mode(mode).await,
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.set_mode(mode).await,
        }
    }

    /// Clear the conversation context ("release model context") in place,
    /// keeping the agent/process alive. Nomi empties its engine history.
    pub async fn clear_context(&self) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => m.clear_context().await,
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(_) => Ok(()),
        }
    }

    /// Push a mid-turn steering interjection into the running turn. The Nomi
    /// native engine injects mid-turn. `Ok(true)` = queued into a live turn;
    /// `Ok(false)` = no turn running (caller should send normally).
    pub fn steer(&self, text: String) -> Result<bool, AppError> {
        match self {
            Self::Nomi(m) => m.steer(text),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.steer(text),
        }
    }

    /// Queue trusted host resource state without creating a turn or pretending
    /// the event came from the user.
    ///
    /// Nomi owns a dedicated inbox whose entries are injected into the
    /// provider's top-level system context at the next model boundary.
    pub fn notify_system_resource(
        &self,
        notice: String,
    ) -> Result<SystemResourceNoticeDelivery, AppError> {
        if notice.trim().is_empty() {
            return Err(AppError::BadRequest(
                "System resource notice must not be empty".into(),
            ));
        }
        match self {
            Self::Nomi(m) => m.notify_system_resource(notice),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.notify_system_resource(notice),
        }
    }

    /// Validate an exact rewind checkpoint without mutating the runtime.
    pub async fn ensure_can_rewind_last_turn(
        &self,
        expected_source_message_id: &str,
    ) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => {
                m.ensure_can_rewind_last_turn(expected_source_message_id)
                    .await
            }
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(_) => Ok(()),
        }
    }

    /// Rewind the last user turn (edit & resubmit the most recent user message).
    /// The Nomi native engine rewinds its in-memory transcript.
    pub async fn rewind_last_turn(
        &self,
        expected_source_message_id: &str,
    ) -> Result<(), AppError> {
        match self {
            Self::Nomi(m) => m.rewind_last_turn(expected_source_message_id).await,
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(_) => Ok(()),
        }
    }

    /// Get the current session model info. The per-session model catalog was a
    /// property of the external CLI protocol; a Nomi session's model is fixed by
    /// the conversation's provider binding, so it reports `model_info = None` and
    /// the UI hides the in-session model picker without an error.
    pub async fn get_model(&self) -> Result<GetModelInfoResponse, AppError> {
        match self {
            Self::Nomi(_) => Ok(GetModelInfoResponse { model_info: None }),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.get_model().await,
        }
    }

    /// Switch the active model. Unsupported: a Nomi session's model comes from
    /// the conversation's provider binding, so this returns a `BadRequest` the
    /// caller can surface rather than silently no-op.
    pub async fn set_model(&self, model_id: &str) -> Result<(), AppError> {
        if model_id.trim().is_empty() {
            return Err(AppError::BadRequest("model_id must not be empty".into()));
        }
        match self {
            Self::Nomi(_) => Err(AppError::BadRequest(
                "Model switching is not supported for this agent type".into(),
            )),
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.set_model(model_id).await,
        }
    }

    /// Slash commands available in the current session.
    pub async fn get_slash_commands(&self) -> Result<Vec<SlashCommandItem>, AppError> {
        match self {
            Self::Nomi(m) => m.get_slash_commands().await,
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.get_slash_commands().await,
        }
    }

    /// Dispatch a side-question to the agent.
    ///
    /// No backend implements side-questions yet, so every variant honestly
    /// reports `unsupported` (the UI surfaces this as a warning toast). An
    /// earlier engine returned a hardcoded fake-success answer here, presenting
    /// a placeholder string to the user as a real reply; that path was removed
    /// rather than shipped.
    pub async fn handle_side_question(&self, req: SideQuestionRequest) -> Result<SideQuestionResponse, AppError> {
        if req.question.trim().is_empty() {
            return Err(AppError::BadRequest("question must not be empty".into()));
        }
        match self {
            Self::Nomi(_) => {
                Ok(SideQuestionResponse {
                    status: "unsupported".into(),
                    answer: None,
                })
            }
            #[cfg(any(test, feature = "test-support"))]
            Self::Mock(m) => m.handle_side_question(req).await,
        }
    }
}
