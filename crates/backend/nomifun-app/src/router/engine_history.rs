//! Bounded Runtime history reconstructed from canonical AgentSession facts.

use nomifun_agent_contracts::{AgentSessionId, OperationId, SessionEventRecord};
use nomifun_agent_session::{AgentSessionStore, ChatCausalityFacts, MessageProjection};
use nomifun_common::AppError;
use serde_json::{Value, json};

use super::engine_session_host::EngineTurnReceipt;

pub struct EngineHistoryRecord {
    pub sequence: i64,
    pub event_json: String,
    pub model_operation_id: Option<String>,
    pub model_claimed: bool,
}

pub struct EngineHistoryTurn {
    pub operation_id: String,
    pub root_message_id: String,
    pub receipt_status: String,
    pub root_content_json: String,
    pub request_payload_json: String,
    pub records: Vec<EngineHistoryRecord>,
    pub serialized_bytes: usize,
}

pub struct EngineHistoryWindow {
    pub turns: Vec<EngineHistoryTurn>,
    pub has_older: bool,
}

pub struct EngineHistoryMessage {
    pub kind: String,
    pub position: Option<String>,
    pub content_json: String,
}

pub struct EngineMessageHistoryWindow {
    pub messages: Vec<EngineHistoryMessage>,
    pub has_older: bool,
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine history: {message}"))
}

async fn facts(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
) -> Result<ChatCausalityFacts, AppError> {
    store
        .chat_causality_facts(
            &AgentSessionId::from(receipt.session().session().conversation_id.clone()),
            &OperationId::from(receipt.operation_id().to_owned()),
        )
        .await
        .map_err(failure)
}

fn payload<'a>(facts: &'a ChatCausalityFacts, event: &SessionEventRecord) -> Result<&'a Value, AppError> {
    facts
        .event_payloads
        .get(event.event_id.as_ref())
        .ok_or_else(|| failure(format!("event {} has no resolved payload", event.event_id.as_ref())))
}

fn source_message<'a>(
    facts: &'a ChatCausalityFacts,
    turn: &SessionEventRecord,
) -> Result<(&'a SessionEventRecord, &'a Value), AppError> {
    let source_id = payload(facts, turn)?
        .get("source_message_id")
        .and_then(Value::as_str)
        .ok_or_else(|| failure("turn has no source message identity"))?;
    let source = facts
        .events
        .iter()
        .find(|event| event.event_id.as_ref() == source_id)
        .ok_or_else(|| failure("turn source message is missing"))?;
    Ok((source, payload(facts, source)?))
}

fn projection_content(message: &MessageProjection) -> Option<&str> {
    message.projection.get("content").and_then(Value::as_str)
}

pub(super) async fn load_messages(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    limit: usize,
    byte_limit: usize,
) -> Result<EngineMessageHistoryWindow, AppError> {
    load_messages_before(store, receipt, limit, byte_limit, None).await
}

pub(super) async fn load_messages_before(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    limit: usize,
    byte_limit: usize,
    before_operation: Option<&str>,
) -> Result<EngineMessageHistoryWindow, AppError> {
    if !(1..=4096).contains(&limit) || !(1..=8 * 1024 * 1024).contains(&byte_limit) {
        return Err(failure("invalid compatibility history budget"));
    }
    if before_operation
        .is_some_and(|id| id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control))
    {
        return Err(failure("invalid historical turn cursor"));
    }
    let facts = facts(store, receipt).await?;
    let floor = facts
        .events
        .iter()
        .filter(|event| event.kind.0 == "context/cleared")
        .map(|event| event.seq)
        .max()
        .unwrap_or(0);
    let current_root = facts
        .events
        .iter()
        .find(|event| event.event_id.as_ref() == receipt.root_message_id())
        .ok_or_else(|| failure("current root message is missing"))?;
    let before_seq = if let Some(operation) = before_operation {
        let turn = facts
            .events
            .iter()
            .find(|event| {
                event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation
            })
            .ok_or_else(|| failure("historical cursor is outside the current Session"))?;
        source_message(&facts, turn)?.0.seq
    } else {
        current_root.seq
    };
    let session_id = AgentSessionId::from(receipt.session().session().conversation_id.clone());
    let mut projections = store.messages_after(&session_id, 0).await.map_err(failure)?;
    projections.retain(|message| {
        message.last_seq < before_seq
            && message.first_seq > floor
            && message.presentation_intent == "message"
            && projection_content(message).is_some_and(|content| !content.is_empty())
    });
    projections.sort_by(|left, right| {
        right
            .last_seq
            .cmp(&left.last_seq)
            .then_with(|| right.projection_id.cmp(&left.projection_id))
    });
    let mut window = EngineMessageHistoryWindow {
        has_older: projections.len() > limit,
        messages: Vec::new(),
    };
    let mut bytes = 0usize;
    for projection in projections.into_iter().take(limit) {
        let content = projection_content(&projection).unwrap_or_default();
        let content_json = serde_json::to_string(&json!({ "content": content }))
            .map_err(failure)?;
        let position = projection
            .projection
            .get("state")
            .and_then(Value::as_str)
            .map(|state| if state == "accepted" { "right" } else { "left" }.to_owned());
        let next = bytes
            .saturating_add(content_json.len())
            .saturating_add(position.as_ref().map_or(0, String::len))
            .saturating_add(4);
        if next > byte_limit {
            if window.messages.is_empty() && before_operation.is_none() {
                return Err(failure("latest compatibility message exceeds budget"));
            }
            window.has_older = true;
            break;
        }
        window.messages.push(EngineHistoryMessage {
            kind: "text".to_owned(),
            position,
            content_json,
        });
        bytes = next;
    }
    Ok(window)
}

