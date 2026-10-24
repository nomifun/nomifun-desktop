//! Durable Session input, before execution admission. Pending work is derived
//! from the original accepted Event and its unique Turn/steering reference.
use super::*;
use nomifun_agent_contracts::{SessionInputView,SessionInputState as InputState,SessionInputPlacement as Placement,SessionInputKind as InputKind};

pub fn session_input_namespace(owner:&PrincipalRef,session:&AgentSessionId)->String {
    format!("session-input:{}:{}:{}:",owner.principal_kind,owner.principal_id,session.as_ref())
}

pub(super) fn command_key(owner:&PrincipalRef,session:&AgentSessionId,key:&IdempotencyKey)->Result<IdempotencyKey,SessionStoreError> {
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
    session_input_queue::validate_delivery_envelope_tx(tx,session,&input.0).await?;
    Ok(PendingSessionInput {event,input})
}

/// Cancellation terminates this accepted input, unlike a repairable preparation
/// failure. Read its exact original fact; a later retry cannot revive it.
pub(super) async fn cancelled_input_tx(tx:&mut Transaction<'_,Sqlite>,accepted:&SessionEventRecord)
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
    let context=InputViewContext::load(tx,session).await?;
    let view=input_view_tx(tx,session,input,&context).await?;
    let seq=view.event_seq;Ok((serde_json::to_value(view)?,seq))
}

/// Shared transaction-scoped observations, never a persisted queue or an
/// execution grant. Batch readers fold queue controls and select the first
/// eligible root once, so a long queue does not hydrate itself per row.
pub(super) struct InputViewContext {
    pub(super) queue:session_input_queue::QueueState,
    observed_seq:u64,
    status:String,
    active_source:Option<String>,
    first:Option<EventId>,
    unsettled:bool,
}
impl InputViewContext {
    pub(super) async fn load(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId)->Result<Self,SessionStoreError> {
        let queue=session_input_queue::queue_state_tx(tx,session).await?;
        let head=head_by_id_tx(tx,session.as_ref()).await?;
        let active_source:Option<String>=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(head.active_turn_id.as_deref()).fetch_optional(&mut **tx).await?;
        let first=if active_source.is_none() && head.status=="ready" {
            session_input_queue::first_eligible_root_tx(tx,session,&queue).await?
        } else {None};
        let unsettled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
            .bind(session.as_ref()).fetch_one(&mut **tx).await?;
        let boundary:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind IN ('turn/started','turn/completed','turn/failed','turn/cancelled','turn/paused','turn/resume-authorized','message/input-blocked','message/input-queue-changed','context/cleared')")
            .bind(session.as_ref()).fetch_one(&mut **tx).await?;
        Ok(Self {queue,observed_seq:as_u64(boundary,"input placement observation")?,status:head.status,active_source,
            first,unsettled})
    }
}

