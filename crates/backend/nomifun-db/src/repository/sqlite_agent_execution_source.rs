//! Source-authored batches and new inputs to an existing child use the
//! original Execution writer, events and Session identity.
use super::*;

pub(super) async fn find_command_tx(tx: &mut Transaction<'_, Sqlite>, user_id: &str,
    identity: &DelegationCreationIdentity) -> Result<Option<AgentExecutionRow>, DbError> {
    let mut rows = sqlx::query_as::<_, AgentExecutionRow>(
        "SELECT DISTINCT execution.* FROM agent_executions execution \
         JOIN agent_execution_events command ON command.execution_id=execution.execution_id \
           AND command.event_type IN('created','plan_changed') \
         WHERE execution.user_id=? AND execution.deleted_at IS NULL \
           AND COALESCE(json_extract(command.payload,'$.delegation_source.agent_session_id'),json_extract(execution.initial_plan_input,'$.source.agent_session_id'))=? \
           AND COALESCE(json_extract(command.payload,'$.delegation_source.turn_operation_id'),json_extract(execution.initial_plan_input,'$.source.turn_operation_id'))=? \
           AND json_extract(command.payload,'$.delegation_command_digest')=? \
         ORDER BY execution.id LIMIT 2",
    ).bind(user_id).bind(&identity.session_id).bind(&identity.turn_operation_id).bind(&identity.digest)
        .fetch_all(&mut **tx).await?;
    if rows.len() > 1 { return Err(conflict("delegation creation identity")); }
    Ok(rows.pop())
}

fn initial_source(row: &AgentExecutionRow) -> Result<Option<nomifun_common::AgentExecutionSource>, DbError> {
    let value: serde_json::Value = serde_json::from_str(&row.initial_plan_input)
        .map_err(|error| DbError::Init(format!("invalid original source command: {error}")))?;
    match value.get("mode").and_then(serde_json::Value::as_str) {
        Some("explicit"|"automatic")=>{},
        Some("automation")=>return Ok(None),
        _=>return Err(conflict("supported active source container")),
    }
    value.get("source").filter(|source|!source.is_null()).cloned().map(serde_json::from_value).transpose()
        .map_err(|error| DbError::Init(format!("invalid original delegation source: {error}")))
}

async fn owning_source_tx(tx:&mut Transaction<'_,Sqlite>,row:&AgentExecutionRow,step_id:&str,
    sender_session:&str,sender_turn:&str)->Result<nomifun_common::AgentExecutionSource,DbError> {
    let original=initial_source(row)?;
    if let Some(source)=original.as_ref().filter(|source|source.agent_session_id==sender_session&&source.turn_operation_id==sender_turn) {
        return Ok(source.clone());
    }
    let step=step_source_tx(tx,&row.user_id,&row.execution_id,step_id).await?;
    if let Some(source)=step.as_ref().filter(|source|source.agent_session_id==sender_session&&source.turn_operation_id==sender_turn) {
        return Ok(source.clone());
    }
    if !original.as_ref().into_iter().chain(step.as_ref()).any(|source|source.agent_session_id==sender_session) {
        return Err(conflict("child followup source ownership"));
    }
    let turns=crate::agent_reply_source_turns_tx(tx,&row.user_id,sender_session,sender_turn).await?;
    original.into_iter().chain(step).find(|source|source.agent_session_id==sender_session&&turns.contains(&source.turn_operation_id))
        .ok_or_else(||conflict("child followup source ownership"))
}

