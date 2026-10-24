//! One logical Step restart over original command events and exact native facts.
//! Pending work is derived from requested/resolved events, never another ledger.
use super::*;
use crate::{AgentExecutionRestartCommand,AgentExecutionRestartTarget,AgentExecutionRestartPauseRef,
    AgentExecutionRestartCompletion};
use serde_json::{Value,json};

fn payload(event:&AgentExecutionEventRow)->Result<Value,DbError> {
    serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))
}

fn command(event:AgentExecutionEventRow,replayed:bool)->Result<AgentExecutionRestartCommand,DbError> {
    let value=payload(&event)?;
    let target=value.get("restart_target").cloned().map(serde_json::from_value::<AgentExecutionRestartTarget>)
        .transpose().map_err(|error|DbError::Init(error.to_string()))?;
    if let Some(target)=target.as_ref() {
        if target.execution_id!=event.execution_id||event.step_id.as_deref()!=Some(target.step_id.as_str())
            ||event.attempt_id.as_deref()!=Some(target.attempt_id.as_str())||target.command_sequence!=event.sequence
            ||value["operation_id"].as_str()!=Some(target.command_operation_id.as_str()) {
            return Err(conflict("original Restart target"));
        }
    }
    Ok(AgentExecutionRestartCommand {event,target,replayed})
}

async fn request_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,operation:&str)
    ->Result<Option<AgentExecutionEventRow>,DbError> {
    Ok(sqlx::query_as("SELECT event.* FROM agent_execution_events event JOIN agent_executions execution \
        ON execution.execution_id=event.execution_id WHERE execution.user_id=? AND execution.execution_id=? \
        AND event.event_type='step_changed' AND json_extract(event.payload,'$.change')='restart_requested' \
        AND json_extract(event.payload,'$.operation_id')=? ORDER BY event.sequence LIMIT 1")
        .bind(user).bind(execution).bind(operation).fetch_optional(&mut **tx).await?)
}

async fn resolved_tx(tx:&mut Transaction<'_,Sqlite>,execution:&str,operation:&str)->Result<bool,DbError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_events WHERE execution_id=? \
        AND event_type='step_changed' AND json_extract(payload,'$.change')='restart_resolved' \
        AND json_extract(payload,'$.operation_id')=?)")
        .bind(execution).bind(operation).fetch_one(&mut **tx).await?)
}

pub(super) async fn pending(pool:&SqlitePool,user:&str,execution:&str)->Result<Vec<AgentExecutionRestartCommand>,DbError> {
    let rows:Vec<AgentExecutionEventRow>=sqlx::query_as("SELECT request.* FROM agent_execution_events request \
        JOIN agent_executions execution ON execution.execution_id=request.execution_id \
        WHERE execution.user_id=? AND execution.execution_id=? AND execution.deleted_at IS NULL \
        AND request.event_type='step_changed' AND json_extract(request.payload,'$.change')='restart_requested' \
        AND NOT EXISTS(SELECT 1 FROM agent_execution_events result WHERE result.execution_id=request.execution_id \
          AND result.event_type='step_changed' AND json_extract(result.payload,'$.change')='restart_resolved' \
          AND json_extract(result.payload,'$.operation_id')=json_extract(request.payload,'$.operation_id')) \
        ORDER BY request.sequence")
        .bind(user).bind(execution).fetch_all(pool).await?;
    rows.into_iter().map(|row|command(row,true)).collect()
}

