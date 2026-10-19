//! Durable Session input, before execution admission. Pending work is derived
//! from the original accepted Event and its unique Turn/steering reference.
use super::*;

pub fn session_input_namespace(owner:&PrincipalRef,session:&AgentSessionId)->String {
    format!("session-input:{}:{}:{}:",owner.principal_kind,owner.principal_id,session.as_ref())
}

fn command_key(owner:&PrincipalRef,session:&AgentSessionId,key:&IdempotencyKey)->Result<IdempotencyKey,SessionStoreError> {
    if key.as_ref().trim().is_empty() || key.as_ref().len()>1024 {return Err(SessionStoreError::InvalidEvent("Session input requires a bounded command identity".into()));}
    Ok(format!("{}{}",session_input_namespace(owner,session),digest_payload(&(owner,key.as_ref()))?.as_ref()).into())
}

pub fn is_session_input_event(event:&SessionEventRecord,owner:&PrincipalRef)->bool {
    let prefix=session_input_namespace(owner,&event.agent_session_id);
    event.kind.0=="message/user-accepted" && event.kind_version==1 && event.producer_id.as_ref()=="session_api"
        && event.correlation_id.as_ref()==event.event_id.as_ref()
        && event.idempotency_key.as_ref().strip_prefix(&prefix).is_some_and(|digest|digest.len()==64
            && digest.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)))
}

#[derive(Clone,Debug)]
pub struct PendingSessionInput {
    pub event:SessionEventRecord,
    pub input:StrictJsonValue,
}

#[derive(Clone,Debug)]
pub struct SessionInputClaim {
    pub operation_id:OperationId,
    pub source_message_id:EventId,
    pub claim_event_id:EventId,
    pub steering:bool,
}

/// Resolve a steering reference without rewriting its immutable payload or
/// copying accepted content into another fact. Both current exact controls
/// and unified Session input retain their original source identity.
pub fn canonical_steering_input(facts:&ChatCausalityFacts,event:&SessionEventRecord)
    ->Result<(EventId,StrictJsonValue),SessionStoreError> {
    if event.agent_session_id!=facts.session.agent_session_id || event.kind.0!="turn/steer-accepted" {
        return Err(SessionStoreError::InvalidEvent("steering input does not belong to its canonical Session".into()));
    }
    let payload=facts.event_payloads.get(event.event_id.as_ref())
        .ok_or_else(||SessionStoreError::InvalidPayload("steering input payload is missing".into()))?;
    if payload.get("target_operation_id").and_then(Value::as_str)!=Some(event.correlation_id.as_ref()) {
        return Err(SessionStoreError::InvalidEvent("steering input target differs from its exact Turn".into()));
    }
    if event.kind_version==1 {
        return Ok((event.event_id.clone(),StrictJsonValue(payload.get("input").cloned()
            .ok_or_else(||SessionStoreError::InvalidPayload("exact steering input is missing".into()))?)));
    }
    if event.kind_version!=2 || payload.get("input").is_some() || event.producer_id.as_ref()!="session_api" {
        return Err(SessionStoreError::InvalidPayload("steering input reference has an invalid version or content mirror".into()));
    }
    let id=payload.get("input_event_id").and_then(Value::as_str)
        .ok_or_else(||SessionStoreError::InvalidPayload("steering input reference is missing".into()))?;
    if event.causation_event_id.as_ref().map(|cause|cause.as_ref())!=Some(id) {
        return Err(SessionStoreError::InvalidEvent("steering input causation differs from its accepted source".into()));
    }
    let original=facts.events.iter().find(|original|original.event_id.as_ref()==id)
        .ok_or_else(||SessionStoreError::InvalidEvent("original accepted input fact is missing".into()))?;
    if original.agent_session_id!=event.agent_session_id || original.seq>=event.seq || !is_session_input_event(original,&facts.session.owner_ref) {
        return Err(SessionStoreError::InvalidEvent("steering input reference has invalid accepted provenance".into()));
    }
    let input=facts.event_payloads.get(id).cloned().ok_or_else(||SessionStoreError::InvalidPayload("accepted input payload is missing".into()))?;
    Ok((original.event_id.clone(),StrictJsonValue(input)))
}

#[derive(Clone,Copy,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="SCREAMING_SNAKE_CASE")]
pub enum SessionInputBlockedReason { AttachmentPreparationFailed,SkillUnavailable,InputBudgetExceeded,ContextPreparationFailed,CancelledByOwner }

