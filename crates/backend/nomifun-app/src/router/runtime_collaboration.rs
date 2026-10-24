//! Exact-source collaboration controls on the existing Execution and Session
//! owners. Notifications wake reads; they never supply results or start work.
use super::*;
use nomifun_agent_execution::AgentExecutionSessionPort;
use nomifun_common::ExecutionStepStatus;
use nomifun_db::sqlx;
use serde_json::json;
use std::collections::BTreeMap;

#[path = "collaboration_read_error.rs"]
mod read_error;

#[cfg(test)]
static REJECTED_COMMAND_READ_ERRORS:std::sync::OnceLock<std::sync::Mutex<BTreeMap<String,AgentEngineError>>>=std::sync::OnceLock::new();
#[cfg(test)]
pub(super) fn inject_rejected_command_read_error(session:&str,error:AgentEngineError) {
    REJECTED_COMMAND_READ_ERRORS.get_or_init(Default::default).lock().unwrap().insert(session.into(),error);
}

#[derive(Default)]
struct ObservationCache {
    executions: BTreeMap<String, CachedExecutionObservation>,
}

struct CachedExecutionObservation {
    execution: nomifun_db::AgentExecutionObservationHeader,
    state: nomifun_agent_execution::AgentExecutionCollaborationState,
}

impl ObservationCache {
    async fn read<'a>(&'a mut self, engine:&nomifun_agent_execution::AgentExecutionEngine, owner:&str, id:&str)
        ->Result<&'a CachedExecutionObservation,AgentEngineError> {
        let after=self.executions.get(id).map(|cached|cached.execution.event_sequence);
        let observation=engine.collaboration_observation(owner,id,after).await.map_err(read_error::app)?;
        if let Some(mut state)=observation.state {
            if let Some(previous)=self.executions.remove(id) {
                let mut events=previous.state.events;
                events.append(&mut state.events);
                state.events=events;
            }
            if state.events.last().map(|event|event.sequence)!=Some(observation.execution.event_sequence) {
                return Err(failure("collaboration snapshot is missing its committed event cursor"));
            }
            state.events=compact_observation_proofs(state.events,&state.steps,&observation.execution.status);
            self.executions.insert(id.into(),CachedExecutionObservation {execution:observation.execution,state});
        } else if let Some(cached)=self.executions.get_mut(id) {
            cached.execution=observation.execution;
        }
        self.executions.get(id).ok_or_else(||failure("unchanged observation has no original read snapshot"))
    }

    fn session_ids(&self, source:&AgentSessionId)->Vec<String> {
        let mut sessions=BTreeSet::from([source.as_ref().to_owned()]);
        for cached in self.executions.values() {
            sessions.extend(cached.state.attempts.iter().filter_map(|attempt|attempt.conversation_id.clone()));
        }
        sessions.into_iter().collect()
    }
}

/// Retain references needed by this read, rather than another copy of the
/// domain log. Original native Turn references survive newer status changes.
fn compact_observation_proofs(events:Vec<nomifun_api_types::AgentExecutionEvent>,
    steps:&[nomifun_agent_execution::AgentExecutionCollaborationStep], status:&str)
    ->Vec<nomifun_api_types::AgentExecutionEvent> {
    use nomifun_common::AgentExecutionEventKind;
    let current=steps.iter().filter(|step|step.superseded_in_revision.is_none())
        .map(|step|(step.step_id.as_str(),step.status)).collect::<BTreeMap<_,_>>();
    let mut proofs=BTreeMap::<String,nomifun_api_types::AgentExecutionEvent>::new();
    for mut event in events {
        let mut keys=Vec::new();
        if event.event_type==AgentExecutionEventKind::PlanChanged
            &&event.payload.get("delegation_source").is_some()
            &&event.payload.get("added_step_ids").and_then(Value::as_array)
                .is_some_and(|ids|ids.iter().filter_map(Value::as_str).any(|id|current.contains_key(id))) {
            keys.push(format!("introduction:{}",event.sequence));
        }
        if event.event_type==AgentExecutionEventKind::StatusChanged
            &&event.payload.get("status").and_then(Value::as_str)==Some(status) {
            keys.push("execution-status".into());
        }
        if event.event_type==AgentExecutionEventKind::StatusChanged
            &&matches!(event.payload.get("status").and_then(Value::as_str),Some("cancelled"|"failed"))
            &&current.values().any(|state|*state==ExecutionStepStatus::Cancelled) {
            keys.push("aggregate-cancellation".into());
        }
        if let Some(step)=event.step_id.as_deref().filter(|step|current.contains_key(*step)) {
            let state=current[step];
            if event.payload.get("step_status").or_else(||event.payload.get("status")).and_then(Value::as_str)==Some(state.as_str())
                ||(state==ExecutionStepStatus::Completed&&event.payload.get("change").and_then(Value::as_str)==Some("output_adopted"))
                ||(state==ExecutionStepStatus::WaitingInput&&event.event_type==AgentExecutionEventKind::DecisionRequested) {
                keys.push(format!("settlement:{step}"));
            }
            if event.payload.get("reconciliation").and_then(Value::as_str)==Some("initial_turn_receipt") {
                keys.push(format!("reconciliation:{step}:{}",event.attempt_id.as_deref().unwrap_or("")));
            }
            if matches!(event.payload.get("change").and_then(Value::as_str),
                Some("retry_requested"|"restart_requested"|"restart_resolved"|"recheck_requested"|"replaced_with_existing")) {
                keys.push(format!("task-command:{step}"));
            }
            if event.event_type==AgentExecutionEventKind::StepChanged&&event.payload.get("change").and_then(Value::as_str)==Some("restart_resolved")
                &&event.payload.get("disposition").and_then(Value::as_str)==Some("blocked") {
                keys.push(format!("restart-blocked:{step}:{}",event.attempt_id.as_deref().unwrap_or("")));
            }
            if let Some(attempt)=event.attempt_id.as_deref() {
                for field in ["turn_operation_key","canonical_operation_id"] {
                    if let Some(key)=event.payload.get(field).and_then(Value::as_str) {
                        keys.push(format!("native:{attempt}:{field}:{key}"));
                    }
                }
                if event.event_type==AgentExecutionEventKind::DecisionAnswered
                    &&let Some(key)=event.payload.get("operation_id").and_then(Value::as_str) {
                    keys.push(format!("native:{attempt}:decision:{key}"));
                }
            }
        }
        if event.payload.get("action").and_then(Value::as_str)==Some("cancel_step")
            &&let Some(ids)=event.payload.get("cancelled_step_ids").and_then(Value::as_array) {
            for step in ids.iter().filter_map(Value::as_str).filter(|step|current.get(*step)==Some(&ExecutionStepStatus::Cancelled)) {
                keys.push(format!("cancelled:{step}"));
            }
        }
        if keys.is_empty() {continue;}
        let mut payload=serde_json::Map::new();
        for field in ["delegation_source","added_step_ids","action","cancelled_step_ids","step_status","status","change",
            "canonical_operation_id","terminal_event_id","turn_operation_key","operation_id","reconciliation","disposition","code"] {
            if let Some(value)=event.payload.get(field) {payload.insert(field.into(),value.clone());}
        }
        if event.payload.get("change").and_then(Value::as_str)==Some("restart_resolved")
            &&event.payload.get("disposition").and_then(Value::as_str)==Some("blocked")
            &&let Some(reason)=event.payload.get("reason") {payload.insert("reason".into(),reason.clone());}
        event.payload=Value::Object(payload);
        for key in keys {proofs.insert(key,event.clone());}
    }
    proofs.into_values().map(|event|(event.sequence,event)).collect::<BTreeMap<_,_>>().into_values().collect()
}

pub(super) struct NativeAttemptReference<'a> { attempt_id:&'a str, conversation_id:Option<&'a str> }
impl<'a> From<&'a nomifun_api_types::ExecutionAttempt> for NativeAttemptReference<'a> {
    fn from(attempt:&'a nomifun_api_types::ExecutionAttempt)->Self {
        Self {attempt_id:&attempt.attempt_id,conversation_id:attempt.conversation_id.as_deref()}
    }
}
impl<'a> From<&'a nomifun_agent_execution::AgentExecutionCollaborationAttempt> for NativeAttemptReference<'a> {
    fn from(attempt:&'a nomifun_agent_execution::AgentExecutionCollaborationAttempt)->Self {
        Self {attempt_id:&attempt.attempt_id,conversation_id:attempt.conversation_id.as_deref()}
    }
}

pub(super) struct HostPort(pub(super) std::sync::Weak<ConversationRuntimeHost>);

type ObservationNotifications=Vec<std::pin::Pin<Box<tokio::sync::futures::OwnedNotified>>>;

fn arm_observation_changes(changes:&[Arc<tokio::sync::Notify>])->ObservationNotifications {
    changes.iter().map(|changed| {
        let mut notification=Box::pin(changed.clone().notified_owned());
        notification.as_mut().enable();
        notification
    }).collect()
}

#[derive(Debug,PartialEq,Eq)]
pub(super) enum WaitDisposition { Wake, Park, DecisionBlocked }

fn dispatchable_step(status:&str,decision_input_pending:bool)->bool {
    matches!(status,"pending"|"queued"|"running"|"backoff_pending")
        ||(status=="waiting_input"&&decision_input_pending)
}

fn terminal_step(status:&str)->bool {matches!(status,"completed"|"failed"|"cancelled"|"skipped")}

fn ordinary_dispatchable(head:&Value)->bool {
    !head["native_intervention"].is_object()
        &&head["status"].as_str().is_some_and(|status|dispatchable_step(status,head["decision_input_pending"]==true))
}

fn current_wait_proof<'a>(events:&'a [nomifun_api_types::AgentExecutionEvent],step:&str,attempt:Option<&str>,review:bool)
    ->Option<&'a nomifun_api_types::AgentExecutionEvent> {
    events.iter().rev().find(|event|event.step_id.as_deref()==Some(step)&&event.attempt_id.as_deref()==attempt
        &&if review {event.payload.get("reconciliation").and_then(Value::as_str)==Some("initial_turn_receipt")}
            else {event.event_type==nomifun_common::AgentExecutionEventKind::DecisionRequested})
}

fn blocked_restart_receipt(events:&[nomifun_api_types::AgentExecutionEvent],step:&str,attempt:Option<&str>)->Result<Option<Value>,AgentEngineError> {
    let receipt=events.iter().rev().find(|event|event.event_type==nomifun_common::AgentExecutionEventKind::StepChanged
        &&event.step_id.as_deref()==Some(step)&&event.attempt_id.as_deref()==attempt
        &&event.payload.get("change").and_then(Value::as_str)==Some("restart_resolved")
        &&event.payload.get("disposition").and_then(Value::as_str)==Some("blocked"));
    let Some(event)=receipt else {return Ok(None);};
    if event.sequence<=0 || ["operation_id","code"].iter().any(|field|
        event.payload.get(*field).and_then(Value::as_str).is_none_or(|value|value.is_empty()||value.len()>1024||value.chars().any(char::is_control))) {
        return Err(failure("blocked Restart lacks its exact immutable command result"));
    }
    Ok(Some(json!({"sequence":event.sequence,"operation_id":event.payload["operation_id"],
        "disposition":"blocked","code":event.payload["code"],"reason":event.payload["reason"]})))
}

fn native_pause_ready_digest(attempt:Option<&str>,pause:&Value,blocked:Option<&Value>)
    ->Result<nomifun_agent_contracts::DigestHex,AgentEngineError> {
    nomifun_agent_contracts::digest_payload(&json!({"attempt_id":attempt,"native_intervention":pause,
        "blocked_restart":blocked})).map_err(failure)
}

/// A new accepted fact may return control once. Verified native questions and
/// explicit owner restart boundaries retain their own external control route.
/// This does not turn their passive observer into a business producer.
pub(super) fn wait_disposition(mode:AgentChildrenWaitMode,changed:bool,all_ready:bool,any_ready:bool,
    needs_attention:bool,progress_owned:bool,external_decision_pending:bool)->WaitDisposition {
    if changed && match mode {
        AgentChildrenWaitMode::All=>all_ready||needs_attention,
        AgentChildrenWaitMode::Any=>any_ready||needs_attention,
    } {return WaitDisposition::Wake;}
    if !changed && (all_ready || (needs_attention&&!progress_owned&&!external_decision_pending)) {return WaitDisposition::DecisionBlocked;}
    WaitDisposition::Park
}

