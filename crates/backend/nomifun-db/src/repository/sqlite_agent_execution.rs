use std::collections::{HashMap, HashSet, VecDeque};

use nomifun_common::{
    AgentExecutionActor, AgentExecutionCancellationScope, AgentExecutionEventKind, AgentExecutionStatus, ExecutionAttemptStatus,
    ExecutionStepKind, ExecutionStepStatus, MAX_AGENT_EXECUTION_PARALLELISM,
    MAX_AGENT_DELEGATION_DEPTH, MAX_AGENT_EXECUTION_PARTICIPANTS, MAX_AGENT_EXECUTION_STEPS,
    now_ms,
};
use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::{
    AgentExecutionAttemptDetailRow, AgentExecutionAttemptRow, AgentExecutionDetailRows,
    AgentExecutionEventRow, AgentExecutionParticipantRow, AgentExecutionRow,
    AgentExecutionStepDependencyRow, AgentExecutionStepDetailRow, AgentExecutionStepRow,
    ConversationExecutionLinkRow,
    AttemptConversationEffects, RecoveryReviewBlock,
};
use crate::repository::agent_preset_lineage::validate_and_lock_agent_preset_lineage;
use crate::repository::agent_execution::{
    AdoptAgentExecutionStepOutputParams, AgentExecutionAttemptSessionKind,
    AgentExecutionLeaseToken,
    AgentExecutionTaskRetirementFacts,
    AgentExecutionAttemptRecoveryDisposition, AgentExecutionAttemptRecoveryResult,
    AppendAgentExecutionStepsFromAttemptParams, AppendAgentExecutionStepsFromAttemptResult,
    AppendAgentExecutionStepsParams,
    AttemptConversationEffectParams, AttemptConversationEffectResult,
    CreateAgentExecutionAttemptParams, CreateAgentExecutionParams, IAgentExecutionRepository,
    LoopRepeatResetParams,
    AgentExecutionSourceAppend, NewAgentExecutionEvent, NewAgentExecutionParticipant, NewAgentExecutionStep,
    NewAgentExecutionStepDependency, ReconcileAgentExecutionPlanParams,
    ConversationCleanupCursor, ConversationCleanupTurnTarget, PendingConversationCleanup, PendingConversationCleanupPage,
    RetryAgentExecutionStep, SettleAgentExecutionAttemptParams,
    AgentExecutionAttemptDisposition, RecoveredAgentExecutionAttemptOutput, AgentExecutionTerminalTurnGuard,
    UpdateAgentExecutionParams,
};

#[path = "sqlite_agent_execution_stop.rs"]
mod scoped_stop;

#[path = "sqlite_agent_execution_cleanup.rs"]
mod conversation_cleanup;

#[path = "sqlite_agent_execution_retirement.rs"]
mod task_retirement;

#[path = "sqlite_agent_execution_source.rs"]
mod source_continuation;

#[path = "sqlite_agent_execution_parent_reply.rs"]
mod parent_reply;

#[path = "sqlite_agent_execution_task_control.rs"]
mod task_control;
#[path = "sqlite_agent_execution_restart.rs"]
mod restart;

#[path = "sqlite_agent_execution_pause.rs"]
mod native_pause;

#[derive(Clone, Debug)]
pub struct SqliteAgentExecutionRepository {
    pool: SqlitePool,
}

impl SqliteAgentExecutionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Original creation ownership for empty-Session retirement. This immutable
/// source read also covers a soft-deleted Execution; it grants no dispatch.
pub async fn original_attempt_creation_source_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,attempt:&str)
    ->Result<Option<nomifun_common::AgentExecutionSource>,DbError> {
    let identity:Option<(String,String)>=sqlx::query_as("SELECT attempt.execution_id,attempt.step_id FROM agent_execution_attempts attempt JOIN agent_executions execution ON execution.execution_id=attempt.execution_id WHERE attempt.attempt_id=? AND execution.user_id=?")
        .bind(attempt).bind(user).fetch_optional(&mut **tx).await?;
    let (execution,step)=identity.ok_or_else(||conflict("original Attempt creation owner"))?;
    source_continuation::step_source_tx(tx,user,&execution,&step,true).await
}

fn conflict(entity: &str) -> DbError {
    DbError::Conflict(format!(
        "{entity} changed concurrently, does not belong to the caller, or no longer exists"
    ))
}

struct DelegationCreationIdentity {
    session_id: String,
    turn_operation_id: String,
    digest: String,
}

// The immutable Created event retains only this command digest. Current goal,
// policies and the prepared workspace may change after creation and cannot
// identify a replay of the original delegation.
fn delegation_creation_identity(params: &CreateAgentExecutionParams) -> Result<Option<DelegationCreationIdentity>, DbError> {
    let mut initial: serde_json::Value = serde_json::from_str(&params.initial_plan_input)
        .map_err(|error| DbError::Conflict(format!("invalid initial planning input: {error}")))?;
    if !matches!(initial.get("mode").and_then(serde_json::Value::as_str), Some("automatic" | "explicit")) {
        return Ok(None);
    }
    let Some(source) = initial.get("source").filter(|source| !source.is_null()) else { return Ok(None); };
    let session_id = source.get("agent_session_id").and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty()).ok_or_else(|| conflict("delegation source Session"))?.to_owned();
    let turn_operation_id = source.get("turn_operation_id").and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty()).ok_or_else(|| conflict("delegation source Turn"))?.to_owned();
    if params.lead_conversation_id.as_deref() != Some(session_id.as_str()) {
        return Err(conflict("delegation source and lead Session"));
    }
    initial.as_object_mut().ok_or_else(|| conflict("initial planning input"))?.remove("source");
    let command = serde_json::json!({
        "schema":"delegation-creation-v1",
        "agent_session_id":session_id,
        "turn_operation_id":turn_operation_id,
        "goal":params.goal.trim(),
        "adaptation_policy":params.adaptation_policy,
        "decision_policy":params.decision_policy,
        "delegation_policy":params.delegation_policy,
        "max_parallel":params.max_parallel,
        "work_dir":params.work_dir.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "initial_plan":initial,
    });
    let digest = nomifun_agent_contracts::digest_payload(&command)
        .map_err(|error| DbError::Conflict(format!("delegation command cannot be canonicalized: {error}")))?.0;
    Ok(Some(DelegationCreationIdentity { session_id, turn_operation_id, digest }))
}

async fn verify_terminal_turn_tx(
    tx: &mut Transaction<'_, Sqlite>, user_id: &str, execution_id: &str,
    step_id: &str, attempt_id: &str, source: &AgentExecutionTerminalTurnGuard,
    allow_paused_reconciliation: bool,
) -> Result<(), DbError> {
    let (conversation_id, operation_id, terminal_event_id) = source.source();
    let (turn_state, terminal_kind) = match source {
        AgentExecutionTerminalTurnGuard::InputRequired { user_input_request, .. } => {
            user_input_request.validate().map_err(DbError::Conflict)?;
            ("completed", "turn/completed")
        }
        AgentExecutionTerminalTurnGuard::Cancelled { .. } => ("cancelled", "turn/cancelled"),
    };
    let row: Option<(String, Option<String>, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT execution.decision_policy, terminal.inline_json, payload.body \
         FROM conversation_execution_links link \
         JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id \
           AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id \
           AND attempt.status='running' AND attempt.attempt_no=(SELECT MAX(current.attempt_no) FROM agent_execution_attempts current \
               WHERE current.execution_id=link.execution_id AND current.step_id=link.step_id) \
         JOIN agent_execution_steps step ON step.execution_id=link.execution_id AND step.step_id=link.step_id \
           AND step.status='running' AND step.superseded_in_revision IS NULL \
         JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
         JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         JOIN agent_turns turn ON turn.session_id=session.agent_session_id AND turn.operation_id=? \
         JOIN agent_events started ON started.event_id=turn.started_event_id \
         JOIN agent_events terminal ON terminal.event_id=turn.terminal_event_id \
         LEFT JOIN agent_payloads payload ON payload.payload_id=terminal.payload_id AND payload.session_id=session.agent_session_id \
         JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         WHERE link.conversation_id=? AND link.execution_id=? AND link.step_id=? AND link.attempt_id=? \
           AND link.active=1 AND link.relation IN ('attempt','automation') \
           AND (SELECT COUNT(*) FROM conversation_execution_links peer WHERE peer.conversation_id=link.conversation_id \
               AND peer.active=1 AND peer.relation IN ('attempt','automation'))=1 \
           AND (SELECT COUNT(*) FROM conversation_execution_links peer WHERE peer.execution_id=link.execution_id \
               AND peer.step_id=link.step_id AND peer.attempt_id=link.attempt_id \
               AND peer.active=1 AND peer.relation IN ('attempt','automation'))=1 \
           AND session.state='live' AND execution.user_id=? AND execution.deleted_at IS NULL \
           AND (execution.status IN ('running','waiting_input') OR (? AND execution.status='paused')) \
           AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? \
           AND turn.state=? AND terminal.event_id=? \
           AND terminal.session_id=session.agent_session_id AND terminal.correlation_id=turn.operation_id \
           AND terminal.kind=? AND terminal.kind_version=1 \
           AND started.session_id=session.agent_session_id AND started.correlation_id=turn.operation_id \
           AND started.kind='turn/started' AND started.kind_version=1 \
           AND terminal.seq>started.seq \
           AND head.active_turn_id IS NULL AND head.status='ready' \
           AND NOT EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=session.agent_session_id \
               AND effect.state IN ('pending','unknown')) \
           AND NOT EXISTS(SELECT 1 FROM agent_turns successor WHERE successor.session_id=session.agent_session_id \
               AND successor.operation_id<>turn.operation_id AND successor.accepted_at>turn.accepted_at)",
    ).bind(operation_id).bind(conversation_id).bind(execution_id)
        .bind(step_id).bind(attempt_id).bind(user_id).bind(allow_paused_reconciliation).bind(user_id).bind(turn_state).bind(terminal_event_id).bind(terminal_kind)
        .fetch_optional(&mut **tx).await?;
    let (policy, inline, stored) = row.ok_or_else(|| conflict("canonical terminal source Turn"))?;
    let key = crate::execution_attempt_turn_key_tx(tx,user_id,execution_id,step_id,attempt_id).await?;
    if operation_id != format!("turn:user:{user_id}:{conversation_id}:{key}") {
        return Err(conflict("current Agent Execution terminal invocation"));
    }
    let value: serde_json::Value = match (inline, stored) {
        (Some(value), _) => serde_json::from_str(&value),
        (None, Some(bytes)) => serde_json::from_slice(&bytes),
        _ => return Err(conflict("canonical terminal payload")),
    }.map_err(|_| conflict("canonical terminal payload"))?;
    match source {
        AgentExecutionTerminalTurnGuard::InputRequired { user_input_request, .. } => {
            if !matches!(policy.as_str(), "automatic" | "ask_user") { return Err(conflict("canonical human handoff policy")); }
            let request: nomifun_agent_contracts::AgentUserInputRequest = serde_json::from_value(
                value.get("user_input_request").cloned().ok_or_else(|| conflict("canonical user input request"))?
            ).map_err(|_| conflict("canonical user input request"))?;
            request.validate().map_err(DbError::Conflict)?;
            if &request != user_input_request { return Err(conflict("canonical user input request")); }
        }
        AgentExecutionTerminalTurnGuard::Cancelled { .. } => {
            if value.get("user_input_request").is_some() { return Err(conflict("cancelled Turn cannot declare a human handoff")); }
        }
    }
    Ok(())
}

fn is_terminal_execution_status(status: &str) -> bool {
    matches!(
        status,
        "completed" | "completed_with_failures" | "failed" | "cancelled"
    )
}

/// Acquire the transaction writer lock while proving that a lead Session is
/// owned by the caller. Only a live canonical AgentSession is eligible.
async fn lock_owned_lead_session_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    agent_session_id: &str,
) -> Result<(), DbError> {
    let canonical = sqlx::query(
        "UPDATE agent_sessions SET next_seq = next_seq \
         WHERE agent_session_id = ? AND state = 'live' \
           AND json_extract(owner_ref_json, '$.principal_kind') = 'user' \
           AND json_extract(owner_ref_json, '$.principal_id') = ?",
    )
    .bind(agent_session_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if canonical == 1 {
        return Ok(());
    }

    Err(conflict("lead AgentSession"))
}

/// Atomically make `execution_id` the current lead owner of
/// `conversation_id`. Lead rows are audit identity and therefore never become
/// active again: switching deactivates the previous current rows and appends a
/// replacement row for the new current owner.
async fn switch_current_lead_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    conversation_id: &str,
    now: i64,
) -> Result<ConversationExecutionLinkRow, DbError> {
    let execution = sqlx::query(
        "UPDATE agent_executions SET updated_at = updated_at \
         WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(execution_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    if execution.rows_affected() == 0 {
        return Err(conflict("lead execution"));
    }
    lock_owned_lead_session_tx(tx, user_id, conversation_id).await?;

    let is_attempt_conversation: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM conversation_execution_links link \
             JOIN agent_executions execution ON execution.execution_id = link.execution_id \
             WHERE link.conversation_id = ? AND link.relation = 'attempt' \
               AND execution.user_id = ? \
         )",
    )
    .bind(conversation_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    if is_attempt_conversation != 0 {
        return Err(DbError::Conflict(
            "an Attempt Conversation permanently belongs to its Agent Execution and cannot become a lead"
                .into(),
        ));
    }

    let occupied: i64 = sqlx::query_scalar(
        "SELECT EXISTS( \
             SELECT 1 FROM conversation_execution_links link \
             JOIN agent_executions execution ON execution.execution_id = link.execution_id \
             WHERE link.conversation_id = ? AND link.relation = 'lead' \
               AND link.active = 1 AND link.execution_id <> ? \
               AND execution.user_id = ? AND execution.deleted_at IS NULL \
               AND execution.status NOT IN ( \
                   'completed', 'completed_with_failures', 'failed', 'cancelled' \
               ) \
         )",
    )
    .bind(conversation_id)
    .bind(execution_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    if occupied != 0 {
        return Err(DbError::Conflict(
            "conversation already has an unfinished Agent Execution".to_owned(),
        ));
    }

    sqlx::query(
        "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
         WHERE relation = 'lead' AND active = 1 \
           AND (execution_id = ? OR conversation_id = ?)",
    )
    .bind(now)
    .bind(execution_id)
    .bind(conversation_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO conversation_execution_links (\
            conversation_id, execution_id, relation, step_id, attempt_id, \
            active, created_at, updated_at\
         ) VALUES (?, ?, 'lead', NULL, NULL, 1, ?, ?)",
    )
    .bind(conversation_id)
    .bind(execution_id)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(sqlx::query_as::<_, ConversationExecutionLinkRow>(
        "SELECT * FROM conversation_execution_links \
         WHERE conversation_id = ? AND execution_id = ? AND relation = 'lead' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(conversation_id)
    .bind(execution_id)
    .fetch_one(&mut **tx)
    .await?)
}

/// Restore the immutable lead identity of a terminal Execution when a control
/// operation reopens it. If it is already current this is a no-op; otherwise a
/// replacement lead row is appended under the same write transaction.
async fn activate_execution_lead_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    now: i64,
) -> Result<(), DbError> {
    let lead_identity: Option<(String, bool)> = sqlx::query_as(
        "SELECT conversation_id, active FROM conversation_execution_links \
         WHERE execution_id = ? AND relation = 'lead' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(execution_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((conversation_id, active)) = lead_identity else {
        return Ok(());
    };
    if active {
        return Ok(());
    }
    switch_current_lead_tx(tx, user_id, execution_id, &conversation_id, now).await?;
    Ok(())
}

/// Acquire SQLite's transaction write lock while proving that this scheduler
/// generation still owns an unexpired lease. Once this no-op update succeeds,
/// no competing generation can replace the lease until the transaction ends,
/// so every following mutation in the transaction is fenced atomically.
async fn fence_scheduler_write_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    lease: Option<&AgentExecutionLeaseToken>,
    now: i64,
) -> Result<(), DbError> {
    let Some(lease) = lease else {
        return Ok(());
    };
    let result = sqlx::query(
        "UPDATE agent_executions SET lease_owner = lease_owner \
         WHERE execution_id = ? AND lease_owner = ? AND lease_expires_at > ? \
           AND deleted_at IS NULL AND status IN ('running', 'waiting_input', 'paused')",
    )
    .bind(execution_id)
    .bind(lease.owner())
    .bind(now)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Err(DbError::Conflict(
            "Agent Execution scheduler lease is no longer valid".to_owned(),
        ));
    }
    Ok(())
}

