//! Permanent Nomi patch-recovery obligations reconstructed from canonical
//! AgentSession events. No old Conversation receipt/journal table is read.

use super::engine_session_host::{EngineSessionHost, EngineTurnReceipt};
use nomifun_agent_contracts::{AgentSessionId, OperationId, ResolvedSnapshotRef};
use nomifun_coding_engine::{CodingEngineEvent, CodingPatchRecoveryState};
use nomifun_common::AppError;
use serde_json::Value;

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi patch recovery: {message}"))
}

pub(super) async fn load(
    host: &EngineSessionHost,
    receipt: &EngineTurnReceipt,
    snapshot: &ResolvedSnapshotRef,
) -> Result<CodingPatchRecoveryState, AppError> {
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

    let mut latest_state: Option<(u64, String, CodingPatchRecoveryState)> = None;
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
        if let Ok(CodingEngineEvent::PatchRecoveryUpdated { state }) =
            serde_json::from_value::<CodingEngineEvent>(value.clone())
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
        return Ok(CodingPatchRecoveryState::default());
    };
    let source_terminal = facts.events.iter().find(|event| {
        event.correlation_id.as_ref() == source_operation
            && event.kind.0 == "turn/completed"
    });
    if source_terminal.is_none() {
        return Err(failure("recovery source is not a completed canonical Turn"));
    }
    let source_start = facts.events.iter().find_map(|event| {
        if event.kind.0 != "runtime/progress-recorded"
            || event.correlation_id.as_ref() != source_operation
        {
            return None;
        }
        let payload = facts.event_payloads.get(event.event_id.as_ref())?.get("event")?;
        serde_json::from_value::<CodingEngineEvent>(payload.clone())
            .ok()
            .and_then(|event| match event {
                CodingEngineEvent::TurnStarted {
                    binding,
                    turn_operation_id,
                } => Some((binding, turn_operation_id)),
                _ => None,
            })
    });
    let Some((recorded, turn_operation_id)) = source_start else {
        return Err(failure("recovery source has no engine binding"));
    };
    let binding = receipt.session().engine_binding();
    if recorded.agent_session_id() != &session
        || turn_operation_id.as_ref() != source_operation
        || recorded.build_id().as_ref() != binding.build_id
        || recorded.build_digest().as_ref() != binding.build_digest
        || recorded.resolved_snapshot_ref() != snapshot
    {
        return Err(failure(
            "recovery state differs from exact Session engine/snapshot",
        ));
    }
    if latest_patch_dispatch > state_seq && !state.has_pending() {
        return Err(failure(
            "a later patch dispatch is not covered by recovery state",
        ));
    }
    Ok(state)
}
