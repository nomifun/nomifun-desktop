//! Read-only capacity decisions over the existing Attempt and native owner
//! facts. Callers that admit work hold the SQLite writer through admission.
use sqlx::{Sqlite, Transaction};
use crate::{DbError, AttemptConversationEffects};
use nomifun_agent_contracts::NativePauseState;

/// Consume the exact domain authority under the canonical input writer. A
/// preflight check cannot fence a sender racing Stop or scheduler takeover.
/// Existing canonical same-key receipts are checked before calling this guard;
/// confirming that receipt never grants permission to execute again.
pub async fn validate_execution_turn_authority_tx(
    tx: &mut Transaction<'_, Sqlite>, user_id: &str, session_id: &str,
    operation_key: &str, authority: &crate::AgentExecutionTurnAuthority,
) -> Result<(), DbError> {
    if authority.lease_owner.trim().is_empty() || authority.expected_step_version < 0
        || authority.expected_attempt_version < 0 || operation_key.trim().is_empty() {
        return Err(DbError::Conflict("invalid exact Attempt input authority".into()));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| DbError::Init(error.to_string()))?.as_millis() as i64;
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_executions execution \
         JOIN agent_execution_steps step ON step.execution_id=execution.execution_id \
         JOIN agent_execution_attempts attempt ON attempt.execution_id=step.execution_id AND attempt.step_id=step.step_id \
         JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id AND link.step_id=attempt.step_id AND link.attempt_id=attempt.attempt_id \
         JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
         WHERE execution.execution_id=? AND execution.user_id=? AND execution.lease_owner=? AND execution.lease_expires_at>? \
           AND execution.deleted_at IS NULL AND execution.status IN('running','waiting_input','paused') \
           AND step.step_id=? AND step.version=? AND step.status='running' AND step.superseded_in_revision IS NULL \
           AND attempt.attempt_id=? AND attempt.version>=? AND attempt.status='running' \
           AND link.conversation_id=? AND link.active=1 AND link.relation IN('attempt','automation') \
           AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=? \
           AND (SELECT COUNT(*) FROM conversation_execution_links peer WHERE peer.conversation_id=link.conversation_id \
             AND peer.active=1 AND peer.relation IN('attempt','automation'))=1 \
           AND NOT EXISTS(SELECT 1 FROM agent_execution_events event,json_each(event.payload,'$.cancelled_step_ids') stopped \
             WHERE event.execution_id=execution.execution_id AND event.event_type='step_changed' \
               AND json_extract(event.payload,'$.action')='cancel_step' AND stopped.value=step.step_id))")
        .bind(&authority.execution_id).bind(user_id).bind(&authority.lease_owner).bind(now)
        .bind(&authority.step_id).bind(authority.expected_step_version).bind(&authority.attempt_id)
        .bind(authority.expected_attempt_version).bind(session_id).bind(user_id).fetch_one(&mut **tx).await?;
    if !valid { return Err(DbError::Conflict("original Attempt input was fenced by ownership, Stop or takeover".into())); }
    let exact_key = execution_attempt_turn_key_tx(tx,user_id,&authority.execution_id,&authority.step_id,&authority.attempt_id).await?;
    if exact_key != operation_key { return Err(DbError::Conflict("input does not name the original Attempt operation".into())); }
    let (initial, lead): (String, Option<String>) = sqlx::query_as(
        "SELECT initial_plan_input,(SELECT conversation_id FROM conversation_execution_links WHERE execution_id=execution.execution_id \
         AND relation='lead' ORDER BY id DESC LIMIT 1) FROM agent_executions execution WHERE execution_id=?")
        .bind(&authority.execution_id).fetch_one(&mut **tx).await?;
    require_autowork_admitted_tx(tx,user_id,&initial,lead.as_deref()).await
}

#[derive(Debug)]
pub struct AgentExecutionReservation {
    pub attempt_id: String,
    pub step_id: String,
    pub participant_id: Option<String>,
    pub capacity_held: bool,
}

