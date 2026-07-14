use std::sync::Arc;

use crate::service::ConversationService;
use nomifun_ai_agent::AgentRuntimeRegistry;

/// Shared state for conversation route handlers.
#[derive(Clone)]
pub struct ConversationRouterState {
    pub service: ConversationService,
    pub runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}
