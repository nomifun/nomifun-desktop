//! Native Restart consumes its original user command under the same writer as
//! resume admission. A reserved key is a lookup, never authorization by itself.
use sqlx::{Sqlite,Transaction};
use serde_json::{Value,json};
use crate::{AgentExecutionEventRow,AgentExecutionRestartTarget,DbError};

pub async fn ensure_execution_restart_resume_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,session:&str,
    operation:&str,key:&str,pause_revision:u64,checkpoint_revision:u64,checkpoint_digest:&str,
    prepared_fence:Option<u64>)->Result<(),DbError> {
    let Some(command_operation)=key.strip_prefix("execution-task-restart:") else {return Ok(());};
    let reject=||DbError::Conflict("native Restart no longer owns its exact original user command and pause".into());
    let rows:Vec<AgentExecutionEventRow>=sqlx::query_as("SELECT event.* FROM agent_execution_events event \
        JOIN agent_executions execution ON execution.execution_id=event.execution_id \
        WHERE execution.user_id=? AND execution.deleted_at IS NULL AND event.event_type='step_changed' \
          AND json_extract(event.payload,'$.change')='restart_requested' \
          AND json_extract(event.payload,'$.operation_id')=? LIMIT 2")
        .bind(user).bind(command_operation).fetch_all(&mut **tx).await?;
    let [event]=rows.as_slice() else {return Err(reject());};
    if event.actor_type!="user"||event.actor_id.as_deref()!=Some(user)||event.on_behalf_of_user_id!=user {
        return Err(reject());
    }
    let value:Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let target:AgentExecutionRestartTarget=serde_json::from_value(value.get("restart_target").cloned().ok_or_else(reject)?)
        .map_err(|error|DbError::Init(error.to_string()))?;
    let pause=target.pause.as_ref().ok_or_else(reject)?;
    let command=&value["command_request"];
    let request=&command["request"];
    let actor:nomifun_common::AgentExecutionActor=serde_json::from_value(command["actor"].clone()).map_err(|_|reject())?;
    let source_key=request["idempotency_key"].as_str().filter(|key|!key.trim().is_empty()&&key.len()<=128
        &&!key.chars().any(char::is_control)).ok_or_else(reject)?;
    let derived=format!("restart-step:{}",nomifun_agent_contracts::digest_payload(&json!({"kind":"restart_step",
        "owner_id":user,"execution_id":event.execution_id,"step_id":event.step_id,"idempotency_key":source_key}))
        .map_err(|error|DbError::Init(error.to_string()))?.as_ref());
    if command["kind"]!="restart_step"||command["owner_id"]!=user||command["execution_id"]!=event.execution_id
        ||command["step_id"].as_str()!=event.step_id.as_deref()
        ||!matches!(actor,nomifun_common::AgentExecutionActor::User {user_id} if user_id==user)
        ||request["expected_execution_version"].as_i64().is_none_or(|value|value<0)
        ||request["expected_step_version"].as_i64().is_none_or(|value|value<0)
        ||request["expected_attempt_id"].as_str()!=Some(target.attempt_id.as_str())
        ||derived!=command_operation||target.command_operation_id!=command_operation
        ||target.command_sequence!=event.sequence||target.execution_id!=event.execution_id
        ||Some(target.step_id.as_str())!=event.step_id.as_deref()||Some(target.attempt_id.as_str())!=event.attempt_id.as_deref()
        ||target.conversation_id!=session||target.canonical_operation_id!=operation
        ||pause.revision!=pause_revision||pause.checkpoint_revision!=checkpoint_revision
        ||pause.checkpoint_digest.as_deref()!=Some(checkpoint_digest)
        ||prepared_fence.is_some_and(|fence|fence!=pause.execution_fence) {
        return Err(reject());
    }
    // An old authorized receipt is read before this guard. A resolved command
    // can therefore never mint a new authorization, regardless of its outcome.
    let resolved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_events WHERE execution_id=? \
        AND step_id=? AND event_type='step_changed' AND json_extract(payload,'$.change')='restart_resolved' \
        AND json_extract(payload,'$.operation_id')=?)")
        .bind(&target.execution_id).bind(&target.step_id).bind(command_operation).fetch_one(&mut **tx).await?;
    if resolved {return Err(reject());}
    let original:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link \
        JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id \
        JOIN agent_execution_steps step ON step.execution_id=link.execution_id AND step.step_id=link.step_id \
        JOIN agent_executions execution ON execution.execution_id=link.execution_id \
        WHERE execution.execution_id=? AND execution.user_id=? AND execution.deleted_at IS NULL AND execution.status<>'cancelled' \
          AND step.step_id=? AND step.superseded_in_revision IS NULL AND step.status='running' AND attempt.attempt_id=? AND attempt.status='running' \
          AND link.conversation_id=? AND link.relation='attempt' AND link.active=1 \
          AND NOT EXISTS(SELECT 1 FROM agent_execution_attempts later WHERE later.execution_id=attempt.execution_id AND later.step_id=attempt.step_id AND later.attempt_no>attempt.attempt_no))")
        .bind(&target.execution_id).bind(user).bind(&target.step_id).bind(&target.attempt_id).bind(session).fetch_one(&mut **tx).await?;
    if !original||!super::agent_execution_admission::native_resume_target_eligible_tx(tx,user,session,operation).await? {
        return Err(reject());
    }
    let original_key=super::agent_execution_admission::execution_attempt_turn_key_tx(tx,user,&target.execution_id,&target.step_id,&target.attempt_id).await?;
    if operation!=format!("turn:user:{user}:{session}:{original_key}") {return Err(reject());}
    let native:Option<(String,i64,i64,Option<String>)>=sqlx::query_as("SELECT turn.native_pause_json,turn.execution_fence,turn.native_checkpoint_revision,turn.native_checkpoint_digest \
        FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id \
        JOIN agent_sessions session ON session.agent_session_id=turn.session_id \
        WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='running' AND turn.terminal_event_id IS NULL \
          AND turn.native_pause_json IS NOT NULL AND head.status='paused' AND head.active_turn_id=turn.operation_id \
          AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
          AND json_extract(session.owner_ref_json,'$.principal_id')=?")
        .bind(session).bind(operation).bind(user).fetch_optional(&mut **tx).await?;
    let Some((raw,fence,revision,digest))=native else {return Err(reject());};
    let current:nomifun_agent_contracts::NativePauseState=serde_json::from_str(&raw).map_err(|error|DbError::Init(error.to_string()))?;
    if fence<0||revision<0||fence as u64!=pause.execution_fence||revision as u64!=pause.checkpoint_revision
        ||digest.as_deref()!=pause.checkpoint_digest.as_deref()||current.revision!=pause.revision
        ||current.checkpoint_revision!=pause.checkpoint_revision||current.execution_fence!=pause.execution_fence
        ||current.checkpoint_digest.as_ref().map(|digest|digest.as_ref())!=pause.checkpoint_digest.as_deref()
        ||matches!(current.reason.as_str(),"EXECUTION_USER_REQUESTED"|"USER_REQUESTED") {
        return Err(reject());
    }
    Ok(())
}
