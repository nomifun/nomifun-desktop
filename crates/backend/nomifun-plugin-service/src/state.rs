use std::sync::Arc;

use async_trait::async_trait;

use crate::error::PluginServiceError;
use crate::service::PluginApplicationService;

#[async_trait]
pub trait PluginAuthoringCompletionPort: Send + Sync {
    async fn complete(
        &self,
        provider_id: &str,
        model: &str,
        system: String,
        prompt: String,
        max_tokens: u32,
    ) -> Result<String, PluginServiceError>;
}

#[derive(Clone)]
pub struct PluginRouterState {
    pub service: Arc<PluginApplicationService>,
    authoring_completion: Option<Arc<dyn PluginAuthoringCompletionPort>>,
}

impl PluginRouterState {
    pub fn new(service: Arc<PluginApplicationService>) -> Self {
        Self {
            service,
            authoring_completion: None,
        }
    }

    pub fn with_authoring_completion(
        mut self,
        completion: Arc<dyn PluginAuthoringCompletionPort>,
    ) -> Self {
        self.authoring_completion = Some(completion);
        self
    }

    pub fn authoring_completion(&self) -> Option<&Arc<dyn PluginAuthoringCompletionPort>> {
        self.authoring_completion.as_ref()
    }
}
