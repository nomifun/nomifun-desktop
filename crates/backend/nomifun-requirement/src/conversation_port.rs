//! Narrow Conversation-backed Session boundary used by Requirements/AutoWork.
//!
//! The Requirement domain owns claims, retries, verdicts, and durable
//! Requirement facts. The current Conversation service owns turn admission,
//! runtime preparation, delivery receipts, and cancellation. This module keeps
//! that ownership explicit without exposing the full service/runtime registry
//! to the Requirement service or runner.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::runtime_registry::AgentRuntimeRegistry;
use nomifun_api_types::SendMessageRequest;
use nomifun_common::AppError;
use nomifun_conversation::runtime_state::RuntimeBuildLease;
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition, BackgroundTurnRuntimePreparation,
    ObservedIdempotentMessageDelivery,
};
use nomifun_conversation::{ConversationService, IdempotentMessageDelivery};
use nomifun_db::RequirementConversationTurnAuthority;

/// Exact command/query surface AutoWork needs from the current Session owner.
#[async_trait]
pub trait AutoWorkConversationPort: Send + Sync {
    fn begin_runtime_preparation(
        &self,
        conversation_id: &str,
        requester_user_id: &str,
    ) -> Result<RuntimeBuildLease, AppError>;

    fn user_cancelled_since(&self, conversation_id: &str, since_ms: i64) -> bool;

    async fn cancel_active_turn(&self, conversation_id: &str) -> Result<(), AppError>;

    async fn save_config(
        &self,
        conversation_id: &str,
        enabled: bool,
        tag: Option<&str>,
        max_requirements: Option<u32>,
    ) -> Result<(), AppError>;

    #[allow(clippy::too_many_arguments)]
    async fn send_observed_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: SendMessageRequest,
        build_lease: RuntimeBuildLease,
        runtime_preparation: BackgroundTurnRuntimePreparation,
        authority: RequirementConversationTurnAuthority,
    ) -> Result<ObservedIdempotentMessageDelivery, AppError>;

    async fn delivery_result(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: &SendMessageRequest,
        authority: &RequirementConversationTurnAuthority,
    ) -> Result<Option<IdempotentMessageDelivery>, AppError>;

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<BackgroundTurnReconciliationDisposition, AppError>;
}

/// Transitional adapter over the existing Conversation owner.
///
/// It is deliberately stateless: no cached runtime handle, retry policy,
/// fallback, or second Session identity is introduced.
struct ConversationAutoWorkPort {
    service: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

#[async_trait]
impl AutoWorkConversationPort for ConversationAutoWorkPort {
    fn begin_runtime_preparation(
        &self,
        conversation_id: &str,
        requester_user_id: &str,
    ) -> Result<RuntimeBuildLease, AppError> {
        self.service
            .begin_public_runtime_preparation(conversation_id, requester_user_id)
    }

    fn user_cancelled_since(&self, conversation_id: &str, since_ms: i64) -> bool {
        self.service
            .user_cancelled_since(conversation_id, since_ms)
    }

    async fn cancel_active_turn(&self, conversation_id: &str) -> Result<(), AppError> {
        if let Some(runtime) = self.runtime_registry.get_runtime(conversation_id) {
            runtime.cancel().await?;
        }
        Ok(())
    }

    async fn save_config(
        &self,
        conversation_id: &str,
        enabled: bool,
        tag: Option<&str>,
        max_requirements: Option<u32>,
    ) -> Result<(), AppError> {
        self.service
            .update_extra(
                conversation_id,
                serde_json::json!({
                    "autowork": {
                        "enabled": enabled,
                        "tag": tag,
                        "max_requirements": max_requirements,
                    }
                }),
            )
            .await
    }

    async fn send_observed_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: SendMessageRequest,
        build_lease: RuntimeBuildLease,
        runtime_preparation: BackgroundTurnRuntimePreparation,
        authority: RequirementConversationTurnAuthority,
    ) -> Result<ObservedIdempotentMessageDelivery, AppError> {
        self.service
            .send_observed_autowork_message_with_idempotency_key(
                user_id,
                conversation_id,
                operation_id,
                request,
                &self.runtime_registry,
                build_lease,
                runtime_preparation,
                authority,
            )
            .await
    }

    async fn delivery_result(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: &SendMessageRequest,
        authority: &RequirementConversationTurnAuthority,
    ) -> Result<Option<IdempotentMessageDelivery>, AppError> {
        self.service
            .autowork_delivery_result_with_idempotency_key(
                user_id,
                conversation_id,
                operation_id,
                request,
                authority,
            )
            .await
    }

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<BackgroundTurnReconciliationDisposition, AppError> {
        self.service
            .reconcile_quiescent_running_turn_for_background(
                user_id,
                conversation_id,
                operation_id,
                &self.runtime_registry,
            )
            .await
    }
}

/// Build the single transitional Conversation-backed AutoWork port.
pub fn conversation_autowork_port(
    service: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> Arc<dyn AutoWorkConversationPort> {
    Arc::new(ConversationAutoWorkPort {
        service,
        runtime_registry,
    })
}