pub(super) async fn input_view_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId,context:&InputViewContext)
    ->Result<SessionInputView,SessionStoreError> {
    let accepted=accepted_input_tx(tx,session,input).await?;
    session_input_queue::validate_selected_replacement_tx(tx,session,input).await?;
    let (facts,seq)=delivery_state_facts_tx(tx,session,input,&accepted).await?;
    let state:InputState=serde_json::from_value(facts["state"].clone())?;
    let mut view=SessionInputView {state,reason:facts["reason"].as_str().map(str::to_owned),
        event_seq:seq,placement:Placement::Transcript,kind:InputKind::Root,
        turn_id:facts["turn_id"].as_str().map(EventId::from),order_seq:facts["order_seq"].as_u64().unwrap_or(accepted.event.seq)};
    if facts["kind"]=="steering" {
        view.kind=InputKind::Steering;
        if state==InputState::Pending {view.placement=Placement::Steering;}
        return Ok(view);
    }
    if state==InputState::Claimed {return Ok(view);}
    if matches!(state,InputState::Cancelled|InputState::Discarded) {
        if session_input_queue::desktop_delivery_intent(&accepted.input.0)?.is_some() {
            // Removing, clearing or replacing a never-consumed queue item
            // must not publish it as a new cancelled conversation bubble.
            view.placement=Placement::Queued;
            let stopped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id WHERE e.session_id=? AND e.kind='message/input-queue-changed' AND e.kind_version=1 AND e.producer_id='session_api' AND e.causation_event_id=? AND e.seq>? AND e.correlation_id=e.session_id AND COALESCE(json_array_length(e.inline_json,'$.except_message_ids'),json_array_length(CAST(p.body AS TEXT),'$.value.except_message_ids'),0)=0 AND COALESCE(json_extract(e.inline_json,'$.action.kind'),json_extract(CAST(p.body AS TEXT),'$.value.action.kind'))='pause' AND COALESCE(json_extract(e.inline_json,'$.through_seq'),json_extract(CAST(p.body AS TEXT),'$.value.through_seq'))>=?)")
                .bind(session.as_ref()).bind(input.as_ref()).bind(as_i64(seq,"cancelled input sequence")?).bind(as_i64(accepted.event.seq,"original input sequence")?).fetch_one(&mut **tx).await?;
            if stopped && state==InputState::Cancelled {view.placement=Placement::Transcript;view.order_seq=seq;}
        }
        return Ok(view);
    }
    let Some(intent)=context.queue.intent(&accepted)? else {return Ok(view);};
    view.placement=Placement::Queued;
    if state==InputState::Blocked {return Ok(view);}
    view.event_seq=seq.max(context.observed_seq);
    if context.queue.held(&accepted)? {return Ok(view);}
    if let nomifun_agent_contracts::SessionInputDeliveryIntent::PreferCurrent {turn_id}=&intent {
        if context.status=="running" && context.active_source.as_deref()==Some(turn_id.as_ref()) {
            view.placement=Placement::Steering;view.kind=InputKind::Steering;view.turn_id=Some(turn_id.clone());
            view.order_seq=context.queue.intent_sequence(input).unwrap_or(accepted.event.seq);
            return Ok(view);
        }
        view.reason=Some("TARGET_TURN_ENDED".into());
    }
    if context.active_source.is_none() && context.status=="ready" && context.first.as_ref()==Some(input) {
        view.placement=Placement::Transcript;
        // Saved queue time is not execution order. Before the actual claim,
        // anchor preparation after the boundary that made this root eligible.
        view.order_seq=view.order_seq.max(context.observed_seq);
        if context.unsettled {view.reason=Some("WAITING_FOR_EFFECT_SETTLEMENT".into());}
    } else if context.status=="paused" {view.reason=Some("SESSION_PAUSED".into());}
    Ok(view)
}

async fn delivery_state_facts_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId,accepted:&PendingSessionInput)
    ->Result<(Value,u64),SessionStoreError> {
    let claims=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND ((e.kind='turn/started' AND e.causation_event_id=?) OR \
        (e.kind='turn/steer-accepted' AND e.kind_version=2 AND COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=?)) LIMIT 2")
        .bind(session.as_ref()).bind(input.as_ref()).bind(input.as_ref()).fetch_all(&mut **tx).await?;
    if claims.len()>1 {return Err(SessionStoreError::InvalidEvent("Session input has more than one consumption claim".into()));}
    if let Some(claim)=claims.into_iter().next() {
        let claim=event_from_row(claim)?;
        if claim.kind.0=="turn/started" {return Ok((json!({"state":"claimed","turn_id":input,"order_seq":claim.seq}),claim.seq));}
        let payload=payload_value_for_event_tx(tx,&claim).await?;
        if claim.producer_id.as_ref()!="session_api" || claim.causation_event_id.as_ref()!=Some(input)
            || payload.get("target_operation_id").and_then(Value::as_str)!=Some(claim.correlation_id.as_ref()) || payload.get("input").is_some() {
            return Err(SessionStoreError::InvalidEvent("input consumption claim differs from its original source and target".into()));
        }
        let source:String=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(session.as_ref()).bind(claim.correlation_id.as_ref()).fetch_one(&mut **tx).await?;
        let steering=|state:&str,reason:Option<&str>|json!({"state":state,"reason":reason,"kind":"steering","turn_id":source,"order_seq":claim.seq});
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
                    return Ok((steering("claimed",None),record.seq));
                }
                deferred=Some(record.seq);
            }
        }
        let terminal:Option<i64>=sqlx::query_scalar("SELECT terminal.seq FROM agent_turns turn JOIN agent_events terminal ON terminal.session_id=turn.session_id AND terminal.event_id=turn.terminal_event_id WHERE turn.session_id=? AND turn.operation_id=?")
            .bind(session.as_ref()).bind(claim.correlation_id.as_ref()).fetch_optional(&mut **tx).await?;
        if let Some(seq)=terminal {return Ok((steering("blocked",Some("TURN_ENDED_BEFORE_INPUT_DELIVERY")),as_u64(seq,"undelivered input terminal sequence")?));}
        if let Some(seq)=deferred {return Ok((steering("blocked",Some("TURN_ENDED_BEFORE_INPUT_DELIVERY")),seq));}
        return Ok((steering("pending",None),claim.seq));
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
    let context=InputViewContext::load(tx,session).await?;
    for message in messages.iter_mut().filter(|row|row.presentation_intent=="message"&&row.projection.get("state").and_then(Value::as_str)==Some("accepted")) {
        let Some(id)=message.projection_id.strip_prefix("message:") else {continue};
        if Uuid::parse_str(id).ok().is_none_or(|id|id.get_version_num()!=7) {continue;}
        let Some(row)=event_by_event_id_tx(tx,id).await? else {return Err(SessionStoreError::InvalidEvent("accepted message has no original fact".into()));};
        let event=event_from_row(row)?;
        let owner=live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref;
        if !is_session_input_event(&event,&owner) {continue;}
        let view=input_view_tx(tx,session,&event.event_id,&context).await?;
        message.last_seq=message.last_seq.max(view.event_seq);
        message.projection["turn_id"]=json!(view.turn_id);
        message.projection["input_delivery"]=serde_json::to_value(view)?;
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
    let placement_changed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND seq>? AND seq<=? AND kind IN ('message/input-queue-changed','turn/started','turn/completed','turn/failed','turn/cancelled','turn/paused','turn/resume-authorized','message/input-blocked'))")
        .bind(session.as_ref()).bind(as_i64(after,"placement cursor")?).bind(as_i64(head,"placement head")?).fetch_one(&mut **tx).await?;
    if placement_changed {
        ids.extend(pending_inputs_tx(tx,session,None,true).await?.into_iter().map(|input|input.event.event_id.as_ref().to_owned()));
    }
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

