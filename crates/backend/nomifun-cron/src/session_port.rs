//! Typed Session boundary used by Cron execution.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ResolvedPresetSnapshot, SendMessageRequest,
};
use nomifun_common::AppError;
use nomifun_conversation::runtime_state::RuntimeBuildLease;
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition, BackgroundTurnRuntimePreparation,
    ObservedIdempotentMessageDelivery, PublicTurnDeliveryState,
};
use nomifun_conversation::{ConversationService, IdempotentMessageDelivery};

/// Exact Session operations needed by the Cron domain.
#[async_trait]
pub trait CronSessionPort: Send + Sync {
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

    fn begin_runtime_preparation(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<RuntimeBuildLease, AppError>;

    async fn send_observed_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
        build_lease: RuntimeBuildLease,
        runtime_preparation: BackgroundTurnRuntimePreparation,
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

    fn begin_runtime_preparation(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<RuntimeBuildLease, AppError> {
        self.service
            .begin_public_runtime_preparation(session_id, user_id)
    }

    async fn send_observed_turn(
        &self,
        user_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
        build_lease: RuntimeBuildLease,
        runtime_preparation: BackgroundTurnRuntimePreparation,
    ) -> Result<ObservedIdempotentMessageDelivery, AppError> {
        self.service
            .send_observed_background_message_with_idempotency_key(
                user_id,
                session_id,
                idempotency_key,
                request,
                &self.runtime_registry,
                build_lease,
                runtime_preparation,
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