impl SessionInputBlockedReason {
    fn identity(self)->&'static str {match self {
        Self::AttachmentPreparationFailed=>"attachment_preparation_failed",
        Self::SkillUnavailable=>"skill_unavailable",Self::InputBudgetExceeded=>"input_budget_exceeded",Self::ContextPreparationFailed=>"context_preparation_failed",
        Self::CancelledByOwner=>"cancelled_by_owner",
    }}
}

pub(super) async fn accepted_input_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId)
    ->Result<PendingSessionInput,SessionStoreError> {
    let event=event_by_event_id_tx(tx,input.as_ref()).await?.map(event_from_row).transpose()?
        .ok_or_else(||SessionStoreError::NotFound("accepted Session input".into()))?;
    let owner=live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref;
    if event.agent_session_id!=*session || !is_session_input_event(&event,&owner) {
        return Err(SessionStoreError::InvalidEvent("Session input identity differs from its accepted fact".into()));
    }
    let input=StrictJsonValue(payload_value_for_event_tx(tx,&event).await?);
    Ok(PendingSessionInput {event,input})
}

/// Cancellation terminates this accepted input, unlike a repairable preparation
/// failure. Read its exact original fact; a later retry cannot revive it.
async fn cancelled_input_tx(tx:&mut Transaction<'_,Sqlite>,accepted:&SessionEventRecord)
    ->Result<Option<SessionEventRecord>,SessionStoreError> {
    let identity=format!("session-input-blocked:{}:cancelled_by_owner",accepted.event_id.as_ref());
    let Some(row)=event_by_event_id_tx(tx,&identity).await? else {return Ok(None);};
    let event=event_from_row(row)?;
    let payload=payload_value_for_event_tx(tx,&event).await?;
    let reason:SessionInputBlockedReason=serde_json::from_value(payload.get("reason").cloned()
        .ok_or_else(||SessionStoreError::InvalidPayload("cancelled input reason is missing".into()))?)?;
    if event.agent_session_id!=accepted.agent_session_id || event.seq<=accepted.seq
        || event.kind.0!="message/input-blocked" || event.kind_version!=1
        || event.producer_id.as_ref()!="runtime_supervisor" || event.idempotency_key.as_ref()!=identity
        || event.causation_event_id.as_ref()!=Some(&accepted.event_id)
        || event.correlation_id.as_ref()!=accepted.event_id.as_ref()
        || payload.get("input_event_id").and_then(Value::as_str)!=Some(accepted.event_id.as_ref())
        || reason!=SessionInputBlockedReason::CancelledByOwner {
        return Err(SessionStoreError::InvalidEvent("input cancellation lost its exact accepted source".into()));
    }
    Ok(Some(event))
}

pub(super) async fn input_claimed_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId)->Result<bool,SessionStoreError> {
    Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_events started ON started.event_id=turn.started_event_id AND started.session_id=turn.session_id AND started.kind='turn/started' AND started.correlation_id=turn.operation_id WHERE turn.session_id=? AND (turn.source_message_id=? OR started.causation_event_id=?)) OR \
        EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads p ON p.payload_id=claim.payload_id AND p.session_id=claim.session_id WHERE claim.session_id=? AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
        AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=?)")
        .bind(session.as_ref()).bind(input.as_ref()).bind(input.as_ref()).bind(session.as_ref()).bind(input.as_ref()).fetch_one(&mut **tx).await?)
}

pub(super) async fn question_source_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,source:&str,seq:u64)
    ->Result<nomifun_agent_contracts::AgentUserInputRequest,SessionStoreError> {
    validate_uuidv7(source,"question Turn source")?;
    let operation:String=sqlx::query_scalar("SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=?")
        .bind(session.as_ref()).bind(source).fetch_optional(&mut **tx).await?
        .ok_or_else(||SessionStoreError::InvalidEvent("reply association has no original Turn".into()))?;
    let row=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND seq=?")
        .bind(session.as_ref()).bind(as_i64(seq,"question source sequence")?).fetch_optional(&mut **tx).await?
        .ok_or_else(||SessionStoreError::InvalidEvent("reply association has no committed question".into()))?;
    let event=event_from_row(row)?;let payload=payload_value_for_event_tx(tx,&event).await?;
    if event.kind.0!="runtime/progress-recorded" || event.kind_version!=1 || event.producer_id.as_ref()!="runtime_supervisor"
        || event.correlation_id.as_ref()!=operation || payload.pointer("/event/event").and_then(Value::as_str)!=Some("user_input_requested") {
        return Err(SessionStoreError::InvalidEvent("reply association differs from its exact question source".into()));
    }
    let request:nomifun_agent_contracts::AgentUserInputRequest=serde_json::from_value(payload.pointer("/event/request").cloned()
        .ok_or_else(||SessionStoreError::InvalidPayload("referenced question body is missing".into()))?)?;
    request.validate().map_err(SessionStoreError::InvalidPayload)?;Ok(request)
}