async fn require_existing_autowork_admitted_tx(
    tx:&mut Transaction<'_,Sqlite>,user_id:&str,execution_id:&str,
)->Result<(),DbError> {
    let locked=sqlx::query("UPDATE agent_executions SET version=version WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
        .bind(execution_id).bind(user_id).execute(&mut **tx).await?;
    if locked.rows_affected()!=1 {return Err(conflict("Agent Execution admission owner"));}
    let (input,lead):(String,Option<String>)=sqlx::query_as(
        "SELECT initial_plan_input,(SELECT conversation_id FROM conversation_execution_links \
         WHERE execution_id=execution.execution_id AND relation='lead' AND active=1 LIMIT 1) \
         FROM agent_executions execution WHERE execution_id=? AND user_id=? AND deleted_at IS NULL",
    ).bind(execution_id).bind(user_id).fetch_one(&mut **tx).await?;
    super::agent_execution_admission::require_autowork_admitted_tx(tx,user_id,&input,lead.as_deref()).await
}

async fn active_attempt_conversation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    step_id: &str,
    attempt_id: &str,
) -> Result<String, DbError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT link.conversation_id FROM conversation_execution_links link \
         JOIN agent_executions execution ON execution.execution_id = link.execution_id \
         JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
         WHERE link.execution_id = ? AND link.step_id = ? AND link.attempt_id = ? \
           AND link.relation IN ('attempt', 'automation') AND link.active = 1 \
           AND execution.user_id = ? AND session.state = 'live' \
           AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
           AND json_extract(session.owner_ref_json, '$.principal_id') = ? \
           AND execution.deleted_at IS NULL \
         ORDER BY link.id LIMIT 2",
    )
    .bind(execution_id)
    .bind(&step_id)
    .bind(&attempt_id)
    .bind(user_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    match rows.as_slice() {
        [conversation_id] => Ok(conversation_id.clone()),
        [] => Err(DbError::Conflict(
            "waiting attempt has no active Agent conversation".to_owned(),
        )),
        _ => Err(DbError::Conflict(
            "attempt has multiple active Agent conversations".to_owned(),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginalAttemptReceiptReconciliation {
    OngoingTurnObserved,
    ReReadRequired,
    InitialDeliveryPending,
    CompletedReceiptAdopted,
    CancelledReceiptAdopted,
    UserInputRequested,
    ReviewBlocked,
}

fn review_block_runtime_state(
    operation_id: &str,
    receipt_state: &str,
    reason: &str,
    previous_runtime_state: Option<&str>,
) -> Result<String, DbError> {
    let mut state = AttemptConversationEffects::decode(previous_runtime_state)
        .map_err(|error| DbError::Init(error.to_string()))?;
    // A manual-review barrier never discards a committed steering intent or
    // decision input. Their exact identities/text remain available to owner
    // review even when they cannot safely be delivered automatically.
    state.review_blocked = Some(RecoveryReviewBlock {
        kind: "interrupted_turn".to_owned(), operation_id: operation_id.to_owned(),
        receipt_state: receipt_state.to_owned(), reason: reason.to_owned(),
    });
    state.encode().map_err(|error| DbError::Init(error.to_string()))
}

async fn reconcile_original_attempt_receipt_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    step_id: &str,
    expected_step_version: i64,
    attempt_id: &str,
    expected_attempt_version: i64,
    recovered_output: Option<&RecoveredAgentExecutionAttemptOutput>,
    preserve_ongoing_turn: bool,
    now: i64,
) -> Result<OriginalAttemptReceiptReconciliation, DbError> {
    let active_sessions: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT link.conversation_id \
         FROM conversation_execution_links link \
         JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
         WHERE link.execution_id = ? AND link.step_id = ? AND link.attempt_id = ? \
            AND link.relation IN ('attempt', 'automation') \
           AND session.state = 'live' \
           AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
           AND json_extract(session.owner_ref_json, '$.principal_id') = ? \
         ORDER BY link.conversation_id LIMIT 2",
    )
    .bind(execution_id)
    .bind(step_id)
    .bind(attempt_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;
    let previous_runtime_state: Option<String> = sqlx::query_scalar(
        "SELECT runtime_state FROM agent_execution_attempts WHERE execution_id=? AND attempt_id=?"
    ).bind(execution_id).bind(attempt_id).fetch_one(&mut **tx).await?;
    let effects = AttemptConversationEffects::decode(previous_runtime_state.as_deref())
        .map_err(|error| DbError::Init(error.to_string()))?;
    let operation_key = crate::execution_attempt_turn_key_tx(tx,user_id,execution_id,step_id,attempt_id).await?;
    let receipt = match active_sessions.as_slice() {
        [session_id] => {
            let operation_id = format!("turn:user:{user_id}:{session_id}:{operation_key}");
            sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
                "SELECT state, terminal_event_id, finished_at \
                 FROM agent_turns WHERE session_id = ? AND operation_id = ?",
            )
            .bind(session_id)
            .bind(operation_id)
            .fetch_optional(&mut **tx)
            .await?
        }
        _ => None,
    };

    if let Some((state, terminal_event_id, finished_at)) = receipt.as_ref()
        && let Some(output) = recovered_output
        && matches!(state.as_str(), "completed" | "failed" | "cancelled" | "interrupted")
    {
        if output.disposition==AgentExecutionAttemptDisposition::InterventionRequired {
            return Err(conflict("nonterminal intervention cannot become terminal output"));
        }
        let expected_operation = format!(
            "turn:user:{user_id}:{}:{operation_key}", output.conversation_id
        );
        if output.attempt_id != attempt_id
            || active_sessions.as_slice() != [output.conversation_id.clone()]
            || output.canonical_operation_id != expected_operation
            || terminal_event_id.as_deref() != Some(output.terminal_event_id.as_str())
            || output.tokens.is_some_and(|tokens| tokens < 0)
            || output.output_files.iter().any(|path| path.trim().is_empty())
        {
            return Err(conflict("canonical Agent Execution recovery output"));
        }
        if (state == "cancelled") != (output.disposition == AgentExecutionAttemptDisposition::Cancelled)
            || output.user_input_request.is_some() != (output.disposition == AgentExecutionAttemptDisposition::InputRequired) {
            return Err(conflict("canonical terminal disposition"));
        }
        // Session control metadata is not a public answer. Only typed output
        // returned by the canonical owner may settle the business result.
        let has_output = output.text.as_ref().is_some_and(|text| !text.trim().is_empty())
            || !output.output_files.is_empty();
        let waiting = output.disposition == AgentExecutionAttemptDisposition::InputRequired;
        let cancelled = output.disposition == AgentExecutionAttemptDisposition::Cancelled;
        if effects.review_blocked.is_some() && (waiting||cancelled) {
            // Restore only inside this same writer so the exact terminal
            // verifier can consume the old link. Any failed proof rolls the
            // restoration back; no running admission escapes the transaction.
            sqlx::query("UPDATE agent_execution_attempts SET status='running' WHERE execution_id=? AND attempt_id=? AND version=? AND status='waiting_input'")
                .bind(execution_id).bind(attempt_id).bind(expected_attempt_version).execute(&mut **tx).await?;
            sqlx::query("UPDATE agent_execution_steps SET status='running' WHERE execution_id=? AND step_id=? AND version=? AND status='waiting_input'")
                .bind(execution_id).bind(step_id).bind(expected_step_version).execute(&mut **tx).await?;
            sqlx::query("UPDATE conversation_execution_links SET active=1 WHERE id=(SELECT MAX(id) FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation IN('attempt','automation'))")
                .bind(execution_id).bind(step_id).bind(attempt_id).execute(&mut **tx).await?;
        }
        if waiting || cancelled {
            let source = if waiting {
                AgentExecutionTerminalTurnGuard::InputRequired {
                    conversation_id: output.conversation_id.clone(), canonical_operation_id: output.canonical_operation_id.clone(),
                    terminal_event_id: output.terminal_event_id.clone(), user_input_request: output.user_input_request.clone()
                        .ok_or_else(|| conflict("recovered canonical human handoff"))?,
                }
            } else { AgentExecutionTerminalTurnGuard::Cancelled {
                conversation_id: output.conversation_id.clone(), canonical_operation_id: output.canonical_operation_id.clone(),
                terminal_event_id: output.terminal_event_id.clone(),
            } };
            verify_terminal_turn_tx(tx, user_id, execution_id, step_id, attempt_id, &source, true).await?;
        }
        let succeeded = state == "completed" && output.disposition == AgentExecutionAttemptDisposition::Delivered
            && has_output && output.user_input_request.is_none();
        let error = (!succeeded && !waiting && !cancelled).then(|| output.error.as_deref().unwrap_or(
            "Canonical Agent Turn ended without a successful public delivery"
        ));
        let delivered_files = if succeeded || waiting { output.output_files.as_slice() } else { &[] };
        let output_files = serde_json::to_string(delivered_files)
            .map_err(|error| DbError::Init(format!("encode recovered canonical output files: {error}")))?;
        let attempt = sqlx::query(
            "UPDATE agent_execution_attempts SET status = ?, question = ?, error = ?, \
                output_summary = ?, output_files = ?, tokens = ?, runtime_state = NULL, retry_after = NULL, \
                finished_at = ?, version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status IN ('running','waiting_input')",
        )
        .bind(if waiting { "waiting_input" } else if cancelled { "cancelled" } else if succeeded { "completed" } else { "failed" })
        .bind(if waiting { output.user_input_request.as_ref().map(|request| request.question.as_str()) } else { None })
        .bind(error)
        .bind((succeeded || waiting).then_some(output.text.as_deref()).flatten())
        .bind(output_files)
        .bind(output.tokens)
        .bind((!waiting).then_some(finished_at.unwrap_or(now)))
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .execute(&mut **tx)
        .await?;
        if attempt.rows_affected() != 1 {
            return Err(conflict("running Agent Execution attempt"));
        }
        let step = sqlx::query(
            "UPDATE agent_execution_steps SET status = ?, dispatch_after = NULL, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND status IN ('running','waiting_input') AND superseded_in_revision IS NULL",
        )
        .bind(if waiting { "waiting_input" } else if cancelled { "cancelled" } else if succeeded { "completed" } else { "failed" })
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .execute(&mut **tx)
        .await?;
        if step.rows_affected() != 1 {
            return Err(conflict("running Agent Execution step"));
        }
        if !waiting {
            requeue_terminal_attempt_cleanup_tx(tx, execution_id, step_id, now).await?;
            sqlx::query(
            "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? \
               AND relation IN ('attempt', 'automation') AND active = 1",
        )
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .execute(&mut **tx)
        .await?; }
        return Ok(if waiting { OriginalAttemptReceiptReconciliation::UserInputRequested }
            else if cancelled { OriginalAttemptReceiptReconciliation::CancelledReceiptAdopted }
            else { OriginalAttemptReceiptReconciliation::CompletedReceiptAdopted });
    }

    if recovered_output.is_none() && receipt.as_ref().is_some_and(|(state,terminal,_)|
        matches!(state.as_str(),"completed"|"failed"|"cancelled"|"interrupted") && terminal.is_some()) {
        return Ok(OriginalAttemptReceiptReconciliation::ReReadRequired);
    }

    if receipt.is_none() && let [session]=active_sessions.as_slice()
        && effects.pending_conversation_effects.iter().any(|effect|matches!(effect,
            crate::PendingConversationEffect::InitialInput {..})) {
        let input_identity=format!("user:{user_id}:{session}:{operation_key}");
        let original_operation=format!("turn:{input_identity}");
        let reused_session:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND conversation_id=? AND relation='automation') OR EXISTS(SELECT 1 FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND json_extract(payload,'$.continuation_step_id')=? AND json_extract(payload,'$.target_session_id')=?)")
            .bind(execution_id).bind(step_id).bind(attempt_id).bind(session).bind(execution_id).bind(step_id).bind(session).fetch_one(&mut **tx).await?;
        let clear:bool=sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_sessions session JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
             WHERE session.agent_session_id=? AND session.state='live' AND head.status='ready' AND head.active_turn_id IS NULL \
               AND NOT EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=session.agent_session_id AND (effect.state IN ('pending','unknown') OR effect.turn_id=?)) \
               AND NOT EXISTS(SELECT 1 FROM agent_events accepted WHERE accepted.session_id=session.agent_session_id AND accepted.idempotency_key IN(?,?)) \
               AND (? OR NOT EXISTS(SELECT 1 FROM agent_events work WHERE work.session_id=session.agent_session_id AND work.kind IN('message/user-accepted','turn/started','runtime/progress-recorded'))))"
        ).bind(session).bind(&original_operation).bind(format!("{input_identity}:message")).bind(format!("{input_identity}:turn")).bind(reused_session).fetch_one(&mut **tx).await?;
        if clear {
            // The caller already holds the current lease's SQLite writer.
            // Bump the exact Step version before committing the re-delivery:
            // any old sender must fail canonical same-writer admission, even
            // when it had passed a preflight before this transaction.
            let changed=sqlx::query("UPDATE agent_execution_steps SET status='running',version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=? AND superseded_in_revision IS NULL AND status IN('running','waiting_input')")
                .bind(now).bind(execution_id).bind(step_id).bind(expected_step_version).execute(&mut **tx).await?;
            if changed.rows_affected()!=1 {return Err(conflict("original initial input Step fence"));}
            let mut effects=effects;effects.review_blocked=None;
            let changed=sqlx::query("UPDATE agent_execution_attempts SET status='running',question=NULL,error=NULL,runtime_state=?,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status IN('running','waiting_input')")
                .bind(effects.encode().map_err(|error|DbError::Init(error.to_string()))?).bind(now).bind(execution_id).bind(step_id).bind(attempt_id).bind(expected_attempt_version).execute(&mut **tx).await?;
            if changed.rows_affected()!=1 {return Err(conflict("original initial input Attempt fence"));}
            sqlx::query("UPDATE conversation_execution_links SET active=1,cleanup_completed_at=NULL,updated_at=? WHERE id=(SELECT MAX(id) FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND conversation_id=? AND relation IN('attempt','automation'))")
                .bind(now).bind(execution_id).bind(step_id).bind(attempt_id).bind(session).execute(&mut **tx).await?;
            return Ok(OriginalAttemptReceiptReconciliation::InitialDeliveryPending);
        }
    }

    if let [session_id] = active_sessions.as_slice() {
        let restarting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_events request WHERE request.execution_id=? AND request.step_id=? AND request.attempt_id=? AND request.event_type='step_changed' AND json_extract(request.payload,'$.change')='restart_requested' AND NOT EXISTS(SELECT 1 FROM agent_execution_events result WHERE result.execution_id=request.execution_id AND result.step_id=request.step_id AND result.event_type='step_changed' AND json_extract(result.payload,'$.change')='restart_resolved' AND json_extract(result.payload,'$.operation_id')=json_extract(request.payload,'$.operation_id')))")
            .bind(execution_id).bind(step_id).bind(attempt_id).fetch_one(&mut **tx).await?;
        let operation_id=format!("turn:user:{user_id}:{session_id}:{operation_key}");
        if !restarting && let Some(review)=native_pause::technical_boundary_tx(tx,session_id,&operation_id).await? {
            native_pause::retain_review_tx(tx,execution_id,step_id,expected_step_version,attempt_id,
                expected_attempt_version,&effects,review,now).await?;
            return Ok(OriginalAttemptReceiptReconciliation::ReviewBlocked);
        }
    }

    if preserve_ongoing_turn && let [session_id] = active_sessions.as_slice() {
        let operation_id = format!("turn:user:{user_id}:{session_id}:{operation_key}");
        let ongoing: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id \
             WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='running' AND turn.terminal_event_id IS NULL \
               AND head.active_turn_id=turn.operation_id AND head.status IN ('running','paused','reconciliation'))",
        ).bind(session_id).bind(operation_id).fetch_one(&mut **tx).await?;
        if ongoing != 0 {
            if effects.review_blocked.is_some() {
                let mut restored=effects.clone();restored.review_blocked=None;
                let changed=sqlx::query("UPDATE agent_execution_attempts SET status='running',question=NULL,error=NULL,runtime_state=?,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status='waiting_input'")
                    .bind(restored.encode().map_err(|error|DbError::Init(error.to_string()))?).bind(now).bind(execution_id).bind(step_id).bind(attempt_id).bind(expected_attempt_version).execute(&mut **tx).await?;
                if changed.rows_affected()!=1 {return Err(conflict("reconciled original native owner"));}
                let changed=sqlx::query("UPDATE agent_execution_steps SET status='running',version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=? AND status='waiting_input' AND superseded_in_revision IS NULL")
                    .bind(now).bind(execution_id).bind(step_id).bind(expected_step_version).execute(&mut **tx).await?;
                if changed.rows_affected()!=1 {return Err(conflict("reconciled original task"));}
                sqlx::query("UPDATE conversation_execution_links SET active=1,cleanup_completed_at=NULL,updated_at=? WHERE id=(SELECT MAX(id) FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation IN('attempt','automation'))")
                    .bind(now).bind(execution_id).bind(step_id).bind(attempt_id).execute(&mut **tx).await?;
            }
            // Native recovery owns its own lease and checkpoint checks. An
            // expired Execution lease grants no right to interrupt that owner.
            return Ok(OriginalAttemptReceiptReconciliation::OngoingTurnObserved);
        }
    }

    let (receipt_state, reason) = match receipt.as_ref().map(|receipt| receipt.0.as_str()) {
        Some("completed" | "failed" | "cancelled" | "interrupted") => (
            "terminal_output_unavailable",
            "The canonical Turn is terminal, but its exact typed public delivery has not been validated. Automatic retry is blocked; control metadata is not task output.",
        ),
        Some("accepted" | "running") => (
            "running",
            "The canonical Agent Turn was durably admitted before interruption, but its terminal outcome is unknown. Automatic retry is blocked to prevent duplicate model or tool effects.",
        ),
        Some(_) => (
            "unsupported",
            "The canonical Agent Turn has an unsupported receipt state. Automatic retry is blocked because its external effect outcome cannot be proven.",
        ),
        None if active_sessions.len() != 1 => (
            "identity_ambiguous",
            "The interrupted Agent Turn cannot be matched to one exact AgentSession. Automatic retry is blocked to prevent duplicate effects.",
        ),
        None => (
            "missing",
            "No terminal receipt exists for the interrupted canonical Agent Turn. This does not prove that model or tool effects were never started, so automatic retry is blocked.",
        ),
    };
    if effects.review_blocked.is_some() { return Ok(OriginalAttemptReceiptReconciliation::ReviewBlocked); }
    sqlx::query(
        "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
         WHERE execution_id = ? AND step_id = ? AND attempt_id = ? \
           AND relation IN ('attempt', 'automation') AND active = 1",
    )
    .bind(now)
    .bind(execution_id)
    .bind(step_id)
    .bind(attempt_id)
    .execute(&mut **tx)
    .await?;
    let operation_id = active_sessions.first().map(|session_id| {
        format!("turn:user:{user_id}:{session_id}:{operation_key}")
    }).unwrap_or(operation_key);
    let runtime_state = review_block_runtime_state(&operation_id, receipt_state, reason, previous_runtime_state.as_deref())?;
    let attempt = sqlx::query(
        "UPDATE agent_execution_attempts SET status = 'waiting_input', question = NULL, error = ?, \
            runtime_state = ?, retry_after = NULL, finished_at = NULL, \
            version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
           AND status IN ('running','waiting_input')",
    )
    .bind(reason)
    .bind(runtime_state)
    .bind(now)
    .bind(execution_id)
    .bind(step_id)
    .bind(attempt_id)
    .bind(expected_attempt_version)
    .execute(&mut **tx)
    .await?;
    if attempt.rows_affected() != 1 {
        return Err(conflict("running Agent Execution attempt"));
    }
    let step = sqlx::query(
        "UPDATE agent_execution_steps SET status = 'waiting_input', dispatch_after = NULL, \
            version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND step_id = ? AND version = ? \
           AND status IN ('running','waiting_input') AND superseded_in_revision IS NULL",
    )
    .bind(now)
    .bind(execution_id)
    .bind(step_id)
    .bind(expected_step_version)
    .execute(&mut **tx)
    .await?;
    if step.rows_affected() != 1 {
        return Err(conflict("running Agent Execution step"));
    }
    Ok(OriginalAttemptReceiptReconciliation::ReviewBlocked)
}

fn scoped_event(
    event: &NewAgentExecutionEvent,
    step_id: &str,
    attempt_id: Option<&str>,
) -> NewAgentExecutionEvent {
    let mut scoped = event.clone();
    scoped.step_id = Some(step_id.to_owned());
    scoped.attempt_id = attempt_id.map(str::to_owned);
    scoped
}

async fn append_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    event: &NewAgentExecutionEvent,
    now: i64,
) -> Result<AgentExecutionEventRow, DbError> {
    let step_id = event.step_id.clone();
    let attempt_id = event.attempt_id.clone();
    // Attribution is derived and authorized under the same SQLite write lock
    // as the domain mutation. An owner-scoped service call is not sufficient
    // authority for an Agent: its calling conversation must still have one
    // unambiguous active relation to the target execution.
    let execution: Option<(String, i64)> = sqlx::query_as(
        "UPDATE agent_executions SET updated_at = updated_at \
         WHERE execution_id = ? \
         RETURNING user_id, event_sequence",
    )
    .bind(execution_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((on_behalf_of_user_id, current_event_sequence)) = execution else {
        return Err(conflict("agent execution event"));
    };
    let owner = sqlx::query(
        "UPDATE users SET updated_at = updated_at WHERE user_id = ?",
    )
    .bind(&on_behalf_of_user_id)
    .execute(&mut **tx)
    .await?;
    if owner.rows_affected() == 0 {
        return Err(DbError::Conflict(
            "execution event owner no longer exists".to_owned(),
        ));
    }
    if let Some(step_id) = step_id.as_deref() {
        let step = sqlx::query(
            "UPDATE agent_execution_steps SET updated_at = updated_at \
             WHERE execution_id = ? AND step_id = ?",
        )
        .bind(execution_id)
        .bind(step_id)
        .execute(&mut **tx)
        .await?;
        if step.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "execution event step '{step_id}' does not belong to Execution '{execution_id}'"
            )));
        }
    }
    if let Some(attempt_id) = attempt_id.as_deref() {
        let Some(step_id) = step_id.as_deref() else {
            return Err(DbError::Conflict(
                "execution event attempt requires a step".to_owned(),
            ));
        };
        let attempt = sqlx::query(
            "UPDATE agent_execution_attempts SET updated_at = updated_at \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ?",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .execute(&mut **tx)
        .await?;
        if attempt.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "execution event attempt '{attempt_id}' does not belong to step '{step_id}'"
            )));
        }
    }
    if (event.event_type == AgentExecutionEventKind::Created) != (current_event_sequence == 0) {
        return Err(DbError::Conflict(
            "live execution events require exactly one Created baseline".to_owned(),
        ));
    }
    let (actor_type, actor_id, actor_conversation_id, actor_attempt_id) = match &event.actor {
        AgentExecutionActor::System => ("system".to_owned(), None, None, None),
        AgentExecutionActor::User { user_id } => {
            if user_id != &on_behalf_of_user_id {
                return Err(DbError::Conflict(
                    "execution event user actor does not match the execution owner".to_owned(),
                ));
            }
            (
                "user".to_owned(),
                Some(user_id.clone()),
                None,
                None,
            )
        }
        AgentExecutionActor::Agent {
            agent_id,
            conversation_id,
            attempt_id,
        } => {
            if agent_id.trim().is_empty() {
                return Err(DbError::Conflict(
                    "execution event Agent actor id must not be empty".to_owned(),
                ));
            }
            if let Some(conversation_id) = conversation_id {
                if agent_id != conversation_id {
                    return Err(DbError::Conflict(
                        "a conversation-backed Agent actor id must be its Conversation id"
                            .to_owned(),
                    ));
                }
                let actor_attempt_id = attempt_id.clone();
                let links = sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT link.relation, link.attempt_id \
                     FROM conversation_execution_links link \
                     JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                     WHERE link.execution_id = ? AND link.conversation_id = ? \
                       AND link.active = 1 AND execution.user_id = ? \
                       AND execution.deleted_at IS NULL",
                )
                .bind(execution_id)
                .bind(conversation_id)
                .bind(&on_behalf_of_user_id)
                .fetch_all(&mut **tx)
                .await?;
                lock_owned_lead_session_tx(tx, &on_behalf_of_user_id, conversation_id).await?;
                let matching_links = links
                    .iter()
                    .filter(|(relation, linked_attempt_id)| match actor_attempt_id.as_ref() {
                        Some(attempt_id) => {
                            matches!(relation.as_str(), "attempt" | "automation")
                                && linked_attempt_id.as_ref() == Some(attempt_id)
                        }
                        None => relation == "lead",
                    })
                    .count();
                if matching_links != 1 {
                    return Err(DbError::Conflict(
                        "Agent caller must have exactly one matching active link to the execution"
                            .to_owned(),
                    ));
                }
                if let Some(attempt_id) = actor_attempt_id.as_deref() {
                    let active_attempt_context: i64 = sqlx::query_scalar(
                         "SELECT COUNT(*) FROM conversation_execution_links link \
                         JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                         WHERE link.conversation_id = ? AND link.attempt_id = ? \
                           AND link.relation IN ('attempt', 'automation') AND link.active = 1 \
                           AND execution.user_id = ? AND execution.deleted_at IS NULL",
                    )
                    .bind(conversation_id)
                    .bind(attempt_id)
                    .bind(&on_behalf_of_user_id)
                    .fetch_one(&mut **tx)
                    .await?;
                    if active_attempt_context != 1 {
                        return Err(DbError::Conflict(
                            "Agent caller attempt context is not active and unambiguous".to_owned(),
                        ));
                    }
                }
            } else {
                if attempt_id.is_some() {
                    return Err(DbError::Conflict(
                        "an external Agent actor cannot claim a local attempt".to_owned(),
                    ));
                }
                let establishes_initiator = event.event_type == AgentExecutionEventKind::Created
                    && current_event_sequence == 0;
                if !establishes_initiator {
                    let authorized: i64 = sqlx::query_scalar(
                        "SELECT EXISTS( \
                             SELECT 1 FROM agent_execution_events baseline \
                             WHERE baseline.execution_id = ? \
                              AND baseline.sequence = 1 \
                              AND baseline.event_type = 'created' \
                              AND baseline.actor_type = 'agent' \
                              AND baseline.actor_conversation_id IS NULL \
                              AND baseline.actor_attempt_id IS NULL \
                              AND baseline.actor_id = ? \
                         )",
                    )
                    .bind(execution_id)
                    .bind(agent_id)
                    .fetch_one(&mut **tx)
                    .await?;
                    if authorized == 0 {
                        return Err(DbError::Conflict(
                            "external Agent actor does not match the root execution initiator"
                                .to_owned(),
                        ));
                    }
                }
            }
            (
                "agent".to_owned(),
                Some(agent_id.clone()),
                conversation_id.clone(),
                attempt_id.clone(),
            )
        }
    };
    let sequence: i64 = sqlx::query_scalar(
         "UPDATE agent_executions \
         SET event_sequence = event_sequence + 1, updated_at = ? \
         WHERE execution_id = ? RETURNING event_sequence",
    )
    .bind(now)
    .bind(execution_id)
    .fetch_one(&mut **tx)
    .await?;
    let presentation_payload = crate::execution_presentation::annotate_event_tx(tx, execution_id, event, &on_behalf_of_user_id).await?;
    sqlx::query(
        "INSERT INTO agent_execution_events (\
            execution_id, sequence, event_type, step_id, attempt_id, \
            actor_type, actor_id, actor_conversation_id, actor_attempt_id, \
            on_behalf_of_user_id, payload, created_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(execution_id)
    .bind(sequence)
    .bind(event.event_type.as_str())
    .bind(&step_id)
    .bind(&attempt_id)
    .bind(&actor_type)
    .bind(&actor_id)
    .bind(&actor_conversation_id)
    .bind(&actor_attempt_id)
    .bind(&on_behalf_of_user_id)
    .bind(&presentation_payload)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(AgentExecutionEventRow {
        execution_id: execution_id.to_owned(),
        sequence,
        event_type: event.event_type.as_str().to_owned(),
        step_id,
        attempt_id,
        actor_type,
        actor_id,
        actor_conversation_id,
        actor_attempt_id,
        on_behalf_of_user_id,
        payload: presentation_payload,
        created_at: now,
        published_at: None,
    })
}

