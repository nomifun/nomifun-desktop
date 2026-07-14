use std::sync::Arc;

use crate::auto_work_runner::AutoWorkRunner;
use crate::service::RequirementService;

#[derive(Clone)]
pub struct RequirementRouterState {
    pub requirement_service: Arc<RequirementService>,
    pub auto_work_runner: Arc<AutoWorkRunner>,
}
