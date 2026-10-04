//! Closed Runtime replay and typed context from canonical AgentSession events.
use std::collections::BTreeMap;

use nomifun_agent_contracts::MAX_NATIVE_HISTORY_WINDOW_BYTES;
use nomifun_agent_runtime::{AgentEngineEvent, AgentPriorTask, EngineBinding, replay_closed_history};
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};
use nomifun_common::AppError;

/// The canonical terminal can outlive a crashed Runtime that never wrote its
/// private terminal record. Reconstruct only interruption, NEVER success or
/// cleanup proof. The durable canonical receipt remains the authority.
pub(super) fn project_interrupted_terminal(events: &mut Vec<AgentEngineEvent>, receipt_status: &str) {
    if matches!(events.last(), Some(AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnFailed { .. } | AgentEngineEvent::TurnCancelled { .. })) {
        return;
    }
    match receipt_status {
        "failed" | "interrupted" => events.push(AgentEngineEvent::TurnFailed {
            model_steps: 0,
            message: "Canonical owner ended interrupted execution. Saved tool observations are historical data; uncertain outcomes require reconciliation, not replay.".into(),
        }),
        "cancelled" => events.push(AgentEngineEvent::TurnCancelled { model_steps: 0 }),
        _ => {}
    }
}

pub(super) struct AgentHistory {
    pub messages: Vec<ChatMessage>,
    pub prior_task: Option<AgentPriorTask>,
}

pub(super) async fn load(
    window: super::engine_history::EngineHistoryWindow,
    session_host: &super::engine_session_host::EngineSessionHost,
    admitted: &super::engine_session_host::EngineTurnReceipt,
    context_byte_limit: usize,
) -> Result<AgentHistory, AppError> {
    let conversation = admitted.session().session().conversation_id.as_str();
    let snapshot = &admitted.session().snapshot().snapshot_ref;
    let fail = |message: String| AppError::Conflict(format!("Nomi history: {message}"));
    let replay_budget = window.turns.first().map_or(0,|turn|turn.serialized_bytes.saturating_add(8 * 1024 * 1024))
        .max(32 * 1024 * 1024).min(MAX_NATIVE_HISTORY_WINDOW_BYTES);
    let mut closed_turns = Vec::new();
    let mut bytes = 0usize;
    let mut prior_task = None;
    let mut context = bounded_context_messages(window.context_messages, context_byte_limit)?.into_iter().peekable();
    let mut historical_compatibility = BTreeMap::new();
    for turn in window.turns {
        let root: serde_json::Value = serde_json::from_str(&turn.root_content_json)
            .map_err(|error| fail(error.to_string()))?;
        let text = root
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| fail("accepted root has no text".into()))?;
        if bytes.saturating_add(turn.serialized_bytes) > replay_budget {
            if closed_turns.is_empty() {
                return Err(fail("latest turn exceeds replay byte budget".into()));
            }
            break;
        }
        bytes = bytes.saturating_add(turn.serialized_bytes);
        let receipt = turn.request_payload_json;
        let events = decode_turn_events(turn.records, &turn.receipt_status, turn.unstarted_terminal)?;
        if let Some(events) = &events {
        let Some(AgentEngineEvent::TurnStarted {
            binding: recorded,
            turn_operation_id: recorded_operation,
        }) = events.first()
        else {
            return Err(fail("Runtime history has no exact TurnStarted record".into()));
        };
        let recorded_snapshot = recorded.resolved_snapshot_ref();
        let snapshot_compatible = if recorded_snapshot == snapshot {
            true
        } else if let Some(compatible) = historical_compatibility.get(recorded_snapshot) {
            *compatible
        } else {
            let compatible = session_host
                .historical_model_binding_compatible(admitted.session(), recorded_snapshot)
                .await?;
            historical_compatibility.insert(recorded_snapshot.clone(), compatible);
            compatible
        };
        if !history_source_matches(
            recorded,
            recorded_operation,
            conversation,
            &turn.operation_id,
            snapshot_compatible,
        ) {
            return Err(fail(
                "history differs from the exact Session binding".into(),
            ));
        }
        }
        let receipt: serde_json::Value =
            serde_json::from_str(&receipt).map_err(|error| fail(error.to_string()))?;
        let files = super::runtime_attachments::references(&receipt)?;
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(ChatContentPart::Text { text: text.into() });
        }
        if let Some(description) = super::runtime_attachments::description(&files, true) {
            content.push(description);
        }
        let unresolved = if let Some(events) = &events { unresolved_steering(events).await? } else { Vec::new() };
        let extra_bytes = unresolved
            .iter()
            .map(|message| serde_json::to_vec(message).map(|raw| raw.len()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| fail(error.to_string()))?
            .into_iter()
            .sum::<usize>();
        if bytes.saturating_add(extra_bytes) > replay_budget {
            if closed_turns.is_empty() {
                return Err(fail("latest turn steering exceeds replay budget".into()));
            }
            break;
        }
        bytes = bytes.saturating_add(extra_bytes);
        if closed_turns.is_empty() {
            // Latest only. An unstarted failure cannot revive an older task.
            prior_task = events.as_ref().map(|events| AgentPriorTask::from_closed_turn(&turn.operation_id, events))
                .transpose().map_err(|error| fail(error.to_string()))?.flatten();
        }
        closed_turns.push((
            turn.source_seq,
            ChatMessage {
                role: ChatRole::User,
                content,
                provider_round_id: None,
            },
            events,
            unresolved,
            turn.receipt_status,
        ));
    }
    let mut history = Vec::new();
    let mut replay_turns: Vec<(ChatMessage, Vec<AgentEngineEvent>, Vec<ChatMessage>)> = Vec::new();
    for (source_seq, requirement, events, unresolved, status) in closed_turns.into_iter().rev() {
        while context.peek().is_some_and(|(seq, _)| *seq < source_seq) {
            append_context(&mut history, &mut replay_turns, context.next().expect("peeked context").1);
        }
        if let Some(events) = events {
            replay_turns.push((requirement, events, unresolved));
        } else {
            // The canonical owner may reject/cancel admission before Runtime
            // starts. Keep the accepted input and that terminal fact as data;
            // neither a model response nor tool execution is reconstructed.
            append_context(&mut history, &mut replay_turns, requirement);
            append_context(&mut history, &mut replay_turns, ChatMessage {
                role: ChatRole::Assistant,
                content: vec![ChatContentPart::Text { text: format!(
                    "Historical request was canonically {status} before Runtime execution started. No Runtime result or tool execution is recorded.") }],
                provider_round_id: None,
            });
        }
    }
    for (_, message) in context { append_context(&mut history, &mut replay_turns, message); }
    // Preserve the Runtime's full-window receipt retention across compaction.
    replay_closed_history(&mut history, replay_turns).map_err(|error| fail(error.to_string()))?;
    Ok(AgentHistory {
        messages: history,
        prior_task,
    })
}

