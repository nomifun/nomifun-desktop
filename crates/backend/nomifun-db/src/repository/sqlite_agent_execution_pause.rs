//! Same-writer reduction of an original native decision boundary. This is an
//! observation of Session facts, never another pause, execution or ledger.
use super::*;
use nomifun_agent_contracts::{AgentWaitReason, NativePauseState};
use serde_json::{Value, json};

pub(super) async fn technical_boundary_tx(tx:&mut Transaction<'_,Sqlite>,session:&str,operation:&str)
    ->Result<Option<RecoveryReviewBlock>,DbError> {
    type Row=(String,Option<String>,i64,i64,i64,Option<String>,String,Option<String>,i64);
    let row:Option<Row>=sqlx::query_as("SELECT turn.started_event_id,turn.native_pause_json,turn.execution_fence,turn.native_pause_revision,turn.native_checkpoint_revision,turn.native_checkpoint_digest,head.status,head.active_turn_id,head.last_seq FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='running' AND turn.terminal_event_id IS NULL")
        .bind(session).bind(operation).fetch_optional(&mut **tx).await?;
    let Some((start,raw,fence,revision,checkpoint,digest,head,active,last))=row else {return Ok(None);};
    if active.as_deref()!=Some(operation) {return Ok(None);}
    if let Some(raw)=raw {
        let pause:NativePauseState=serde_json::from_str(&raw).map_err(|error|DbError::Init(format!("invalid original native pause: {error}")))?;
        if head!="paused" || u64::try_from(fence).ok()!=Some(pause.execution_fence)
            || u64::try_from(revision).ok().filter(|revision|*revision>0)!=Some(pause.revision)
            || u64::try_from(checkpoint).ok()!=Some(pause.checkpoint_revision)
            || pause.checkpoint_digest.as_ref().map(|digest|digest.as_ref())!=digest.as_deref() {
            return Err(conflict("original native pause identity"));
        }
        let id=format!("native-paused:{session}:{operation}:{}",pause.revision);
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events pause JOIN agent_events started ON started.session_id=pause.session_id AND started.event_id=? WHERE pause.session_id=? AND pause.event_id=? AND pause.kind='turn/paused' AND pause.kind_version=1 AND pause.producer_id='runtime_supervisor' AND pause.correlation_id=? AND pause.causation_event_id=started.event_id AND pause.seq>started.seq AND pause.seq<=? AND started.kind='turn/started' AND started.kind_version=1 AND started.correlation_id=?)")
            .bind(&start).bind(session).bind(&id).bind(operation).bind(last).bind(operation).fetch_one(&mut **tx).await?;
        if !valid || crate::repository::agent_source_reply::event_payload_tx(tx,session,&id).await?.get("pause")
            !=Some(&serde_json::to_value(&pause).map_err(|error|DbError::Init(error.to_string()))?) {
            return Err(conflict("original canonical pause event"));
        }
        if matches!(pause.reason.as_str(),"EXECUTION_USER_REQUESTED"|"USER_REQUESTED") {return Ok(None);}
        return Ok(Some(RecoveryReviewBlock {kind:"native_pause".into(),operation_id:operation.into(),receipt_state:"paused".into(),reason:pause.reason}));
    }
    if head=="reconciliation" {
        let uncertain:Option<String>=sqlx::query_scalar("SELECT event.event_id FROM agent_effects effect JOIN agent_events event ON event.session_id=effect.session_id AND event.event_id=effect.terminal_event_id WHERE effect.session_id=? AND effect.turn_id=? AND effect.state='unknown' AND event.kind='effect/uncertain' AND event.kind_version=1 AND event.seq<=? ORDER BY event.seq DESC LIMIT 1")
            .bind(session).bind(operation).bind(last).fetch_optional(&mut **tx).await?;
        let id=uncertain.ok_or_else(||conflict("original reconciliation Effect source"))?;
        let payload=crate::repository::agent_source_reply::event_payload_tx(tx,session,&id).await?;
        if payload.get("turn_id").and_then(Value::as_str)!=Some(operation)
            ||payload.get("recovery").and_then(Value::as_str)==Some("native_owner_verified_reconciliation") {
            return Err(conflict("original unresolved Effect boundary"));
        }
        return Ok(Some(RecoveryReviewBlock {kind:"native_pause".into(),operation_id:operation.into(),receipt_state:"reconciliation".into(),reason:"NATIVE_EFFECT_RECONCILIATION_REQUIRED".into()}));
    }
    if head=="paused" {return Err(conflict("paused original Turn has no canonical native pause"));}
    let boundary:Option<(String,i64)>=sqlx::query_as("SELECT event.event_id,event.seq FROM agent_events event LEFT JOIN agent_payloads payload ON payload.session_id=event.session_id AND payload.payload_id=event.payload_id JOIN agent_events started ON started.session_id=event.session_id AND started.event_id=? WHERE event.session_id=? AND event.correlation_id=? AND event.kind='runtime/progress-recorded' AND event.kind_version=1 AND event.producer_id='runtime_supervisor' AND event.seq>started.seq AND event.seq<=? AND (event.causation_event_id=started.event_id OR event.causation_event_id=started.causation_event_id) AND (COALESCE(json_extract(event.inline_json,'$.event.event'),json_extract(CAST(payload.body AS TEXT),'$.value.event.event')) IN('turn_waiting','awaiting_children','children_wait_resolved','turn_wait_resolved','model_step_started','tool_started') OR (COALESCE(json_extract(event.inline_json,'$.event.event'),json_extract(CAST(payload.body AS TEXT),'$.value.event.event'))='steering_inputs' AND COALESCE(json_array_length(event.inline_json,'$.event.inputs'),json_array_length(CAST(payload.body AS TEXT),'$.value.event.inputs'),0)>0)) ORDER BY event.seq DESC LIMIT 1")
        .bind(&start).bind(session).bind(operation).bind(last).fetch_optional(&mut **tx).await?;
    let Some((id,_))=boundary else {return Ok(None);};
    let payload=crate::repository::agent_source_reply::event_payload_tx(tx,session,&id).await?;
    if payload.pointer("/event/event").and_then(Value::as_str)!=Some("turn_waiting") {return Ok(None);}
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wait {event:String,model_steps:u16,request_id:Option<String>,reason:AgentWaitReason}
    let wait:Wait=serde_json::from_value(payload["event"].clone()).map_err(|error|DbError::Init(format!("invalid original wait boundary: {error}")))?;
    if wait.event!="turn_waiting"||wait.model_steps==0 {return Err(conflict("original typed wait boundary"));}
    if wait.reason!=AgentWaitReason::InterventionRequired {return Ok(None);}
    if wait.request_id.is_some() {return Err(conflict("intervention wait cannot borrow a user question"));}
    Ok(Some(RecoveryReviewBlock {kind:"native_pause".into(),operation_id:operation.into(),receipt_state:"waiting".into(),reason:"EXECUTION_INTERVENTION_REQUIRED".into()}))
}

