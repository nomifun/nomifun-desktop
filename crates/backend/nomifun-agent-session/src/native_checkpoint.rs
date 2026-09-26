//! Atomic native progress snapshots in the canonical Turn row. A loaded
//! snapshot is data, NOT permission to resume; a recovery owner must fence old
//! writers and reconcile all events/effects after its cursor before execution.
use super::*;

pub const MAX_NATIVE_CHECKPOINT_BYTES: usize = nomifun_agent_contracts::MAX_NATIVE_EXECUTION_CHECKPOINT_BYTES;

#[derive(Clone, Debug)]
pub struct NativeCheckpointWrite {
    pub owner: PrincipalRef,
    pub operation_id: OperationId,
    pub snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pub active_set_generation: u64,
    pub expected_revision: u64,
    pub execution_fence: u64,
    pub lease: Option<NativeExecutionLease>,
    pub state: StrictJsonValue,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpoint {
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub revision: u64,
    pub through_seq: u64,
    pub digest: nomifun_agent_contracts::DigestHex,
    pub execution_fence: u64,
    pub turn_state: String,
    pub state: StrictJsonValue,
}

impl AgentSessionStore {
    /// Metadata event and latest snapshot commit together. The caller supplies
    /// only an ordinary runtime progress envelope; no new event authority is
    /// created and model-authored input cannot call this API.
    pub async fn save_native_checkpoint(
        &self,
        append: &SessionEventAppend,
        request: NativeCheckpointWrite,
    ) -> Result<NativeCheckpoint, SessionStoreError> {
        let bytes = canonical_json_bytes(&request.state.0)?;
        if !request.state.0.is_object() || bytes.len() > MAX_NATIVE_CHECKPOINT_BYTES {
            return Err(SessionStoreError::InvalidPayload("native checkpoint exceeds the bounded state contract".into()));
        }
        let digest = digest_bytes(&bytes);
        let revision = request.expected_revision.checked_add(1)
            .ok_or_else(|| SessionStoreError::Conflict("checkpoint revision exhausted".into()))?;
        let SessionEventPayloadRef::InlineJson(payload) = &append.semantic_event.payload else {
            return Err(SessionStoreError::InvalidEvent("checkpoint metadata must be inline JSON".into()));
        };
        if append.semantic_event.kind.0 != "runtime/progress-recorded"
            || append.semantic_event.correlation_id.as_ref() != request.operation_id.as_ref()
            || payload.0.pointer("/event/event").and_then(Value::as_str) != Some("execution_checkpoint_saved")
            || payload.0.pointer("/event/revision").and_then(Value::as_u64) != Some(revision)
            || payload.0.pointer("/event/digest").and_then(Value::as_str) != Some(digest.as_ref())
        {
            return Err(SessionStoreError::InvalidEvent("checkpoint metadata differs from its state or Turn".into()));
        }
        let mut tx = self.begin_write_transaction().await?;
        if let Some(lease) = &request.lease {
            if lease.session_id() != &append.agent_session_id || lease.operation_id() != &request.operation_id
                || lease.fence() != request.execution_fence { return Err(SessionStoreError::ExecutionFenced); }
            super::native_execution::check_native_lease_tx(&mut tx, lease, true).await?;
        } else {
            super::native_execution::reject_unleased_native_turn_tx(&mut tx, &append.agent_session_id, &request.operation_id).await?;
        }
        let session = live_session_by_id_tx(&mut tx, append.agent_session_id.as_ref()).await?;
        if session.owner_ref != request.owner || session.agent_binding.resolved_snapshot_ref != request.snapshot {
            return Err(SessionStoreError::Conflict("checkpoint owner or Snapshot differs from the live Session".into()));
        }
        let head = head_by_id_tx(&mut tx, append.agent_session_id.as_ref()).await?;
        if head.status != "running" || head.active_turn_id.as_deref() != Some(request.operation_id.as_ref())
            || head.active_set_generation != request.active_set_generation {
            return Err(SessionStoreError::Conflict("checkpoint requires the exact active Turn and capability generation".into()));
        }
        let row: (i64, Option<String>, Option<i64>, i64, String) = sqlx::query_as(
            "SELECT native_checkpoint_revision, native_checkpoint_digest, native_checkpoint_seq, execution_fence, state \
             FROM agent_turns WHERE session_id = ? AND operation_id = ?")
            .bind(append.agent_session_id.as_ref()).bind(request.operation_id.as_ref())
            .fetch_one(&mut *tx).await?;
        if row.4 != "running" || as_u64(row.3, "execution fence")? != request.execution_fence {
            return Err(SessionStoreError::Conflict("checkpoint writer is terminal or fenced".into()));
        }
        if as_u64(row.0, "checkpoint revision")? == revision && row.1.as_deref() == Some(digest.as_ref()) {
            let existing = event_by_producer_key_tx(&mut tx, append.producer_id.as_ref(), append.idempotency_key.as_ref()).await?;
            if let Some(existing) = existing {
                let record = event_from_row(existing)?;
                if record.agent_session_id == append.agent_session_id && record.correlation_id.as_ref() == request.operation_id.as_ref() {
                    let stored = payload_value_for_event_tx(&mut tx, &record).await?;
                    if stored == payload.0 {
                        return Ok(NativeCheckpoint { agent_session_id: append.agent_session_id.clone(),
                            operation_id: request.operation_id, revision, through_seq: as_u64(row.2.unwrap_or(0), "checkpoint cursor")?,
                            digest, execution_fence: request.execution_fence, turn_state: "running".into(), state: request.state });
                    }
                }
            }
            return Err(SessionStoreError::IdempotencyConflict("checkpoint replay has different metadata".into()));
        }
        if as_u64(row.0, "checkpoint revision")? != request.expected_revision {
            return Err(SessionStoreError::Conflict("checkpoint revision changed".into()));
        }
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_effects WHERE session_id = ? AND turn_id = ? AND state IN ('pending','unknown')")
            .bind(append.agent_session_id.as_ref()).bind(request.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        if pending != 0 {
            return Err(SessionStoreError::CheckpointNotQuiescent);
        }
        let appended = self.append_event_tx(&mut tx, append, None).await?;
        let ack = required_ack(appended)?;
        let updated = sqlx::query(
            "UPDATE agent_turns SET native_checkpoint_json = ?, native_checkpoint_digest = ?, \
             native_checkpoint_revision = ?, native_checkpoint_seq = ? \
             WHERE session_id = ? AND operation_id = ? AND state = 'running' \
             AND native_checkpoint_revision = ? AND execution_fence = ?")
            .bind(String::from_utf8(bytes).map_err(|_| SessionStoreError::InvalidPayload("checkpoint is not UTF-8".into()))?)
            .bind(digest.as_ref()).bind(as_i64(revision, "checkpoint revision")?)
            .bind(as_i64(ack.seq, "checkpoint cursor")?)
            .bind(append.agent_session_id.as_ref()).bind(request.operation_id.as_ref())
            .bind(as_i64(request.expected_revision, "checkpoint revision")?)
            .bind(as_i64(request.execution_fence, "execution fence")?).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(SessionStoreError::Conflict("checkpoint compare-and-swap lost its Turn".into()));
        }
        tx.commit().await?;
        Ok(NativeCheckpoint { agent_session_id: append.agent_session_id.clone(), operation_id: request.operation_id,
            revision, through_seq: ack.seq, digest, execution_fence: request.execution_fence, turn_state: "running".into(), state: request.state })
    }

