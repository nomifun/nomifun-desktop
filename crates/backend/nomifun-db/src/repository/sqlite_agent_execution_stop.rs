//! Scoped cancellation stays in the Execution writer and original event chain.
use super::*;

/// Read only the original delegation facts. A DAG join can depend on several
/// owners and must never acquire ownership through those dependency edges.
async fn delegated_steps_tx(
    tx: &mut Transaction<'_, Sqlite>, execution_id: &str, root: &str,
) -> Result<HashSet<String>, DbError> {
    let events: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT step_id,payload FROM agent_execution_events \
         WHERE execution_id=? AND event_type='plan_changed' \
           AND (actor_attempt_id IS NOT NULL OR json_type(payload,'$.continuation_of_step_id')='text') \
         ORDER BY sequence",
    ).bind(execution_id).fetch_all(&mut **tx).await?;
    let mut ownership: HashMap<String, Vec<String>> = HashMap::new();
    for (caller, payload) in events {
        let value: serde_json::Value = serde_json::from_str(&payload)
            .map_err(|_| conflict("delegation event payload"))?;
        if value.get("added_step_ids").is_none() { continue; }
        let caller = value.get("continuation_of_step_id").and_then(serde_json::Value::as_str).map(str::to_owned)
            .or(caller).ok_or_else(|| conflict("delegation caller Step"))?;
        ownership.entry(caller).or_default().extend(delegation_added_step_ids(&payload)?);
    }
    let mut selected = HashSet::from([root.to_owned()]);
    let mut queue = VecDeque::from([root.to_owned()]);
    while let Some(parent) = queue.pop_front() {
        for child in ownership.get(&parent).into_iter().flatten() {
            if selected.insert(child.clone()) { queue.push_back(child.clone()); }
        }
    }
    Ok(selected)
}

pub(super) async fn cancel_step(
    pool: &SqlitePool, user_id: &str, execution_id: &str,
    expected_execution_version: i64, step_id: &str, expected_step_version: i64,
    scope: AgentExecutionCancellationScope, event: &NewAgentExecutionEvent,
) -> Result<AgentExecutionDetailRows, DbError> {
    if !matches!(&event.actor, AgentExecutionActor::User { user_id: actor } if actor == user_id)
        || event.event_type != AgentExecutionEventKind::StepChanged {
        return Err(conflict("user Step cancellation authority"));
    }
    let now = now_ms();
    let mut tx = pool.begin().await?;
    // Acquire the same SQLite writer as start/delegate/retry BEFORE reading
    // ownership or targets. A concurrent append is either included or fenced.
    let locked = sqlx::query(
        "UPDATE agent_executions SET version=version WHERE execution_id=? AND user_id=? AND deleted_at IS NULL",
    ).bind(execution_id).bind(user_id).execute(&mut *tx).await?;
    if locked.rows_affected() != 1 { return Err(conflict("Agent Execution")); }
    let root: AgentExecutionStepRow = sqlx::query_as(
        "SELECT * FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND superseded_in_revision IS NULL",
    ).bind(execution_id).bind(step_id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| conflict("cancelled Step target"))?;

    // An exact replay acknowledges only its original captured versions/scope;
    // it never refreshes a stale button to select a successor task.
    let previous: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM agent_execution_events WHERE execution_id=? AND step_id=? \
         AND event_type='step_changed' AND json_extract(payload,'$.action')='cancel_step'",
    ).bind(execution_id).bind(step_id).fetch_all(&mut *tx).await?;
    for raw in previous {
        let payload: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| conflict("Step cancellation event"))?;
        if payload.get("expected_execution_version").and_then(serde_json::Value::as_i64) == Some(expected_execution_version)
            && payload.get("expected_step_version").and_then(serde_json::Value::as_i64) == Some(expected_step_version)
            && payload.get("scope").and_then(serde_json::Value::as_str) == Some(scope.as_str()) {
            let detail = load_execution_detail_tx(&mut tx, user_id, execution_id).await?
                .ok_or_else(|| conflict("Agent Execution"))?;
            tx.commit().await?;
            return Ok(detail);
        }
    }
    if root.version != expected_step_version || (scope == AgentExecutionCancellationScope::Task && root.status == "cancelled") {
        return Err(conflict("cancelled Step version"));
    }
    if scope == AgentExecutionCancellationScope::Task && matches!(root.status.as_str(), "completed" | "failed" | "skipped") {
        return Err(conflict("already settled Step"));
    }
    let mut selected = if scope != AgentExecutionCancellationScope::Task { delegated_steps_tx(&mut tx, execution_id, step_id).await? }
        else { HashSet::from([step_id.to_owned()]) };
    if scope == AgentExecutionCancellationScope::DelegatedWork { selected.remove(step_id); }
    // Step identity/version fixes the exact task. Unrelated sibling settlement
    // or delegation can advance the aggregate without invalidating this Stop.
    // The captured aggregate version remains part of exact replay authority.
    if expected_execution_version < 0 { return Err(conflict("Agent Execution cancellation version")); }
    let changed = sqlx::query(
        "UPDATE agent_executions SET version=version+1,updated_at=? \
         WHERE execution_id=? AND user_id=? AND version>=? AND deleted_at IS NULL \
           AND status NOT IN ('completed','completed_with_failures','failed','cancelled')",
    ).bind(now).bind(execution_id).bind(user_id).bind(expected_execution_version).execute(&mut *tx).await?;
    if changed.rows_affected() != 1 { return Err(conflict("Agent Execution cancellation version")); }
    let mut cancelled_step_ids: Vec<String> = selected.into_iter().collect();
    cancelled_step_ids.sort();
    for selected_step in &cancelled_step_ids {
        // Preserve already committed results; the scope event still fences
        // retries/derivation from an already completed ownership root.
        let stopped = sqlx::query("UPDATE agent_execution_steps SET status='cancelled',dispatch_after=NULL,version=version+1,updated_at=? \
            WHERE execution_id=? AND step_id=? AND superseded_in_revision IS NULL \
              AND status NOT IN ('completed','failed','skipped','cancelled')")
            .bind(now).bind(execution_id).bind(selected_step).execute(&mut *tx).await?;
        if stopped.rows_affected() != 0 {
            requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, selected_step, now).await?;
        }
        sqlx::query("UPDATE agent_execution_attempts SET status='cancelled',question=NULL,finished_at=?,version=version+1,updated_at=? \
            WHERE execution_id=? AND step_id=? AND status IN ('queued','running','waiting_input')")
            .bind(now).bind(now).bind(execution_id).bind(selected_step).execute(&mut *tx).await?;
        sqlx::query("UPDATE conversation_execution_links SET active=0,updated_at=? \
            WHERE execution_id=? AND step_id=? AND relation IN ('attempt','automation') AND active=1")
            .bind(now).bind(execution_id).bind(selected_step).execute(&mut *tx).await?;
    }
    let mut event = scoped_event(event, step_id, None);
    event.payload = serde_json::json!({"action":"cancel_step","scope":scope,
        "expected_execution_version":expected_execution_version,"expected_step_version":expected_step_version,
        "cancelled_step_ids":cancelled_step_ids}).to_string();
    append_event_tx(&mut tx, execution_id, &event, now).await?;
    let detail = load_execution_detail_tx(&mut tx, user_id, execution_id).await?
        .ok_or_else(|| conflict("Agent Execution"))?;
    tx.commit().await?;
    Ok(detail)
}

