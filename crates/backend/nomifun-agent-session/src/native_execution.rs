//! Scoped, expiring native execution ownership. Claims, renewal and producer
//! writes serialize through the canonical SQLite writer transaction.
use super::*;

pub const NATIVE_EXECUTION_LEASE_MS: i64 = 90_000;

pub(super) async fn reject_unleased_native_turn_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId) -> Result<(), SessionStoreError> {
    let owned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id = ? AND operation_id = ? AND execution_owner IS NOT NULL")
        .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&mut **tx).await?;
    if owned != 0 { return Err(SessionStoreError::ExecutionFenced); }
    Ok(())
}

/// Not deserializable model input. Only the canonical Store can construct it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeExecutionLease {
    session_id: AgentSessionId,
    operation_id: OperationId,
    holder: String,
    fence: u64,
    generation: u64,
}

impl NativeExecutionLease {
    pub fn session_id(&self) -> &AgentSessionId { &self.session_id }
    pub fn operation_id(&self) -> &OperationId { &self.operation_id }
    pub fn fence(&self) -> u64 { self.fence }
    pub fn generation(&self) -> u64 { self.generation }
}

#[derive(Clone, Debug)]
pub struct NativeExecutionClaim {
    pub owner: PrincipalRef,
    pub agent_session_id: AgentSessionId,
    pub operation_id: OperationId,
    pub snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pub active_set_generation: u64,
    pub holder: String,
    pub expected_fence: u64,
    /// None starts a previously unowned Turn. Some attempts recovery of this
    /// exact checkpoint, only after the previous producer lease has expired.
    pub checkpoint: Option<(u64, DigestHex, u64)>,
}

