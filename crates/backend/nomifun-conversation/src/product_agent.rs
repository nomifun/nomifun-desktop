//! Product-owned target resolution into one immutable Agent binding.

use async_trait::async_trait;
use nomifun_api_types::AgentResolvedSnapshot;
use nomifun_common::{AppError, ProviderWithModel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAgentTarget {
    pub target_kind: String,
    pub target_id: String,
    pub default_template_key: String,
}

#[derive(Debug, Clone)]
pub struct ProductAgentResolution {
    pub snapshot: AgentResolvedSnapshot,
    pub runtime_extra: serde_json::Value,
}

#[async_trait]
pub trait ProductAgentSnapshotResolver: Send + Sync {
    async fn resolve(
        &self,
        owner_id: &str,
        target: &ProductAgentTarget,
        requested_model: Option<&ProviderWithModel>,
    ) -> Result<ProductAgentResolution, AppError>;
}
