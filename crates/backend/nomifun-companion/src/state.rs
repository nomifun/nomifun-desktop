//! Router state for the companion domain. Holds the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::CompanionService;

#[derive(Clone)]
pub struct CompanionRouterState {
    pub service: Arc<CompanionService>,
    pub preset_service: Option<Arc<nomifun_preset::PresetService>>,
    pub knowledge_service: Option<Arc<nomifun_knowledge::KnowledgeService>>,
}

impl CompanionRouterState {
    pub fn new(service: Arc<CompanionService>) -> Self {
        Self { service, preset_service: None, knowledge_service: None }
    }

    pub fn with_preset_service(mut self, service: Arc<nomifun_preset::PresetService>) -> Self {
        self.preset_service = Some(service);
        self
    }

    pub fn with_knowledge_service(mut self, service: Arc<nomifun_knowledge::KnowledgeService>) -> Self {
        self.knowledge_service = Some(service);
        self
    }
}
