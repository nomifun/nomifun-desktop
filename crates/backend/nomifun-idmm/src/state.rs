//! Router state for the IDMM domain. Holds the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::IdmmService;

#[derive(Clone)]
pub struct IdmmRouterState {
    pub service: Arc<IdmmService>,
}

impl IdmmRouterState {
    pub fn new(service: Arc<IdmmService>) -> Self {
        Self { service }
    }
}
