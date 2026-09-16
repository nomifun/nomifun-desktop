//! Scoped read-only adapter; Conversation owns rows and Coding owns its codec.
use super::super::engine_session_host::{EngineSessionHost, EngineTurnReceipt};
use nomifun_chat_model_broker::{ChatCausality, ChatContentPart, ChatMessage, ChatRole};
use nomifun_coding_engine::{
    CodingEngineError, CodingEngineEvent, CodingHistoryPage, CodingHistoryPort, CodingRecordedTurn,
};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) struct HistoryPort {
    pub host: Arc<EngineSessionHost>,
    pub receipt: EngineTurnReceipt,
    pub cancellation: CancellationToken,
}

impl std::fmt::Debug for HistoryPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodingHistoryPort(scoped)")
    }
}
fn invalid() -> CodingEngineError {
    CodingEngineError::ContextAssembly(
        "Historical turn unavailable or outside the admitted history contract".into(),
    )
}

#[async_trait::async_trait]
impl CodingHistoryPort for HistoryPort {
    async fn read_previous(
        &self,
        causality: &ChatCausality,
        before_operation: Option<&str>,
    ) -> Result<CodingHistoryPage, CodingEngineError> {
        if self.cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        if causality.agent_session_id.as_ref() != self.receipt.session().session().conversation_id
            || causality.turn_operation_id.as_ref() != self.receipt.operation_id()
            || causality.causation_event_id.as_ref() != self.receipt.root_message_id()
            || causality.resolved_snapshot_ref != self.receipt.session().snapshot().snapshot_ref
        {
            return Err(invalid());
        }
        let mut window = self
            .host
            .read_history_before(&self.receipt, 1, before_operation)
            .await
            .map_err(|_| invalid())?;
        if self.cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let Some(turn) = window.turns.pop() else {
            return Ok(CodingHistoryPage {
                turn: None,
                has_older: window.has_older,
            });
        };
        let root: Value = serde_json::from_str(&turn.root_content_json).map_err(|_| invalid())?;
        let text = root
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let receipt: Value =
            serde_json::from_str(&turn.request_payload_json).map_err(|_| invalid())?;
        let files =
            super::super::coding_attachments::references(&receipt).map_err(|_| invalid())?;
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(ChatContentPart::Text { text: text.into() });
        }
        if let Some(description) = super::super::coding_attachments::description(&files, true) {
            content.push(description);
        }
        if content.is_empty() {
            return Err(invalid());
        }
        let mut events = Vec::new();
        for record in turn.records {
            let value: Value = serde_json::from_str(&record.event_json).map_err(|_| invalid())?;
            if matches!(
                value.get("event").and_then(Value::as_str),
                Some(
                    "host_tool_dispatch"
                        | "host_tool_settled"
                        | "host_resource_dispatch"
                        | "host_resource_settled"
                        | "host_process_dispatch"
                        | "host_process_quiescent"
                        | "host_cleanup_proven"
                )
            ) {
                continue;
            }
            events.push(serde_json::from_value::<CodingEngineEvent>(value).map_err(|_| invalid())?);
        }
        Ok(CodingHistoryPage {
            has_older: window.has_older,
            turn: Some(CodingRecordedTurn {
                operation_id: turn.operation_id,
                receipt_status: turn.receipt_status,
                requirement: ChatMessage {
                    role: ChatRole::User,
                    content,
                    provider_round_id: None,
                },
                events,
            }),
        })
    }
}