async fn bump_execution_version_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    expected_version: i64,
    now: i64,
) -> Result<(), DbError> {
    let result = sqlx::query(
        "UPDATE agent_executions SET version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(execution_id)
    .bind(user_id)
    .bind(expected_version)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() != 1 {
        return Err(conflict("agent execution"));
    }
    Ok(())
}

/// Formal Step closure ends its continuing purpose across every immutable
/// Attempt, including old backoff generations. Re-arm their existing exact
/// inactive cleanup owners without rewriting any result/retry audit. Callers
/// use this only for an actual terminal/removed Step transition; pending retry
/// alone never revisits a cleanup receipt.
async fn requeue_terminal_attempt_cleanup_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    step_id: &str,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE conversation_execution_links SET cleanup_completed_at = NULL, updated_at = ? \
         WHERE execution_id = ? AND step_id = ? \
           AND relation IN ('attempt', 'automation') AND active = 0 AND cleanup_completed_at IS NOT NULL \
           AND EXISTS(SELECT 1 FROM agent_execution_steps step \
             JOIN agent_execution_attempts attempt ON attempt.execution_id = step.execution_id AND attempt.step_id = step.step_id \
             WHERE step.execution_id = conversation_execution_links.execution_id \
               AND step.step_id = conversation_execution_links.step_id \
               AND attempt.attempt_id = conversation_execution_links.attempt_id \
               AND attempt.status IN ('completed', 'failed', 'cancelled', 'interrupted'))",
    )
    .bind(now)
    .bind(execution_id)
    .bind(step_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn terminate_unfinished_execution_children_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    attempt_error: Option<&str>,
    now: i64,
) -> Result<(), DbError> {
    let stopped_steps: Vec<String> = sqlx::query_scalar(
        "UPDATE agent_execution_steps SET status = 'cancelled', dispatch_after = NULL, \
            version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND superseded_in_revision IS NULL \
           AND status NOT IN ('completed', 'failed', 'skipped', 'cancelled') RETURNING step_id",
    )
    .bind(now)
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    for step_id in stopped_steps {
        requeue_terminal_attempt_cleanup_tx(tx, execution_id, &step_id, now).await?;
    }
    sqlx::query(
        "UPDATE agent_execution_attempts SET \
            status = 'cancelled', question = NULL, \
            error = COALESCE(error, ?), finished_at = ?, \
            version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND status IN ('queued', 'running', 'waiting_input')",
    )
    .bind(attempt_error)
    .bind(now)
    .bind(now)
    .bind(execution_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
         WHERE execution_id = ? AND relation IN ('attempt', 'automation') AND active = 1",
    )
    .bind(now)
    .bind(execution_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_participant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    participant: &NewAgentExecutionParticipant,
    introduced_in_revision: i64,
    now: i64,
) -> Result<(), DbError> {
    nomifun_common::validate_uuidv7(&participant.participant_id)
        .map_err(|error| DbError::Conflict(format!("invalid participant_id: {error}")))?;
    nomifun_common::AgentId::parse(&participant.source_agent_id)
        .map_err(|error| DbError::Conflict(format!("invalid source_agent_id: {error}")))?;
    if let Some(constraints) = participant.constraints.as_deref() {
        crate::repository::agent_execution::validate_participant_constraints_json(constraints)?;
    }
    let source_agent = sqlx::query(
        "UPDATE agent_metadata SET updated_at = updated_at WHERE agent_id = ?",
    )
    .bind(&participant.source_agent_id)
    .execute(&mut **tx)
    .await?;
    if source_agent.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "Agent Execution participant source agent '{}' does not exist",
            participant.source_agent_id
        )));
    }
    validate_and_lock_agent_preset_lineage(
        tx,
        participant.preset_id.as_deref(),
        participant.preset_revision,
        participant.agent_snapshot.as_deref(),
        "Agent Execution participant",
    )
    .await?;
    match (
        participant.provider_id.as_deref(),
        participant.model.as_deref(),
    ) {
        (Some(provider_id), Some(model))
            if nomifun_common::ProviderId::parse(provider_id).is_ok()
                && !model.is_empty()
                && model.trim() == model =>
        {
            // Runtime participants retain immutable provider history, so the
            // provider row is a hard logical parent. Lock and validate it in
            // this write transaction; physical FK/trigger semantics remain
            // deliberately absent.
            let provider = sqlx::query(
                "UPDATE providers SET updated_at = updated_at WHERE provider_id = ?",
            )
            .bind(provider_id)
            .execute(&mut **tx)
            .await?;
            if provider.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "Agent Execution participant provider '{provider_id}' does not exist"
                )));
            }
        }
        (None, None) => {}
        _ => {
            return Err(DbError::Conflict(
                "Agent Execution participant provider_id and model must be an absent or non-empty pair"
                    .to_owned(),
            ));
        }
    }
    sqlx::query(
        "INSERT INTO agent_execution_participants (\
            participant_id, execution_id, source_agent_id, preset_id, preset_revision, agent_snapshot, \
            provider_id, model, role, capability, constraints, description, system_prompt, \
            enabled_skills, disabled_builtin_skills, sort_order, introduced_in_revision, created_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&participant.participant_id)
    .bind(execution_id)
    .bind(&participant.source_agent_id)
    .bind(&participant.preset_id)
    .bind(participant.preset_revision)
    .bind(&participant.agent_snapshot)
    .bind(&participant.provider_id)
    .bind(&participant.model)
    .bind(&participant.role)
    .bind(&participant.capability)
    .bind(&participant.constraints)
    .bind(&participant.description)
    .bind(&participant.system_prompt)
    .bind(&participant.enabled_skills)
    .bind(&participant.disabled_builtin_skills)
    .bind(participant.sort_order)
    .bind(introduced_in_revision)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_step_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    step: &NewAgentExecutionStep,
    assigned_participant_id: Option<&str>,
    delegation_depth: i64,
    introduced_in_revision: i64,
    now: i64,
) -> Result<(), DbError> {
    nomifun_common::validate_uuidv7(&step.step_id)
        .map_err(|error| DbError::Conflict(format!("invalid step_id: {error}")))?;
    if let Some(assigned_participant_id) = assigned_participant_id {
        let participant = sqlx::query(
            "UPDATE agent_execution_participants SET created_at = created_at \
             WHERE execution_id = ? AND participant_id = ? \
               AND retired_in_revision IS NULL",
        )
        .bind(execution_id)
        .bind(assigned_participant_id)
        .execute(&mut **tx)
        .await?;
        if participant.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Agent Execution step assigned participant '{assigned_participant_id}' \
                 does not exist or is retired"
            )));
        }
    }
    sqlx::query(
        "INSERT INTO agent_execution_steps (\
            step_id, execution_id, title, spec, role, tool_policy, kind, agent_mode, profile, fanout_group, \
            control_policy, delegation_depth, status, assigned_participant_id, \
            assignment_score, assignment_rationale, assignment_source, assignment_locked, \
            failure_policy, preset_prompt, graph_x, graph_y, version, introduced_in_revision, \
            created_at, updated_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
    )
    .bind(&step.step_id)
    .bind(execution_id)
    .bind(&step.title)
    .bind(&step.spec)
    .bind(&step.role)
    .bind(step.tool_policy.as_str())
    .bind(step.kind.as_str())
    .bind(step.agent_mode.map(|value| value.as_str()))
    .bind(&step.profile)
    .bind(&step.fanout_group)
    .bind(&step.control_policy)
    .bind(delegation_depth)
    .bind(step.status.as_str())
    .bind(assigned_participant_id)
    .bind(step.assignment_score)
    .bind(&step.assignment_rationale)
    .bind(step.assignment_source.map(|value| value.as_str()))
    .bind(step.assignment_locked)
    .bind(step.failure_policy.as_str())
    .bind(&step.preset_prompt)
    .bind(step.graph_x)
    .bind(step.graph_y)
    .bind(introduced_in_revision)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate_new_ids<'a>(
    entity: &str,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<HashSet<String>, DbError> {
    let mut unique = HashSet::new();
    for id in ids {
        nomifun_common::validate_uuidv7(id)
            .map_err(|error| DbError::Conflict(format!("invalid {entity} id: {error}")))?;
        if !unique.insert(id.to_owned()) {
            return Err(DbError::Conflict(format!(
                "new {entity} ids must be unique UUIDv7 values"
            )));
        }
    }
    Ok(unique)
}

fn validate_dependency_graph(
    step_ids: &HashSet<String>,
    dependencies: &[NewAgentExecutionStepDependency],
) -> Result<(), DbError> {
    let mut indegree: HashMap<String, usize> =
        step_ids.iter().map(|id| (id.clone(), 0)).collect();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut edges: HashSet<(String, String)> = HashSet::new();
    for dependency in dependencies {
        let blocker = &dependency.blocker_step_id;
        let blocked = &dependency.blocked_step_id;
        if blocker == blocked || !step_ids.contains(blocker) || !step_ids.contains(blocked) {
            return Err(DbError::Conflict(
                "execution plan contains a dangling or self dependency".into(),
            ));
        }
        if !edges.insert((blocker.clone(), blocked.clone())) {
            return Err(DbError::Conflict(
                "execution plan contains a duplicate dependency".into(),
            ));
        }
        *indegree.get_mut(blocked).expect("blocked step was validated") += 1;
        outgoing
            .entry(blocker.clone())
            .or_default()
            .push(blocked.clone());
    }

    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect();
    let mut visited = 0usize;
    while let Some(step_id) = queue.pop_front() {
        visited += 1;
        for blocked in outgoing.get(&step_id).into_iter().flatten() {
            let degree = indegree
                .get_mut(blocked)
                .expect("blocked step was validated");
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(blocked.clone());
            }
        }
    }
    if visited != step_ids.len() {
        return Err(DbError::Conflict("execution plan dependency graph contains a cycle".into()));
    }
    Ok(())
}

fn delegation_added_step_ids(payload: &str) -> Result<Vec<String>, DbError> {
    let payload: serde_json::Value = serde_json::from_str(payload).map_err(|_| {
        DbError::Conflict("persisted delegation operation payload is invalid".to_owned())
    })?;
    let added_step_ids = payload
        .get("added_step_ids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            DbError::Conflict(
                "persisted delegation operation has no added_step_ids".to_owned(),
            )
        })?
        .iter()
        .map(|value| {
            let id = value.as_str().ok_or_else(|| {
                DbError::Conflict("persisted delegation operation has invalid Step ids".to_owned())
            })?;
            nomifun_common::validate_uuidv7(id).map_err(|_| {
                DbError::Conflict("persisted delegation operation has invalid Step ids".to_owned())
            })?;
            Ok::<String, DbError>(id.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if added_step_ids.is_empty() {
        return Err(DbError::Conflict(
            "persisted delegation operation has an empty Step set".to_owned(),
        ));
    }
    Ok(added_step_ids)
}

async fn apply_loop_repeat_reset_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    controller_step_id: &str,
    reset: &LoopRepeatResetParams,
    now: i64,
) -> Result<(), DbError> {
    if reset.body_step_id.trim().is_empty() || reset.expected_steps.is_empty() {
        return Err(DbError::Conflict(
            "loop repeat reset requires a body and its descendant closure".into(),
        ));
    }
    let controller_kind: Option<String> = sqlx::query_scalar(
        "SELECT kind FROM agent_execution_steps \
         WHERE execution_id = ? AND step_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .bind(controller_step_id)
    .fetch_optional(&mut **tx)
    .await?;
    if controller_kind.as_deref() != Some("loop") {
        return Err(DbError::Conflict(
            "only a Loop controller can request an iteration reset".into(),
        ));
    }
    let direct_body: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_execution_step_dependencies \
         WHERE execution_id = ? AND blocker_step_id = ? AND blocked_step_id = ? \
           AND superseded_in_revision IS NULL)",
    )
    .bind(execution_id)
    .bind(&reset.body_step_id)
    .bind(controller_step_id)
    .fetch_one(&mut **tx)
    .await?;
    if direct_body == 0 {
        return Err(DbError::Conflict(
            "loop repeat body is not the controller's active dependency".into(),
        ));
    }

    let active_steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_steps \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let by_id: HashMap<String, AgentExecutionStepRow> = active_steps
        .into_iter()
        .map(|step| (step.step_id.clone(), step))
        .collect();
    if !by_id.contains_key(&reset.body_step_id) {
        return Err(conflict("loop body step"));
    }
    let dependencies: Vec<AgentExecutionStepDependencyRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_step_dependencies \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    for dependency in dependencies {
        outgoing
            .entry(dependency.blocker_step_id)
            .or_default()
            .push(dependency.blocked_step_id);
    }
    let mut closure = HashSet::from([reset.body_step_id.clone()]);
    let mut queue = VecDeque::from([reset.body_step_id.clone()]);
    while let Some(step_id) = queue.pop_front() {
        for downstream in outgoing.get(&step_id).into_iter().flatten() {
            if closure.insert(downstream.clone()) {
                queue.push_back(downstream.clone());
            }
        }
    }
    closure.remove(controller_step_id);

    let expected: HashMap<String, i64> = reset
        .expected_steps
        .iter()
        .map(|step| (step.step_id.clone(), step.expected_step_version))
        .collect();
    if expected.len() != reset.expected_steps.len()
        || expected.keys().cloned().collect::<HashSet<_>>() != closure
    {
        return Err(DbError::Conflict(
            "loop repeat reset must cover the complete active body/downstream closure".into(),
        ));
    }

    let mut reset_ids: Vec<String> = closure.into_iter().collect();
    reset_ids.sort();
    for step_id in reset_ids {
        scoped_stop::ensure_not_stopped_tx(tx, execution_id, &step_id).await?;
        let step = by_id
            .get(&step_id)
            .ok_or_else(|| conflict("loop repeat step"))?;
        if expected.get(&step_id) != Some(&step.version) {
            return Err(conflict("loop repeat step"));
        }
        if !matches!(
            step.status.as_str(),
            "pending" | "completed" | "failed" | "skipped"
        ) {
            return Err(DbError::Conflict(format!(
                "loop repeat cannot reset {step_id} from {}",
                step.status
            )));
        }
        let active_attempt: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_execution_attempts \
             WHERE execution_id = ? AND step_id = ? \
               AND status IN ('queued', 'running', 'waiting_input'))",
        )
        .bind(execution_id)
        .bind(&step_id)
        .fetch_one(&mut **tx)
        .await?;
        if active_attempt != 0 {
            return Err(DbError::Conflict(format!(
                "loop repeat step {step_id} has an active attempt"
            )));
        }
        if step.status == "pending" && step.dispatch_after.is_none() {
            continue;
        }
        let result = sqlx::query(
            "UPDATE agent_execution_steps SET status = 'pending', dispatch_after = NULL, \
                version = version + 1, \
                updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL \
               AND status IN ('pending', 'completed', 'failed', 'skipped')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(&step_id)
        .bind(step.version)
        .execute(&mut **tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("loop repeat step"));
        }
    }
    Ok(())
}

