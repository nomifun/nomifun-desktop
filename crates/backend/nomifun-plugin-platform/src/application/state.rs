use std::sync::Arc;

use async_trait::async_trait;

use crate::application::error::PluginServiceError;
use crate::application::service::PluginApplicationService;

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
    pub runtime: Option<Arc<crate::runtime::PluginRuntimeM1ApplicationService>>,
    authoring_completion: Option<Arc<dyn PluginAuthoringCompletionPort>>,
}

impl PluginRouterState {
    pub fn new(service: Arc<PluginApplicationService>) -> Self {
        Self {
            service,
            runtime: None,
            authoring_completion: None,
        }
    }

    pub fn with_runtime(mut self, runtime: Arc<crate::runtime::PluginRuntimeM1ApplicationService>) -> Self {
        self.runtime = Some(runtime);
        self
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
