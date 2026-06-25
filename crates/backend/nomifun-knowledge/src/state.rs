//! Router state for the knowledge domain. Holds the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::KnowledgeService;

#[derive(Clone)]
pub struct KnowledgeRouterState {
    pub service: Arc<KnowledgeService>,
}

impl KnowledgeRouterState {
    pub fn new(service: Arc<KnowledgeService>) -> Self {
        Self { service }
    }
}
