//! Router state for the companion domain. Holds the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::CompanionService;

#[derive(Clone)]
pub struct CompanionRouterState {
    pub service: Arc<CompanionService>,
}

impl CompanionRouterState {
    pub fn new(service: Arc<CompanionService>) -> Self {
        Self { service }
    }
}
