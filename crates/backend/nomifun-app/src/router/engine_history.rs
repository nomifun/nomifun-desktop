//! Bounded Runtime history reconstructed from canonical AgentSession facts.

use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    AgentBindingChangedPayloadV1, AgentBindingValue, AgentHandoffBindingRefV1, AgentSessionId,
    OperationId, SessionEventRecord,
};
use nomifun_agent_session::{AgentSessionStore, ChatCausalityFacts, canonical_context_messages};
pub use nomifun_agent_session::{CanonicalContextMessage, CanonicalContextRole};
use nomifun_common::AppError;
use serde_json::Value;

use super::engine_session_host::EngineTurnReceipt;

pub struct EngineHistoryRecord {
    pub sequence: i64,
    pub event_json: String,
    pub model_operation_id: Option<String>,
    pub model_claimed: bool,
}

pub struct EngineHistoryTurn {
    pub source_seq: u64,
    pub unstarted_terminal: bool,
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
    pub context_messages: Vec<CanonicalContextMessage>,
    pub has_older: bool,
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine history: {message}"))
}

fn require_prior_cursor(cursor_seq: u64, accepted_root_seq: u64) -> Result<(), AppError> {
    if cursor_seq >= accepted_root_seq {
        return Err(failure("historical cursor is not before the fixed accepted root"));
    }
    Ok(())
}

fn addressed_turn_eligible(event: &SessionEventRecord, fixed_root: u64, floor: u64, operation: &str) -> bool {
    event.kind.0 == "turn/started" && event.seq > floor && event.seq < fixed_root
        && event.correlation_id.as_ref() == operation
}

fn addressed_terminal_eligible(terminal: &SessionEventRecord, started: u64, fixed_root: u64) -> bool {
    matches!(terminal.kind.0.as_str(), "turn/completed" | "turn/failed" | "turn/cancelled")
        && terminal.seq > started && terminal.seq < fixed_root
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
    Ok(context_floor.max(canonical_agent_transition_floor(events, event_payloads, current)?))
}

