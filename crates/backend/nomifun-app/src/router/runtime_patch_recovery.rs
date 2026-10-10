//! Permanent Nomi patch-recovery obligations reconstructed from canonical
//! AgentSession events. No old Conversation receipt/journal table is read.

use super::engine_session_host::{EngineSessionHost, EngineTurnReceipt};
use nomifun_agent_contracts::{AgentBindingValue, AgentSessionId, OperationId, ResolvedSnapshotRef, SessionEventRecord};
use nomifun_agent_runtime::{AgentEngineEvent, AgentPatchRecoveryState};
use nomifun_common::AppError;
use serde_json::Value;
use std::collections::BTreeMap;

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi patch recovery: {message}"))
}

pub(super) async fn load(
    host: &EngineSessionHost,
    receipt: &EngineTurnReceipt,
    snapshot: &ResolvedSnapshotRef,
) -> Result<AgentPatchRecoveryState, AppError> {
    let session = AgentSessionId::from(receipt.session().session().conversation_id.clone());
    let current = OperationId::from(receipt.operation_id().to_owned());
    let store = host.canonical_store()?;
    let facts = store
        .chat_causality_facts(&session, &current)
        .await
        .map_err(failure)?;
    if facts.head.status != "running"
        || facts.head.active_turn_id.as_deref() != Some(current.as_ref())
        || facts.session.agent_binding != *receipt.session().agent_binding()
        || facts.session.agent_binding.resolved_snapshot_ref != *snapshot
    {
        return Err(failure("current turn authority changed"));
    }

    let progress = current_binding_progress(
        &facts.events, &facts.event_payloads, &facts.session.agent_binding, &current,
    )?;

    let mut latest_state: Option<(u64, String, AgentPatchRecoveryState)> = None;
    let mut latest_patch_dispatch = 0_u64;
    for (event, value) in progress {
        if value.get("event").and_then(Value::as_str) == Some("host_tool_dispatch")
            && value
                .get("dispatch")
                .and_then(|dispatch| dispatch.get("action_id"))
                .and_then(Value::as_str)
                == Some("workspace.files/patch")
        {
            latest_patch_dispatch = event.seq;
        }
        if let Some(state) = recovery_update(value)? {
            latest_state = Some((
                event.seq,
                event.correlation_id.as_ref().to_owned(),
                state,
            ));
        }
    }

    let Some((state_seq, source_operation, state)) = latest_state else {
        if latest_patch_dispatch != 0 {
            return Err(failure(
                "prior patch dispatch has no permanent recovery state",
            ));
        }
        return Ok(AgentPatchRecoveryState::default());
    };
    let source_terminal = facts.events.iter().find(|event| {
        event.correlation_id.as_ref() == source_operation
            && patch_recovery_source_terminal(event.kind.0.as_str())
    });
    let Some(source_terminal) = source_terminal else {
        return Err(failure("recovery source has no canonical terminal Turn"));
    };
    let source_start = facts.events.iter().find_map(|event| {
        if event.kind.0 != "runtime/progress-recorded"
            || event.correlation_id.as_ref() != source_operation
        {
            return None;
        }
        let payload = facts.event_payloads.get(event.event_id.as_ref())?.get("event")?;
        serde_json::from_value::<AgentEngineEvent>(payload.clone())
            .ok()
            .and_then(|engine_event| match engine_event {
                AgentEngineEvent::TurnStarted {
                    binding,
                    turn_operation_id,
                } => Some((binding, turn_operation_id, event.seq)),
                _ => None,
            })
    });
    let Some((recorded, turn_operation_id, source_start_seq)) = source_start else {
        return Err(failure("recovery source has no engine binding"));
    };
    if source_start_seq >= state_seq || source_terminal.seq <= state_seq {
        return Err(failure("recovery state is outside its source Turn boundary"));
    }
    if latest_patch_dispatch > state_seq && !state.has_pending() {
        return Err(failure(
            "a later patch dispatch is not covered by recovery state",
        ));
    }
    if !patch_recovery_source_matches(
        &recorded,
        &turn_operation_id,
        &session,
        source_operation.as_str(),
        snapshot,
    ) {
        let exact_source = patch_recovery_source_matches(
            &recorded, &turn_operation_id, &session, source_operation.as_str(),
            recorded.resolved_snapshot_ref(),
        );
        // A fully settled empty state grants no recovery authority. Only the
        // trusted host may prove a model-only transition; pending mutations
        // keep the original exact-Snapshot requirement unchanged.
        if exact_source && state == AgentPatchRecoveryState::default()
            && settled_model_transition(&state, exact_source,
                host.historical_model_binding_compatible(receipt.session(), recorded.resolved_snapshot_ref()).await?)
        {
            return Ok(AgentPatchRecoveryState::default());
        }
        return Err(AppError::SessionConfigurationChanged(
            "Pending or incompatible patch recovery is bound to the previous Agent configuration; resolve it before continuing with another model configuration".into(),
        ));
    }
    Ok(state)
}