fn append_context(
    prefix: &mut Vec<ChatMessage>,
    turns: &mut [(ChatMessage, Vec<AgentEngineEvent>, Vec<ChatMessage>)],
    message: ChatMessage,
) {
    if let Some((_, _, following)) = turns.last_mut() { following.push(message); }
    else { prefix.push(message); }
}

pub(super) fn decode_turn_events(
    records: Vec<super::engine_history::EngineHistoryRecord>,
    receipt_status: &str,
    unstarted_terminal: bool,
) -> Result<Option<Vec<AgentEngineEvent>>, AppError> {
    let fail = |message: String| AppError::Conflict(format!("Nomi history: {message}"));
    let mut events = Vec::new();
    for record in records {
        let value: serde_json::Value = serde_json::from_str(&record.event_json)
            .map_err(|error| fail(error.to_string()))?;
        if matches!(value.get("event").and_then(serde_json::Value::as_str),
            Some("host_tool_dispatch" | "host_tool_settled" | "host_resource_dispatch"
                | "host_resource_settled" | "host_process_dispatch" | "host_process_quiescent" | "host_cleanup_proven")) {
            continue;
        }
        events.push(serde_json::from_value::<AgentEngineEvent>(value)
            .map_err(|error| fail(error.to_string()))?);
    }
    if events.is_empty() && unstarted_terminal && matches!(receipt_status, "failed" | "cancelled") {
        return Ok(None);
    }
    if !matches!(events.first(), Some(AgentEngineEvent::TurnStarted { .. })) {
        return Err(fail("Runtime history has no exact TurnStarted record".into()));
    }
    project_interrupted_terminal(&mut events, receipt_status);
    Ok(Some(events))
}

fn history_source_matches(
    recorded: &EngineBinding,
    recorded_operation: &nomifun_agent_contracts::OperationId,
    conversation: &str,
    expected_operation: &str,
    snapshot_compatible: bool,
) -> bool {
    // Closed history is versioned, parsed data rather than an executable
    // checkpoint. App updates may change the engine build while retaining the
    // same Session and a model-compatible Snapshot. Keep executable recovery
    // build-bound, but allow this read-only replay to cross that build boundary.
    recorded.validate().is_ok()
        && recorded.agent_session_id().as_ref() == conversation
        && recorded_operation.as_ref() == expected_operation
        && snapshot_compatible
}

/// Select a bounded suffix of typed event context. Roles come from event kinds,
/// never a UI side/position or projection state.
fn bounded_context_messages(
    rows: Vec<super::engine_history::CanonicalContextMessage>,
    byte_limit: usize,
) -> Result<Vec<(u64, ChatMessage)>, AppError> {
    let mut messages = Vec::new();
    let mut bytes = 0usize;
    for row in rows.into_iter().rev().take(4096) {
        let message = ChatMessage {
            role: match row.role {
                super::engine_history::CanonicalContextRole::User => ChatRole::User,
                super::engine_history::CanonicalContextRole::Assistant => ChatRole::Assistant,
            },
            content: vec![ChatContentPart::Text { text: row.content }],
            provider_round_id: None,
        };
        let size = serde_json::to_vec(&message)
            .map_err(|error| AppError::Conflict(error.to_string()))?
            .len();
        if bytes.saturating_add(size) > byte_limit {
            break;
        }
        bytes += size;
        messages.push((row.seq, message));
    }
    messages.reverse();
    Ok(messages)
}

