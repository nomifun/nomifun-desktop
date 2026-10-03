//! Permanent Nomi patch-recovery obligations reconstructed from canonical
//! AgentSession events. No old Conversation receipt/journal table is read.

use super::engine_session_host::{EngineSessionHost, EngineTurnReceipt};
use nomifun_agent_contracts::{AgentSessionId, OperationId, ResolvedSnapshotRef};
use nomifun_agent_runtime::{AgentEngineEvent, AgentPatchRecoveryState};
use nomifun_common::AppError;
use serde_json::Value;

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
    {
        return Err(failure("current turn authority changed"));
    }

    let mut progress = facts
        .events
        .iter()
        .filter(|event| {
            event.kind.0 == "runtime/progress-recorded"
                && event.correlation_id.as_ref() != current.as_ref()
        })
        .filter_map(|event| {
            facts
                .event_payloads
                .get(event.event_id.as_ref())
                .and_then(|payload| payload.get("event"))
                .map(|payload| (event, payload))
        })
        .collect::<Vec<_>>();
    progress.sort_by_key(|(event, _)| event.seq);

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
        if let Ok(AgentEngineEvent::PatchRecoveryUpdated { state }) =
            serde_json::from_value::<AgentEngineEvent>(value.clone())
        {
            state.validate().map_err(failure)?;
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
    use super::{patch_recovery_source_matches, patch_recovery_source_terminal, settled_model_transition};
    use nomifun_agent_contracts::{
        AgentSessionId, DigestHex, OperationId, ResolvedSnapshotId, ResolvedSnapshotRef,
        RuntimeBindingId,
    };
    use nomifun_agent_runtime::{EngineBinding, EngineBuildId};

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
