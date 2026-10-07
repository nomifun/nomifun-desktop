//! Router state for the companion domain. Holds the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::CompanionService;

#[derive(Clone)]
pub struct CompanionRouterState {
    pub service: Arc<CompanionService>,
    pub knowledge_service: Option<Arc<nomifun_knowledge::KnowledgeService>>,
}

impl CompanionRouterState {
    pub fn new(service: Arc<CompanionService>) -> Self {
        Self { service, knowledge_service: None }
    }

    pub fn with_knowledge_service(mut self, service: Arc<nomifun_knowledge::KnowledgeService>) -> Self {
        self.knowledge_service = Some(service);
        self
    }
}
