use std::sync::Arc;

use crate::orchestrator::Orchestrator;
use crate::service::RequirementService;

#[derive(Clone)]
pub struct RequirementRouterState {
    pub requirement_service: Arc<RequirementService>,
    pub orchestrator: Arc<Orchestrator>,
}