pub(super) async fn append(pool: &SqlitePool, user_id: &str, execution_id: &str, expected_version: i64,
    source: &AgentExecutionSourceAppend, params: &AppendAgentExecutionStepsParams, event: &NewAgentExecutionEvent)
    -> Result<AppendAgentExecutionStepsFromAttemptResult, DbError> {
    let new_ids = validate_step_append(params, event, matches!(source,AgentExecutionSourceAppend::ChildFollowup {..}))?;
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let row: AgentExecutionRow = sqlx::query_as(
        "UPDATE agent_executions SET version=version WHERE execution_id=? AND user_id=? AND deleted_at IS NULL RETURNING *",
    ).bind(execution_id).bind(user_id).fetch_optional(&mut *tx).await?.ok_or_else(|| conflict("source Execution"))?;
    let original = initial_source(&row)?;
    let delivery_source;
    let admitted_session;
    let admitted_turn;
    let mut scoped = event.clone();
    let mut depth = None;
    let payload = match source {
        AgentExecutionSourceAppend::Delegation { command } => {
            let original=original.as_ref().ok_or_else(||conflict("source-authored Execution"))?;
            delivery_source=original.clone();
            let identity = delegation_creation_identity(command)?.ok_or_else(|| conflict("source delegation command"))?;
            if original.agent_session_id != identity.session_id {
                return Err(conflict("delegation source ownership"));
            }
            admitted_session=identity.session_id.clone();admitted_turn=identity.turn_operation_id.clone();
            if let Some(prior) = find_command_tx(&mut tx, user_id, &identity).await? {
                if prior.execution_id != execution_id { return Err(conflict("source command Execution")); }
                let detail = load_execution_detail_tx(&mut tx,user_id,execution_id).await?.ok_or_else(||conflict("source Execution"))?;
                let payload:Option<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.delegation_command_digest')=? AND json_type(payload,'$.added_step_ids')='array' ORDER BY sequence LIMIT 1")
                    .bind(execution_id).bind(&identity.digest).fetch_optional(&mut *tx).await?;
                let added_step_ids=match payload {
                    Some(payload)=>delegation_added_step_ids(&payload)?,
                    None=>{
                        let first=detail.steps.iter().map(|step|step.introduced_in_revision).min();
                        detail.steps.iter().filter(|step|Some(step.introduced_in_revision)==first).map(|step|step.step_id.clone()).collect()
                    },
                };
                tx.commit().await?;
                return Ok(AppendAgentExecutionStepsFromAttemptResult { detail, added_step_ids });
            }
            if original.turn_operation_id!=identity.turn_operation_id
                &&!crate::agent_reply_source_turns_tx(&mut tx,user_id,&identity.session_id,&identity.turn_operation_id).await?.contains(&original.turn_operation_id) {
                return Err(conflict("delegation reply source ownership"));
            }
            let value:serde_json::Value=serde_json::from_str(&command.initial_plan_input)
                .map_err(|error|DbError::Conflict(error.to_string()))?;
            serde_json::json!({"change":"source_steps_appended","delegation_command_digest":identity.digest,
                "delegation_source":value["source"],"added_step_ids":params.new_steps.iter().map(|step|&step.step_id).collect::<Vec<_>>()})
        }
        AgentExecutionSourceAppend::ChildFollowup {message_source,original_step_id,operation_id,content_digest} => {
            let sender_session=&message_source.agent_session_id;let sender_turn=&message_source.turn_operation_id;
            admitted_session=sender_session.clone();admitted_turn=sender_turn.clone();
            let previous:Option<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND json_extract(payload,'$.operation_id')=? ORDER BY sequence LIMIT 1")
                .bind(execution_id).bind(operation_id).fetch_optional(&mut *tx).await?;
            if let Some(previous)=previous {
                let value:serde_json::Value=serde_json::from_str(&previous).map_err(|error|DbError::Init(error.to_string()))?;
                if value.get("content_digest").and_then(serde_json::Value::as_str)!=Some(content_digest)
                    ||value.get("continuation_of_step_id").and_then(serde_json::Value::as_str)!=Some(original_step_id)
                    ||value.pointer("/message_source/agent_session_id").and_then(serde_json::Value::as_str)!=Some(sender_session.as_str())
                    ||value.pointer("/message_source/turn_operation_id").and_then(serde_json::Value::as_str)!=Some(sender_turn.as_str()) {
                    return Err(conflict("child followup command content"));
                }
                let added_step_ids=delegation_added_step_ids(&previous)?;
                let detail=load_execution_detail_tx(&mut tx,user_id,execution_id).await?.ok_or_else(||conflict("source Execution"))?;
                tx.commit().await?;
                return Ok(AppendAgentExecutionStepsFromAttemptResult {detail,added_step_ids});
            }
            delivery_source=owning_source_tx(&mut tx,&row,original_step_id,sender_session,sender_turn).await?;
            scoped_stop::ensure_not_stopped_tx(&mut tx,execution_id,original_step_id).await?;
            scoped_stop::ensure_delegation_not_stopped_tx(&mut tx,execution_id,original_step_id).await?;
            let predecessor:AgentExecutionStepRow=sqlx::query_as("SELECT * FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND superseded_in_revision IS NULL AND status='completed' AND kind='agent'")
                .bind(execution_id).bind(original_step_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("completed child followup target"))?;
            let [next]=params.new_steps.as_slice() else {return Err(conflict("one followup input Step"));};
            if next.kind!=ExecutionStepKind::Agent||next.assigned_participant_id!=predecessor.assigned_participant_id
                ||next.tool_policy.to_string()!=predecessor.tool_policy||next.profile!=predecessor.profile
                ||!params.new_dependencies.is_empty()
                ||nomifun_agent_contracts::digest_bytes(next.spec.as_bytes()).0!=*content_digest {
                return Err(conflict("frozen child followup configuration"));
            }
            let target:(String,String)=sqlx::query_as("SELECT attempt.attempt_id,link.conversation_id FROM agent_execution_attempts attempt JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id AND link.step_id=attempt.step_id AND link.attempt_id=attempt.attempt_id AND link.relation='attempt' JOIN agent_sessions session ON session.agent_session_id=link.conversation_id AND session.state='live' WHERE attempt.execution_id=? AND attempt.step_id=? AND attempt.status='completed' ORDER BY attempt.attempt_no DESC,link.id DESC LIMIT 1")
                .bind(execution_id).bind(original_step_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("completed child Session"))?;
            depth=Some(predecessor.delegation_depth);
            scoped.step_id=Some(original_step_id.clone());
            scoped.attempt_id=Some(target.0.clone());
            serde_json::json!({"change":"source_followup_appended","operation_id":operation_id,"content_digest":content_digest,
                "delegation_source":delivery_source,"added_step_ids":[next.step_id],"continuation_of_step_id":original_step_id,
                "continuation_step_id":next.step_id,"original_attempt_id":target.0,"target_session_id":target.1,
                "sender_session_id":sender_session,"sender_turn_operation_id":sender_turn,"message_source":message_source})
        }
    };
    if row.version!=expected_version {return Err(conflict("source append version"));}
    let AgentExecutionActor::Agent {conversation_id:Some(caller),..}=&event.actor else {return Err(conflict("source append actor"));};
    if caller!=&delivery_source.agent_session_id {return Err(conflict("source append actor"));}
    let admitted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id JOIN agent_sessions session ON session.agent_session_id=turn.session_id WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='running' AND head.active_turn_id=turn.operation_id AND head.status='running' AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_id')=?)")
        .bind(&admitted_session).bind(&admitted_turn).bind(user_id).fetch_one(&mut *tx).await?;
    if !admitted {return Err(conflict("source Turn admission"));}
    if let AgentExecutionSourceAppend::ChildFollowup {message_source,original_step_id,operation_id,content_digest}=source {
        crate::repository::agent_source_reply::verify_message_command_tx(&mut tx,user_id,message_source,execution_id,original_step_id,operation_id,content_digest).await?;
    }
    scoped.payload=payload.to_string();
    let mut detail=append_steps_tx(&mut tx,user_id,execution_id,expected_version,params,&scoped,&new_ids,now).await?;
    if let Some(depth)=depth {
        sqlx::query("UPDATE agent_execution_steps SET delegation_depth=? WHERE execution_id=? AND step_id=?")
            .bind(depth).bind(execution_id).bind(&params.new_steps[0].step_id).execute(&mut *tx).await?;
        detail=load_execution_detail_tx(&mut tx,user_id,execution_id).await?.ok_or_else(||conflict("source Execution"))?;
    }
    let added_step_ids=params.new_steps.iter().map(|step|step.step_id.clone()).collect();
    tx.commit().await?;
    Ok(AppendAgentExecutionStepsFromAttemptResult {detail,added_step_ids})
}

