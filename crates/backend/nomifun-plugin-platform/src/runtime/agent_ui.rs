//! The application owns Sessions; the plugin host owns only a scoped UI grant.
use async_trait::async_trait;
use nomifun_agent_contracts::{PluginAgentSessionRequest, StrictJsonValue};

use super::PluginRuntimeApplicationError;

#[async_trait]
pub trait PluginAgentSessionPort: Send + Sync {
    /// Called before issuing a Surface capability. Must not create/start a Session.
    async fn authorize(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<(), PluginRuntimeApplicationError>;

    /// Identity is supplied by the verified Surface row, never by the iframe.
    /// Recheck current Session ownership/availability at the application boundary.
    async fn request(
        &self,
        owner_user_id: &str,
        plugin_id: &str,
        agent_session_id: &str,
        request: PluginAgentSessionRequest,
    ) -> Result<StrictJsonValue, PluginRuntimeApplicationError>;
}
