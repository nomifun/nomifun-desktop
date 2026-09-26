//! Recovery failure is a durable outcome, not a license to replay effects.
//! Quarantine, producer fencing and nonterminal suspension commit together.
use super::*;

pub const NATIVE_RECOVERY_BLOCKED: &str = "NATIVE_RECOVERY_RECONCILIATION_REQUIRED";

#[derive(Clone, Debug, serde::Serialize)]
pub struct NativeExecutionInspection {
    pub operation_id: OperationId,
    pub state: String,
    pub execution_fence: u64,
    pub execution_generation: u64,
    pub lease_until_ms: i64,
    pub producer_lease_live: bool,
    pub checkpoint_revision: u64,
    pub checkpoint_seq: Option<u64>,
    pub checkpoint_digest: Option<String>,
    pub checkpoint_retained: bool,
    pub pending_effects: u64,
    pub unknown_effects: u64,
    pub session_payload_bytes: u64,
    pub recovery_blocked: bool,
    /// Diagnostic only. This response never grants recovery or effect replay.
    pub automatic_replay_authorized: bool,
    pub turn_state: String,
    pub pause: Option<NativePauseState>,
    pub pause_requested: bool,
    pub budget: nomifun_agent_contracts::NativeExecutionBudget,
    pub model_progress: Value,
}

#[derive(sqlx::FromRow)]
struct InspectionRow {
    operation_id: String,
    state: String,
    execution_fence: i64,
    execution_generation: i64,
    execution_lease_until: i64,
    execution_owner: Option<String>,
    native_checkpoint_revision: i64,
    native_checkpoint_seq: Option<i64>,
    native_checkpoint_digest: Option<String>,
    checkpoint_retained: bool,
}