pub(super) async fn retain_review_tx(tx:&mut Transaction<'_,Sqlite>,execution:&str,step:&str,step_version:i64,
    attempt:&str,attempt_version:i64,effects:&AttemptConversationEffects,review:RecoveryReviewBlock,now:i64)->Result<(),DbError> {
    if effects.review_blocked.as_ref()==Some(&review) {return Ok(());}
    let mut retained=effects.clone();let reason=review.reason.clone();retained.review_blocked=Some(review);
    let changed=sqlx::query("UPDATE agent_execution_attempts SET status='waiting_input',question=NULL,error=?,runtime_state=?,retry_after=NULL,finished_at=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND attempt_id=? AND version=? AND status IN('running','waiting_input')")
        .bind(&reason).bind(retained.encode().map_err(|error|DbError::Init(error.to_string()))?).bind(now).bind(execution).bind(step).bind(attempt).bind(attempt_version).execute(&mut **tx).await?;
    if changed.rows_affected()!=1 {return Err(conflict("original native review Attempt"));}
    let changed=sqlx::query("UPDATE agent_execution_steps SET status='waiting_input',dispatch_after=NULL,version=version+1,updated_at=? WHERE execution_id=? AND step_id=? AND version=? AND status IN('running','waiting_input') AND superseded_in_revision IS NULL")
        .bind(now).bind(execution).bind(step).bind(step_version).execute(&mut **tx).await?;
    if changed.rows_affected()!=1 {return Err(conflict("original native review Step"));}
    Ok(())
}

/// InitialInput acknowledges native admission, not the final business result.
/// Every byte and both original immutable intake facts are verified under the
/// caller's writer before only this queue head may be removed.
pub(super) async fn initial_admission_tx(tx:&mut Transaction<'_,Sqlite>,owner:&str,session:&str,attempt:&str,request:&Value)
    ->Result<Option<(String,String,String)>,DbError> {
    let key=format!("{attempt}:initial-turn");let identity=format!("user:{owner}:{session}:{key}");let operation=format!("turn:{identity}");
    let row:Option<(String,String)>=sqlx::query_as("SELECT source_message_id,started_event_id FROM agent_turns WHERE session_id=? AND operation_id=? AND turn_id=operation_id")
        .bind(session).bind(&operation).fetch_optional(&mut **tx).await?;
    let Some((accepted,started))=row else {return Ok(None);};
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events accepted JOIN agent_events started ON started.session_id=accepted.session_id WHERE accepted.session_id=? AND accepted.event_id=? AND accepted.kind='message/user-accepted' AND accepted.kind_version=1 AND accepted.producer_id='session_api' AND accepted.idempotency_key=? AND accepted.correlation_id=accepted.event_id AND accepted.causation_event_id IS NOT NULL AND started.event_id=? AND started.kind='turn/started' AND started.kind_version=1 AND started.producer_id='session_api' AND started.idempotency_key=? AND started.correlation_id=? AND started.causation_event_id=accepted.event_id AND started.seq>accepted.seq)")
        .bind(session).bind(&accepted).bind(format!("{identity}:message")).bind(&started).bind(format!("{identity}:turn")).bind(&operation).fetch_one(&mut **tx).await?;
    if !valid {return Err(conflict("original InitialInput canonical admission pair"));}
    let body=crate::repository::agent_source_reply::event_payload_tx(tx,session,&accepted).await?;
    let actual=json!({"content":body["content"],"files":body["files"],"inject_skills":body["inject_skills"],"hidden":body["hidden"],"origin":body["origin"],"channel_platform":body["channel_platform"]});
    let start=crate::repository::agent_source_reply::event_payload_tx(tx,session,&started).await?;
    if &actual!=request || start.get("operation_id").and_then(Value::as_str)!=Some(operation.as_str())
        ||start.get("source_message_id").and_then(Value::as_str)!=Some(accepted.as_str())
        ||start.get("admission")!=body.get("admission") {
        return Err(conflict("original InitialInput exact accepted bytes"));
    }
    Ok(Some((operation,accepted,started)))
}
