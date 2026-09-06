use std::sync::Arc;

use crate::service::PluginApplicationService;

#[derive(Clone)]
pub struct PluginRouterState {
    pub service: Arc<PluginApplicationService>,
}

impl PluginRouterState {
    pub fn new(service: Arc<PluginApplicationService>) -> Self {
        Self { service }
    }
}
