use nomifun_agent_contracts::{AgentPresetId, UserId};
use nomifun_api_types::{AgentUiBindingDto, AgentUiContributionDto};

use crate::ControlPlaneError;

/// Preset presentation storage has a lifecycle independent of execution
/// revisions. Persist choices even when withdrawn; reads never choose a fallback.
#[async_trait::async_trait]
pub trait AgentUiBindingStore: Send + Sync {
    async fn load(
        &self,
        owner: &UserId,
        preset: &AgentPresetId,
    ) -> Result<AgentUiBindingDto, ControlPlaneError>;
    async fn put(
        &self,
        owner: &UserId,
        preset: &AgentPresetId,
        selection: Option<AgentUiContributionDto>,
        expected_version: u64,
    ) -> Result<AgentUiBindingDto, ControlPlaneError>;
}
