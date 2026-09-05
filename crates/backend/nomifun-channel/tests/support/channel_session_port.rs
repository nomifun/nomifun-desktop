//! Test-only Conversation-backed Channel session port.
//!
//! The production Channel crate depends only on its typed session contract.
//! These integration tests still exercise the current Conversation owner, so
//! the compatibility wiring lives here instead of in `nomifun-channel/src`.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::{AgentRuntimeRegistry, AgentStreamEvent};
use nomifun_api_types::{
    ConversationResponse, ConversationRuntimeStateKind, CreateConversationRequest,
    ListMessagesQuery, MessageListResponse, SendMessageRequest,
};
use nomifun_common::AppError;
use nomifun_conversation::{
    ConversationService, IdempotentMessageDelivery, PublicTurnDeliveryState,
};
use tokio::sync::broadcast;
use tracing::warn;

struct ConversationChannelSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

#[async_trait]
impl nomifun_channel::ChannelSessionPort for ConversationChannelSessionPort {
    async fn is_busy(&self, session_id: &str) -> bool {
        let summary = self.service.runtime_summary_for(session_id).await;
        matches!(
            summary.state,
            ConversationRuntimeStateKind::Starting | ConversationRuntimeStateKind::Running
        )
    }

    async fn turn_outcome(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<nomifun_channel::ChannelTurnReceiptState, AppError> {
        self.service
            .public_turn_delivery_state(owner_id, session_id, idempotency_key)
            .await
            .map(channel_turn_receipt_state_from_conversation)
    }

    async fn cancel(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        self.service
            .cancel(owner_id, session_id, &self.runtime_registry)
            .await
    }

    async fn list_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError> {
        self.service
            .list_messages(owner_id, session_id, query)
            .await
    }

    async fn send_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<nomifun_channel::ChannelTurnDelivery, AppError> {
        let delivery = self
            .service
            .send_message_with_idempotency_key(
                owner_id,
                session_id,
                idempotency_key,
                request,
                &self.runtime_registry,
            )
            .await?;
        let events = if delivery.completed {
            None
        } else {
            wait_for_runtime_subscription(&self.runtime_registry, session_id).await
        };
        Ok(nomifun_channel::ChannelTurnDelivery {
            delivery: channel_delivery_from_conversation(delivery),
            events,
        })
    }

    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, session_id).await
    }

    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .create_idempotent(owner_id, request, creation_key)
            .await
    }
}

/// Build the test-only Conversation-backed Channel session port.
pub fn conversation_channel_session_port(
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> Arc<dyn nomifun_channel::ChannelSessionPort> {
    Arc::new(ConversationChannelSessionPort {
        service,
        runtime_registry,
    })
}

async fn wait_for_runtime_subscription(
    runtime_registry: &Arc<dyn AgentRuntimeRegistry>,
    session_id: &str,
) -> Option<broadcast::Receiver<AgentStreamEvent>> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(handle) = runtime_registry.get_runtime(session_id) {
            return Some(handle.subscribe());
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                session_id,
                "runtime did not register before channel relay subscription timeout"
            );
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

fn channel_delivery_from_conversation(
    delivery: IdempotentMessageDelivery,
) -> nomifun_channel::ChannelTurnDeliveryReceipt {
    nomifun_channel::ChannelTurnDeliveryReceipt {
        message_id: delivery.message_id,
        replayed: delivery.replayed,
        completed: delivery.completed,
        result_ok: delivery.result_ok,
        result_text: delivery.result_text,
        result_error: delivery.result_error,
        result_error_code: delivery.result_error_code,
        result_error_retryable: delivery.result_error_retryable,
    }
}

fn channel_turn_receipt_state_from_conversation(
    state: PublicTurnDeliveryState,
) -> nomifun_channel::ChannelTurnReceiptState {
    match state {
        PublicTurnDeliveryState::Missing => nomifun_channel::ChannelTurnReceiptState::Missing,
        PublicTurnDeliveryState::Accepted { message_id } => {
            nomifun_channel::ChannelTurnReceiptState::Accepted { message_id }
        }
        PublicTurnDeliveryState::Completed(delivery) => {
            nomifun_channel::ChannelTurnReceiptState::Completed(
                nomifun_channel::ChannelCompletedTurnReceipt {
                    message_id: delivery.message_id,
                    replayed: delivery.replayed,
                    result_ok: delivery.result_ok,
                    result_text: delivery.result_text,
                    result_error: delivery.result_error,
                    result_error_code: delivery.result_error_code,
                    result_error_retryable: delivery.result_error_retryable,
                },
            )
        }
    }
}