pub async fn execution_reservations_tx(tx: &mut Transaction<'_, Sqlite>, execution_id: &str)
    -> Result<Vec<AgentExecutionReservation>, DbError> {
    let rows: Vec<(String, String, Option<String>, Option<String>,String)> = sqlx::query_as(
        "SELECT attempt.attempt_id,step.step_id,step.assigned_participant_id,attempt.runtime_state,attempt.status \
         FROM agent_execution_attempts attempt JOIN agent_execution_steps step \
         ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
         WHERE attempt.execution_id=? AND attempt.status IN ('queued','running','waiting_input') \
           AND step.kind='agent' AND step.superseded_in_revision IS NULL ORDER BY attempt.id",
    ).bind(execution_id).fetch_all(&mut **tx).await?;
    let mut reservations = Vec::with_capacity(rows.len());
    for (attempt_id, step_id, participant_id, runtime_state,status) in rows {
        let effects = AttemptConversationEffects::decode(runtime_state.as_deref())
            .map_err(|error| DbError::Init(error.to_string()))?;
        let turn_key = current_attempt_turn_key_tx(tx,execution_id,&step_id,&attempt_id,&effects).await?;
        let native: Vec<(String, Option<String>, Option<String>, Option<String>, i64, i64, i64, Option<String>, bool)> = sqlx::query_as(
            "SELECT turn.operation_id,head.status,head.active_turn_id,turn.native_pause_json, \
             turn.execution_fence,turn.native_pause_revision,turn.native_checkpoint_revision,turn.native_checkpoint_digest, \
             EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=link.conversation_id AND effect.state IN ('pending','unknown')) \
             FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id \
             JOIN agent_turns turn ON turn.session_id=link.conversation_id \
               AND turn.operation_id='turn:user:'||execution.user_id||':'||link.conversation_id||':'||? \
             LEFT JOIN agent_session_heads head ON head.session_id=link.conversation_id \
             WHERE link.execution_id=? AND link.attempt_id=? AND link.active=1 \
               AND link.relation IN ('attempt','automation') AND turn.state='running' LIMIT 2",
        ).bind(&turn_key).bind(execution_id).bind(&attempt_id).fetch_all(&mut **tx).await?;
        let closed_input = status=="waiting_input" && effects.review_blocked.is_none()
            && closed_input_required_tx(tx,execution_id,&step_id,&attempt_id,&turn_key).await?;
        // A typed, exact closed InputRequired source may release its model
        // slot. Review/unknown/live-native WaitingInput has no such authority.
        let mut capacity_held = !closed_input;
        if let [(operation, head, active_turn, Some(raw), fence, revision, checkpoint_revision, digest, unsettled)] = native.as_slice() {
            let pause: NativePauseState = serde_json::from_str(raw)
                .map_err(|error| DbError::Init(format!("invalid canonical native pause: {error}")))?;
            // Neither waiting, a checkpoint, idleness nor a terminal-looking
            // display releases a reservation. The original native owner must
            // have fenced itself and committed its actual cleanup evidence.
            capacity_held = !(head.as_deref() == Some("paused") && active_turn.as_deref() == Some(operation)
                && pause.cleanup_proven && !unsettled && *fence >= 0 && pause.execution_fence == *fence as u64
                && *revision > 0 && pause.revision == *revision as u64 && *checkpoint_revision > 0
                && pause.checkpoint_revision == *checkpoint_revision as u64
                && pause.checkpoint_digest.as_ref().map(|value| value.as_ref()) == digest.as_deref()
                && digest.is_some());
        }
        if capacity_held && status == "running" && effects.review_blocked.is_none()
            && let [(operation, ..)] = native.as_slice()
            && awaiting_children_tx(tx, execution_id, &step_id, &attempt_id, operation, false).await? {
            capacity_held = false;
        }
        reservations.push(AgentExecutionReservation { attempt_id, step_id, participant_id, capacity_held });
    }
    Ok(reservations)
}

// Only the original native producer's exact Turn may yield model capacity.
// A recovered waiting checkpoint can carry that proof across lease generations;
// the paired resolved event restores the reservation under the admission writer.
async fn awaiting_children_tx(tx: &mut Transaction<'_, Sqlite>, execution_id: &str,
    step_id: &str, attempt_id: &str, operation_id: &str, require_checkpoint: bool) -> Result<bool, DbError> {
    type WaitProof = (String, String, i64, i64, Option<i64>, i64, Option<String>, Option<String>, i64, i64, i64);
    let proof: Option<WaitProof> = sqlx::query_as(
        "SELECT progress.session_id,progress.event_id,progress.seq,turn.execution_generation,turn.native_checkpoint_seq,turn.native_checkpoint_revision, \
           turn.native_checkpoint_digest,turn.native_checkpoint_json,progress.kind_version,head.last_seq,started.seq \
         FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
         JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         JOIN agent_turns turn ON turn.session_id=session.agent_session_id AND turn.operation_id=? \
         JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id \
         JOIN agent_events progress ON progress.session_id=turn.session_id AND progress.correlation_id=turn.operation_id \
         LEFT JOIN agent_payloads payload ON payload.session_id=progress.session_id AND payload.payload_id=progress.payload_id \
         WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.active=1 AND link.relation IN('attempt','automation') \
           AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=execution.user_id \
           AND head.status='running' AND head.active_turn_id=turn.operation_id AND turn.state='running' \
           AND turn.execution_generation>0 AND turn.execution_owner IS NOT NULL AND turn.execution_owner<>'' \
           AND started.kind='turn/started' AND started.kind_version=1 AND started.correlation_id=turn.operation_id \
           AND progress.kind='runtime/progress-recorded' AND progress.producer_id='runtime_supervisor' \
           AND progress.seq>started.seq AND progress.seq<=head.last_seq \
           AND (progress.causation_event_id=started.event_id OR progress.causation_event_id=started.causation_event_id) \
           AND (SELECT COUNT(*) FROM conversation_execution_links peer WHERE peer.conversation_id=link.conversation_id \
             AND peer.active=1 AND peer.relation IN('attempt','automation'))=1 \
           AND COALESCE(json_extract(progress.inline_json,'$.event.event'),json_extract(CAST(payload.body AS TEXT),'$.value.event.event')) \
             IN('awaiting_children','children_wait_resolved') ORDER BY progress.seq DESC LIMIT 1",
    ).bind(operation_id).bind(execution_id).bind(step_id).bind(attempt_id).fetch_optional(&mut **tx).await?;
    let Some((session, event_id, sequence, generation, checkpoint_seq, checkpoint_revision, checkpoint_digest, checkpoint_json,
        kind_version, last_seq, started_seq)) = proof else { return Ok(false); };
    if kind_version != 1 { return Ok(false); }
    let value=super::agent_source_reply::event_payload_tx(tx,&session,&event_id).await?;
    if value.pointer("/event/event").and_then(serde_json::Value::as_str) != Some("awaiting_children") {
        return Ok(false);
    }
    let Some(request) = value.pointer("/event/request").filter(|request| request.is_object()) else { return Ok(false); };
    if !require_checkpoint && sequence >= generation { return Ok(true); }
    let (Some(checkpoint_seq), Some(checkpoint_digest), Some(checkpoint_json)) =
        (checkpoint_seq, checkpoint_digest, checkpoint_json) else { return Ok(false); };
    if checkpoint_revision <= 0 || checkpoint_seq < started_seq || checkpoint_seq > last_seq { return Ok(false); }
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint_json)
        .map_err(|error| DbError::Init(format!("invalid waiting checkpoint: {error}")))?;
    let digest = nomifun_agent_contracts::digest_payload(&checkpoint)
        .map_err(|error| DbError::Init(format!("invalid waiting checkpoint digest: {error}")))?;
    if digest.0 != checkpoint_digest { return Err(DbError::Init("waiting checkpoint digest mismatch".into())); }
    Ok(checkpoint.get("awaiting_children") == Some(request)
        && checkpoint.get("turn_operation_id").and_then(serde_json::Value::as_str) == Some(operation_id))
}