pub(super) const PENDING_INPUT_PREDICATE: &str = " WHERE input.session_id=? AND input.kind='message/user-accepted' \
            AND input.producer_id='session_api' AND input.kind_version=1 AND substr(input.idempotency_key,1,length(?))=? \
            AND input.seq>COALESCE((SELECT MAX(clear.seq) FROM agent_events clear WHERE clear.session_id=input.session_id AND clear.kind='context/cleared'),0) \
            AND NOT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_events started ON started.event_id=turn.started_event_id AND started.session_id=turn.session_id AND started.kind='turn/started' AND started.correlation_id=turn.operation_id WHERE turn.session_id=input.session_id AND (turn.source_message_id=input.event_id OR started.causation_event_id=input.event_id)) \
            AND NOT EXISTS(SELECT 1 FROM agent_events claim LEFT JOIN agent_payloads cp ON cp.payload_id=claim.payload_id AND cp.session_id=claim.session_id WHERE claim.session_id=input.session_id AND claim.kind='turn/steer-accepted' AND claim.kind_version=2 \
                AND COALESCE(json_extract(claim.inline_json,'$.input_event_id'),json_extract(CAST(cp.body AS TEXT),'$.value.input_event_id'))=input.event_id) \
            AND (? OR input.event_id=? OR NOT EXISTS(SELECT 1 FROM agent_events blocked LEFT JOIN agent_payloads bp ON bp.payload_id=blocked.payload_id AND bp.session_id=blocked.session_id WHERE blocked.session_id=input.session_id AND blocked.kind='message/input-blocked' \
                AND COALESCE(json_extract(blocked.inline_json,'$.input_event_id'),json_extract(CAST(bp.body AS TEXT),'$.value.input_event_id'))=input.event_id))";

