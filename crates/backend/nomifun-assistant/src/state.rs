//! Router state carrying the assistant service for axum handlers.

use std::sync::Arc;

use crate::service::AssistantService;

/// Shared state injected into `/api/assistants/*` handlers.
#[derive(Clone)]
pub struct AssistantRouterState {
    pub service: Arc<AssistantService>,
}