/// A binding transition ends the old executable recovery segment only after
/// its permanent obligations are proven settled. Context clearing is not such
/// authority. All canonical history stays intact and readable as data.
fn current_binding_progress<'a>(
    events: &'a [SessionEventRecord],
    payloads: &'a BTreeMap<String, Value>,
    current_binding: &AgentBindingValue,
    current_operation: &OperationId,
) -> Result<Vec<(&'a SessionEventRecord, &'a Value)>, AppError> {
    let floor = super::engine_history::canonical_agent_transition_floor(
        events, payloads, current_binding,
    )?;
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|event| event.seq);
    let mut selected = Vec::new();
    let mut state: Option<(u64, AgentPatchRecoveryState)> = None;
    let mut dispatched = 0;
    for event in ordered {
        if event.kind.0 == "session/agent-binding-changed" {
            super::engine_history::canonical_agent_transition_floor(
                std::slice::from_ref(event), payloads, current_binding,
            )?;
            let pending = state.as_ref().is_some_and(|(_, state)| state.has_pending());
            let uncovered = dispatched > state.as_ref().map_or(0, |(seq, _)| *seq);
            if pending || uncovered {
                return Err(failure("Agent transition crosses pending or uncovered patch recovery"));
            }
            state = None;
            dispatched = 0;
            continue;
        }
        if event.kind.0 != "runtime/progress-recorded"
            || event.correlation_id.as_ref() == current_operation.as_ref()
        {
            continue;
        }
        let payload = payloads.get(event.event_id.as_ref())
            .ok_or_else(|| failure("patch recovery progress has no resolved canonical payload"))?;
        let value = payload.get("event")
            .ok_or_else(|| failure("patch recovery progress has no native event"))?;
        if event.seq > floor {
            selected.push((event, value));
        } else {
            if value.get("event").and_then(Value::as_str) == Some("host_tool_dispatch")
                && value.get("dispatch").and_then(|dispatch| dispatch.get("action_id"))
                    .and_then(Value::as_str) == Some("workspace.files/patch")
            {
                dispatched = event.seq;
            }
            if let Some(updated) = recovery_update(value)? {
                state = Some((event.seq, updated));
            }
        }
    }
    Ok(selected)
}

fn recovery_update(value: &Value) -> Result<Option<AgentPatchRecoveryState>, AppError> {
    if value.get("event").and_then(Value::as_str) != Some("patch_recovery_updated") {
        return Ok(None);
    }
    let event: AgentEngineEvent = serde_json::from_value(value.clone()).map_err(failure)?;
    let AgentEngineEvent::PatchRecoveryUpdated { state } = event else {
        return Err(failure("canonical patch recovery event has another native identity"));
    };
    state.validate().map_err(failure)?;
    Ok(Some(state))
}

fn settled_model_transition(state: &AgentPatchRecoveryState, exact_source: bool, model_compatible: bool) -> bool {
    state == &AgentPatchRecoveryState::default() && exact_source && model_compatible
}

fn patch_recovery_source_terminal(kind: &str) -> bool {
    matches!(kind, "turn/completed" | "turn/failed" | "turn/cancelled")
}

fn patch_recovery_source_matches(
    recorded: &nomifun_agent_runtime::EngineBinding,
    turn_operation_id: &OperationId,
    session: &AgentSessionId,
    source_operation: &str,
    snapshot: &ResolvedSnapshotRef,
) -> bool {
    // This is a versioned, validated recovery obligation, not an executable
    // checkpoint. It remains portable across app/engine builds while the exact
    // Session, source Turn and immutable capability Snapshot stay identical.
    recorded.validate().is_ok()
        && recorded.agent_session_id() == session
        && turn_operation_id.as_ref() == source_operation
        && recorded.resolved_snapshot_ref() == snapshot
}