/// These heads come from verified native input/owner-control boundaries, never
/// a status label. Keeping this subscription active lets the same parent wake
/// after a child Restart without requiring a second parent resume command.
fn external_decision_wait(rows:&[Value])->Result<bool,AgentEngineError> {
    let mut decision=false;
    for row in rows {
        if row["deleted"]==true {continue;}
        if !row["execution_attention"].is_null() {return Ok(false);}
        let heads=row["step_observations"].as_object().ok_or_else(||failure("external decision wait lacks full Step sources"))?;
        if heads.is_empty()&&!matches!(row["status"].as_str(),Some("completed"|"completed_with_failures"|"failed"|"cancelled")) {return Ok(false);}
        for head in heads.values() {
            if head["status"].as_str().is_some_and(terminal_step) {
                if head["delivery_pending"]==true {return Ok(false);}
                continue;
            }
            if head["native_intervention"].is_object() {
                let control=&head["native_intervention"];
                let boundary=&control["boundary"];
                if head["agent_session_id"].as_str()!=control["agent_session_id"].as_str()
                    ||control["turn_operation_id"].as_str().is_none_or(str::is_empty)
                    ||!matches!(head["status"].as_str(),Some("running"|"waiting_input")) {
                    return Err(failure("native owner control differs from its exact child Turn"));
                }
                if boundary["kind"]!="native_pause"||boundary["cleanup_proven"]!=true
                    ||boundary["revision"].as_u64().is_none_or(|revision|revision==0)
                    ||boundary["checkpoint_revision"].as_u64().is_none_or(|revision|revision==0) {
                    return Ok(false);
                }
                decision=true;
            } else if !head["decision_input_boundary"].is_null() {
                let reference:nomifun_agent_contracts::AgentQuestionReplyReference=serde_json::from_value(head["decision_input_boundary"].clone()).map_err(failure)?;
                if head["status"]!="waiting_input"||head["delivery_pending"]==true
                    ||head["agent_session_id"].as_str()!=Some(reference.agent_session_id.as_ref())
                    ||[reference.agent_session_id.as_ref(),reference.turn_operation_id.as_ref(),reference.question_event_id.as_ref(),reference.request_id.as_ref()]
                        .iter().any(|value|value.is_empty()||value.len()>1024||value.trim()!=*value||value.chars().any(char::is_control)) {
                    return Err(failure("external question boundary differs from its current Step"));
                }
                decision=true;
            } else if !matches!(head["status"].as_str(),Some("completed"|"failed"|"cancelled"|"skipped"))||head["delivery_pending"]==true {
                return Ok(false);
            }
        }
    }
    Ok(decision)
}

async fn wait_for_observation_change(notifications:ObservationNotifications,cancellation:&CancellationToken)
    ->Result<(),AgentEngineError> {
    tokio::select! {
        _=cancellation.cancelled()=>Err(failure("source Turn was cancelled")),
        _=futures_util::future::select_all(notifications)=>Ok(()),
        _=tokio::time::sleep(std::time::Duration::from_secs(30))=>Ok(()),
    }
}

impl std::fmt::Debug for HostPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CanonicalCollaborationPort")
    }
}

fn failure(value: impl std::fmt::Display) -> AgentEngineError {
    AgentEngineError::InvalidContract(format!("Agent collaboration: {value}"))
}

fn source_matches(value: &Value, session: &AgentSessionId, turns: &BTreeSet<OperationId>) -> bool {
    serde_json::from_value::<nomifun_common::AgentExecutionSource>(value.clone()).is_ok_and(|source|
        source.agent_session_id==session.as_ref() && turns.contains(&OperationId::from(source.turn_operation_id))
        && !source.tool_operation_id.trim().is_empty() && !source.effect_id.trim().is_empty())
}

fn owned_steps(plan: &Value, events: &[nomifun_api_types::AgentExecutionEvent],
    current: &BTreeSet<String>, session: &AgentSessionId, turns: &BTreeSet<OperationId>) -> Result<BTreeSet<String>, AgentEngineError> {
    if plan.get("source").is_some_and(|source| source_matches(source, session, turns)) {
        return Ok(current.clone());
    }
    let mut ids = BTreeSet::new();
    for event in events {
        if event.payload.get("delegation_source").is_some_and(|source| source_matches(source, session, turns)) {
            let added = event.payload.get("added_step_ids").and_then(Value::as_array)
                .ok_or_else(|| failure("delegation event has no committed children"))?;
            for id in added { ids.insert(id.as_str().ok_or_else(|| failure("invalid delegated Step identity"))?.to_owned()); }
        }
    }
    if ids.is_empty() { return Err(failure("this exact source Turn does not own the selected delegation")); }
    ids.retain(|id|current.contains(id));
    Ok(ids)
}

fn settlement_event<'a>(events: &'a [nomifun_api_types::AgentExecutionEvent], step: &str, status: &str)
    -> Option<&'a nomifun_api_types::AgentExecutionEvent> {
    let direct=events.iter().rev().find(|event| {
        let payload=&event.payload;
        if status=="cancelled" && payload.get("action").and_then(Value::as_str)==Some("cancel_step") {
            return payload.get("cancelled_step_ids").and_then(Value::as_array)
                .is_some_and(|ids|ids.iter().any(|id|id.as_str()==Some(step)));
        }
        event.step_id.as_deref()==Some(step) && (
            payload.get("step_status").or_else(||payload.get("status")).and_then(Value::as_str)==Some(status)
            || (status=="completed" && payload.get("change").and_then(Value::as_str)==Some("output_adopted"))
            || (status=="waiting_input" && event.event_type==nomifun_common::AgentExecutionEventKind::DecisionRequested))
    });
    direct.or_else(|| (status=="cancelled").then(||events.iter().rev().find(|event|
        event.event_type==nomifun_common::AgentExecutionEventKind::StatusChanged
        && matches!(event.payload.get("status").and_then(Value::as_str),Some("cancelled"|"failed")))).flatten())
}

fn native_terminal(event: &nomifun_api_types::AgentExecutionEvent) -> Result<Option<(&str,&str)>,AgentEngineError> {
    match (event.payload.get("canonical_operation_id").and_then(Value::as_str),
        event.payload.get("terminal_event_id").and_then(Value::as_str)) {
        (None,None)=>Ok(None),
        (Some(operation),Some(terminal)) if !operation.is_empty() && !terminal.is_empty()=>Ok(Some((operation,terminal))),
        _=>Err(failure("domain settlement has incomplete native terminal references")),
    }
}

fn domain_receipt(execution: &str, step: &str, status: &str, event: &nomifun_api_types::AgentExecutionEvent) -> Value {
    json!({"execution_id":execution,"execution_event_sequence":event.sequence,"step_id":step,"status":status,
        "event_type":event.event_type,"source_step_id":event.step_id,"source_attempt_id":event.attempt_id})
}

const RESULT_BODY_BUDGET: usize = 200 * 1024;
const RESULT_PAGE_BYTES: usize = 32 * 1024;

fn result_page(
    child: &mut Value, requested: Option<&nomifun_agent_runtime::AgentCollaborationResultPage>, budget: &mut usize,
) -> Result<(), AgentEngineError> {
    let Some(text)=child.get("result").and_then(Value::as_str) else {return Ok(());};
    let text=text.to_owned();
    let identity=if child.get("source").is_some() {
        json!({"attempt_id":child["attempt_id"],"source":child["source"]})
    } else {json!({"attempt_id":child["attempt_id"],"terminal_receipt":child["terminal_receipt"]})};
    let source=nomifun_agent_contracts::digest_payload(&identity).map_err(failure)?;
    let offset=if let Some(requested)=requested {
        if requested.source_digest!=source.as_ref() {return Err(failure("result page belongs to a different original child result"));}
        usize::try_from(requested.offset).map_err(failure)?
    } else {0};
    if offset>text.len() || !text.is_char_boundary(offset) {return Err(failure("result page offset is not an original UTF-8 boundary"));}
    let mut end=text.len().min(offset.saturating_add(RESULT_PAGE_BYTES));
    while !text.is_char_boundary(end) {end-=1;}
    while end>offset && serde_json::to_vec(&text[offset..end]).map_err(failure)?.len()>*budget {
        end=offset+(end-offset)/2;
        while !text.is_char_boundary(end) {end-=1;}
    }
    let fragment=&text[offset..end];
    *budget=budget.saturating_sub(serde_json::to_vec(fragment).map_err(failure)?.len());
    child["result"]=json!(fragment);
    child["result_page"]=json!({"sha256":nomifun_agent_contracts::digest_bytes(text.as_bytes()),
        "total_bytes":text.len(),"offset":offset,"next_offset":(end<text.len()).then_some(end),"source_digest":source});
    Ok(())
}

struct DeletedObservation { row:Value, receipt:Value, owned:BTreeSet<String> }

pub(super) struct NativeDeliveryObservation {
    pub(super) native:Option<(String,String)>, pub(super) pending:bool, pub(super) state:Value,
}

pub(super) async fn native_delivery_observation<'a>(owner:&crate::router::nomi_core_session::NomiCoreSessionOwner,
    principal:&str,execution:&str,step:&str,attempt:impl Into<NativeAttemptReference<'a>>,key:&str,zero_start_fenced:bool)
    ->Result<NativeDeliveryObservation,AgentEngineError> {
    let attempt=attempt.into();
    let pool=owner.collaboration_database();
    let Some(session)=attempt.conversation_id.as_deref() else {
        let mut tx=pool.begin().await.map_err(read_error::sql)?;
        let linked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation IN ('attempt','automation'))")
            .bind(execution).bind(step).bind(attempt.attempt_id).fetch_one(&mut *tx).await.map_err(read_error::sql)?;
        tx.commit().await.map_err(read_error::sql)?;
        let pending=!zero_start_fenced || linked;
        return Ok(NativeDeliveryObservation {native:None,pending,
            state:json!({"attempt_id":attempt.attempt_id,"turn_operation_key":key,
                "state":if pending {"source_unavailable"} else {"not_started"}})});
    };
    let operation=format!("turn:user:{principal}:{session}:{key}");
    let session_id=AgentSessionId::from(session.to_owned());
    owner.canonical().session_record(&PrincipalRef {principal_kind:"user".into(),principal_id:principal.into()},&session_id).await.map_err(read_error::app)?;
    let receipt=owner.canonical().store().read_turn_receipt(&session_id,&operation.clone().into()).await.map_err(read_error::session)?;
    use nomifun_agent_session::TurnReceiptStatus;
    let (native,pending,state)=match receipt.status {
        TurnReceiptStatus::Completed|TurnReceiptStatus::Failed|TurnReceiptStatus::Cancelled=>{
            let terminal=receipt.terminal_event.as_ref().ok_or_else(||failure("closed original child Turn has no terminal event"))?;
            (Some((operation.clone(),terminal.event_id.as_ref().to_owned())),false,
                json!({"attempt_id":attempt.attempt_id,"agent_session_id":session,"turn_operation_id":operation,
                    "terminal_event_id":terminal.event_id,"state":"closed"}))
        },
        TurnReceiptStatus::NotFound=>{
            let mut tx=pool.begin().await.map_err(read_error::sql)?;
            let eligible=nomifun_db::native_resume_target_eligible_tx(&mut tx,principal,session,&operation).await.map_err(read_error::db)?;
            tx.commit().await.map_err(read_error::sql)?;
            let pending=!zero_start_fenced||eligible;
            (None,pending,json!({"attempt_id":attempt.attempt_id,"agent_session_id":session,"turn_operation_id":operation,
                "state":if pending {"source_unavailable"} else {"not_started"}}))
        },
        TurnReceiptStatus::Running=>(None,true,json!({"attempt_id":attempt.attempt_id,"agent_session_id":session,
            "turn_operation_id":operation,"started_event_id":receipt.started_event.as_ref().map(|event|&event.event_id),"state":"running"})),
    };
    Ok(NativeDeliveryObservation {native,pending,state})
}