pub(super) async fn delivery_state_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId)
    ->Result<(Value,u64),SessionStoreError> {
    let(mut state,seq)=delivery_state_facts_tx(tx,session,input).await?;
    state["event_seq"]=json!(seq);Ok((state,seq))
}

async fn delivery_state_facts_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId)
    ->Result<(Value,u64),SessionStoreError> {
    let accepted=accepted_input_tx(tx,session,input).await?;
    let claims=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND ((e.kind='turn/started' AND e.causation_event_id=?) OR \
        (e.kind='turn/steer-accepted' AND e.kind_version=2 AND COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=?)) LIMIT 2")
        .bind(session.as_ref()).bind(input.as_ref()).bind(input.as_ref()).fetch_all(&mut **tx).await?;
    if claims.len()>1 {return Err(SessionStoreError::InvalidEvent("Session input has more than one consumption claim".into()));}
    if let Some(claim)=claims.into_iter().next() {
        let claim=event_from_row(claim)?;
        if claim.kind.0=="turn/started" {return Ok((json!({"state":"claimed"}),claim.seq));}
        let payload=payload_value_for_event_tx(tx,&claim).await?;
        if claim.producer_id.as_ref()!="session_api" || claim.causation_event_id.as_ref()!=Some(input)
            || payload.get("target_operation_id").and_then(Value::as_str)!=Some(claim.correlation_id.as_ref()) || payload.get("input").is_some() {
            return Err(SessionStoreError::InvalidEvent("input consumption claim differs from its original source and target".into()));
        }
        // Claiming a safe step is not delivery. Only the same native Turn's
        // exact receipt/source pair establishes that the original content is
        // retained in Runtime history. Shared-content hydration validates the
        // source and restores its body; no duplicate body or delivery ledger.
        let records=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
            FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id WHERE e.session_id=? \
            AND e.kind='runtime/progress-recorded' AND e.correlation_id=? AND e.seq>? AND \
            COALESCE(json_extract(e.inline_json,'$.event.event'),json_extract(CAST(p.body AS TEXT),'$.value.event.event')) IN ('steering_inputs','steering_deferred') ORDER BY e.seq")
            .bind(session.as_ref()).bind(claim.correlation_id.as_ref()).bind(as_i64(claim.seq,"input claim sequence")?).fetch_all(&mut **tx).await?;
        let mut deferred=None;
        for record in records {
            let record=event_from_row(record)?;let payload=payload_value_for_event_tx(tx,&record).await?;
            if record.kind_version!=1 || record.producer_id.as_ref()!="runtime_supervisor" {
                return Err(SessionStoreError::InvalidEvent("input delivery has no native Runtime provenance".into()));
            }
            let inputs=payload.pointer("/event/inputs").and_then(Value::as_array)
                .ok_or_else(||SessionStoreError::InvalidPayload("input delivery has no typed input records".into()))?;
            if inputs.len()>16 {return Err(SessionStoreError::InvalidPayload("input delivery exceeds its bounded input records".into()));}
            if let Some(delivered)=inputs.iter().find(|delivered|delivered.get("receipt_operation_id").and_then(Value::as_str)==Some(claim.event_id.as_ref())) {
                if delivered.get("message_id").and_then(Value::as_str)!=Some(input.as_ref())
                    || delivered.get("text")!=accepted.input.0.get("content") {
                    return Err(SessionStoreError::InvalidPayload("input delivery differs from its exact accepted source".into()));
                }
                if payload.pointer("/event/event").and_then(Value::as_str)==Some("steering_inputs") {
                    return Ok((json!({"state":"claimed"}),record.seq));
                }
                deferred=Some(record.seq);
            }
        }
        let terminal:Option<i64>=sqlx::query_scalar("SELECT terminal.seq FROM agent_turns turn JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id WHERE turn.session_id=? AND turn.operation_id=?")
            .bind(session.as_ref()).bind(claim.correlation_id.as_ref()).fetch_optional(&mut **tx).await?;
        if let Some(seq)=terminal {return Ok((json!({"state":"blocked","reason":"TURN_ENDED_BEFORE_INPUT_DELIVERY"}),as_u64(seq,"undelivered input terminal sequence")?));}
        if let Some(seq)=deferred {return Ok((json!({"state":"blocked","reason":"TURN_ENDED_BEFORE_INPUT_DELIVERY"}),seq));}
        return Ok((json!({"state":"pending"}),claim.seq));
    }
    let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    if accepted.event.seq<=as_u64(floor,"input context floor")? {
        return Ok((json!({"state":"discarded","reason":"CONTEXT_CLEARED"}),as_u64(floor,"input context floor")?));
    }
    if let Some(cancelled)=cancelled_input_tx(tx,&accepted.event).await? {
        return Ok((json!({"state":"cancelled","reason":SessionInputBlockedReason::CancelledByOwner}),cancelled.seq));
    }
    let blocked=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
        FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND e.kind='message/input-blocked' AND COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=? ORDER BY e.seq DESC LIMIT 1")
        .bind(session.as_ref()).bind(input.as_ref()).fetch_optional(&mut **tx).await?;
    if let Some(row)=blocked {
        let event=event_from_row(row)?;let payload=payload_value_for_event_tx(tx,&event).await?;
        let reason:SessionInputBlockedReason=serde_json::from_value(payload.get("reason").cloned()
            .ok_or_else(||SessionStoreError::InvalidPayload("input blocked reason is missing".into()))?)?;
        if event.kind_version!=1 || event.producer_id.as_ref()!="runtime_supervisor" || event.causation_event_id.as_ref()!=Some(input)
            || event.correlation_id.as_ref()!=input.as_ref() {return Err(SessionStoreError::InvalidEvent("input blocked fact lost its accepted source".into()));}
        return Ok((json!({"state":"blocked","reason":reason}),event.seq));
    }
    Ok((json!({"state":"pending"}),accepted.event.seq))
}

