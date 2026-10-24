//! Durable transfer of an exited native Turn's settlement responsibility.
//! This capability can finish already-admitted observations and cleanup only;
//! it never renews an execution lease, authorizes work or resumes a checkpoint.
use super::*;

const REQUESTED: &str = "runtime/native-settlement-requested";
const SETTLED: &str = "runtime/native-settlement-settled";
const TECHNICAL_PAUSE: &str = "EXECUTION_CLEANUP_UNPROVEN";
#[path = "native_settlement_bundle.rs"]
mod bundle;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTurnSettlementRequest {
    /// The host's typed run exit, encoded once. This is data, never replay input.
    pub outcome: StrictJsonValue,
    pub first_cleanup_failure: String,
}

/// Only the Store constructs this capability. Every use revalidates its saved
/// immutable request and exact producer/pause identity in the writer snapshot.
#[derive(Clone, Debug)]
pub struct NativeTurnSettlementReceipt {
    pub event_id: EventId,
    pub seq: u64,
    pub pause: Option<NativePauseState>,
    pub outcome: StrictJsonValue,
    session: AgentSessionId,
    operation: OperationId,
    generation: u64,
    fence: u64,
    holder: String,
    started: EventId,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    user_pause_request: Option<EventId>,
}

impl NativeTurnSettlementReceipt {
    pub fn session_id(&self) -> &AgentSessionId { &self.session }
    pub fn operation_id(&self) -> &OperationId { &self.operation }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn outcome(&self) -> &StrictJsonValue { &self.outcome }
}

#[derive(Clone, Debug)]
pub struct PendingNativeTurnSettlement {
    pub rowid: i64,
    pub owner: PrincipalRef,
    pub receipt: NativeTurnSettlementReceipt,
}