/// A stopped range cannot be reopened by a parent retry, replan, loop reset or
/// another delegation. This is a query over the original cancellation event,
/// never a second control ledger.
pub(super) async fn ensure_not_stopped_tx(
    tx: &mut Transaction<'_, Sqlite>, execution_id: &str, step_id: &str,
) -> Result<(), DbError> {
    if is_stopped_tx(tx, execution_id, step_id).await? { return Err(conflict("persistently stopped Step")); }
    Ok(())
}

/// A delegation-only fence preserves the parent's business execution and
/// retries while preventing any Attempt of that same Step from creating work.
pub(super) async fn ensure_delegation_not_stopped_tx(
    tx: &mut Transaction<'_, Sqlite>, execution_id: &str, step_id: &str,
) -> Result<(), DbError> {
    let stopped: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_execution_events \
         WHERE execution_id=? AND step_id=? AND event_type='step_changed' \
           AND json_extract(payload,'$.action')='cancel_step' \
           AND json_extract(payload,'$.scope') IN ('delegated_work','task_and_delegated'))",
    ).bind(execution_id).bind(step_id).fetch_one(&mut **tx).await?;
    if stopped != 0 { return Err(conflict("persistently stopped delegation root")); }
    Ok(())
}

pub(super) async fn is_stopped_tx(
    tx: &mut Transaction<'_, Sqlite>, execution_id: &str, step_id: &str,
) -> Result<bool, DbError> {
    let stopped: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_execution_events event,json_each(event.payload,'$.cancelled_step_ids') selected \
         WHERE event.execution_id=? AND event.event_type='step_changed' \
           AND json_extract(event.payload,'$.action')='cancel_step' AND selected.value=?)",
    ).bind(execution_id).bind(step_id).fetch_one(&mut **tx).await?;
    Ok(stopped != 0)
}