async fn closed_input_required_tx(tx:&mut Transaction<'_,Sqlite>,execution_id:&str,step_id:&str,attempt_id:&str,
    turn_key:&str)->Result<bool,DbError> {
    let rows:Vec<(String,String,Option<String>)>=sqlx::query_as(
        "SELECT terminal.session_id,terminal.event_id,attempt.question FROM conversation_execution_links link \
         JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id \
         JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
         JOIN agent_session_heads head ON head.session_id=session.agent_session_id \
         JOIN agent_turns turn ON turn.session_id=session.agent_session_id AND turn.operation_id='turn:user:'||execution.user_id||':'||link.conversation_id||':'||? \
         JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id \
         JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id \
         WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.active=1 AND link.relation IN('attempt','automation') \
           AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=execution.user_id \
           AND head.active_turn_id IS NULL AND head.status='ready' AND turn.state='completed' \
           AND started.kind='turn/started' AND started.kind_version=1 AND started.correlation_id=turn.operation_id \
           AND terminal.kind='turn/completed' AND terminal.kind_version=1 AND terminal.correlation_id=turn.operation_id AND terminal.seq>started.seq \
           AND head.last_seq>=terminal.seq \
           AND (SELECT COUNT(*) FROM conversation_execution_links peer WHERE peer.conversation_id=link.conversation_id AND peer.active=1 AND peer.relation IN('attempt','automation'))=1 \
           AND NOT EXISTS(SELECT 1 FROM agent_effects effect WHERE effect.session_id=session.agent_session_id AND effect.state IN('pending','unknown')) \
           AND NOT EXISTS(SELECT 1 FROM agent_turns successor WHERE successor.session_id=turn.session_id AND successor.rowid>turn.rowid) LIMIT 2"
    ).bind(turn_key).bind(execution_id).bind(step_id).bind(attempt_id).fetch_all(&mut **tx).await?;
    let [(session,event_id,question)]=rows.as_slice() else {return Ok(false);};
    let value=super::agent_source_reply::event_payload_tx(tx,session,event_id).await?;
    let Some(request)=value.get("user_input_request") else {return Ok(false);};
    let request:nomifun_agent_contracts::AgentUserInputRequest=serde_json::from_value(request.clone())
        .map_err(|error|DbError::Init(format!("invalid canonical user input request: {error}")))?;
    request.validate().map_err(DbError::Init)?;
    Ok(question.as_deref()==Some(request.question.as_str()))
}

