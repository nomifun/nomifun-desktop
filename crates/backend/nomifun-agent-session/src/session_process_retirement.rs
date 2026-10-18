//! Retire native handles only from the original first-party launch and a
//! committed terminal owner result. Model prose and in-memory exit flags are
//! never cleanup evidence. No additional lifecycle fact is needed.
use super::*;
use nomifun_agent_contracts::{NativeRetainedProcess, NativeSessionProcessRetention};
use nomifun_engine_core::{EngineProcessPoll, EngineToolResult};

#[derive(Clone)]
struct Dispatch {
    call: String,
    action: String,
}

#[derive(sqlx::FromRow)]
struct ProcessSettlement {
    #[sqlx(flatten)]
    event: StoredEventRow,
    process_call: String,
    process_action: String,
    process_operation: String,
}

fn process_observation(value: &Value, dispatch: &Dispatch) -> Option<(String, EngineProcessPoll)> {
    if value.get("error").is_some_and(|error| !error.is_null()) { return None; }
    let result: EngineToolResult = serde_json::from_value(value.get("result")?.clone()).ok()?;
    if result.call_id.as_ref() != dispatch.call || result.validate_for(&result.call_id).is_err() { return None; }
    let mut output: Value = serde_json::from_str(&result.output_text()).ok()?;
    let id = output.get("process_id")?.as_str()?.to_owned();
    let object = output.as_object_mut()?;
    for field in ["process_id", "success", "component_identity", "schema", "operation", "control_applied", "code", "message"] {
        object.remove(field);
    }
    Some((id, serde_json::from_value(output).ok()?))
}

fn reaped(poll: &EngineProcessPoll) -> bool {
    match poll {
        EngineProcessPoll::Exited { cleanup, .. } | EngineProcessPoll::Cancelled { cleanup, .. }
        | EngineProcessPoll::TimedOut { cleanup, .. } => cleanup.reaped && cleanup.errors.is_empty(),
        // A lost owner may have reaped its tree without knowing its outcome.
        // Retiring that handle must not erase the outstanding reconciliation.
        _ => false,
    }
}