async fn resolve_tx(tx:&mut Transaction<'_,Sqlite>,request:&AgentExecutionEventRow,disposition:&str,code:Option<&str>)
    ->Result<(),DbError> {
    resolve_details_tx(tx,request,disposition,code,None).await
}

async fn resolve_details_tx(tx:&mut Transaction<'_,Sqlite>,request:&AgentExecutionEventRow,disposition:&str,code:Option<&str>,
    native:Option<(&str,u64)>)->Result<(),DbError> {
    let original=payload(request)?;
    let mut value=json!({"change":"restart_resolved","operation_id":original["operation_id"],
        "restart_request_sequence":request.sequence,"disposition":disposition});
    if let Some((event,sequence))=native {
        value["native_resume_event_id"]=event.into();value["native_resume_event_seq"]=sequence.into();
    }
    if let Some(code)=code {
        value["code"]=code.into();
        value["reason"]=match code {
            "USER_PAUSE_REQUIRES_CONTINUE"=>"This original task was paused by its owner; use its explicit Continue control.",
            "AGENT_NATIVE_RESUME_FORBIDDEN"=>"The original native pause requires its authenticated owner to continue it.",
            "RESTART_SUPERSEDED_BY_CONTROL"=>"A later Stop or task replacement superseded this restart.",
            _=>"The original task requires exact execution, effect or cleanup reconciliation before restart.",
        }.into();
    }
    append_event_tx(tx,&request.execution_id,&NewAgentExecutionEvent {event_type:AgentExecutionEventKind::StepChanged,
        step_id:request.step_id.clone(),attempt_id:request.attempt_id.clone(),actor:AgentExecutionActor::system(),
        payload:value.to_string()},now_ms()).await?;
    Ok(())
}

enum ResumeStage {Authorized {event:String,sequence:u64},Queued,Blocked(&'static str)}

async fn resume_event_tx(tx:&mut Transaction<'_,Sqlite>,target:&AgentExecutionRestartTarget,id:&str)
    ->Result<Option<(String,i64,Option<String>,Option<String>)>,DbError> {
    Ok(sqlx::query_as("SELECT kind,seq,causation_event_id,inline_json FROM agent_events WHERE session_id=? AND event_id=? AND correlation_id=? AND kind_version=1 AND producer_id='runtime_supervisor'")
        .bind(&target.conversation_id).bind(id).bind(&target.canonical_operation_id).fetch_optional(&mut **tx).await?)
}

async fn resume_stage_tx(tx:&mut Transaction<'_,Sqlite>,target:&AgentExecutionRestartTarget,
    supplied_event:&str,supplied_sequence:u64)->Result<ResumeStage,DbError> {
    let pause=target.pause.as_ref().ok_or_else(||conflict("Restart original native pause"))?;
    let key=format!("execution-task-restart:{}",target.command_operation_id);
    let digest=nomifun_agent_contracts::digest_bytes(key.as_bytes());
    let authorized=format!("native-resume:{}:{}",target.conversation_id,digest.as_ref());
    let queued=format!("native-resume-request:{}:{}",target.conversation_id,digest.as_ref());
    let supplied=resume_event_tx(tx,target,supplied_event).await?.ok_or_else(||conflict("Restart native resume original receipt"))?;
    if i64::try_from(supplied_sequence).ok()!=Some(supplied.1)
        ||![(authorized.as_str(),"turn/resume-authorized"),(queued.as_str(),"turn/resume-requested")]
            .contains(&(supplied_event,supplied.0.as_str())) {return Err(conflict("Restart native receipt identity"));}
    let cause=format!("native-paused:{}:{}:{}",target.conversation_id,target.canonical_operation_id,pause.revision);
    if supplied.2.as_deref()!=Some(cause.as_str()) {return Err(conflict("Restart native receipt pause source"));}
    let value:Value=serde_json::from_str(supplied.3.as_deref().ok_or_else(||conflict("Restart native receipt payload"))?)
        .map_err(|error|DbError::Init(error.to_string()))?;
    if supplied.0=="turn/resume-requested"&&(value["request"]["expected_pause_revision"].as_u64()!=Some(pause.revision)
        ||value["request"]["expected_checkpoint_revision"].as_u64()!=Some(pause.checkpoint_revision)
        ||value["request"]["expected_checkpoint_digest"].as_str()!=pause.checkpoint_digest.as_deref()
        ||value["request"]["idempotency_key"]!=key||value["request"]["operation_id"]!=target.canonical_operation_id) {
        return Err(conflict("Restart queued receipt differs from its frozen request"));
    }
    let grant=resume_event_tx(tx,target,&authorized).await?;
    if let Some((kind,sequence,parent,raw))=grant {
        let grant:Value=serde_json::from_str(raw.as_deref().ok_or_else(||conflict("Restart native grant payload"))?)
            .map_err(|error|DbError::Init(error.to_string()))?;
        if kind!="turn/resume-authorized"||parent.as_deref()!=Some(cause.as_str())||grant["pause_revision"].as_u64()!=Some(pause.revision)
            ||grant["request_digest"].as_str().is_none()
            ||(supplied.0=="turn/resume-requested"&&grant["request_digest"]!=value["request_digest"]) {
            return Err(conflict("Restart native authorization differs from the original queued pause"));
        }
        return Ok(ResumeStage::Authorized {event:authorized,sequence:u64::try_from(sequence).map_err(|_|conflict("native resume sequence"))?});
    }
    if supplied.0!="turn/resume-requested" {return Err(conflict("Restart authorization receipt disappeared"));}
    let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/resume-blocked' AND kind_version=1 AND producer_id='runtime_supervisor' AND causation_event_id=?)")
        .bind(&target.conversation_id).bind(&target.canonical_operation_id).bind(&queued).fetch_one(&mut **tx).await?;
    if blocked {return Ok(ResumeStage::Blocked("NATIVE_RESUME_REQUEST_BLOCKED"));}
    let current:Option<(String,Option<String>,Option<String>,i64,i64,Option<String>)>=sqlx::query_as(
        "SELECT state,terminal_event_id,native_pause_json,execution_fence,native_checkpoint_revision,native_checkpoint_digest FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(&target.conversation_id).bind(&target.canonical_operation_id).fetch_optional(&mut **tx).await?;
    let still=match current {
        Some((state,None,Some(raw),fence,revision,digest)) if state=="running"=>{
            let actual:nomifun_agent_contracts::NativePauseState=serde_json::from_str(&raw).map_err(|error|DbError::Init(error.to_string()))?;
            actual.revision==pause.revision&&actual.execution_fence==pause.execution_fence
                &&u64::try_from(fence).ok()==Some(pause.execution_fence)
                &&u64::try_from(revision).ok()==Some(pause.checkpoint_revision)&&digest==pause.checkpoint_digest
        },_=>false,
    };
    Ok(if still {ResumeStage::Queued}else{ResumeStage::Blocked("NATIVE_RESUME_REQUEST_SUPERSEDED")})
}

async fn original_pending_advanced_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,step:&str,
    saved:&AgentExecutionStepDetailRow)->Result<bool,DbError> {
    let Some(original)=saved.current_attempt.as_ref() else {return Ok(false);};
    let Some(session)=original.conversation_id.as_ref() else {return Ok(false);};
    let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link JOIN agent_sessions session ON session.agent_session_id=link.conversation_id WHERE link.execution_id=? AND link.step_id=? AND link.attempt_id=? AND link.relation='attempt' AND link.conversation_id=? AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?)")
        .bind(execution).bind(step).bind(&original.attempt.attempt_id).bind(session).bind(user).fetch_one(&mut **tx).await?;
    if !exact {return Ok(false);}
    let operation=format!("turn:user:{user}:{session}:{}:initial-turn",original.attempt.attempt_id);
    let accepted:Option<String>=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=? AND turn_id=operation_id")
        .bind(session).bind(&operation).fetch_optional(&mut **tx).await?.flatten();
    if let Some(accepted)=accepted {
        // Once the write-ahead input has retired, the original accepted bytes
        // and its formal start pair are the authority. A current transcript or
        // another Turn cannot substitute for that exact first admission.
        let body=crate::repository::agent_source_reply::event_payload_tx(tx,session,&accepted).await?;
        let envelope=json!({"content":body["content"],"files":body["files"],"inject_skills":body["inject_skills"],
            "hidden":body["hidden"],"origin":body["origin"],"channel_platform":body["channel_platform"]});
        return Ok(native_pause::initial_admission_tx(tx,user,session,&original.attempt.attempt_id,&envelope).await?.is_some());
    }
    // Linking can win before canonical admission. This is still the same
    // durable input, not a fresh invocation; its original owner/reducer keeps
    // delivery custody after this control observation is acknowledged.
    let effects=AttemptConversationEffects::decode(original.attempt.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
    let linked:bool=sqlx::query_scalar("SELECT COUNT(*)=1 FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation='attempt' AND active=1")
        .bind(execution).bind(step).bind(&original.attempt.attempt_id).fetch_one(&mut **tx).await?;
    Ok(linked&&original.attempt.status=="running"&&saved.step.status=="running"&&effects.review_blocked.is_none()
        &&matches!(effects.pending_conversation_effects.as_slice(),[crate::PendingConversationEffect::InitialInput {..}]))
}

pub(super) async fn request(pool:&SqlitePool,user:&str,execution:&str,step:&str,expected_execution:i64,
    expected_step:i64,expected_attempt:Option<&str>,event:&NewAgentExecutionEvent)->Result<AgentExecutionRestartCommand,DbError> {
    let (mut tx,guard)=crate::begin_agent_write(pool).await?;
    let mut value:Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let operation=value["operation_id"].as_str().filter(|key|!key.is_empty()).ok_or_else(||conflict("Restart command identity"))?.to_owned();
    if let Some(prior)=request_tx(&mut tx,user,execution,&operation).await? {
        let old=payload(&prior)?;
        if old.get("command_request")!=value.get("command_request")||old.get("content_digest")!=value.get("content_digest") {
            return Err(conflict("Restart identity reused with different original command"));
        }
        let result=command(prior,true)?;guard.commit(tx).await?;return Ok(result);
    }
    require_existing_autowork_admitted_tx(&mut tx,user,execution).await?;
    let authorized=task_control::authorize_tx(&mut tx,user,execution,step,event,"restart",expected_step).await?;
    if let Some(control)=authorized {
        if !matches!(control.action,nomifun_agent_contracts::AgentCollaborationTaskAction::Restart {expected_attempt_id}
            if expected_attempt_id.as_deref()==expected_attempt) {return Err(conflict("Restart differs from its bound parent Attempt target"));}
    } else if !matches!(&event.actor,AgentExecutionActor::User {user_id} if user_id==user) {
        return Err(conflict("Restart requires its authenticated user or verified original parent command"));
    }
    scoped_stop::ensure_not_stopped_tx(&mut tx,execution,step).await?;
    let execution_row:AgentExecutionRow=sqlx::query_as("SELECT * FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
        .bind(execution).bind(user).fetch_optional(&mut *tx).await?.ok_or_else(||conflict("Restart Execution"))?;
    if execution_row.version!=expected_execution||matches!(execution_row.status.as_str(),"cancelled"|"paused"|"planning") {
        return Err(conflict("Restart requires the selected non-stopped Execution version"));
    }
    let saved=load_step_detail_tx(&mut tx,user,execution,step).await?.ok_or_else(||conflict("Restart Step"))?;
    if saved.step.version!=expected_step||saved.step.status=="cancelled"||saved.step.kind!="agent"
        ||saved.current_attempt.as_ref().map(|attempt|attempt.attempt.attempt_id.as_str())!=expected_attempt {
        return Err(conflict("Restart original Step and Attempt"));
    }
    if saved.current_attempt.is_none()&&matches!(saved.step.status.as_str(),"running"|"waiting_input") {
        return Err(conflict("Restart cannot infer an original running Attempt from missing facts"));
    }
    let another:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_events request WHERE request.execution_id=? \
        AND request.step_id=? AND request.event_type='step_changed' AND json_extract(request.payload,'$.change')='restart_requested' \
        AND NOT EXISTS(SELECT 1 FROM agent_execution_events result WHERE result.execution_id=request.execution_id \
          AND result.event_type='step_changed' AND json_extract(result.payload,'$.change')='restart_resolved' \
          AND json_extract(result.payload,'$.operation_id')=json_extract(request.payload,'$.operation_id')))")
        .bind(execution).bind(step).fetch_one(&mut *tx).await?;
    if another {return Err(conflict("this Step already has its exact pending Restart"));}
    let sequence=execution_row.event_sequence+1;
    let mut target=None;let mut blocked=None;
    if let Some(original)=saved.current_attempt.as_ref() {
        if let Some(session)=original.conversation_id.as_ref() {
            let own_session:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE agent_session_id=? AND state='live' AND json_extract(owner_ref_json,'$.principal_kind')='user' AND json_extract(owner_ref_json,'$.principal_id')=?)")
                .bind(session).bind(user).fetch_one(&mut *tx).await?;
            if !own_session {return Err(conflict("Restart original Session owner"));}
            let key=crate::execution_attempt_turn_key_tx(&mut tx,user,execution,step,&original.attempt.attempt_id).await?;
            let operation_id=format!("turn:user:{user}:{session}:{key}");
            let native:Option<(Option<String>,String,Option<String>,i64,i64,i64,Option<String>,String,Option<String>)>=sqlx::query_as(
                "SELECT turn.native_pause_json,turn.state,turn.terminal_event_id,turn.execution_fence,turn.native_pause_revision, \
                 turn.native_checkpoint_revision,turn.native_checkpoint_digest,head.status,head.active_turn_id \
                 FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id \
                 WHERE turn.session_id=? AND turn.operation_id=?")
                .bind(session).bind(&operation_id).fetch_optional(&mut *tx).await?;
            let pause=native.as_ref().and_then(|row|row.0.as_ref()).map(|raw|serde_json::from_str::<nomifun_agent_contracts::NativePauseState>(raw)
                .map_err(|error|DbError::Init(error.to_string()))).transpose()?;
            if let Some(pause)=pause.as_ref() {
                let row=native.as_ref().unwrap();
                if row.1!="running"||row.2.is_some()||row.3<0||row.4<0||row.5<0
                    ||pause.execution_fence!=row.3 as u64||pause.revision!=row.4 as u64
                    ||pause.checkpoint_revision!=row.5 as u64
                    ||pause.checkpoint_digest.as_ref().map(|digest|digest.as_ref())!=row.6.as_deref()
                    ||!matches!(row.7.as_str(),"paused"|"reconciliation"|"waiting_input")||row.8.as_deref()!=Some(operation_id.as_str()) {
                    return Err(conflict("Restart original native pause/head/checkpoint identity"));
                }
                if matches!(pause.reason.as_str(),"EXECUTION_USER_REQUESTED"|"USER_REQUESTED") {blocked=Some("USER_PAUSE_REQUIRES_CONTINUE");}
                else if !matches!(&event.actor,AgentExecutionActor::User {user_id} if user_id==user) {
                    blocked=Some("AGENT_NATIVE_RESUME_FORBIDDEN");
                }
            }
            target=Some(AgentExecutionRestartTarget {execution_id:execution.into(),step_id:step.into(),
                attempt_id:original.attempt.attempt_id.clone(),conversation_id:session.clone(),canonical_operation_id:operation_id,
                command_operation_id:operation.clone(),command_sequence:sequence,
                pause:pause.as_ref().map(|pause|AgentExecutionRestartPauseRef {revision:pause.revision,
                    checkpoint_revision:pause.checkpoint_revision,checkpoint_digest:pause.checkpoint_digest.as_ref().map(|digest|digest.as_ref().into()),
                    execution_fence:pause.execution_fence})});
            let stopping=pause.is_none()&&matches!(original.attempt.status.as_str(),"running"|"waiting_input");
            if blocked.is_none()&&pause.is_some() {
                let mut effects=AttemptConversationEffects::decode(original.attempt.runtime_state.as_deref())
                    .map_err(|error|DbError::Init(error.to_string()))?;
                effects.review_blocked=None;
                sqlx::query("UPDATE agent_execution_attempts SET status='running',question=NULL,runtime_state=?,version=version+1,updated_at=? WHERE attempt_id=? AND version=?")
                    .bind(effects.encode().map_err(|error|DbError::Init(error.to_string()))?).bind(now_ms())
                    .bind(&original.attempt.attempt_id).bind(original.attempt.version).execute(&mut *tx).await?;
                sqlx::query("UPDATE conversation_execution_links SET active=1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation='attempt'")
                    .bind(now_ms()).bind(execution).bind(step).bind(&original.attempt.attempt_id).execute(&mut *tx).await?;
            } else if blocked.is_none()&&stopping {
                sqlx::query("UPDATE agent_execution_attempts SET status='cancelled',finished_at=?,version=version+1,updated_at=? WHERE attempt_id=? AND version=?")
                    .bind(now_ms()).bind(now_ms()).bind(&original.attempt.attempt_id).bind(original.attempt.version).execute(&mut *tx).await?;
                sqlx::query("UPDATE conversation_execution_links SET active=0,cleanup_completed_at=NULL,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND active=1 AND relation='attempt'")
                    .bind(now_ms()).bind(execution).bind(step).bind(&original.attempt.attempt_id).execute(&mut *tx).await?;
            }
        }
    }
    if blocked.is_none() {
        sqlx::query("UPDATE agent_execution_steps SET version=version+1,status=CASE WHEN status='waiting_input' THEN 'running' ELSE status END,updated_at=? WHERE execution_id=? AND step_id=? AND version=?")
            .bind(now_ms()).bind(execution).bind(step).bind(expected_step).execute(&mut *tx).await?;
    }
    bump_execution_version_tx(&mut tx,user,execution,expected_execution,now_ms()).await?;
    if is_terminal_execution_status(&execution_row.status)&&matches!(&event.actor,AgentExecutionActor::Agent {..}) {
        activate_execution_lead_tx(&mut tx,user,execution,now_ms()).await?;
    }
    value["change"]="restart_requested".into();
    if let Some(target)=target {value["restart_target"]=serde_json::to_value(target).map_err(|error|DbError::Init(error.to_string()))?;}
    let mut request=scoped_event(event,step,expected_attempt);request.payload=value.to_string();
    let stored=append_event_tx(&mut tx,execution,&request,now_ms()).await?;
    if let Some(code)=blocked {resolve_tx(&mut tx,&stored,"blocked",Some(code)).await?;}
    let result=command(stored,false)?;guard.commit(tx).await?;Ok(result)
}

pub(super) async fn complete(pool:&SqlitePool,user:&str,execution:&str,operation:&str,
    completion:&AgentExecutionRestartCompletion)->Result<AgentExecutionDetailRows,DbError> {
    let (mut tx,guard)=crate::begin_agent_write(pool).await?;
    let request=request_tx(&mut tx,user,execution,operation).await?.ok_or_else(||conflict("original Restart request"))?;
    let original=command(request.clone(),true)?;
    let step=request.step_id.as_deref().ok_or_else(||conflict("original Restart Step"))?;
    let detail=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("Restart Execution"))?;
    if resolved_tx(&mut tx,execution,operation).await? {guard.commit(tx).await?;return Ok(detail);}
    let saved=load_step_detail_tx(&mut tx,user,execution,step).await?;
    let stopped=match scoped_stop::ensure_not_stopped_tx(&mut tx,execution,step).await {
        Ok(())=>false,Err(DbError::Conflict(_))=>true,Err(error)=>return Err(error),
    };
    let superseded=stopped||detail.execution.status=="cancelled"||saved.as_ref().is_none_or(|saved|
        saved.step.status=="cancelled"||saved.current_attempt.as_ref().map(|attempt|attempt.attempt.attempt_id.as_str())!=request.attempt_id.as_deref());
    if superseded {resolve_tx(&mut tx,&request,"superseded",Some("RESTART_SUPERSEDED_BY_CONTROL")).await?;}
    else if let AgentExecutionRestartCompletion::Blocked {code}=completion {
        resolve_tx(&mut tx,&request,"blocked",Some(code)).await?;
    } else {
        let saved=saved.unwrap();
        let completion=if matches!(completion,AgentExecutionRestartCompletion::OriginalPending)
            &&saved.current_attempt.is_none()&&matches!(saved.step.status.as_str(),"failed"|"skipped") {
            AgentExecutionRestartCompletion::RetryOriginalStep
        } else {completion.clone()};
        match &completion {
            AgentExecutionRestartCompletion::ContinuedOriginal {resume_event_id,resume_event_seq}=>{
                let target=original.target.as_ref().ok_or_else(||conflict("Restart native continuation target"))?;
                if request.actor_type!="user"||request.actor_id.as_deref()!=Some(user) {return Err(conflict("only the original user Restart can authorize native continuation"));}
                let stage=match resume_stage_tx(&mut tx,target,resume_event_id,*resume_event_seq).await {
                    Ok(stage)=>stage,Err(DbError::Conflict(_))=>ResumeStage::Blocked("NATIVE_RESUME_RECEIPT_REQUIRES_RECONCILIATION"),
                    Err(error)=>return Err(error),
                };
                match stage {
                    ResumeStage::Authorized {event,sequence}=>resolve_details_tx(&mut tx,&request,"continued_original",None,Some((&event,sequence))).await?,
                    ResumeStage::Queued=>{},
                    ResumeStage::Blocked(code)=>resolve_tx(&mut tx,&request,"blocked",Some(code)).await?,
                }
            },
            AgentExecutionRestartCompletion::OriginalPending=>{
                if saved.step.status!="pending"||saved.current_attempt.as_ref().is_some_and(|attempt|attempt.attempt.status!="queued") {
                    let advanced=match original_pending_advanced_tx(&mut tx,user,execution,step,&saved).await {
                        Ok(advanced)=>advanced,Err(DbError::Conflict(_))=>false,Err(error)=>return Err(error),
                    };
                    if advanced {resolve_tx(&mut tx,&request,"original_pending",None).await?;}
                    else {resolve_tx(&mut tx,&request,"blocked",Some("ORIGINAL_INPUT_REQUIRES_RECONCILIATION")).await?;}
                } else {
                    let valid=if let Some(attempt)=saved.current_attempt.as_ref() {
                    let effects=AttemptConversationEffects::decode(attempt.attempt.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
                    effects.review_blocked.is_none()&&matches!(effects.pending_conversation_effects.as_slice(),[crate::PendingConversationEffect::InitialInput {..}])
                    } else {true};
                    if valid {resolve_tx(&mut tx,&request,"original_pending",None).await?;}
                    else {resolve_tx(&mut tx,&request,"blocked",Some("ORIGINAL_INPUT_REQUIRES_RECONCILIATION")).await?;}
                }
            },
            AgentExecutionRestartCompletion::RetryOriginalStep=>{
                let unknown:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links link JOIN agent_effects effect ON effect.session_id=link.conversation_id WHERE link.execution_id=? AND link.step_id=? AND link.relation='attempt' AND effect.state IN('pending','unknown'))")
                    .bind(execution).bind(step).fetch_one(&mut *tx).await?;
                if unknown {
                    resolve_tx(&mut tx,&request,"blocked",Some("ORIGINAL_EFFECTS_REQUIRE_RECONCILIATION")).await?;
                    let result=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("Restart final Execution"))?;
                    guard.commit(tx).await?;return Ok(result);
                }
                if let Some(attempt)=saved.current_attempt.as_ref() {
                    if matches!(attempt.attempt.status.as_str(),"queued"|"running"|"waiting_input") {return Err(conflict("original Restart Attempt remains active"));}
                    if let Some(target)=original.target.as_ref() {
                        let live:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=? AND state IN('accepted','running') AND terminal_event_id IS NULL)")
                            .bind(&target.conversation_id).bind(&target.canonical_operation_id).fetch_one(&mut *tx).await?;
                        let pending_cleanup:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation='attempt' AND (active=1 OR cleanup_completed_at IS NULL))")
                            .bind(execution).bind(step).bind(&attempt.attempt.attempt_id).fetch_one(&mut *tx).await?;
                        if live||pending_cleanup {return Err(conflict("Restart requires the exact original native closure and cleanup acknowledgement"));}
                    } else {
                        let effects=AttemptConversationEffects::decode(attempt.attempt.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
                        if effects.review_blocked.is_some()||attempt.attempt.status=="interrupted" {
                            resolve_tx(&mut tx,&request,"blocked",Some("NATIVE_RESTART_REQUIRES_RECONCILIATION")).await?;
                            let result=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("Restart final Execution"))?;
                            guard.commit(tx).await?;return Ok(result);
                        }
                    }
                }
                let descendants:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_execution_step_dependencies dep JOIN agent_execution_steps child ON child.execution_id=dep.execution_id AND child.step_id=dep.blocked_step_id WHERE dep.execution_id=? AND dep.blocker_step_id=? AND dep.superseded_in_revision IS NULL AND child.superseded_in_revision IS NULL AND child.status NOT IN('pending','skipped'))")
                    .bind(execution).bind(step).fetch_one(&mut *tx).await?;
                if descendants {resolve_tx(&mut tx,&request,"blocked",Some("DEPENDENT_TASK_REQUIRES_EXPLICIT_REPLAN")).await?;}
                else {
                    sqlx::query("UPDATE agent_execution_steps SET status='pending',dispatch_after=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=?")
                        .bind(now_ms()).bind(execution).bind(step).bind(saved.step.version).execute(&mut *tx).await?;
                    sqlx::query("UPDATE agent_executions SET status='running',version=version+1,updated_at=? WHERE execution_id=? AND user_id=? AND status<>'cancelled' AND deleted_at IS NULL")
                        .bind(now_ms()).bind(execution).bind(user).execute(&mut *tx).await?;
                    if is_terminal_execution_status(&detail.execution.status) {activate_execution_lead_tx(&mut tx,user,execution,now_ms()).await?;}
                    resolve_tx(&mut tx,&request,"retry_ready",None).await?;
                }
            },
            AgentExecutionRestartCompletion::Blocked {..}=>unreachable!(),
        }
    }
    let result=load_execution_detail_tx(&mut tx,user,execution).await?.ok_or_else(||conflict("Restart final Execution"))?;
    guard.commit(tx).await?;Ok(result)
}

#[cfg(test)]
#[path = "sqlite_agent_execution_restart_tests.rs"]
mod tests;