#[cfg(test)]
mod tests {
    use super::{current_binding_progress, patch_recovery_source_matches, patch_recovery_source_terminal, settled_model_transition};
    use nomifun_agent_contracts::{
        AgentBindingChangedPayloadV1, AgentBindingValue, AgentHandoffBindingRefV1, AgentHandoffMode,
        AgentPresetId, AgentSessionId, CorrelationId, DigestHex, EventProducerId,
        IdempotencyKey, OperationId, PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef,
        RuntimeBindingId, SessionEventKind, SessionEventPayloadRef, SessionEventRecord,
    };
    use nomifun_agent_runtime::{AgentEngineEvent, AgentPatchRecoveryState, EngineBinding, EngineBuildId};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    fn binding(version: u64) -> AgentBindingValue {
        AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from(format!("preset-{version}")), revision: 1,
                revision_digest: "a".repeat(64).into(),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: format!("snapshot-{version}").into(), snapshot_digest: "b".repeat(64).into(),
            },
            typed_resource_bindings: Vec::new(), binding_version: version,
        }
    }

    fn event(seq: u64, kind: &str) -> SessionEventRecord {
        SessionEventRecord {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000001".into(), seq,
            event_id: format!("event-{seq}").into(), producer_id: EventProducerId::from("test"),
            idempotency_key: IdempotencyKey::from(format!("key-{seq}")),
            kind: SessionEventKind(kind.into()), kind_version: 1,
            correlation_id: CorrelationId::from(format!("correlation-{seq}")),
            causation_event_id: None, payload: SessionEventPayloadRef::Empty,
        }
    }

    fn transition(seq: u64, target: &AgentBindingValue) -> Value {
        serde_json::to_value(AgentBindingChangedPayloadV1 {
            transition_id: format!("correlation-{seq}").into(), request_digest: "c".repeat(64).into(),
            previous_binding_ref: AgentHandoffBindingRefV1::from(&binding(target.binding_version - 1)),
            next_binding_ref: AgentHandoffBindingRefV1::from(target),
            previous_agent_label: "Agent".into(), next_agent_label: "Agent".into(),
            handoff_mode: AgentHandoffMode::ContextOnly, handoff_payload_id: None,
            handoff_payload_digest: None, completion_gate_inherited: false, effective_after_seq: seq - 1,
        }).unwrap()
    }

    fn progress(state: AgentPatchRecoveryState) -> Value {
        json!({ "event": serde_json::to_value(AgentEngineEvent::PatchRecoveryUpdated { state }).unwrap() })
    }

    #[test]
    fn settled_patch_history_does_not_grant_recovery_in_the_new_binding_segment() {
        let current = binding(2);
        let events = vec![event(1, "runtime/progress-recorded"), event(2, "runtime/progress-recorded"),
            event(3, "session/agent-binding-changed"), event(4, "runtime/progress-recorded")];
        let payloads = BTreeMap::from([
            ("event-1".into(), json!({"event":{"event":"host_tool_dispatch","dispatch":{"action_id":"workspace.files/patch"}}})),
            ("event-2".into(), progress(AgentPatchRecoveryState::default())),
            ("event-3".into(), transition(3, &current)),
            ("event-4".into(), json!({"event":{"event":"turn_started"}})),
        ]);
        let before = payloads.clone();
        let selected = current_binding_progress(&events, &payloads, &current, &"correlation-4".into()).unwrap();
        assert!(selected.is_empty(), "settled state on an old Snapshot must not reach the new Turn's exact-source check");
        assert_eq!(payloads, before, "the original observations remain immutable history");

        let mut payloads = payloads;
        payloads.insert("event-4".into(), progress(AgentPatchRecoveryState {
            targets: vec!["current.txt".into()], ..Default::default()
        }));
        let selected = current_binding_progress(&events, &payloads, &current, &"next-turn".into()).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.seq, 4, "new segment recovery remains executable only under its own exact binding");
    }

    #[test]
    fn binding_segments_cannot_hide_pending_or_uncovered_patch_obligations() {
        let current = binding(2);
        let events = vec![event(1, "runtime/progress-recorded"), event(2, "runtime/progress-recorded"),
            event(3, "session/agent-binding-changed")];
        for state in [
            AgentPatchRecoveryState { targets: vec!["pending.txt".into()], ..Default::default() },
            AgentPatchRecoveryState { unresolved_targets: vec!["pending.txt".into()], unresolved_before_input: Some(1), ..Default::default() },
            AgentPatchRecoveryState { target_budget_exceeded: true, ..Default::default() },
        ] {
            let payloads = BTreeMap::from([
                ("event-1".into(), progress(AgentPatchRecoveryState::default())),
                ("event-2".into(), progress(state)),
                ("event-3".into(), transition(3, &current)),
            ]);
            assert!(current_binding_progress(&events, &payloads, &current, &"next-turn".into()).is_err());
        }
        let payloads = BTreeMap::from([
            ("event-1".into(), progress(AgentPatchRecoveryState::default())),
            ("event-2".into(), json!({"event":{"event":"host_tool_dispatch","dispatch":{"action_id":"workspace.files/patch"}}})),
            ("event-3".into(), transition(3, &current)),
        ]);
        assert!(current_binding_progress(&events, &payloads, &current, &"next-turn".into()).is_err());
        let mut malformed = payloads;
        malformed.insert("event-2".into(), json!({"event":{"event":"patch_recovery_updated","state":{"version":2}}}));
        assert!(current_binding_progress(&events, &malformed, &current, &"next-turn".into()).is_err(),
            "an undecodable obligation cannot be treated as absent when entering a new segment");
    }

    #[test]
    fn context_clearing_does_not_clear_permanent_recovery_and_fake_transitions_fail_closed() {
        let events = vec![event(1, "runtime/progress-recorded"), event(2, "context/cleared")];
        let payloads = BTreeMap::from([("event-1".into(), progress(AgentPatchRecoveryState {
            targets: vec!["pending.txt".into()], ..Default::default()
        }))]);
        let selected = current_binding_progress(&events, &payloads, &binding(1), &"next-turn".into()).unwrap();
        assert_eq!(selected.len(), 1);
        let fake = vec![event(3, "session/agent-binding-changed")];
        let payloads = BTreeMap::from([("event-3".into(), transition(3, &binding(2)))]);
        assert!(current_binding_progress(&fake, &payloads, &binding(1), &"next-turn".into()).is_err());
    }

    #[test]
    fn settled_patch_state_needs_exact_source_and_owner_model_proof() {
        let state = nomifun_agent_runtime::AgentPatchRecoveryState::default();
        assert!(settled_model_transition(&state, true, true));
        assert!(!settled_model_transition(&state, false, true));
        assert!(!settled_model_transition(&state, true, false));
        for state in [
            nomifun_agent_runtime::AgentPatchRecoveryState { targets: vec!["file.txt".into()], ..state.clone() },
            nomifun_agent_runtime::AgentPatchRecoveryState { unresolved_targets: vec!["file.txt".into()], unresolved_before_input: Some(1), ..state.clone() },
            nomifun_agent_runtime::AgentPatchRecoveryState { target_budget_exceeded: true, ..state.clone() },
            nomifun_agent_runtime::AgentPatchRecoveryState { version: 1, ..state.clone() },
            nomifun_agent_runtime::AgentPatchRecoveryState { version: 0, ..state.clone() },
        ] {
            assert!(!settled_model_transition(&state, true, true));
        }
    }

    #[test]
    fn completed_failed_and_cancelled_turns_can_own_permanent_patch_recovery() {
        for kind in ["turn/completed", "turn/failed", "turn/cancelled"] {
            assert!(patch_recovery_source_terminal(kind), "{kind}");
        }
        for kind in [
            "turn/started",
            "turn/paused",
            "turn/unknown",
            "message/completed",
        ] {
            assert!(!patch_recovery_source_terminal(kind), "{kind}");
        }
    }

    #[test]
    fn versioned_patch_recovery_crosses_builds_but_not_session_turn_or_snapshot() {
        let session = AgentSessionId::from("session");
        let snapshot = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("a".repeat(64)),
        };
        let recorded = EngineBinding::new(
            session.clone(),
            RuntimeBindingId::from("old-runtime"),
            EngineBuildId::from("old-build"),
            DigestHex::from("b".repeat(64)),
            snapshot.clone(),
        )
        .unwrap();
        let operation = OperationId::from("old-turn");

        assert!(
            patch_recovery_source_matches(
                &recorded,
                &operation,
                &session,
                "old-turn",
                &snapshot,
            ),
            "build identity is deliberately absent from this portable-state check"
        );
        assert!(!patch_recovery_source_matches(
            &recorded,
            &operation,
            &AgentSessionId::from("other"),
            "old-turn",
            &snapshot,
        ));
        assert!(!patch_recovery_source_matches(
            &recorded,
            &OperationId::from("other"),
            &session,
            "old-turn",
            &snapshot,
        ));
        let other = ResolvedSnapshotRef {
            snapshot_id: ResolvedSnapshotId::from("other"),
            snapshot_digest: DigestHex::from("c".repeat(64)),
        };
        assert!(!patch_recovery_source_matches(
            &recorded,
            &operation,
            &session,
            "old-turn",
            &other,
        ));
        let malformed: EngineBinding = serde_json::from_value(serde_json::json!({
            "agent_session_id": "session",
            "runtime_binding_id": "old-runtime",
            "build_id": "old-build",
            "build_digest": "bad",
            "resolved_snapshot_ref": snapshot,
        }))
        .unwrap();
        assert!(!patch_recovery_source_matches(
            &malformed,
            &operation,
            &session,
            "old-turn",
            malformed.resolved_snapshot_ref(),
        ));
    }
}