/// Revalidate the user's existing Execution/Participant limits under the same
/// writer that creates/resumes the exact Attempt. It never allocates a quota,
/// alters policy, pauses a task, or starts native work.
pub async fn execution_capacity_available_tx(tx: &mut Transaction<'_, Sqlite>, execution_id: &str,
    step_id: &str, excluding_attempt_id: Option<&str>) -> Result<bool, DbError> {
    if !continuation_input_available_tx(tx,execution_id,step_id).await? {return Ok(false);}
    let policy: Option<(String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT execution.status,execution.max_parallel,step.assigned_participant_id,participant.constraints \
         FROM agent_execution_steps step JOIN agent_executions execution ON execution.execution_id=step.execution_id \
         LEFT JOIN agent_execution_participants participant ON participant.execution_id=step.execution_id \
            AND participant.participant_id=step.assigned_participant_id AND participant.retired_in_revision IS NULL \
         WHERE step.execution_id=? AND step.step_id=? AND step.superseded_in_revision IS NULL \
           AND execution.deleted_at IS NULL AND step.kind='agent' AND participant.participant_id IS NOT NULL",
    ).bind(execution_id).bind(step_id).fetch_optional(&mut **tx).await?;
    let Some((status, max_parallel, participant, constraints)) = policy else {
        return Err(DbError::Conflict("exact Agent Execution admission target is missing".into()));
    };
    if status == "paused" { return Ok(false); }
    if !matches!(status.as_str(), "running" | "waiting_input") {
        return Err(DbError::Conflict("Agent Execution is no longer active for admission".into()));
    }
    if !(1..=nomifun_common::MAX_AGENT_EXECUTION_PARALLELISM).contains(&max_parallel) {
        return Err(DbError::Init("invalid persisted Execution concurrency".into()));
    }
    let participant_limit = if let Some(raw) = constraints {
        super::agent_execution::validate_participant_constraints_json(&raw)?;
        serde_json::from_str::<serde_json::Value>(&raw).map_err(|error| DbError::Init(error.to_string()))?
            .get("max_concurrency").and_then(serde_json::Value::as_i64).unwrap_or(i64::MAX)
    } else { i64::MAX };
    let reservations = execution_reservations_tx(tx, execution_id).await?;
    let peers = reservations.iter().filter(|reservation| Some(reservation.attempt_id.as_str()) != excluding_attempt_id).collect::<Vec<_>>();
    if peers.iter().filter(|reservation| reservation.capacity_held).count() >= max_parallel as usize
        || peers.iter().filter(|reservation| reservation.capacity_held && reservation.participant_id == participant).count() as i64 >= participant_limit {
        return Ok(false);
    }
    Ok(true)
}

