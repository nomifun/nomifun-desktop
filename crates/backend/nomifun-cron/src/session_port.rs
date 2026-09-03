//! Typed Session boundary used by Cron execution.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ResolvedPresetSnapshot, SendMessageRequest,
};
use nomifun_common::AppError;
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition, BackgroundTurnRuntimePreparation,
    ObservedIdempotentMessageDelivery, PublicTurnDeliveryState,
};
use nomifun_conversation::{ConversationService, IdempotentMessageDelivery};

/// One Cron turn handed to the canonical Session owner.
///
/// Cron supplies only the user-visible message and the server-owned per-run
/// overlay (for example the Cron id and selected skills). The Session port
/// resolves the authoritative agent type, model, delegation policy, workspace
/// identity and conversation creation timestamp from its own projection before
/// it constructs any runtime preparation.
#[derive(Debug)]
pub struct CronTurnRequest {
    pub message: SendMessageRequest,
    pub runtime_extra: serde_json::Value,
    pub clear_context: bool,
}

/// Exact Session operations needed by the Cron domain.
#[async_trait]
pub trait CronSessionPort: Send + Sync {
    async fn list_by_cron_job(
        &self,
        user_id: &str,
        cron_job_id: &str,
    ) -> Result<Vec<ConversationResponse>, AppError>;

    async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<PublicTurnDeliveryState, AppError>;

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<BackgroundTurnReconciliationDisposition, AppError>;

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn send_observed_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        turn: CronTurnRequest,
    ) -> Result<ObservedIdempotentMessageDelivery, AppError>;

    async fn delivery_result(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: &SendMessageRequest,
    ) -> Result<Option<IdempotentMessageDelivery>, AppError>;
}

struct ConversationCronSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

#[async_trait]
impl CronSessionPort for ConversationCronSessionPort {
    async fn list_by_cron_job(
        &self,
        user_id: &str,
        cron_job_id: &str,
    ) -> Result<Vec<ConversationResponse>, AppError> {
        self.service.list_by_cron_job(user_id, cron_job_id).await
    }

    async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<PublicTurnDeliveryState, AppError> {
        self.service
            .public_turn_delivery_state(user_id, session_id, idempotency_key)
            .await
    }

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<BackgroundTurnReconciliationDisposition, AppError> {
        self.service
            .reconcile_quiescent_running_turn_for_background(
                user_id,
                session_id,
                idempotency_key,
                &self.runtime_registry,
            )
            .await
    }

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        match snapshot {
            Some(snapshot) => {
                self.service
                    .create_from_preset_snapshot_idempotent(
                        user_id,
                        request,
                        snapshot,
                        creation_key,
                    )
                    .await
            }
            None => {
                self.service
                    .create_idempotent(user_id, request, creation_key)
                    .await
            }
        }
    }

    async fn send_observed_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        turn: CronTurnRequest,
    ) -> Result<ObservedIdempotentMessageDelivery, AppError> {
        let build_lease = self
            .service
            .begin_public_runtime_preparation(session_id, user_id)?;
        let session = self.service.get(user_id, session_id).await?;
        build_lease.ensure_active()?;
        let runtime_options =
            runtime_options_from_session(user_id, session, turn.runtime_extra)?;
        self.service
            .send_observed_background_message_with_idempotency_key(
                user_id,
                session_id,
                idempotency_key,
                turn.message,
                &self.runtime_registry,
                build_lease,
                BackgroundTurnRuntimePreparation {
                    runtime_options,
                    clear_context: turn.clear_context,
                    pre_send_hook: None,
                },
            )
            .await
    }

    async fn delivery_result(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: &SendMessageRequest,
    ) -> Result<Option<IdempotentMessageDelivery>, AppError> {
        self.service
            .idempotent_delivery_result_with_idempotency_key(
                user_id,
                session_id,
                idempotency_key,
                request,
            )
            .await
    }
}

