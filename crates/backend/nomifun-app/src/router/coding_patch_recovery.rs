//! Permanent Coding recovery obligations, not conversational replay. No tool
//! execution, filesystem access, effect settlement or quarantine resolution.
use super::engine_session_host::EngineTurnReceipt;
use nomifun_agent_contracts::ResolvedSnapshotRef;
use nomifun_coding_engine::{CodingEngineEvent, CodingPatchRecoveryState};
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Coding patch recovery: {message}"))
}

pub(super) async fn load(
    pool: &SqlitePool,
    receipt: &EngineTurnReceipt,
    snapshot: &ResolvedSnapshotRef,
) -> Result<CodingPatchRecoveryState, AppError> {
    let session = &receipt.session().session().conversation_id;
    let user = &receipt.session().principal().principal_id;
    let binding = receipt.session().engine_binding();
    let mut tx = pool.begin().await.map_err(failure)?;
    let (admitted,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM conversations c JOIN conversation_delivery_receipts r ON r.operation_id = c.active_turn_operation_id \
         WHERE c.conversation_id = ? AND c.user_id = ? AND c.status = 'running' AND c.admission_epoch = ? \
         AND r.operation_id = ? AND r.user_id = c.user_id AND r.conversation_id = c.conversation_id AND r.kind = 'turn' AND r.status = 'accepted')")
        .bind(session).bind(user).bind(receipt.admission_epoch()).bind(receipt.operation_id())
        .fetch_one(&mut *tx).await.map_err(failure)?;
    if !admitted {
        return Err(failure("current turn authority changed"));
    }

    // No join to messages, hidden flags, a finite replay window, or summaries.
    // Each started engine turn re-emits its loaded state. The owner has already
    // joined/recovered earlier turns before admitting this one.
    let head: Option<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, turn_operation_id, length(CAST(event_json AS BLOB)) FROM conversation_runtime_events \
         WHERE conversation_id = ? AND turn_operation_id != ? AND json_extract(event_json, '$.event') = 'patch_recovery_updated' \
         ORDER BY id DESC LIMIT 1")
        .bind(session).bind(receipt.operation_id()).fetch_optional(&mut *tx).await.map_err(failure)?;
    let state_id = head.as_ref().map_or(0, |(id, _, _)| *id);
    let (latest_dispatch,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(id), 0) FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id != ? AND id > ? \
         AND json_extract(event_json, '$.event') = 'host_tool_dispatch' AND json_extract(event_json, '$.dispatch.capability_id') = 'fs.patch'")
        .bind(session).bind(receipt.operation_id()).bind(state_id).fetch_one(&mut *tx).await.map_err(failure)?;
    let Some((id, operation, bytes)) = head else {
        if latest_dispatch != 0 {
            return Err(failure(
                "prior patch dispatch has no permanent recovery state",
            ));
        }
        tx.commit().await.map_err(failure)?;
        return Ok(CodingPatchRecoveryState::default());
    };
    if !(1..=128 * 1024).contains(&bytes) || operation.is_empty() || operation.len() > 1024 {
        return Err(failure("invalid or oversized recovery event"));
    }
    let source: Option<(String, i64)> = sqlx::query_as(
        "SELECT r.status, length(CAST(e.event_json AS BLOB)) FROM conversation_delivery_receipts r \
         JOIN conversation_runtime_events e ON e.turn_operation_id = r.operation_id AND e.conversation_id = r.conversation_id AND e.sequence = 1 \
         WHERE r.user_id = ? AND r.conversation_id = ? AND r.operation_id = ? AND r.kind = 'turn'")
        .bind(user).bind(session).bind(&operation).fetch_optional(&mut *tx).await.map_err(failure)?;
    let Some((status, root_bytes)) = source else {
        return Err(failure("recovery state has no permanent owner/turn root"));
    };
    if status != "completed" || !(1..=16 * 1024).contains(&root_bytes) {
        return Err(failure(
            "recovery source is unsettled or its binding is oversized",
        ));
    }
    let (root,): (String,) = sqlx::query_as(
        "SELECT event_json FROM conversation_runtime_events WHERE conversation_id = ? AND turn_operation_id = ? AND sequence = 1")
        .bind(session).bind(&operation).fetch_one(&mut *tx).await.map_err(failure)?;
    let CodingEngineEvent::TurnStarted {
        binding: recorded,
        turn_operation_id,
    } = serde_json::from_str::<CodingEngineEvent>(&root).map_err(failure)?
    else {
        return Err(failure("recovery source has no engine binding"));
    };
    if recorded.agent_session_id().as_ref() != session
        || turn_operation_id.as_ref() != operation
        || recorded.family_id().as_ref() != binding.family_id
        || recorded.build_id().as_ref() != binding.build_id
        || recorded.build_digest().as_ref() != binding.build_digest
        || recorded.resolved_snapshot_ref() != snapshot
    {
        return Err(failure(
            "recovery state differs from exact Session engine/snapshot",
        ));
    }
    let (raw,): (String,) = sqlx::query_as(
        "SELECT event_json FROM conversation_runtime_events WHERE id = ? AND conversation_id = ? AND turn_operation_id = ?")
        .bind(id).bind(session).bind(&operation).fetch_one(&mut *tx).await.map_err(failure)?;
    let CodingEngineEvent::PatchRecoveryUpdated { state } =
        serde_json::from_str::<CodingEngineEvent>(&raw).map_err(failure)?
    else {
        return Err(failure("invalid recovery event kind"));
    };
    state.validate().map_err(failure)?;
    if latest_dispatch > id && !state.has_pending() {
        return Err(failure(
            "a later patch dispatch is not covered by recovery state",
        ));
    }
    tx.commit().await.map_err(failure)?;
    Ok(state)
}