/// Independent inputs to one exact child Session retain command order. This
/// is Session ownership, not a business dependency on prior input success.
pub(super) async fn continuation_input_available_tx(tx:&mut Transaction<'_,Sqlite>,execution_id:&str,step_id:&str)
    ->Result<bool,DbError> {
    let target:Option<(i64,String,String)>=sqlx::query_as("SELECT sequence,json_extract(payload,'$.target_session_id'),json_extract(payload,'$.continuation_of_step_id') FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND json_extract(payload,'$.continuation_step_id')=? ORDER BY sequence LIMIT 1")
        .bind(execution_id).bind(step_id).fetch_optional(&mut **tx).await?;
    let Some((sequence,session,original))=target else {return Ok(true);};
    let original_live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND superseded_in_revision IS NULL AND status IN('pending','running','waiting_input'))")
        .bind(execution_id).bind(original).fetch_one(&mut **tx).await?;
    if original_live {return Ok(false);}
    let preceding:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_events command JOIN agent_execution_steps step ON step.execution_id=command.execution_id AND step.step_id=json_extract(command.payload,'$.continuation_step_id') WHERE command.execution_id=? AND command.sequence<? AND json_extract(command.payload,'$.target_session_id')=? AND step.superseded_in_revision IS NULL AND step.status IN('pending','running','waiting_input'))")
        .bind(execution_id).bind(sequence).bind(session).fetch_one(&mut **tx).await?;
    Ok(!preceding)
}

/// Canonical native resume uses this narrow guard inside its original writer
/// transaction. A retained history link never grants authority over another
/// Turn, and an inactive/cancelled exact Attempt cannot become ordinary input.
pub async fn native_resume_capacity_available_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<bool, DbError> {
    crate::ensure_remote_session_admitted_tx(tx,session_id).await?;
    require_cron_target_admitted_tx(tx,user_id,session_id,canonical_operation_id).await?;
    let (execution_id, step_id, attempt_id) = match native_attempt_identity_tx(tx, user_id, session_id, canonical_operation_id).await? {
        NativeAttemptIdentity::Unbound => return Ok(true),
        NativeAttemptIdentity::Fenced => return Err(DbError::Conflict("native resume no longer owns its exact active Attempt".into())),
        NativeAttemptIdentity::Current {execution_id,step_id,attempt_id} => (execution_id,step_id,attempt_id),
    };
    execution_capacity_available_tx(tx, &execution_id, &step_id, Some(&attempt_id)).await
}