pub(super) async fn target(pool:&SqlitePool,user_id:&str,execution_id:&str,step_id:&str)->Result<Option<crate::AgentExecutionContinuationTarget>,DbError> {
    let target:Option<(String,String,String)>=sqlx::query_as("SELECT json_extract(event.payload,'$.target_session_id'),json_extract(event.payload,'$.continuation_of_step_id'),json_extract(event.payload,'$.original_attempt_id') FROM agent_execution_events event JOIN agent_executions execution ON execution.execution_id=event.execution_id WHERE event.execution_id=? AND execution.user_id=? AND execution.deleted_at IS NULL AND event.event_type='plan_changed' AND json_extract(event.payload,'$.continuation_step_id')=? ORDER BY event.sequence DESC LIMIT 1")
        .bind(execution_id).bind(user_id).bind(step_id).fetch_optional(pool).await?;
    let Some((session,original,attempt))=target else {return Ok(None);};
    let proven:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.conversation_id=? AND link.relation='attempt' AND attempt.status IN('completed','failed','cancelled'))")
        .bind(execution_id).bind(&original).bind(&attempt).bind(&session).fetch_one(pool).await?;
    if !proven {return Err(conflict("original child continuation Session"));}
    Ok(Some(crate::AgentExecutionContinuationTarget {conversation_id:session,original_step_id:original,original_attempt_id:attempt}))
}

