//! Narrow typed Session boundary used by Requirements/AutoWork.
//!
//! The Requirement domain owns claims, retries, verdicts, and durable
//! Requirement facts. The host-owned Session implementation owns turn
//! admission, runtime preparation, delivery receipts, and cancellation. This
//! module keeps that ownership explicit without exposing the host
//! implementation or its runtime registry to the Requirement service or runner.

use async_trait::async_trait;
use nomifun_api_types::SendMessageRequest;
use nomifun_common::AppError;
use nomifun_conversation::runtime_state::RuntimeBuildLease;
use nomifun_conversation::service::{
    BackgroundTurnReconciliationDisposition, BackgroundTurnRuntimePreparation,
    ObservedIdempotentMessageDelivery,
};
use nomifun_conversation::{IdempotentMessageDelivery, PublicTurnDeliveryState};
use nomifun_db::RequirementConversationTurnAuthority;

/// Exact typed command/query surface AutoWork needs from the host-owned Session.
#[async_trait]
pub trait AutoWorkSessionPort: Send + Sync {
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

    async fn public_turn_delivery_state(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<PublicTurnDeliveryState, AppError>;

    async fn reconcile_quiescent_running_turn(
        &self,
        user_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<BackgroundTurnReconciliationDisposition, AppError>;
}

/// Source-compatible name retained for the existing host composition.
///
/// This is only a trait alias. It does not construct an adapter, own runtime
/// state, or create a second Session authority.
pub use AutoWorkSessionPort as AutoWorkConversationPort;