/// The canonical Agent transition bounds execution-owned state independently
/// of conversational context clearing. Other native consumers share this
/// exact transition validation rather than interpreting a second boundary.
pub(super) fn canonical_agent_transition_floor(
    events: &[SessionEventRecord],
    event_payloads: &BTreeMap<String, Value>,
    current: &AgentBindingValue,
) -> Result<u64, AppError> {
    let transition = events
        .iter()
        .filter(|event| event.kind.0 == "session/agent-binding-changed")
        .max_by_key(|event| event.seq);
    let Some(transition) = transition else {
        return Ok(0);
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
    Ok(transition.seq)
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
    load_selected(store, receipt, limit, before_operation, None).await
}

/// Exact addressed data-only history uses the same owner facts, fixed accepted
/// root, clear-context/Agent-transition floor and complete journal decoder as
/// the ordinary reader. It selects one eligible CLOSED Turn, not a later cursor.
pub(super) async fn load_exact(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    operation: &str,
) -> Result<EngineHistoryWindow, AppError> {
    if operation.is_empty() || operation.len() > 256 || operation.chars().any(char::is_control) {
        return Err(failure("invalid exact historical Turn address"));
    }
    load_selected(store, receipt, 1, None, Some(operation)).await
}

async fn load_selected(
    store: &AgentSessionStore,
    receipt: &EngineTurnReceipt,
    limit: usize,
    before_operation: Option<&str>,
    exact_operation: Option<&str>,
) -> Result<EngineHistoryWindow, AppError> {
    if !(1..=32).contains(&limit) {
        return Err(failure("turn limit must be 1..32"));
    }
    let facts = facts(store, receipt).await?;
    // Native Runtime replay is binding-specific. A canonical Agent transition
    // is the only boundary that turns older Agent transcripts into data-only
    // context. An unmarked mismatch fails the Runtime binding check.
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
    if before_operation.is_some() {require_prior_cursor(before_seq,current_root.seq)?;}
    let mut turns = facts
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "turn/started" && event.seq < before_seq && event.seq > floor
                && exact_operation.is_none_or(|operation| addressed_turn_eligible(event,current_root.seq,floor,operation))
        })
        .collect::<Vec<_>>();
    turns.sort_by_key(|event| std::cmp::Reverse(event.seq));
    if exact_operation.is_some() && turns.len() > 1 {
        return Err(failure("exact historical Turn address is ambiguous"));
    }
    let mut window = EngineHistoryWindow {
        has_older: turns.len() > limit,
        turns: Vec::new(),
        context_messages: Vec::new(),
    };
    let mut total = 0usize;
    // Ordinary turns keep the previous small candidate window. A larger
    // approved turn may be read without increasing every normal Session's
    // history/compaction work to the absolute recovery ceiling.
    let mut window_budget = 32 * 1024 * 1024usize;
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
        if exact_operation.is_some() && !terminal.is_some_and(|terminal|
            addressed_terminal_eligible(terminal,turn.seq,current_root.seq))
        {
            return Err(failure("exact historical Turn is not canonically closed before the accepted root"));
        }
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
                .ok_or_else(|| failure("journal record has no producer sequence"))?;
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
        if window.turns.is_empty() && serialized_bytes > window_budget {
            let authorized = facts.events.iter().filter(|event| event.kind.0 == "turn/resume-authorized" && event.correlation_id == turn.correlation_id)
                .filter_map(|event| facts.event_payloads.get(event.event_id.as_ref()))
                .filter_map(|value| value.pointer("/budget/journal_bytes").and_then(Value::as_u64)).max().unwrap_or(16 * 1024 * 1024);
            if serialized_bytes as u64 > authorized.saturating_add(8 * 1024 * 1024) { return Err(failure("large historical turn has no matching storage allowance")); }
            window_budget = serialized_bytes.saturating_add(8 * 1024 * 1024).min(nomifun_agent_contracts::MAX_NATIVE_HISTORY_WINDOW_BYTES);
        }
        if records.len() as u64 > nomifun_agent_contracts::MAX_NATIVE_APPROVED_JOURNAL_RECORDS + 4000
            || serialized_bytes > nomifun_agent_contracts::MAX_NATIVE_APPROVED_REPLAY_BYTES
            || total.saturating_add(serialized_bytes) > window_budget
        {
            if window.turns.is_empty() {
                return Err(failure("latest turn exceeds the history budget"));
            }
            window.has_older = true;
            break;
        }
        total = total.saturating_add(serialized_bytes);
        let root_content_json = serde_json::to_string(root_payload).map_err(failure)?;
        let unstarted_terminal = records.is_empty()
            && terminal.is_some_and(|terminal| {
                terminal.kind.0 == "turn/cancelled"
                    || (terminal.kind.0 == "turn/failed"
                        && facts.event_payloads.get(terminal.event_id.as_ref())
                            .and_then(|payload| payload.get("code")).and_then(Value::as_str)
                            == Some("runtime_dispatch_failed"))
            })
            && !facts.events.iter().any(|event| event.seq > turn.seq
                && terminal.is_some_and(|terminal| event.seq < terminal.seq)
                && (event.kind.0 == "context/model-visible-applied"
                    || event.kind.0.starts_with("tool/") || event.kind.0.starts_with("effect/")));
        window.turns.push(EngineHistoryTurn {
            source_seq: root.seq,
            unstarted_terminal,
            operation_id,
            root_message_id: root.event_id.as_ref().to_owned(),
            receipt_status,
            request_payload_json: root_content_json.clone(),
            root_content_json,
            records,
            serialized_bytes,
        });
    }
    if before_operation.is_none() && exact_operation.is_none() {
        let context_floor = facts.events.iter().filter(|event| event.kind.0 == "context/cleared")
            .map(|event| event.seq).max().unwrap_or(0);
        let retained_floor = if window.has_older {
            window.turns.last().map_or(context_floor, |turn| turn.source_seq.saturating_sub(1))
        } else { context_floor };
        window.context_messages = canonical_context_messages(
            &facts.events, &facts.event_payloads, retained_floor, floor, current_root.seq,
        ).map_err(failure)?;
        if context_floor == 0 && !window.has_older {
            if let Some(base) = &facts.fork_context {
                let mut inherited = base.base_context(context_floor).map_err(failure)?;
                // A self-contained base is historical data before this child's
                // first event, never a new accepted input or source authority.
                inherited.append(&mut window.context_messages);
                window.context_messages = inherited;
            }
        }
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

    #[test]
    fn supplied_history_cursors_cannot_cross_the_fixed_accepted_root() {
        assert!(require_prior_cursor(9,10).is_ok());
        assert!(require_prior_cursor(10,10).is_err());
        assert!(require_prior_cursor(11,10).is_err());
        assert!(require_prior_cursor(u64::MAX,10).is_err());
    }

    #[test]
    fn addressed_history_selects_old_exact_target_without_crossing_fixed_root_or_reset_floor() {
        let mut turns = (1..=12).map(|seq| event(seq,"turn/started")).collect::<Vec<_>>();
        for turn in &mut turns {turn.correlation_id = format!("turn-{seq}",seq=turn.seq).into();}
        let selected = turns.iter().filter(|turn| addressed_turn_eligible(turn,13,0,"turn-1")).collect::<Vec<_>>();
        assert_eq!(selected.len(),1);assert_eq!(selected[0].seq,1);
        assert!(!addressed_turn_eligible(selected[0],13,1,"turn-1"),"clear/Agent-transition floor is still exclusive");
        assert!(!addressed_turn_eligible(&turns[11],12,0,"turn-12"),"current accepted root cannot become history");
        assert!(!addressed_turn_eligible(&turns[11],11,0,"turn-12"),"future Turn cannot become history");
        assert!(!addressed_turn_eligible(&turns[0],13,0,"foreign-turn"));
        assert!(addressed_terminal_eligible(&event(2,"turn/failed"),1,13));
        assert!(!addressed_terminal_eligible(&event(1,"turn/completed"),1,13));
        assert!(!addressed_terminal_eligible(&event(13,"turn/completed"),1,13));
        assert!(!addressed_terminal_eligible(&event(2,"turn/paused"),1,13));
    }

    fn event(seq: u64, kind: &str) -> SessionEventRecord {
        SessionEventRecord {
            agent_session_id: AgentSessionId::from(
                "0190f5fe-7c00-7a00-8000-000000000001",
            ),
            seq,
            event_id: EventId::from(format!("event-{seq}")),
            producer_id: EventProducerId::from("test"),
            idempotency_key: IdempotencyKey::from(format!("key-{seq}")),
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

    fn context_event(seq: u64, kind: &str, correlation: &str, value: Value) -> (SessionEventRecord, Value) {
        let mut event = event(seq, kind);
        event.correlation_id = correlation.into();
        (event, value)
    }

    fn context_rows(rows: Vec<(SessionEventRecord, Value)>, floor: u64, native_floor: u64, before: u64)
        -> Result<Vec<CanonicalContextMessage>, AppError> {
        let payloads = rows.iter().map(|(event, value)| (event.event_id.as_ref().to_owned(), value.clone())).collect();
        canonical_context_messages(&rows.into_iter().map(|(event, _)| event).collect::<Vec<_>>(),
            &payloads, floor, native_floor, before).map_err(failure)
    }

    #[test]
    fn canonical_domain_context_keeps_creation_prompts_and_execution_notices_with_typed_roles() {
        let rows = context_rows(vec![
            context_event(3, "message/user-accepted", "creation", serde_json::json!({
                "content":"creation prompt", "position":"left", "state":"completed"})),
            context_event(4, "message/assistant-projected", "notice", serde_json::json!({
                "content":"execution summary", "position":"right", "state":"accepted"})),
            context_event(5, "message/assistant-projected", "cron", serde_json::json!({
                "content":"Cron notice", "notice_kind":"cron"})),
        ], 0, 0, 6).unwrap();
        assert_eq!(rows.iter().map(|row| (&row.role, row.content.as_str())).collect::<Vec<_>>(), vec![
            (&CanonicalContextRole::User, "creation prompt"),
            (&CanonicalContextRole::Assistant, "execution summary"),
            (&CanonicalContextRole::Assistant, "Cron notice"),
        ]);
    }

    #[test]
    fn native_turn_messages_cannot_replace_a_missing_runtime_journal() {
        let rows = context_rows(vec![
            context_event(3, "message/user-accepted", "root", serde_json::json!({"content":"native request"})),
            context_event(4, "turn/started", "operation", serde_json::json!({"source_message_id":"event-3"})),
            context_event(5, "message/content-part", "answer", serde_json::json!({"content":"apparent success"})),
            context_event(6, "message/completed", "answer", serde_json::json!({"part_count":1})),
        ], 0, 0, 7).unwrap();
        assert!(rows.is_empty(), "native history must come from its structured Runtime records");
    }

    #[test]
    fn agent_transition_keeps_only_digest_verified_canonical_transcript_as_data_context() {
        let digest = nomifun_agent_contracts::digest_bytes(b"previous answer");
        let rows = context_rows(vec![
            context_event(3, "message/user-accepted", "source", serde_json::json!({"content":"previous request"})),
            context_event(4, "message/content-part", "answer", serde_json::json!({"content":"previous "})),
            context_event(5, "message/content-part", "answer", serde_json::json!({"content":"answer"})),
            context_event(6, "message/completed", "answer", serde_json::json!({"part_count":2,"content_digest":digest})),
            context_event(10, "message/user-accepted", "current", serde_json::json!({"content":"new request"})),
            context_event(11, "turn/started", "current-op", serde_json::json!({"source_message_id":"event-10"})),
            context_event(12, "message/assistant-projected", "summary", serde_json::json!({"content":"typed summary"})),
        ], 0, 8, 13).unwrap();
        assert_eq!(rows.iter().map(|row| row.content.as_str()).collect::<Vec<_>>(),
            ["previous request", "previous answer", "typed summary"]);
        assert!(context_rows(vec![
            context_event(4, "message/content-part", "answer", serde_json::json!({"content":"changed"})),
            context_event(6, "message/completed", "answer", serde_json::json!({"part_count":1,"content_digest":"a".repeat(64)})),
        ], 0, 8, 13).is_err(), "a projection cannot hide a mismatched canonical completion");
    }

    #[test]
    fn canonical_context_respects_clear_floor_and_fixed_accepted_root() {
        let rows = context_rows(vec![
            context_event(3, "message/user-accepted", "old", serde_json::json!({"content":"cleared"})),
            context_event(5, "message/assistant-projected", "notice", serde_json::json!({"content":"retained"})),
            context_event(6, "message/user-accepted", "root", serde_json::json!({"content":"current"})),
            context_event(7, "message/assistant-projected", "future", serde_json::json!({"content":"future"})),
        ], 4, 4, 6).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "retained");
    }

    #[test]
    fn agent_transition_prefix_survives_subsequent_native_turns_without_copying_their_messages() {
        let mut rows = vec![
            context_event(3, "message/user-accepted", "old-root", serde_json::json!({"content":"previous Agent request"})),
            context_event(4, "message/content-part", "old-answer", serde_json::json!({"content":"previous Agent answer"})),
            context_event(5, "message/completed", "old-answer", serde_json::json!({"part_count":1,
                "content_digest":nomifun_agent_contracts::digest_bytes(b"previous Agent answer")})),
        ];
        let prefix = context_rows(rows.clone(), 0, 8, 9).unwrap();
        for native_turn in 0..3 {
            let seq = 10 + native_turn * 4;
            rows.push(context_event(seq, "message/user-accepted", "native-root", serde_json::json!({"content":"native request"})));
            rows.push(context_event(seq + 1, "turn/started", "native-op", serde_json::json!({"source_message_id":format!("event-{seq}")})));
            rows.push(context_event(seq + 2, "message/content-part", "native-answer", serde_json::json!({"content":"native response"})));
            assert_eq!(context_rows(rows.clone(), 0, 8, seq + 3).unwrap(), prefix,
                "reconstruct from canonical events on every Turn, without importing current-binding projections");
        }
    }
}