pub(super) async fn original_child_binding_tx(tx:&mut Transaction<'_,Sqlite>,user_id:&str,execution_id:&str,
    target:&crate::AgentExecutionContinuationTarget)->Result<nomifun_agent_contracts::AgentHandoffBindingRefV1,DbError> {
    let payload:Option<String>=sqlx::query_scalar("SELECT event.payload FROM agent_execution_events event JOIN agent_executions execution ON execution.execution_id=event.execution_id WHERE event.execution_id=? AND event.step_id=? AND event.attempt_id=? AND execution.user_id=? AND execution.deleted_at IS NULL AND json_type(event.payload,'$.canonical_operation_id')='text' AND json_type(event.payload,'$.terminal_event_id')='text' ORDER BY event.sequence DESC LIMIT 1")
        .bind(execution_id).bind(&target.original_step_id).bind(&target.original_attempt_id).bind(user_id).fetch_optional(&mut **tx).await?;
    let source=if let Some(payload)=payload {
        serde_json::from_str::<serde_json::Value>(&payload).map_err(|error|DbError::Init(error.to_string()))?
    } else {
        let closed:Option<String>=sqlx::query_scalar("SELECT json_extract(payload,'$.closed_turn') FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.original_attempt_id')=? AND json_extract(payload,'$.continuation_of_step_id')=? AND json_type(payload,'$.closed_turn')='object' ORDER BY sequence LIMIT 1")
            .bind(execution_id).bind(&target.original_attempt_id).bind(&target.original_step_id).fetch_optional(&mut **tx).await?;
        serde_json::from_str(&closed.ok_or_else(||conflict("original child canonical admission reference"))?).map_err(|error|DbError::Init(error.to_string()))?
    };
    let operation=source.get("canonical_operation_id").and_then(serde_json::Value::as_str).ok_or_else(||conflict("original child Turn reference"))?;
    let terminal=source.get("terminal_event_id").and_then(serde_json::Value::as_str).ok_or_else(||conflict("original child terminal reference"))?;
    let started:Option<String>=sqlx::query_scalar("SELECT turn.started_event_id FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id JOIN agent_turns turn ON turn.session_id=link.conversation_id JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.conversation_id=? AND link.relation='attempt' AND execution.user_id=? AND execution.deleted_at IS NULL AND turn.operation_id=? AND turn.terminal_event_id=? AND turn.state IN('completed','failed','cancelled') AND started.kind='turn/started' AND started.kind_version=1 AND started.correlation_id=turn.operation_id AND terminal.kind_version=1 AND terminal.correlation_id=turn.operation_id AND terminal.seq>started.seq")
        .bind(execution_id).bind(&target.original_step_id).bind(&target.original_attempt_id).bind(&target.conversation_id).bind(user_id)
        .bind(operation).bind(terminal).fetch_optional(&mut **tx).await?;
    let started=started.ok_or_else(||conflict("original child exact Turn admission"))?;
    let payload=crate::repository::agent_source_reply::event_payload_tx(tx,&target.conversation_id,&started).await?;
    crate::repository::agent_source_reply::binding(&payload)?.ok_or_else(||conflict("original child frozen binding"))
}