/// Translate the canonical Session projection into the legacy runtime
/// preparation shape at the one remaining host adapter boundary.
///
/// The caller may add per-run metadata, but cannot override Session-owned
/// identity or policy fields by putting similarly named values in `extra`.
fn runtime_options_from_session(
    user_id: &str,
    session: ConversationResponse,
    runtime_extra: serde_json::Value,
) -> Result<AgentRuntimeBuildOptions, AppError> {
    let ConversationResponse {
        conversation_id,
        r#type: agent_type,
        model,
        delegation_policy,
        created_at,
        extra: mut session_extra,
        ..
    } = session;

    let session_extra = session_extra.as_object_mut().ok_or_else(|| {
        AppError::Internal(format!(
            "conversation {conversation_id} extra must be a JSON object"
        ))
    })?;
    let runtime_extra = runtime_extra.as_object().ok_or_else(|| {
        AppError::BadRequest("Cron runtime extra must be a JSON object".to_owned())
    })?;
    for (key, value) in runtime_extra {
        session_extra.insert(key.clone(), value.clone());
    }

    let workspace = session_extra
        .get("workspace")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "conversation {conversation_id} has no canonical workspace"
            ))
        })?
        .to_owned();

    Ok(AgentRuntimeBuildOptions {
        user_id: user_id.to_owned(),
        agent_type,
        workspace,
        model,
        conversation_id,
        delegation_policy,
        extra: session_extra.clone().into(),
        conversation_created_at: Some(created_at),
        workspace_binding_lease: None,
    })
}

/// Build the transitional Conversation-backed Cron Session port.
pub fn conversation_cron_session_port(
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> Arc<dyn CronSessionPort> {
    Arc::new(ConversationCronSessionPort {
        service,
        runtime_registry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{
        AgentType, ConversationStatus, DecisionPolicy, DelegationPolicy, ProviderWithModel,
    };

    const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    fn session(extra: serde_json::Value) -> ConversationResponse {
        ConversationResponse {
            conversation_id: CONVERSATION_ID.to_owned(),
            name: "scheduled".to_owned(),
            r#type: AgentType::Nomi,
            model: Some(ProviderWithModel {
                provider_id: PROVIDER_ID.to_owned(),
                model: "gpt-5".to_owned(),
                use_model: None,
            }),
            status: ConversationStatus::Finished,
            runtime: None,
            source: None,
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            delegation_policy: DelegationPolicy::PreferParallel,
            execution_model_pool: None,
            decision_policy: DecisionPolicy::Automatic,
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: 42,
            modified_at: 43,
            extra,
        }
    }

    #[test]
    fn runtime_options_take_authority_from_session_projection() {
        let options = runtime_options_from_session(
            USER_ID,
            session(serde_json::json!({
                "workspace": "  C:/session-workspace  ",
                "delegation_policy": "disabled",
                "model": "forged",
            })),
            serde_json::json!({
                "cron_job_id": "0190f5fe-7c00-7a00-8abc-012345678902",
                "workspace": "C:/cron-workspace",
                "agent_type": "forged",
            }),
        )
        .expect("valid Session projection");

        assert_eq!(options.user_id, USER_ID);
        assert_eq!(options.conversation_id, CONVERSATION_ID);
        assert_eq!(options.agent_type, AgentType::Nomi);
        assert_eq!(
            options.model.as_ref().map(|model| model.provider_id.as_str()),
            Some(PROVIDER_ID)
        );
        assert_eq!(options.delegation_policy, DelegationPolicy::PreferParallel);
        assert_eq!(options.conversation_created_at, Some(42));
        assert_eq!(options.workspace, "C:/cron-workspace");
        assert_eq!(
            options.extra["cron_job_id"],
            "0190f5fe-7c00-7a00-8abc-012345678902"
        );
        assert_eq!(options.extra["delegation_policy"], "disabled");
    }

    #[test]
    fn runtime_options_reject_missing_session_workspace() {
        let error = runtime_options_from_session(USER_ID, session(serde_json::json!({})), serde_json::json!({}))
            .expect_err("a runtime must have a canonical workspace");
        assert!(error.to_string().contains("no canonical workspace"));
    }
}