pub(super) async fn hydrate(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,messages:&mut [MessageProjection])
    ->Result<(),SessionStoreError> {
    for message in messages.iter_mut().filter(|row|row.presentation_intent=="message"&&row.projection.get("state").and_then(Value::as_str)==Some("accepted")) {
        let Some(id)=message.projection_id.strip_prefix("message:") else {continue};
        if Uuid::parse_str(id).ok().is_none_or(|id|id.get_version_num()!=7) {continue;}
        let Some(row)=event_by_event_id_tx(tx,id).await? else {return Err(SessionStoreError::InvalidEvent("accepted message has no original fact".into()));};
        let event=event_from_row(row)?;
        let owner=live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref;
        if !is_session_input_event(&event,&owner) {continue;}
        let(state,seq)=delivery_state_tx(tx,session,&event.event_id).await?;
        message.projection["input_delivery"]=state;message.last_seq=message.last_seq.max(seq);
        message.semantic_digest=digest_payload(&message.projection)?.0;
    }
    Ok(())
}

pub(super) async fn attach_updates_after(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,after:u64,head:u64,messages:&mut Vec<MessageProjection>)
    ->Result<(),SessionStoreError> {
    let mut ids:Vec<String>=sqlx::query_scalar("SELECT DISTINCT CASE WHEN e.kind='turn/started' THEN e.causation_event_id \
        ELSE COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id')) END FROM agent_events e \
        LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id WHERE e.session_id=? AND e.seq>? AND e.seq<=? \
        AND (e.kind IN ('turn/started','message/input-blocked') OR (e.kind='turn/steer-accepted' AND e.kind_version=2))")
        .bind(session.as_ref()).bind(as_i64(after,"input update cursor")?).bind(as_i64(head,"input update head")?).fetch_all(&mut **tx).await?;
    let owner=live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref;
    // Native delivery and terminal facts can change a previously claimed
    // input's visible state. Re-emit its original projection at the actual
    // changed sequence, without creating another message/body.
    ids.extend(sqlx::query_scalar::<_,String>("SELECT DISTINCT COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(cp.body AS TEXT),'$.value.input_event_id')) \
        FROM agent_events changed JOIN agent_events claim ON claim.session_id=changed.session_id AND claim.correlation_id=changed.correlation_id \
        LEFT JOIN agent_payloads cp ON cp.payload_id=claim.payload_id AND cp.session_id=claim.session_id \
        WHERE changed.session_id=? AND changed.seq>? AND changed.seq<=? AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
        AND changed.kind IN ('runtime/progress-recorded','turn/completed','turn/failed','turn/cancelled')")
        .bind(session.as_ref()).bind(as_i64(after,"input delivery update cursor")?).bind(as_i64(head,"input delivery update head")?).fetch_all(&mut **tx).await?);
    let cleared:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='context/cleared' AND seq>? AND seq<=?)")
        .bind(session.as_ref()).bind(as_i64(after,"clear update cursor")?).bind(as_i64(head,"clear update head")?).fetch_one(&mut **tx).await?;
    if cleared {
        let prefix=session_input_namespace(&owner,session);
        ids.extend(sqlx::query_scalar::<_,String>("SELECT event_id FROM agent_events WHERE session_id=? AND kind='message/user-accepted' AND producer_id='session_api' AND substr(idempotency_key,1,length(?))=?")
            .bind(session.as_ref()).bind(&prefix).bind(&prefix).fetch_all(&mut **tx).await?);
    }
    for id in ids {
        let Some(original)=event_by_event_id_tx(tx,&id).await? else {return Err(SessionStoreError::InvalidEvent("input update source is missing".into()));};
        if !is_session_input_event(&event_from_row(original)?,&owner) {continue;}
        let identity=format!("message:{id}");if messages.iter().any(|row|row.projection_id==identity) {continue;}
        let row=sqlx::query_as::<_,StoredProjectionRow>("SELECT session_id,projection_id,first_seq,last_seq,presentation_intent,projection_json,semantic_digest FROM agent_messages WHERE session_id=? AND projection_id=?")
            .bind(session.as_ref()).bind(identity).fetch_optional(&mut **tx).await?;
        let row=row.ok_or_else(||SessionStoreError::InvalidSession("accepted input projection is missing".into()))?;
        messages.push(projection_from_row(row)?);
    }
    hydrate(tx,session,messages).await?;
    messages.retain(|message|message.last_seq>after);
    Ok(())
}