pub(super) async fn verify_child_binding_tx(tx:&mut Transaction<'_,Sqlite>,user_id:&str,execution_id:&str,step_id:&str,session:&str)
    ->Result<(),DbError> {
    let target:Option<(String,String,String)>=sqlx::query_as("SELECT json_extract(payload,'$.target_session_id'),json_extract(payload,'$.continuation_of_step_id'),json_extract(payload,'$.original_attempt_id') FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND json_extract(payload,'$.continuation_step_id')=? ORDER BY sequence LIMIT 1")
        .bind(execution_id).bind(step_id).fetch_optional(&mut **tx).await?;
    let Some((conversation_id,original_step_id,original_attempt_id))=target else {return Ok(());};
    if conversation_id!=session {return Err(conflict("continued child Session"));}
    let target=crate::AgentExecutionContinuationTarget {conversation_id,original_step_id,original_attempt_id};
    let expected=original_child_binding_tx(tx,user_id,execution_id,&target).await?;
    let raw:String=sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=? AND state='live' AND json_extract(owner_ref_json,'$.principal_id')=?")
        .bind(session).bind(user_id).fetch_one(&mut **tx).await?;
    let binding:nomifun_agent_contracts::AgentBindingValue=serde_json::from_str(&raw).map_err(|error|DbError::Init(error.to_string()))?;
    if nomifun_agent_contracts::AgentHandoffBindingRefV1::from(&binding)!=expected {return Err(conflict("original child frozen binding changed"));}
    Ok(())
}

pub(super) async fn step_source(pool:&SqlitePool,user_id:&str,execution_id:&str,step_id:&str)
    ->Result<Option<nomifun_common::AgentExecutionSource>,DbError> {
    let mut tx=pool.begin().await?;
    let source=step_source_tx(&mut tx,user_id,execution_id,step_id).await?;
    tx.commit().await?;Ok(source)
}

async fn step_source_tx(tx:&mut Transaction<'_,Sqlite>,user_id:&str,execution_id:&str,step_id:&str)
    ->Result<Option<nomifun_common::AgentExecutionSource>,DbError> {
    let origin:Option<(String,i64,i64)>=sqlx::query_as("SELECT execution.initial_plan_input,step.introduced_in_revision,(SELECT MIN(root.introduced_in_revision) FROM agent_execution_steps root WHERE root.execution_id=execution.execution_id) FROM agent_execution_steps step JOIN agent_executions execution ON execution.execution_id=step.execution_id WHERE step.execution_id=? AND step.step_id=? AND execution.user_id=? AND execution.deleted_at IS NULL")
        .bind(execution_id).bind(step_id).bind(user_id).fetch_optional(&mut **tx).await?;
    let Some((initial,introduced,first))=origin else {return Err(conflict("Step source target"));};
    let appended:Option<String>=sqlx::query_scalar("SELECT event.payload FROM agent_execution_events event WHERE event.execution_id=? AND event.event_type='plan_changed' AND EXISTS(SELECT 1 FROM json_each(event.payload,'$.added_step_ids') child WHERE child.value=?) ORDER BY event.sequence LIMIT 1")
        .bind(execution_id).bind(step_id).fetch_optional(&mut **tx).await?;
    let value=if let Some(payload)=appended {
        let value:serde_json::Value=serde_json::from_str(&payload).map_err(|error|DbError::Init(error.to_string()))?;
        if value.get("delegation_source").is_none()&&matches!(value.get("change").and_then(serde_json::Value::as_str),Some("source_steps_appended"|"delegated_steps_appended"|"source_followup_appended")) {
            return Err(conflict("introduced delegation source"));
        }
        value.get("delegation_source").cloned()
    } else if introduced==first {
        let value:serde_json::Value=serde_json::from_str(&initial).map_err(|error|DbError::Init(error.to_string()))?;
        if matches!(value.get("mode").and_then(serde_json::Value::as_str),Some("explicit"|"automatic")) {
            value.get("source").cloned()
        } else {None}
    } else {None};
    let source=value.filter(|value|!value.is_null()).map(serde_json::from_value)
        .transpose().map_err(|error|DbError::Init(format!("invalid exact Step source: {error}")))?;
    Ok(source)
}

pub(super) async fn redirect_closed_steer(pool:&SqlitePool,user_id:&str,execution_id:&str,step_id:&str,attempt_id:&str,
    operation_id:&str,closed:&crate::AgentExecutionClosedTurn)->Result<(),DbError> {
    let mut tx=pool.begin().await?;
    let row:AgentExecutionRow=sqlx::query_as("UPDATE agent_executions SET version=version WHERE execution_id=? AND user_id=? AND deleted_at IS NULL RETURNING *")
        .bind(execution_id).bind(user_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("closed guidance Execution"))?;
    let previous:Option<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND json_extract(payload,'$.operation_id')=? AND json_type(payload,'$.redirected_from_sequence')='integer' ORDER BY sequence LIMIT 1")
        .bind(execution_id).bind(operation_id).fetch_optional(&mut *tx).await?;
    if let Some(previous)=previous {
        let value:serde_json::Value=serde_json::from_str(&previous).map_err(|error|DbError::Init(error.to_string()))?;
        if value.pointer("/closed_turn/conversation_id").and_then(serde_json::Value::as_str)!=Some(closed.conversation_id.as_str())
            ||value.pointer("/closed_turn/canonical_operation_id").and_then(serde_json::Value::as_str)!=Some(closed.canonical_operation_id.as_str())
            ||value.pointer("/closed_turn/terminal_event_id").and_then(serde_json::Value::as_str)!=Some(closed.terminal_event_id.as_str())
            ||value.get("continuation_of_step_id").and_then(serde_json::Value::as_str)!=Some(step_id) {return Err(conflict("redirected guidance receipt"));}
        tx.commit().await?;return Ok(());
    }
    scoped_stop::ensure_not_stopped_tx(&mut tx,execution_id,step_id).await?;
    scoped_stop::ensure_delegation_not_stopped_tx(&mut tx,execution_id,step_id).await?;
    let step:AgentExecutionStepRow=sqlx::query_as("SELECT * FROM agent_execution_steps WHERE execution_id=? AND step_id=? AND kind='agent' AND status='running' AND superseded_in_revision IS NULL")
        .bind(execution_id).bind(step_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("closed guidance Step"))?;
    let attempt:AgentExecutionAttemptRow=sqlx::query_as("SELECT * FROM agent_execution_attempts WHERE execution_id=? AND step_id=? AND attempt_id=? AND status='running' AND attempt_no=(SELECT MAX(attempt_no) FROM agent_execution_attempts WHERE execution_id=? AND step_id=?)")
        .bind(execution_id).bind(step_id).bind(attempt_id).bind(execution_id).bind(step_id).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("closed guidance Attempt"))?;
    let mut effects=AttemptConversationEffects::decode(attempt.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
    let Some(crate::PendingConversationEffect::Steer {operation_id:pending,target_operation_id,content})=effects.pending_conversation_effects.first().cloned()
        else {return Err(conflict("original pending guidance"));};
    if pending!=operation_id||effects.review_blocked.is_some()
        ||closed.canonical_operation_id!=target_operation_id {
        return Err(conflict("original guidance invocation"));
    }
    let native:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link JOIN agent_sessions session ON session.agent_session_id=link.conversation_id JOIN agent_turns turn ON turn.session_id=link.conversation_id JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.conversation_id=? AND link.relation='attempt' AND link.active=1 AND turn.operation_id=? AND turn.terminal_event_id=? AND terminal.correlation_id=turn.operation_id AND terminal.kind_version=1 AND terminal.producer_id='runtime_supervisor' AND started.kind='turn/started' AND started.kind_version=1 AND started.correlation_id=turn.operation_id AND terminal.seq>started.seq AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_id')=? AND ((turn.state='completed' AND terminal.kind='turn/completed') OR (turn.state='failed' AND terminal.kind='turn/failed') OR (turn.state='cancelled' AND terminal.kind='turn/cancelled')))")
        .bind(execution_id).bind(step_id).bind(attempt_id).bind(&closed.conversation_id).bind(&closed.canonical_operation_id).bind(&closed.terminal_event_id).bind(user_id).fetch_one(&mut *tx).await?;
    if !native {return Err(conflict("exact closed native guidance Turn"));}
    let command:Option<(i64,String,String,Option<String>)>=sqlx::query_as("SELECT sequence,payload,actor_type,actor_id FROM agent_execution_events WHERE execution_id=? AND step_id=? AND attempt_id=? AND event_type='step_changed' AND json_extract(payload,'$.operation_id')=? ORDER BY sequence LIMIT 1")
        .bind(execution_id).bind(step_id).bind(attempt_id).bind(operation_id).fetch_optional(&mut *tx).await?;
    let (sequence,raw,actor,actor_id)=command.ok_or_else(||conflict("persisted guidance source"))?;
    let command:serde_json::Value=serde_json::from_str(&raw).map_err(|error|DbError::Init(error.to_string()))?;
    let digest=nomifun_agent_contracts::digest_bytes(content.as_bytes()).0;
    let message_source=if actor=="agent" {
        if command.get("content_digest").and_then(serde_json::Value::as_str)!=Some(digest.as_str()) {return Err(conflict("persisted Agent guidance digest"));}
        let source:crate::AgentExecutionMessageSource=serde_json::from_value(command.get("message_source").cloned().ok_or_else(||conflict("persisted Agent guidance source"))?)
            .map_err(|error|DbError::Init(error.to_string()))?;
        if actor_id.as_deref()!=Some(source.agent_session_id.as_str()) {return Err(conflict("persisted guidance actor"));}
        Some(source)
    } else if actor=="user"&&actor_id.as_deref()==Some(user_id) {None}
        else {return Err(conflict("persisted guidance authority"));};
    let next=NewAgentExecutionStep {step_id:nomifun_common::generate_id(),title:step.title.clone(),spec:content,role:step.role.clone(),
        tool_policy:step.tool_policy.parse().map_err(|error|DbError::Init(format!("invalid original child tool scope: {error}")))?,
        kind:ExecutionStepKind::Agent,agent_mode:Some(nomifun_common::AgentStepMode::Normal),profile:step.profile.clone(),fanout_group:None,
        control_policy:None,status:ExecutionStepStatus::Pending,assigned_participant_id:step.assigned_participant_id.clone(),assignment_score:step.assignment_score,
        assignment_rationale:Some("accepted guidance continued after its exact Turn closed".into()),
        assignment_source:step.assignment_source.as_ref().map(|value|value.parse()).transpose().map_err(|error|DbError::Init(format!("invalid original assignment: {error}")))?,
        assignment_locked:true,failure_policy:step.failure_policy.parse().map_err(|error|DbError::Init(format!("invalid original failure policy: {error}")))?,
        preset_prompt:step.preset_prompt.clone(),graph_x:None,graph_y:None};
    let source=step_source_tx(&mut tx,user_id,execution_id,step_id).await?;
    let event=NewAgentExecutionEvent {event_type:AgentExecutionEventKind::PlanChanged,step_id:Some(step_id.into()),attempt_id:Some(attempt_id.into()),
        actor:AgentExecutionActor::system(),payload:serde_json::json!({"change":"source_followup_appended","operation_id":operation_id,
            "content_digest":digest,"delegation_source":source,"added_step_ids":[next.step_id],"continuation_of_step_id":step_id,
            "continuation_step_id":next.step_id,"original_attempt_id":attempt_id,"target_session_id":closed.conversation_id,
            "message_source":message_source,"sender_session_id":message_source.as_ref().map(|value|&value.agent_session_id),
            "sender_turn_operation_id":message_source.as_ref().map(|value|&value.turn_operation_id),"redirected_from_sequence":sequence,
            "closed_turn":{"conversation_id":closed.conversation_id,"canonical_operation_id":closed.canonical_operation_id,"terminal_event_id":closed.terminal_event_id}}).to_string()};
    let ids=HashSet::from([next.step_id.clone()]);let next_id=next.step_id.clone();
    append_steps_tx(&mut tx,user_id,execution_id,row.version,&AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},&event,&ids,now_ms()).await?;
    sqlx::query("UPDATE agent_execution_steps SET delegation_depth=? WHERE execution_id=? AND step_id=?")
        .bind(step.delegation_depth).bind(execution_id).bind(next_id).execute(&mut *tx).await?;
    effects.pending_conversation_effects.remove(0);
    let state=if effects.pending_conversation_effects.is_empty() {None} else {Some(effects.encode().map_err(|error|DbError::Init(error.to_string()))?)};
    sqlx::query("UPDATE agent_execution_attempts SET runtime_state=?,version=version+1,updated_at=? WHERE execution_id=? AND attempt_id=? AND version=?")
        .bind(state).bind(now_ms()).bind(execution_id).bind(attempt_id).bind(attempt.version).execute(&mut *tx).await?;
    tx.commit().await?;Ok(())
}
