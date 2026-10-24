//! Exact logical-task controls use the existing command events and DAG.
use super::*;
use nomifun_agent_contracts::{AgentCollaborationMessage,AgentCollaborationMessageIntent,AgentCollaborationTaskAction,AgentCollaborationTaskControl};

pub(super) async fn authorize_owned_step_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,step:&str,
    event:&NewAgentExecutionEvent)->Result<(),DbError> {
    let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let source:crate::AgentExecutionMessageSource=serde_json::from_value(payload.get("message_source").cloned()
        .ok_or_else(||conflict("parent task control source"))?).map_err(|error|DbError::Conflict(error.to_string()))?;
    let row:AgentExecutionRow=sqlx::query_as("SELECT * FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
        .bind(execution).bind(user).fetch_optional(&mut **tx).await?.ok_or_else(||conflict("parent task Execution"))?;
    source_continuation::owning_source_tx(tx,&row,step,&source.agent_session_id,&source.turn_operation_id).await?;
    Ok(())
}

pub(super) async fn authorize_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,step:&str,
    event:&NewAgentExecutionEvent,action:&str,expected_step_version:i64)->Result<Option<AgentCollaborationTaskControl>,DbError> {
    let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let Some(source)=payload.get("message_source").filter(|source|!source.is_null()) else {
        // Existing user controls and scheduler policy retries retain their
        // own authority. Native controls require an exact original proposal.
        // Hosted management Actions retain their existing actor proof at
        // append_event_tx; this path grants no native message authority.
        return Ok(None);
    };
    let source:crate::AgentExecutionMessageSource=serde_json::from_value(source.clone()).map_err(|error|DbError::Conflict(error.to_string()))?;
    if !matches!(&event.actor,AgentExecutionActor::Agent {conversation_id:Some(session),..} if session==&source.agent_session_id) {
        return Err(conflict("parent task control actor"));
    }
    let operation=payload.get("operation_id").and_then(serde_json::Value::as_str).ok_or_else(||conflict("task command operation"))?;
    let digest=payload.get("content_digest").and_then(serde_json::Value::as_str).ok_or_else(||conflict("task command digest"))?;
    crate::repository::agent_source_reply::verify_message_source_tx(tx,user,&source).await?;
    let (_,arguments)=crate::repository::agent_source_reply::agent_message_command_bound_tx(tx,&source,execution,step,operation,digest).await?;
    let command:AgentCollaborationMessage=serde_json::from_value(arguments.0).map_err(|error|DbError::Init(error.to_string()))?;
    let identity=serde_json::json!({"kind":"native_task_control","digest":nomifun_agent_contracts::digest_payload(&command).map_err(|error|DbError::Init(error.to_string()))?});
    if payload.get("command_request")!=Some(&identity) {
        return Err(conflict("task control differs from its complete original native command"));
    }
    let AgentCollaborationMessageIntent::TaskControl {control}=command.intent else {return Err(conflict("parent task control intent"));};
    let kind=match &control.action {AgentCollaborationTaskAction::Restart {..}=>"restart",AgentCollaborationTaskAction::Retry {}=>"retry",AgentCollaborationTaskAction::Recheck {}=>"recheck",
        AgentCollaborationTaskAction::Stop {}=>"stop",AgentCollaborationTaskAction::ReplaceWithExisting {..}=>"replace_with_existing"};
    if kind!=action||control.expected_step_version!=expected_step_version {return Err(conflict("parent task exact action and version"));}
    authorize_owned_step_tx(tx,user,execution,step,event).await?;
    Ok(Some(control))
}

pub(super) async fn replay_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,event:&NewAgentExecutionEvent)
    ->Result<Option<AgentExecutionDetailRows>,DbError> {
    let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let Some(operation)=payload.get("operation_id").and_then(serde_json::Value::as_str) else {return Ok(None);};
    let request=payload.get("command_request").ok_or_else(||conflict("original task command identity"))?;
    let previous:Option<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.operation_id')=? ORDER BY sequence LIMIT 1")
        .bind(execution).bind(operation).fetch_optional(&mut **tx).await?;
    let Some(previous)=previous else {return Ok(None);};
    let previous:serde_json::Value=serde_json::from_str(&previous).map_err(|error|DbError::Init(error.to_string()))?;
    if previous.get("command_request")!=Some(request)||previous.get("content_digest")!=payload.get("content_digest") {
        return Err(conflict("task command identity reused with different content"));
    }
    load_execution_detail_tx(tx,user,execution).await
}

