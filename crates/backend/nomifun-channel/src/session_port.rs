//! Typed Session boundary used by Channel message delivery.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::{AgentRuntimeRegistry, AgentStreamEvent};
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ListMessagesQuery, MessageListResponse,
    SendMessageRequest,
};
use nomifun_common::AppError;
use nomifun_conversation::{
    ConversationService, IdempotentMessageDelivery, PublicTurnDeliveryState,
};
use tokio::sync::broadcast;
use tracing::warn;

/// One admitted Channel turn and its optional live event stream.
pub struct ChannelTurnDelivery {
    pub delivery: IdempotentMessageDelivery,
    pub events: Option<broadcast::Receiver<AgentStreamEvent>>,
}

/// Exact Session command/query surface used by the Channel domain.
#[async_trait]
pub trait ChannelSessionPort: Send + Sync {
    async fn is_busy(&self, session_id: &str) -> bool;

    async fn turn_outcome(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<PublicTurnDeliveryState, AppError>;

    async fn cancel(&self, owner_id: &str, session_id: &str) -> Result<(), AppError>;

    async fn list_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        query: ListMessagesQuery,
    ) -> Result<MessageListResponse, AppError>;

    async fn send_turn(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        request: SendMessageRequest,
    ) -> Result<ChannelTurnDelivery, AppError>;

    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError>;
}

struct ConversationChannelSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

#[async_trait]
impl ChannelSessionPort for ConversationChannelSessionPort {
    async fn is_busy(&self, session_id: &str) -> bool {
        use nomifun_api_types::ConversationRuntimeStateKind;

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
    ) -> Result<PublicTurnDeliveryState, AppError> {
        self.service
            .public_turn_delivery_state(owner_id, session_id, idempotency_key)
            .await
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
    ) -> Result<ChannelTurnDelivery, AppError> {
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
        Ok(ChannelTurnDelivery { delivery, events })
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

/// Build the transitional Conversation-backed Channel Session port.
///
/// It delegates to one existing Session/runtime owner and retains no facts,
/// fallback, or alternate identity.
pub fn conversation_channel_session_port(
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> Arc<dyn ChannelSessionPort> {
    Arc::new(ConversationChannelSessionPort {
        service,
        runtime_registry,
    })
}
