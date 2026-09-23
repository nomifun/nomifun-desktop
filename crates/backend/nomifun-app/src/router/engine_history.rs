//! Bounded Runtime history reconstructed from canonical AgentSession facts.

use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    AgentBindingChangedPayloadV1, AgentBindingValue, AgentHandoffBindingRefV1, AgentSessionId,
    OperationId, SessionEventRecord,
};
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

fn native_replay_floor(
    events: &[SessionEventRecord],
    event_payloads: &BTreeMap<String, Value>,
    current: &AgentBindingValue,
) -> Result<u64, AppError> {
    let context_floor = events
        .iter()
        .filter(|event| event.kind.0 == "context/cleared")
        .map(|event| event.seq)
        .max()
        .unwrap_or(0);
    let transition = events
        .iter()
        .filter(|event| event.kind.0 == "session/agent-binding-changed")
        .max_by_key(|event| event.seq);
    let Some(transition) = transition else {
        return Ok(context_floor);
    };
    let payload = event_payloads
        .get(transition.event_id.as_ref())
        .ok_or_else(|| failure("Agent transition event has no resolved payload"))?;
    let payload: AgentBindingChangedPayloadV1 = serde_json::from_value(payload.clone())
        .map_err(|error| failure(format!("Agent transition event is invalid: {error}")))?;
    let previous_version = payload.previous_binding_ref.binding_version;
    let current_ref = AgentHandoffBindingRefV1::from(current);
    let target_reached_current = payload.next_binding_ref.binding_version
        < current_ref.binding_version
        || payload.next_binding_ref == current_ref;
    if !target_reached_current
        || payload.completion_gate_inherited
        || payload.transition_id.as_ref() != transition.correlation_id.as_ref()
        || payload.effective_after_seq >= transition.seq
        || payload.next_binding_ref.binding_version != previous_version.saturating_add(1)
        || (payload.previous_binding_ref.preset_revision_ref
            == payload.next_binding_ref.preset_revision_ref
            && payload.previous_binding_ref.resolved_snapshot_ref
                == payload.next_binding_ref.resolved_snapshot_ref)
    {
        return Err(failure(
            "Agent transition event does not prove the exact current binding boundary",
        ));
    }
    Ok(context_floor.max(transition.seq))
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
    // Native Runtime replay is binding-specific. A canonical Agent transition
    // is the only boundary that permits older mismatched turns to fall back to
    // data-only message projection; an unmarked mismatch remains visible to the
    // runtime compatibility check and fails closed.
    let floor = native_replay_floor(
        &facts.events,
        &facts.event_payloads,
        receipt.session().agent_binding(),
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentHandoffMode, AgentPresetId, CorrelationId, DigestHex, EventId, EventProducerId,
        IdempotencyKey, OperationId, PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        SessionEventKind, SessionEventPayloadRef,
    };

    fn event(seq: u64, kind: &str) -> SessionEventRecord {
        SessionEventRecord {
            agent_session_id: AgentSessionId::from(
                "0190f5fe-7c00-7a00-8000-000000000001",
            ),
            seq,
            event_id: EventId::from(format!("event-{seq}")),
            producer_id: EventProducerId::from("test"),
            idempotency_key: IdempotencyKey::from(format!("key-{seq}")),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            kind: SessionEventKind(kind.to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId::from(format!("correlation-{seq}")),
            causation_event_id: None,
            payload: SessionEventPayloadRef::Empty,
        }
    }

    fn binding(version: u64, suffix: &str) -> AgentBindingValue {
        AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from(format!("preset-{suffix}")),
                revision: 1,
                revision_digest: DigestHex::from("a".repeat(64)),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from(format!("snapshot-{suffix}")),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
            typed_resource_bindings: Vec::new(),
            binding_version: version,
        }
    }

    fn transition_payload(current: &AgentBindingValue) -> Value {
        serde_json::to_value(AgentBindingChangedPayloadV1 {
            transition_id: OperationId::from("correlation-9"),
            request_digest: DigestHex::from("c".repeat(64)),
            previous_binding_ref: AgentHandoffBindingRefV1::from(&binding(1, "source")),
            next_binding_ref: AgentHandoffBindingRefV1::from(current),
            previous_agent_label: "Source".to_owned(),
            next_agent_label: "Target".to_owned(),
            handoff_mode: AgentHandoffMode::ContextOnly,
            handoff_payload_id: None,
            handoff_payload_digest: None,
            completion_gate_inherited: false,
            effective_after_seq: 8,
        })
        .unwrap()
    }

    #[test]
    fn native_replay_starts_after_the_latest_legal_agent_transition() {
        let events = vec![
            event(4, "context/cleared"),
            event(8, "turn/completed"),
            event(9, "session/agent-binding-changed"),
            event(12, "turn/completed"),
        ];
        let current = binding(2, "target");
        let payloads = BTreeMap::from([("event-9".to_owned(), transition_payload(&current))]);
        assert_eq!(native_replay_floor(&events, &payloads, &current).unwrap(), 9);
    }

    #[test]
    fn model_only_history_keeps_the_existing_context_floor_without_a_transition() {
        let events = vec![event(4, "context/cleared"), event(8, "turn/completed")];
        assert_eq!(
            native_replay_floor(&events, &BTreeMap::new(), &binding(1, "source")).unwrap(),
            4
        );
    }

    #[test]
    fn unproven_agent_transition_fails_closed_instead_of_segmenting_history() {
        let events = vec![event(9, "session/agent-binding-changed")];
        let current = binding(2, "target");
        let payloads = BTreeMap::from([(
            "event-9".to_owned(),
            transition_payload(&binding(2, "different-target")),
        )]);
        assert!(native_replay_floor(&events, &payloads, &current).is_err());
    }

    #[test]
    fn later_narrow_binding_versions_keep_the_proven_agent_boundary() {
        let events = vec![event(9, "session/agent-binding-changed")];
        let transition_target = binding(2, "target");
        let current_model_variant = binding(3, "target-model-variant");
        let payloads = BTreeMap::from([(
            "event-9".to_owned(),
            transition_payload(&transition_target),
        )]);
        assert_eq!(
            native_replay_floor(&events, &payloads, &current_model_variant).unwrap(),
            9
        );
    }
}
