use std::sync::Arc;

use crate::{AgentRegistry, AgentService, RemoteAgentService};

/// Router state for remote agent routes.
#[derive(Clone)]
pub struct RemoteAgentRouterState {
    pub service: Arc<RemoteAgentService>,
}

#[derive(Clone)]
pub struct AgentRouterState {
    pub agent_registry: Arc<AgentRegistry>,
    pub service: Arc<AgentService>,
}
