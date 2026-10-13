//! Exact committed tool-call identity and its current native execution scope.
//! These are read-only views of canonical facts, never a second authority ledger.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedToolCallCausation {
    pub event_id: EventId,
    pub seq: u64,
}

/// Only the Store constructs this scope from an active native lease. A native
/// execution's first fence is legitimately zero; its generation is always a
/// committed event sequence. No timestamp fallback is permitted here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeExecutionScope {
    generation: u64,
    fence: u64,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
}

impl NativeExecutionScope {
    pub fn generation(&self) -> u64 { self.generation }
    pub fn fence(&self) -> u64 { self.fence }
    pub fn resolved_snapshot_ref(&self) -> &nomifun_agent_contracts::ResolvedSnapshotRef { &self.snapshot }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedNativeToolCallCausation {
    pub tool_call: CommittedToolCallCausation,
    pub execution_scope: NativeExecutionScope,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeClaimPayload {
    operation_id: OperationId,
    execution_fence: u64,
    holder_digest: DigestHex,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeToolCallPayload {
    operation_id: OperationId,
    capability_id: CapabilityId,
    action_id: ActionId,
    call_id: nomifun_agent_contracts::chat_model::ToolCallId,
    #[serde(rename = "name", default)]
    _name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeProgressPayload {
    producer_seq: u64,
    event: Value,
}

#[derive(serde::Deserialize)]
#[serde(tag = "event")]
enum NativeSettlementIdentity {
    #[serde(rename = "host_tool_settled")]
    HostToolSettled { operation_id: OperationId },
}

impl AgentSessionStore {
    /// The exact unique predecessor of this invocation, including its durable
    /// sequence. It retains the existing non-native Effect causation contract.
    pub async fn tool_call_causation(
        &self,
        session_id: &AgentSessionId,
        turn_id: &OperationId,
        operation_id: &OperationId,
        capability_module: &CapabilityId,
        action_id: &ActionId,
    ) -> Result<CommittedToolCallCausation, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        tool_call_causation_tx(&mut tx, session_id, turn_id, operation_id, capability_module, action_id).await
    }

    /// One SQLite snapshot proves the exact tool call and the current native
    /// execution that dispatched it. Historical calls cannot borrow a resumed
    /// execution's scope, and expired/unowned/paused turns grant no authority.
    pub async fn native_tool_call_causation(
        &self,
        owner: &PrincipalRef,
        session_id: &AgentSessionId,
        turn_id: &OperationId,
        operation_id: &OperationId,
        capability_module: &CapabilityId,
        action_id: &ActionId,
    ) -> Result<CommittedNativeToolCallCausation, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let session = require_live_session_tx(&mut tx, session_id.as_ref()).await?;
        let head = head_by_id_tx(&mut tx, session_id.as_ref()).await?;
        if session.owner_ref != *owner || head.status != "running"
            || head.active_turn_id.as_deref() != Some(turn_id.as_ref()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let row: Option<(i64, i64, String, i64, String)> = sqlx::query_as(
            "SELECT t.execution_generation, t.execution_fence, t.execution_owner, t.execution_lease_until, t.started_event_id \
             FROM agent_turns t \
             WHERE t.session_id=? AND t.turn_id=? AND t.operation_id=? \
              AND t.state='running' AND t.terminal_event_id IS NULL AND t.native_pause_json IS NULL \
              AND t.execution_generation>0 AND t.execution_fence>=0 AND t.execution_owner IS NOT NULL",
        )
        .bind(session_id.as_ref()).bind(turn_id.as_ref()).bind(turn_id.as_ref())
        .fetch_optional(&mut *tx).await?;
        let (generation, fence, holder, lease_until, started_event_id) = row.ok_or(SessionStoreError::ExecutionFenced)?;
        if holder.is_empty() || holder.len() > 128 || holder.trim() != holder || lease_until <= wall_clock_now_ms() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let execution_scope = NativeExecutionScope {
            generation: as_u64(generation, "execution generation")?,
            fence: as_u64(fence, "execution fence")?,
            snapshot: session.agent_binding.resolved_snapshot_ref.clone(),
        };
        // Resolve the exact existing claim that created this fence. Its
        // identity is authored by the native writer; this does not author a
        // new claim or infer one from a timestamp/head cursor.
        let claim_id = format!("native-execution:{}:{}:{}", session_id.as_ref(), turn_id.as_ref(), fence);
        let claim = event_by_event_id_tx(&mut tx, &claim_id).await?
            .map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        if claim.agent_session_id != *session_id || claim.kind.0 != "runtime/execution-claimed"
            || claim.kind_version != 1 || claim.producer_id.as_ref() != "runtime_supervisor"
            || claim.correlation_id.as_ref() != turn_id.as_ref()
            || claim.causation_event_id.as_ref().map(EventId::as_ref) != Some(started_event_id.as_str()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let SessionEventPayloadRef::InlineJson(payload) = &claim.payload else {
            return Err(SessionStoreError::ExecutionFenced);
        };
        let payload: NativeClaimPayload = serde_json::from_value(payload.0.clone())
            .map_err(|_| SessionStoreError::ExecutionFenced)?;
        if payload.operation_id != *turn_id || payload.execution_fence != execution_scope.fence
            || payload.holder_digest != digest_bytes(holder.as_bytes()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let started = event_by_event_id_tx(&mut tx, &started_event_id).await?
            .map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        if started.agent_session_id != *session_id || started.kind.0 != "turn/started"
            || started.kind_version != 1 || started.correlation_id.as_ref() != turn_id.as_ref()
            || claim.seq <= started.seq {
            return Err(SessionStoreError::ExecutionFenced);
        }
        if execution_scope.fence == 0 {
            if execution_scope.generation != started.seq {
                return Err(SessionStoreError::ExecutionFenced);
            }
        } else if execution_scope.generation != claim.seq {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let tool_call = tool_call_causation_tx(
            &mut tx, session_id, turn_id, operation_id, capability_module, action_id,
        ).await?;
        if tool_call.seq <= execution_scope.generation || tool_call.seq <= claim.seq {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let native_call = event_by_event_id_tx(&mut tx, tool_call.event_id.as_ref()).await?
            .map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        if native_call.agent_session_id != *session_id || native_call.kind.0 != "tool/call-started"
            || native_call.kind_version != 1 || native_call.producer_id.as_ref() != "runtime_supervisor"
            || native_call.seq != tool_call.seq
            || native_call.causation_event_id.as_ref().map(EventId::as_ref) != Some(started_event_id.as_str()) {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let SessionEventPayloadRef::InlineJson(payload) = &native_call.payload else {
            return Err(SessionStoreError::ExecutionFenced);
        };
        let payload: NativeToolCallPayload = serde_json::from_value(payload.0.clone())
            .map_err(|_| SessionStoreError::ExecutionFenced)?;
        // The call ID follows the existing ChatToolCall natural-key contract.
        // Correlation remains the tool message identity, not the Turn ID.
        if payload.operation_id != *operation_id || payload.capability_id != *capability_module
            || payload.action_id != *action_id || payload.call_id.as_ref().is_empty()
            || payload.call_id.as_ref().trim() != payload.call_id.as_ref() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let completed: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_events \
             WHERE session_id=? AND kind='tool/result-recorded' AND causation_event_id=?)",
        )
        .bind(session_id.as_ref()).bind(tool_call.event_id.as_ref())
        .fetch_one(&mut *tx).await?;
        if completed != 0 {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let root = started.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
        if native_tool_settlement_exists_tx(
            &mut tx, session_id, turn_id, operation_id, tool_call.seq, root,
        ).await? {
            return Err(SessionStoreError::ExecutionFenced);
        }
        Ok(CommittedNativeToolCallCausation { tool_call, execution_scope })
    }
}

async fn native_tool_settlement_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &AgentSessionId,
    turn_id: &OperationId,
    operation_id: &OperationId,
    after_seq: u64,
    root: &EventId,
) -> Result<bool, SessionStoreError> {
    // Journal settlement is committed before the separate tool-result fact.
    // If its projection failed, the exact native settlement still prevents a
    // completed dispatch from being used to mint another observation lease.
    let rows = sqlx::query_as::<_, StoredEventRow>(
        "SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
         FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND kind_version=1 \
          AND producer_id='runtime_supervisor' AND correlation_id=? AND causation_event_id=? AND seq>? \
          AND (json_extract(inline_json, '$.event.event')='host_tool_settled' OR payload_id IS NOT NULL) \
         ORDER BY seq",
    )
    .bind(session_id.as_ref()).bind(turn_id.as_ref()).bind(root.as_ref())
    .bind(as_i64(after_seq, "tool call sequence")?).fetch_all(&mut **tx).await?;
    for row in rows {
        let event = event_from_row(row)?;
        let value = payload_value_for_event_tx(tx, &event).await?;
        if value.pointer("/event/event").and_then(Value::as_str) != Some("host_tool_settled") {
            continue;
        }
        let progress: NativeProgressPayload = serde_json::from_value(value)
            .map_err(|_| SessionStoreError::ExecutionFenced)?;
        if progress.producer_seq == 0 {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let NativeSettlementIdentity::HostToolSettled { operation_id: settled } =
            serde_json::from_value(progress.event).map_err(|_| SessionStoreError::ExecutionFenced)?;
        if settled == *operation_id {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn tool_call_causation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &AgentSessionId,
    turn_id: &OperationId,
    operation_id: &OperationId,
    capability_module: &CapabilityId,
    action_id: &ActionId,
) -> Result<CommittedToolCallCausation, SessionStoreError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT e.event_id, e.seq FROM agent_events e \
         JOIN agent_turns t ON t.session_id=e.session_id \
          AND t.turn_id=? AND t.started_event_id=e.causation_event_id \
          AND t.state IN ('accepted','running') \
         WHERE e.session_id=? AND e.kind='tool/call-started' \
          AND json_extract(e.inline_json, '$.operation_id')=? \
          AND json_extract(e.inline_json, '$.capability_id')=? \
          AND json_extract(e.inline_json, '$.action_id')=? \
         ORDER BY e.seq LIMIT 2",
    )
    .bind(turn_id.as_ref()).bind(session_id.as_ref()).bind(operation_id.as_ref())
    .bind(capability_module.as_ref()).bind(action_id.as_ref())
    .fetch_all(&mut **tx).await?;
    match rows.as_slice() {
        [(event_id, seq)] => Ok(CommittedToolCallCausation {
            event_id: event_id.clone().into(), seq: as_u64(*seq, "tool call sequence")?,
        }),
        [] => Err(SessionStoreError::InvalidEvent(
            "effect requires its exact committed tool/call-started predecessor".into(),
        )),
        _ => Err(SessionStoreError::InvalidEvent(
            "effect tool/call-started predecessor is ambiguous".into(),
        )),
    }
}