impl AgentSessionStore {
    /// Bounded metadata for the owner/next testing agent; no original input,
    /// checkpoint body, credentials, raw tool output or holder ID is exposed.
    pub async fn inspect_latest_native_execution(&self, owner: &PrincipalRef, session_id: &AgentSessionId)
        -> Result<Option<NativeExecutionInspection>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if &session.owner_ref != owner { return Err(SessionStoreError::Conflict("execution inspection belongs to another owner".into())); }
        let row = sqlx::query_as::<_, InspectionRow>(
            "SELECT t.operation_id,t.state,t.execution_fence,t.execution_generation,t.execution_lease_until,t.execution_owner, \
             t.native_checkpoint_revision,t.native_checkpoint_seq,t.native_checkpoint_digest, \
             (t.native_checkpoint_json IS NOT NULL) AS checkpoint_retained FROM agent_turns t \
             JOIN agent_events e ON e.event_id=t.started_event_id WHERE t.session_id=? ORDER BY e.seq DESC LIMIT 1")
            .bind(session_id.as_ref()).fetch_optional(&mut *tx).await?;
        let Some(row) = row else { return Ok(None); };
        let counts: (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(state='pending'),0),COALESCE(SUM(state='unknown'),0) FROM agent_effects WHERE session_id=? AND turn_id=?")
            .bind(session_id.as_ref()).bind(&row.operation_id).fetch_one(&mut *tx).await?;
        let payload_bytes: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(byte_len),0) FROM agent_payloads WHERE session_id=?")
            .bind(session_id.as_ref()).fetch_one(&mut *tx).await?;
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/execution-recovery-blocked')")
            .bind(session_id.as_ref()).bind(&row.operation_id).fetch_one(&mut *tx).await?;
        let suspension: (Option<String>, bool) = sqlx::query_as("SELECT native_pause_json,native_pause_requested_json IS NOT NULL FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session_id.as_ref()).bind(&row.operation_id).fetch_one(&mut *tx).await?;
        let pause: Option<NativePauseState> = suspension.0.map(|value| serde_json::from_str(&value)).transpose()?;
        let budget = native_pause::native_budget_tx(&mut tx,session_id,&row.operation_id.clone().into()).await?;
        let model: (Option<i64>,Option<i64>,Option<i64>,Option<i64>) = sqlx::query_as("SELECT json_extract(native_checkpoint_json,'$.model_steps'),json_extract(native_checkpoint_json,'$.segments.model_steps_per_segment'),json_extract(native_checkpoint_json,'$.segments.segment'),json_extract(native_checkpoint_json,'$.segments.policy.max_segments') FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session_id.as_ref()).bind(&row.operation_id).fetch_one(&mut *tx).await?;
        let used: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(CAST(json_extract(inline_json,'$.event.step') AS INTEGER)),0) FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='model_step_started'")
            .bind(session_id.as_ref()).bind(&row.operation_id).fetch_one(&mut *tx).await?;
        let model_progress = json!({"used_model_steps":used,"checkpoint_model_steps":model.0,"steps_per_segment":model.1,"segment":model.2,"maximum_segments":model.3});
        Ok(Some(NativeExecutionInspection {
            operation_id: row.operation_id.into(), producer_lease_live: row.state == "running" && row.execution_owner.is_some() && row.execution_lease_until > wall_clock_now_ms(),
            state: if pause.is_some() { "paused".into() } else { row.state.clone() }, turn_state: row.state, pause, pause_requested: suspension.1, budget, model_progress,
            execution_fence: as_u64(row.execution_fence, "execution fence")?,
            execution_generation: as_u64(row.execution_generation, "execution generation")?, lease_until_ms: row.execution_lease_until,
            checkpoint_revision: as_u64(row.native_checkpoint_revision, "checkpoint revision")?,
            checkpoint_seq: row.native_checkpoint_seq.map(|seq| as_u64(seq, "checkpoint cursor")).transpose()?,
            checkpoint_digest: row.native_checkpoint_digest, checkpoint_retained: row.checkpoint_retained,
            pending_effects: as_u64(counts.0, "pending effect count")?, unknown_effects: as_u64(counts.1, "unknown effect count")?,
            session_payload_bytes: as_u64(payload_bytes, "Session payload bytes")?, recovery_blocked: blocked,
            automatic_replay_authorized: false,
        }))
    }

    pub async fn has_native_execution_owner(&self, session_id: &AgentSessionId, operation_id: &OperationId) -> Result<bool, SessionStoreError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=? AND execution_owner IS NOT NULL)")
            .bind(session_id.as_ref()).bind(operation_id.as_ref()).fetch_one(&self.pool).await?)
    }

    /// Release only a recovery claim that never admitted any new native/model
    /// work. A failed loader must not strand its own live 90-second lease.
    pub async fn release_unattached_native_claim(&self, lease: &NativeExecutionLease) -> Result<(), SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        native_execution::check_native_lease_tx(&mut tx, lease, false).await?;
        let activity: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND seq>? \
            AND (kind IN ('runtime/progress-recorded','context/model-visible-applied','tool/call-started','effect/started'))")
            .bind(lease.session_id().as_ref()).bind(as_i64(lease.generation(), "execution generation")?).fetch_one(&mut *tx).await?;
        if activity != 0 { return Err(SessionStoreError::RecoveryRequiresReconciliation); }
        sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id=?")
            .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Used only for an expired orphan selected by startup recovery. Pending
    /// effects become UNKNOWN, never failed/safe-to-retry. The original task
    /// and latest checkpoint remain. Another live/replacement producer wins.
    pub async fn quarantine_native_recovery(&self, owner: &PrincipalRef, session_id: &AgentSessionId,
        operation_id: &OperationId, expected_fence: u64) -> Result<bool, SessionStoreError> {
        let mut tx = self.begin_write_transaction().await?;
        let session = live_session_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if &session.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if !matches!(head.status.as_str(), "running" | "reconciliation") || head.active_turn_id.as_deref() != Some(operation_id.as_ref()) { return Ok(false); }
        let row: (i64, i64, String, i64, Option<String>) = sqlx::query_as(
            "SELECT execution_fence,execution_lease_until,started_event_id,native_checkpoint_revision,execution_owner \
             FROM agent_turns WHERE session_id=? AND operation_id=? AND state='running'")
            .bind(session_id.as_ref()).bind(operation_id.as_ref()).fetch_one(&mut *tx).await?;
        if as_u64(row.0, "execution fence")? != expected_fence { return Err(SessionStoreError::ExecutionFenced); }
        if row.4.is_some() && row.1 > wall_clock_now_ms() { return Err(SessionStoreError::ExecutionLeaseActive); }
        let next_fence = expected_fence.checked_add(1).ok_or_else(|| SessionStoreError::Conflict("execution fence exhausted".into()))?;
        let rows = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id, \
             resource_binding_id,resource_key,input_digest,strategy,state,bounded_observation_json, \
             started_event_id,terminal_event_id,created_at,settled_at FROM agent_effects \
             WHERE session_id=? AND turn_id=? AND state='pending' ORDER BY effect_id")
            .bind(session_id.as_ref()).bind(operation_id.as_ref()).fetch_all(&mut *tx).await?;
        let pending = rows.len();
        for row in rows {
            let effect = effect_from_row(row)?;
            let started = event_by_event_id_tx(&mut tx, effect.started_event_id.as_ref()).await?.ok_or_else(|| SessionStoreError::InvalidEvent("pending effect has no start receipt".into()))?;
            let request = EffectEventRequest {
                agent_session_id: session_id.clone(), effect_id: effect.effect_id.clone(), turn_id: effect.turn_id,
                operation_id: effect.operation_id, owner_domain: effect.owner_domain, capability_module: effect.capability_module,
                action_id: effect.action_id, resource_binding_id: effect.resource_binding_id, resource_key: effect.resource_key,
                input_digest: effect.input_digest, recorded_at: wall_clock_now_ms(),
                event_id: format!("native-recovery-effect:{}:{}:{next_fence}", session_id.as_ref(), effect.effect_id).into(),
                producer_id: "runtime_supervisor".into(), idempotency_key: started.idempotency_key.into(),
                correlation_id: effect.effect_id.into(), strategy: effect.strategy, causation_event_id: Some(effect.started_event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"outcome":"unknown","recovery":"process_restart_external_reconciliation_required"}))),
            };
            self.append_event_tx_with_policy(&mut tx, &effect_append(request, "effect/uncertain")?, None, AppendSessionStatePolicy::EffectSettlement).await?;
        }
        let identity = format!("native-recovery-blocked:{}:{}:{next_fence}", session_id.as_ref(), operation_id.as_ref());
        let diagnostic = SessionEventAppend {
            agent_session_id: session_id.clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: identity.into(), runtime_binding_id: None, runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/execution-recovery-blocked".into()), kind_version: 1,
                correlation_id: operation_id.as_ref().into(), causation_event_id: Some(row.2.clone().into()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"code":NATIVE_RECOVERY_BLOCKED,
                    "execution_fence":next_fence,"checkpoint_revision":row.3,"quarantined_effects":pending,
                    "automatic_replay_allowed":false,"next_action":"owner_reconciliation_required"}))),
            },
        };
        let ack = required_ack(self.append_event_tx(&mut tx, &diagnostic, None).await?)?;
        let _ = ack;
        self.pause_native_execution_tx(&mut tx,session_id,operation_id,expected_fence,NATIVE_RECOVERY_BLOCKED,false).await?;
        tx.commit().await?;
        Ok(true)
    }
}
