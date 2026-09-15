//! A view adapter, not another Session manager. Keep the normal backend/Agent seam.
use std::sync::Weak;

use nomifun_agent_contracts::PluginAgentSessionRequest;
use nomifun_plugin_platform::runtime::{PluginAgentSessionPort, PluginRuntimeApplicationError};

use super::*;

pub(crate) struct NomiCorePluginUiSessions {
    owner: Weak<NomiCoreSessionOwner>,
}

impl NomiCorePluginUiSessions {
    pub(crate) fn new(owner: &Arc<NomiCoreSessionOwner>) -> Self {
        Self {
            owner: Arc::downgrade(owner),
        }
    }

    fn owner(&self) -> Result<Arc<NomiCoreSessionOwner>, PluginRuntimeApplicationError> {
        self.owner.upgrade().ok_or_else(|| {
            PluginRuntimeApplicationError::Runtime("Agent Session owner has stopped".into())
        })
    }

    async fn access(
        &self,
        user: &str,
        id: &str,
    ) -> Result<
        (
            Arc<NomiCoreSessionOwner>,
            AuthenticatedOwner,
            ConversationResponse,
            NomiCoreSessionMetadata,
        ),
        PluginRuntimeApplicationError,
    > {
        // Both Surface open and every Session bridge request pass this boundary.
        let id = parse_agent_session_id(id).map_err(ui_error)?;
        let owner = self.owner()?;
        let user = AuthenticatedOwner(UserId::from(user));
        let response = load_session_from_owner(&owner, &user, &id)
            .await
            .map_err(ui_error)?;
        let metadata = session_metadata(&response, &user).map_err(ui_error)?;
        Ok((owner, user, response, metadata))
    }
}

fn ui_error(error: NomiCoreApiError) -> PluginRuntimeApplicationError {
    PluginRuntimeApplicationError::AgentSession {
        status: error.status.as_u16(),
        code: error.code,
        message: error.message,
        details: error.details,
    }
}

#[async_trait]
impl PluginAgentSessionPort for NomiCorePluginUiSessions {
    async fn authorize(&self, user: &str, id: &str) -> Result<(), PluginRuntimeApplicationError> {
        self.access(user, id).await.map(|_| ())
    }

    async fn request(
        &self,
        user: &str,
        plugin_id: &str,
        id: &str,
        request: PluginAgentSessionRequest,
    ) -> Result<StrictJsonValue, PluginRuntimeApplicationError> {
        request
            .validate()
            .map_err(|error| PluginRuntimeApplicationError::Invalid(error.to_string()))?;
        let (owner, user, response, metadata) = self.access(user, id).await?;
        let value = match request {
            PluginAgentSessionRequest::Observe { after_seq, limit } => {
                let observation =
                    build_session_observation(&owner, &user, &response, metadata, after_seq, limit)
                        .await
                        .map_err(ui_error)?;
                serde_json::to_value(observation)
            }
            PluginAgentSessionRequest::Turn {
                input,
                idempotency_key,
            } => {
                // Separate plugin effects from built-in UI keys; keep identity across
                // view reloads and plugin upgrades. No automatic retry is performed.
                let result = start_owned_session_turn(
                    &owner,
                    &user,
                    id,
                    CreateAgentSessionTurnRequestDto {
                        input: input.0,
                        idempotency_key: format!("plugin-ui:{plugin_id}:{idempotency_key}"),
                    },
                )
                .await
                .map_err(ui_error)?;
                serde_json::to_value(result)
            }
            PluginAgentSessionRequest::Cancel {} => {
                owner
                    .cancel_session(user.as_ref(), id)
                    .await
                    .map_err(NomiCoreApiError::from)
                    .map_err(ui_error)?;
                Ok(json!({ "agent_session_id": id, "canceled": true }))
            }
        }
        .map_err(|error| PluginRuntimeApplicationError::Runtime(error.to_string()))?;
        Ok(StrictJsonValue(value))
    }
}