/// The exclusive upper cursor keeps a historical release independent of later
/// observations. Original launch tuples must already have been validated by
/// their retention reader, or supplied by the exact live native owner.
pub(super) async fn confirmed_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
    processes: &BTreeMap<String, NativeRetainedProcess>, through: u64,
) -> Result<BTreeSet<String>, SessionStoreError> {
    if processes.is_empty() { return Ok(BTreeSet::new()); }
    let creators = processes.values().map(|process| json!({"turn":process.creator_turn_id,"operation":process.creator_call_id})).collect::<Vec<_>>();
    let creators=serde_json::to_string(&creators)?;
    let creation_floor: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(started.seq) FROM agent_turns turn JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id \
         JOIN json_each(?) creator ON turn.operation_id=json_extract(creator.value,'$.turn') WHERE turn.session_id=?")
        .bind(&creators).bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let Some(creation_floor)=creation_floor else { return Ok(BTreeSet::new()); };
    // The original launch dispatch is a tighter floor than the Turn start:
    // hundreds of sequential short commands in one Turn do not repeatedly
    // hydrate results preceding the candidate's actual creation.
    let floor: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(e.seq) FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
         JOIN json_each(?) creator ON e.correlation_id=json_extract(creator.value,'$.turn') \
         AND COALESCE(json_extract(e.inline_json,'$.event.dispatch.operation_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.dispatch.operation_id'))=json_extract(creator.value,'$.operation') \
         WHERE e.session_id=? AND e.seq>=? AND e.seq<? AND e.kind='runtime/progress-recorded' AND e.kind_version=1 AND e.producer_id='runtime_supervisor' \
         AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_tool_dispatch' \
         AND COALESCE(json_extract(e.inline_json,'$.event.dispatch.capability_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.dispatch.capability_id'))='workspace.process' \
         AND COALESCE(json_extract(e.inline_json,'$.event.dispatch.action_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.dispatch.action_id'))='workspace.process/start'")
        .bind(&creators).bind(session.as_ref()).bind(creation_floor).bind(as_i64(through, "process retirement cursor")?).fetch_one(&mut **tx).await?;
    let Some(mut cursor) = floor else { return Ok(BTreeSet::new()); };
    cursor = cursor.saturating_sub(1);
    let mut launched = BTreeSet::new();
    let mut completed = BTreeSet::new();
    loop {
    // Filter the formal first-party dispatch before hydrating any result.
    // File/media/MCP payloads never enter this bounded reader.
    let rows = sqlx::query_as::<_, ProcessSettlement>(
        "SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id, \
         COALESCE(json_extract(d.inline_json,'$.event.dispatch.call_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.call_id')) AS process_call, \
         COALESCE(json_extract(d.inline_json,'$.event.dispatch.action_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.action_id')) AS process_action, \
         COALESCE(json_extract(d.inline_json,'$.event.dispatch.operation_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.operation_id')) AS process_operation \
         FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
         JOIN agent_events d ON d.session_id=e.session_id AND d.correlation_id=e.correlation_id AND d.seq<e.seq \
         AND d.kind='runtime/progress-recorded' AND d.kind_version=1 AND d.producer_id='runtime_supervisor' \
         LEFT JOIN agent_payloads dp ON dp.payload_id=d.payload_id AND dp.session_id=d.session_id \
         WHERE e.session_id=? AND e.seq>? AND e.seq<? AND e.kind='runtime/progress-recorded' AND e.kind_version=1 AND e.producer_id='runtime_supervisor' \
         AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='host_tool_settled' \
         AND COALESCE(json_extract(d.inline_json,'$.event.event'),json_extract(CAST(dp.body AS TEXT),'$.value.event.event'))='host_tool_dispatch' \
         AND COALESCE(json_extract(d.inline_json,'$.event.dispatch.capability_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.capability_id'))='workspace.process' \
         AND COALESCE(json_extract(d.inline_json,'$.event.dispatch.action_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.action_id')) IN \
             ('workspace.process/start','workspace.process/poll','workspace.process/input','workspace.process/close_stdin','workspace.process/resize','workspace.process/cancel') \
         AND COALESCE(json_extract(e.inline_json,'$.event.operation_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.operation_id'))= \
             COALESCE(json_extract(d.inline_json,'$.event.dispatch.operation_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.operation_id')) \
         AND COALESCE(json_extract(e.inline_json,'$.event.call_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.call_id'))= \
             COALESCE(json_extract(d.inline_json,'$.event.dispatch.call_id'),json_extract(CAST(dp.body AS TEXT),'$.value.event.dispatch.call_id')) ORDER BY e.seq LIMIT 64")
        .bind(session.as_ref()).bind(cursor).bind(as_i64(through, "process retirement cursor")?).fetch_all(&mut **tx).await?;
    if rows.is_empty() { break; }
    for row in rows {
        cursor = cursor.max(row.event.seq);
        let event = event_from_row(row.event)?;
        let payload = payload_value_for_event_tx(tx, &event).await?;
        let Some(native) = payload.get("event") else { continue; };
        let turn = event.correlation_id.as_ref();
        let dispatch=Dispatch {call:row.process_call,action:row.process_action};
        let operation=row.process_operation;
                if native.get("call_id").and_then(Value::as_str) != Some(dispatch.call.as_str()) { continue; }
                let Some((id, poll)) = process_observation(native, &dispatch) else { continue; };
                let Some(original) = processes.get(&id) else { continue; };
                if dispatch.action == "workspace.process/start" {
                    if original.creator_turn_id.as_ref() != turn || original.creator_call_id.as_ref() != operation.as_str() { continue; }
                    if matches!(&poll, EngineProcessPoll::Running { pid, .. } | EngineProcessPoll::Lost { pid, .. } if *pid != original.pid) { continue; }
                    launched.insert(id.clone());
                }
                if launched.contains(&id) && reaped(&poll) { completed.insert(id); }
    }
    if completed.len()==processes.len() { break; }
    }
    Ok(completed)
}

impl AgentSessionStore {
    /// Read-only proof for dropping an exact live owner's already-terminal
    /// native handles. This neither cancels work nor authorizes another launch.
    pub async fn confirmed_reaped_session_processes(&self, retention: &NativeSessionProcessRetention)
        -> Result<BTreeSet<String>, SessionStoreError> {
        retention.validate().map_err(SessionStoreError::InvalidPayload)?;
        let mut tx = self.pool.begin().await?;
        let live = live_from_row(session_row_by_id_tx(&mut tx, retention.agent_session_id.as_ref()).await?)?;
        if live.owner_ref != retention.owner_ref { return Err(SessionStoreError::ExecutionFenced); }
        for process in &retention.processes {
            let started: Option<String> = sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
                .bind(retention.agent_session_id.as_ref()).bind(process.creator_turn_id.as_ref()).fetch_optional(&mut *tx).await?;
            let Some(started) = started else { return Err(SessionStoreError::ExecutionFenced); };
            let event = event_by_event_id_tx(&mut tx, &started).await?.map(event_from_row).transpose()?
                .ok_or_else(|| SessionStoreError::InvalidEvent("process creator Turn start is missing".into()))?;
            if payload_value_for_event_tx(&mut tx, &event).await?.get("resolved_snapshot_ref") != Some(&serde_json::to_value(&process.resolved_snapshot_ref)?) {
                return Err(SessionStoreError::ExecutionFenced);
            }
        }
        let processes = retention.processes.iter().map(|process| (process.process_id.clone(), process.clone())).collect();
        let confirmed = confirmed_tx(&mut tx, &retention.agent_session_id, &processes, i64::MAX as u64).await?;
        tx.commit().await?;
        Ok(confirmed)
    }
}
