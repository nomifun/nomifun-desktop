//! Router state for the public-agent domain: the `Arc`-wrapped service.

use std::sync::Arc;

use crate::service::PublicAgentService;

#[derive(Clone)]
pub struct PublicAgentRouterState {
    pub service: Arc<PublicAgentService>,
    pub preset_service: Option<Arc<nomifun_preset::PresetService>>,
}

impl PublicAgentRouterState {
    pub fn new(service: Arc<PublicAgentService>) -> Self {
        Self { service, preset_service: None }
    }

    pub fn with_preset_service(mut self, service: Arc<nomifun_preset::PresetService>) -> Self {
        self.preset_service = Some(service);
        self
    }
}