pub(super) async fn pending_inputs_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,include_blocked:Option<&EventId>,all_blocked:bool)
        ->Result<Vec<PendingSessionInput>,SessionStoreError> {
        let live=require_live_session_tx(tx,session.as_ref()).await?;
        let prefix=session_input_namespace(&live.owner_ref,session);
        let query=format!("SELECT input.session_id,input.seq,input.event_id,input.producer_id,input.idempotency_key,input.kind,input.kind_version,input.correlation_id,input.causation_event_id,input.inline_json,input.payload_id FROM agent_events input{} ORDER BY input.seq",PENDING_INPUT_PREDICATE);
        let rows=sqlx::query_as::<_,StoredEventRow>(&query)
            .bind(session.as_ref()).bind(&prefix).bind(&prefix).bind(all_blocked).bind(include_blocked.map(|id|id.as_ref())).fetch_all(&mut **tx).await?;
        let mut inputs=Vec::new();for row in rows {
            let event=event_from_row(row)?;
            if !is_session_input_event(&event,&live.owner_ref) {return Err(SessionStoreError::InvalidEvent("pending input lost its original Session provenance".into()));}
            if (all_blocked || include_blocked==Some(&event.event_id)) && cancelled_input_tx(tx,&event).await?.is_some() {continue;}
            let input=StrictJsonValue(payload_value_for_event_tx(tx,&event).await?);
            session_input_queue::validate_delivery_envelope_tx(tx,session,&input.0).await?;
            inputs.push(PendingSessionInput {event,input});
        }
        Ok(inputs)
    }

impl AgentSessionStore {
    pub async fn session_input(&self,session:&AgentSessionId,input:&EventId)->Result<PendingSessionInput,SessionStoreError> {
        let mut tx=self.pool.begin().await?;require_live_session_tx(&mut tx,session.as_ref()).await?;
        let result=accepted_input_tx(&mut tx,session,input).await?;
        session_input_queue::validate_selected_replacement_tx(&mut tx,session,input).await?;
        tx.commit().await?;Ok(result)
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
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let live=live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if live.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(row)=event_by_producer_key_tx(&mut tx,producer.as_ref(),key.as_ref()).await? {
            let record=event_from_row(row)?;let accepted=accepted_input_tx(&mut tx,session,&record.event_id).await?;
            if accepted.input!=input {return Err(SessionStoreError::IdempotencyConflict("Session input key was reused with different content".into()));}
            let ack=event_ack(&record);db_write_observation.commit(tx).await?;
            return Ok(SessionEventAppendResult {record:Some(record),ack:Some(ack.clone()),cursor:ack.cursor,persisted:true,duplicate:true});
        }
        session_input_queue::validate_delivery_envelope_tx(&mut tx,session,&input.0).await?;
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
        let result=self.append_event_tx(&mut tx,&event,None).await?;db_write_observation.commit(tx).await?;Ok(result)
    }

