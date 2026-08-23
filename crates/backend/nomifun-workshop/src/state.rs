//! Router state for the workshop domain: the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::WorkshopService;
use crate::template_draft::TemplateDraftRunner;

#[derive(Clone)]
pub struct WorkshopRouterState {
    pub service: Arc<WorkshopService>,
    pub template_draft_runner: Arc<dyn TemplateDraftRunner>,
}

impl WorkshopRouterState {
    pub fn new(
        service: Arc<WorkshopService>,
        template_draft_runner: Arc<dyn TemplateDraftRunner>,
    ) -> Self {
        Self {
            service,
            template_draft_runner,
        }
    }
}