/// Reopen only the skipped downstream nodes whose failed dependency chain is
/// actually repaired by adopting `adopted_step_id`. A shared descendant stays
/// skipped while any other blocker remains failed/skipped/cancelled, so an
/// adoption never resets an unrelated branch.
async fn reopen_adopt_downstream_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    adopted_step_id: &str,
    now: i64,
) -> Result<Vec<String>, DbError> {
    let steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_steps \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut by_id: HashMap<String, AgentExecutionStepRow> = steps
        .into_iter()
        .map(|step| (step.step_id.clone(), step))
        .collect();
    if !by_id.contains_key(adopted_step_id) {
        return Err(conflict("adopted execution step"));
    }
    let dependencies: Vec<AgentExecutionStepDependencyRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_step_dependencies \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    let mut incoming: HashMap<String, Vec<String>> = HashMap::new();
    for dependency in dependencies {
        outgoing
            .entry(dependency.blocker_step_id.clone())
            .or_default()
            .push(dependency.blocked_step_id.clone());
        incoming
            .entry(dependency.blocked_step_id)
            .or_default()
            .push(dependency.blocker_step_id);
    }
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::from([adopted_step_id.to_owned()]);
    while let Some(step_id) = queue.pop_front() {
        for downstream in outgoing.get(&step_id).into_iter().flatten() {
            if reachable.insert(downstream.clone()) {
                queue.push_back(downstream.clone());
            }
        }
    }

    let mut candidates: Vec<String> = reachable.into_iter().collect();
    candidates.sort();
    let mut reopened = Vec::new();
    loop {
        let mut changed = false;
        for step_id in &candidates {
            let Some(step) = by_id.get(step_id) else {
                return Err(conflict("adopt downstream step"));
            };
            if step.status != "skipped" {
                continue;
            }
            if scoped_stop::is_stopped_tx(tx, execution_id, step_id).await? {
                // A dependency repair does not authorize reopening a task
                // whose independently recorded user stop remains in force.
                continue;
            }
            let has_failed_blocker = incoming
                .get(step_id)
                .into_iter()
                .flatten()
                .any(|blocker_id| {
                    by_id.get(blocker_id).is_none_or(|blocker| {
                        matches!(blocker.status.as_str(), "failed" | "skipped" | "cancelled")
                    })
                });
            if has_failed_blocker {
                continue;
            }
            let expected_version = step.version;
            let result = sqlx::query(
                "UPDATE agent_execution_steps SET status = 'pending', dispatch_after = NULL, \
                    version = version + 1, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? AND version = ? \
                   AND superseded_in_revision IS NULL AND status = 'skipped' \
                   AND NOT EXISTS(SELECT 1 FROM agent_execution_attempts attempt \
                                  WHERE attempt.execution_id = agent_execution_steps.execution_id \
                                    AND attempt.step_id = agent_execution_steps.step_id \
                                    AND attempt.status IN ('queued', 'running', 'waiting_input'))",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .bind(expected_version)
            .execute(&mut **tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(conflict("adopt downstream step"));
            }
            let step = by_id
                .get_mut(step_id)
                .ok_or_else(|| conflict("adopt downstream step"))?;
            step.status = "pending".to_owned();
            step.dispatch_after = None;
            step.version += 1;
            step.updated_at = now;
            reopened.push(step_id.clone());
            changed = true;
        }
        if !changed {
            break;
        }
    }
    Ok(reopened)
}

async fn attempt_details_tx(
    tx: &mut Transaction<'_, Sqlite>,
    execution_id: &str,
    step_id: Option<&str>,
) -> Result<Vec<AgentExecutionAttemptDetailRow>, DbError> {
    let attempts = if let Some(step_id) = step_id {
        sqlx::query_as::<_, AgentExecutionAttemptRow>(
            "SELECT * FROM agent_execution_attempts \
             WHERE execution_id = ? AND step_id = ? ORDER BY attempt_no",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_all(&mut **tx)
        .await?
    } else {
        sqlx::query_as::<_, AgentExecutionAttemptRow>(
            "SELECT * FROM agent_execution_attempts \
             WHERE execution_id = ? ORDER BY step_id, attempt_no",
        )
        .bind(execution_id)
        .fetch_all(&mut **tx)
        .await?
    };
    let links: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT step_id, attempt_id, conversation_id \
         FROM conversation_execution_links \
         WHERE execution_id = ? AND relation IN ('attempt', 'automation') \
         ORDER BY active, updated_at",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let conversations: HashMap<(String, String), String> = links
        .into_iter()
        .map(|(step, attempt, conversation)| ((step, attempt), conversation))
        .collect();
    Ok(attempts
        .into_iter()
        .map(|attempt| {
            let conversation_id = conversations
                .get(&(attempt.step_id.clone(), attempt.attempt_id.clone()))
                .cloned();
            AgentExecutionAttemptDetailRow {
                attempt,
                conversation_id,
            }
        })
        .collect())
}

async fn load_step_detail_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
    step_id: &str,
) -> Result<Option<AgentExecutionStepDetailRow>, DbError> {
    let step = sqlx::query_as::<_, AgentExecutionStepRow>(
        "SELECT step.* FROM agent_execution_steps step \
         JOIN agent_executions execution ON execution.execution_id = step.execution_id \
         WHERE step.execution_id = ? AND step.step_id = ? \
           AND step.superseded_in_revision IS NULL AND execution.user_id = ? \
           AND execution.deleted_at IS NULL",
    )
    .bind(execution_id)
    .bind(step_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(step) = step else {
        return Ok(None);
    };
    let mut attempts = attempt_details_tx(tx, execution_id, Some(step_id)).await?;
    let current_attempt = attempts.pop();
    Ok(Some(AgentExecutionStepDetailRow {
        step,
        current_attempt,
    }))
}

async fn load_execution_detail_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    execution_id: &str,
) -> Result<Option<AgentExecutionDetailRows>, DbError> {
    let execution = sqlx::query_as::<_, AgentExecutionRow>(
        "SELECT * FROM agent_executions WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
    )
    .bind(execution_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(execution) = execution else {
        return Ok(None);
    };
    let lead_conversation_id = sqlx::query_scalar(
        "SELECT conversation_id FROM conversation_execution_links \
         WHERE execution_id = ? AND relation = 'lead' \
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(execution_id)
    .fetch_optional(&mut **tx)
    .await?;
    let participants = sqlx::query_as::<_, AgentExecutionParticipantRow>(
        "SELECT * FROM agent_execution_participants \
         WHERE execution_id = ? ORDER BY sort_order, id",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let steps = sqlx::query_as::<_, AgentExecutionStepRow>(
        "SELECT * FROM agent_execution_steps WHERE execution_id = ? ORDER BY created_at, id",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let dependencies = sqlx::query_as::<_, AgentExecutionStepDependencyRow>(
        "SELECT * FROM agent_execution_step_dependencies \
         WHERE execution_id = ? ORDER BY blocker_step_id, blocked_step_id",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let attempts = attempt_details_tx(tx, execution_id, None).await?;
    Ok(Some(AgentExecutionDetailRows {
        execution,
        lead_conversation_id,
        participants,
        steps,
        dependencies,
        attempts,
    }))
}

fn validate_step_append(params: &AppendAgentExecutionStepsParams, event: &NewAgentExecutionEvent, source_followup: bool) -> Result<HashSet<String>, DbError> {
    if params.new_steps.is_empty() {
        return Err(DbError::Conflict(
            "append must introduce at least one Step".to_owned(),
        ));
    }
    if event.event_type != AgentExecutionEventKind::PlanChanged {
        return Err(DbError::Conflict(
            "Step append requires a PlanChanged event".to_owned(),
        ));
    }
    if !source_followup && matches!(
        &event.actor,
        AgentExecutionActor::Agent {
            attempt_id: Some(_),
            ..
        }
    ) {
        return Err(DbError::Conflict(
            "an Attempt Agent must use append_steps_from_attempt".to_owned(),
        ));
    }
    let new_step_ids =
        validate_new_ids("Step", params.new_steps.iter().map(|step| step.step_id.as_str()))?;
    if params
        .new_steps
        .iter()
        .any(|step| step.status != ExecutionStepStatus::Pending)
    {
        return Err(DbError::Conflict(
            "appended Step ids must be unique UUIDv7 values and every new Step must be Pending"
                .to_owned(),
        ));
    }
    validate_dependency_graph(&new_step_ids, &params.new_dependencies)?;

    Ok(new_step_ids)
}

async fn append_steps_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str, execution_id: &str,
    expected_version: i64, params: &AppendAgentExecutionStepsParams, event: &NewAgentExecutionEvent,
    new_step_ids: &HashSet<String>, now: i64) -> Result<AgentExecutionDetailRows, DbError> {
    let (previous_status, current_revision): (String, i64) = sqlx::query_as(
        "SELECT status, plan_revision FROM agent_executions WHERE execution_id = ?",
    )
    .bind(execution_id)
    .fetch_one(&mut **tx)
    .await?;
    let active_steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_steps \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    if active_steps.len() + params.new_steps.len() > MAX_AGENT_EXECUTION_STEPS {
        return Err(DbError::Conflict(format!(
            "active Agent Execution DAG exceeds {MAX_AGENT_EXECUTION_STEPS} steps"
        )));
    }
    let active_participant_ids: HashSet<String> = sqlx::query_scalar(
        "SELECT participant_id FROM agent_execution_participants \
         WHERE execution_id = ? AND retired_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .collect();
    for step in &params.new_steps {
        if step.kind == ExecutionStepKind::Agent
            && !step
                .assigned_participant_id
                .as_ref()
                .is_some_and(|id| active_participant_ids.contains(id))
        {
            return Err(DbError::Conflict(
                "an appended Agent Step must reference an active Participant".to_owned(),
            ));
        }
    }
    let active_dependencies: Vec<AgentExecutionStepDependencyRow> = sqlx::query_as(
        "SELECT * FROM agent_execution_step_dependencies \
         WHERE execution_id = ? AND superseded_in_revision IS NULL",
    )
    .bind(execution_id)
    .fetch_all(&mut **tx)
    .await?;
    let full_step_ids: HashSet<String> = active_steps
        .iter()
        .map(|step| step.step_id.clone())
        .chain(new_step_ids.iter().cloned())
        .collect();
    let full_dependencies: Vec<NewAgentExecutionStepDependency> = active_dependencies
        .iter()
        .map(|dependency| NewAgentExecutionStepDependency {
            blocker_step_id: dependency.blocker_step_id.clone(),
            blocked_step_id: dependency.blocked_step_id.clone(),
        })
        .chain(params.new_dependencies.iter().cloned())
        .collect();
    validate_dependency_graph(&full_step_ids, &full_dependencies)?;

    let new_revision: Option<i64> = sqlx::query_scalar(
        "UPDATE agent_executions SET \
            status = CASE \
                WHEN status IN ('completed', 'completed_with_failures', 'failed') \
                THEN 'running' ELSE status END, \
            plan_revision = plan_revision + 1, version = version + 1, updated_at = ? \
         WHERE execution_id = ? AND user_id = ? AND version = ? AND plan_revision = ? \
           AND deleted_at IS NULL \
           AND status IN (\
               'running', 'paused', 'waiting_input', 'completed', \
               'completed_with_failures', 'failed'\
           ) \
         RETURNING plan_revision",
    )
    .bind(now)
    .bind(execution_id)
    .bind(user_id)
    .bind(expected_version)
    .bind(current_revision)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(new_revision) = new_revision else {
        return Err(conflict("Agent Execution Step append"));
    };
    for step in &params.new_steps {
        insert_step_tx(
            tx,
            execution_id,
            step,
            step.assigned_participant_id.as_deref(),
            0,
            new_revision,
            now,
        )
        .await?;
    }
    for dependency in &params.new_dependencies {
        sqlx::query(
            "INSERT INTO agent_execution_step_dependencies (\
                execution_id, blocker_step_id, blocked_step_id, introduced_in_revision\
             ) VALUES (?, ?, ?, ?)",
        )
        .bind(execution_id)
        .bind(&dependency.blocker_step_id)
        .bind(&dependency.blocked_step_id)
        .bind(new_revision)
        .execute(&mut **tx)
        .await?;
    }
    if is_terminal_execution_status(&previous_status) {
        activate_execution_lead_tx(tx, user_id, execution_id, now).await?;
    }
    append_event_tx(tx, execution_id, event, now).await?;
    let detail = load_execution_detail_tx(tx, user_id, execution_id)
        .await?
        .ok_or_else(|| conflict("Agent Execution"))?;
    Ok(detail)
}

#[async_trait::async_trait]
impl IAgentExecutionRepository for SqliteAgentExecutionRepository {
    async fn execution_attempt_turn_operation_key(&self,user_id:&str,execution_id:&str,step_id:&str,attempt_id:&str)->Result<String,DbError> {
        let mut tx=self.pool.begin().await?;
        let key=crate::execution_attempt_turn_key_tx(&mut tx,user_id,execution_id,step_id,attempt_id).await?;
        tx.commit().await?;Ok(key)
    }
    async fn collaboration_observation(&self, user_id: &str, execution_id: &str, after: Option<i64>)
        -> Result<Option<super::AgentExecutionObservationRows>, DbError> {
        super::agent_execution_observation::observation(&self.pool,user_id,execution_id,after).await
    }

    async fn collaboration_revisions(&self, user_id: &str, execution_ids: &[String], session_ids: &[String])
        -> Result<super::AgentCollaborationObservationRevisions, DbError> {
        super::agent_execution_observation::revisions(&self.pool,user_id,execution_ids,session_ids).await
    }
    async fn create_execution_with_participants(
        &self,
        user_id: &str,
        params: &CreateAgentExecutionParams,
        participants: &[NewAgentExecutionParticipant],
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionRow, DbError> {
        if event.event_type != AgentExecutionEventKind::Created {
            return Err(DbError::Conflict(
                "live Agent Execution creation requires a Created baseline event".to_owned(),
            ));
        }
        if participants.is_empty() {
            return Err(DbError::Conflict(
                "an agent execution requires at least one participant".into(),
            ));
        }
        if participants.len() > MAX_AGENT_EXECUTION_PARTICIPANTS {
            return Err(DbError::Conflict(format!(
                "Agent Execution exceeds {MAX_AGENT_EXECUTION_PARTICIPANTS} active participants"
            )));
        }
        if !(1..=MAX_AGENT_EXECUTION_PARALLELISM).contains(&params.max_parallel) {
            return Err(DbError::Conflict(format!(
                "max_parallel must be between 1 and {MAX_AGENT_EXECUTION_PARALLELISM}"
            )));
        }
        let delegation = delegation_creation_identity(params)?;
        let execution_id = nomifun_common::AgentExecutionId::new().into_string();
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let owner = sqlx::query(
            "UPDATE users SET updated_at = updated_at WHERE user_id = ?",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if owner.rows_affected() == 0 {
            return Err(DbError::NotFound("Agent Execution owner".to_owned()));
        }
        super::agent_execution_admission::require_autowork_admitted_tx(
            &mut tx,user_id,&params.initial_plan_input,params.lead_conversation_id.as_deref()).await?;
        if let Some(identity) = &delegation {
            if let Some(prior) = source_continuation::find_command_tx(&mut tx,user_id,identity).await? {
                tx.commit().await?;
                return Ok(prior);
            }
        }
        if let Some(conversation_id) = params.lead_conversation_id.as_deref() {
            let is_attempt_conversation: i64 = sqlx::query_scalar(
                "SELECT EXISTS( \
                    SELECT 1 FROM conversation_execution_links link \
                    JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                    WHERE link.conversation_id = ? AND link.relation = 'attempt' \
                      AND execution.user_id = ? \
                )",
            )
            .bind(conversation_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if is_attempt_conversation != 0 {
                return Err(DbError::Conflict(
                    "an Attempt Conversation permanently belongs to its Agent Execution and cannot become a lead"
                        .into(),
                ));
            }
        }
        if let Some(conversation_id) = params.lead_conversation_id.as_deref() {
            let active_execution_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(DISTINCT execution.id) \
                   FROM conversation_execution_links link \
                   JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                  WHERE link.conversation_id = ? AND link.relation = 'lead' \
                    AND link.active = 1 AND execution.user_id = ? \
                    AND execution.deleted_at IS NULL \
                    AND execution.status NOT IN ( \
                        'completed', 'completed_with_failures', 'failed', 'cancelled' \
                    )",
            )
            .bind(conversation_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if active_execution_count != 0 {
                return Err(DbError::Conflict(
                    "conversation already has an unfinished Agent Execution".to_owned(),
                ));
            }
        }
        sqlx::query(
            "INSERT INTO agent_executions (\
                execution_id, user_id, goal, status, adaptation_policy, decision_policy, \
                delegation_policy, max_parallel, work_dir, initial_plan_input, \
                version, plan_revision, event_sequence, \
                created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, 0, ?, ?)",
        )
        .bind(&execution_id)
        .bind(user_id)
        .bind(&params.goal)
        .bind(params.status.as_str())
        .bind(params.adaptation_policy.as_str())
        .bind(params.decision_policy.as_str())
        .bind(params.delegation_policy.as_str())
        .bind(params.max_parallel)
        .bind(&params.work_dir)
        .bind(&params.initial_plan_input)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for participant in participants {
            insert_participant_tx(&mut tx, &execution_id, participant, 0, now).await?;
        }
        if let Some(conversation_id) = params.lead_conversation_id.as_deref() {
            switch_current_lead_tx(&mut tx, user_id, &execution_id, conversation_id, now)
                .await?;
        }
        let mut created_event = event.clone();
        let mut payload: serde_json::Value = serde_json::from_str(&created_event.payload)
            .map_err(|error| DbError::Conflict(format!("invalid creation event payload: {error}")))?;
        if let Some(object) = payload.as_object_mut() {
            object.remove("delegation_command_digest");
            if let Some(identity) = delegation {
                object.insert("delegation_command_digest".into(), identity.digest.into());
            }
            created_event.payload = payload.to_string();
        } else if delegation.is_some() {
            return Err(conflict("delegation creation event payload"));
        }
        append_event_tx(&mut tx, &execution_id, &created_event, now).await?;
        let row = sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions WHERE execution_id = ?",
        )
        .bind(&execution_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn get_execution(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<AgentExecutionRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn get_execution_detail(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Option<AgentExecutionDetailRows>, DbError> {
        let mut tx = self.pool.begin().await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id).await?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn list_executions(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AgentExecutionRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions WHERE user_id = ? AND deleted_at IS NULL \
             ORDER BY updated_at DESC, id LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(limit.clamp(1, 500))
        .bind(offset.max(0))
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_recoverable_executions(
        &self,
        statuses: &[AgentExecutionStatus],
    ) -> Result<Vec<AgentExecutionRow>, DbError> {
        if statuses.is_empty() {
            return Ok(Vec::new());
        }
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT * FROM agent_executions WHERE deleted_at IS NULL AND status IN (",
        );
        let mut separated = builder.separated(", ");
        for status in statuses {
            separated.push_bind(status.as_str());
        }
        separated.push_unseparated(")");
        builder.push(" ORDER BY updated_at, id");
        Ok(builder
            .build_query_as::<AgentExecutionRow>()
            .fetch_all(&self.pool)
            .await?)
    }

    async fn update_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        lease: Option<&AgentExecutionLeaseToken>,
        params: &UpdateAgentExecutionParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionRow, DbError> {
        if params.status == Some(AgentExecutionStatus::Paused) {
            return Err(DbError::Conflict(
                "pause must use the atomic Agent Execution pause transition".to_owned(),
            ));
        }
        let now = now_ms();
        let requested_status = params.status.map(|status| status.as_str());
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, lease, now).await?;
        let result = sqlx::query(
            "UPDATE agent_executions SET \
                goal = COALESCE(?, goal), status = COALESCE(?, status), \
                lease_owner = CASE WHEN COALESCE(?, status) IN ('running', 'waiting_input', 'paused') \
                                   THEN lease_owner ELSE NULL END, \
                lease_expires_at = CASE WHEN COALESCE(?, status) IN ('running', 'waiting_input', 'paused') \
                                        THEN lease_expires_at ELSE NULL END, \
                max_parallel = COALESCE(?, max_parallel), \
                work_dir = CASE WHEN ? THEN ? ELSE work_dir END, \
                summary = CASE WHEN ? THEN ? ELSE summary END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND (? IS NULL OR ? = status OR status NOT IN (\
                    'completed', 'completed_with_failures', 'failed', 'cancelled'\
               )) \
               AND NOT (status = 'paused' AND ?)",
        )
        .bind(&params.goal)
        .bind(requested_status)
        .bind(requested_status)
        .bind(requested_status)
        .bind(params.max_parallel)
        .bind(params.work_dir.is_some())
        .bind(params.work_dir.as_ref().and_then(|value| value.as_deref()))
        .bind(params.summary.is_some())
        .bind(params.summary.as_ref().and_then(|value| value.as_deref()))
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .bind(requested_status)
        .bind(requested_status)
        .bind(params.status.is_some())
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        append_event_tx(&mut tx, execution_id, event, now).await?;
        let row = sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn pause_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;

        // Acquire the same writer as dispatch. A start either committed before
        // this fence and keeps its original owner, or is rejected while paused.
        let result = sqlx::query(
            "UPDATE agent_executions SET status = 'paused', version = version + 1, \
                updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }

        // Queued reservations have not crossed start_attempt and are the only
        // attempts that can safely release their Step back to Pending.
        sqlx::query(
            "UPDATE agent_execution_attempts SET status = 'cancelled', \
                question = NULL, finished_at = ?, version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND status = 'queued'",
        )
        .bind(now)
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
             WHERE execution_id = ? AND relation IN ('attempt', 'automation') AND active = 1 \
               AND EXISTS(SELECT 1 FROM agent_execution_attempts attempt \
                          WHERE attempt.execution_id = conversation_execution_links.execution_id \
                            AND attempt.step_id = conversation_execution_links.step_id \
                            AND attempt.attempt_id = conversation_execution_links.attempt_id \
                            AND attempt.status = 'cancelled')",
        )
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;

        append_event_tx(&mut tx, execution_id, event, now).await?;
        let row = sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn resume_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE agent_executions SET \
                status = CASE WHEN EXISTS( \
                    SELECT 1 FROM agent_execution_attempts attempt \
                    WHERE attempt.execution_id = agent_executions.execution_id \
                      AND attempt.status = 'waiting_input' \
                ) THEN 'waiting_input' ELSE 'running' END, \
                version = version + 1, \
                updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status = 'paused' \
               AND NOT EXISTS( \
                   SELECT 1 FROM agent_execution_attempts attempt \
                   WHERE attempt.execution_id = agent_executions.execution_id \
                     AND attempt.status = 'waiting_input' \
                     AND json_valid(attempt.runtime_state) \
                     AND json_type(attempt.runtime_state, '$.review_blocked') IS NOT NULL \
               )",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        append_event_tx(&mut tx, execution_id, event, now).await?;
        let row = sqlx::query_as::<_, AgentExecutionRow>(
            "SELECT * FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn cancel_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE agent_executions SET status = 'cancelled', version = version + 1, \
                lease_owner = NULL, lease_expires_at = NULL, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? \
               AND deleted_at IS NULL \
               AND status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        terminate_unfinished_execution_children_tx(&mut tx, execution_id, None, now).await?;
        append_event_tx(&mut tx, execution_id, event, now).await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("agent execution"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn cancel_step(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_execution_version: i64,
        step_id: &str,
        expected_step_version: i64,
        scope: AgentExecutionCancellationScope,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        scoped_stop::cancel_step(&self.pool, user_id, execution_id,
            expected_execution_version, step_id, expected_step_version,
            scope, event).await
    }

    async fn fail_active_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        lease: &AgentExecutionLeaseToken,
        reason: &str,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        if reason.trim().is_empty() {
            return Err(DbError::Conflict(
                "fatal Agent Execution failure requires a reason".to_owned(),
            ));
        }
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, Some(lease), now).await?;
        let result = sqlx::query(
            "UPDATE agent_executions SET status = 'failed', summary = ?, \
                version = version + 1, lease_owner = NULL, lease_expires_at = NULL, \
                updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? \
               AND deleted_at IS NULL AND status IN ('running', 'waiting_input')",
        )
        .bind(reason)
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("active Agent Execution failure"));
        }
        terminate_unfinished_execution_children_tx(
            &mut tx,
            execution_id,
            Some(reason),
            now,
        )
        .await?;
        append_event_tx(&mut tx, execution_id, event, now).await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("active Agent Execution failure"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn delete_execution(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        event: &NewAgentExecutionEvent,
    ) -> Result<bool, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        // Claim the exact aggregate version before mutating any child row.  All
        // following writes share this transaction, so deletion is all-or-none.
        let result = sqlx::query(
            "UPDATE agent_executions SET \
                status = CASE WHEN status IN (\
                    'completed', 'completed_with_failures', 'failed', 'cancelled'\
                ) THEN status ELSE 'cancelled' END, \
                version = version + 1, lease_owner = NULL, lease_expires_at = NULL, \
                updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            let exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agent_executions \
                 WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL)",
            )
            .bind(execution_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if exists != 0 {
                return Err(conflict("agent execution"));
            }
            tx.rollback().await?;
            return Ok(false);
        }
        let stopped_steps: Vec<String> = sqlx::query_scalar(
            "UPDATE agent_execution_steps SET status = 'cancelled', dispatch_after = NULL, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND superseded_in_revision IS NULL \
               AND status NOT IN ('completed', 'failed', 'skipped', 'cancelled') RETURNING step_id",
        )
        .bind(now)
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        for step_id in stopped_steps {
            requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, &step_id, now).await?;
        }
        sqlx::query(
            "UPDATE agent_execution_attempts SET \
                status = 'cancelled', \
                question = NULL, finished_at = ?, version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND status IN ('queued', 'running', 'waiting_input')",
        )
        .bind(now)
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
             WHERE execution_id = ? AND active = 1",
        )
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;
        append_event_tx(&mut tx, execution_id, event, now).await?;
        sqlx::query(
            "UPDATE agent_executions SET deleted_at = ?, updated_at = ? WHERE execution_id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn try_acquire_lease(
        &self,
        execution_id: &str,
        expected_version: i64,
        owner: &str,
        expires_at: i64,
    ) -> Result<Option<AgentExecutionRow>, DbError> {
        let now = now_ms();
        if owner.trim().is_empty() || expires_at <= now {
            return Err(DbError::Conflict(
                "lease owner must be non-empty and expiry must be in the future".into(),
            ));
        }
        Ok(sqlx::query_as::<_, AgentExecutionRow>(
            "UPDATE agent_executions SET lease_owner = ?, lease_expires_at = ? \
             WHERE execution_id = ? AND version = ? AND (\
                lease_owner IS NULL OR lease_expires_at <= ? OR lease_owner = ?\
             ) AND deleted_at IS NULL AND status IN ('running', 'waiting_input', 'paused') \
             RETURNING *",
        )
        .bind(owner)
        .bind(expires_at)
        .bind(execution_id)
        .bind(expected_version)
        .bind(now)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn renew_lease(
        &self,
        execution_id: &str,
        owner: &str,
        expected_expires_at: i64,
        expires_at: i64,
    ) -> Result<Option<AgentExecutionRow>, DbError> {
        if owner.trim().is_empty() || expires_at <= now_ms() {
            return Err(DbError::Conflict(
                "lease owner must be non-empty and expiry must be in the future".into(),
            ));
        }
        Ok(sqlx::query_as::<_, AgentExecutionRow>(
             "UPDATE agent_executions SET lease_expires_at = ? \
             WHERE execution_id = ? AND lease_owner = ? AND lease_expires_at = ? \
               AND deleted_at IS NULL AND status IN ('running', 'waiting_input', 'paused') RETURNING *",
        )
        .bind(expires_at)
        .bind(execution_id)
        .bind(owner)
        .bind(expected_expires_at)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn release_lease(
        &self,
        execution_id: &str,
        owner: &str,
        expected_expires_at: i64,
    ) -> Result<Option<AgentExecutionRow>, DbError> {
        if owner.trim().is_empty() {
            return Err(DbError::Conflict("lease owner must be non-empty".into()));
        }
        Ok(sqlx::query_as::<_, AgentExecutionRow>(
            "UPDATE agent_executions SET lease_owner = NULL, lease_expires_at = NULL \
             WHERE execution_id = ? AND lease_owner = ? AND lease_expires_at = ? \
               AND deleted_at IS NULL RETURNING *",
        )
        .bind(execution_id)
        .bind(owner)
        .bind(expected_expires_at)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn reconcile_plan(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        params: &ReconcileAgentExecutionPlanParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let execution = sqlx::query(
            "UPDATE agent_executions SET version = version \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution.rows_affected() == 0 {
            return Err(conflict("agent execution plan"));
        }
        if !params.new_steps.is_empty()
            && let AgentExecutionActor::Agent { attempt_id: Some(attempt_id), .. } = &event.actor {
            let caller: String = sqlx::query_scalar(
                "SELECT step_id FROM agent_execution_attempts WHERE execution_id=? AND attempt_id=?",
            ).bind(execution_id).bind(attempt_id).fetch_optional(&mut *tx).await?
                .ok_or_else(|| conflict("replanning Attempt"))?;
            scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, &caller).await?;
            scoped_stop::ensure_delegation_not_stopped_tx(&mut tx, execution_id, &caller).await?;
        }
        let active_steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
            "SELECT step.* FROM agent_execution_steps step \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.superseded_in_revision IS NULL \
               AND execution.user_id = ? AND execution.deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        let active_step_ids: HashSet<String> = active_steps
            .iter()
            .map(|step| step.step_id.clone())
            .collect();
        let keep_step_ids: HashSet<String> = params.keep_step_ids.iter().cloned().collect();
        if keep_step_ids.len() != params.keep_step_ids.len()
            || !keep_step_ids.is_subset(&active_step_ids)
        {
            return Err(DbError::Conflict(
                "replan keep_step_ids contain duplicates or non-active steps".into(),
            ));
        }
        let historical_step_ids: HashSet<String> = sqlx::query_scalar(
            "SELECT step_id FROM agent_execution_steps WHERE execution_id = ?",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let new_step_ids =
            validate_new_ids("Step", params.new_steps.iter().map(|step| step.step_id.as_str()))?;
        if !new_step_ids.is_disjoint(&historical_step_ids) {
            return Err(DbError::Conflict(
                "replan cannot reuse a historical Step id".into(),
            ));
        }
        let final_step_ids: HashSet<String> = keep_step_ids
            .iter()
            .chain(new_step_ids.iter())
            .cloned()
            .collect();
        if final_step_ids.is_empty() {
            return Err(DbError::Conflict(
                "an execution plan requires at least one active step".into(),
            ));
        }
        if final_step_ids.len() > MAX_AGENT_EXECUTION_STEPS {
            return Err(DbError::Conflict(format!(
                "active Agent Execution DAG exceeds {MAX_AGENT_EXECUTION_STEPS} steps"
            )));
        }
        validate_dependency_graph(&final_step_ids, &params.new_dependencies)?;

        let active_participants: Vec<AgentExecutionParticipantRow> = sqlx::query_as(
            "SELECT * FROM agent_execution_participants \
             WHERE execution_id = ? AND retired_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        let active_participant_ids: HashSet<String> = active_participants
            .iter()
            .map(|participant| participant.participant_id.clone())
            .collect();
        let retire_ids: HashSet<String> = params.retire_participant_ids.iter().cloned().collect();
        if retire_ids.len() != params.retire_participant_ids.len()
            || !retire_ids.is_subset(&active_participant_ids)
        {
            return Err(DbError::Conflict(
                "replan retire_participant_ids contain duplicates or non-active participants".into(),
            ));
        }
        let historical_participant_ids: HashSet<String> = sqlx::query_scalar(
            "SELECT participant_id FROM agent_execution_participants WHERE execution_id = ?",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let new_participant_ids = validate_new_ids(
            "Participant",
            params
                .new_participants
                .iter()
                .map(|participant| participant.participant_id.as_str()),
        )?;
        if !new_participant_ids.is_disjoint(&historical_participant_ids) {
            return Err(DbError::Conflict(
                "replan cannot reuse a historical Participant id".into(),
            ));
        }
        let final_participant_ids: HashSet<String> = active_participant_ids
            .difference(&retire_ids)
            .chain(new_participant_ids.iter())
            .cloned()
            .collect();
        if final_participant_ids.is_empty() {
            return Err(DbError::Conflict(
                "an active execution plan requires at least one participant".into(),
            ));
        }
        if final_participant_ids.len() > MAX_AGENT_EXECUTION_PARTICIPANTS {
            return Err(DbError::Conflict(format!(
                "Agent Execution exceeds {MAX_AGENT_EXECUTION_PARTICIPANTS} active participants"
            )));
        }
        for step in active_steps
            .iter()
            .filter(|step| keep_step_ids.contains(&step.step_id))
        {
            if step.kind == "agent"
                && !step
                    .assigned_participant_id
                    .as_ref()
                    .is_some_and(|id| final_participant_ids.contains(id))
            {
                return Err(DbError::Conflict(
                    "a kept agent step references a retired participant".into(),
                ));
            }
        }
        for step in &params.new_steps {
            if step.kind == ExecutionStepKind::Agent
                && !step
                    .assigned_participant_id
                    .as_ref()
                    .is_some_and(|id| final_participant_ids.contains(id))
            {
                return Err(DbError::Conflict(
                    "a new agent step must reference an active participant".into(),
                ));
            }
        }

        let mut superseded_step_ids: Vec<String> = active_step_ids
            .difference(&keep_step_ids)
            .cloned()
            .collect();
        superseded_step_ids.sort();

        let new_revision: Option<i64> = sqlx::query_scalar(
            "UPDATE agent_executions SET \
                goal = COALESCE(?, goal), \
                adaptation_policy = COALESCE(?, adaptation_policy), \
                decision_policy = COALESCE(?, decision_policy), \
                delegation_policy = COALESCE(?, delegation_policy), \
                status = ?, \
                lease_owner = CASE WHEN ? IN ('running', 'waiting_input', 'paused') THEN lease_owner ELSE NULL END, \
                lease_expires_at = CASE WHEN ? IN ('running', 'waiting_input', 'paused') THEN lease_expires_at ELSE NULL END, \
                plan_revision = plan_revision + 1, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled') \
             RETURNING plan_revision",
        )
        .bind(&params.goal)
        .bind(params.adaptation_policy.map(|value| value.as_str()))
        .bind(params.decision_policy.map(|value| value.as_str()))
        .bind(params.delegation_policy.map(|value| value.as_str()))
        .bind(params.execution_status.as_str())
        .bind(params.execution_status.as_str())
        .bind(params.execution_status.as_str())
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(new_revision) = new_revision else {
            return Err(conflict("agent execution plan"));
        };

        sqlx::query(
            "UPDATE agent_execution_step_dependencies \
             SET superseded_in_revision = ? \
             WHERE execution_id = ? AND superseded_in_revision IS NULL",
        )
        .bind(new_revision)
        .bind(execution_id)
        .execute(&mut *tx)
        .await?;
        for step_id in &superseded_step_ids {
            sqlx::query(
                "UPDATE agent_execution_attempts SET \
                    status = CASE status WHEN 'queued' THEN 'cancelled' ELSE 'interrupted' END, \
                    question = NULL, finished_at = ?, version = version + 1, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? \
                   AND status IN ('queued', 'running', 'waiting_input')",
            )
            .bind(now)
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? \
                   AND relation IN ('attempt', 'automation') AND active = 1",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE agent_execution_steps SET \
                    status = CASE \
                        WHEN status = 'pending' THEN 'skipped' \
                        WHEN status IN ('running', 'waiting_input') THEN 'cancelled' \
                        ELSE status \
                    END, \
                    dispatch_after = NULL, \
                    superseded_in_revision = ?, version = version + 1, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? AND superseded_in_revision IS NULL",
            )
            .bind(new_revision)
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .execute(&mut *tx)
            .await?;
            // A removed/replaced unfinished Step cannot continue its backoff
            // purpose. Its old immutable Attempts retain their own targets.
            if active_steps.iter().any(|step| &step.step_id == step_id
                && !matches!(step.status.as_str(), "completed" | "failed" | "skipped" | "cancelled")) {
                requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, step_id, now).await?;
            }
        }
        for participant_id in &params.retire_participant_ids {
            sqlx::query(
                "UPDATE agent_execution_participants SET retired_in_revision = ? \
                 WHERE execution_id = ? AND participant_id = ? AND retired_in_revision IS NULL",
            )
            .bind(new_revision)
            .bind(execution_id)
            .bind(participant_id)
            .execute(&mut *tx)
            .await?;
        }
        for participant in &params.new_participants {
            insert_participant_tx(&mut tx, execution_id, participant, new_revision, now).await?;
        }
        for step in &params.new_steps {
            insert_step_tx(
                &mut tx,
                execution_id,
                step,
                step.assigned_participant_id.as_deref(),
                0,
                new_revision,
                now,
            )
            .await?;
        }
        for dependency in &params.new_dependencies {
            sqlx::query(
                "INSERT INTO agent_execution_step_dependencies (\
                    execution_id, blocker_step_id, blocked_step_id, introduced_in_revision\
                 ) VALUES (?, ?, ?, ?)",
            )
            .bind(execution_id)
            .bind(&dependency.blocker_step_id)
            .bind(&dependency.blocked_step_id)
            .bind(new_revision)
            .execute(&mut *tx)
            .await?;
        }
        let mut event = event.clone();
        if event.event_type == AgentExecutionEventKind::PlanChanged {
            let mut payload: serde_json::Value = serde_json::from_str(&event.payload)
                .map_err(|_| conflict("plan reconciliation event payload"))?;
            let object = payload.as_object_mut().ok_or_else(|| conflict("plan reconciliation event payload"))?;
            // The original event author records its actual selected revision
            // and removed Steps. A cleanup reader must not infer these from
            // unrelated historical plans or accept caller-supplied identities.
            object.insert("plan_revision".into(), new_revision.into());
            object.insert("superseded_step_ids".into(), serde_json::json!(superseded_step_ids));
            event.payload = payload.to_string();
        }
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("agent execution"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn find_steps_append_from_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        operation_id: &str,
    ) -> Result<Option<AppendAgentExecutionStepsFromAttemptResult>, DbError> {
        if operation_id.trim().is_empty() {
            return Err(DbError::Conflict(
                "delegation operation_id must not be empty".to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let payloads: Vec<String> = sqlx::query_scalar(
            "SELECT event.payload \
             FROM agent_execution_events event \
             JOIN agent_executions execution ON execution.execution_id = event.execution_id \
             WHERE event.execution_id = ? AND execution.user_id = ? \
               AND execution.deleted_at IS NULL \
               AND event.event_type = 'plan_changed' AND event.actor_type = 'agent' \
               AND event.actor_conversation_id IS NOT NULL \
               AND event.actor_attempt_id IS NOT NULL \
               AND event.attempt_id = event.actor_attempt_id \
               AND json_extract(event.payload, '$.operation_id') = ? \
             ORDER BY event.sequence LIMIT 2",
        )
        .bind(execution_id)
        .bind(user_id)
        .bind(operation_id)
        .fetch_all(&mut *tx)
        .await?;
        let payload = match payloads.as_slice() {
            [] => {
                tx.commit().await?;
                return Ok(None);
            }
            [payload] => payload,
            _ => {
                return Err(DbError::Conflict(
                    "delegation operation is not unique".to_owned(),
                ));
            }
        };
        let added_step_ids = delegation_added_step_ids(payload)?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("Agent Execution"))?;
        tx.commit().await?;
        Ok(Some(AppendAgentExecutionStepsFromAttemptResult {
            detail,
            added_step_ids,
        }))
    }

    async fn append_steps_from_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        params: &AppendAgentExecutionStepsFromAttemptParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AppendAgentExecutionStepsFromAttemptResult, DbError> {
        if params.operation_id.trim().is_empty() {
            return Err(DbError::Conflict(
                "delegation operation_id must not be empty".to_owned(),
            ));
        }
        if event.event_type != AgentExecutionEventKind::PlanChanged {
            return Err(DbError::Conflict(
                "delegation append requires a PlanChanged event".to_owned(),
            ));
        }
        match &event.actor {
            AgentExecutionActor::Agent {
                conversation_id: Some(conversation_id),
                attempt_id: Some(attempt_id),
                ..
            } if conversation_id == &params.caller_conversation_id
                && attempt_id == &params.caller_attempt_id => {}
            _ => {
                return Err(DbError::Conflict(
                    "delegation append event must be attributed to the calling Attempt"
                        .to_owned(),
                ));
            }
        }

        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        // Acquire SQLite's write lock before the replay lookup. A lost-response
        // retry may arrive after the Attempt settled, so owner/existence is the
        // only predicate at this stage; first-time writes prove active caller
        // authority below.
        let locked = sqlx::query(
            "UPDATE agent_executions SET version = version \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() != 1 {
            return Err(conflict("delegating Agent Execution"));
        }

        let replay_rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT payload, actor_conversation_id, actor_attempt_id \
             FROM agent_execution_events \
             WHERE execution_id = ? AND event_type = 'plan_changed' \
               AND json_extract(payload, '$.operation_id') = ? \
             ORDER BY sequence LIMIT 2",
        )
        .bind(execution_id)
        .bind(&params.operation_id)
        .fetch_all(&mut *tx)
        .await?;
        if let [(payload, actor_conversation_id, actor_attempt_id)] = replay_rows.as_slice() {
            if actor_conversation_id.as_deref()
                != Some(params.caller_conversation_id.as_str())
                || actor_attempt_id.as_deref() != Some(params.caller_attempt_id.as_str())
            {
                return Err(DbError::Conflict(
                    "delegation operation belongs to another Attempt".to_owned(),
                ));
            }
            let added_step_ids = delegation_added_step_ids(payload)?;
            let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
                .await?
                .ok_or_else(|| conflict("Agent Execution"))?;
            tx.commit().await?;
            return Ok(AppendAgentExecutionStepsFromAttemptResult {
                detail,
                added_step_ids,
            });
        }
        if !replay_rows.is_empty() {
            return Err(DbError::Conflict(
                "delegation operation is not unique".to_owned(),
            ));
        }

        if params.new_steps.is_empty() {
            return Err(DbError::Conflict(
                "delegation must append at least one Step".to_owned(),
            ));
        }
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, &params.caller_step_id).await?;
        scoped_stop::ensure_delegation_not_stopped_tx(&mut tx, execution_id, &params.caller_step_id).await?;
        let new_step_ids =
            validate_new_ids("Step", params.new_steps.iter().map(|step| step.step_id.as_str()))?;
        if params
            .new_steps
            .iter()
            .any(|step| step.status != ExecutionStepStatus::Pending)
        {
            return Err(DbError::Conflict(
                "delegated Step ids must be unique UUIDv7 values and every new Step must be Pending"
                    .to_owned(),
            ));
        }
        // This also proves that dependency endpoints are batch-local. The
        // caller/downstream edges are derived below and cannot be supplied by
        // an Agent payload.
        validate_dependency_graph(&new_step_ids, &params.new_dependencies)?;

        let caller_rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT step.delegation_depth, execution.plan_revision \
             FROM agent_execution_steps step \
             JOIN agent_execution_attempts attempt \
               ON attempt.execution_id = step.execution_id AND attempt.step_id = step.step_id \
             JOIN conversation_execution_links link \
               ON link.execution_id = attempt.execution_id \
              AND link.step_id = attempt.step_id AND link.attempt_id = attempt.attempt_id \
             JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.step_id = ? AND step.version = ? \
               AND step.superseded_in_revision IS NULL AND step.status = 'running' \
               AND step.kind = 'agent' \
               AND attempt.attempt_id = ? AND attempt.version = ? AND attempt.status = 'running' \
               AND link.conversation_id = ? \
               AND link.relation IN ('attempt', 'automation') AND link.active = 1 \
               AND session.state = 'live' \
               AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
               AND json_extract(session.owner_ref_json, '$.principal_id') = ? \
               AND execution.user_id = ? \
               AND execution.deleted_at IS NULL \
               AND execution.status IN ('running', 'waiting_input') \
               AND execution.delegation_policy <> 'disabled' \
             ORDER BY link.id LIMIT 2",
        )
        .bind(execution_id)
        .bind(&params.caller_step_id)
        .bind(params.expected_caller_step_version)
        .bind(&params.caller_attempt_id)
        .bind(params.expected_caller_attempt_version)
        .bind(&params.caller_conversation_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;
        let [(caller_depth, current_revision)] = caller_rows.as_slice() else {
            return Err(conflict("delegating Agent Attempt"));
        };
        if *caller_depth >= MAX_AGENT_DELEGATION_DEPTH {
            return Err(DbError::Conflict(format!(
                "Agent delegation depth cannot exceed {MAX_AGENT_DELEGATION_DEPTH}"
            )));
        }
        let appended_depth = *caller_depth + 1;

        let active_steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
            "SELECT * FROM agent_execution_steps \
             WHERE execution_id = ? AND superseded_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        if active_steps.len() + params.new_steps.len() > MAX_AGENT_EXECUTION_STEPS {
            return Err(DbError::Conflict(format!(
                "active Agent Execution DAG exceeds {MAX_AGENT_EXECUTION_STEPS} steps"
            )));
        }
        let active_participant_ids: HashSet<String> = sqlx::query_scalar(
            "SELECT participant_id FROM agent_execution_participants \
             WHERE execution_id = ? AND retired_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        for step in &params.new_steps {
            if step.kind == ExecutionStepKind::Agent
                && !step
                    .assigned_participant_id
                    .as_ref()
                    .is_some_and(|id| active_participant_ids.contains(id))
            {
                return Err(DbError::Conflict(
                    "a delegated Agent Step must reference an active Participant".to_owned(),
                ));
            }
        }

        let active_dependencies: Vec<AgentExecutionStepDependencyRow> = sqlx::query_as(
            "SELECT * FROM agent_execution_step_dependencies \
             WHERE execution_id = ? AND superseded_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        let pending_step_ids: HashSet<String> = active_steps
            .iter()
            .filter(|step| step.status == ExecutionStepStatus::Pending.as_str())
            .map(|step| step.step_id.clone())
            .collect();
        let caller_pending_downstream: HashSet<String> = active_dependencies
            .iter()
            .filter(|dependency| {
                dependency.blocker_step_id == params.caller_step_id
                    && pending_step_ids.contains(&dependency.blocked_step_id)
            })
            .map(|dependency| dependency.blocked_step_id.clone())
            .collect();

        let internal_blockers: HashSet<String> = params
            .new_dependencies
            .iter()
            .map(|dependency| dependency.blocker_step_id.clone())
            .collect();
        let leaf_step_ids: Vec<String> = params
            .new_steps
            .iter()
            .filter(|step| !internal_blockers.contains(&step.step_id))
            .map(|step| step.step_id.clone())
            .collect();
        let mut derived_dependencies = Vec::new();
        for leaf_step_id in &leaf_step_ids {
            for downstream_step_id in &caller_pending_downstream {
                derived_dependencies.push(NewAgentExecutionStepDependency {
                    blocker_step_id: leaf_step_id.clone(),
                    blocked_step_id: downstream_step_id.clone(),
                });
            }
        }

        let full_step_ids: HashSet<String> = active_steps
            .iter()
            .map(|step| step.step_id.clone())
            .chain(new_step_ids.iter().cloned())
            .collect();
        let full_dependencies: Vec<NewAgentExecutionStepDependency> = active_dependencies
            .iter()
            .map(|dependency| NewAgentExecutionStepDependency {
                blocker_step_id: dependency.blocker_step_id.clone(),
                blocked_step_id: dependency.blocked_step_id.clone(),
            })
            .chain(params.new_dependencies.iter().cloned())
            .chain(derived_dependencies.iter().cloned())
            .collect();
        validate_dependency_graph(&full_step_ids, &full_dependencies)?;

        let new_revision: Option<i64> = sqlx::query_scalar(
            "UPDATE agent_executions SET plan_revision = plan_revision + 1, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND plan_revision = ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input') \
               AND delegation_policy <> 'disabled' \
             RETURNING plan_revision",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(current_revision)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(new_revision) = new_revision else {
            return Err(conflict("delegating Agent Execution plan"));
        };
        for step in &params.new_steps {
            insert_step_tx(
                &mut tx,
                execution_id,
                step,
                step.assigned_participant_id.as_deref(),
                appended_depth,
                new_revision,
                now,
            )
            .await?;
        }
        let added_step_ids: Vec<String> =
            params.new_steps.iter().map(|step| step.step_id.clone()).collect();
        for dependency in params
            .new_dependencies
            .iter()
            .chain(derived_dependencies.iter())
        {
            sqlx::query(
                "INSERT INTO agent_execution_step_dependencies (\
                    execution_id, blocker_step_id, blocked_step_id, introduced_in_revision\
                 ) VALUES (?, ?, ?, ?)",
            )
            .bind(execution_id)
            .bind(&dependency.blocker_step_id)
            .bind(&dependency.blocked_step_id)
            .bind(new_revision)
            .execute(&mut *tx)
            .await?;
        }
        let mut event_payload: serde_json::Value = serde_json::from_str(&event.payload)
            .map_err(|_| DbError::Conflict("delegation event payload must be valid JSON".into()))?;
        let event_payload = event_payload.as_object_mut().ok_or_else(|| {
            DbError::Conflict("delegation event payload must be a JSON object".into())
        })?;
        event_payload.insert(
            "operation_id".to_owned(),
            serde_json::Value::String(params.operation_id.clone()),
        );
        event_payload.insert(
            "added_step_ids".to_owned(),
            serde_json::to_value(&added_step_ids).map_err(|error| {
                DbError::Conflict(format!("encode delegated Step ids: {error}"))
            })?,
        );
        let mut event = scoped_event(
            event,
            &params.caller_step_id,
            Some(&params.caller_attempt_id),
        );
        event.payload = serde_json::to_string(&event_payload).map_err(|error| {
            DbError::Conflict(format!("encode delegation event payload: {error}"))
        })?;
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("Agent Execution"))?;
        tx.commit().await?;
        Ok(AppendAgentExecutionStepsFromAttemptResult {
            detail,
            added_step_ids,
        })
    }

    async fn append_steps(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        params: &AppendAgentExecutionStepsParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        let new_step_ids = validate_step_append(params, event, false)?;
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE agent_executions SET version = version \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status IN (\
                   'running', 'paused', 'waiting_input', 'completed', \
                   'completed_with_failures', 'failed'\
               )",
        )
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?;
        if locked.rows_affected() != 1 {
            return Err(conflict("Agent Execution Step append"));
        }
        let detail = append_steps_tx(&mut tx, user_id, execution_id, expected_version, params, event, &new_step_ids, now).await?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn find_source_command(&self, user_id: &str, command: &CreateAgentExecutionParams)
        -> Result<Option<AgentExecutionRow>, DbError> {
        let Some(identity)=delegation_creation_identity(command)? else {return Ok(None);};
        let mut tx=self.pool.begin().await?;
        let row=source_continuation::find_command_tx(&mut tx,user_id,&identity).await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn append_source_steps(&self,user_id:&str,execution_id:&str,expected_version:i64,
        source:&AgentExecutionSourceAppend,params:&AppendAgentExecutionStepsParams,event:&NewAgentExecutionEvent)
        ->Result<AppendAgentExecutionStepsFromAttemptResult,DbError> {
        source_continuation::append(&self.pool,user_id,execution_id,expected_version,source,params,event).await
    }

    async fn continuation_target(&self,user_id:&str,execution_id:&str,step_id:&str)->Result<Option<crate::AgentExecutionContinuationTarget>,DbError> {
        source_continuation::target(&self.pool,user_id,execution_id,step_id).await
    }

    async fn original_child_binding(&self,user_id:&str,execution_id:&str,target:&crate::AgentExecutionContinuationTarget)
        ->Result<nomifun_agent_contracts::AgentHandoffBindingRefV1,DbError> {
        let mut tx=self.pool.begin().await?;
        let binding=source_continuation::original_child_binding_tx(&mut tx,user_id,execution_id,target).await?;
        tx.commit().await?;Ok(binding)
    }

    async fn step_source(&self,user_id:&str,execution_id:&str,step_id:&str)->Result<Option<nomifun_common::AgentExecutionSource>,DbError> {
        source_continuation::step_source(&self.pool,user_id,execution_id,step_id).await
    }

    async fn step_session_available(&self,user_id:&str,execution_id:&str,step_id:&str)->Result<bool,DbError> {
        let mut tx=self.pool.begin().await?;
        let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL)")
            .bind(execution_id).bind(user_id).fetch_one(&mut *tx).await?;
        if !owned {return Err(conflict("Step Session admission owner"));}
        let available=super::agent_execution_admission::continuation_input_available_tx(&mut tx,execution_id,step_id).await?;
        tx.commit().await?;Ok(available)
    }

    async fn source_reply_turns(&self,user_id:&str,session_id:&str,current_turn:&str)->Result<Vec<String>,DbError> {
        let mut tx=self.pool.begin().await?;
        let turns=crate::agent_reply_source_turns_tx(&mut tx,user_id,session_id,current_turn).await?;
        tx.commit().await?;Ok(turns)
    }

    async fn step_message_source(&self,user_id:&str,execution_id:&str,step_id:&str)->Result<Option<crate::AgentExecutionMessageSource>,DbError> {
        let payload:Option<String>=sqlx::query_scalar("SELECT json_extract(event.payload,'$.message_source') FROM agent_execution_events event JOIN agent_executions execution ON execution.execution_id=event.execution_id WHERE event.execution_id=? AND execution.user_id=? AND execution.deleted_at IS NULL AND event.event_type='plan_changed' AND json_extract(event.payload,'$.continuation_step_id')=? AND json_type(event.payload,'$.message_source')='object' ORDER BY event.sequence LIMIT 1")
            .bind(execution_id).bind(user_id).bind(step_id).fetch_optional(&self.pool).await?;
        payload.map(|value|serde_json::from_str(&value).map_err(|error|DbError::Init(format!("invalid Step message source: {error}")))).transpose()
    }

    async fn redirect_closed_steer(&self,user_id:&str,execution_id:&str,step_id:&str,attempt_id:&str,
        operation_id:&str,closed:&crate::AgentExecutionClosedTurn)->Result<(),DbError> {
        source_continuation::redirect_closed_steer(&self.pool,user_id,execution_id,step_id,attempt_id,operation_id,closed).await
    }

    async fn get_step(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
    ) -> Result<Option<AgentExecutionStepRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentExecutionStepRow>(
            "SELECT step.* FROM agent_execution_steps step \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.step_id = ? \
               AND step.superseded_in_revision IS NULL AND execution.user_id = ? \
               AND execution.deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn get_step_detail(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
    ) -> Result<Option<AgentExecutionStepDetailRow>, DbError> {
        let mut tx = self.pool.begin().await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id).await?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn transition_step_status(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        expected_execution_version: i64,
        expected_step_version: i64,
        lease: Option<&AgentExecutionLeaseToken>,
        status: ExecutionStepStatus,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, lease, now).await?;
        bump_execution_version_tx(
            &mut tx,
            user_id,
            execution_id,
            expected_execution_version,
            now,
        )
        .await?;
        let previous_status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND version=?",
        ).bind(execution_id).bind(step_id).bind(expected_step_version).fetch_optional(&mut *tx).await?;
        let result = sqlx::query(
            "UPDATE agent_execution_steps SET status = ?, \
                dispatch_after = CASE WHEN ? = 'pending' THEN dispatch_after ELSE NULL END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL \
               AND NOT EXISTS(SELECT 1 FROM agent_execution_attempts attempt \
                              WHERE attempt.execution_id = agent_execution_steps.execution_id \
                                AND attempt.step_id = agent_execution_steps.step_id \
                                AND attempt.status IN ('queued', 'running', 'waiting_input')) \
               AND EXISTS(SELECT 1 FROM agent_executions execution \
                          WHERE execution.execution_id = agent_execution_steps.execution_id \
                            AND execution.user_id = ? AND execution.deleted_at IS NULL)",
        )
        .bind(status.as_str())
        .bind(status.as_str())
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution step"));
        }
        if status.is_terminal() && previous_status.as_deref() != Some(status.as_str()) {
            requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, step_id, now).await?;
        }
        let event = scoped_event(event, step_id, None);
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let row = sqlx::query_as::<_, AgentExecutionStepRow>(
            "SELECT * FROM agent_execution_steps WHERE execution_id = ? AND step_id = ?",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn reset_steps_for_retry(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_execution_version: i64,
        steps: &[RetryAgentExecutionStep],
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionDetailRows, DbError> {
        if steps.is_empty() {
            return Err(DbError::Conflict("retry command has no steps".into()));
        }
        let requested: HashMap<String, i64> = steps
            .iter()
            .map(|step| (step.step_id.clone(), step.expected_step_version))
            .collect();
        if requested.len() != steps.len() {
            return Err(DbError::Conflict("retry command repeats a step".into()));
        }
        let now = now_ms();
        let (mut tx,write_guard) = crate::begin_agent_write(&self.pool).await?;
        require_existing_autowork_admitted_tx(&mut tx,user_id,execution_id).await?;
        let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
        if let Some(detail)=task_control::replay_tx(&mut tx,user_id,execution_id,event).await? {
            write_guard.commit(tx).await?;
            return Ok(detail);
        }
        for step in steps {
            task_control::authorize_tx(&mut tx,user_id,execution_id,&step.step_id,event,"retry",step.expected_step_version).await?;
        }
        let previous_status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_execution_version)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(previous_status) = previous_status else {
            return Err(conflict("agent execution"));
        };
        bump_execution_version_tx(
            &mut tx,
            user_id,
            execution_id,
            expected_execution_version,
            now,
        )
        .await?;
        let active_steps: Vec<AgentExecutionStepRow> = sqlx::query_as(
            "SELECT * FROM agent_execution_steps \
             WHERE execution_id = ? AND superseded_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        let by_id: HashMap<String, AgentExecutionStepRow> = active_steps
            .into_iter()
            .map(|step| (step.step_id.clone(), step))
            .collect();
        for (step_id, expected_version) in &requested {
            scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
            let Some(step) = by_id.get(step_id) else {
                return Err(conflict("agent execution step"));
            };
            if step.version != *expected_version {
                return Err(conflict("agent execution step"));
            }
            let automatic_backoff_pending =
                step.status == "pending" && step.dispatch_after.is_some();
            if !automatic_backoff_pending
                && !matches!(step.status.as_str(), "completed" | "failed" | "skipped")
            {
                return Err(DbError::Conflict(format!(
                    "step {step_id} is neither settled nor waiting on automatic backoff"
                )));
            }
        }
        let dependencies: Vec<AgentExecutionStepDependencyRow> = sqlx::query_as(
            "SELECT * FROM agent_execution_step_dependencies \
             WHERE execution_id = ? AND superseded_in_revision IS NULL",
        )
        .bind(execution_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
        for dependency in dependencies {
            outgoing
                .entry(dependency.blocker_step_id)
                .or_default()
                .push(dependency.blocked_step_id);
        }
        let mut reset_ids: HashSet<String> = requested.keys().cloned().collect();
        let mut queue: VecDeque<String> = requested.keys().cloned().collect();
        while let Some(step_id) = queue.pop_front() {
            for downstream in outgoing.get(&step_id).into_iter().flatten() {
                if reset_ids.insert(downstream.clone()) {
                    queue.push_back(downstream.clone());
                }
            }
        }
        for step_id in &reset_ids {
            if payload.get("message_source").is_some() {
                task_control::authorize_owned_step_tx(&mut tx,user_id,execution_id,step_id,event).await?;
            }
            scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
            let step = by_id
                .get(step_id)
                .ok_or_else(|| conflict("agent execution step"))?;
            if step.status == "cancelled" {
                return Err(DbError::Conflict(format!(
                    "cancelled step {step_id} cannot be retried"
                )));
            }
            let active_attempt: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agent_execution_attempts \
                 WHERE execution_id = ? AND step_id = ? \
                   AND status IN ('queued', 'running', 'waiting_input'))",
            )
            .bind(execution_id)
            .bind(step_id)
            .fetch_one(&mut *tx)
            .await?;
            if active_attempt != 0 {
                return Err(DbError::Conflict(format!(
                    "step {step_id} has an active attempt and cannot be reset"
                )));
            }
            let unresolved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link WHERE link.execution_id=? AND link.step_id=? AND link.relation IN('attempt','automation') AND (EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=link.conversation_id AND effect.state IN('pending','unknown')) OR EXISTS(SELECT 1 FROM agent_turns turn WHERE turn.session_id=link.conversation_id AND turn.state IN('accepted','running') AND turn.terminal_event_id IS NULL)))")
                .bind(execution_id).bind(step_id).fetch_one(&mut *tx).await?;
            if unresolved {return Err(conflict("retry requires the original native execution and effects to be reconciled"));}
            let result = sqlx::query(
                "UPDATE agent_execution_steps SET status = 'pending', dispatch_after = NULL, \
                    version = version + 1, \
                    updated_at = ? WHERE execution_id = ? AND step_id = ? AND version = ? \
                    AND superseded_in_revision IS NULL",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .bind(step.version)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(conflict("agent execution step"));
            }
        }
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET status = 'running' \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL \
               AND status IN ('running', 'paused', 'waiting_input', \
                              'completed', 'completed_with_failures', 'failed')",
        )
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        if is_terminal_execution_status(&previous_status) {
            activate_execution_lead_tx(&mut tx, user_id, execution_id, now).await?;
        }
        append_event_tx(&mut tx, execution_id, event, now).await?;
        let detail = load_execution_detail_tx(&mut tx, user_id, execution_id)
            .await?
            .ok_or_else(|| conflict("agent execution"))?;
        write_guard.commit(tx).await?;
        Ok(detail)
    }

    async fn request_step_reconciliation(&self,user_id:&str,execution_id:&str,expected_execution_version:i64,
        step_id:&str,expected_step_version:i64,event:&NewAgentExecutionEvent)->Result<AgentExecutionDetailRows,DbError> {
        task_control::request_reconciliation(&self.pool,user_id,execution_id,expected_execution_version,step_id,expected_step_version,event).await
    }
    async fn request_step_restart(&self,user:&str,execution:&str,step:&str,expected_execution:i64,expected_step:i64,
        expected_attempt:Option<&str>,event:&NewAgentExecutionEvent)->Result<crate::AgentExecutionRestartCommand,DbError> {
        restart::request(&self.pool,user,execution,step,expected_execution,expected_step,expected_attempt,event).await
    }
    async fn pending_step_restarts(&self,user:&str,execution:&str)->Result<Vec<crate::AgentExecutionRestartCommand>,DbError> {
        restart::pending(&self.pool,user,execution).await
    }
    async fn complete_step_restart(&self,user:&str,execution:&str,operation:&str,completion:&crate::AgentExecutionRestartCompletion)
        ->Result<AgentExecutionDetailRows,DbError> {
        restart::complete(&self.pool,user,execution,operation,completion).await
    }

    async fn replace_step_with_existing(&self,user_id:&str,execution_id:&str,expected_execution_version:i64,
        params:&crate::ReplaceAgentExecutionStepWithExistingParams,event:&NewAgentExecutionEvent)->Result<AgentExecutionDetailRows,DbError> {
        task_control::replace_with_existing(&self.pool,user_id,execution_id,expected_execution_version,params,event).await
    }

    async fn adopt_step_output(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_execution_version: i64,
        step_id: &str,
        expected_step_version: i64,
        params: &AdoptAgentExecutionStepOutputParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepDetailRow, DbError> {
        if params.output_summary.trim().is_empty() {
            return Err(DbError::Conflict("adopted step output is empty".into()));
        }
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE agent_executions SET version=version WHERE execution_id=? AND user_id=? AND version=? AND deleted_at IS NULL",
        ).bind(execution_id).bind(user_id).bind(expected_execution_version).execute(&mut *tx).await?;
        if locked.rows_affected() != 1 { return Err(conflict("Agent Execution adoption version")); }
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        let step: Option<AgentExecutionStepRow> = sqlx::query_as(
            "SELECT step.* FROM agent_execution_steps step \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.step_id = ? AND step.version = ? \
               AND step.superseded_in_revision IS NULL AND execution.user_id = ? \
               AND execution.deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(step) = step else {
            return Err(conflict("agent execution step"));
        };
        let previous_status: String = sqlx::query_scalar(
            "SELECT status FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_execution_version)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("agent execution"))?;
        if step.status == "cancelled" {
            return Err(DbError::Conflict(
                "a cancelled step cannot adopt new output".into(),
            ));
        }
        let active_attempt: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_execution_attempts \
             WHERE execution_id = ? AND step_id = ? \
               AND status IN ('queued', 'running', 'waiting_input'))",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_one(&mut *tx)
        .await?;
        if active_attempt != 0 {
            return Err(DbError::Conflict(
                "cannot adopt output while the step has an active attempt".into(),
            ));
        }

        let latest: Option<(Option<String>, String, Option<String>)> = sqlx::query_as(
            "SELECT attempt.participant_id, attempt.effective_config, \
                (SELECT link.conversation_id FROM conversation_execution_links link \
                 WHERE link.execution_id = attempt.execution_id \
                   AND link.step_id = attempt.step_id AND link.attempt_id = attempt.attempt_id \
                   AND link.relation IN ('attempt', 'automation') \
                 ORDER BY link.active DESC, link.updated_at DESC LIMIT 1) \
             FROM agent_execution_attempts attempt \
             WHERE attempt.execution_id = ? AND attempt.step_id = ? \
             ORDER BY attempt.attempt_no DESC LIMIT 1",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_optional(&mut *tx)
        .await?;
        let participant_id = latest
            .as_ref()
            .and_then(|(participant_id, _, _)| participant_id.clone())
            .or_else(|| step.assigned_participant_id.clone());
        let effective_config = latest
            .as_ref()
            .map(|(_, effective_config, _)| effective_config.clone())
            .unwrap_or_else(|| "{}".to_string());
        let conversation_id = latest.and_then(|(_, _, conversation_id)| conversation_id);

        let execution_result = sqlx::query(
            "UPDATE agent_executions SET \
                status = CASE WHEN status IN (\
                    'completed', 'completed_with_failures', 'failed'\
                ) THEN 'running' ELSE status END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status IN ('running', 'paused', 'waiting_input', \
                              'completed', 'completed_with_failures', 'failed')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_execution_version)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        let step_result = sqlx::query(
            "UPDATE agent_execution_steps SET status = 'completed', dispatch_after = NULL, \
                version = version + 1, \
                updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL",
        )
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .execute(&mut *tx)
        .await?;
        if step_result.rows_affected() != 1 {
            return Err(conflict("agent execution step"));
        }
        reopen_adopt_downstream_tx(&mut tx, execution_id, step_id, now).await?;
        let attempt_no: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(attempt_no), -1) + 1 FROM agent_execution_attempts \
             WHERE execution_id = ? AND step_id = ?",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_one(&mut *tx)
        .await?;
        let attempt_id = nomifun_common::AgentExecutionAttemptId::new().into_string();
        if let Some(participant_id) = participant_id.as_deref() {
            let participant = sqlx::query(
                "UPDATE agent_execution_participants SET created_at = created_at \
                 WHERE execution_id = ? AND participant_id = ? \
                   AND retired_in_revision IS NULL",
            )
            .bind(execution_id)
            .bind(participant_id)
            .execute(&mut *tx)
            .await?;
            if participant.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "adopted Agent Execution participant '{participant_id}' does not exist"
                )));
            }
        }
        sqlx::query(
            "INSERT INTO agent_execution_attempts (\
                attempt_id, execution_id, step_id, attempt_no, participant_id, status, trigger_reason, \
                effective_config, output_summary, output_files, tokens, runtime_state, \
                started_at, finished_at, version, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, 'completed', 'adopt', ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&attempt_id)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_no)
        .bind(&participant_id)
        .bind(&effective_config)
        .bind(&params.output_summary)
        .bind(&params.output_files)
        .bind(params.tokens)
        .bind(&params.runtime_state)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if let Some(conversation_id) = conversation_id {
            sqlx::query(
                "INSERT INTO conversation_execution_links (\
                    conversation_id, execution_id, relation, step_id, attempt_id, \
                    active, created_at, updated_at\
                 ) VALUES (?, ?, 'attempt', ?, ?, 0, ?, ?)",
            )
            .bind(conversation_id)
            .bind(execution_id)
            .bind(step_id)
            .bind(&attempt_id)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        if is_terminal_execution_status(&previous_status) {
            activate_execution_lead_tx(&mut tx, user_id, execution_id, now).await?;
        }
        let event = scoped_event(event, step_id, Some(&attempt_id));
        requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, step_id, now).await?;
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn resume_waiting_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_execution_version: i64,
        step_id: &str,
        expected_step_version: i64,
        attempt_id: &str,
        expected_attempt_version: i64,
        params: &AttemptConversationEffectParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AttemptConversationEffectResult, DbError> {
        let now = now_ms();
        let mut conn = self.pool.acquire().await?;
        // Acquire the writer lock before reading the active Session link. A
        // deferred read snapshot cannot reliably upgrade while another writer
        // holds the lock; BEGIN IMMEDIATE waits before all owner and CAS checks.
        let mut tx = Transaction::begin(&mut *conn, Some("BEGIN IMMEDIATE".into())).await?;
        require_existing_autowork_admitted_tx(&mut tx,user_id,execution_id).await?;
        let conversation_id = active_attempt_conversation_tx(
            &mut tx,
            user_id,
            execution_id,
            step_id,
            attempt_id,
        )
        .await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        parent_reply::validate_tx(&mut tx,user_id,execution_id,step_id,attempt_id,&conversation_id,params,event).await?;
        let admitted = super::agent_execution_admission::execution_capacity_available_tx(
            &mut tx, execution_id, step_id, Some(attempt_id),
        ).await?;
        let next_status = if admitted { "running" } else { "waiting_input" };
        let step_result = sqlx::query(
            "UPDATE agent_execution_steps SET status = ?, version = version + 1, \
                updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL AND status = 'waiting_input'",
        )
        .bind(next_status)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .execute(&mut *tx)
        .await?;
        if step_result.rows_affected() != 1 {
            return Err(conflict("agent execution step"));
        }
        let attempt_result = sqlx::query(
            "UPDATE agent_execution_attempts SET status = ?, question = NULL, \
                runtime_state = ?, version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status = 'waiting_input' \
               AND (runtime_state IS NULL OR NOT json_valid(runtime_state) \
                    OR json_type(runtime_state, '$.review_blocked') IS NULL)",
        )
        .bind(next_status)
        .bind(&params.runtime_state)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .execute(&mut *tx)
        .await?;
        if attempt_result.rows_affected() != 1 {
            return Err(conflict("agent execution attempt"));
        }
        let waiting_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_execution_attempts \
             WHERE execution_id = ? AND status = 'waiting_input'",
        )
        .bind(execution_id)
        .fetch_one(&mut *tx)
        .await?;
        let aggregate_status = if waiting_count == 0 {
            "running"
        } else {
            "waiting_input"
        };
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET status = CASE WHEN status='paused' THEN status ELSE ? END, version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input', 'paused')",
        )
        .bind(aggregate_status)
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_execution_version)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        let event = scoped_event(event, step_id, Some(attempt_id));
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        let event_sequence:i64=sqlx::query_scalar("SELECT event_sequence FROM agent_executions WHERE execution_id=?")
            .bind(execution_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(AttemptConversationEffectResult {
            event_sequence,
            detail,
            conversation_id,
        })
    }

    async fn admit_waiting_attempt_input(
        &self, user_id: &str, execution_id: &str, step_id: &str, attempt_id: &str,
        lease: &AgentExecutionLeaseToken, event: &NewAgentExecutionEvent,
    ) -> Result<Option<AgentExecutionStepDetailRow>, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, Some(lease), now).await?;
        require_existing_autowork_admitted_tx(&mut tx,user_id,execution_id).await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id).await?
            .ok_or_else(|| conflict("decision continuation Step"))?;
        let attempt = detail.current_attempt.as_ref().filter(|attempt| attempt.attempt.attempt_id == attempt_id)
            .ok_or_else(|| conflict("decision continuation Attempt"))?;
        if detail.step.status != "waiting_input" || attempt.attempt.status != "waiting_input" { return Ok(None); }
        let effects = AttemptConversationEffects::decode(attempt.attempt.runtime_state.as_deref())
            .map_err(|error| DbError::Init(error.to_string()))?;
        if effects.review_blocked.is_some() || !matches!(effects.pending_conversation_effects.first(),
            Some(crate::PendingConversationEffect::DecisionInput { .. })) { return Ok(None); }
        if !super::agent_execution_admission::execution_capacity_available_tx(&mut tx, execution_id, step_id, Some(attempt_id)).await? {
            return Ok(None);
        }
        active_attempt_conversation_tx(&mut tx, user_id, execution_id, step_id, attempt_id).await?;
        sqlx::query("UPDATE agent_execution_steps SET status='running',version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=? AND status='waiting_input'")
            .bind(now).bind(execution_id).bind(step_id).bind(detail.step.version).execute(&mut *tx).await?;
        sqlx::query("UPDATE agent_execution_attempts SET status='running',question=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status='waiting_input'")
            .bind(now).bind(execution_id).bind(step_id).bind(attempt_id).bind(attempt.attempt.version).execute(&mut *tx).await?;
        sqlx::query("UPDATE agent_executions SET status=CASE WHEN EXISTS(SELECT 1 FROM agent_execution_attempts attempt WHERE attempt.execution_id=agent_executions.execution_id AND attempt.status='waiting_input') THEN 'waiting_input' ELSE 'running' END,version=version+1,updated_at=? WHERE execution_id=? AND user_id=?")
            .bind(now).bind(execution_id).bind(user_id).execute(&mut *tx).await?;
        append_event_tx(&mut tx, execution_id, &scoped_event(event, step_id, Some(attempt_id)), now).await?;
        let admitted = load_step_detail_tx(&mut tx, user_id, execution_id, step_id).await?
            .ok_or_else(|| conflict("admitted continuation"))?;
        tx.commit().await?;
        Ok(Some(admitted))
    }

    async fn attempt_reservations(&self, execution_id: &str) -> Result<Vec<crate::AgentExecutionReservation>, DbError> {
        let mut tx = self.pool.begin().await?;
        let reservations = super::agent_execution_admission::execution_reservations_tx(&mut tx, execution_id).await?;
        tx.commit().await?;
        Ok(reservations)
    }

    async fn enqueue_attempt_conversation_effect(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_execution_version: i64,
        step_id: &str,
        expected_step_version: i64,
        attempt_id: &str,
        expected_attempt_version: i64,
        params: &AttemptConversationEffectParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AttemptConversationEffectResult, DbError> {
        let now = now_ms();
        let mut conn=self.pool.acquire().await?;
        let mut tx=Transaction::begin(&mut *conn,Some("BEGIN IMMEDIATE".into())).await?;
        let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
        if let Some(source)=payload.get("message_source").filter(|source|!source.is_null()) {
            let source:crate::AgentExecutionMessageSource=serde_json::from_value(source.clone()).map_err(|error|DbError::Conflict(error.to_string()))?;
            if !matches!(&event.actor,AgentExecutionActor::Agent {conversation_id:Some(session),..} if session==&source.agent_session_id) {
                return Err(conflict("Agent message source actor"));
            }
            let operation=payload.get("operation_id").and_then(serde_json::Value::as_str).ok_or_else(||conflict("Agent message operation"))?;
            let digest=payload.get("content_digest").and_then(serde_json::Value::as_str).ok_or_else(||conflict("Agent message content digest"))?;
            crate::repository::agent_source_reply::verify_message_command_tx(&mut tx,user_id,&source,execution_id,step_id,operation,digest).await?;
            let effects=AttemptConversationEffects::decode(params.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
            if !effects.pending_conversation_effects.iter().any(|effect|matches!(effect,crate::PendingConversationEffect::Steer {operation_id,content,..}
                if operation_id==operation&&nomifun_agent_contracts::digest_bytes(content.as_bytes()).as_ref()==digest)) {
                return Err(conflict("Agent message pending guidance content"));
            }
        } else if matches!(&event.actor,AgentExecutionActor::Agent {..})
            &&payload.get("content_digest").is_some_and(|value|!value.is_null()) {
            return Err(conflict("Agent message requires its original native epoch"));
        }
        let conversation_id = active_attempt_conversation_tx(
            &mut tx,
            user_id,
            execution_id,
            step_id,
            attempt_id,
        )
        .await?;
        let step_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_execution_steps step \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.step_id = ? AND step.version = ? \
               AND step.superseded_in_revision IS NULL AND step.status = 'running' \
               AND execution.user_id = ? AND execution.version = ? \
               AND execution.deleted_at IS NULL \
               AND execution.status IN ('running', 'waiting_input', 'paused')",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .bind(expected_execution_version)
        .fetch_one(&mut *tx)
        .await?;
        if step_exists != 1 {
            return Err(conflict("running agent execution step"));
        }
        let attempt_result = sqlx::query(
            "UPDATE agent_execution_attempts SET runtime_state = ?, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status = 'running'",
        )
        .bind(&params.runtime_state)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .execute(&mut *tx)
        .await?;
        if attempt_result.rows_affected() != 1 {
            return Err(conflict("agent execution attempt"));
        }
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND version = ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input', 'paused')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .bind(expected_execution_version)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        let event = scoped_event(event, step_id, Some(attempt_id));
        let committed = append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        tx.commit().await?;
        Ok(AttemptConversationEffectResult {
            event_sequence: committed.sequence,
            detail,
            conversation_id,
        })
    }

    async fn acknowledge_attempt_conversation_effect(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        attempt_id: &str,
        expected_attempt_version: i64,
        params: &AttemptConversationEffectParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepDetailRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE agent_execution_attempts SET runtime_state = ?, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status IN ('running', 'waiting_input') \
               AND EXISTS(SELECT 1 FROM agent_executions execution \
                          WHERE execution.execution_id = agent_execution_attempts.execution_id \
                            AND execution.user_id = ? AND execution.deleted_at IS NULL \
                            AND execution.status IN ('running', 'waiting_input', 'paused'))",
        )
        .bind(&params.runtime_state)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() != 1 {
            return Err(conflict("agent execution attempt effect"));
        }
        sqlx::query(
            "UPDATE agent_executions SET version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        let event = scoped_event(event, step_id, Some(attempt_id));
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn acknowledge_initial_input_admission(&self,user:&str,execution:&str,step:&str,attempt:&str,
        expected_attempt_version:i64,lease:&AgentExecutionLeaseToken,event:&NewAgentExecutionEvent)->Result<bool,DbError> {
        let now=now_ms();let (mut tx,write_guard)=crate::begin_agent_write(&self.pool).await?;
        fence_scheduler_write_tx(&mut tx,execution,Some(lease),now).await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx,execution,step).await?;
        let row:Option<(Option<String>,i64)>=sqlx::query_as("SELECT attempt.runtime_state,attempt.version FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id JOIN agent_executions execution ON execution.execution_id=attempt.execution_id WHERE attempt.execution_id=? AND attempt.step_id=? AND attempt.attempt_id=? AND attempt.status IN('running','waiting_input') AND step.superseded_in_revision IS NULL AND step.status IN('running','waiting_input') AND execution.user_id=? AND execution.deleted_at IS NULL AND execution.status IN('running','waiting_input','paused')")
            .bind(execution).bind(step).bind(attempt).bind(user).fetch_optional(&mut *tx).await?;
        let (raw,current_version)=row.ok_or_else(||conflict("original InitialInput Attempt"))?;
        let mut effects=AttemptConversationEffects::decode(raw.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
        let Some(crate::PendingConversationEffect::InitialInput {request})=effects.pending_conversation_effects.first() else {
            write_guard.commit(tx).await?;return Ok(false);
        };
        if current_version!=expected_attempt_version {return Err(conflict("original InitialInput admission version"));}
        let session=active_attempt_conversation_tx(&mut tx,user,execution,step,attempt).await?;
        let Some((operation,accepted,started))=native_pause::initial_admission_tx(&mut tx,user,&session,attempt,request).await? else {
            write_guard.commit(tx).await?;return Ok(false);
        };
        effects.pending_conversation_effects.remove(0);
        let raw=if effects==AttemptConversationEffects::default() {None}else{Some(effects.encode().map_err(|error|DbError::Init(error.to_string()))?)};
        let changed=sqlx::query("UPDATE agent_execution_attempts SET runtime_state=?,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status IN('running','waiting_input')")
            .bind(raw).bind(now).bind(execution).bind(step).bind(attempt).bind(expected_attempt_version).execute(&mut *tx).await?;
        if changed.rows_affected()!=1 {return Err(conflict("original InitialInput admission ACK"));}
        sqlx::query("UPDATE agent_executions SET version=version+1,updated_at=? WHERE execution_id=? AND user_id=?")
            .bind(now).bind(execution).bind(user).execute(&mut *tx).await?;
        let mut event=scoped_event(event,step,Some(attempt));
        event.payload=serde_json::json!({"change":"conversation_effect_delivered","effect":"initial_input",
            "turn_operation_key":format!("{attempt}:initial-turn"),"canonical_operation_id":operation,
            "accepted_event_id":accepted,"started_event_id":started}).to_string();
        append_event_tx(&mut tx,execution,&event,now).await?;
        write_guard.commit(tx).await?;Ok(true)
    }

    async fn create_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        expected_step_version: i64,
        lease: Option<&AgentExecutionLeaseToken>,
        params: &CreateAgentExecutionAttemptParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepDetailRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, lease, now).await?;
        require_existing_autowork_admitted_tx(&mut tx,user_id,execution_id).await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        let step: Option<AgentExecutionStepRow> = sqlx::query_as(
            "SELECT step.* FROM agent_execution_steps step \
             JOIN agent_executions execution ON execution.execution_id = step.execution_id \
             WHERE step.execution_id = ? AND step.step_id = ? AND step.version = ? \
               AND step.superseded_in_revision IS NULL AND step.status = 'pending' \
               AND execution.user_id = ? AND execution.deleted_at IS NULL \
               AND execution.status IN ('running', 'waiting_input')",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(step) = step else {
            return Err(conflict("agent execution step"));
        };
        if step.kind == "agent" && !super::agent_execution_admission::execution_capacity_available_tx(
            &mut tx, execution_id, step_id, None,
        ).await? { return Err(conflict("Agent Execution admission is waiting for its existing reservations")); }
        if params.start_immediately {
            if step.kind == "agent" || params.participant_id.is_some() {
                return Err(DbError::Conflict(
                    "only a control step without a participant may start immediately".into(),
                ));
            }
            let result = sqlx::query(
                "UPDATE agent_execution_steps SET status = 'running', dispatch_after = NULL, \
                    version = version + 1, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? AND version = ? \
                   AND superseded_in_revision IS NULL AND status = 'pending'",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .bind(expected_step_version)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(conflict("agent execution step"));
            }
        } else {
            if step.kind != "agent"
                || params.participant_id.as_ref() != step.assigned_participant_id.as_ref()
            {
                return Err(DbError::Conflict(
                    "an Agent step attempt must queue with its active assigned participant".into(),
                ));
            }
            let result = sqlx::query(
                "UPDATE agent_execution_steps SET dispatch_after = NULL, \
                    version = version + 1, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? AND version = ? \
                   AND superseded_in_revision IS NULL AND status = 'pending'",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .bind(expected_step_version)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() != 1 {
                return Err(conflict("agent execution step"));
            }
        }
        let attempt_no: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(attempt_no), -1) + 1 FROM agent_execution_attempts \
             WHERE execution_id = ? AND step_id = ?",
        )
        .bind(execution_id)
        .bind(step_id)
        .fetch_one(&mut *tx)
        .await?;
        let attempt_status = if params.start_immediately {
            "running"
        } else {
            "queued"
        };
        let started_at = params.start_immediately.then_some(now);
        let attempt_id = nomifun_common::AgentExecutionAttemptId::new().into_string();
        if let Some(participant_id) = params.participant_id.as_deref() {
            let participant = sqlx::query(
                "UPDATE agent_execution_participants SET created_at = created_at \
                 WHERE execution_id = ? AND participant_id = ? \
                   AND retired_in_revision IS NULL",
            )
            .bind(execution_id)
            .bind(participant_id)
            .execute(&mut *tx)
            .await?;
            if participant.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "Agent Execution participant '{participant_id}' does not exist"
                )));
            }
        }
        sqlx::query(
            "INSERT INTO agent_execution_attempts (\
                attempt_id, execution_id, step_id, attempt_no, participant_id, status, trigger_reason, \
                effective_config, output_files, retry_after, runtime_state, started_at, \
                version, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, '[]', ?, ?, ?, 0, ?, ?)",
        )
        .bind(&attempt_id)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_no)
        .bind(&params.participant_id)
        .bind(attempt_status)
        .bind(&params.trigger_reason)
        .bind(&params.effective_config)
        .bind(params.retry_after)
        .bind(&params.runtime_state)
        .bind(started_at)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL \
               AND status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        let event = scoped_event(event, step_id, Some(&attempt_id));
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn start_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        expected_step_version: i64,
        attempt_id: &str,
        expected_attempt_version: i64,
        conversation_id: &str,
        session_kind: AgentExecutionAttemptSessionKind,
        lease: Option<&AgentExecutionLeaseToken>,
        initial_input: Option<&crate::PendingConversationEffect>,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepDetailRow, DbError> {
        let now = now_ms();
        let (mut tx,write_guard) = crate::begin_agent_write(&self.pool).await?;
        fence_scheduler_write_tx(&mut tx, execution_id, lease, now).await?;
        require_existing_autowork_admitted_tx(&mut tx,user_id,execution_id).await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        let original_state: Option<String> = sqlx::query_scalar(
            "SELECT runtime_state FROM agent_execution_attempts WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status='queued'",
        ).bind(execution_id).bind(step_id).bind(attempt_id).bind(expected_attempt_version)
            .fetch_optional(&mut *tx).await?.ok_or_else(|| conflict("queued Attempt initial input"))?;
        let mut effects=AttemptConversationEffects::decode(original_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
        if let Some(input)=initial_input {
            if !matches!(input,crate::PendingConversationEffect::InitialInput {request} if request.is_object())
                || effects.pending_conversation_effects.iter().any(|prior|!matches!(prior,crate::PendingConversationEffect::InitialInput {..}))
                || effects.pending_conversation_effects.len()>1 || effects.review_blocked.is_some() {
                return Err(conflict("exact initial Attempt input outbox"));
            }
            let [crate::PendingConversationEffect::InitialInput {request:original}]=effects.pending_conversation_effects.as_slice()
                else {return Err(conflict("original queued input is required before preparing its Session"));};
            let crate::PendingConversationEffect::InitialInput {request:prepared}=input else {unreachable!()};
            if ["content","origin","hidden","channel_platform"].iter().any(|field|original.get(*field)!=prepared.get(*field))
                || ["files","inject_skills"].iter().any(|field|original.get(*field).and_then(serde_json::Value::as_array).is_some_and(|prior|
                    prior.iter().any(|value|prepared.get(*field).and_then(serde_json::Value::as_array).is_none_or(|next|!next.contains(value))))) {
                return Err(conflict("prepared input changed the original task or removed frozen selections"));
            }
            effects.pending_conversation_effects=vec![input.clone()];
        }
        let runtime_state=if initial_input.is_some() {Some(effects.encode().map_err(|error|DbError::Init(error.to_string()))?)} else {original_state};
        let step_result = sqlx::query(
            "UPDATE agent_execution_steps SET status = 'running', dispatch_after = NULL, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL AND status = 'pending' \
               AND EXISTS(SELECT 1 FROM agent_executions execution \
                          WHERE execution.execution_id = agent_execution_steps.execution_id \
                            AND execution.user_id = ? AND execution.deleted_at IS NULL \
                            AND execution.status IN ('running', 'waiting_input'))",
        )
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if step_result.rows_affected() != 1 {
            return Err(conflict("agent execution step"));
        }
        let attempt_result = sqlx::query(
            "UPDATE agent_execution_attempts SET status = 'running', started_at = ?, runtime_state = ?, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status = 'queued'",
        )
        .bind(now)
        .bind(runtime_state)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .execute(&mut *tx)
        .await?;
        if attempt_result.rows_affected() != 1 {
            return Err(conflict("agent execution attempt"));
        }
        if session_kind == AgentExecutionAttemptSessionKind::AutomationLead {
            let exact_automation_lead: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM conversation_execution_links link \
                 JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                 WHERE link.conversation_id = ? AND link.execution_id = ? \
                   AND link.relation = 'lead' AND link.active = 1 \
                   AND execution.user_id = ? AND execution.deleted_at IS NULL \
                   AND json_extract(execution.initial_plan_input, '$.mode') = 'automation' \
                   AND EXISTS(SELECT 1 FROM agent_session_heads head WHERE head.session_id=link.conversation_id \
                              AND head.status='ready' AND head.active_turn_id IS NULL)",
            )
            .bind(conversation_id)
            .bind(execution_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if exact_automation_lead != 1 {
                return Err(conflict("AutoWork lead AgentSession"));
            }
        }
        source_continuation::verify_child_binding_tx(&mut tx,user_id,execution_id,step_id,conversation_id).await?;
        let link_result = sqlx::query(
            "INSERT INTO conversation_execution_links (\
                conversation_id, execution_id, relation, step_id, attempt_id, \
                active, created_at, updated_at\
             ) SELECT session.agent_session_id, ?, ?, ?, ?, 1, ?, ? \
               FROM agent_sessions session \
              WHERE session.agent_session_id = ? AND session.state = 'live' \
                AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
                AND json_extract(session.owner_ref_json, '$.principal_id') = ?",
        )
        .bind(execution_id)
        .bind(session_kind.relation())
        .bind(step_id)
        .bind(attempt_id)
        .bind(now)
        .bind(now)
        .bind(conversation_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if link_result.rows_affected() != 1 {
            return Err(conflict("attempt conversation"));
        }
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL \
               AND status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')",
        )
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        let event = scoped_event(event, step_id, Some(attempt_id));
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        write_guard.commit(tx).await?;
        Ok(detail)
    }

    async fn reconcile_recovered_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        expected_step_version: i64,
        attempt_id: &str,
        expected_attempt_version: i64,
        lease: &AgentExecutionLeaseToken,
        recovered_output: Option<&RecoveredAgentExecutionAttemptOutput>,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionAttemptRecoveryResult, DbError> {
        let now = now_ms();
        let (mut tx,write_guard) = crate::begin_agent_write(&self.pool).await?;
        fence_scheduler_write_tx(&mut tx, execution_id, Some(lease), now).await?;
        scoped_stop::ensure_not_stopped_tx(&mut tx, execution_id, step_id).await?;
        let attempt_status: Option<String> = sqlx::query_scalar(
            "SELECT attempt.status FROM agent_execution_attempts attempt \
             JOIN agent_execution_steps step \
               ON step.execution_id = attempt.execution_id AND step.step_id = attempt.step_id \
             JOIN agent_executions execution ON execution.execution_id = attempt.execution_id \
             WHERE attempt.execution_id = ? AND attempt.step_id = ? AND attempt.attempt_id = ? \
               AND attempt.version = ? AND (attempt.status IN ('queued', 'running') OR (attempt.status='waiting_input' AND json_type(attempt.runtime_state,'$.review_blocked')='object')) \
               AND step.version = ? AND step.superseded_in_revision IS NULL \
               AND execution.user_id = ? AND execution.deleted_at IS NULL",
        )
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .bind(expected_step_version)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let attempt_status =
            attempt_status.ok_or_else(|| conflict("recoverable Agent Execution attempt"))?;

        let disposition = if attempt_status == "queued" {
            let state:Option<String>=sqlx::query_scalar("SELECT runtime_state FROM agent_execution_attempts WHERE execution_id=? AND attempt_id=?")
                .bind(execution_id).bind(attempt_id).fetch_one(&mut *tx).await?;
            let effects=AttemptConversationEffects::decode(state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
            if effects.pending_conversation_effects.iter().any(|effect|matches!(effect,crate::PendingConversationEffect::InitialInput {..})) {
                AgentExecutionAttemptRecoveryDisposition::QueuedReservationRetained
            } else {
                let reason="The original queued input was not durably retained. Reconcile its immutable command before delivering; current task text cannot reconstruct the original input.";
                let state=review_block_runtime_state(&format!("{attempt_id}:initial-turn"),"initial_input_missing",reason,state.as_deref())?;
                let changed=sqlx::query("UPDATE agent_execution_attempts SET status='waiting_input',question=NULL,error=?,runtime_state=?,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status='queued'")
                    .bind(reason).bind(state).bind(now).bind(execution_id).bind(step_id).bind(attempt_id).bind(expected_attempt_version).execute(&mut *tx).await?;
                if changed.rows_affected()!=1 {return Err(conflict("queued original input"));}
                let changed=sqlx::query("UPDATE agent_execution_steps SET status='waiting_input',dispatch_after=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=? AND status='pending' AND superseded_in_revision IS NULL")
                    .bind(now).bind(execution_id).bind(step_id).bind(expected_step_version).execute(&mut *tx).await?;
                if changed.rows_affected()!=1 {return Err(conflict("queued original task"));}
                AgentExecutionAttemptRecoveryDisposition::ReviewBlocked
            }
        } else {
            match reconcile_original_attempt_receipt_tx(
                &mut tx,
                user_id,
                execution_id,
                step_id,
                expected_step_version,
                attempt_id,
                expected_attempt_version,
                recovered_output,
                true,
                now,
            )
            .await?
            {
                OriginalAttemptReceiptReconciliation::InitialDeliveryPending => AgentExecutionAttemptRecoveryDisposition::InitialDeliveryPending,
                OriginalAttemptReceiptReconciliation::ReReadRequired => AgentExecutionAttemptRecoveryDisposition::ReReadRequired,
                OriginalAttemptReceiptReconciliation::OngoingTurnObserved => {
                    AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved
                }
                OriginalAttemptReceiptReconciliation::CompletedReceiptAdopted => {
                    AgentExecutionAttemptRecoveryDisposition::CompletedReceiptAdopted
                }
                OriginalAttemptReceiptReconciliation::CancelledReceiptAdopted => {
                    AgentExecutionAttemptRecoveryDisposition::CancelledReceiptAdopted
                }
                OriginalAttemptReceiptReconciliation::UserInputRequested => {
                    AgentExecutionAttemptRecoveryDisposition::UserInputRequested
                }
                OriginalAttemptReceiptReconciliation::ReviewBlocked => {
                    AgentExecutionAttemptRecoveryDisposition::ReviewBlocked
                }
            }
        };

        if matches!(disposition, AgentExecutionAttemptRecoveryDisposition::ReReadRequired | AgentExecutionAttemptRecoveryDisposition::QueuedReservationRetained)
            || (disposition==AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved && attempt_status!="waiting_input")
            || (disposition==AgentExecutionAttemptRecoveryDisposition::ReviewBlocked && attempt_status=="waiting_input"
                && sqlx::query_scalar::<_,i64>("SELECT version FROM agent_execution_attempts WHERE execution_id=? AND attempt_id=?")
                    .bind(execution_id).bind(attempt_id).fetch_one(&mut *tx).await?==expected_attempt_version) {
            let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id).await?
                .ok_or_else(|| conflict("ongoing Agent Execution step"))?;
            write_guard.commit(tx).await?;
            return Ok(AgentExecutionAttemptRecoveryResult { detail, disposition });
        }
        let execution_status = match disposition {
            AgentExecutionAttemptRecoveryDisposition::ReviewBlocked | AgentExecutionAttemptRecoveryDisposition::UserInputRequested => Some("waiting_input"),
            _ => None,
        };
        let execution = sqlx::query(
            "UPDATE agent_executions SET status = CASE WHEN status='paused' THEN status ELSE COALESCE(?, status) END, \
                lease_owner = CASE WHEN ?='cancelled' THEN NULL ELSE lease_owner END, \
                lease_expires_at = CASE WHEN ?='cancelled' THEN NULL ELSE lease_expires_at END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL \
               AND status IN ('running', 'waiting_input', 'paused')",
        )
        .bind(execution_status)
        .bind(execution_status)
        .bind(execution_status)
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution.rows_affected() != 1 {
            return Err(conflict("Agent Execution recovery"));
        }
        let mut event = scoped_event(event, step_id, Some(attempt_id));
        if let Some(output) = recovered_output {
            let mut payload: serde_json::Value = serde_json::from_str(&event.payload)
                .map_err(|_| conflict("recovered Attempt event payload"))?;
            let object = payload.as_object_mut().ok_or_else(|| conflict("recovered Attempt event payload"))?;
            object.insert("canonical_operation_id".into(), output.canonical_operation_id.clone().into());
            object.insert("terminal_event_id".into(), output.terminal_event_id.clone().into());
            event.payload = payload.to_string();
        }
        if disposition == AgentExecutionAttemptRecoveryDisposition::UserInputRequested {
            let output = recovered_output.ok_or_else(|| conflict("recovered user input source"))?;
            let request = output.user_input_request.as_ref().ok_or_else(|| conflict("recovered user input request"))?;
            // Reconciliation has verified this exact canonical invocation in
            // the same writer before retiring its delivered input queue. Keep
            // its identity in the original domain event; caller metadata must
            // neither choose a successor nor erase a continuation's identity.
            let prefix=format!("turn:user:{user_id}:{}:",output.conversation_id);
            let turn_operation_key=output.canonical_operation_id.strip_prefix(&prefix)
                .filter(|key|!key.is_empty()).ok_or_else(||conflict("recovered input Turn identity"))?;
            event.event_type = AgentExecutionEventKind::DecisionRequested;
            event.payload = serde_json::json!({
                "question":request.question,"user_input_request":request,
                "canonical_operation_id":output.canonical_operation_id,"terminal_event_id":output.terminal_event_id,
                "turn_operation_key":turn_operation_key,
                "reconciliation":"completed_user_input_handoff",
            }).to_string();
        }
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("Agent Execution recovery step"))?;
        write_guard.commit(tx).await?;
        Ok(AgentExecutionAttemptRecoveryResult {
            detail,
            disposition,
        })
    }

    async fn settle_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        expected_step_version: i64,
        attempt_id: &str,
        expected_attempt_version: i64,
        lease: Option<&AgentExecutionLeaseToken>,
        params: &SettleAgentExecutionAttemptParams,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionStepDetailRow, DbError> {
        if params.attempt_status == ExecutionAttemptStatus::Queued {
            return Err(DbError::Conflict(
                "a settled attempt cannot transition back to queued".into(),
            ));
        }
        if params.loop_repeat_reset.is_some()
            && (params.attempt_status != ExecutionAttemptStatus::Completed
                || params.step_status != ExecutionStepStatus::Pending
                || params.execution_status.is_some())
        {
            return Err(DbError::Conflict(
                "a Loop repeat must complete its control attempt, reset the controller to pending, and keep the execution running"
                    .into(),
            ));
        }
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        fence_scheduler_write_tx(&mut tx, execution_id, lease, now).await?;
        if params.expected_active_session_turn.is_some() && params.expected_terminal_turn.is_some() {
            return Err(conflict("mutually exclusive canonical source Turn guards"));
        }
        if let Some(source) = &params.expected_terminal_turn {
            match source {
                AgentExecutionTerminalTurnGuard::InputRequired { user_input_request, .. } => {
                    if params.attempt_status != ExecutionAttemptStatus::WaitingInput
                        || params.step_status != ExecutionStepStatus::WaitingInput
                        || params.execution_status != Some(AgentExecutionStatus::WaitingInput)
                        || params.question.as_ref().and_then(|question| question.as_deref()) != Some(user_input_request.question.as_str()) {
                        return Err(conflict("canonical user input settlement"));
                    }
                }
                AgentExecutionTerminalTurnGuard::Cancelled { .. } => {
                    if params.attempt_status != ExecutionAttemptStatus::Cancelled
                        || params.step_status != ExecutionStepStatus::Cancelled
                        || params.execution_status.is_some() {
                        return Err(conflict("canonical cancellation settlement"));
                    }
                }
            }
            verify_terminal_turn_tx(&mut tx, user_id, execution_id, step_id, attempt_id, source, false).await?;
        }
        if let Some(source) = &params.expected_active_session_turn {
            if params.attempt_status != ExecutionAttemptStatus::WaitingInput
                || params.step_status != ExecutionStepStatus::WaitingInput
                || source.conversation_id.trim().is_empty()
                || source.canonical_operation_id.trim().is_empty() {
                return Err(conflict("Native Agent decision source Turn"));
            }
            // The exact calling Turn must still own the active Session and
            // Attempt in this same write transaction. A delayed old tool call
            // cannot move a successor Turn into WaitingInput.
            let valid: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM conversation_execution_links link \
                 JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
                 JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
                 JOIN agent_turns turn ON turn.session_id=session.agent_session_id AND turn.operation_id=? \
                 JOIN agent_executions execution ON execution.execution_id=link.execution_id \
                 WHERE link.conversation_id=? AND link.execution_id=? AND link.step_id=? AND link.attempt_id=? \
                   AND link.active=1 AND link.relation IN ('attempt','automation') \
                   AND session.state='live' AND execution.user_id=? AND execution.deleted_at IS NULL \
                   AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
                   AND json_extract(session.owner_ref_json,'$.principal_id')=? \
                   AND head.active_turn_id=turn.operation_id AND head.status IN ('running','paused') \
                   AND turn.state IN ('accepted','running') AND turn.terminal_event_id IS NULL",
            ).bind(&source.canonical_operation_id).bind(&source.conversation_id).bind(execution_id)
                .bind(step_id).bind(attempt_id).bind(user_id).bind(user_id).fetch_one(&mut *tx).await?;
            if valid != 1 { return Err(conflict("Native Agent decision source Turn")); }
        }
        let terminal = params.attempt_status.is_terminal();
        let waiting_for_input = params.attempt_status == ExecutionAttemptStatus::WaitingInput;
        let question_present = params.question.is_some() || !waiting_for_input;
        let question_value = if waiting_for_input {
            params.question.as_ref().and_then(|value| value.as_deref())
        } else {
            None
        };
        let implicit_finished_at = terminal.then_some(now);
        let finished_present = params.finished_at.is_some() || terminal;
        let finished_value = params
            .finished_at
            .as_ref()
            .and_then(|value| *value)
            .or(implicit_finished_at);
        let attempt_result = sqlx::query(
            "UPDATE agent_execution_attempts SET \
                status = ?, question = CASE WHEN ? THEN ? ELSE question END, \
                error = CASE WHEN ? THEN ? ELSE error END, \
                output_summary = CASE WHEN ? THEN ? ELSE output_summary END, \
                output_files = COALESCE(?, output_files), \
                tokens = CASE WHEN ? THEN ? ELSE tokens END, \
                retry_after = CASE WHEN ? THEN ? ELSE retry_after END, \
                runtime_state = CASE WHEN ? THEN ? ELSE runtime_state END, \
                started_at = CASE WHEN ? THEN ? ELSE started_at END, \
                finished_at = CASE WHEN ? THEN ? ELSE finished_at END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND attempt_id = ? AND version = ? \
               AND status IN ('queued', 'running', 'waiting_input') \
               AND (status <> 'queued' OR ? = 'cancelled')",
        )
        .bind(params.attempt_status.as_str())
        .bind(question_present)
        .bind(question_value)
        .bind(params.error.is_some())
        .bind(params.error.as_ref().and_then(|value| value.as_deref()))
        .bind(params.output_summary.is_some())
        .bind(
            params
                .output_summary
                .as_ref()
                .and_then(|value| value.as_deref()),
        )
        .bind(&params.output_files)
        .bind(params.tokens.is_some())
        .bind(params.tokens.as_ref().and_then(|value| *value))
        .bind(params.retry_after.is_some())
        .bind(params.retry_after.as_ref().and_then(|value| *value))
        .bind(params.runtime_state.is_some())
        .bind(
            params
                .runtime_state
                .as_ref()
                .and_then(|value| value.as_deref()),
        )
        .bind(params.started_at.is_some())
        .bind(params.started_at.as_ref().and_then(|value| *value))
        .bind(finished_present)
        .bind(finished_value)
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(attempt_id)
        .bind(expected_attempt_version)
        .bind(params.attempt_status.as_str())
        .execute(&mut *tx)
        .await?;
        if attempt_result.rows_affected() != 1 {
            return Err(conflict("agent execution attempt"));
        }
        let step_result = sqlx::query(
            "UPDATE agent_execution_steps SET status = ?, \
                dispatch_after = CASE WHEN ? THEN ? ELSE dispatch_after END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND step_id = ? AND version = ? \
               AND superseded_in_revision IS NULL \
               AND EXISTS(SELECT 1 FROM agent_executions execution \
                          WHERE execution.execution_id = agent_execution_steps.execution_id \
                            AND execution.user_id = ? \
                            AND execution.deleted_at IS NULL)",
        )
        .bind(params.step_status.as_str())
        .bind(params.retry_after.is_some())
        .bind(params.retry_after.as_ref().and_then(|value| *value))
        .bind(now)
        .bind(execution_id)
        .bind(step_id)
        .bind(expected_step_version)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if step_result.rows_affected() != 1 {
            return Err(conflict("agent execution step"));
        }
        if let Some(reset) = params.loop_repeat_reset.as_ref() {
            apply_loop_repeat_reset_tx(&mut tx, execution_id, step_id, reset, now).await?;
        }
        let execution_result = sqlx::query(
            "UPDATE agent_executions SET status = CASE WHEN status='paused' THEN status ELSE COALESCE(?, status) END, \
                lease_owner = CASE WHEN status='paused' OR COALESCE(?, status) IN ('running', 'waiting_input') \
                                   THEN lease_owner ELSE NULL END, \
                lease_expires_at = CASE WHEN status='paused' OR COALESCE(?, status) IN ('running', 'waiting_input') \
                                        THEN lease_expires_at ELSE NULL END, \
                version = version + 1, updated_at = ? \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL \
               AND status NOT IN ('completed', 'completed_with_failures', 'failed', 'cancelled')",
        )
        .bind(params.execution_status.map(|value| value.as_str()))
        .bind(params.execution_status.map(|value| value.as_str()))
        .bind(params.execution_status.map(|value| value.as_str()))
        .bind(now)
        .bind(execution_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if execution_result.rows_affected() != 1 {
            return Err(conflict("agent execution"));
        }
        if params.execution_status == Some(AgentExecutionStatus::Cancelled) {
            terminate_unfinished_execution_children_tx(&mut tx, execution_id, None, now).await?;
        }
        if terminal {
            sqlx::query(
                "UPDATE conversation_execution_links SET active = 0, updated_at = ? \
                 WHERE execution_id = ? AND step_id = ? AND attempt_id = ? \
                   AND relation IN ('attempt', 'automation') AND active = 1",
            )
            .bind(now)
            .bind(execution_id)
            .bind(step_id)
            .bind(attempt_id)
            .execute(&mut *tx)
            .await?;
        }
        if params.step_status.is_terminal() {
            requeue_terminal_attempt_cleanup_tx(&mut tx, execution_id, step_id, now).await?;
        }
        let event = scoped_event(event, step_id, Some(attempt_id));
        append_event_tx(&mut tx, execution_id, &event, now).await?;
        let detail = load_step_detail_tx(&mut tx, user_id, execution_id, step_id)
            .await?
            .ok_or_else(|| conflict("agent execution step"))?;
        tx.commit().await?;
        Ok(detail)
    }

    async fn get_attempt(
        &self,
        user_id: &str,
        execution_id: &str,
        step_id: &str,
        attempt_id: &str,
    ) -> Result<Option<AgentExecutionAttemptDetailRow>, DbError> {
        let mut tx = self.pool.begin().await?;
        let owned: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_executions \
             WHERE execution_id = ? AND user_id = ? AND deleted_at IS NULL)",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if owned == 0 {
            tx.commit().await?;
            return Ok(None);
        }
        let row = attempt_details_tx(&mut tx, execution_id, Some(step_id))
            .await?
            .into_iter()
            .find(|detail| detail.attempt.attempt_id == attempt_id);
        tx.commit().await?;
        Ok(row)
    }

    async fn list_conversation_links(
        &self,
        user_id: &str,
        execution_id: &str,
    ) -> Result<Vec<ConversationExecutionLinkRow>, DbError> {
        Ok(sqlx::query_as::<_, ConversationExecutionLinkRow>(
            "SELECT link.* FROM conversation_execution_links link \
             JOIN agent_executions execution ON execution.execution_id = link.execution_id \
             WHERE link.execution_id = ? AND execution.user_id = ? \
               AND execution.deleted_at IS NULL \
             ORDER BY link.active DESC, link.created_at, link.id",
        )
        .bind(execution_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn resolve_conversation_link(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<ConversationExecutionLinkRow>, DbError> {
        Ok(sqlx::query_as::<_, ConversationExecutionLinkRow>(
            "SELECT link.* FROM conversation_execution_links link \
             JOIN agent_executions execution ON execution.execution_id = link.execution_id \
             JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
             WHERE link.conversation_id = ? \
               AND execution.user_id = ? AND session.state <> 'deleted' \
               AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
               AND json_extract(session.owner_ref_json, '$.principal_id') = ? \
             ORDER BY link.updated_at DESC, link.id",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn read_task_retirement_facts(
        &self, user_id: &str, conversation_id: &str, execution_id: &str,
        step_id: &str, attempt_id: &str,
    ) -> Result<Option<AgentExecutionTaskRetirementFacts>, DbError> {
        task_retirement::read(&self.pool,user_id,conversation_id,execution_id,step_id,attempt_id).await
    }

    async fn has_attempt_conversation_link(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<bool, DbError> {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM conversation_execution_links link \
                 JOIN agent_executions execution ON execution.execution_id = link.execution_id \
                 JOIN agent_sessions session ON session.agent_session_id = link.conversation_id \
                 WHERE link.conversation_id = ? AND link.relation = 'attempt' \
                   AND execution.user_id = ? AND session.state <> 'deleted' \
                   AND json_extract(session.owner_ref_json, '$.principal_kind') = 'user' \
                   AND json_extract(session.owner_ref_json, '$.principal_id') = ?\
             )",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists != 0)
    }

    async fn list_pending_conversation_cleanups(
        &self, execution_id: Option<&str>, cursor: Option<ConversationCleanupCursor>, limit: i64,
    ) -> Result<PendingConversationCleanupPage, DbError> {
        conversation_cleanup::list(&self.pool, execution_id, cursor, limit).await
    }

    async fn automation_source_cancelled(&self,user_id:&str,execution_id:&str)->Result<bool,DbError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_executions execution \
             JOIN requirements requirement ON requirement.requirement_id=json_extract(execution.initial_plan_input,'$.source.requirement_id') \
               AND requirement.claim_generation=json_extract(execution.initial_plan_input,'$.source.claim_generation') \
             JOIN conversation_execution_links lead ON lead.execution_id=execution.execution_id AND lead.relation='lead' \
               AND lead.conversation_id=requirement.owner_conversation_id \
             JOIN agent_sessions session ON session.agent_session_id=lead.conversation_id \
             WHERE execution.execution_id=? AND execution.user_id=? AND execution.deleted_at IS NULL \
               AND json_extract(execution.initial_plan_input,'$.mode')='automation' AND requirement.status='cancelled' \
               AND requirement.owner_terminal_id IS NULL AND requirement.claim_token IS NOT NULL \
               AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
               AND json_extract(session.owner_ref_json,'$.principal_id')=?)",
        ).bind(execution_id).bind(user_id).bind(user_id).fetch_one(&self.pool).await?)
    }

    async fn automation_source_cleanup_execution(&self,user_id:&str,requirement_id:&str,claim_generation:i64,
        source_operation_id:&str)->Result<Option<String>,DbError> {
        let ids:Vec<String>=sqlx::query_scalar(
            "SELECT execution_id FROM agent_executions WHERE user_id=? \
             AND json_extract(initial_plan_input,'$.mode')='automation' \
             AND json_extract(initial_plan_input,'$.source.requirement_id')=? \
             AND json_extract(initial_plan_input,'$.source.claim_generation')=? \
             AND json_extract(initial_plan_input,'$.source.operation_id')=? ORDER BY id LIMIT 2",
        ).bind(user_id).bind(requirement_id).bind(claim_generation).bind(source_operation_id).fetch_all(&self.pool).await?;
        match ids.as_slice() {[]=>Ok(None),[id]=>Ok(Some(id.clone())),_=>Err(conflict("ambiguous original AutoWork cleanup source"))}
    }

    async fn validate_conversation_cleanup(&self, cleanup: &PendingConversationCleanup) -> Result<bool, DbError> {
        conversation_cleanup::validate(&self.pool, cleanup).await
    }

    async fn mark_conversation_cleanup_completed_exact(
        &self, cleanup: &PendingConversationCleanup, completed_at: i64,
    ) -> Result<bool, DbError> {
        conversation_cleanup::acknowledge(&self.pool, cleanup, completed_at).await
    }
    async fn append_event(
        &self,
        user_id: &str,
        execution_id: &str,
        expected_version: i64,
        event: &NewAgentExecutionEvent,
    ) -> Result<AgentExecutionEventRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        bump_execution_version_tx(&mut tx, user_id, execution_id, expected_version, now).await?;
        let row = append_event_tx(&mut tx, execution_id, event, now).await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn list_events(
        &self,
        user_id: &str,
        execution_id: &str,
        after_sequence: i64,
        limit: i64,
    ) -> Result<Vec<AgentExecutionEventRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentExecutionEventRow>(
            "SELECT event.* FROM agent_execution_events event \
             JOIN agent_executions execution ON execution.execution_id = event.execution_id \
             WHERE event.execution_id = ? AND event.sequence > ? AND execution.user_id = ? \
               AND execution.deleted_at IS NULL \
             ORDER BY event.sequence LIMIT ?",
        )
        .bind(execution_id)
        .bind(after_sequence.max(0))
        .bind(user_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_unpublished_events(
        &self,
        limit: i64,
    ) -> Result<Vec<AgentExecutionEventRow>, DbError> {
        Ok(sqlx::query_as::<_, AgentExecutionEventRow>(
            "SELECT * FROM agent_execution_events WHERE published_at IS NULL \
             ORDER BY execution_id, sequence LIMIT ?",
        )
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?)
    }

    async fn mark_event_published(
        &self,
        execution_id: &str,
        sequence: i64,
        published_at: i64,
    ) -> Result<bool, DbError> {
        let result = sqlx::query(
            "UPDATE agent_execution_events \
             SET published_at = ? \
             WHERE execution_id = ? AND sequence = ? AND published_at IS NULL",
        )
        .bind(published_at)
        .bind(execution_id)
        .bind(sequence)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn list_reopenable_provider_usages(
        &self,
        provider_id: &str,
    ) -> Result<Vec<(String, String)>, DbError> {
        Ok(sqlx::query_as(
            "SELECT DISTINCT execution.execution_id, execution.goal \
             FROM agent_execution_participants participant \
             JOIN agent_executions execution ON execution.execution_id = participant.execution_id \
             WHERE participant.provider_id = ? \
               AND participant.retired_in_revision IS NULL \
               AND execution.status <> 'cancelled' \
               AND execution.deleted_at IS NULL \
             ORDER BY execution.updated_at DESC, execution.execution_id",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?)
    }
}