    /// Filter persistent delivery eligibility before bounding a dispatch batch.
    pub async fn pending_session_inputs(&self,session:&AgentSessionId,include_blocked:Option<&EventId>)
        ->Result<Vec<PendingSessionInput>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let candidates=pending_inputs_tx(&mut tx,session,include_blocked,false).await?;
        let state=session_input_queue::queue_state_tx(&mut tx,session).await?;
        let mut inputs=Vec::new();
        for input in candidates {if !state.held(&input)? {inputs.push(input);}}
        state.sort(&mut inputs)?;inputs.truncate(64);
        tx.commit().await?;Ok(inputs)
    }

    /// has_pending and take use this same exact-target candidate selection.
    pub async fn pending_session_inputs_for_steering(&self,session:&AgentSessionId,target:&OperationId,include_blocked:Option<&EventId>)
        ->Result<Vec<PendingSessionInput>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let state=session_input_queue::queue_state_tx(&mut tx,session).await?;
        let inputs=session_input_queue::pending_steering_inputs_tx(&mut tx,session,target,include_blocked,&state).await?;
        for input in &inputs {session_input_queue::validate_selected_replacement_tx(&mut tx,session,&input.event.event_id).await?;}
        tx.commit().await?;Ok(inputs)
    }

    pub async fn claim_session_input_as_turn(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,operation:&OperationId,
        admission:StrictJsonValue)->Result<SessionEventAppendResult,SessionStoreError> {
        self.try_claim_session_input_as_turn(owner,session,input,operation,admission).await?
            .ok_or(SessionStoreError::ExecutionFenced)
    }

    pub async fn try_claim_session_input_as_turn(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,operation:&OperationId,
        admission:StrictJsonValue)->Result<Option<SessionEventAppendResult>,SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        session_input_queue::validate_selected_replacement_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {db_write_observation.commit(tx).await?;return Ok(None);}
        let queue=session_input_queue::queue_state_tx(&mut tx,session).await?;
        if queue.held(&accepted)? {db_write_observation.commit(tx).await?;return Ok(None);}
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if accepted.event.seq<=as_u64(floor,"input context floor")? {db_write_observation.commit(tx).await?;return Ok(None);}
        if input_claimed_tx(&mut tx,session,input).await? {db_write_observation.commit(tx).await?;return Ok(None);}
        let head=head_by_id_tx(&mut tx,session.as_ref()).await?;
        if head.status!="ready" || head.active_turn_id.is_some() {db_write_observation.commit(tx).await?;return Ok(None);}
        let unsettled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if unsettled {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        if session_input_queue::desktop_delivery_intent(&accepted.input.0)?.is_some() {
            let mut eligible=Vec::new();
            for candidate in pending_inputs_tx(&mut tx,session,Some(input),false).await? {
                if !queue.held(&candidate)? {eligible.push(candidate);}
            }
            queue.sort(&mut eligible)?;
            if eligible.first().is_none_or(|candidate|candidate.event.event_id!=*input) {db_write_observation.commit(tx).await?;return Ok(None);}
        }
        let mut payload=json!({"operation_id":operation,"source_message_id":input,"admission":admission});
        for field in ["route_identity","resolved_snapshot_ref"] {if let Some(value)=admission.0.get(field) {payload[field]=value.clone();}}
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:new_event_id(),producer_id:"session_api".into(),
            idempotency_key:format!("session-input-claim:{}",input.as_ref()).into(),semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                kind:SessionEventKind("turn/started".into()),kind_version:1,correlation_id:CorrelationId::from(operation.as_ref()),
                causation_event_id:Some(accepted.event.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))}};
        let result=self.append_event_tx(&mut tx,&event,None).await?;db_write_observation.commit(tx).await?;Ok(Some(result))
    }

    pub async fn claim_session_input_as_steering(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,target:&OperationId)
        ->Result<SessionEventAppendResult,SessionStoreError> {
        self.try_claim_session_input_as_steering(owner,session,input,target).await?
            .ok_or(SessionStoreError::ExecutionFenced)
    }

    pub async fn try_claim_session_input_as_steering(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,target:&OperationId)
        ->Result<Option<SessionEventAppendResult>,SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        if live_session_by_id_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        session_input_queue::validate_selected_replacement_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {db_write_observation.commit(tx).await?;return Ok(None);}
        let head=head_by_id_tx(&mut tx,session.as_ref()).await?;
        if head.status!="running" || head.active_turn_id.as_deref()!=Some(target.as_ref()) {db_write_observation.commit(tx).await?;return Ok(None);}
        let queue=session_input_queue::queue_state_tx(&mut tx,session).await?;
        if !session_input_queue::steering_eligible_tx(&mut tx,session,&accepted,target,&queue).await? {db_write_observation.commit(tx).await?;return Ok(None);}
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if accepted.event.seq<=as_u64(floor,"input context floor")? {db_write_observation.commit(tx).await?;return Ok(None);}
        if input_claimed_tx(&mut tx,session,input).await? {db_write_observation.commit(tx).await?;return Ok(None);}
        let unknown:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state='unknown')")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if unknown {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:new_event_id(),producer_id:"session_api".into(),
            idempotency_key:format!("session-input-claim:{}",input.as_ref()).into(),semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
                kind:SessionEventKind("turn/steer-accepted".into()),kind_version:2,correlation_id:CorrelationId::from(target.as_ref()),
                causation_event_id:Some(input.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"input_event_id":input,"target_operation_id":target})))}};
        let result=self.append_event_tx(&mut tx,&event,None).await?;db_write_observation.commit(tx).await?;Ok(Some(result))
    }

    pub async fn mark_session_input_blocked(&self,session:&AgentSessionId,input:&EventId,reason:SessionInputBlockedReason)
        ->Result<(),SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let accepted=accepted_input_tx(&mut tx,session,input).await?;
        if cancelled_input_tx(&mut tx,&accepted.event).await?.is_some() {db_write_observation.commit(tx).await?;return Ok(());}
        if input_claimed_tx(&mut tx,session,input).await? {db_write_observation.commit(tx).await?;return Ok(());}
        let identity=format!("session-input-blocked:{}:{}",input.as_ref(),reason.identity());
        let event=SessionEventAppend {agent_session_id:session.clone(),event_id:identity.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:identity.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind("message/input-blocked".into()),kind_version:1,
                correlation_id:CorrelationId::from(input.as_ref()),causation_event_id:Some(input.clone()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"input_event_id":input,"reason":reason})))}};
        self.append_event_tx(&mut tx,&event,None).await?;db_write_observation.commit(tx).await?;Ok(())
    }
}