pub(super) async fn request_reconciliation(pool:&SqlitePool,user:&str,execution:&str,expected_execution_version:i64,
    step:&str,expected_step_version:i64,event:&NewAgentExecutionEvent)->Result<AgentExecutionDetailRows,DbError> {
    let (mut tx,guard)=crate::begin_agent_write(pool).await?;
    require_existing_autowork_admitted_tx(&mut tx,user,execution).await?;
    if let Some(detail)=replay_tx(&mut tx,user,execution,event).await? {guard.commit(tx).await?;return Ok(detail);}
    authorize_tx(&mut tx,user,execution,step,event,"recheck",expected_step_version).await?;
    scoped_stop::ensure_not_stopped_tx(&mut tx,execution,step).await?;
    let current=load_step_detail_tx(&mut tx,user,execution,step).await?.ok_or_else(||conflict("reconciliation task"))?;
    if current.step.version!=expected_step_version||current.current_attempt.is_none() {
        return Err(conflict("reconciliation original Attempt"));
    }
    let now=now_ms();
    bump_execution_version_tx(&mut tx,user,execution,expected_execution_version,now).await?;
    let event=scoped_event(event,step,current.current_attempt.as_ref().map(|current|current.attempt.attempt_id.as_str()));
    append_event_tx(&mut tx,execution,&event,now).await?;
    let detail=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("reconciliation Execution"))?;
    guard.commit(tx).await?;Ok(detail)
}