impl AgentSessionStore {
    pub async fn session_input(&self,session:&AgentSessionId,input:&EventId)->Result<PendingSessionInput,SessionStoreError> {
        let mut tx=self.pool.begin().await?;require_live_session_tx(&mut tx,session.as_ref()).await?;
        let result=accepted_input_tx(&mut tx,session,input).await?;tx.commit().await?;Ok(result)
    }
    pub async fn session_input_delivery(&self,session:&AgentSessionId,input:&EventId)->Result<Value,SessionStoreError> {
        let mut tx=self.pool.begin().await?;require_live_session_tx(&mut tx,session.as_ref()).await?;
        let(state,_)=delivery_state_tx(&mut tx,session,input).await?;tx.commit().await?;Ok(state)
    }
    pub async fn question_source(&self,session:&AgentSessionId,source:&str,seq:u64)
        ->Result<nomifun_agent_contracts::AgentUserInputRequest,SessionStoreError> {
        let mut tx=self.pool.begin().await?;require_live_session_tx(&mut tx,session.as_ref()).await?;
        let request=question_source_tx(&mut tx,session,source,seq).await?;tx.commit().await?;Ok(request)
    }
    pub async fn session_input_by_key(&self,owner:&PrincipalRef,session:&AgentSessionId,producer:&EventProducerId,key:&IdempotencyKey)
        ->Result<Option<PendingSessionInput>,SessionStoreError> {
        let key=command_key(owner,session,key)?;
        let mut tx=self.pool.begin().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let Some(row)=event_by_producer_key_tx(&mut tx,producer.as_ref(),key.as_ref()).await? else {tx.commit().await?;return Ok(None);};
        let event=event_from_row(row)?;let input=accepted_input_tx(&mut tx,session,&event.event_id).await?;
        tx.commit().await?;Ok(Some(input))
    }