impl AgentSessionStore {
    /// Read-only accounting; never deletes immutable evidence to renew a
    /// window. The caller must still hold this exact native execution lease.
    pub async fn native_payload_bytes(&self, lease: &NativeExecutionLease) -> Result<u64, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        check_native_lease_tx(&mut tx, lease, false).await?;
        let bytes: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(byte_len),0) FROM agent_payloads WHERE session_id=?")
            .bind(lease.session_id.as_ref()).fetch_one(&mut *tx).await?;
        as_u64(bytes, "native Session payload bytes")
    }

    pub async fn native_execution_deadline(&self, session: &AgentSessionId, operation: &OperationId) -> Result<i64, SessionStoreError> {
        Ok(sqlx::query_scalar("SELECT execution_lease_until FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&self.pool).await?)
    }
    pub async fn native_execution_generation(&self, session: &AgentSessionId, operation: &OperationId) -> Result<u64, SessionStoreError> {
        let generation: i64 = sqlx::query_scalar("SELECT CASE WHEN execution_generation > 0 THEN execution_generation ELSE started_at END FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&self.pool).await?;
        as_u64(generation, "execution generation")
    }

    /// One bounded snapshot for a product relay. State from another execution
    /// generation cannot authorize this relay's late terminal notification.
    /// This is diagnostic data and grants no execution/replay authority.
    pub async fn native_execution_notification_state(&self, session: &AgentSessionId, operation: &OperationId, generation: u64)
        -> Result<Option<String>, SessionStoreError> {
        Ok(sqlx::query_scalar("SELECT CASE WHEN t.state='running' AND t.native_pause_json IS NOT NULL THEN 'paused' ELSE t.state END \
            FROM agent_turns t JOIN agent_sessions s ON s.agent_session_id=t.session_id \
            WHERE t.session_id=? AND t.operation_id=? AND s.state='live' \
            AND (CASE WHEN t.execution_generation>0 THEN t.execution_generation ELSE t.started_at END)=?")
            .bind(session.as_ref()).bind(operation.as_ref()).bind(as_i64(generation,"execution generation")?).fetch_optional(&self.pool).await?)
    }

    pub async fn claim_native_execution(&self, claim: NativeExecutionClaim) -> Result<NativeExecutionLease, SessionStoreError> {
        self.claim_native_execution_inner(claim, false).await
    }

    pub async fn claim_native_empty_recovery(&self, claim: NativeExecutionClaim) -> Result<NativeExecutionLease, SessionStoreError> {
        if claim.checkpoint.is_some() { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
        self.claim_native_execution_inner(claim, true).await
    }

    async fn claim_native_execution_inner(&self, claim: NativeExecutionClaim, recover_empty: bool) -> Result<NativeExecutionLease, SessionStoreError> {
        if claim.holder.is_empty() || claim.holder.len() > 128 || claim.holder.trim() != claim.holder {
            return Err(SessionStoreError::InvalidSession("invalid native execution holder".into()));
        }
        let mut tx = self.begin_write_transaction().await?;
        let session = live_session_by_id_tx(&mut tx, claim.agent_session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, claim.agent_session_id.as_ref()).await?;
        if session.owner_ref != claim.owner || session.agent_binding.resolved_snapshot_ref != claim.snapshot
            || head.status != "running" || head.active_turn_id.as_deref() != Some(claim.operation_id.as_ref())
            || head.active_set_generation != claim.active_set_generation {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let row: (i64, Option<String>, i64, i64, Option<String>, Option<i64>, String, Option<String>) = sqlx::query_as(
            "SELECT execution_fence, execution_owner, execution_lease_until, native_checkpoint_revision, \
             native_checkpoint_digest, native_checkpoint_seq, state, native_checkpoint_json \
             FROM agent_turns WHERE session_id = ? AND operation_id = ?")
            .bind(claim.agent_session_id.as_ref()).bind(claim.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        let current_fence = as_u64(row.0, "execution fence")?;
        if current_fence != claim.expected_fence || row.6 != "running" { return Err(SessionStoreError::ExecutionFenced); }
        let now = wall_clock_now_ms();
        let same_holder = row.1.as_deref() == Some(claim.holder.as_str());
        let prior: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events e JOIN agent_turns t ON t.session_id=e.session_id AND t.operation_id=? JOIN agent_events s ON s.event_id=t.started_event_id \
            WHERE e.session_id=? AND e.seq>s.seq AND ((e.correlation_id=? AND e.kind IN ('runtime/progress-recorded','context/model-visible-applied')) OR (e.kind='tool/call-started' AND e.causation_event_id=t.started_event_id))")
            .bind(claim.operation_id.as_ref()).bind(claim.agent_session_id.as_ref()).bind(claim.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        let same_acquisition = same_holder && claim.checkpoint.is_none() && !recover_empty && prior == 0;
        let fence = if same_acquisition {
            current_fence // Idempotent acquisition after a lost acknowledgement.
        } else if let Some((revision, digest, through_seq)) = &claim.checkpoint {
            if row.2 > now && row.1.is_some() { return Err(SessionStoreError::ExecutionLeaseActive); }
            if as_u64(row.3, "checkpoint revision")? != *revision || row.4.as_deref() != Some(digest.as_ref())
                || row.5 != Some(as_i64(*through_seq, "checkpoint cursor")?) {
                return Err(SessionStoreError::Conflict("recovery checkpoint changed".into()));
            }
            let state: Value = serde_json::from_str(row.7.as_deref().ok_or_else(|| SessionStoreError::Conflict("checkpoint body missing".into()))?)?;
            if digest_bytes(&canonical_json_bytes(&state)?) != *digest { return Err(SessionStoreError::InvalidPayload("recovery checkpoint digest mismatch".into())); }
            verify_recovery_tail_tx(&mut tx, &claim.agent_session_id, &claim.operation_id, *through_seq).await?;
            current_fence.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("execution fence exhausted".into()))?
        } else if recover_empty {
            if row.1.is_some() && row.2 > now { return Err(SessionStoreError::ExecutionLeaseActive); }
            let effects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND turn_id=?")
                .bind(claim.agent_session_id.as_ref()).bind(claim.operation_id.as_ref()).fetch_one(&mut *tx).await?;
            if effects != 0 || row.7.is_some()
                || (prior != 0 && !empty_native_preamble_tx(&mut tx, &claim.agent_session_id, &claim.operation_id).await?) {
                return Err(SessionStoreError::RecoveryRequiresReconciliation);
            }
            current_fence.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("execution fence exhausted".into()))?
        } else {
            if row.1.is_some() { return Err(SessionStoreError::ExecutionLeaseActive); }
            if current_fence != 0 || prior != 0 { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
            current_fence
        };
        let started: (String, i64, i64) = sqlx::query_as("SELECT t.started_event_id, e.seq, t.execution_generation FROM agent_turns t JOIN agent_events e ON e.event_id=t.started_event_id WHERE t.session_id=? AND t.operation_id=?")
            .bind(claim.agent_session_id.as_ref()).bind(claim.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        let generation = if same_acquisition { as_u64(started.2, "execution generation")? } else {
            let identity = format!("native-execution:{}:{}:{fence}", claim.agent_session_id.as_ref(), claim.operation_id.as_ref());
            let event = SessionEventAppend { agent_session_id: claim.agent_session_id.clone(), event_id: identity.clone().into(),
                producer_id: "runtime_supervisor".into(), idempotency_key: identity.into(), runtime_binding_id: None, runtime_producer_seq: None,
                semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: SessionEventKind("runtime/execution-claimed".into()), kind_version: 1,
                    correlation_id: claim.operation_id.as_ref().into(), causation_event_id: Some(started.0.clone().into()),
                    payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":claim.operation_id,"execution_fence":fence,"holder_digest":digest_bytes(claim.holder.as_bytes())}))),
                } };
            let ack = required_ack(self.append_event_tx(&mut tx, &event, None).await?)?;
            if claim.checkpoint.is_some() || recover_empty { ack.seq } else { as_u64(started.1, "turn generation")? }
        };
        let updated = sqlx::query("UPDATE agent_turns SET execution_owner = ?, execution_fence = ?, execution_lease_until = ?, execution_generation = ? \
            WHERE session_id = ? AND operation_id = ? AND state = 'running' AND execution_fence = ?")
            .bind(&claim.holder).bind(as_i64(fence, "execution fence")?).bind(now.saturating_add(NATIVE_EXECUTION_LEASE_MS))
            .bind(as_i64(generation, "execution generation")?)
            .bind(claim.agent_session_id.as_ref()).bind(claim.operation_id.as_ref()).bind(row.0).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 { return Err(SessionStoreError::ExecutionFenced); }
        tx.commit().await?;
        Ok(NativeExecutionLease { session_id: claim.agent_session_id, operation_id: claim.operation_id, holder: claim.holder, fence, generation })
    }

    pub async fn renew_native_execution(&self, lease: &NativeExecutionLease) -> Result<(), SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        check_native_lease_tx(&mut tx, lease, true).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn heartbeat_native_execution(&self, lease: &NativeExecutionLease) -> Result<bool, SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        if check_native_identity_tx(&mut tx, lease).await? != "running" { return Ok(false); }
        check_native_lease_tx(&mut tx, lease, true).await?;
        tx.commit().await?;
        return Ok(true);
    }

    pub async fn verify_native_execution(&self, lease: &NativeExecutionLease) -> Result<(), SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        check_native_lease_tx(&mut tx, lease, false).await?;
        Ok(())
    }

    pub async fn append_native_event(&self, lease: &NativeExecutionLease, append: &SessionEventAppend,
        payload: Option<&SessionPayloadRecord>) -> Result<SessionEventAppendResult, SessionStoreError> {
        if append.agent_session_id != lease.session_id || append.producer_id.as_ref() != "runtime_supervisor"
            || !matches!(append.semantic_event.kind.0.as_str(), "runtime/progress-recorded" | "tool/call-started" | "turn/cancelled") {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let mut tx = self.begin_write_transaction().await?;
        check_native_lease_tx(&mut tx, lease, true).await?;
        let result = self.append_event_tx(&mut tx, append, payload).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Close an already-admitted observation after cancellation. This cannot
    /// admit new tools/models, and a superseded lease is rejected even here.
    pub async fn append_native_observation(&self, lease: &NativeExecutionLease, append: &SessionEventAppend,
        payload: Option<&SessionPayloadRecord>) -> Result<SessionEventAppendResult, SessionStoreError> {
        if append.agent_session_id != lease.session_id || append.producer_id.as_ref() != "runtime_supervisor"
            || !matches!(append.semantic_event.kind.0.as_str(), "runtime/progress-recorded" | "thinking/content-part"
                | "message/content-part" | "message/completed" | "tool/result-recorded") {
            return Err(SessionStoreError::ExecutionFenced);
        }
        if append.semantic_event.kind.0 == "runtime/progress-recorded" {
            let data = match &append.semantic_event.payload {
                SessionEventPayloadRef::Empty => return Err(SessionStoreError::InvalidEvent("native observation payload missing".into())),
                SessionEventPayloadRef::InlineJson(value) => value.0.clone(),
                SessionEventPayloadRef::Stored(_) => match payload.map(|value| &value.body) {
                    Some(SessionPayloadBody::Json(value)) => value.0.clone(),
                    _ => return Err(SessionStoreError::InvalidEvent("native observation payload missing".into())),
                },
            };
            let kind = data.pointer("/event/event").and_then(Value::as_str);
            if !matches!(kind, Some(
                "host_tool_settled" | "host_resource_settled" | "host_process_quiescent" | "host_cleanup_proven"
                | "turn_started" | "turn_input_scope" | "steering_deferred" | "steering_inputs"
                | "turn_completed" | "turn_failed" | "turn_cancelled" | "turn_paused" | "output_text_delta" | "reasoning_delta"
                | "tool_completed" | "work_status" | "plan_updated" | "patch_recovery_updated"
                | "model_response_rejected" | "model_output_truncated" | "context_compacted" | "compaction_usage"
                | "completion_observation" | "completion_reported" | "completion_delivered" | "instructions_updated" | "context_prepared"
                | "runtime_modules_activated" | "completion_review" | "execution_budget_prepared" | "tool_results_ordered" | "usage"
            )) { return Err(SessionStoreError::ExecutionFenced); }
        }
        let mut tx = self.begin_write_transaction().await?;
        check_native_identity_tx(&mut tx, lease).await?;
        let result = self.append_event_tx(&mut tx, append, payload).await?;
        tx.commit().await?;
        Ok(result)
    }
}

pub(super) async fn check_native_identity_tx(tx: &mut Transaction<'_, Sqlite>, lease: &NativeExecutionLease) -> Result<String, SessionStoreError> {
    let row: Option<(i64, Option<String>, String)> = sqlx::query_as(
        "SELECT execution_fence, execution_owner, state FROM agent_turns WHERE session_id = ? AND operation_id = ?")
        .bind(lease.session_id.as_ref()).bind(lease.operation_id.as_ref()).fetch_optional(&mut **tx).await?;
    match row {
        Some((fence, holder, state)) if fence >= 0 && fence as u64 == lease.fence
            && holder.as_deref() == Some(lease.holder.as_str()) => Ok(state),
        _ => Err(SessionStoreError::ExecutionFenced),
    }
}

/// Renewal and the protected write happen under the same SQLite lock. An
/// expired producer may renew only if no replacement won the ownership CAS.
pub(super) async fn check_native_lease_tx(tx: &mut Transaction<'_, Sqlite>, lease: &NativeExecutionLease, renew: bool) -> Result<(), SessionStoreError> {
    if check_native_identity_tx(tx, lease).await? != "running" {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let head = head_by_id_tx(tx, lease.session_id.as_ref()).await?;
    if head.status != "running" || head.active_turn_id.as_deref() != Some(lease.operation_id.as_ref()) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    if renew {
        sqlx::query("UPDATE agent_turns SET execution_lease_until = ? WHERE session_id = ? AND operation_id = ?")
            .bind(wall_clock_now_ms().saturating_add(NATIVE_EXECUTION_LEASE_MS))
            .bind(lease.session_id.as_ref()).bind(lease.operation_id.as_ref()).execute(&mut **tx).await?;
    }
    Ok(())
}

async fn verify_recovery_tail_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId, through_seq: u64) -> Result<(), SessionStoreError> {
    let unsettled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id = ? AND state IN ('pending','unknown')")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    if unsettled != 0 { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
    let rows = event_rows_for_session_tx(tx, session.as_ref()).await?;
    for row in rows {
        let event = event_from_row(row)?;
        if event.seq <= through_seq { continue; }
        if event.kind.0 == "tool/call-started" { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
        if event.kind.0 != "runtime/progress-recorded" || event.correlation_id.as_ref() != operation.as_ref() { continue; }
        let value = payload_value_for_event_tx(tx, &event).await?;
        // Only an unexecuted model prefix can be discarded automatically.
        // Effects/controls outside this list require owner reconciliation, not
        // replaying a stale checkpoint over work that may already have run.
        if !matches!(value.pointer("/event/event").and_then(Value::as_str), Some(
            "model_step_started" | "output_text_delta" | "reasoning_delta" | "tool_call_delta" | "tool_call_completed"
            | "usage" | "compaction_started" | "compaction_usage" | "context_compacted"
            | "context_limit_recovery_started" | "model_output_truncated" | "model_response_rejected"
            | "execution_resumed" | "execution_budget_prepared" | "execution_segment_renewed"
            | "context_prepared" | "runtime_modules_activated"
        )) { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
    }
    Ok(())
}

/// A crash between TurnStarted/TurnInputScope and the first checkpoint is
/// still an untouched execution. No model claim, proposal, tool or state
/// update may be hidden behind this narrow bootstrap exception.
async fn empty_native_preamble_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId) -> Result<bool, SessionStoreError> {
    let rows = event_rows_for_session_tx(tx, session.as_ref()).await?;
    let mut kinds = Vec::new();
    for row in rows {
        let event = event_from_row(row)?;
        if event.correlation_id.as_ref() != operation.as_ref() { continue; }
        if event.kind.0 == "context/model-visible-applied" { return Ok(false); }
        if event.kind.0 != "runtime/progress-recorded" { continue; }
        let value = payload_value_for_event_tx(tx, &event).await?;
        let kind = value.pointer("/event/event").and_then(Value::as_str).unwrap_or("");
        if kinds.len() >= 2 || value.get("producer_seq").and_then(Value::as_u64) != Some(kinds.len() as u64 + 1) { return Ok(false); }
        kinds.push(kind.to_owned());
    }
    Ok(matches!(kinds.as_slice(), [started] if started == "turn_started")
        || matches!(kinds.as_slice(), [started, scope] if started == "turn_started" && scope == "turn_input_scope"))
}
