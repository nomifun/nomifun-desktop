use std::sync::Arc;

use crate::{AgentRegistry, AgentService};

#[derive(Clone)]
pub struct AgentRouterState {
    pub agent_registry: Arc<AgentRegistry>,
    pub service: Arc<AgentService>,
}
