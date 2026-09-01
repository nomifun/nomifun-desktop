//! Typed compatibility port for conversation transport operations.
//!
//! The Gateway transport does not own conversation facts or runtime handles.
//! The app composition supplies one adapter for the legacy compatibility
//! surface; Fresh-v4 AgentSession routes do not construct this port.

use async_trait::async_trait;
use nomifun_api_types::{
    ConversationListResponse, ConversationResponse, ListConversationsQuery,
    ListMessagesQuery, MessageListResponse, SendMessageRequest, UpdateConversationRequest,
};
use nomifun_common::{AppError, ProviderWithModel};
use serde_json::Value;

/// Runtime-neutral create request projected by the Gateway compatibility
/// surface. The app adapter decides how an old storage DTO is populated; the
/// Gateway itself does not select a runtime implementation.
#[derive(Debug, Clone)]
pub struct ConversationCreateSpec {
    pub name: Option<String>,
    pub model: Option<ProviderWithModel>,
    pub extra: Value,
}

/// The small durable receipt projection needed by Gateway callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationDeliveryReceipt {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

/// Result of registering a completion notification for a target turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryNotifyRegistration {
    Registered,
    RefusedDeliveryNotifyOrigin,
}

/// Compatibility transport operations required by the Gateway capability
/// facade. Implementations must delegate to the owning session service and
/// must not cache state, select a runtime, or create a second authority.
#[async_trait]
pub trait ConversationCapabilityPort: Send + Sync {
    async fn list(
        &self,
        user_id: &str,
        query: ListConversationsQuery,
        exclude_companion_companion: bool,
    ) -> Result<ConversationListResponse, AppError>;

    async fn runtime_summary_for(
        &self,
        conversation_id: &str,
    ) -> nomifun_api_types::ConversationRuntimeSummary;

    async fn latest_completed_turn_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Option<ConversationDeliveryReceipt>, AppError>;

    async fn list_messages(
        &self,
        user_id: &str,
        conversation_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError>;

    async fn register_delivery_notify(
        &self,
        user_id: &str,
        target_conversation_id: &str,
        idempotency_key: &str,
        requester_conversation_id: &str,
    ) -> Result<DeliveryNotifyRegistration, AppError>;

    async fn send_message_with_idempotency_key(
        &self,
        user_id: &str,
        conversation_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<ConversationDeliveryReceipt, AppError>;

    async fn get(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn create(
        &self,
        user_id: &str,
        request: ConversationCreateSpec,
    ) -> Result<ConversationResponse, AppError>;

    async fn update(
        &self,
        user_id: &str,
        conversation_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError>;

    async fn delete(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError>;

    async fn cancel(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError>;

    /// Compatibility Cron needs to know whether the owner permits automatic
    /// model reconciliation for this conversation kind. The decision remains
    /// with the app-side owner rather than leaking a runtime enum into Gateway.
    async fn supports_scheduled_model_reconciliation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<bool, AppError>;
}
