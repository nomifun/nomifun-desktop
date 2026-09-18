use std::sync::Arc;

use crate::service::ConversationService;
use nomifun_ai_agent::AgentRuntimeSessions;

/// Shared state for conversation route handlers.
#[derive(Clone)]
pub struct ConversationRouterState {
    pub service: ConversationService,
    pub runtime_sessions: Arc<dyn AgentRuntimeSessions>,
}