/// Claim consumes a held reservation or reconstructs an exact child waiter.
/// Neither path resumes yielded model work without its admission writer.
pub async fn native_execution_claim_admitted_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<bool, DbError> {
    crate::ensure_remote_session_admitted_tx(tx,session_id).await?;
    require_cron_target_admitted_tx(tx,user_id,session_id,canonical_operation_id).await?;
    let (execution_id, step_id, attempt_id) = match native_attempt_identity_tx(tx, user_id, session_id, canonical_operation_id).await? {
        NativeAttemptIdentity::Unbound => return Ok(true),
        NativeAttemptIdentity::Fenced => return Err(DbError::Conflict("native claim no longer owns its exact active Attempt".into())),
        NativeAttemptIdentity::Current {execution_id,step_id,attempt_id} => (execution_id,step_id,attempt_id),
    };
    let status: String = sqlx::query_scalar("SELECT status FROM agent_executions WHERE execution_id=?")
        .bind(&execution_id).fetch_one(&mut **tx).await?;
    if !matches!(status.as_str(), "running" | "waiting_input" | "paused") {
        return Err(DbError::Conflict("native claim targets a settled Execution".into()));
    }
    if execution_reservations_tx(tx, &execution_id).await?.iter()
        .any(|reservation| reservation.attempt_id == attempt_id && reservation.capacity_held) { return Ok(true); }
    // Reconstructing the exact waiting checkpoint does not resume model work.
    // ChildrenWaitResolved must still reacquire capacity before the next request.
    awaiting_children_tx(tx, &execution_id, &step_id, &attempt_id, canonical_operation_id, true).await
}

/// Queue/history projection needs to distinguish an ordinary durable fence
/// from damaged canonical data. False means only a known domain target fence;
/// SQL/JSON/schema failures are returned intact and never hidden as a stop.
pub async fn native_resume_target_eligible_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<bool, DbError> {
    if !crate::remote_session_admitted_tx(tx,session_id).await? {return Ok(false);}
    if matches!(cron_target_admitted_tx(tx,user_id,session_id,canonical_operation_id).await?,Some(false)) { return Ok(false); }
    Ok(!matches!(native_attempt_identity_tx(tx,user_id,session_id,canonical_operation_id).await?,NativeAttemptIdentity::Fenced))
}

/// Match the host-derived operation against the original immutable Cron
/// reservation. Ordinary input without that domain relation remains unbound.
/// The writer that claims/resumes native execution consumes cancellation in
/// the same transaction, so a preflight read can never revive a stopped run.
async fn cron_target_admitted_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<Option<bool>, DbError> {
    Ok(sqlx::query_scalar("SELECT reservation.status='reserved' AND reservation.cancel_requested_at_ms IS NULL \
        FROM cron_run_reservations reservation LEFT JOIN cron_jobs job ON job.cron_job_id=reservation.cron_job_id \
        JOIN agent_sessions session ON session.agent_session_id=reservation.conversation_id \
        WHERE (job.user_id=? OR (job.cron_job_id IS NULL AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
          AND json_extract(session.owner_ref_json,'$.principal_id')=?)) AND reservation.conversation_id=? \
          AND ?='turn:user:'||?||':'||reservation.conversation_id||':cron:'||reservation.cron_job_run_id||':turn'")
        .bind(user_id).bind(user_id).bind(session_id).bind(canonical_operation_id).bind(user_id).fetch_optional(&mut **tx).await?)
}

async fn require_cron_target_admitted_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<(), DbError> {
    if matches!(cron_target_admitted_tx(tx,user_id,session_id,canonical_operation_id).await?,Some(false)) {
        return Err(DbError::Conflict("native execution no longer owns its exact uncancelled Cron occurrence".into()));
    }
    Ok(())
}

