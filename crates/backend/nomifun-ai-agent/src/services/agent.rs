//! Business-logic layer for the ai-agent crate.
//!
//! Per `AGENTS.md` "Domain Crate Structure", this is the sole location
//! for agent-related business logic. HTTP handlers in `routes/` should
//! only extract inputs, call methods on this service, and wrap the
//! result in `ApiResponse`.
//!
//! Session-scoped operations (mode/model/config/usage/capabilities/
//! slash-commands/side-question/workspace) now live in
//! `nomifun-conversation::ConversationService`, which dispatches through
//! `AgentRuntimeHandle`. This service retains agent-catalog listing and
//! model-provider health checks.

use std::path::PathBuf;
use std::sync::Arc;

use nomifun_api_types::{
    AgentMetadata, ProviderHealthCheckRequest, ProviderHealthCheckResponse,
};
use nomifun_common::AppError;
use nomifun_model_invoke::ModelInvokeService;

use super::provider_health::ProviderHealthCheckService;
use crate::registry::AgentRegistry;

pub struct AgentService {
    registry: Arc<AgentRegistry>,
    provider_health: ProviderHealthCheckService,
}

impl AgentService {
    pub fn new(
        registry: Arc<AgentRegistry>,
        data_dir: PathBuf,
        model_invoke_service: Arc<ModelInvokeService>,
    ) -> Arc<Self> {
        let provider_health = ProviderHealthCheckService::new(
            data_dir.clone(),
            model_invoke_service,
        );
        Arc::new(Self {
            registry,
            provider_health,
        })
    }
}

// Agent operations
impl AgentService {
    pub async fn list_agents(&self) -> Result<Vec<AgentMetadata>, AppError> {
        Ok(self.registry.list_all().await)
    }

    pub async fn refresh_agents(&self) -> Result<Vec<AgentMetadata>, AppError> {
        self.registry.refresh_availability().await;
        Ok(self.registry.list_all().await)
    }

    pub async fn provider_health_check(
        &self,
        req: ProviderHealthCheckRequest,
    ) -> Result<ProviderHealthCheckResponse, AppError> {
        self.provider_health.health_check(req).await
    }
}
