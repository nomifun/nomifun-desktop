//! Router state for the workshop domain: the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::WorkshopService;
use crate::workflow_draft::WorkflowDraftRunner;

#[derive(Clone)]
pub struct WorkshopRouterState {
    pub service: Arc<WorkshopService>,
    pub workflow_draft_runner: Arc<dyn WorkflowDraftRunner>,
}

impl WorkshopRouterState {
    pub fn new(
        service: Arc<WorkshopService>,
        workflow_draft_runner: Arc<dyn WorkflowDraftRunner>,
    ) -> Self {
        Self {
            service,
            workflow_draft_runner,
        }
    }
}