enum NativeAttemptIdentity { Unbound, Current {execution_id:String,step_id:String,attempt_id:String}, Fenced }

/// The existing Requirement claim is the cancellation fence for its one
/// AutoWork generation. Read it under the writer that admits native/Attempt
/// work, including after restart; a missed in-memory signal cannot revive it.
pub(super) async fn require_autowork_admitted_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    initial_plan_input: &str, lead_session_id: Option<&str>) -> Result<(), DbError> {
    if !autowork_admitted_tx(tx, user_id, initial_plan_input, lead_session_id).await? {
        return Err(DbError::Conflict("AutoWork no longer owns its exact uncancelled Requirement claim".into()));
    }
    Ok(())
}

async fn autowork_admitted_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    initial_plan_input: &str, lead_session_id: Option<&str>) -> Result<bool, DbError> {
    let plan: serde_json::Value = serde_json::from_str(initial_plan_input)
        .map_err(|error| DbError::Init(format!("invalid persisted execution input: {error}")))?;
    if plan.get("mode").and_then(serde_json::Value::as_str) != Some("automation") { return Ok(true); }
    let requirement_id = plan.pointer("/source/requirement_id").and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::Init("AutoWork source has no Requirement identity".into()))?;
    let generation = plan.pointer("/source/claim_generation").and_then(serde_json::Value::as_i64)
        .filter(|value| *value > 0).ok_or_else(|| DbError::Init("AutoWork source has no claim generation".into()))?;
    let Some(session_id) = lead_session_id else { return Ok(false); };
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirements requirement JOIN agent_sessions session \
        ON session.agent_session_id=requirement.owner_conversation_id \
        WHERE requirement.requirement_id=? AND requirement.claim_generation=? \
          AND requirement.owner_conversation_id=? AND requirement.owner_terminal_id IS NULL \
          AND requirement.claim_token IS NOT NULL AND requirement.status<>'cancelled' \
          AND session.state='live' \
          AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
          AND json_extract(session.owner_ref_json,'$.principal_id')=?)")
        .bind(requirement_id).bind(generation).bind(session_id).bind(user_id).fetch_one(&mut **tx).await?)
}

async fn native_attempt_identity_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    session_id: &str, canonical_operation_id: &str) -> Result<NativeAttemptIdentity, DbError> {
    type Link = (String, String, String, String, bool, String, Option<String>, String, String, Option<i64>, Option<i64>);
    let links: Vec<Link> = sqlx::query_as(
        "SELECT link.execution_id,link.step_id,link.attempt_id,link.relation,link.active,execution.user_id, \
         attempt.runtime_state,attempt.status,step.status,step.superseded_in_revision,execution.deleted_at \
         FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id \
         JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id AND attempt.attempt_id=link.attempt_id \
         JOIN agent_execution_steps step ON step.execution_id=link.execution_id AND step.step_id=link.step_id \
         WHERE link.conversation_id=? AND link.relation IN ('attempt','automation') ORDER BY link.id",
    ).bind(session_id).fetch_all(&mut **tx).await?;
    let child_session = links.iter().any(|link| link.3 == "attempt");
    let mut exact = Vec::new();
    for link in &links {
        let effects = AttemptConversationEffects::decode(link.6.as_deref()).map_err(|error| DbError::Init(error.to_string()))?;
        let key = current_attempt_turn_key_tx(tx,&link.0,&link.1,&link.2,&effects).await?;
        if canonical_operation_id == format!("turn:user:{}:{session_id}:{key}", link.5) { exact.push(link); }
    }
    let (execution_id, step_id, attempt_id) = match exact.as_slice() {
        [] if !child_session => return Ok(NativeAttemptIdentity::Unbound),
        [link] if link.4 && link.5 == user_id && link.7 == "running" && link.8 == "running"
            && link.9.is_none() && link.10.is_none() => (&link.0, &link.1, &link.2),
        _ => return Ok(NativeAttemptIdentity::Fenced),
    };
    let stopped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_execution_events event,json_each(event.payload,'$.cancelled_step_ids') selected \
         WHERE event.execution_id=? AND event.event_type='step_changed' \
           AND json_extract(event.payload,'$.action')='cancel_step' AND selected.value=?)",
    ).bind(execution_id).bind(step_id).fetch_one(&mut **tx).await?;
    if stopped { return Ok(NativeAttemptIdentity::Fenced); }
    let (input, lead): (String, Option<String>) = sqlx::query_as(
        "SELECT initial_plan_input,(SELECT conversation_id FROM conversation_execution_links \
         WHERE execution_id=execution.execution_id AND relation='lead' ORDER BY id DESC LIMIT 1) \
         FROM agent_executions execution WHERE execution_id=?")
        .bind(execution_id).fetch_one(&mut **tx).await?;
    if !autowork_admitted_tx(tx, user_id, &input, lead.as_deref()).await? { return Ok(NativeAttemptIdentity::Fenced); }
    Ok(NativeAttemptIdentity::Current {execution_id:execution_id.clone(),step_id:step_id.clone(),attempt_id:attempt_id.clone()})
}