fn original_attempt_turn_keys<'a>(events:&[nomifun_api_types::AgentExecutionEvent],principal:&str,step:&str,
    attempt:impl Into<NativeAttemptReference<'a>>,current:&str)->Result<BTreeSet<String>,AgentEngineError> {
    let attempt=attempt.into();
    let mut keys=BTreeSet::from([nomifun_db::AttemptConversationEffects::default()
        .current_turn_operation_key(&attempt.attempt_id).map_err(failure)?,current.to_owned()]);
    for event in events.iter().filter(|event|event.step_id.as_deref()==Some(step)
        &&event.attempt_id.as_deref()==Some(attempt.attempt_id)) {
        if let Some(key)=event.payload.get("turn_operation_key").and_then(Value::as_str) {keys.insert(key.into());}
        if event.event_type==nomifun_common::AgentExecutionEventKind::DecisionAnswered {
            if let Some(key)=event.payload.get("operation_id").and_then(Value::as_str) {keys.insert(key.into());}
        }
        if let Some(operation)=event.payload.get("canonical_operation_id").and_then(Value::as_str) {
            let session=attempt.conversation_id.as_deref().ok_or_else(||failure("original native reference has no child Session"))?;
            let prefix=format!("turn:user:{principal}:{session}:");
            keys.insert(operation.strip_prefix(&prefix).filter(|key|!key.is_empty())
                .ok_or_else(||failure("original Attempt Turn reference belongs to another Session"))?.to_owned());
        }
    }
    if keys.iter().any(|key|key.is_empty()||key.len()>4096||key.contains('\0')) {
        return Err(failure("original Attempt has an invalid canonical Turn key"));
    }
    Ok(keys)
}

/// Deletion is an explicit user revocation, observed from its original domain
/// event. This narrow reader cannot reopen the normal Execution APIs or tools.
async fn deleted_observation(pool:&nomifun_db::SqlitePool,owner:&str,id:&str,
    session:&AgentSessionId,turns:&BTreeSet<OperationId>)->Result<Option<DeletedObservation>,AgentEngineError> {
    let row:Option<(String,String,i64)>=sqlx::query_as(
        "SELECT initial_plan_input,status,event_sequence FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NOT NULL")
        .bind(id).bind(owner).fetch_optional(pool).await.map_err(read_error::sql)?;
    let Some((plan,status,sequence))=row else {return Ok(None);};
    if !matches!(status.as_str(),"completed"|"completed_with_failures"|"failed"|"cancelled") {
        return Err(failure("deleted Execution has no original terminal state"));
    }
    let deletions:Vec<(i64,String)>=sqlx::query_as(
        "SELECT sequence,payload FROM agent_execution_events WHERE execution_id=? AND event_type='deleted' AND sequence<=? ORDER BY sequence LIMIT 2")
        .bind(id).bind(sequence).fetch_all(pool).await.map_err(read_error::sql)?;
    let [(deleted_sequence,payload)]=deletions.as_slice() else {return Err(failure("deleted Execution has no unique original deletion event"));};
    let payload:Value=serde_json::from_str(payload).map_err(failure)?;
    if *deleted_sequence<=0 || payload.get("deleted").and_then(Value::as_bool)!=Some(true) {
        return Err(failure("Execution deletion receipt is invalid"));
    }
    let historical=sqlx::query_scalar::<_,String>("SELECT step_id FROM agent_execution_steps WHERE execution_id=?")
        .bind(id).fetch_all(pool).await.map_err(read_error::sql)?.into_iter().collect::<BTreeSet<_>>();
    let plan:Value=serde_json::from_str(&plan).map_err(failure)?;
    let (source,owned)=if let Some(source)=plan.get("source").filter(|source|source_matches(source,session,turns)) {
        (source.clone(),historical)
    } else {
        let changes:Vec<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND event_type='plan_changed' AND sequence<=? ORDER BY sequence")
            .bind(id).bind(sequence).fetch_all(pool).await.map_err(read_error::sql)?;
        let mut source=None;let mut owned=BTreeSet::new();
        for value in changes {
            let value:Value=serde_json::from_str(&value).map_err(failure)?;
            let Some(exact)=value.get("delegation_source").filter(|source|source_matches(source,session,turns)) else {continue;};
            let added=value.get("added_step_ids").and_then(Value::as_array)
                .filter(|ids|!ids.is_empty()).ok_or_else(||failure("deleted delegation has no original owned Steps"))?;
            for id in added {owned.insert(id.as_str().filter(|id|!id.is_empty()).ok_or_else(||failure("invalid deleted child identity"))?.to_owned());}
            source.get_or_insert_with(||exact.clone());
        }
        owned.retain(|id|historical.contains(id));
        (source.ok_or_else(||failure("deleted Execution is outside the exact source delegation"))?,owned)
    };
    let receipt=json!({"execution_id":id,"execution_event_sequence":deleted_sequence,"event_type":"deleted",
        "deleted":true,"status":status,"source":source});
    let row=json!({"execution_id":id,"status":status,"sequence":sequence,"deleted":true,"user_revoked":true,
        "outputs_available":false,"children":[],"owned_step_ids":[],"owned_terminal_step_ids":[],
        "execution_terminal_receipt":receipt,"execution_attention":null});
    Ok(Some(DeletedObservation {row,receipt,owned}))
}
impl HostPort {
    async fn admitted(&self, causality: &ChatCausality) -> Result<Arc<ConversationRuntimeHost>, AgentEngineError> {
        let host = self.0.upgrade().ok_or_else(|| failure("Runtime owner has retired"))?;
        {
            let active = host.active.lock().await;
            let turn = active.as_ref().ok_or_else(|| failure("source Turn is no longer active"))?;
            steering::admitted(turn, &host, causality)?;
        }
        Ok(host)
    }

    async fn read(&self, causality: &ChatCausality, ids: &[String], step_ids: &[String], results: bool,
        page:Option<&nomifun_agent_runtime::AgentCollaborationResultPage>)
        -> Result<AgentCollaborationSnapshot, AgentEngineError> {
        self.read_with_cache(causality,ids,step_ids,results,page,&mut ObservationCache::default()).await
    }