pub(super) async fn replace_with_existing(pool:&SqlitePool,user:&str,execution:&str,expected_execution_version:i64,
    params:&crate::ReplaceAgentExecutionStepWithExistingParams,event:&NewAgentExecutionEvent)->Result<AgentExecutionDetailRows,DbError> {
    if params.old_step_id==params.replacement_step_id {return Err(conflict("a task cannot replace itself"));}
    let (mut tx,guard)=crate::begin_agent_write(pool).await?;
    require_existing_autowork_admitted_tx(&mut tx,user,execution).await?;
    if let Some(detail)=replay_tx(&mut tx,user,execution,event).await? {guard.commit(tx).await?;return Ok(detail);}
    let control=authorize_tx(&mut tx,user,execution,&params.old_step_id,event,"replace_with_existing",params.expected_old_step_version).await?;
    let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    if payload.get("message_source").is_some() {
        authorize_owned_step_tx(&mut tx,user,execution,&params.replacement_step_id,event).await?;
        if !matches!(control.map(|control|control.action),Some(AgentCollaborationTaskAction::ReplaceWithExisting {replacement_step_id,expected_replacement_step_version})
            if replacement_step_id==params.replacement_step_id&&expected_replacement_step_version==params.expected_replacement_step_version) {
            return Err(conflict("replacement command target"));
        }
    }
    scoped_stop::ensure_not_stopped_tx(&mut tx,execution,&params.old_step_id).await?;
    let current=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("replacement Execution"))?;
    let old=current.steps.iter().find(|step|step.step_id==params.old_step_id&&step.superseded_in_revision.is_none())
        .ok_or_else(||conflict("replacement original task"))?;
    let replacement=current.steps.iter().find(|step|step.step_id==params.replacement_step_id&&step.superseded_in_revision.is_none())
        .ok_or_else(||conflict("replacement completed task"))?;
    if old.version!=params.expected_old_step_version||replacement.version!=params.expected_replacement_step_version
        ||replacement.status!="completed"||current.execution.version!=expected_execution_version {
        return Err(conflict("replacement task versions or result"));
    }
    let uncertain:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link JOIN agent_effects effect ON effect.session_id=link.conversation_id WHERE link.execution_id=? AND link.step_id=? AND link.relation IN('attempt','automation') AND effect.state IN('pending','unknown'))")
        .bind(execution).bind(&params.old_step_id).fetch_one(&mut *tx).await?;
    if uncertain {
        return Err(conflict("original task effects still require reconciliation before its delivery obligation can be replaced"));
    }
    let proof=&params.replacement_terminal;
    let replacement_attempt=current.attempts.iter().filter(|attempt|attempt.attempt.step_id==params.replacement_step_id)
        .max_by_key(|attempt|attempt.attempt.attempt_no).ok_or_else(||conflict("replacement original Attempt"))?;
    let key=crate::execution_attempt_turn_key_tx(&mut tx,user,execution,&params.replacement_step_id,&replacement_attempt.attempt.attempt_id).await?;
    if proof.canonical_operation_id!=format!("turn:user:{user}:{}:{key}",proof.conversation_id) {
        return Err(conflict("replacement must use its original exact Attempt operation"));
    }
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_attempts attempt JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id AND link.step_id=attempt.step_id AND link.attempt_id=attempt.attempt_id JOIN agent_turns turn ON turn.session_id=link.conversation_id JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id WHERE attempt.execution_id=? AND attempt.step_id=? AND attempt.status='completed' AND attempt.attempt_no=(SELECT MAX(candidate.attempt_no) FROM agent_execution_attempts candidate WHERE candidate.execution_id=attempt.execution_id AND candidate.step_id=attempt.step_id) AND link.conversation_id=? AND turn.operation_id=? AND turn.state='completed' AND turn.terminal_event_id=? AND terminal.kind='turn/completed' AND terminal.correlation_id=turn.operation_id)")
        .bind(execution).bind(&params.replacement_step_id).bind(&proof.conversation_id).bind(&proof.canonical_operation_id).bind(&proof.terminal_event_id).fetch_one(&mut *tx).await?;
    if !valid {return Err(conflict("replacement exact successful canonical result"));}
    let mut dependencies=Vec::new();
    let mut seen=HashSet::new();
    for edge in current.dependencies.iter().filter(|edge|edge.superseded_in_revision.is_none()) {
        if edge.blocked_step_id==params.old_step_id {continue;}
        let blocker=if edge.blocker_step_id==params.old_step_id {
            let downstream=current.steps.iter().find(|step|step.step_id==edge.blocked_step_id).ok_or_else(||conflict("replacement downstream"))?;
            if downstream.status!="pending"||current.attempts.iter().any(|attempt|attempt.attempt.step_id==downstream.step_id) {
                return Err(conflict("already-started downstream requires its own explicit plan decision"));
            }
            params.replacement_step_id.clone()
        } else {edge.blocker_step_id.clone()};
        if seen.insert((blocker.clone(),edge.blocked_step_id.clone())) {
            dependencies.push(NewAgentExecutionStepDependency {blocker_step_id:blocker,blocked_step_id:edge.blocked_step_id.clone()});
        }
    }
    let ids=current.steps.iter().filter(|step|step.superseded_in_revision.is_none()&&step.step_id!=params.old_step_id)
        .map(|step|step.step_id.clone()).collect::<HashSet<_>>();
    validate_dependency_graph(&ids,&dependencies)?;
    let now=now_ms();
    let revision:Option<i64>=sqlx::query_scalar("UPDATE agent_executions SET plan_revision=plan_revision+1,version=version+1,status=CASE WHEN status='paused' THEN status ELSE 'running' END,updated_at=? WHERE execution_id=? AND user_id=? AND version=? AND deleted_at IS NULL AND status IN('running','waiting_input','paused') RETURNING plan_revision")
        .bind(now).bind(execution).bind(user).bind(expected_execution_version).fetch_optional(&mut *tx).await?;
    let revision=revision.ok_or_else(||conflict("replacement active Execution"))?;
    sqlx::query("UPDATE agent_execution_step_dependencies SET superseded_in_revision=? WHERE execution_id=? AND superseded_in_revision IS NULL")
        .bind(revision).bind(execution).execute(&mut *tx).await?;
    for edge in dependencies {sqlx::query("INSERT INTO agent_execution_step_dependencies(execution_id,blocker_step_id,blocked_step_id,introduced_in_revision) VALUES(?,?,?,?)")
        .bind(execution).bind(edge.blocker_step_id).bind(edge.blocked_step_id).bind(revision).execute(&mut *tx).await?;}
    // Old effects and cleanup evidence remain intact. Supersession closes the
    // business obligation without pretending the old execution succeeded.
    sqlx::query("UPDATE agent_execution_steps SET status=CASE WHEN status IN('completed','failed','cancelled','skipped') THEN status ELSE 'cancelled' END,superseded_in_revision=?,dispatch_after=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=?")
        .bind(revision).bind(now).bind(execution).bind(&params.old_step_id).bind(params.expected_old_step_version).execute(&mut *tx).await?;
    sqlx::query("UPDATE agent_execution_attempts SET status='interrupted',finished_at=?,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND status IN('queued','running','waiting_input')")
        .bind(now).bind(now).bind(execution).bind(&params.old_step_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE conversation_execution_links SET active=0,updated_at=? WHERE execution_id=? AND step_id=? AND relation IN('attempt','automation')")
        .bind(now).bind(execution).bind(&params.old_step_id).execute(&mut *tx).await?;
    requeue_terminal_attempt_cleanup_tx(&mut tx,execution,&params.old_step_id,now).await?;
    let mut event=event.clone();event.event_type=AgentExecutionEventKind::PlanChanged;event.step_id=Some(params.old_step_id.clone());
    let mut payload=payload;let object=payload.as_object_mut().ok_or_else(||conflict("replacement event"))?;
    object.insert("change".into(),"replaced_with_existing".into());object.insert("plan_revision".into(),revision.into());
    object.insert("superseded_step_ids".into(),serde_json::json!([params.old_step_id]));
    object.insert("replacement_step_id".into(),params.replacement_step_id.clone().into());
    object.insert("canonical_operation_id".into(),proof.canonical_operation_id.clone().into());
    object.insert("terminal_event_id".into(),proof.terminal_event_id.clone().into());event.payload=payload.to_string();
    append_event_tx(&mut tx,execution,&event,now).await?;
    let detail=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("replacement Execution"))?;
    guard.commit(tx).await?;Ok(detail)
}
