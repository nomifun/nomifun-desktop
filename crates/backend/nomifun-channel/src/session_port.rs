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
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
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
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

impl ChannelCompletedTurnReceipt {
    /// A question is a valid channel reply, not completion of the user's task.
    pub fn has_public_reply(&self) -> bool {
        self.result_ok == Some(true)
            && matches!(self.turn_outcome,
                Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { .. }
                    | nomifun_agent_contracts::AgentTurnOutcome::InputRequired { .. }))
            && self.result_text.as_ref().is_some_and(|text| !text.trim().is_empty())
    }
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

    /// Resolve the existing canonical opening identity for a dedicated group
    /// chat. A miss creates nothing and never guesses from display metadata.
    async fn find_created_session(&self, _owner_id: &str, _creation_key: &str)
        -> Result<Option<String>, AppError> { Ok(None) }

    /// Only the Channel owner calls this after authoritative auto-group
    /// admission. The Host freezes a real model-only binding; presentation
    /// extras cannot supply or broaden that authority.
    async fn create_group_guest_idempotent(&self, _owner_id: &str,
        _request: CreateConversationRequest, _creation_key: &str) -> Result<ConversationResponse, AppError> {
        Err(AppError::Conflict("model-only group guest creation is unavailable".into()))
    }

    /// Recheck the original guest creation command and its subtractive ceiling
    /// at the same canonical input boundary on every guest invocation.
    async fn send_group_guest_turn(&self, _owner_id: &str, _session_id: &str,
        _idempotency_key: &str, _request: SendMessageRequest, _creation_key: &str) -> Result<ChannelTurnDelivery, AppError> {
        Err(AppError::Conflict("model-only group guest delivery is unavailable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_channel_turn_requires_a_public_reply_and_preserves_questions() {
        let mut receipt = ChannelCompletedTurnReceipt {
            message_id: "input".into(), replayed: true,
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { request_id: "ask".into() }),
            result_ok: Some(true), result_text: Some("请提供姓名。".into()),
            result_error: None, result_error_code: None, result_error_retryable: Some(false),
        };
        assert!(receipt.has_public_reply());
        receipt.result_text = None;
        assert!(!receipt.has_public_reply());
        receipt.result_text = Some("earlier progress".into());
        receipt.turn_outcome = Some(nomifun_agent_contracts::AgentTurnOutcome::Unavailable);
        assert!(!receipt.has_public_reply());
        receipt.turn_outcome = None;
        assert!(!receipt.has_public_reply());
    }

    fn completed_delivery() -> ChannelTurnDeliveryReceipt {
        ChannelTurnDeliveryReceipt {
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Failed { message_ids: vec![] }),
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
            turn_outcome: delivery.turn_outcome.clone(),
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
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Failed { message_ids: vec![] }),
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