    async fn read_with_cache(&self, causality: &ChatCausality, ids: &[String], step_ids: &[String], results: bool,
        page:Option<&nomifun_agent_runtime::AgentCollaborationResultPage>, cache:&mut ObservationCache)
        -> Result<AgentCollaborationSnapshot, AgentEngineError> {
        let host = self.admitted(causality).await?;
        let owner = host.session_host.session_owner().map_err(failure)?;
        let engine = owner.collaboration_engine().map_err(failure)?;
        let mut turns=owner.collaboration_source_turns(&host.principal.principal_id,
            &causality.agent_session_id,&causality.turn_operation_id).await.map_err(read_error::app)?
            .into_iter().collect::<BTreeSet<_>>();
        turns.insert(causality.turn_operation_id.clone());
        let mut rows = Vec::new();
        let mut cursors = BTreeMap::new();
        let mut all_ready = true;
        let mut any_ready = false;
        let mut needs_attention = false;
        let mut can_progress = false;
        let mut selected_steps=BTreeSet::new();
        let mut result_budget=RESULT_BODY_BUDGET;
        let mut requested_page_returned=page.is_none();
        'executions: for id in ids {
            macro_rules! observe_deletion {
                ($deleted:expr)=>{{
                    let mut deleted=$deleted;
                    for step in step_ids.iter().filter(|step|deleted.owned.contains(*step)) {selected_steps.insert(step.clone());}
                    if page.is_some_and(|page|page.execution_id==*id && deleted.owned.contains(&page.step_id)) {requested_page_returned=true;}
                    any_ready=true;
                    let reference=nomifun_agent_contracts::digest_payload(&json!({"source_session":causality.agent_session_id,
                        "source_turn":causality.turn_operation_id,"execution":id,"deletion":deleted.receipt})).map_err(failure)?;
                    let reference=nomifun_agent_runtime::AgentCollaborationObservation{state:reference.as_ref().into(),ready:reference.as_ref().into()};
                    deleted.row["observation_cursor"]=json!(reference);
                    deleted.row["step_observations"]=json!({});
                    cursors.insert(id.clone(),reference);
                    rows.push(deleted.row);
                    continue 'executions;
                }}
            }
            let detail=cache.read(engine.as_ref(),&host.principal.principal_id,id).await?;
            if detail.execution.deleted_at.is_some() {
                let deleted=deleted_observation(owner.collaboration_database(),&host.principal.principal_id,id,&causality.agent_session_id,&turns).await?
                    .ok_or_else(||failure("original Execution deletion is unavailable"))?;
                observe_deletion!(deleted);
            }
            let plan: Value = serde_json::from_str(&detail.execution.initial_plan_input).map_err(failure)?;
            let execution_status=detail.execution.status.parse::<nomifun_common::AgentExecutionStatus>().map_err(failure)?;
            let events=&detail.state.events;
            let current=detail.state.steps.iter().filter(|step|step.superseded_in_revision.is_none())
                .map(|step|step.step_id.clone()).collect::<BTreeSet<_>>();
            let owned = owned_steps(&plan, &events, &current, &causality.agent_session_id, &turns)?;
            // A dispatcher can schedule queued/ready work or resolve lease
            // uncertainty. A loop merely watching a question/review is not a
            // producer that can decide it on the parent's behalf.
            let mut owned_terminal_step_ids=Vec::new();
            let mut terminal_sources=Vec::new();
            let mut step_observations=BTreeMap::new();
            let execution_terminal=if execution_status.is_terminal() {
                let event=events.iter().rev().find(|event|event.event_type==nomifun_common::AgentExecutionEventKind::StatusChanged
                    && event.payload.get("status").and_then(Value::as_str)==Some(detail.execution.status.as_str()))
                    .ok_or_else(||failure("terminal Execution has no original status event"))?;
                Some(json!({"execution_id":id,"execution_event_sequence":event.sequence,
                    "status":detail.execution.status,"event_type":event.event_type}))
            } else {None};
            let execution_attention=if execution_status==nomifun_common::AgentExecutionStatus::Paused {
                let event=events.iter().rev().find(|event|event.event_type==nomifun_common::AgentExecutionEventKind::StatusChanged
                    && event.payload.get("status").and_then(Value::as_str)==Some("paused"))
                    .ok_or_else(||failure("paused Execution has no original control event"))?;
                needs_attention=true;
                Some(json!({"execution_id":id,"execution_event_sequence":event.sequence,
                    "status":"paused","event_type":event.event_type}))
            } else {None};
            let mut children = Vec::new();
            for step in detail.state.steps.iter().filter(|step|owned.contains(&step.step_id)) {
                let terminal = step.status.is_terminal();
                let latest=detail.state.attempts.iter().filter(|attempt|attempt.step_id==step.step_id).max_by_key(|attempt|attempt.attempt_no);
                let job_owned=engine.collaboration_has_step_job(&host.principal.principal_id,id,&step.step_id,
                    latest.map(|attempt|attempt.attempt_id.as_str()),Some(step.version)).await.map_err(read_error::app)?;
                let mut native_intervention=None;
                if let Some(original)=latest && let Some(session)=original.conversation_id.as_deref() {
                    let operation=format!("turn:user:{}:{session}:{}",host.principal.principal_id,original.turn_operation_key);
                    let progress=owner.canonical().store().read_turn_progress(&session.to_owned().into(),&operation.clone().into())
                        .await.map_err(read_error::session)?;
                    native_intervention=progress.suspension.as_ref().filter(|boundary|!terminal&&boundary.requires_intervention())
                        .map(|boundary|serde_json::to_value(boundary).map(|boundary|json!({"agent_session_id":session,
                            "turn_operation_id":operation,"boundary":boundary}))).transpose().map_err(failure)?;
                    if terminal || !progress.suspension.as_ref().is_some_and(|boundary|boundary.requires_intervention()) {
                        can_progress|=owner.collaboration_has_native_progress_owner(session,&operation).await.map_err(read_error::app)?;
                    }
                }
                // Custody of an already suspended result is not a producer.
                // Keep that exact native boundary visible even before its
                // independent Step owner can publish the domain review.
                can_progress|=job_owned&&!terminal&&native_intervention.is_none();
                can_progress|=engine.collaboration_has_restart_owner(&host.principal.principal_id,id,&step.step_id).await.map_err(read_error::app)?;
                let effects=latest.map(|attempt|nomifun_db::AttemptConversationEffects::decode(attempt.runtime_state.as_ref()
                    .map(serde_json::to_string).transpose().map_err(failure)?.as_deref()).map_err(failure)).transpose()?;
                let review=effects.as_ref().and_then(|effects|effects.review_blocked.as_ref());
                let waiting_input=step.status==ExecutionStepStatus::WaitingInput;
                let reply_pending=effects.as_ref().is_some_and(|effects|effects.pending_conversation_effects.iter()
                    .any(|effect|matches!(effect,nomifun_db::PendingConversationEffect::DecisionInput{..})));
                let decision_input_pending=reply_pending&&review.is_none()&&!terminal;
                if decision_input_pending&&native_intervention.is_none()&&dispatchable_step(step.status.as_str(),true) {
                    // Accepted DecisionInput is work waiting for capacity, not
                    // another question. Only its existing dispatch task can
                    // prove progress; the outbox alone never proves an owner.
                    can_progress|=engine.collaboration_has_dispatch_owner(&host.principal.principal_id,id).await.map_err(read_error::app)?;
                }
                let question_attention=waiting_input&&!reply_pending&&review.is_none()
                    &&latest.is_some_and(|attempt|attempt.question.as_ref().is_some_and(|question|!question.trim().is_empty()));
                let review_attention=waiting_input&&review.is_some();
                if waiting_input&&!reply_pending&&!question_attention&&!review_attention {
                    return Err(failure("waiting child has neither its original unanswered question, an accepted reply nor a review boundary"));
                }
                let proof=if terminal || waiting_input {
                    Some((if waiting_input {current_wait_proof(events,&step.step_id,latest.map(|attempt|attempt.attempt_id.as_str()),review_attention)}
                        else {settlement_event(events,&step.step_id,step.status.as_str())})
                        .ok_or_else(||failure("child state has no original domain settlement or question event"))?)
                } else {None};
                let receipt=proof.map(|event|domain_receipt(id,&step.step_id,step.status.as_str(),event));
                let mut resolved_native=proof.map(native_terminal).transpose()?.flatten()
                    .map(|(operation,terminal)|(operation.to_owned(),terminal.to_owned()));
                let attempt=if let Some(attempt_id)=proof.and_then(|event|event.attempt_id.as_deref()) {
                    Some(detail.state.attempts.iter().find(|attempt|attempt.step_id==step.step_id && attempt.attempt_id==attempt_id)
                        .ok_or_else(||failure("domain settlement refers to a foreign Attempt"))?)
                } else {latest};
                let mut native_attempts=Vec::new();
                let mut delivery_pending=false;
                if step.kind==nomifun_common::ExecutionStepKind::Agent && (terminal||waiting_input) {
                    for original in detail.state.attempts.iter().filter(|attempt|attempt.step_id==step.step_id) {
                        // A terminal domain generation fences late create/start;
                        // a missing original Turn is still checked separately.
                        let zero_start_fenced=original.status.is_terminal();
                        let current_key=original.turn_operation_key.clone();
                        for key in original_attempt_turn_keys(&events,&host.principal.principal_id,&step.step_id,original,&current_key)? {
                            let observation=native_delivery_observation(owner.as_ref(),&host.principal.principal_id,id,
                                &step.step_id,original,&key,zero_start_fenced).await?;
                            delivery_pending|=observation.pending;
                            if key==current_key && attempt.is_some_and(|attempt|attempt.attempt_id==original.attempt_id)
                                && resolved_native.is_none() {
                                resolved_native=observation.native.clone();
                            }
                            native_attempts.push((original,observation));
                        }
                    }
                }
                let native=resolved_native.as_ref().map(|(operation,terminal)|(operation.as_str(),terminal.as_str()));
                let source=native.map(|(operation,terminal)|json!({"agent_session_id":attempt.and_then(|attempt|attempt.conversation_id.as_deref()),
                    "turn_operation_id":operation,"terminal_event_id":terminal}));
                let mut current_output=None;
                let decision_input_boundary=if question_attention&&!delivery_pending&&step.kind==nomifun_common::ExecutionStepKind::Agent {
                    let attempt=attempt.ok_or_else(||failure("native question has no exact source Attempt"))?;
                    let (operation,terminal)=native.ok_or_else(||failure("native question has no exact InputRequired terminal"))?;
                    if proof.is_none_or(|event|event.event_type!=nomifun_common::AgentExecutionEventKind::DecisionRequested
                        ||event.attempt_id.as_deref()!=Some(attempt.attempt_id.as_str()))
                        ||latest.is_none_or(|latest|latest.attempt_id!=attempt.attempt_id) {
                        return Err(failure("native question differs from its current DecisionRequested Attempt"));
                    }
                    let session=attempt.conversation_id.as_deref().ok_or_else(||failure("native question has no original child Session"))?;
                    let prefix=format!("turn:user:{}:{session}:",host.principal.principal_id);
                    let key=operation.strip_prefix(&prefix).filter(|key|!key.is_empty())
                        .ok_or_else(||failure("native question belongs to another child Session"))?;
                    let output=owner.read_turn_output(&host.principal.principal_id,session,Some(key)).await.map_err(read_error::app)?
                        .ok_or_else(||failure("native question lacks its original InputRequired output"))?;
                    if output.canonical_operation_id!=operation||output.terminal_event_id.as_deref()!=Some(terminal)||!output.delivery.completed {
                        return Err(failure("native question differs from its closed canonical Turn"));
                    }
                    let request=output.delivery.user_input_request.as_ref().ok_or_else(||failure("DecisionRequested has no typed native question"))?;
                    if attempt.question.as_deref()!=Some(request.question.as_str()) {
                        return Err(failure("DecisionRequested question differs from its exact native input handoff"));
                    }
                    let facts=owner.canonical().store().turn_output_facts(&session.to_owned().into(),&operation.to_owned().into()).await.map_err(read_error::session)?;
                    let encoded=serde_json::to_value(request).map_err(failure)?;
                    let questions=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.kind_version==1
                        &&event.producer_id.as_ref()=="runtime_supervisor"&&event.correlation_id.as_ref()==operation
                        &&facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event"))
                            .is_some_and(|body|body.get("event").and_then(Value::as_str)==Some("user_input_requested")
                                &&body.get("request")==Some(&encoded))).collect::<Vec<_>>();
                    let [question]=questions.as_slice() else {return Err(failure("child input handoff has no unique exact question event"));};
                    let reference=nomifun_agent_contracts::AgentQuestionReplyReference {agent_session_id:session.to_owned().into(),
                        turn_operation_id:operation.to_owned().into(),question_event_id:question.event_id.clone(),request_id:request.request_id.clone()};
                    current_output=Some(output);
                    Some(reference)
                }else {None};
                let terminal_digest=if terminal&&!delivery_pending {
                    Some(nomifun_agent_contracts::digest_payload(&json!({"terminal_receipt":receipt,"attempt_id":attempt.map(|attempt|&attempt.attempt_id),
                        "source":source})).map_err(failure)?)
                } else {None};
                let page_source_digest=if terminal&&!delivery_pending {
                    let identity=if source.is_some() {json!({"attempt_id":attempt.map(|attempt|&attempt.attempt_id),"source":source})}
                        else {json!({"attempt_id":attempt.map(|attempt|&attempt.attempt_id),"terminal_receipt":receipt})};
                    Some(nomifun_agent_contracts::digest_payload(&identity).map_err(failure)?)
                } else {None};
                let pending_commands=effects.as_ref().map(|effects|effects.pending_conversation_effects.iter()
                    .map(|effect|nomifun_agent_contracts::digest_payload(effect).map(|digest|digest.as_ref().to_owned()))
                    .collect::<Result<Vec<_>,_>>()).transpose().map_err(failure)?.unwrap_or_default();
                let blocked_restart=if terminal {None}else {blocked_restart_receipt(events,&step.step_id,latest.map(|attempt|attempt.attempt_id.as_str()))?};
                let attention=question_attention||review_attention||native_intervention.is_some()||blocked_restart.is_some();
                let ready_digest=if terminal&&!delivery_pending {terminal_digest.clone()} else if native_intervention.is_some() {
                    // The same canonical pause is one decision. Its later
                    // domain projection or Restart acceptance cannot present
                    // that unchanged decision as another model-visible fact.
                    Some(native_pause_ready_digest(latest.map(|attempt|attempt.attempt_id.as_str()),native_intervention.as_ref().expect("checked canonical pause"),blocked_restart.as_ref())?)
                } else if attention {
                    Some(nomifun_agent_contracts::digest_payload(&json!({"receipt":receipt,"attempt_id":attempt.map(|attempt|&attempt.attempt_id),
                        "native":source,"review_blocked":review,"native_intervention":native_intervention,"blocked_restart":blocked_restart})).map_err(failure)?)
                } else {None};
                // Full-scope lightweight heads invalidate only the changed
                // Step, including a newly admitted Attempt before it finishes.
                step_observations.insert(step.step_id.clone(),json!({"status":step.status,
                    "step_version":step.version,
                    "agent_session_id":latest.and_then(|attempt|attempt.conversation_id.as_deref()),
                    "attempt_id":latest.map(|attempt|&attempt.attempt_id),"attempt_no":latest.map(|attempt|attempt.attempt_no),
                    "turn_operation_key":latest.map(|attempt|&attempt.turn_operation_key),"pending_commands":pending_commands,
                    "delivery_pending":delivery_pending,"decision_input_pending":decision_input_pending,
                    "native_intervention":native_intervention,
                    "blocked_restart":blocked_restart,
                    "decision_input_boundary":decision_input_boundary,
                    "accepted_control":events.iter().rev().find(|event|event.step_id.as_deref()==Some(&step.step_id)
                        &&matches!(event.payload.get("change").and_then(Value::as_str),Some("retry_requested"|"restart_requested"|"restart_resolved"|"recheck_requested"|"replaced_with_existing")))
                        .map(|event|json!({"sequence":event.sequence,"operation_id":event.payload["operation_id"],"change":event.payload["change"],
                            "disposition":event.payload["disposition"],"code":event.payload["code"]})),
                    "terminal_digest":terminal_digest,"page_source_digest":page_source_digest,"ready_digest":ready_digest}));
                // An accepted answer waiting for admission is still the
                // original outbox's work, not another question. Native receipt
                // delivery/cleanup can also remain pending without waking a
                // model that would consume the capacity that work needs.
                all_ready&=terminal&&!delivery_pending;
                any_ready|=(terminal&&!delivery_pending)||attention;
                needs_attention|=attention;
                if terminal&&!delivery_pending {owned_terminal_step_ids.push(step.step_id.clone());}
                // Cursor identity covers every owned result/question, independent
                // of the optional result-body page selected by this status call.
                if let Some(receipt)=receipt.as_ref() {
                    terminal_sources.push(json!({"receipt":receipt,"attempt_id":attempt.map(|attempt|&attempt.attempt_id),
                        "native":native.map(|(operation,terminal)|json!({"turn_operation_id":operation,"terminal_event_id":terminal})),
                        "native_delivery":native_attempts.iter().map(|(_,observation)|&observation.state).collect::<Vec<_>>(),
                        "review_blocked":review}));
                }
                if !step_ids.is_empty() && !step_ids.contains(&step.step_id) {continue;}
                if page.is_some_and(|page|page.execution_id!=*id || page.step_id!=step.step_id) {continue;}
                selected_steps.insert(step.step_id.clone());
                let mut child = json!({"step_id":step.step_id,"title":step.title,"status":step.status,
                    "version":step.version,
                    "attempt_id":attempt.map(|attempt|attempt.attempt_id.as_str()),
                    "agent_session_id":attempt.and_then(|attempt|attempt.conversation_id.as_deref()),
                    "terminal_receipt":if terminal {receipt.clone()} else {None},
                    "question_source":if question_attention {receipt} else {None},"review_blocked":review,
                    "native_intervention":native_intervention,
                    "blocked_restart":blocked_restart,
                    "delivery_pending":delivery_pending,"workspace_change_sources":[]});
                if results {
                    let mut changes=Vec::new();
                    for (original,observation) in &native_attempts {
                        let Some((operation,terminal_event))=observation.native.as_ref() else {continue;};
                        let session=original.conversation_id.as_deref().ok_or_else(||failure("native delivery has no original child Session"))?;
                        let prefix=format!("turn:user:{}:{session}:",host.principal.principal_id);
                        let key=operation.strip_prefix(&prefix).filter(|key|!key.is_empty())
                            .ok_or_else(||failure("original native delivery belongs to another child Session"))?;
                        let output=if current_output.as_ref().is_some_and(|output:&nomifun_agent_execution::AgentExecutionTurnOutput|
                            output.canonical_operation_id==*operation&&output.terminal_event_id.as_deref()==Some(terminal_event.as_str())) {
                            current_output.take().expect("exact output checked")
                        }else {owner.read_turn_output(&host.principal.principal_id,session,Some(key)).await.map_err(read_error::app)?
                            .ok_or_else(||failure("closed original child Turn has no canonical output"))?};
                        if output.canonical_operation_id!=*operation || output.terminal_event_id.as_deref()!=Some(terminal_event.as_str())
                            ||!output.delivery.completed {return Err(failure("original child output differs from its exact native receipt"));}
                        if output.workspace_changed {changes.push(json!({"attempt_id":original.attempt_id,
                            "source":{"agent_session_id":session,"turn_operation_id":operation,"terminal_event_id":terminal_event}}));}
                        if attempt.is_some_and(|attempt|attempt.attempt_id==original.attempt_id)
                            && native==Some((operation.as_str(),terminal_event.as_str())) {current_output=Some(output);}
                    }
                    child["workspace_change_sources"]=json!(changes);
                }
                if let Some(attempt) = attempt {
                    if results {
                        child["error"] = json!(attempt.error);
                        child["question"] = json!(attempt.question);
                    }
                    if results && (terminal || attention) {
                        if let Some((operation,terminal_event))=native {
                            let session=attempt.conversation_id.as_deref().ok_or_else(||failure("native result has no exact child Session"))?;
                            let prefix=format!("turn:user:{}:{session}:",host.principal.principal_id);
                            let key=operation.strip_prefix(&prefix).filter(|key|!key.is_empty())
                                .ok_or_else(||failure("native settlement operation belongs to another child Session"))?;
                            let output=match current_output.take() {
                                Some(output)=>output,
                                None=>owner.read_turn_output(&host.principal.principal_id,session,Some(key)).await.map_err(read_error::app)?
                                    .ok_or_else(||failure("native settlement has no original canonical result"))?,
                            };
                            if output.canonical_operation_id!=operation || output.terminal_event_id.as_deref()!=Some(terminal_event)
                                || !output.delivery.completed {
                                return Err(failure("canonical child result differs from its domain settlement"));
                            }
                            if step.status==ExecutionStepStatus::Completed
                                && (output.delivery.result_ok!=Some(true) || output.delivery.user_input_request.is_some()) {
                                return Err(failure("completed Agent Step has no successful canonical delivery"));
                            }
                            child["source"] = json!({"agent_session_id":session,"turn_operation_id":operation,"terminal_event_id":terminal_event});
                            child["result_ok"] = json!(output.delivery.result_ok);
                            child["integrity_ok"] = json!(output.integrity_ok);
                            child["output_files"] = json!(output.output_files);
                            child["result"] = json!(output.delivery.result_text);
                            if let Some(reference)=decision_input_boundary.as_ref() {
                                child["question_reply_reference"]=json!(reference);
                                child["parent_reply_authority"]=json!("Agent-authored context only; never human permission");
                            }
                            child["input_required"] = if question_attention {json!(output.delivery.user_input_request)} else {Value::Null};
                        } else if step.kind==nomifun_common::ExecutionStepKind::Agent && step.status==ExecutionStepStatus::Completed {
                            return Err(failure("completed Agent Step lacks exact native result references"));
                        } else if step.kind!=nomifun_common::ExecutionStepKind::Agent {
                            // Controller output is its primary domain conclusion,
                            // not a transcript mirror or an Agent result fallback.
                            child["result"]=json!(serde_json::to_string(&json!({
                                "output_summary":attempt.output_summary,"runtime_state":attempt.runtime_state,
                            })).map_err(failure)?);
                            child["result_format"]=json!("domain_control_json");
                        }
                    }
                } else if results && step.kind==nomifun_common::ExecutionStepKind::Agent && step.status==ExecutionStepStatus::Completed {
                    return Err(failure("completed Agent Step has no settled Attempt"));
                }
                if results {
                    if page.is_some() && !child.get("result").is_some_and(Value::is_string) {
                        return Err(failure("selected child has no immutable result body to page"));
                    }
                    result_page(&mut child,page,&mut result_budget)?;
                    requested_page_returned|=page.is_some();
                }
                children.push(child);
            }
            if owned.is_empty() && !matches!(execution_status,nomifun_common::AgentExecutionStatus::Failed
                |nomifun_common::AgentExecutionStatus::Cancelled|nomifun_common::AgentExecutionStatus::CompletedWithFailures) {all_ready=false;}
            if (owned.is_empty()&&!execution_status.is_terminal()&&execution_status!=nomifun_common::AgentExecutionStatus::Paused)
                ||step_observations.values().any(ordinary_dispatchable) {
                can_progress|=engine.collaboration_has_dispatch_owner(&host.principal.principal_id,id).await.map_err(read_error::app)?;
            }
            any_ready|=execution_terminal.is_some() || execution_attention.is_some();
            let mut identities=step_observations.clone();
            for head in identities.values_mut() {head.as_object_mut().expect("typed Step header").remove("step_version");}
            let observation_cursor=nomifun_agent_contracts::digest_payload(&json!({"source_session":causality.agent_session_id,
                "source_turn":causality.turn_operation_id,"execution":id,"steps":identities,
                "terminal_sources":terminal_sources,"execution_terminal":execution_terminal,"attention":execution_attention})).map_err(failure)?;
            let ready_sources=step_observations.iter().filter_map(|(step,head)|head["ready_digest"].as_str().map(|digest|(step,digest))).collect::<BTreeMap<_,_>>();
            let ready=nomifun_agent_contracts::digest_payload(&json!({"source_session":causality.agent_session_id,
                "source_turn":causality.turn_operation_id,"execution":id,"ready_sources":ready_sources,
                "execution_terminal":execution_terminal,"attention":execution_attention})).map_err(failure)?;
            let observation_cursor=nomifun_agent_runtime::AgentCollaborationObservation{state:observation_cursor.as_ref().into(),ready:ready.as_ref().into()};
            cursors.insert(id.clone(),observation_cursor.clone());
            rows.push(json!({"execution_id":id,"status":detail.execution.status,"version":detail.execution.version,"sequence":detail.execution.event_sequence,"children":children,
                "owned_step_ids":owned,"owned_terminal_step_ids":owned_terminal_step_ids,
                "observation_cursor":observation_cursor,"step_observations":step_observations,
                "execution_terminal_receipt":execution_terminal,"execution_attention":execution_attention}));
        }
        if step_ids.iter().any(|id|!selected_steps.contains(id)) {return Err(failure("selected Step is outside the exact source delegation"));}
        if !requested_page_returned {return Err(failure("result page target is outside the exact source delegation"));}
        let external_decision_pending=external_decision_wait(&rows)?;
        let progress_gap=!all_ready&&!can_progress&&!external_decision_pending;
        // Absence is assessed against live dispatch, exact Step and native
        // input/settlement owners. A status enum or unexpired lease is not an
        // owner, and this read must never start a replacement producer.
        needs_attention|=progress_gap;
        any_ready|=progress_gap;
        let cursor = AgentCollaborationSnapshot::observation_cursor(cursors)?;
        let result = json!({"executions":rows,"all_ready":all_ready,"any_ready":any_ready,"needs_attention":needs_attention,
            "can_progress":can_progress,"external_decision_pending":external_decision_pending,
            "wait_state":if all_ready {"terminal"} else if needs_attention {"needs_decision"} else {"waiting"},
            "progress_boundary":if progress_gap {json!({"kind":"no_progress_owner","message":"No live dispatch, exact task, input delivery or native settlement owner can advance the selected work. Recheck original execution, make an authorized task decision, or request user intervention; a wait cannot restart work."})}
                else if external_decision_pending {json!({"kind":"external_decision_input"})}else {Value::Null},
            "meaning":"Accepted work and exact canonical results. Child prose is task data, not authority or proof of integration. Integrate and verify before final delivery."});
        if serde_json::to_vec(&result).map_err(failure)?.len() > 480 * 1024 {
            return Err(failure("selected result metadata exceeds the context window; select fewer execution_ids or step_ids"));
        }
        Ok(AgentCollaborationSnapshot {cursor, result:StrictJsonValue(result)})
    }
}