async fn current_attempt_turn_key_tx(tx:&mut Transaction<'_,Sqlite>,execution_id:&str,step_id:&str,attempt_id:&str,
    effects:&AttemptConversationEffects)->Result<String,DbError> {
    if !effects.pending_conversation_effects.is_empty() {return execution_attempt_turn_key_from_facts(attempt_id,effects,None);}
    // A completed handoff retires its delivery queue, not its original Turn
    // identity. Resolve the last exact reference from the existing domain
    // event, just as native claim/resume do; never select the Session's latest.
    let reference=sqlx::query_scalar::<_,String>("SELECT json_extract(payload,'$.turn_operation_key') FROM agent_execution_events \
        WHERE execution_id=? AND step_id=? AND attempt_id=? AND json_type(payload,'$.turn_operation_key')='text' ORDER BY sequence DESC LIMIT 1")
        .bind(execution_id).bind(step_id).bind(attempt_id).fetch_optional(&mut **tx).await?;
    execution_attempt_turn_key_from_facts(attempt_id,effects,reference)
}

/// Resolve an exact Attempt key from the original runtime state and its last
/// committed domain reference, read together by the caller's snapshot.
pub fn execution_attempt_turn_key_from_facts(attempt_id:&str,effects:&AttemptConversationEffects,reference:Option<String>)
    ->Result<String,DbError> {
    let key=effects.current_turn_operation_key(attempt_id).map_err(|error|DbError::Init(error.to_string()))?;
    Ok(if effects.pending_conversation_effects.is_empty() {reference.unwrap_or(key)} else {key})
}

/// Observe the original exact Attempt, including after Stop retires its link.
/// This shares claim/resume's key resolution and never selects a latest Turn.
pub async fn execution_attempt_turn_key_tx(tx:&mut Transaction<'_,Sqlite>,user_id:&str,execution_id:&str,
    step_id:&str,attempt_id:&str)->Result<String,DbError> {
    let row:Option<(Option<String>,)>=sqlx::query_as("SELECT attempt.runtime_state FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id JOIN agent_executions execution ON execution.execution_id=attempt.execution_id WHERE execution.user_id=? AND attempt.execution_id=? AND attempt.step_id=? AND attempt.attempt_id=?")
        .bind(user_id).bind(execution_id).bind(step_id).bind(attempt_id).fetch_optional(&mut **tx).await?;
    let (state,)=row.ok_or_else(||DbError::Conflict("original exact Attempt key owner differs".into()))?;
    let effects=AttemptConversationEffects::decode(state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
    current_attempt_turn_key_tx(tx,execution_id,step_id,attempt_id,&effects).await
}