#[derive(Clone,Debug)]
pub struct NativeTurnSettlementBundleReceipt {
    pub runtime_seq: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedRequest {
    request: NativeTurnSettlementRequest,
    execution_generation: u64,
    execution_fence: u64,
    execution_owner: String,
    snapshot: nomifun_agent_contracts::ResolvedSnapshotRef,
    pause: Option<NativePauseState>,
    user_pause_request: Option<EventId>,
}

fn invalid(message: &str) -> SessionStoreError { SessionStoreError::InvalidEvent(message.into()) }

fn validate_request(request: &NativeTurnSettlementRequest) -> Result<(), SessionStoreError> {
    let outcome = &request.outcome.0;
    if outcome.get("model_steps").and_then(Value::as_u64).is_none_or(|step| step > u16::MAX as u64)
        || !matches!(outcome.pointer("/terminal/status").and_then(Value::as_str),
            Some("completed" | "input_required" | "cancelled" | "paused" | "failed"))
        || request.first_cleanup_failure.is_empty() || request.first_cleanup_failure.len() > 8192 {
        return Err(invalid("native settlement requires the bounded original typed run exit and first cleanup failure"));
    }
    Ok(())
}

fn request_identity(session: &AgentSessionId, operation: &OperationId, generation: u64) -> String {
    format!("native-settlement:{}:{}:{generation}", session.as_ref(), operation.as_ref())
}

fn encode_event_payload(session: &AgentSessionId, value: Value)
    -> Result<(SessionEventPayloadRef, Option<SessionPayloadRecord>), SessionStoreError> {
    let bytes = canonical_json_bytes(&value)?;
    if bytes.len() <= MAX_INLINE_JSON_BYTES { return Ok((SessionEventPayloadRef::InlineJson(StrictJsonValue(value)), None)); }
    if bytes.len() > MAX_SINGLE_PAYLOAD_BYTES { return Err(SessionStoreError::InvalidPayload("native settlement exceeds canonical Payload bound".into())); }
    let digest = digest_bytes(&bytes);
    let payload = SessionPayloadRecord { agent_session_id: session.clone(), payload_id: format!("native-settlement-payload:{}:{}",session.as_ref(),digest.as_ref()).into(),
        media_type: "application/json".into(), byte_len: bytes.len() as u64, digest, body: SessionPayloadBody::Json(StrictJsonValue(value)) };
    Ok((SessionEventPayloadRef::Stored(payload.payload_id.clone()), Some(payload)))
}

async fn receipt_tx(tx: &mut Transaction<'_, Sqlite>, event: SessionEventRecord)
    -> Result<NativeTurnSettlementReceipt, SessionStoreError> {
    if event.kind.0 != REQUESTED || event.kind_version != 1 || event.producer_id.as_ref() != "runtime_supervisor" {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let saved: SavedRequest = serde_json::from_value(payload_value_for_event_tx(tx, &event).await?)?;
    validate_request(&saved.request)?;
    let started = event.causation_event_id.clone().ok_or(SessionStoreError::ExecutionFenced)?;
    let start = event_by_event_id_tx(tx, started.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    let start_payload=payload_value_for_event_tx(tx,&start).await?;
    if start_payload.get("resolved_snapshot_ref")!=Some(&serde_json::to_value(&saved.snapshot)?) {return Err(SessionStoreError::ExecutionFenced);}
    let stopped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=? AND state='cancelled')")
        .bind(event.agent_session_id.as_ref()).bind(event.correlation_id.as_ref()).fetch_one(&mut **tx).await?;
    if let Some(pause)=saved.pause.as_ref() {
        let id=format!("native-paused:{}:{}:{}",event.agent_session_id.as_ref(),event.correlation_id.as_ref(),pause.revision);
        let fact=event_by_event_id_tx(tx,&id).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        if fact.kind.0!="turn/paused"||fact.causation_event_id.as_ref()!=Some(&started)||fact.seq>=event.seq
            ||payload_value_for_event_tx(tx,&fact).await?.get("pause")!=Some(&serde_json::to_value(pause)?) {return Err(SessionStoreError::ExecutionFenced);}
    }
    if let Some(request)=saved.user_pause_request.as_ref() {
        let request=event_by_event_id_tx(tx,request.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let payload=payload_value_for_event_tx(tx,&request).await?;
        let owner=live_session_by_id_tx(tx,event.agent_session_id.as_ref()).await?.owner_ref;
        if request.agent_session_id!=event.agent_session_id||request.correlation_id!=event.correlation_id||request.kind.0!="turn/pause-requested"
            ||request.kind_version!=1||request.producer_id.as_ref()!="runtime_supervisor"
            ||request.causation_event_id.as_ref()!=Some(&started)||request.seq>=event.seq||payload["owner_digest"]!=serde_json::to_value(digest_payload(&owner)?)?
            ||saved.pause.as_ref().is_none_or(|pause|pause.reason!="EXECUTION_USER_REQUESTED") {return Err(SessionStoreError::ExecutionFenced);}
    }
    if start.agent_session_id != event.agent_session_id || start.kind.0 != "turn/started"
        || start.correlation_id != event.correlation_id || start.seq > saved.execution_generation
        || event.event_id.as_ref() != request_identity(&event.agent_session_id, &OperationId::from(event.correlation_id.as_ref()), saved.execution_generation)
        || saved.pause.as_ref().is_some_and(|pause| pause.execution_fence != saved.execution_fence.saturating_add(1)
            || !((pause.reason == TECHNICAL_PAUSE && !pause.cleanup_proven)
                ||(saved.request.outcome.0.pointer("/terminal/status").and_then(Value::as_str)==Some("paused")
                    &&saved.request.outcome.0.pointer("/terminal/reason").and_then(Value::as_str)==Some(pause.reason.as_str())&&pause.cleanup_proven)
                ||(saved.user_pause_request.is_some()&&pause.reason=="EXECUTION_USER_REQUESTED")||stopped)) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    Ok(NativeTurnSettlementReceipt { event_id: event.event_id, seq: event.seq, pause: saved.pause, outcome: saved.request.outcome,
        session: event.agent_session_id, operation: event.correlation_id.as_ref().into(), generation: saved.execution_generation,
        fence: saved.execution_fence, holder: saved.execution_owner, started, snapshot: saved.snapshot,user_pause_request:saved.user_pause_request })
}

/// Validates an immutable capability without converting it into producer work
/// authority. A later pause/resume/producer generation fences it completely.
async fn current_tx(tx: &mut Transaction<'_, Sqlite>, receipt: &NativeTurnSettlementReceipt)
    -> Result<String, SessionStoreError> {
    let event = event_by_event_id_tx(tx, receipt.event_id.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    let saved = receipt_tx(tx, event).await?;
    if saved.session != receipt.session || saved.operation != receipt.operation || saved.generation != receipt.generation
        || saved.fence != receipt.fence || saved.holder != receipt.holder || saved.started != receipt.started
        || saved.snapshot != receipt.snapshot || saved.outcome != receipt.outcome
        || saved.user_pause_request!=receipt.user_pause_request
        || serde_json::to_value(&saved.pause)? != serde_json::to_value(&receipt.pause)? {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let session = live_session_by_id_tx(tx, receipt.session.as_ref()).await?;
    let (state, holder, generation, fence, pause): (String, Option<String>, i64, i64, Option<String>) = sqlx::query_as(
        "SELECT state,execution_owner,execution_generation,execution_fence,native_pause_json FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).fetch_one(&mut **tx).await?;
    if holder.as_deref() != Some(receipt.holder.as_str()) || as_u64(generation,"settlement generation")? != receipt.generation {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let expected_fence = receipt.pause.as_ref().map_or(receipt.fence, |pause| pause.execution_fence);
    if as_u64(fence,"settlement fence")? != expected_fence { return Err(SessionStoreError::ExecutionFenced); }
    if state == "running" {
        let actual: Option<NativePauseState> = pause.as_deref().map(serde_json::from_str).transpose()?;
        let head = head_by_id_tx(tx, receipt.session.as_ref()).await?;
        if session.agent_binding.resolved_snapshot_ref != receipt.snapshot || head.status != "paused" || head.active_turn_id.as_deref() != Some(receipt.operation.as_ref())
            || serde_json::to_value(&actual)? != serde_json::to_value(&receipt.pause)? || actual.is_none() {
            return Err(SessionStoreError::ExecutionFenced);
        }
    } else if !matches!(state.as_str(), "cancelled" | "completed" | "failed") { return Err(SessionStoreError::ExecutionFenced); }
    Ok(state)
}

async fn pending_user_pause_tx(tx:&mut Transaction<'_,Sqlite>,receipt:&NativeTurnSettlementReceipt)->Result<Option<EventId>,SessionStoreError> {
    let raw:Option<String>=sqlx::query_scalar("SELECT native_pause_requested_json FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).fetch_one(&mut **tx).await?;
    let Some(raw)=raw else {return Ok(None);};
    let row=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/pause-requested' ORDER BY seq DESC LIMIT 1")
        .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).fetch_one(&mut **tx).await?;
    let event=event_from_row(row)?;let value=payload_value_for_event_tx(tx,&event).await?;
    let owner=live_session_by_id_tx(tx,receipt.session.as_ref()).await?.owner_ref;
    if event.kind_version!=1||event.producer_id.as_ref()!="runtime_supervisor"||event.causation_event_id.as_ref()!=Some(&receipt.started)
        ||value!=serde_json::from_str::<Value>(&raw)?||value["owner_digest"]!=serde_json::to_value(digest_payload(&owner)?)? {return Err(SessionStoreError::ExecutionFenced);}
    Ok(Some(event.event_id))
}

async fn requires_user_pause_tx(tx:&mut Transaction<'_,Sqlite>,receipt:&NativeTurnSettlementReceipt)->Result<bool,SessionStoreError> {
    Ok(receipt.user_pause_request.is_some()||pending_user_pause_tx(tx,receipt).await?.is_some())
}

async fn cleanup_proven_tx(tx: &mut Transaction<'_, Sqlite>, receipt: &NativeTurnSettlementReceipt) -> Result<bool, SessionStoreError> {
    let rows: Vec<StoredEventRow> = sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='runtime/progress-recorded' AND producer_id='runtime_supervisor' AND seq>=? ORDER BY seq DESC")
        .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(as_i64(receipt.generation,"settlement generation")?).fetch_all(&mut **tx).await?;
    let mut later_work=false;
    for row in rows {
        let event = event_from_row(row)?;
        let payload=payload_value_for_event_tx(tx,&event).await?;
        let kind=payload.pointer("/event/event").and_then(Value::as_str);
        later_work|=matches!(kind,Some("model_step_started"|"tool_started"|"host_tool_dispatch"|"host_resource_dispatch"));
        if kind == Some("host_cleanup_proven") {
            let started = event_by_event_id_tx(tx, receipt.started.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
            return Ok(!later_work&&(event.causation_event_id.as_ref() == Some(&receipt.started) || event.causation_event_id == started.causation_event_id));
        }
    }
    Ok(false)
}

async fn original_cause_tx(tx:&mut Transaction<'_,Sqlite>,receipt:&NativeTurnSettlementReceipt,cause:Option<&EventId>)
    ->Result<bool,SessionStoreError> {
    let started=event_by_event_id_tx(tx,receipt.started.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    let mut next=cause.cloned();
    for _ in 0..128 {
        let Some(id)=next else {return Ok(false);};
        if id==receipt.started||Some(&id)==started.causation_event_id.as_ref() {return Ok(true);}
        let Some(event)=event_by_event_id_tx(tx,id.as_ref()).await?.map(event_from_row).transpose()? else {return Ok(false);};
        if event.agent_session_id!=receipt.session||event.seq<receipt.generation {return Ok(false);}
        next=event.causation_event_id;
    }
    Ok(false)
}

async fn owner_dispatch_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,operation:&OperationId,generation:u64,
    invocation:&str,call:&str)->Result<Option<Value>,SessionStoreError> {
    let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id WHERE e.session_id=? AND e.correlation_id=? AND e.kind='runtime/progress-recorded' AND e.producer_id='runtime_supervisor' AND e.seq>=? \
        AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event')) IN ('host_tool_dispatch','host_resource_dispatch') \
        AND COALESCE(json_extract(e.inline_json,'$.event.dispatch.operation_id'),json_extract(e.inline_json,'$.event.operation_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.dispatch.operation_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.operation_id'))=? ORDER BY e.seq DESC LIMIT 2")
        .bind(session.as_ref()).bind(operation.as_ref()).bind(as_i64(generation,"owner dispatch generation")?).bind(invocation).fetch_all(&mut **tx).await?;
    if rows.len()>1 {return Err(invalid("original invocation has ambiguous native dispatch facts"));}
    for row in rows {
        let event=event_from_row(row)?;
        let value=payload_value_for_event_tx(tx,&event).await?;
        let event=&value["event"];
        if matches!(event["event"].as_str(),Some("host_tool_dispatch"|"host_resource_dispatch")) {
            let dispatch=event.get("dispatch").unwrap_or(event);
            if dispatch["operation_id"].as_str()==Some(invocation)&&(call.is_empty()||dispatch["call_id"].as_str()==Some(call)) {return Ok(Some(dispatch.clone()));}
        }
    }
    Ok(None)
}

async fn original_proposal_step_tx(tx:&mut Transaction<'_,Sqlite>,receipt:&NativeTurnSettlementReceipt,call:&str)->Result<u64,SessionStoreError> {
    let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id WHERE e.session_id=? AND e.correlation_id=? AND e.kind='runtime/progress-recorded' AND e.producer_id='runtime_supervisor' AND e.seq>=? \
        AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='tool_call_completed' \
        AND COALESCE(json_extract(e.inline_json,'$.event.call.call_id'),json_extract(CAST(p.body AS TEXT),'$.value.event.call.call_id'))=? ORDER BY e.seq DESC LIMIT 2")
        .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(as_i64(receipt.generation,"proposal generation")?).bind(call).fetch_all(&mut **tx).await?;
    let [row]=rows.as_slice() else {return Err(SessionStoreError::ExecutionFenced);};
    payload_value_for_event_tx(tx,&event_from_row(row.clone())?).await?.pointer("/event/step").and_then(Value::as_u64).ok_or(SessionStoreError::ExecutionFenced)
}

fn terminal_matches(receipt: &NativeTurnSettlementReceipt, kind: &str, value: &Value, runtime: bool) -> bool {
    terminal_matches_outcome(&receipt.outcome.0,kind,value,runtime)
}

fn terminal_matches_outcome(outcome:&Value,kind:&str,value:&Value,runtime:bool)->bool {
    let terminal = &outcome["terminal"];
    let status = terminal["status"].as_str();
    let expected = match (status,runtime) {
        (Some("completed"),true)=>"turn_completed", (Some("input_required"),true)=>"turn_input_required",
        (Some("failed"),true)=>"turn_failed", (Some("cancelled"),true)=>"turn_cancelled", (Some("paused"),true)=>"turn_paused",
        (Some("completed"|"input_required"),false)=>"turn/completed", (Some("failed"),false)=>"turn/failed",
        (Some("cancelled"),false)=>"turn/cancelled", _=>return false,
    };
    if kind != expected || value.get("model_steps") != outcome.get("model_steps") { return false; }
    match status {
        Some("completed")=>value.get("finish_reason") == terminal.get("finish_reason"),
        Some("input_required")=>value.get(if runtime {"request"} else {"user_input_request"}) == terminal.get("request"),
        Some("failed")=>value.get("message") == terminal.get("message") && (!runtime || value.get("failure") == terminal.get("failure")),
        Some("paused")=>value.get("reason") == terminal.get("reason"),
        Some("cancelled")=>true,
        _=>false,
    }
}

impl AgentSessionStore {
    /// Read the original acknowledged control outcome after its writer reply
    /// was lost. This grants no execution or append permission and deliberately
    /// reads facts at that acknowledgement, never the Session's latest pause.
    pub async fn native_turn_settlement_user_pause_committed(&self,receipt:&NativeTurnSettlementReceipt)->Result<bool,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let request=event_by_event_id_tx(&mut tx,receipt.event_id.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let saved=receipt_tx(&mut tx,request).await?;
        if saved.session!=receipt.session||saved.operation!=receipt.operation||saved.generation!=receipt.generation
            ||saved.fence!=receipt.fence||saved.holder!=receipt.holder||saved.started!=receipt.started||saved.snapshot!=receipt.snapshot
            ||saved.outcome!=receipt.outcome||saved.user_pause_request!=receipt.user_pause_request
            ||serde_json::to_value(&saved.pause)?!=serde_json::to_value(&receipt.pause)? {return Err(SessionStoreError::ExecutionFenced);}
        let id=format!("{}:settled",receipt.event_id.as_ref());
        let Some(done)=event_by_event_id_tx(&mut tx,&id).await?.map(event_from_row).transpose()? else {tx.commit().await?;return Ok(false);};
        if done.agent_session_id!=receipt.session||done.correlation_id.as_ref()!=receipt.operation.as_ref()||done.kind.0!=SETTLED
            ||done.kind_version!=1||done.producer_id.as_ref()!="runtime_supervisor"||done.causation_event_id.as_ref()!=Some(&receipt.event_id)
            ||done.seq<=receipt.seq||payload_value_for_event_tx(&mut tx,&done).await?["execution_generation"]!=receipt.generation {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let pause=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/paused' AND seq>=? AND seq<? ORDER BY seq DESC LIMIT 1")
            .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(as_i64(receipt.generation,"settlement generation")?).bind(as_i64(done.seq,"settlement acknowledgement")?).fetch_optional(&mut *tx).await?;
        let Some(pause)=pause.map(event_from_row).transpose()? else {tx.commit().await?;return Ok(false);};
        let state:NativePauseState=serde_json::from_value(payload_value_for_event_tx(&mut tx,&pause).await?["pause"].clone())?;
        if pause.kind_version!=1||pause.producer_id.as_ref()!="runtime_supervisor"||pause.causation_event_id.as_ref()!=Some(&receipt.started)
            ||!state.cleanup_proven||state.reason!="EXECUTION_USER_REQUESTED" {tx.commit().await?;return Ok(false);}
        let request=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/pause-requested' AND seq>=? AND seq<? ORDER BY seq DESC LIMIT 1")
            .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(as_i64(receipt.generation,"settlement generation")?).bind(as_i64(pause.seq,"committed pause")?).fetch_optional(&mut *tx).await?;
        let Some(request)=request.map(event_from_row).transpose()? else {tx.commit().await?;return Ok(false);};
        let owner=live_session_by_id_tx(&mut tx,receipt.session.as_ref()).await?.owner_ref;
        let valid=request.kind_version==1&&request.producer_id.as_ref()=="runtime_supervisor"&&request.causation_event_id.as_ref()==Some(&receipt.started)
            &&payload_value_for_event_tx(&mut tx,&request).await?["owner_digest"]==serde_json::to_value(digest_payload(&owner)?)?;
        tx.commit().await?;Ok(valid)
    }

    pub async fn native_turn_settlement_requires_user_pause(&self,receipt:&NativeTurnSettlementReceipt)->Result<bool,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        current_tx(&mut tx,receipt).await?;
        let required=requires_user_pause_tx(&mut tx,receipt).await?;
        tx.commit().await?;Ok(required)
    }
    /// Resolve one ambiguous original write by its unchanged natural key.
    /// An absent dispatch/checkpoint is never appended through this reader.
    pub async fn confirm_native_settlement_event(&self,receipt:&NativeTurnSettlementReceipt,append:&SessionEventAppend,payload:Option<&SessionPayloadRecord>)
        ->Result<Option<SessionEventRecord>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        current_tx(&mut tx,receipt).await?;
        if append.agent_session_id!=receipt.session||append.producer_id.as_ref()!="runtime_supervisor"
            ||!original_cause_tx(&mut tx,receipt,append.semantic_event.causation_event_id.as_ref()).await? {return Err(SessionStoreError::ExecutionFenced);}
        let record=match shared_content::duplicate(&mut tx,append,payload).await? {
            Some(record)=>Some(record),None=>duplicate_event_tx(&mut tx,append).await?,
        }.map(event_from_row).transpose()?;
        tx.commit().await?;Ok(record)
    }

    pub async fn confirm_native_owner_event(&self,lease:&NativeExecutionLease,append:&SessionEventAppend,payload:Option<&SessionPayloadRecord>)
        ->Result<Option<SessionEventRecord>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        native_execution::check_native_identity_tx(&mut tx,lease).await?;
        if append.agent_session_id!=*lease.session_id()||append.producer_id.as_ref()!="runtime_supervisor" {return Err(SessionStoreError::ExecutionFenced);}
        let record=match shared_content::duplicate(&mut tx,append,payload).await? {
            Some(record)=>Some(record),None=>duplicate_event_tx(&mut tx,append).await?,
        }.map(event_from_row).transpose()?;
        tx.commit().await?;Ok(record)
    }

    pub async fn native_owner_dispatch_recorded(&self,lease:&NativeExecutionLease,invocation:&str,call:&str)->Result<bool,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let (holder,generation):(Option<String>,i64)=sqlx::query_as("SELECT execution_owner,execution_generation FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?;
        if holder.as_deref()!=Some(lease.holder())||as_u64(generation,"owner dispatch generation")?!=lease.generation() {return Err(SessionStoreError::ExecutionFenced);}
        let recorded=owner_dispatch_tx(&mut tx,lease.session_id(),lease.operation_id(),lease.generation(),invocation,call).await?.is_some();
        tx.commit().await?;Ok(recorded)
    }

    pub async fn request_native_turn_settlement(&self, lease: &NativeExecutionLease, request: &NativeTurnSettlementRequest)
        -> Result<NativeTurnSettlementReceipt, SessionStoreError> {
        validate_request(request)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let identity = request_identity(lease.session_id(), lease.operation_id(), lease.generation());
        if let Some(event) = event_by_event_id_tx(&mut tx,&identity).await?.map(event_from_row).transpose()? {
            let saved: SavedRequest = serde_json::from_value(payload_value_for_event_tx(&mut tx,&event).await?)?;
            if saved.request != *request { return Err(SessionStoreError::IdempotencyConflict("native settlement run exit or first cleanup failure changed".into())); }
            let receipt = receipt_tx(&mut tx,event).await?;
            db_write_observation.commit(tx).await?; return Ok(receipt);
        }
        let live = live_session_by_id_tx(&mut tx,lease.session_id().as_ref()).await?;
        let (started,generation,state,holder,fence,raw_pause,terminal):(String,i64,String,Option<String>,i64,Option<String>,Option<String>) = sqlx::query_as("SELECT started_event_id,execution_generation,state,execution_owner,execution_fence,native_pause_json,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?;
        if as_u64(generation,"settlement generation")? != lease.generation()||holder.as_deref()!=Some(lease.holder())
            ||!matches!(state.as_str(),"running"|"cancelled"|"completed"|"failed") { return Err(SessionStoreError::ExecutionFenced); }
        let existing_pause:Option<NativePauseState>=raw_pause.as_deref().map(serde_json::from_str).transpose()?;
        let start=event_by_event_id_tx(&mut tx,&started).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
        let snapshot:nomifun_agent_contracts::ResolvedSnapshotRef=serde_json::from_value(payload_value_for_event_tx(&mut tx,&start).await?
            .get("resolved_snapshot_ref").cloned().ok_or(SessionStoreError::ExecutionFenced)?)?;
        let current_fence=as_u64(fence,"settlement fence")?;
        let mut user_pause_request=None;
        let pause = if let Some(pause)=existing_pause {
            if state!="running"||current_fence!=lease.fence().saturating_add(1)||pause.execution_fence!=current_fence||!pause.cleanup_proven
                ||request.outcome.0.pointer("/terminal/status").and_then(Value::as_str)!=Some("paused")
                ||request.outcome.0.pointer("/terminal/reason").and_then(Value::as_str)!=Some(pause.reason.as_str()) {return Err(SessionStoreError::ExecutionFenced);}
            Some(pause)
        } else if state == "running" {
            if current_fence!=lease.fence() {return Err(SessionStoreError::ExecutionFenced);}
            let requested: Option<String> = sqlx::query_scalar("SELECT native_pause_requested_json FROM agent_turns WHERE session_id=? AND operation_id=?")
                .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?;
            if let Some(raw)=requested {
                let row=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/pause-requested' ORDER BY seq DESC LIMIT 1")
                    .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?;
                let event=event_from_row(row)?;let value=payload_value_for_event_tx(&mut tx,&event).await?;
                if event.causation_event_id.as_ref().map(EventId::as_ref)!=Some(started.as_str())||value!=serde_json::from_str::<Value>(&raw)?
                    ||value["owner_digest"]!=serde_json::to_value(digest_payload(&live.owner_ref)?)? {return Err(SessionStoreError::ExecutionFenced);}
                user_pause_request=Some(event.event_id);
            }
            Some(self.pause_native_execution_tx(&mut tx,lease.session_id(),lease.operation_id(),lease.fence(),
                if user_pause_request.is_some() {"EXECUTION_USER_REQUESTED"} else {TECHNICAL_PAUSE},false).await?)
        } else if state=="cancelled"&&current_fence==lease.fence().saturating_add(1) {
            let row=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/paused' ORDER BY seq DESC LIMIT 1")
                .bind(lease.session_id().as_ref()).bind(lease.operation_id().as_ref()).fetch_one(&mut *tx).await?;
            let fact=event_from_row(row)?;
            let pause:NativePauseState=serde_json::from_value(payload_value_for_event_tx(&mut tx,&fact).await?.get("pause").cloned().ok_or(SessionStoreError::ExecutionFenced)?)?;
            if pause.execution_fence!=current_fence||fact.causation_event_id.as_ref().map(EventId::as_ref)!=Some(started.as_str())
                ||fact.seq<lease.generation() {return Err(SessionStoreError::ExecutionFenced);}
            Some(pause)
        } else {
            if current_fence!=lease.fence() {return Err(SessionStoreError::ExecutionFenced);}
            if matches!(state.as_str(),"completed"|"failed") {
                let event=event_by_event_id_tx(&mut tx,terminal.as_deref().ok_or(SessionStoreError::ExecutionFenced)?).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
                let payload=payload_value_for_event_tx(&mut tx,&event).await?;
                if !terminal_matches_outcome(&request.outcome.0,&event.kind.0,&payload,false) {return Err(SessionStoreError::ExecutionFenced);}
            }
            None
        };
        let saved = SavedRequest { request:request.clone(),execution_generation:lease.generation(),execution_fence:lease.fence(),
            execution_owner:lease.holder().into(),snapshot,pause,user_pause_request };
        let (reference,payload) = encode_event_payload(lease.session_id(),serde_json::to_value(&saved)?)?;
        let mut append = native_pause::native_control_event(lease.session_id(),lease.operation_id(),identity,REQUESTED,started.into(),json!({}));
        append.semantic_event.payload = reference;
        let result = self.append_event_tx(&mut tx,&append,payload.as_ref()).await?;
        let receipt = receipt_tx(&mut tx,result.record.ok_or_else(||invalid("native settlement request was not persisted"))?).await?;
        db_write_observation.commit(tx).await?; Ok(receipt)
    }

    pub async fn read_native_turn_settlement(&self, owner: &PrincipalRef, session: &AgentSessionId, operation: &OperationId)
        -> Result<Option<NativeTurnSettlementReceipt>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let row = sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND correlation_id=? AND kind=? ORDER BY seq DESC LIMIT 1")
            .bind(session.as_ref()).bind(operation.as_ref()).bind(REQUESTED).fetch_optional(&mut *tx).await?;
        let receipt = match row { Some(row)=>Some(receipt_tx(&mut tx,event_from_row(row)?).await?),None=>None };
        tx.commit().await?; Ok(receipt)
    }

    pub async fn pending_native_turn_settlements(&self, after_rowid: i64, limit: u32)
        -> Result<Vec<PendingNativeTurnSettlement>, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let rows:Vec<(i64,String,String)> = sqlx::query_as("SELECT e.rowid,e.event_id,s.owner_ref_json FROM agent_events e JOIN agent_sessions s ON s.agent_session_id=e.session_id WHERE e.rowid>? AND e.kind=? AND s.state='live' AND NOT EXISTS(SELECT 1 FROM agent_events done WHERE done.session_id=e.session_id AND done.kind=? AND done.causation_event_id=e.event_id) ORDER BY e.rowid LIMIT ?")
            .bind(after_rowid).bind(REQUESTED).bind(SETTLED).bind(limit.clamp(1,500) as i64).fetch_all(&mut *tx).await?;
        let mut pending = Vec::with_capacity(rows.len());
        for (rowid,id,owner) in rows {
            let event = event_by_event_id_tx(&mut tx,&id).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
            pending.push(PendingNativeTurnSettlement {rowid,owner:serde_json::from_str(&owner)?,receipt:receipt_tx(&mut tx,event).await?});
        }
        tx.commit().await?; Ok(pending)
    }

    pub async fn native_turn_settlement_current(&self, receipt: &NativeTurnSettlementReceipt) -> Result<bool, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let result = match current_tx(&mut tx,receipt).await { Ok(_)=>true,Err(SessionStoreError::ExecutionFenced)=>false,Err(error)=>return Err(error) };
        tx.commit().await?; Ok(result)
    }

    pub async fn native_turn_settlement_cleanup_proven(&self, receipt: &NativeTurnSettlementReceipt) -> Result<bool, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        current_tx(&mut tx,receipt).await?;
        let proven = cleanup_proven_tx(&mut tx,receipt).await?;
        tx.commit().await?; Ok(proven)
    }

    pub async fn append_native_settlement_observation(&self, receipt: &NativeTurnSettlementReceipt,
        append: &SessionEventAppend, payload: Option<&SessionPayloadRecord>) -> Result<SessionEventAppendResult, SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        current_tx(&mut tx,receipt).await?;
        if append.agent_session_id != receipt.session || append.producer_id.as_ref() != "runtime_supervisor" { return Err(SessionStoreError::ExecutionFenced); }
        let duplicate=match shared_content::duplicate(&mut tx,append,payload).await? {Some(row)=>Some(row),None=>duplicate_event_tx(&mut tx,append).await?};
        if let Some(record) = duplicate {
            let record=event_from_row(record)?;
            let ack=event_ack(&record);db_write_observation.commit(tx).await?;return Ok(SessionEventAppendResult {cursor:ack.cursor.clone(),record:Some(record),ack:Some(ack),persisted:true,duplicate:true});
        }
        if append.semantic_event.kind_version!=1||!original_cause_tx(&mut tx,receipt,append.semantic_event.causation_event_id.as_ref()).await? {
            return Err(SessionStoreError::ExecutionFenced);
        }
        let value = match &append.semantic_event.payload {
            SessionEventPayloadRef::InlineJson(value)=>value.0.clone(),
            SessionEventPayloadRef::Stored(_)=>match payload.map(|value|&value.body) {Some(SessionPayloadBody::Json(value))=>value.0.clone(),_=>return Err(invalid("settlement Payload is missing"))},
            _=>return Err(invalid("settlement observation needs JSON")),
        };
        match append.semantic_event.kind.0.as_str() {
            "runtime/progress-recorded"=>{
                if append.semantic_event.correlation_id.as_ref()!=receipt.operation.as_ref() {return Err(SessionStoreError::ExecutionFenced);}
                let event=&value["event"];
                let kind=event["event"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
                let allowed=matches!(kind,"host_tool_settled"|"host_resource_settled"|"host_process_quiescent"|"host_cleanup_proven"
                    |"tool_completed"|"tool_outcome_reconciled"|"tool_results_ordered"|"work_status"|"plan_updated"|"completion_observation"
                    |"output_text_delta"|"reasoning_delta"|"completion_delivered"|"patch_recovery_updated");
                if !allowed { return Err(SessionStoreError::ExecutionFenced); }
                match kind {
                    "host_tool_settled"|"host_resource_settled"=>{
                        let invocation=event["operation_id"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
                        let call=event["call_id"].as_str().unwrap_or("");
                        if (kind=="host_tool_settled"&&call.is_empty())||owner_dispatch_tx(&mut tx,&receipt.session,&receipt.operation,receipt.generation,invocation,call).await?.is_none() {return Err(SessionStoreError::ExecutionFenced);}
                    }
                    "tool_completed"|"tool_outcome_reconciled"=>{
                        let call=event.pointer("/result/call_id").and_then(Value::as_str).ok_or(SessionStoreError::ExecutionFenced)?;
                        if event["step"].as_u64()!=Some(original_proposal_step_tx(&mut tx,receipt,call).await?) {return Err(SessionStoreError::ExecutionFenced);}
                    }
                    "tool_results_ordered"=>{
                        let calls=event["call_ids"].as_array().ok_or(SessionStoreError::ExecutionFenced)?;
                        if calls.len()>64 {return Err(SessionStoreError::ExecutionFenced);}
                        for call in calls {
                            if event["step"].as_u64()!=Some(original_proposal_step_tx(&mut tx,receipt,call.as_str().ok_or(SessionStoreError::ExecutionFenced)?).await?) {return Err(SessionStoreError::ExecutionFenced);}
                        }
                    }
                    "output_text_delta"|"reasoning_delta"|"completion_delivered"=>{
                        let step=event["step"].as_u64().ok_or(SessionStoreError::ExecutionFenced)?;
                        let admitted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id WHERE e.session_id=? AND e.correlation_id=? AND e.kind='runtime/progress-recorded' AND e.producer_id='runtime_supervisor' AND e.seq>=? AND COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event'))='model_step_started' AND COALESCE(json_extract(e.inline_json,'$.event.step'),json_extract(CAST(p.body AS TEXT),'$.value.event.step'))=?)")
                            .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(as_i64(receipt.generation,"original model generation")?).bind(as_i64(step,"original model step")?).fetch_one(&mut *tx).await?;
                        if !admitted {return Err(SessionStoreError::ExecutionFenced);}
                    }
                    _=>{},
                }
                let started=event_by_event_id_tx(&mut tx,receipt.started.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
                if append.semantic_event.causation_event_id.as_ref()!=Some(&receipt.started)
                    &&append.semantic_event.causation_event_id!=started.causation_event_id {return Err(SessionStoreError::ExecutionFenced);}
            }
            "tool/call-started"=>{
                // Materialize only the projection of an admission already
                // committed by the old producer; this grants no dispatch.
                let invocation=value["operation_id"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
                let call=value["call_id"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
                let dispatch=owner_dispatch_tx(&mut tx,&receipt.session,&receipt.operation,receipt.generation,invocation,call).await?.ok_or(SessionStoreError::ExecutionFenced)?;
                if append.semantic_event.causation_event_id.as_ref()!=Some(&receipt.started)
                    ||["capability_id","action_id"].iter().any(|field|dispatch.get(*field).is_some()&&dispatch.get(*field)!=value.get(*field)) {return Err(SessionStoreError::ExecutionFenced);}
            }
            "tool/result-recorded"=>{
                let cause=append.semantic_event.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
                let call=event_by_event_id_tx(&mut tx,cause.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
                let input=payload_value_for_event_tx(&mut tx,&call).await?;
                if call.kind.0!="tool/call-started"||call.causation_event_id.as_ref()!=Some(&receipt.started)
                    ||call.correlation_id!=append.semantic_event.correlation_id||input.get("operation_id")!=value.get("operation_id") {
                    return Err(SessionStoreError::ExecutionFenced);
                }
            }
            "message/completed"=>{}
            "message/content-part"|"thinking/content-part"=>{
                let start=event_by_event_id_tx(&mut tx,receipt.started.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
                if value.get("turn_id").and_then(Value::as_str)!=start.causation_event_id.as_ref().map(EventId::as_ref) {return Err(SessionStoreError::ExecutionFenced);}
            }
            nomifun_agent_contracts::TURN_WORKSPACE_FILES_OBSERVED=>{
                let observation:nomifun_agent_contracts::TurnWorkspaceFilesObservedV1=serde_json::from_value(value)?;
                observation.validate().map_err(SessionStoreError::InvalidPayload)?;
                if observation.operation_id!=receipt.operation || observation.execution_generation!=receipt.generation {return Err(SessionStoreError::ExecutionFenced);}
            }
            _=>return Err(SessionStoreError::ExecutionFenced),
        }
        let result=self.append_event_tx(&mut tx,append,payload).await?;
        db_write_observation.commit(tx).await?;Ok(result)
    }

    pub async fn complete_native_turn_settlement(&self,receipt:&NativeTurnSettlementReceipt)->Result<(),SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let identity=format!("{}:settled",receipt.event_id.as_ref());
        if let Some(done)=event_by_event_id_tx(&mut tx,&identity).await?.map(event_from_row).transpose()? {
            if done.agent_session_id!=receipt.session||done.causation_event_id.as_ref()!=Some(&receipt.event_id) {return Err(SessionStoreError::ExecutionFenced);}
            db_write_observation.commit(tx).await?;return Ok(());
        }
        let state=current_tx(&mut tx,receipt).await?;
        if !cleanup_proven_tx(&mut tx,receipt).await?||!matches!(state.as_str(),"cancelled"|"completed"|"failed") {
            return Err(SessionStoreError::Conflict("original Turn settlement or exact cleanup remains unproven".into()));
        }
        self.append_event_tx(&mut tx,&native_pause::native_control_event(&receipt.session,&receipt.operation,identity,SETTLED,receipt.event_id.clone(),json!({"execution_generation":receipt.generation})),None).await?;
        db_write_observation.commit(tx).await?;Ok(())
    }
}