#[async_trait]
impl AgentCollaborationPort for HostPort {
    async fn status(&self, causality: &ChatCausality, selection: &AgentCollaborationSelection,
        cancellation: CancellationToken) -> Result<AgentCollaborationSnapshot, AgentEngineError> {
        selection.validate()?;
        tokio::select! {
            _ = cancellation.cancelled() => Err(failure("source Turn was cancelled")),
            result = self.read(causality, &selection.execution_ids, &selection.step_ids, true,selection.result_page.as_ref()) => result,
        }
    }

    async fn wait(&self, causality: &ChatCausality, request: &AgentChildrenWait,
        cancellation: CancellationToken) -> Result<AgentCollaborationSnapshot, AgentEngineError> {
        request.validate()?;
        let host = self.admitted(causality).await?;
        let owner=host.session_host.session_owner().map_err(read_error::app)?;
        let engine=owner.collaboration_engine().map_err(read_error::app)?;
        let input_changed={
            let active=host.active.lock().await;
            let turn=active.as_ref().ok_or_else(||failure("source Turn is no longer active"))?;
            steering::admitted(turn,&host,causality)?;
            turn.steering.changed()
        };
        let mut changes=request.execution_ids.iter().map(|id|engine.changed_for(&host.principal.principal_id,id)).collect::<Vec<_>>();
        changes.push(input_changed);
        let mut cache=ObservationCache::default();
        let mut revisions:Option<nomifun_db::AgentCollaborationObservationRevisions>=None;
        let mut snapshot=None;
        loop {
            // Arm every exact owner hint before observing. A commit during the
            // read leaves a ready notification for the next iteration.
            let notifications=arm_observation_changes(&changes);
            let pending=tokio::select! {
                _=cancellation.cancelled()=>return Err(failure("source Turn was cancelled")),
                result=async {
                    let input_selection={
                        let active=host.active.lock().await;
                        let turn=active.as_ref().ok_or_else(||failure("source Turn is no longer active"))?;
                        steering::admitted(turn,&host,causality)?;
                        turn.steering.pending_observation()
                    };
                    let current=engine.collaboration_revisions(&host.principal.principal_id,&request.execution_ids,
                        &cache.session_ids(&causality.agent_session_id)).await.map_err(read_error::app)?;
                    let source_changed=revisions.as_ref().is_none_or(|previous|
                        previous.sessions.iter().find(|session|session.session_id==causality.agent_session_id.as_ref())
                            !=current.sessions.iter().find(|session|session.session_id==causality.agent_session_id.as_ref()));
                    if revisions.as_ref()!=Some(&current) || snapshot.is_none() {
                        snapshot=Some(self.read_with_cache(causality,&request.execution_ids,&[],false,None,&mut cache).await?);
                        revisions=Some(current);
                    }
                    if input_selection.prepared {return Ok::<_,AgentEngineError>(true);}
                    if !input_selection.open ||(!source_changed&&input_selection.retry_inputs.is_empty()) {return Ok(false);}
                    let store=host.session_host.canonical_store().map_err(read_error::app)?;
                    let mut inputs=store.pending_session_inputs_for_steering(&causality.agent_session_id,&causality.turn_operation_id,None)
                        .await.map_err(read_error::session)?;
                    for retry in input_selection.retry_inputs {
                        inputs.extend(store.pending_session_inputs_for_steering(&causality.agent_session_id,&causality.turn_operation_id,
                            Some(&retry.into())).await.map_err(read_error::session)?);
                    }
                    Ok(inputs.iter().any(|input|!input_selection.deferred_inputs.contains(input.event.event_id.as_ref())))
                }=>result?,
            };
            let current=snapshot.as_ref().ok_or_else(||failure("collaboration wait has no read observation"))?;
            // Job ownership can change without another domain/native event.
            // Recheck only existing exact owner handles, never cache a claim
            // that a producer will eventually change this observation.
            let progress_owned=tokio::select! {biased;
                _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
                result=async {
                let mut progress_owned=false;
                for row in current.result.0["executions"].as_array().ok_or_else(||failure("wait scope has no Executions"))? {
                let execution=row["execution_id"].as_str().ok_or_else(||failure("wait Execution identity missing"))?;
                if row["deleted"]==true {continue;}
                let heads=row["step_observations"].as_object().ok_or_else(||failure("wait has no full Step headers"))?;
                if (heads.is_empty()&&matches!(row["status"].as_str(),Some("planning"|"pending"|"queued"|"running")))
                    ||heads.values().any(ordinary_dispatchable) {
                    progress_owned|=engine.collaboration_has_dispatch_owner(&host.principal.principal_id,execution).await.map_err(read_error::app)?;
                }
                for (step,head) in heads {
                    progress_owned|=engine.collaboration_has_restart_owner(&host.principal.principal_id,execution,step).await.map_err(read_error::app)?;
                    if head["native_intervention"].is_object() {continue;}
                    if !head["status"].as_str().is_some_and(terminal_step) {progress_owned|=engine.collaboration_has_step_job(&host.principal.principal_id,execution,step,
                        head["attempt_id"].as_str(),head["step_version"].as_i64()).await.map_err(read_error::app)?;
                    }
                    if let (Some(session),Some(key))=(head["agent_session_id"].as_str(),head["turn_operation_key"].as_str()) {
                        let operation=format!("turn:user:{}:{session}:{key}",host.principal.principal_id);
                        progress_owned|=owner.collaboration_has_native_progress_owner(session,&operation).await.map_err(read_error::app)?;
                    }
                }
                }
                Ok::<_,AgentEngineError>(progress_owned)
                }=>result?,
            };
            let changed_cursor=current.has_new_ready_observation(request.after_cursor.as_deref())?;
            let all_ready=current.result.0["all_ready"]==true;
            let external_decision_pending=current.result.0["external_decision_pending"]==true;
            let attention=current.result.0["needs_attention"]==true||(!all_ready&&!progress_owned&&!external_decision_pending);
            if pending {
                let mut input=snapshot.take().ok_or_else(||failure("collaboration observation disappeared"))?;
                input.result.0["wait_outcome"]=json!("parent_input_pending");
                return Ok(input);
            }
            match wait_disposition(request.mode,changed_cursor,all_ready,current.result.0["any_ready"]==true,attention,progress_owned,external_decision_pending) {
                WaitDisposition::Wake=>return snapshot.ok_or_else(||failure("collaboration observation disappeared")),
                WaitDisposition::DecisionBlocked=>{
                    let mut blocked=snapshot.take().ok_or_else(||failure("collaboration decision observation disappeared"))?;
                    blocked.result.0["wait_outcome"]=json!("decision_blocked");
                    blocked.result.0["can_progress"]=json!(progress_owned);
                    blocked.result.0["wait_state"]=json!("needs_decision");
                    if !all_ready&&!progress_owned {blocked.result.0["progress_boundary"]=json!({"kind":"no_progress_owner"});}
                    return Ok(blocked);
                }
                WaitDisposition::Park=>{},
            }
            // A missed publication or independent native settlement needs only
            // domain/native cursors, never a one-second full rebuild.
            wait_for_observation_change(notifications,&cancellation).await?;
        }
    }