/// After an unexpected exit the inbox can be gone even when the permanent
/// receipt says delivery was queued. Preserve that uncertainty as historical
/// data; never enqueue it into a replacement turn or invent execution evidence.
async fn unresolved_steering(
    events: &[AgentEngineEvent],
) -> Result<Vec<ChatMessage>, AppError> {
    let scopes = events
        .iter()
        .filter_map(|event| match event {
            AgentEngineEvent::TurnInputScope { wire_turn_id } => Some(wire_turn_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [wire] = scopes.as_slice() else {
        return if scopes.is_empty() {
            Ok(Vec::new())
        } else {
            Err(AppError::Conflict("duplicate Nomi input scope".into()))
        };
    };
    // Canonical steering admission is represented by `turn/steer-accepted`
    // facts and the Runtime records only durable delivery/deferral decisions.
    // An absent Runtime control record is deliberately not converted into a
    // prompt: replaying an ambiguous steer would duplicate user intent.
    let _ = wire;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        AgentSessionId, DigestHex, OperationId, ResolvedSnapshotId, ResolvedSnapshotRef,
        RuntimeBindingId,
    };
    use nomifun_agent_runtime::{EngineBinding, EngineBuildId};

    fn record(event: AgentEngineEvent) -> super::super::engine_history::EngineHistoryRecord {
        super::super::engine_history::EngineHistoryRecord {
            sequence: 1, event_json: serde_json::to_string(&event).unwrap(),
            model_operation_id: None, model_claimed: false,
        }
    }

    #[test]
    fn missing_runtime_journal_only_allows_an_explicit_unstarted_failure_or_cancellation() {
        assert!(decode_turn_events(Vec::new(), "failed", true).unwrap().is_none());
        assert!(decode_turn_events(Vec::new(), "cancelled", true).unwrap().is_none());
        assert!(decode_turn_events(Vec::new(), "failed", false).is_err(), "a failure without explicit pre-execution evidence is incomplete history");
        for status in ["completed", "running", "paused", "interrupted"] {
            assert!(decode_turn_events(Vec::new(), status, true).is_err(), "{status} cannot use message projections");
        }
        for status in ["failed", "completed"] {
            assert!(decode_turn_events(vec![record(AgentEngineEvent::OutputTextDelta {
                step: 1, text: "unproven output".into(),
            })], status, false).is_err(), "partial Runtime records require their own exact TurnStarted");
        }
    }

    #[test]
    fn canonical_context_budget_keeps_a_contiguous_suffix_and_explicit_roles() {
        use super::super::engine_history::{CanonicalContextMessage, CanonicalContextRole};
        let rows = vec![
            CanonicalContextMessage { seq: 1, role: CanonicalContextRole::User, content: "old".into() },
            CanonicalContextMessage { seq: 2, role: CanonicalContextRole::Assistant, content: "a".repeat(1024) },
            CanonicalContextMessage { seq: 3, role: CanonicalContextRole::User, content: "new".into() },
        ];
        let actual = bounded_context_messages(rows, 200).unwrap();
        assert_eq!(actual.len(), 1, "an oversized middle row must not be skipped to import older context");
        assert_eq!(actual[0].0, 3);
        assert_eq!(actual[0].1.role, ChatRole::User);
    }

    #[test]
    fn domain_context_stays_between_closed_turns_instead_of_becoming_runtime_input() {
        let message = |text: &str| ChatMessage { role: ChatRole::Assistant,
            content: vec![ChatContentPart::Text { text: text.into() }], provider_round_id: None };
        let mut prefix = Vec::new();
        let mut turns = Vec::new();
        append_context(&mut prefix, &mut turns, message("creation context"));
        turns.push((message("native root"), Vec::new(), Vec::new()));
        append_context(&mut prefix, &mut turns, message("Execution summary"));
        assert_eq!(prefix.len(), 1);
        assert_eq!(turns[0].2, vec![message("Execution summary")]);
    }

    #[test]
    fn versioned_closed_history_crosses_builds_but_not_session_turn_or_snapshot() {
        let snapshot = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("a".repeat(64)),
        };
        let recorded = EngineBinding::new(
            AgentSessionId::from("session"),
            RuntimeBindingId::from("old-runtime"),
            EngineBuildId::from("old-build"),
            DigestHex::from("b".repeat(64)),
            snapshot,
        )
        .unwrap();
        let operation = OperationId::from("old-turn");

        assert!(history_source_matches(
            &recorded,
            &operation,
            "session",
            "old-turn",
            true,
        ));
        assert!(!history_source_matches(
            &recorded,
            &operation,
            "other-session",
            "old-turn",
            true,
        ));
        assert!(!history_source_matches(
            &recorded,
            &OperationId::from("other-turn"),
            "session",
            "old-turn",
            true,
        ));
        assert!(!history_source_matches(
            &recorded,
            &operation,
            "session",
            "old-turn",
            false,
        ));
    }
}
