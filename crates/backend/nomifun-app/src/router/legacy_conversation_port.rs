//! Compatibility adapter for the legacy conversation-backed Gateway.
//!
//! This is the only app-side implementation of the Gateway's conversation
//! transport port. It owns no additional state: every operation delegates to
//! the existing conversation service and process-local runtime registry.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{
    ConversationListResponse, ConversationResponse, CreateConversationRequest,
    ListConversationsQuery, ListMessagesQuery, MessageListResponse, SendMessageRequest,
    UpdateConversationRequest,
};
use nomifun_common::{AgentType, AppError};
use nomifun_conversation::{
    ConversationService, DeliveryNotifyRegistration as ServiceNotifyRegistration,
    IdempotentMessageDelivery,
};
use nomifun_db::models::ConversationDeliveryReceiptRow;

use nomifun_gateway::{
    ConversationCapabilityPort, ConversationCreateSpec, ConversationDeliveryReceipt,
    DeliveryNotifyRegistration as GatewayNotifyRegistration,
};

#[derive(Clone)]
pub(crate) struct LegacyConversationCapabilityPort {
    service: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

impl LegacyConversationCapabilityPort {
    pub(crate) fn new(
        service: ConversationService,
        runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    ) -> Self {
        Self {
            service,
            runtime_registry,
        }
    }
}

#[async_trait]
impl ConversationCapabilityPort for LegacyConversationCapabilityPort {
    async fn list(
        &self,
        user_id: &str,
        query: ListConversationsQuery,
        exclude_companion_companion: bool,
    ) -> Result<ConversationListResponse, AppError> {
        self.service
            .list(user_id, query, exclude_companion_companion)
            .await
    }

    async fn runtime_summary_for(
        &self,
        conversation_id: &str,
    ) -> nomifun_api_types::ConversationRuntimeSummary {
        self.service.runtime_summary_for(conversation_id).await
    }

    async fn latest_completed_turn_receipt(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Option<ConversationDeliveryReceipt>, AppError> {
        self.service
            .latest_completed_turn_receipt(user_id, conversation_id)
            .await
            .map(|receipt| receipt.map(receipt_row_to_gateway))
    }

    async fn list_messages(
        &self,
        user_id: &str,
        conversation_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        self.service
            .list_messages(user_id, conversation_id, query)
            .await
    }

    async fn register_delivery_notify(
        &self,
        user_id: &str,
        target_conversation_id: &str,
        idempotency_key: &str,
        requester_conversation_id: &str,
    ) -> Result<GatewayNotifyRegistration, AppError> {
        self.service
            .register_delivery_notify(
                user_id,
                target_conversation_id,
                idempotency_key,
                requester_conversation_id,
            )
            .await
            .map(|result| match result {
                ServiceNotifyRegistration::Registered => {
                    GatewayNotifyRegistration::Registered
                }
                ServiceNotifyRegistration::RefusedDeliveryNotifyOrigin => {
                    GatewayNotifyRegistration::RefusedDeliveryNotifyOrigin
                }
            })
    }

    async fn send_message_with_idempotency_key(
        &self,
        user_id: &str,
        conversation_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<ConversationDeliveryReceipt, AppError> {
        self.service
            .send_message_with_idempotency_key(
                user_id,
                conversation_id,
                idempotency_key,
                request,
                &self.runtime_registry,
            )
            .await
            .map(receipt_to_gateway)
    }

    async fn get(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(user_id, conversation_id).await
    }

    async fn create(
        &self,
        user_id: &str,
        request: ConversationCreateSpec,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .create(
                user_id,
                CreateConversationRequest {
                    r#type: AgentType::Nomi,
                    name: request.name,
                    model: request.model,
                    source: None,
                    channel_chat_id: None,
                    preset_id: None,
                    preset_overrides: None,
                    delegation_policy: Default::default(),
                    execution_model_pool: None,
                    decision_policy: Default::default(),
                    execution_template_id: None,
                    extra: request.extra,
                },
            )
            .await
    }

    async fn update(
        &self,
        user_id: &str,
        conversation_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .update(
                user_id,
                conversation_id,
                request,
                &self.runtime_registry,
            )
            .await
    }

    async fn delete(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError> {
        self.service.delete(user_id, conversation_id).await
    }

    async fn cancel(&self, user_id: &str, conversation_id: &str) -> Result<(), AppError> {
        self.service
            .cancel(user_id, conversation_id, &self.runtime_registry)
            .await
    }

    async fn supports_scheduled_model_reconciliation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<bool, AppError> {
        let conversation = self.service.get(user_id, conversation_id).await?;
        Ok(conversation.r#type == AgentType::Nomi)
    }
}

fn receipt_to_gateway(receipt: IdempotentMessageDelivery) -> ConversationDeliveryReceipt {
    ConversationDeliveryReceipt {
        message_id: receipt.message_id,
        replayed: receipt.replayed,
        completed: receipt.completed,
        result_ok: receipt.result_ok,
        result_text: receipt.result_text,
        result_error: receipt.result_error,
        result_error_code: receipt.result_error_code,
        result_error_retryable: receipt.result_error_retryable,
    }
}

fn receipt_row_to_gateway(
    receipt: ConversationDeliveryReceiptRow,
) -> ConversationDeliveryReceipt {
    ConversationDeliveryReceipt {
        message_id: receipt.message_id,
        replayed: false,
        completed: receipt.status == "completed",
        result_ok: receipt.result_ok,
        result_text: receipt.result_text,
        result_error: receipt.result_error,
        result_error_code: receipt.result_error_code,
        result_error_retryable: receipt.result_error_retryable,
    }
}