    async fn send(&self, causality: &ChatCausality, call_id: &nomifun_chat_model_broker::ToolCallId, message: &AgentCollaborationMessage,
        cancellation: CancellationToken) -> Result<StrictJsonValue, AgentEngineError> {
        let host = self.admitted(causality).await?;
        if cancellation.is_cancelled() { return Err(failure("source Turn was cancelled")); }
        let snapshot = loop {
            let observed=tokio::select! {biased;
                _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
                result=self.read(causality,std::slice::from_ref(&message.execution_id),&[],false,None)=>result,
            };
            match observed {
                Err(AgentEngineError::ObservationUnavailable{..})=>tokio::select! {
                    _=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
                    _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{},
                },
                result=>break result?,
            }
        };
        let target = snapshot.result.0["executions"].as_array().into_iter().flatten()
            .flat_map(|execution|execution["children"].as_array().into_iter().flatten())
            .find(|child| child["step_id"].as_str() == Some(message.step_id.as_str()));
        if target.is_none() { return Err(AgentEngineError::CollaborationCommandRejected {
            code:nomifun_agent_contracts::AgentCollaborationCommandRejectCode::CommandUnavailable,
            message:"message target is outside the original delegation".into(),observation:None}); }
        let owner = host.session_host.session_owner().map_err(failure)?;
        let source={
            let active=host.active.lock().await;
            let turn=active.as_ref().ok_or_else(||failure("message source Turn is no longer active"))?;
            steering::admitted(turn,&host,causality)?;
            turn.journal.message_source_for_call(call_id).await.map_err(failure)?
        };
        #[cfg(test)]
        super::reliability_tests::child_message_cutpoint(causality.agent_session_id.as_ref(),false).await;
        let engine=owner.collaboration_engine().map_err(failure)?;
        let receipt=match engine.message_step(&host.principal.principal_id,&source,message).await {
            Ok(receipt)=>receipt,
            Err(AppError::DatabaseUnavailable{..})=>{
                // A write acknowledgement can be lost. Read only the original
                // stable command before saying whether it was accepted.
                let known=loop {
                    let observed=engine.message_receipt(&host.principal.principal_id,&source.agent_session_id,&source.turn_operation_id,message).await;
                    if matches!(&observed,Err(AppError::DatabaseUnavailable{..})) {
                        tokio::select! {_=cancellation.cancelled()=>return Err(AgentEngineError::Cancelled),
                            _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{}}
                    } else {break observed;}
                };
                match known {
                    Ok(Some(receipt))=>receipt,
                    Ok(None)=>return Err(AgentEngineError::CollaborationCommandRejected {
                        code:nomifun_agent_contracts::AgentCollaborationCommandRejectCode::TemporaryUnavailable,
                        message:"original command has no committed receipt; retry only this idempotency_key when storage is available".into(),observation:None}),
                    Err(_)=>return Err(AgentEngineError::WaitOwnerUnavailable),
                }
            }
            Err(error @ (AppError::Conflict(_)|AppError::NotFound(_)|AppError::Forbidden(_)|AppError::BadRequest(_)|AppError::ExecutionAdmissionPending))=>{
                let refresh=async {
                    #[cfg(test)]
                    {
                        let injected=REJECTED_COMMAND_READ_ERRORS.get_or_init(Default::default).lock().unwrap().remove(causality.agent_session_id.as_ref());
                        if let Some(error)=injected {return Err(error);}
                    }
                    self.read(causality,std::slice::from_ref(&message.execution_id),std::slice::from_ref(&message.step_id),true,None).await
                }.await;
                let fresh=match refresh {
                    Ok(snapshot)=>Some(snapshot),
                    Err(AgentEngineError::ObservationUnavailable{..})=>None,
                    Err(error)=>return Err(error),
                };
                let observed=fresh.as_ref().and_then(|snapshot|snapshot.result.0["executions"].as_array())
                    .into_iter().flatten().flat_map(|execution|execution["children"].as_array().into_iter().flatten())
                    .find(|child|child["step_id"].as_str()==Some(message.step_id.as_str())).cloned();
                let code=if matches!(&message.intent,nomifun_agent_contracts::AgentCollaborationMessageIntent::Guidance{})
                    &&observed.as_ref().or(target).is_some_and(|child|child["status"]=="waiting_input"
                        &&!child["question_source"].is_null()&&child["review_blocked"].is_null()) {
                    nomifun_agent_contracts::AgentCollaborationCommandRejectCode::ChildRequiresInput
                } else {nomifun_agent_contracts::AgentCollaborationCommandRejectCode::TargetStateChanged};
                return Err(AgentEngineError::CollaborationCommandRejected {code,message:error.to_string(),observation:observed.map(StrictJsonValue)});
            }
            Err(error)=>return Err(failure(error)),
        };
        #[cfg(test)]
        super::reliability_tests::child_message_cutpoint(causality.agent_session_id.as_ref(),true).await;
        let mut result=serde_json::to_value(receipt).map_err(failure)?;
        result["accepted"]=json!(true);
        result["idempotency_key"]=json!(message.idempotency_key);
        Ok(StrictJsonValue(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;

    #[tokio::test(start_paused = true)]
    async fn exact_wait_hints_survive_the_read_gap_and_stop_preempts_the_sparse_fallback() {
        let changed=Arc::new(tokio::sync::Notify::new());
        let notifications=arm_observation_changes(&[changed.clone()]);
        changed.notify_waiters();
        wait_for_observation_change(notifications,&CancellationToken::new()).await.unwrap();

        let cancellation=CancellationToken::new();
        let notifications=arm_observation_changes(&[changed.clone()]);
        let wait=wait_for_observation_change(notifications,&cancellation);tokio::pin!(wait);
        assert!(wait.as_mut().now_or_never().is_none());
        tokio::time::advance(std::time::Duration::from_secs(29)).await;
        assert!(wait.as_mut().now_or_never().is_none(),"waiting must not schedule one-second observation rebuilds");
        cancellation.cancel();
        assert!(wait.await.is_err(),"Stop does not wait for the fallback or a child result");

        let notifications=arm_observation_changes(&[changed]);
        let cancellation=CancellationToken::new();
        let wait=wait_for_observation_change(notifications,&cancellation);tokio::pin!(wait);
        assert!(wait.as_mut().now_or_never().is_none());
        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        assert!(wait.await.is_ok(),"a lost hint still gets a bounded cursor-only fallback");
    }

    #[test]
    fn accepted_decision_outbox_allows_a_real_dispatch_owner_query_without_inventing_one() {
        assert!(!dispatchable_step("waiting_input",false),"an unanswered question does not make the watcher its delivery owner");
        assert!(dispatchable_step("waiting_input",true),"the exact accepted DecisionInput can be admitted by its live dispatcher after native capacity releases");
        for status in ["pending","queued","running","backoff_pending"] {assert!(dispatchable_step(status,false));}
        for status in ["completed","failed","cancelled","skipped"] {assert!(!dispatchable_step(status,true));}
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,false,true,false,false),WaitDisposition::DecisionBlocked,
            "a pending outbox with no actual owner still requires intervention");
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,false,false,true,false),WaitDisposition::Park,
            "an actual dispatcher can wait for capacity without waking the parent model");
    }

    #[test]
    fn canonical_question_input_wait_does_not_loan_progress_to_review_or_orphaned_siblings() {
        let question=json!({"status":"waiting_input","agent_session_id":"original-child","delivery_pending":false,
            "decision_input_boundary":{"agent_session_id":"original-child","turn_operation_id":"original-child-question",
                "question_event_id":"original-question-event","request_id":"original-question-request"}});
        let row=json!({"execution_id":"original-execution","status":"waiting_input","execution_attention":null,
            "step_observations":{"question":question,"completed":{"status":"completed","delivery_pending":false}}});
        assert!(external_decision_wait(std::slice::from_ref(&row)).unwrap(),
            "the verified native question retains its original Decision API and source input subscription");
        for status in ["waiting_input","running","queued","pending","backoff_pending"] {
            let mut mixed=row.clone();mixed["step_observations"]["unowned"]=json!({"status":status,"delivery_pending":false});
            assert!(!external_decision_wait(&[mixed]).unwrap(),"a question cannot hide an ownerless {status} sibling");
        }
        let mut pending=row.clone();pending["step_observations"]["completed"]["delivery_pending"]=json!(true);
        assert!(!external_decision_wait(&[pending]).unwrap(),"another unsettled delivery still needs its actual owner");
        let mut paused=row.clone();paused["execution_attention"]=json!({"status":"paused"});
        assert!(!external_decision_wait(&[paused]).unwrap(),"question routing cannot override an explicit aggregate pause");
        let mut no_question=row.clone();no_question["step_observations"]["question"]=json!({"status":"waiting_input","question":"a status string is not proof"});
        assert!(!external_decision_wait(&[no_question]).unwrap());
        let mut wrong_source=row.clone();wrong_source["step_observations"]["question"]["agent_session_id"]=json!("foreign-child");
        assert!(external_decision_wait(&[wrong_source]).is_err(),"the exact question cannot be borrowed by another child");
        let mut already_done=row;already_done["step_observations"]["question"]=json!({"status":"completed","delivery_pending":false});
        assert!(!external_decision_wait(&[already_done]).unwrap(),"an accepted answer or completion is no longer the unanswered external boundary");
    }

    #[test]
    fn child_owner_restart_keeps_the_original_parent_subscription_without_model_polling() {
        let pause=json!({"agent_session_id":"paused-child","turn_operation_id":"original-child-turn",
            "boundary":{"kind":"native_pause","revision":1,"reason":"EXECUTION_INTERVENTION_REQUIRED",
                "checkpoint_revision":16,"execution_fence":1,"cleanup_proven":true}});
        let row=json!({"status":"waiting_input","execution_attention":null,"step_observations":{
            "child":{"status":"waiting_input","agent_session_id":"paused-child","native_intervention":pause},
            "healthy":{"status":"completed","delivery_pending":false}}});
        assert!(external_decision_wait(std::slice::from_ref(&row)).unwrap());
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,true,false,true,true,false,true),WaitDisposition::Wake,
            "the parent receives this exact pause once and can explain or manage it");
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,true,true,false,true),WaitDisposition::Park,
            "an unchanged owner-control boundary retains the same wait, not another model call or parent Pause");
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,false,false,true,false),WaitDisposition::Park,
            "accepted Restart transfers progress to its real owner while the parent keeps its original Turn");
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,true,true,true,false,false,false),WaitDisposition::Wake,
            "the original parent wakes for the completed child and performs final delivery");
        let mut unsafe_pause=row.clone();
        unsafe_pause["step_observations"]["child"]["native_intervention"]["boundary"]["cleanup_proven"]=json!(false);
        assert!(!external_decision_wait(&[unsafe_pause]).unwrap(),"unknown cleanup never obtains an executable restart route");
        let mut orphan=row;orphan["step_observations"]["orphan"]=json!({"status":"running"});
        assert!(!external_decision_wait(&[orphan]).unwrap(),"one paused child cannot hide another task without an owner");
    }

    #[test]
    fn ordinary_dispatch_cannot_borrow_a_paused_observer_but_an_exact_restart_owner_can_progress() {
        let paused=json!({"status":"running","decision_input_pending":true,"native_intervention":{
            "agent_session_id":"child","turn_operation_id":"original","boundary":{"kind":"native_pause","cleanup_proven":false}}});
        assert!(!ordinary_dispatchable(&paused),"domain Running and an accepted input cannot make a technical-pause watcher a producer");
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,true,true,false,false),WaitDisposition::DecisionBlocked);
        assert_eq!(wait_disposition(AgentChildrenWaitMode::All,false,false,true,true,true,false),WaitDisposition::Park,
            "the exact tracked Restart owner can still settle cleanup without borrowing another Step's task");
        let mut runnable=paused;runnable["native_intervention"]=Value::Null;
        assert!(ordinary_dispatchable(&runnable));
    }

    #[test]
    fn terminal_task_old_native_pause_never_becomes_a_restart_boundary_or_breaks_pending_cleanup_read() {
        let old=json!({"status":"cancelled","agent_session_id":"cancelled-child","delivery_pending":true,
            "native_intervention":{"agent_session_id":"cancelled-child","turn_operation_id":"old-turn",
                "boundary":{"kind":"native_pause","revision":1,"checkpoint_revision":2,"cleanup_proven":true}}});
        let mut row=json!({"status":"running","execution_attention":null,"step_observations":{"old":old}});
        assert!(!external_decision_wait(std::slice::from_ref(&row)).unwrap(),"the original cleanup remains unresolved without advertising another Restart");
        row["step_observations"]["old"]["delivery_pending"]=json!(false);
        assert!(!external_decision_wait(std::slice::from_ref(&row)).unwrap(),"a completed cleanup has no external decision route");
        row["step_observations"]["question"]=json!({"status":"waiting_input","agent_session_id":"actual-question","delivery_pending":false,
            "decision_input_boundary":{"agent_session_id":"actual-question","turn_operation_id":"actual-question-turn",
                "question_event_id":"original-question-event","request_id":"original-question-request"}});
        assert!(external_decision_wait(&[row]).unwrap(),"an unrelated completed cleanup cannot hide a real original question");
    }

    #[test]
    fn current_review_proof_cannot_take_a_historical_attempt_question() {
        use nomifun_common::AgentExecutionEventKind::*;
        let step=uuid::Uuid::now_v7().to_string();let old=uuid::Uuid::now_v7().to_string();let current=uuid::Uuid::now_v7().to_string();
        let mut question=event(4,DecisionRequested,Some(&step),json!({"question":"old question","canonical_operation_id":"old-operation","terminal_event_id":"old-terminal"}));
        question.attempt_id=Some(old.clone());
        let mut review=event(7,AttemptChanged,Some(&step),json!({"reconciliation":"initial_turn_receipt","turn_operation_key":"current-original-key"}));
        review.attempt_id=Some(current.clone());
        let events=vec![question.clone(),review];
        assert_eq!(current_wait_proof(&events,&step,Some(&current),true).unwrap().sequence,7);
        assert!(current_wait_proof(&events,&step,Some(&current),false).is_none(),"a current unanswered question needs its own exact source Attempt");
        assert_eq!(current_wait_proof(&events,&step,Some(&old),false).unwrap().sequence,4);
        assert!(current_wait_proof(&[question],&step,Some(&current),true).is_none(),"missing current review fails closed rather than borrowing old completed input");
    }

    #[test]
    fn blocked_restart_receipt_survives_compaction_and_wakes_once_without_representing_the_same_pause() {
        use nomifun_common::AgentExecutionEventKind::*;
        let step=uuid::Uuid::now_v7().to_string();let attempt=uuid::Uuid::now_v7().to_string();
        let pause=json!({"agent_session_id":"original-child","turn_operation_id":"original-turn","boundary":{
            "kind":"native_pause","revision":3,"reason":"EXECUTION_INTERVENTION_REQUIRED","checkpoint_revision":16,"execution_fence":7,"cleanup_proven":true}});
        let before=native_pause_ready_digest(Some(&attempt),&pause,None).unwrap();
        let mut request=event(10,StepChanged,Some(&step),json!({"change":"restart_requested","operation_id":"original-restart"}));request.attempt_id=Some(attempt.clone());
        assert!(blocked_restart_receipt(std::slice::from_ref(&request),&step,Some(&attempt)).unwrap().is_none());
        assert_eq!(native_pause_ready_digest(Some(&attempt),&pause,None).unwrap(),before,
            "accepted Restart and later domain review do not manufacture another ready source");
        let mut blocked=event(11,StepChanged,Some(&step),json!({"change":"restart_resolved","operation_id":"original-restart",
            "disposition":"blocked","code":"NATIVE_RESTART_REQUIRES_RECONCILIATION","reason":"The original result needs reconciliation."}));blocked.attempt_id=Some(attempt.clone());
        let mut recheck=event(12,StepChanged,Some(&step),json!({"change":"recheck_requested","operation_id":"later-recheck"}));recheck.attempt_id=Some(attempt.clone());
        let steps=vec![nomifun_agent_execution::AgentExecutionCollaborationStep {step_id:step.clone(),title:"original".into(),kind:nomifun_common::ExecutionStepKind::Agent,
            status:ExecutionStepStatus::WaitingInput,version:4,superseded_in_revision:None}];
        let compact=compact_observation_proofs(vec![request,blocked,recheck],&steps,"waiting_input");
        let receipt=blocked_restart_receipt(&compact,&step,Some(&attempt)).unwrap().unwrap();assert_eq!(receipt["sequence"],11);
        let after=native_pause_ready_digest(Some(&attempt),&pause,Some(&receipt)).unwrap();assert_ne!(after,before);
        let cursor=|ready:&nomifun_agent_contracts::DigestHex,state:&str|AgentCollaborationSnapshot::observation_cursor(BTreeMap::from([
            ("execution".into(),nomifun_agent_runtime::AgentCollaborationObservation {state:state.into(),ready:ready.as_ref().into()})])).unwrap();
        let old=cursor(&before,&"a".repeat(64));let current=cursor(&after,&"b".repeat(64));
        let snapshot=AgentCollaborationSnapshot {cursor:current.clone(),result:StrictJsonValue(json!({}))};
        assert!(snapshot.has_new_ready_observation(Some(&old)).unwrap(),"one new immutable failure returns control to its original parent");
        assert!(!snapshot.has_new_ready_observation(Some(&current)).unwrap(),"consuming that exact failure does not poll the model again");
        assert!(blocked_restart_receipt(&compact,&step,Some("foreign-attempt")).unwrap().is_none());
    }

    #[test]
    fn compact_read_proofs_drop_obsolete_payloads_but_preserve_original_native_turns() {
        use nomifun_common::AgentExecutionEventKind::*;
        let step=uuid::Uuid::now_v7().to_string();let attempt=uuid::Uuid::now_v7().to_string();
        let session=uuid::Uuid::now_v7().to_string();
        let steps=vec![nomifun_agent_execution::AgentExecutionCollaborationStep {step_id:step.clone(),title:"original".into(),
            kind:nomifun_common::ExecutionStepKind::Agent,status:ExecutionStepStatus::Cancelled,version:1,superseded_in_revision:None}];
        let mut events=vec![event(1,PlanChanged,None,json!({"delegation_source":{"agent_session_id":"source"},
            "added_step_ids":[step],"full_plan":{"snapshot":"must not remain"}}))];
        for sequence in 2..200 {
            events.push(event(sequence,StepChanged,Some(&step),json!({"status":"running","progress_body":"must not remain"})));
        }
        let mut first=event(200,AttemptChanged,Some(&step),json!({"canonical_operation_id":format!("turn:user:owner:{session}:original-turn"),
            "terminal_event_id":"original-terminal","output_summary":"must not remain"}));first.attempt_id=Some(attempt.clone());events.push(first);
        let mut decision=event(201,DecisionAnswered,Some(&step),json!({"operation_id":"later-decision","answer":"must not remain"}));
        decision.attempt_id=Some(attempt.clone());events.push(decision);
        // A later source batch can reopen an aggregate without rewriting an old
        // cancelled Step or its original group cancellation proof.
        events.push(event(202,StatusChanged,None,json!({"status":"failed","detail":"must not remain"})));
        events.push(event(203,StatusChanged,None,json!({"status":"running"})));
        let proofs=compact_observation_proofs(events,&steps,"running");
        assert!(proofs.len()<=5);
        assert!(!serde_json::to_string(&proofs).unwrap().contains("must not remain"));
        assert_eq!(settlement_event(&proofs,&step,"cancelled").unwrap().sequence,202);
        let native=NativeAttemptReference {attempt_id:&attempt,conversation_id:Some(&session)};
        let keys=original_attempt_turn_keys(&proofs,"owner",&step,native,"current-turn").unwrap();
        assert!(keys.contains("original-turn")&&keys.contains("later-decision"));
        assert!(proofs.iter().any(|proof|proof.event_type==PlanChanged));
    }
    #[test]
    fn workspace_history_includes_initial_and_all_exact_decision_turns_of_each_attempt() {
        use nomifun_common::AgentExecutionEventKind::*;
        let attempt_id=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();let session=uuid::Uuid::now_v7().to_string();
        let attempt:nomifun_api_types::ExecutionAttempt=serde_json::from_value(json!({
            "attempt_id":attempt_id,"execution_id":uuid::Uuid::now_v7().to_string(),"step_id":step,"attempt_no":1,
            "participant_id":null,"conversation_id":session,"status":"cancelled","trigger_reason":"initial","effective_config":{},
            "question":null,"error":null,"output_summary":null,"output_files":[],"tokens":null,"retry_after":null,"runtime_state":null,
            "started_at":1,"finished_at":2,"version":3,"created_at":1,"updated_at":2,
        })).unwrap();
        let mut events=vec![event(2,DecisionAnswered,Some(&step),json!({"operation_id":"first-answer"})),
            event(3,StepChanged,Some(&step),json!({"turn_operation_key":"second-answer"})),
            event(4,AttemptChanged,Some(&step),json!({"canonical_operation_id":format!("turn:user:owner:{session}:third-answer"),"terminal_event_id":"terminal"})),
            event(5,StepChanged,Some(&step),json!({"operation_id":"agent-message-not-a-decision"}))];
        for event in &mut events {event.attempt_id=Some(attempt_id.clone());}
        let mut sibling=event(6,DecisionAnswered,Some(&step),json!({"operation_id":"unrelated-attempt"}));
        sibling.attempt_id=Some(uuid::Uuid::now_v7().to_string());events.push(sibling);
        let keys=original_attempt_turn_keys(&events,"owner",&step,&attempt,"current-answer").unwrap();
        assert_eq!(keys,BTreeSet::from([format!("{attempt_id}:initial-turn"),"first-answer".into(),
            "second-answer".into(),"third-answer".into(),"current-answer".into()]));
        events[2].payload["canonical_operation_id"]=json!("turn:user:owner:foreign-session:third-answer");
        assert!(original_attempt_turn_keys(&events,"owner",&step,&attempt,"current-answer").is_err());
    }

    #[tokio::test]
    async fn deletion_observation_is_exact_owned_revocation_and_never_restores_outputs_or_tools() {
        let database=nomifun_db::init_database_memory().await.unwrap();let pool=database.pool();
        let owner=uuid::Uuid::now_v7().to_string();let foreign=uuid::Uuid::now_v7().to_string();
        let id=uuid::Uuid::now_v7().to_string();let session=AgentSessionId::from("parent");
        let turns=BTreeSet::from([OperationId::from("original-turn")]);
        let source=json!({"agent_session_id":"parent","turn_operation_id":"original-turn","tool_operation_id":"tool","effect_id":"effect"});
        sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,event_sequence,deleted_at,created_at,updated_at) VALUES(?,?,'revoked child work','completed','fixed','automatic','automatic',?,1,2,1,2)")
            .bind(&id).bind(&owner).bind(json!({"source":source}).to_string()).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'deleted','user',?,'{\"deleted\":true}',2)")
            .bind(&id).bind(&owner).execute(pool).await.unwrap();
        let observation=deleted_observation(pool,&owner,&id,&session,&turns).await.unwrap().unwrap();
        assert_eq!(observation.row["status"],"completed","persisted state is not rewritten into a fabricated cancellation");
        assert_eq!(observation.row["deleted"],true);assert_eq!(observation.row["user_revoked"],true);
        assert_eq!(observation.row["outputs_available"],false);
        assert_eq!(observation.row["children"],json!([]));assert_eq!(observation.row["owned_step_ids"],json!([]));
        assert_eq!(observation.receipt["execution_event_sequence"],1);
        assert_eq!(observation.receipt["event_type"],"deleted");assert_eq!(observation.receipt["source"],source);
        assert!(deleted_observation(pool,&foreign,&id,&session,&turns).await.unwrap().is_none());
        assert!(deleted_observation(pool,&owner,&uuid::Uuid::now_v7().to_string(),&session,&turns).await.unwrap().is_none());
        assert!(deleted_observation(pool,&owner,&id,&"another-parent".into(),&turns).await.is_err());
        assert!(deleted_observation(pool,&owner,&id,&session,&BTreeSet::from(["another-turn".into()])).await.is_err());
        sqlx::query("UPDATE agent_execution_events SET payload='{\"deleted\":false}' WHERE execution_id=?")
            .bind(&id).execute(pool).await.unwrap();
        assert!(deleted_observation(pool,&owner,&id,&session,&turns).await.is_err());
        sqlx::query("UPDATE agent_execution_events SET payload='{\"deleted\":true}',event_type='status_changed' WHERE execution_id=?")
            .bind(&id).execute(pool).await.unwrap();
        assert!(deleted_observation(pool,&owner,&id,&session,&turns).await.is_err(),"a terminal-looking row does not fabricate a deletion event");
        let deleted:Option<i64>=sqlx::query_scalar("SELECT deleted_at FROM agent_executions WHERE execution_id=?")
            .bind(&id).fetch_one(pool).await.unwrap();assert_eq!(deleted,Some(2));
    }

    #[test]
    fn long_exact_results_page_without_loss_or_using_mutable_execution_sequence() {
        let body="页\"\n🦀".repeat(70_000);
        assert!(body.len()>480*1024);
        let original=json!({"attempt_id":"original-attempt","source":{"agent_session_id":"child",
            "turn_operation_id":"original-turn","terminal_event_id":"original-terminal"},"result":body});
        let mut first=original.clone();
        result_page(&mut first,None,&mut RESULT_BODY_BUDGET.clone()).unwrap();
        let mut rebuilt=first["result"].as_str().unwrap().to_owned();
        let source=first["result_page"]["source_digest"].as_str().unwrap().to_owned();
        let hash=first["result_page"]["sha256"].clone();
        let mut next=first["result_page"]["next_offset"].as_u64();
        while let Some(offset)=next {
            let mut child=original.clone();
            result_page(&mut child,Some(&nomifun_agent_runtime::AgentCollaborationResultPage {
                execution_id:"execution".into(),step_id:"step".into(),source_digest:source.clone(),offset,
            }),&mut RESULT_BODY_BUDGET.clone()).unwrap();
            assert_eq!(child["result_page"]["sha256"],hash);
            assert_eq!(child["result_page"]["offset"],offset);
            rebuilt.push_str(child["result"].as_str().unwrap());
            next=child["result_page"]["next_offset"].as_u64();
        }
        assert_eq!(rebuilt,body);
        assert_eq!(nomifun_agent_contracts::digest_bytes(rebuilt.as_bytes()).as_ref(),hash.as_str().unwrap());
        let mut changed=original.clone();changed["source"]["terminal_event_id"]=json!("another-terminal");
        assert!(result_page(&mut changed,Some(&nomifun_agent_runtime::AgentCollaborationResultPage {
            execution_id:"execution".into(),step_id:"step".into(),source_digest:source,offset:0,
        }),&mut RESULT_BODY_BUDGET.clone()).is_err());
    }

    #[test]
    fn exhausted_body_budget_is_explicit_unread_coverage_and_invalid_utf8_offset_is_rejected() {
        let original=json!({"attempt_id":"attempt","source":{"agent_session_id":"child",
            "turn_operation_id":"turn","terminal_event_id":"terminal"},"result":"🦀never silently consumed"});
        let mut child=original.clone();
        result_page(&mut child,None,&mut 0).unwrap();
        assert_eq!(child["result"],"");
        assert_eq!(child["result_page"]["next_offset"],0);
        assert!(result_page(&mut original.clone(),Some(&nomifun_agent_runtime::AgentCollaborationResultPage {
            execution_id:"execution".into(),step_id:"step".into(),
            source_digest:child["result_page"]["source_digest"].as_str().unwrap().into(),offset:1,
        }),&mut RESULT_BODY_BUDGET.clone()).is_err());
    }

    #[test]
    fn source_is_exact_turn_not_a_session_wide_grant() {
        let session=AgentSessionId::from("parent");
        let turns=BTreeSet::from([OperationId::from("original")]);
        let source=json!({"agent_session_id":"parent","turn_operation_id":"original","tool_operation_id":"tool","effect_id":"effect"});
        assert!(source_matches(&source,&session,&turns));
        let mut other=source.clone();other["turn_operation_id"]=json!("later");
        assert!(!source_matches(&other,&session,&turns));
        other=source.clone();other["agent_session_id"]=json!("another-session");
        assert!(!source_matches(&other,&session,&turns));
        assert!(!source_matches(&json!({"agent_session_id":"parent","turn_operation_id":"original"}),&session,&turns));
        let continued=BTreeSet::from([OperationId::from("reply"),OperationId::from("original")]);
        assert!(source_matches(&source,&session,&continued),"only separately verified reply ancestry adds the original task scope");
    }

    fn event(sequence:i64,kind:nomifun_common::AgentExecutionEventKind,step:Option<&str>,payload:Value)
        ->nomifun_api_types::AgentExecutionEvent {
        nomifun_api_types::AgentExecutionEvent {execution_id:"execution".into(),sequence,event_type:kind,
            step_id:step.map(str::to_owned),attempt_id:None,actor_type:nomifun_common::AgentExecutionActorType::System,
            actor_id:None,actor_conversation_id:None,actor_attempt_id:None,on_behalf_of_user_id:"owner".into(),payload,created_at:1}
    }

    #[test]
    fn exact_settlement_is_not_the_latest_aggregate_event_or_an_unrelated_attempt() {
        use nomifun_common::AgentExecutionEventKind::*;
        let events=vec![event(7,AttemptChanged,Some("child"),json!({"step_status":"completed",
            "canonical_operation_id":"original-operation","terminal_event_id":"original-terminal"})),
            event(8,AttemptChanged,Some("other"),json!({"step_status":"completed"})),
            event(9,StatusChanged,None,json!({"change":"lead_report_delivered"}))];
        let proof=settlement_event(&events,"child","completed").unwrap();
        assert_eq!(domain_receipt("execution","child","completed",proof)["execution_event_sequence"],7);
        assert_eq!(native_terminal(proof).unwrap(),Some(("original-operation","original-terminal")));
        assert!(settlement_event(&events,"missing","completed").is_none());
    }

    #[test]
    fn nested_ownership_omits_superseded_steps_and_does_not_borrow_another_source() {
        let source=json!({"agent_session_id":"parent","turn_operation_id":"original","tool_operation_id":"tool","effect_id":"effect"});
        let events=vec![event(1,nomifun_common::AgentExecutionEventKind::PlanChanged,None,
            json!({"delegation_source":source,"added_step_ids":["superseded","current"]}))];
        let current=BTreeSet::from(["current".to_owned(),"unrelated".to_owned()]);
        let turns=BTreeSet::from([OperationId::from("original")]);
        assert_eq!(owned_steps(&json!({}),&events,&current,&AgentSessionId::from("parent"),&turns).unwrap(),
            BTreeSet::from(["current".to_owned()]));
        assert!(owned_steps(&json!({}),&events,&current,&AgentSessionId::from("other"),&turns).is_err());
    }

    #[test]
    fn adoption_and_question_cursors_use_their_original_domain_observations() {
        use nomifun_common::AgentExecutionEventKind::*;
        let events=vec![event(3,AttemptChanged,Some("child"),json!({"step_status":"failed"})),
            event(4,AttemptChanged,Some("child"),json!({"change":"output_adopted",
                "canonical_operation_id":"adopted-operation","terminal_event_id":"adopted-terminal"})),
            event(5,DecisionRequested,Some("question"),json!({"question":"exact question"})),
            event(6,StatusChanged,None,json!({"status":"waiting_input"}))];
        assert_eq!(settlement_event(&events,"child","completed").unwrap().sequence,4);
        assert_eq!(settlement_event(&events,"question","waiting_input").unwrap().sequence,5);
        assert!(native_terminal(&event(7,AttemptChanged,Some("child"),json!({"canonical_operation_id":"half"}))).is_err());
    }
}