    pub async fn load_native_checkpoint(
        &self, owner: &PrincipalRef, session_id: &AgentSessionId, operation_id: &OperationId,
    ) -> Result<Option<NativeCheckpoint>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if &session.owner_ref != owner { return Err(SessionStoreError::Conflict("checkpoint belongs to another owner".into())); }
        let row: Option<(String, String, i64, i64, i64, String)> = sqlx::query_as(
            "SELECT native_checkpoint_json, native_checkpoint_digest, native_checkpoint_revision, native_checkpoint_seq, execution_fence, state \
             FROM agent_turns WHERE session_id = ? AND operation_id = ? AND state IN ('running','failed','interrupted') AND native_checkpoint_json IS NOT NULL")
            .bind(session_id.as_ref()).bind(operation_id.as_ref()).fetch_optional(&mut *tx).await?;
        let Some((json, expected, revision, seq, fence, turn_state)) = row else { return Ok(None); };
        if json.len() > MAX_NATIVE_CHECKPOINT_BYTES { return Err(SessionStoreError::InvalidPayload("stored checkpoint exceeds its budget".into())); }
        let state: Value = serde_json::from_str(&json)?;
        let digest = digest_bytes(&canonical_json_bytes(&state)?);
        if digest.as_ref() != expected || !state.is_object() {
            return Err(SessionStoreError::InvalidPayload("stored checkpoint digest differs from its state".into()));
        }
        Ok(Some(NativeCheckpoint { agent_session_id: session_id.clone(), operation_id: operation_id.clone(),
            revision: as_u64(revision, "checkpoint revision")?, through_seq: as_u64(seq, "checkpoint cursor")?,
            execution_fence: as_u64(fence, "execution fence")?, turn_state, digest, state: StrictJsonValue(state) }))
    }
}