pub(super) async fn load(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    limit: usize,
) -> Result<EngineHistoryWindow, AppError> {
    load_before(store, receipt, limit, None).await
}

pub(super) async fn load_before(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    limit: usize,
    before_operation: Option<&str>,
) -> Result<EngineHistoryWindow, AppError> {
    if !(1..=32).contains(&limit) {
        return Err(failure("turn limit must be 1..32"));
    }
    let facts = facts(store, receipt).await?;
    let floor = facts
        .events
        .iter()
        .filter(|event| event.kind.0 == "context/cleared")
        .map(|event| event.seq)
        .max()
        .unwrap_or(0);
    let current_root = facts
        .events
        .iter()
        .find(|event| event.event_id.as_ref() == receipt.root_message_id())
        .ok_or_else(|| failure("current root message is missing"))?;
    let before_seq = match before_operation {
        Some(operation) => facts
            .events
            .iter()
            .find(|event| {
                event.kind.0 == "turn/started" && event.correlation_id.as_ref() == operation
            })
            .ok_or_else(|| failure("historical cursor is outside the current Session"))?
            .seq,
        None => current_root.seq,
    };
    let mut turns = facts
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "turn/started" && event.seq < before_seq && event.seq > floor
        })
        .collect::<Vec<_>>();
    turns.sort_by_key(|event| std::cmp::Reverse(event.seq));
    let mut window = EngineHistoryWindow {
        has_older: turns.len() > limit,
        turns: Vec::new(),
    };
    let mut total = 0usize;
    for turn in turns.into_iter().take(limit) {
        let operation_id = turn.correlation_id.as_ref().to_owned();
        let (root, root_payload) = source_message(&facts, turn)?;
        let terminal = facts.events.iter().find(|event| {
            event.correlation_id == turn.correlation_id
                && matches!(
                    event.kind.0.as_str(),
                    "turn/completed" | "turn/failed" | "turn/cancelled"
                )
        });
        let receipt_status = terminal
            .map(|event| event.kind.0.trim_start_matches("turn/").to_owned())
            .unwrap_or_else(|| "running".to_owned());
        let mut records = Vec::new();
        let mut serialized_bytes = serde_json::to_vec(root_payload)
            .map_err(failure)?
            .len();
        let mut progress = facts
            .events
            .iter()
            .filter(|event| {
                event.kind.0 == "runtime/progress-recorded"
                    && event.correlation_id == turn.correlation_id
            })
            .collect::<Vec<_>>();
        progress.sort_by_key(|event| event.seq);
        for (index, event) in progress.into_iter().enumerate() {
            let value = payload(&facts, event)?;
            let sequence = value
                .get("producer_seq")
                .and_then(Value::as_u64)
                .unwrap_or(index as u64 + 1);
            if sequence != index as u64 + 1 {
                return Err(failure("journal sequence is incomplete"));
            }
            let runtime_event = value
                .get("event")
                .ok_or_else(|| failure("runtime progress record has no event"))?;
            let event_json = serde_json::to_string(runtime_event).map_err(failure)?;
            let model_operation_id = runtime_event
                .get("data")
                .and_then(|data| data.get("operation_id"))
                .or_else(|| runtime_event.get("operation_id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let model_claimed = model_operation_id
                .as_ref()
                .is_some_and(|operation| facts.operation_ids.contains(operation));
            serialized_bytes = serialized_bytes
                .saturating_add(event_json.len())
                .saturating_add(model_operation_id.as_ref().map_or(0, String::len));
            records.push(EngineHistoryRecord {
                sequence: i64::try_from(sequence).map_err(failure)?,
                event_json,
                model_operation_id,
                model_claimed,
            });
        }
        if records.len() > 4096
            || serialized_bytes > 8 * 1024 * 1024
            || total.saturating_add(serialized_bytes) > 16 * 1024 * 1024
        {
            if window.turns.is_empty() {
                return Err(failure("latest turn exceeds the history budget"));
            }
            window.has_older = true;
            break;
        }
        total = total.saturating_add(serialized_bytes);
        let root_content_json = serde_json::to_string(root_payload).map_err(failure)?;
        window.turns.push(EngineHistoryTurn {
            operation_id,
            root_message_id: root.event_id.as_ref().to_owned(),
            receipt_status,
            request_payload_json: root_content_json.clone(),
            root_content_json,
            records,
            serialized_bytes,
        });
    }
    Ok(window)
}
