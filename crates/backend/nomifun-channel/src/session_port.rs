//! Typed Session boundary used by Channel message delivery.

use async_trait::async_trait;
use nomifun_ai_agent::AgentStreamEvent;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ListMessagesQuery, MessageListResponse,
    SendMessageRequest,
};
use nomifun_common::AppError;
use tokio::sync::broadcast;

/// Channel-owned projection of one admitted turn and its optional live event
/// stream. The host may use any runtime implementation; the Channel domain
/// never receives a runtime registry or Conversation service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelTurnDeliveryReceipt {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

pub struct ChannelTurnDelivery {
    pub delivery: ChannelTurnDeliveryReceipt,
    pub events: Option<broadcast::Receiver<AgentStreamEvent>>,
}

/// Channel-owned read-only projection of one keyed turn receipt.
///
/// The app host may project a historical runtime receipt into this shape, but
/// that implementation type must not leak into Channel consumers. This
/// projection carries only the delivery facts needed for Channel
/// retry/notification decisions and has no send or settlement authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelCompletedTurnReceipt {
    pub message_id: String,
    pub replayed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

/// Read-only receipt state used by Channel delivery code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelTurnReceiptState {
    Missing,
    Accepted { message_id: String },
    Completed(ChannelCompletedTurnReceipt),
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
    ) -> Result<ChannelTurnReceiptState, AppError>;

    /// Read the exact keyed turn outcome without creating a new turn or
    /// inferring completion from a transient event stream.
    async fn read_turn_receipt(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
    ) -> Result<ChannelTurnReceiptState, AppError> {
        self.turn_outcome(owner_id, session_id, idempotency_key)
            .await
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn completed_delivery() -> ChannelTurnDeliveryReceipt {
        ChannelTurnDeliveryReceipt {
            message_id: "message-1".to_owned(),
            replayed: true,
            completed: true,
            result_ok: Some(false),
            result_text: Some("partial".to_owned()),
            result_error: Some("provider timeout".to_owned()),
            result_error_code: Some("provider_timeout".to_owned()),
            result_error_retryable: Some(true),
        }
    }

    #[test]
    fn receipt_projection_preserves_completed_delivery_facts() {
        let delivery = completed_delivery();
        let projected = ChannelTurnReceiptState::Completed(ChannelCompletedTurnReceipt {
            message_id: delivery.message_id.clone(),
            replayed: delivery.replayed,
            result_ok: delivery.result_ok,
            result_text: delivery.result_text.clone(),
            result_error: delivery.result_error.clone(),
            result_error_code: delivery.result_error_code.clone(),
            result_error_retryable: delivery.result_error_retryable,
        });

        assert!(delivery.completed);
        assert_eq!(
            projected,
            ChannelTurnReceiptState::Completed(ChannelCompletedTurnReceipt {
                message_id: "message-1".to_owned(),
                replayed: true,
                result_ok: Some(false),
                result_text: Some("partial".to_owned()),
                result_error: Some("provider timeout".to_owned()),
                result_error_code: Some("provider_timeout".to_owned()),
                result_error_retryable: Some(true),
            })
        );
    }

    #[test]
    fn receipt_projection_preserves_missing_and_accepted_states() {
        assert_eq!(
            ChannelTurnReceiptState::Missing,
            ChannelTurnReceiptState::Missing
        );
        assert_eq!(
            ChannelTurnReceiptState::Accepted {
                message_id: "message-2".to_owned(),
            },
            ChannelTurnReceiptState::Accepted {
                message_id: "message-2".to_owned(),
            }
        );
    }
}
