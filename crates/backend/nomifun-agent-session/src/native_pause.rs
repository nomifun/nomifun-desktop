//! Nonterminal suspension. No terminal Turn is reopened and no model-facing
//! tool can mint a resume authorization. All changes use the canonical writer.
use super::*;
use nomifun_agent_contracts::{NativeBudgetIncrease, NativeExecutionBudget};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePauseState {
    pub revision: u64,
    pub reason: String,
    pub checkpoint_revision: u64,
    pub checkpoint_digest: Option<DigestHex>,
    pub execution_fence: u64,
    pub cleanup_proven: bool,
    pub paused_at_ms: i64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResumeRequest {
    pub operation_id: OperationId,
    pub idempotency_key: String,
    pub expected_pause_revision: u64,
    pub expected_checkpoint_revision: u64,
    pub expected_checkpoint_digest: DigestHex,
    #[serde(default)]
    pub budget: NativeBudgetIncrease,
    /// Required only when the previous owner did not prove resource cleanup.
    /// This is an explicit human/operator attestation, not automatic proof.
    #[serde(default)]
    pub cleanup_attestation: Option<NativeOwnerEvidence>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeOwnerEvidence {
    pub verified: bool,
    pub evidence_digest: DigestHex,
    pub reference: String,
}

impl NativeOwnerEvidence {
    pub fn validate(&self) -> Result<(), SessionStoreError> {
        if !self.verified || self.evidence_digest.as_ref().len() != 64
            || !self.evidence_digest.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.reference.trim().is_empty() || self.reference.len() > 1024
            || self.reference.chars().any(char::is_control) {
            return Err(SessionStoreError::InvalidEvent("owner reconciliation requires explicit verification and bounded evidence identity".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct NativeResumeReceipt {
    pub operation_id: OperationId,
    pub authorization_event_id: EventId,
    pub seq: u64,
    pub duplicate: bool,
}

/// Data prepared by the trusted Runtime codec, never accepted as HTTP/model
/// input. The commit rechecks every canonical cursor, effect and authority.
pub struct NativeResumePreparation {
    pub expected_head_seq: u64,
    pub expected_fence: u64,
    pub snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pub active_set_generation: u64,
    pub checkpoint_state: StrictJsonValue,
    pub observations: Vec<StrictJsonValue>,
}

pub(super) fn native_key(kind: &str, session: &AgentSessionId, key: &str) -> Result<String, SessionStoreError> {
    if key.trim().is_empty() || key.len() > 256 || key.chars().any(char::is_control) {
        return Err(SessionStoreError::InvalidEvent("native control requires a bounded idempotency key".into()));
    }
    Ok(format!("native-{kind}:{}:{}", session.as_ref(), digest_bytes(key.as_bytes()).as_ref()))
}

pub(super) fn native_control_event(session: &AgentSessionId, operation: &OperationId,
    identity: String, kind: &str, started: EventId, payload: Value) -> SessionEventAppend {
    SessionEventAppend {
        agent_session_id: session.clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
        idempotency_key: identity.into(), runtime_binding_id: None, runtime_producer_seq: None,
        semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
            kind: SessionEventKind(kind.into()), kind_version: 1, correlation_id: operation.as_ref().into(),
            causation_event_id: Some(started), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
        },
    }
}

pub(super) async fn native_payload_capacity_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId) -> Result<u64, SessionStoreError> {
    let capacity: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(CAST(json_extract(native_budget_json,'$.session_payload_bytes') AS INTEGER)),16777216) FROM agent_turns WHERE session_id=?")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let capacity = as_u64(capacity,"authorized payload capacity")?.max(16 * 1024 * 1024);
    if capacity > nomifun_agent_contracts::MAX_NATIVE_APPROVED_PAYLOAD_BYTES { return Err(SessionStoreError::InvalidPayload("stored payload allowance exceeds host ceiling".into())); }
    Ok(capacity)
}

pub(super) async fn native_budget_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, operation: &OperationId) -> Result<NativeExecutionBudget, SessionStoreError> {
    let encoded: Option<String> = sqlx::query_scalar("SELECT native_budget_json FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&mut **tx).await?;
    let mut budget: NativeExecutionBudget = encoded.map(|value| serde_json::from_str(&value)).transpose()?.unwrap_or_default();
    budget.session_payload_bytes = budget.session_payload_bytes.max(native_payload_capacity_tx(tx,session).await?);
    budget.validate().map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
    Ok(budget)
}

impl AgentSessionStore {
    pub async fn native_pause_state(&self, session: &AgentSessionId, operation: &OperationId) -> Result<Option<NativePauseState>, SessionStoreError> {
        let value: Option<String> = sqlx::query_scalar("SELECT native_pause_json FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&self.pool).await?;
        value.map(|value| serde_json::from_str(&value).map_err(SessionStoreError::from)).transpose()
    }

    pub async fn native_pause_requested(&self, lease: &NativeExecutionLease) -> Result<bool, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        native_execution::check_native_lease_tx(&mut tx, lease, false).await?;
        Ok(sqlx::query_scalar("SELECT native_pause_requested_json IS NOT NULL FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?)
    }

    pub async fn native_execution_budget(&self, lease: &NativeExecutionLease) -> Result<NativeExecutionBudget, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        native_execution::check_native_lease_tx(&mut tx, lease, false).await?;
        native_budget_tx(&mut tx, lease.session_id(), lease.operation_id()).await
    }

    pub async fn request_native_pause(&self, owner: &PrincipalRef, session: &AgentSessionId,
        operation: &OperationId, key: &str, reason: &str) -> Result<SessionEventAck, SessionStoreError> {
        if reason.trim().is_empty() || reason.len() > 1024 { return Err(SessionStoreError::InvalidEvent("pause reason is not bounded".into())); }
        let id = native_key("pause-request", session, key)?;
        let digest = digest_payload(&json!({"operation":operation,"reason":reason}))?;
        let mut tx = self.begin_write_transaction().await?;
        let live = live_session_by_id_tx(&mut tx, session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        if let Some(row) = event_by_event_id_tx(&mut tx, &id).await? {
            let event = event_from_row(row)?;
            if payload_value_for_event_tx(&mut tx, &event).await?.get("request_digest").and_then(Value::as_str) != Some(digest.as_ref()) {
                return Err(SessionStoreError::IdempotencyConflict("pause request changed".into()));
            }
            return Ok(event_ack(&event));
        }
        let head = head_by_id_tx(&mut tx, session.as_ref()).await?;
        if head.active_turn_id.as_deref() != Some(operation.as_ref()) || !matches!(head.status.as_str(), "running" | "paused") {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let started: String = sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=? AND state='running'")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&mut *tx).await?;
        let event = native_control_event(session, operation, id, "turn/pause-requested", started.into(),
            json!({"request_digest":digest,"reason":reason,"owner_digest":digest_payload(owner)?}));
        let ack = required_ack(self.append_event_tx(&mut tx, &event, None).await?)?;
        tx.commit().await?;
        Ok(ack)
    }

    pub async fn pause_native_execution(&self, lease: &NativeExecutionLease, reason: &str, cleanup_proven: bool) -> Result<NativePauseState, SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        if native_execution::check_native_identity_tx(&mut tx, lease).await? != "running" { return Err(SessionStoreError::ExecutionFenced); }
        let state = self.pause_native_execution_tx(&mut tx, lease.session_id(), lease.operation_id(), lease.fence(), reason, cleanup_proven).await?;
        tx.commit().await?;
        Ok(state)
    }

    pub(super) async fn pause_native_execution_tx(&self, tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
        operation: &OperationId, fence: u64, reason: &str, cleanup_proven: bool) -> Result<NativePauseState, SessionStoreError> {
        if reason.is_empty() || reason.len() > 128 || !reason.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
            return Err(SessionStoreError::InvalidEvent("pause requires a bounded machine reason".into()));
        }
        let row: (String, i64, Option<String>, i64, i64) = sqlx::query_as(
            "SELECT started_event_id,native_checkpoint_revision,native_checkpoint_digest,native_pause_revision,execution_fence FROM agent_turns WHERE session_id=? AND operation_id=? AND state='running' AND terminal_event_id IS NULL")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_one(&mut **tx).await?;
        if as_u64(row.4,"execution fence")? != fence { return Err(SessionStoreError::ExecutionFenced); }
        let head = head_by_id_tx(tx, session.as_ref()).await?;
        if head.active_turn_id.as_deref().is_some_and(|id| id != operation.as_ref()) { return Err(SessionStoreError::ExecutionFenced); }
        let state = NativePauseState {
            revision: as_u64(row.3,"pause revision")?.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("pause revision exhausted".into()))?,
            reason: reason.into(), checkpoint_revision: as_u64(row.1,"checkpoint revision")?, checkpoint_digest: row.2.map(Into::into),
            execution_fence: fence.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("execution fence exhausted".into()))?,
            cleanup_proven, paused_at_ms: wall_clock_now_ms(),
        };
        let id = format!("native-paused:{}:{}:{}",session.as_ref(),operation.as_ref(),state.revision);
        self.append_event_tx(tx, &native_control_event(session, operation, id, "turn/paused", row.0.into(),
            json!({"pause":state})), None).await?;
        Ok(state)
    }

    pub async fn native_resume_receipt(&self, owner: &PrincipalRef, session: &AgentSessionId, request: &NativeResumeRequest)
        -> Result<Option<NativeResumeReceipt>, SessionStoreError> {
        let id = native_key("resume", session, &request.idempotency_key)?;
        let mut tx = self.pool.begin().await?;
        let live = live_session_by_id_tx(&mut tx, session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        let Some(row) = event_by_event_id_tx(&mut tx, &id).await? else { return Ok(None); };
        let event = event_from_row(row)?;
        let payload = payload_value_for_event_tx(&mut tx, &event).await?;
        if event.kind.0 != "turn/resume-authorized" || payload.get("request_digest").and_then(Value::as_str) != Some(digest_payload(request)?.as_ref()) {
            return Err(SessionStoreError::IdempotencyConflict("resume authorization changed".into()));
        }
        Ok(Some(NativeResumeReceipt { operation_id: request.operation_id.clone(), authorization_event_id: event.event_id, seq: event.seq, duplicate: true }))
    }

    pub async fn commit_native_resume(&self, owner: &PrincipalRef, session: &AgentSessionId, request: &NativeResumeRequest,
        prepared: NativeResumePreparation) -> Result<NativeResumeReceipt, SessionStoreError> {
        if let Some(receipt) = self.native_resume_receipt(owner, session, request).await? { return Ok(receipt); }
        let id = native_key("resume", session, &request.idempotency_key)?;
        let mut tx = self.begin_write_transaction().await?;
        let live = live_session_by_id_tx(&mut tx, session.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        if let Some(row) = event_by_event_id_tx(&mut tx,&id).await? {
            let event = event_from_row(row)?;
            if event.kind.0 != "turn/resume-authorized" || payload_value_for_event_tx(&mut tx,&event).await?.get("request_digest").and_then(Value::as_str) != Some(digest_payload(request)?.as_ref()) {
                return Err(SessionStoreError::IdempotencyConflict("concurrent resume authorization changed".into()));
            }
            return Ok(NativeResumeReceipt { operation_id:request.operation_id.clone(),authorization_event_id:event.event_id,seq:event.seq,duplicate:true });
        }
        if &live.owner_ref != owner || live.agent_binding.resolved_snapshot_ref != prepared.snapshot
            || head.active_set_generation != prepared.active_set_generation || head.status != "paused"
            || head.active_turn_id.as_deref() != Some(request.operation_id.as_ref()) || head.last_seq != prepared.expected_head_seq {
            return Err(SessionStoreError::Conflict("resume authority or prepared event boundary changed".into()));
        }
        let row: (String, String, i64, String, i64) = sqlx::query_as("SELECT started_event_id,native_pause_json,native_checkpoint_revision,native_checkpoint_digest,execution_fence FROM agent_turns WHERE session_id=? AND operation_id=? AND state='running' AND terminal_event_id IS NULL")
            .bind(session.as_ref()).bind(request.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        let pause: NativePauseState = serde_json::from_str(&row.1)?;
        if pause.revision != request.expected_pause_revision || as_u64(row.2,"checkpoint revision")? != request.expected_checkpoint_revision
            || row.3 != request.expected_checkpoint_digest.as_ref() || as_u64(row.4,"execution fence")? != prepared.expected_fence {
            return Err(SessionStoreError::Conflict("resume request targets an outdated pause/checkpoint".into()));
        }
        if !pause.cleanup_proven { request.cleanup_attestation.as_ref().ok_or_else(|| SessionStoreError::Conflict("resource owner cleanup remains unproven".into()))?.validate()?; }
        let unsettled: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if unsettled { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
        let budget = native_budget_tx(&mut tx, session, &request.operation_id).await?.increased(&request.budget)
            .map_err(|error| SessionStoreError::InvalidPayload(error.into()))?;
        let bytes = canonical_json_bytes(&prepared.checkpoint_state.0)?;
        if prepared.checkpoint_state.0.get("turn_operation_id").and_then(Value::as_str) != Some(request.operation_id.as_ref())
            || prepared.checkpoint_state.0.pointer("/binding/agent_session_id").and_then(Value::as_str) != Some(session.as_ref())
            || prepared.checkpoint_state.0.get("active_set_generation").and_then(Value::as_u64) != Some(prepared.active_set_generation)
            || prepared.checkpoint_state.0.pointer("/binding/resolved_snapshot_ref/snapshot_digest").and_then(Value::as_str) != Some(prepared.snapshot.snapshot_digest.as_ref()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        if bytes.len() > MAX_NATIVE_CHECKPOINT_BYTES || prepared.observations.len() > 144 {
            return Err(SessionStoreError::InvalidPayload("prepared resume exceeds its bounded state contract".into()));
        }
        let mut sequence: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/progress-recorded'")
            .bind(session.as_ref()).bind(request.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        for observation in prepared.observations {
            if !matches!(observation.0.get("event").and_then(Value::as_str), Some("tool_outcome_reconciled" | "owner_outcome_reconciled" | "execution_tail_reconciled")) {
                return Err(SessionStoreError::InvalidEvent("resume may append observations, never tool admissions".into()));
            }
            sequence += 1;
            let key = format!("runtime-progress:{}:{}:{sequence}",session.as_ref(),request.operation_id.as_ref());
            self.append_event_tx(&mut tx, &native_control_event(session,&request.operation_id,key,"runtime/progress-recorded",row.0.clone().into(),
                json!({"runtime_binding_id":format!("nomi:{}",session.as_ref()),"producer_seq":sequence,"event":observation.0})),None).await?;
            if observation.0.get("event").and_then(Value::as_str) == Some("tool_outcome_reconciled") {
                self.project_reconciled_invocation_tx(&mut tx,session,&request.operation_id,&row.0,&observation.0,pause.revision).await?;
            }
        }
        sequence += 1;
        let revision = request.expected_checkpoint_revision.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("checkpoint revision exhausted".into()))?;
        let digest = digest_bytes(&bytes);
        let key = format!("runtime-progress:{}:{}:{sequence}",session.as_ref(),request.operation_id.as_ref());
        let checkpoint_ack = required_ack(self.append_event_tx(&mut tx,&native_control_event(session,&request.operation_id,key,"runtime/progress-recorded",row.0.clone().into(),
            json!({"runtime_binding_id":format!("nomi:{}",session.as_ref()),"producer_seq":sequence,"event":{"event":"execution_checkpoint_saved","step":prepared.checkpoint_state.0.get("model_steps"),"revision":revision,"digest":digest}})),None).await?)?;
        sqlx::query("UPDATE agent_turns SET native_checkpoint_json=?,native_checkpoint_digest=?,native_checkpoint_revision=?,native_checkpoint_seq=? WHERE session_id=? AND operation_id=?")
            .bind(String::from_utf8(bytes).map_err(|_| SessionStoreError::InvalidPayload("checkpoint UTF-8".into()))?).bind(digest.as_ref())
            .bind(as_i64(revision,"checkpoint revision")?).bind(as_i64(checkpoint_ack.seq,"checkpoint cursor")?)
            .bind(session.as_ref()).bind(request.operation_id.as_ref()).execute(&mut *tx).await?;
        let pause_cause = format!("native-paused:{}:{}:{}",session.as_ref(),request.operation_id.as_ref(),pause.revision);
        let ack = required_ack(self.append_event_tx(&mut tx,&native_control_event(session,&request.operation_id,id,"turn/resume-authorized",pause_cause.into(),
            json!({"request_digest":digest_payload(request)?,"pause_revision":pause.revision,"budget":budget,"increase":request.budget,
                "owner_digest":digest_payload(owner)?,"cleanup_attestation":request.cleanup_attestation})),None).await?)?;
        tx.commit().await?;
        Ok(NativeResumeReceipt { operation_id: request.operation_id.clone(), authorization_event_id: ack.event_id, seq: ack.seq, duplicate: false })
    }

    async fn project_reconciled_invocation_tx(&self,tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,operation:&OperationId,
        started:&str,observation:&Value,pause_revision:u64) -> Result<(),SessionStoreError> {
        let Some(call_id) = observation.pointer("/result/call_id").and_then(Value::as_str) else { return Err(SessionStoreError::InvalidEvent("reconciled result has no call identity".into())); };
        if call_id.starts_with("agent-instructions:") { return Ok(()); }
        let rows: Vec<(String,String)> = sqlx::query_as("SELECT event_id,correlation_id FROM agent_events WHERE session_id=? AND kind='tool/call-started' AND causation_event_id=? AND json_extract(inline_json,'$.call_id')=? ORDER BY seq LIMIT 2")
            .bind(session.as_ref()).bind(started).bind(call_id).fetch_all(&mut **tx).await?;
        let Some((cause,correlation)) = rows.first() else { return Ok(()); };
        if rows.len() != 1 { return Err(SessionStoreError::InvalidEvent("reconciled display target is ambiguous".into())); }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='tool/result-recorded')")
            .bind(session.as_ref()).bind(correlation).fetch_one(&mut **tx).await?;
        if exists { return Ok(()); }
        let id = format!("native-reconciled-tool:{}:{}:{pause_revision}:{}",session.as_ref(),operation.as_ref(),digest_bytes(call_id.as_bytes()).as_ref());
        let mut event = native_control_event(session,operation,id,"tool/result-recorded",cause.clone().into(),json!({
            "operation_id":observation.get("owner_operation_id"),"call_id":call_id,"output":observation.get("result"),
            "reconciled":true,"reconciliation_source":observation.get("source"),
        }));
        event.semantic_event.correlation_id = correlation.clone().into();
        self.append_event_tx(tx,&event,None).await?;
        Ok(())
    }
}