    pub async fn session_input_claim(&self,session:&AgentSessionId,input:&EventId)->Result<Option<SessionInputClaim>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;accepted_input_tx(&mut tx,session,input).await?;
        let turn:Option<(String,String)>=sqlx::query_as("SELECT operation_id,started_event_id FROM agent_turns WHERE session_id=? AND source_message_id=?")
            .bind(session.as_ref()).bind(input.as_ref()).fetch_optional(&mut *tx).await?;
        let rows=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
            FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
            WHERE e.session_id=? AND e.kind='turn/steer-accepted' AND e.kind_version=2 AND \
            COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=? LIMIT 2")
            .bind(session.as_ref()).bind(input.as_ref()).fetch_all(&mut *tx).await?;
        if rows.len()>1 || (turn.is_some()&&!rows.is_empty()) {return Err(SessionStoreError::InvalidEvent("Session input has more than one consumption claim".into()));}
        let claim=if let Some((operation,started))=turn {
            Some(SessionInputClaim {operation_id:operation.into(),source_message_id:input.clone(),claim_event_id:started.into(),steering:false})
        } else if let Some(row)=rows.into_iter().next() {
            let event=event_from_row(row)?;let payload=payload_value_for_event_tx(&mut tx,&event).await?;
            if event.producer_id.as_ref()!="session_api" || event.causation_event_id.as_ref()!=Some(input)
                || payload.get("target_operation_id").and_then(Value::as_str)!=Some(event.correlation_id.as_ref()) || payload.get("input").is_some() {
                return Err(SessionStoreError::InvalidEvent("Session input consumption claim is invalid".into()));
            }
            let source:String=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?")
                .bind(session.as_ref()).bind(event.correlation_id.as_ref()).fetch_one(&mut *tx).await?;
            Some(SessionInputClaim {operation_id:OperationId::from(event.correlation_id.as_ref()),source_message_id:source.into(),claim_event_id:event.event_id,steering:true})
        } else {None};
        tx.commit().await?;Ok(claim)
    }

    pub async fn steering_input_source(&self,session:&AgentSessionId,event_id:&EventId)
        ->Result<(SessionEventRecord,StrictJsonValue),SessionStoreError> {
        let mut tx=self.pool.begin().await?;require_live_session_tx(&mut tx,session.as_ref()).await?;
        let event=event_by_event_id_tx(&mut tx,event_id.as_ref()).await?.map(event_from_row).transpose()?
            .ok_or_else(||SessionStoreError::NotFound("steering input source".into()))?;
        if event.agent_session_id!=*session || event.kind.0!="turn/steer-accepted" {
            return Err(SessionStoreError::InvalidEvent("steering input source belongs to another Session or kind".into()));
        }
        let payload=payload_value_for_event_tx(&mut tx,&event).await?;
        if payload.get("target_operation_id").and_then(Value::as_str)!=Some(event.correlation_id.as_ref()) {
            return Err(SessionStoreError::InvalidEvent("steering input target differs from its exact Turn".into()));
        }
        let result=if event.kind_version==1 {
            (event,StrictJsonValue(payload.get("input").cloned().ok_or_else(||SessionStoreError::InvalidPayload("exact steering content is missing".into()))?))
        } else if event.kind_version==2 && payload.get("input").is_none() && event.producer_id.as_ref()=="session_api" {
            let id=payload.get("input_event_id").and_then(Value::as_str).ok_or_else(||SessionStoreError::InvalidPayload("steering input reference is missing".into()))?;
            if event.causation_event_id.as_ref().map(|cause|cause.as_ref())!=Some(id) {
                return Err(SessionStoreError::InvalidEvent("steering input causation differs from its accepted source".into()));
            }
            let original=accepted_input_tx(&mut tx,session,&id.into()).await?;
            if original.event.seq>=event.seq {return Err(SessionStoreError::InvalidEvent("steering input reference is not earlier than its claim".into()));}
            (original.event,original.input)
        } else {return Err(SessionStoreError::InvalidEvent("steering input source has an invalid version".into()));};
        tx.commit().await?;Ok(result)
    }

    pub async fn read_session_input_receipt(&self,owner:&PrincipalRef,session:&AgentSessionId,producer:&EventProducerId,
        key:&IdempotencyKey,input:&StrictJsonValue)->Result<Option<SessionEventAppendResult>,SessionStoreError> {
        let key=command_key(owner,session,key)?;
        let mut tx=self.pool.begin().await?;
        let live=live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if live.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let Some(row)=event_by_producer_key_tx(&mut tx,producer.as_ref(),key.as_ref()).await? else {tx.commit().await?;return Ok(None);};
        let record=event_from_row(row)?;
        let accepted=accepted_input_tx(&mut tx,session,&record.event_id).await?;
        if accepted.input!=*input {return Err(SessionStoreError::IdempotencyConflict("Session input key was reused with different content".into()));}
        let ack=event_ack(&record);tx.commit().await?;
        Ok(Some(SessionEventAppendResult {record:Some(record),ack:Some(ack.clone()),cursor:ack.cursor,persisted:true,duplicate:true}))
    }

    /// Saving user information never acquires execution, a native resume or
    /// a current Turn. Permanent request shape is validated before this call.
    pub async fn accept_session_input(&self,owner:&PrincipalRef,session:&AgentSessionId,producer:EventProducerId,
        key:IdempotencyKey,input:StrictJsonValue)->Result<SessionEventAppendResult,SessionStoreError> {
        self.accept_session_input_inner(owner,session,producer,key,input,false).await
    }

    pub async fn accept_initial_session_input(&self,owner:&PrincipalRef,session:&AgentSessionId,producer:EventProducerId,
        key:IdempotencyKey,input:StrictJsonValue)->Result<SessionEventAppendResult,SessionStoreError> {
        self.accept_session_input_inner(owner,session,producer,key,input,true).await
    }

    async fn accept_session_input_inner(&self,owner:&PrincipalRef,session:&AgentSessionId,producer:EventProducerId,
        key:IdempotencyKey,input:StrictJsonValue,initial_only:bool)->Result<SessionEventAppendResult,SessionStoreError> {
        if input.0.get("content").and_then(Value::as_str).is_none_or(|text|text.trim().is_empty()||text.len()>16*1024)
            || canonical_json_bytes(&input.0)?.len()>MAX_INLINE_JSON_BYTES || input.0.get("admission").is_some() {
            return Err(SessionStoreError::InvalidPayload("invalid permanent Session input envelope".into()));
        }
        let key=command_key(owner,session,&key)?;
        let mut tx=self.begin_write_transaction().await?;
        let live=live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if live.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(row)=event_by_producer_key_tx(&mut tx,producer.as_ref(),key.as_ref()).await? {
            let record=event_from_row(row)?;let accepted=accepted_input_tx(&mut tx,session,&record.event_id).await?;
            if accepted.input!=input {return Err(SessionStoreError::IdempotencyConflict("Session input key was reused with different content".into()));}
            let ack=event_ack(&record);tx.commit().await?;
            return Ok(SessionEventAppendResult {record:Some(record),ack:Some(ack.clone()),cursor:ack.cursor,persisted:true,duplicate:true});
        }
        if initial_only {
            let occupied:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=?) OR EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind LIKE 'message/%')")
                .bind(session.as_ref()).bind(session.as_ref()).fetch_one(&mut *tx).await?;
            if occupied {return Err(SessionStoreError::Conflict("initial-only Session input requires an empty message and Turn history".into()));}
        }
        if let Some(reply)=input.0.get("reply_to") {
            let source=reply.get("turn_id").and_then(Value::as_str).ok_or_else(||SessionStoreError::InvalidPayload("reply association requires its original Turn source".into()))?;
            let seq=reply.get("question_seq").and_then(Value::as_u64).filter(|seq|*seq>0).ok_or_else(||SessionStoreError::InvalidPayload("reply association requires its original question sequence".into()))?;
            if reply.as_object().is_none_or(|reply|reply.len()!=2) {return Err(SessionStoreError::InvalidPayload("reply association contains unknown fields".into()));}
            question_source_tx(&mut tx,session,source,seq).await?;
        }
        channel_input_observation::validate_input_source_tx(&mut tx,session,&input.0).await?;
        let cause=latest_turn_boundary_event_tx(&mut tx,session.as_ref()).await?;
        let id=new_event_id();
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:id.clone(),producer_id:producer,idempotency_key:key,
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind("message/user-accepted".into()),kind_version:1,
                correlation_id:CorrelationId::from(id.as_ref()),causation_event_id:Some(cause),payload:SessionEventPayloadRef::InlineJson(input)}};
        let result=self.append_event_tx(&mut tx,&event,None).await?;tx.commit().await?;Ok(result)
    }

    pub async fn pending_session_inputs(&self,session:&AgentSessionId,include_blocked:Option<&EventId>)
        ->Result<Vec<PendingSessionInput>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;let live=require_live_session_tx(&mut tx,session.as_ref()).await?;
        let prefix=session_input_namespace(&live.owner_ref,session);
        let ids:Vec<String>=sqlx::query_scalar("SELECT input.event_id FROM agent_events input WHERE input.session_id=? AND input.kind='message/user-accepted' \
            AND input.producer_id='session_api' AND input.kind_version=1 AND substr(input.idempotency_key,1,length(?))=? \
            AND input.seq>COALESCE((SELECT MAX(clear.seq) FROM agent_events clear WHERE clear.session_id=input.session_id AND clear.kind='context/cleared'),0) \
            AND NOT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_events started ON started.event_id=turn.started_event_id AND started.session_id=turn.session_id AND started.kind='turn/started' AND started.correlation_id=turn.operation_id WHERE turn.session_id=input.session_id AND (turn.source_message_id=input.event_id OR started.causation_event_id=input.event_id)) \
            AND NOT EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads cp ON cp.payload_id=claim.payload_id AND cp.session_id=claim.session_id WHERE claim.session_id=input.session_id AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
                AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(cp.body AS TEXT),'$.value.input_event_id'))=input.event_id) \
            AND (input.event_id=? OR NOT EXISTS(SELECT 1 FROM agent_events blocked LEFT JOIN agent_payloads bp ON bp.payload_id=blocked.payload_id AND bp.session_id=blocked.session_id WHERE blocked.session_id=input.session_id AND blocked.kind='message/input-blocked' \
                AND COALESCE(json_extract(blocked.inline_json,'$.input_event_id'),json_extract(CAST(bp.body AS TEXT),'$.value.input_event_id'))=input.event_id)) ORDER BY input.seq LIMIT 64")
            .bind(session.as_ref()).bind(&prefix).bind(&prefix).bind(include_blocked.map(|id|id.as_ref())).fetch_all(&mut *tx).await?;
        let mut inputs=Vec::new();for id in ids {
            let accepted=accepted_input_tx(&mut tx,session,&id.into()).await?;
            if cancelled_input_tx(&mut tx,&accepted.event).await?.is_none() {inputs.push(accepted);}
        }
        tx.commit().await?;Ok(inputs)
    }

    pub async fn claim_session_input_as_turn(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,operation:&OperationId,
        admission:StrictJsonValue)->Result<SessionEventAppendResult,SessionStoreError> {
        let mut tx=self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {return Err(SessionStoreError::ExecutionFenced);}
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if accepted.event.seq<=as_u64(floor,"input context floor")? {return Err(SessionStoreError::ExecutionFenced);}
        if input_claimed_tx(&mut tx,session,input).await? {return Err(SessionStoreError::Conflict("Session input already has its original claim".into()));}
        let head=head_by_id_tx(&mut tx,session.as_ref()).await?;
        if head.status!="ready" || head.active_turn_id.is_some() {return Err(SessionStoreError::Conflict("execution remains owned by the original Turn".into()));}
        let unsettled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if unsettled {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        let mut payload=json!({"operation_id":operation,"source_message_id":input,"admission":admission});
        for field in ["route_identity","resolved_snapshot_ref"] {if let Some(value)=admission.0.get(field) {payload[field]=value.clone();}}
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:new_event_id(),producer_id:"session_api".into(),
            idempotency_key:format!("session-input-claim:{}",input.as_ref()).into(),semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                kind:SessionEventKind("turn/started".into()),kind_version:1,correlation_id:CorrelationId::from(operation.as_ref()),
                causation_event_id:Some(accepted.event.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))}};
        let result=self.append_event_tx(&mut tx,&event,None).await?;tx.commit().await?;Ok(result)
    }

    pub async fn claim_session_input_as_steering(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,target:&OperationId)
        ->Result<SessionEventAppendResult,SessionStoreError> {
        let mut tx=self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {return Err(SessionStoreError::ExecutionFenced);}
        if accepted.input.0.get("reply_to").is_some() {return Err(SessionStoreError::ExecutionFenced);}
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if accepted.event.seq<=as_u64(floor,"input context floor")? {return Err(SessionStoreError::ExecutionFenced);}
        if input_claimed_tx(&mut tx,session,input).await? {return Err(SessionStoreError::Conflict("Session input already has its original claim".into()));}
        let head=head_by_id_tx(&mut tx,session.as_ref()).await?;
        if head.status!="running" || head.active_turn_id.as_deref()!=Some(target.as_ref()) {return Err(SessionStoreError::ExecutionFenced);}
        let(_,root)=Self::turn_accepted_input_tx(&mut tx,session,target).await?;
        let receiver=nomifun_agent_contracts::ChannelInputSource::from_input(&root.0).map_err(SessionStoreError::InvalidPayload)?;
        let incoming=nomifun_agent_contracts::ChannelInputSource::from_input(&accepted.input.0).map_err(SessionStoreError::InvalidPayload)?;
        if !nomifun_agent_contracts::ChannelInputSource::same_destination(receiver.as_ref(),incoming.as_ref()) {return Err(SessionStoreError::ExecutionFenced);}
        let unknown:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state='unknown')")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if unknown {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:new_event_id(),producer_id:"session_api".into(),
            idempotency_key:format!("session-input-claim:{}",input.as_ref()).into(),semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                kind:SessionEventKind("turn/steer-accepted".into()),kind_version:2,correlation_id:CorrelationId::from(target.as_ref()),
                causation_event_id:Some(input.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"input_event_id":input,"target_operation_id":target})))}};
        let result=self.append_event_tx(&mut tx,&event,None).await?;tx.commit().await?;Ok(result)
    }

    pub async fn mark_session_input_blocked(&self,session:&AgentSessionId,input:&EventId,reason:SessionInputBlockedReason)
        ->Result<(),SessionStoreError> {
        let mut tx=self.begin_write_transaction().await?;
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {tx.commit().await?;return Ok(());}
        if input_claimed_tx(&mut tx,session,input).await? {tx.commit().await?;return Ok(());}
        let identity=format!("session-input-blocked:{}:{}",input.as_ref(),reason.identity());
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:identity.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:identity.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind("message/input-blocked".into()),kind_version:1,
                correlation_id:CorrelationId::from(input.as_ref()),causation_event_id:Some(input.clone()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"input_event_id":input,"reason":reason})))}};
        self.append_event_tx(&mut tx,&event,None).await?;tx.commit().await?;Ok(())
    }
}
