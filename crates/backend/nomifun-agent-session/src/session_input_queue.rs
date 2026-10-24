//! Queue control is a projection of accepted input and reference-only changes.
//! It never owns another message body, task ledger, or runtime.
use super::*;
use nomifun_agent_contracts::{SessionInputDeliveryIntent, SessionInputQueueAction, SessionInputQueueChangedV1 as QueueChange,
    SessionInputQueueChange as Change, SessionInputQueueCommand as Command};
use serde::{Deserialize, Serialize};

const QUEUE_CHANGED: &str = "message/input-queue-changed";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInputQueueItem {
    pub message_id: EventId,
    pub input: StrictJsonValue,
    pub delivery: SessionInputDeliveryIntent,
    pub held: bool,
    pub input_delivery: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInputQueueSnapshot {
    pub version: u64,
    pub items: Vec<SessionInputQueueItem>,
}

#[derive(Default)]
pub(super) struct QueueState {
    version: u64,
    held_through: Option<u64>,
    released: BTreeSet<EventId>,
    order: Vec<EventId>,
    delivery_overrides: BTreeMap<EventId, SessionInputDeliveryIntent>,
    intent_sequences: BTreeMap<EventId,u64>,
    replacement_holds:BTreeSet<EventId>,
    replacement_order:BTreeMap<EventId,u64>,
}

pub(super) fn desktop_delivery_intent(input: &Value) -> Result<Option<SessionInputDeliveryIntent>, SessionStoreError> {
    let Some(value) = input.get("delivery_intent") else { return Ok(None); };
    if input.get("channel_source").is_some() || input.get("input_source").is_some() {
        return Err(SessionStoreError::InvalidPayload("composer delivery cannot author a domain input source".into()));
    }
    Ok(Some(serde_json::from_value(value.clone())?))
}

pub(super) async fn validate_delivery_target_tx(
    tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, intent: &SessionInputDeliveryIntent,
) -> Result<(), SessionStoreError> {
    if let SessionInputDeliveryIntent::PreferCurrent { turn_id } = intent {
        validate_uuidv7(turn_id.as_ref(), "observed Turn source")?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND source_message_id=?)")
            .bind(session.as_ref()).bind(turn_id.as_ref()).fetch_one(&mut **tx).await?;
        if !exists { return Err(SessionStoreError::InvalidEvent("observed Turn does not belong to this Session".into())); }
    }
    Ok(())
}

pub(super) async fn validate_delivery_envelope_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&Value)
    ->Result<(),SessionStoreError> {
    if let Some(delivery)=desktop_delivery_intent(input)? {validate_delivery_target_tx(tx,session,&delivery).await?;}
    Ok(())
}

struct AcceptedRouting {
    seq:u64,
    intent:SessionInputDeliveryIntent,
}
impl AcceptedRouting {
    fn value(&self)->Value {json!({"delivery_intent":self.intent})}
}

/// Read only the provenance and routing fields of an unselected input. A
/// queue control is not authority to hydrate its text, files or Skill bodies.
async fn accepted_routing_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId,owner:&PrincipalRef)
    ->Result<AcceptedRouting,SessionStoreError> {
    type Routing=(i64,String,i64,String,String,String,Option<String>,bool,bool);
    let row=sqlx::query_as::<_,Routing>("SELECT e.seq,e.kind,e.kind_version,e.producer_id,e.idempotency_key,e.correlation_id, \
        COALESCE(json_extract(e.inline_json,'$.delivery_intent'),json_extract(CAST(p.body AS TEXT),'$.value.delivery_intent')), \
        COALESCE(json_type(e.inline_json,'$.channel_source'),json_type(CAST(p.body AS TEXT),'$.value.channel_source')) IS NOT NULL, \
        COALESCE(json_type(e.inline_json,'$.input_source'),json_type(CAST(p.body AS TEXT),'$.value.input_source')) IS NOT NULL \
        FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND e.event_id=?")
        .bind(session.as_ref()).bind(input.as_ref()).fetch_optional(&mut **tx).await?
        .ok_or_else(||SessionStoreError::InvalidEvent("queue routing source is missing".into()))?;
    let(seq,kind,version,producer,key,correlation,intent,has_channel,has_source)=row;
    let prefix=session_input::session_input_namespace(owner,session);
    let valid_key=key.strip_prefix(&prefix).is_some_and(|digest|digest.len()==64
        && digest.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)));
    if kind!="message/user-accepted" || version!=1 || producer!="session_api" || correlation!=input.as_ref()
        || !valid_key || has_channel || has_source {
        return Err(SessionStoreError::InvalidEvent("queue routing source lost its ordinary accepted provenance".into()));
    }
    validate_uuidv7(input.as_ref(),"queue routing source")?;
    let intent:SessionInputDeliveryIntent=serde_json::from_str(intent.as_deref()
        .ok_or_else(||SessionStoreError::InvalidPayload("queue routing source lost its delivery intent".into()))?)?;
    validate_delivery_target_tx(tx,session,&intent).await?;
    Ok(AcceptedRouting {seq:as_u64(seq,"queue routing sequence")?,intent})
}

async fn validate_routing_references_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,
    change:&QueueChange,owner:&PrincipalRef)->Result<BTreeMap<EventId,AcceptedRouting>,SessionStoreError> {
    let mut ids=match &change.action {
        Change::Replace {message_id,replacement_message_id}=>vec![message_id.clone(),replacement_message_id.clone()],
        Change::Control(action)=>match action {
            SessionInputQueueAction::Remove {message_id}|SessionInputQueueAction::Promote {message_id,..}=>vec![message_id.clone()],
            SessionInputQueueAction::Reorder {message_ids}=>message_ids.clone(),
            _=>Vec::new(),
        },
    };
    ids.extend(change.except_message_ids.iter().cloned());
    if ids.iter().collect::<BTreeSet<_>>().len()!=ids.len() {
        return Err(SessionStoreError::InvalidPayload("queue change duplicates an input identity".into()));
    }
    let mut routing=BTreeMap::new();
    for id in ids {
        let input=accepted_routing_tx(tx,&event.agent_session_id,&id,owner).await?;
        if input.seq>=event.seq {return Err(SessionStoreError::InvalidEvent("queue routing reference is not prior to its change".into()));}
        routing.insert(id,input);
    }
    if let Change::Replace {message_id,replacement_message_id}=&change.action {
        let original=&routing[message_id];let replacement=&routing[replacement_message_id];
        let identity=format!("session-input-blocked:{}:cancelled_by_owner",message_id.as_ref());
        let cancellation=event_by_event_id_tx(tx,&identity).await?.map(event_from_row).transpose()?
            .ok_or_else(||SessionStoreError::InvalidEvent("replacement lost original cancellation".into()))?;
        let payload=payload_value_for_event_tx(tx,&cancellation).await?;
        if cancellation.agent_session_id!=event.agent_session_id || cancellation.seq<=original.seq
            || cancellation.seq>=replacement.seq || cancellation.kind.0!="message/input-blocked" || cancellation.kind_version!=1
            || cancellation.producer_id.as_ref()!="runtime_supervisor" || cancellation.idempotency_key.as_ref()!=identity
            || cancellation.causation_event_id.as_ref()!=Some(message_id) || cancellation.correlation_id.as_ref()!=message_id.as_ref()
            || payload["input_event_id"].as_str()!=Some(message_id.as_ref()) || payload["reason"]!="CANCELLED_BY_OWNER"
            || original.intent!=replacement.intent {
            return Err(SessionStoreError::InvalidEvent("replacement lost its original routing or writer order".into()));
        }
    }
    if let Change::Control(SessionInputQueueAction::Promote {turn_id,..})=&change.action {
        validate_delivery_target_tx(tx,&event.agent_session_id,&SessionInputDeliveryIntent::PreferCurrent {turn_id:turn_id.clone()}).await?;
    }
    Ok(routing)
}

/// Validate the full immutable envelope only along the selected input's edit
/// ancestry. Callers must invoke this after resolving their exact accepted
/// input, not recursively from the low-level accepted-input reader.
pub(super) async fn validate_selected_replacement_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,input:&EventId)
    ->Result<(),SessionStoreError> {
    let mut selected=input.clone();
    loop {
        let rows=sqlx::query_as::<_,StoredEventRow>("SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
            FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
            WHERE e.session_id=? AND e.kind='message/input-queue-changed' AND \
            COALESCE(json_extract(e.inline_json,'$.action.replacement_message_id'),json_extract(CAST(p.body AS TEXT),'$.value.action.replacement_message_id'))=? LIMIT 2")
            .bind(session.as_ref()).bind(selected.as_ref()).fetch_all(&mut **tx).await?;
        if rows.len()>1 {return Err(SessionStoreError::InvalidEvent("replacement has multiple queue owners".into()));}
        let Some(row)=rows.into_iter().next() else {return Ok(());};
        let event=event_from_row(row)?;let payload=payload_value_for_event_tx(tx,&event).await?;
        validate_event(&event,&payload)?;validate_references_tx(tx,&event,&payload).await?;
        let change:QueueChange=serde_json::from_value(payload)?;
        let Change::Replace {message_id,replacement_message_id}=change.action else {
            return Err(SessionStoreError::InvalidEvent("replacement source has a different queue action".into()));
        };
        if replacement_message_id!=selected {return Err(SessionStoreError::InvalidEvent("replacement lineage changed its selected input".into()));}
        selected=message_id;
    }
}

pub(super) async fn queue_state_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId) -> Result<QueueState, SessionStoreError> {
    let owner=require_live_session_tx(tx,session.as_ref()).await?.owner_ref;
    let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let rows = sqlx::query_as::<_, StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='message/input-queue-changed' AND seq>? ORDER BY seq")
        .bind(session.as_ref()).bind(floor).fetch_all(&mut **tx).await?;
    let mut state = QueueState { version: as_u64(floor, "queue context floor")?, ..Default::default() };
    for row in rows {
        let event = event_from_row(row)?;
        let payload=payload_value_for_event_tx(tx, &event).await?;
        validate_event(&event,&payload)?;
        let change: QueueChange = serde_json::from_value(payload)?;
        let routing=validate_routing_references_tx(tx,&event,&change,&owner).await?;
        match change.action {
            Change::Replace {message_id,replacement_message_id} => {
                if let Some(index)=state.order.iter().position(|id|id==&message_id) {state.order[index]=replacement_message_id.clone();}
                // Replacement cannot release a user's held batch merely
                // because its newly accepted identity has a later sequence.
                let original=&routing[&message_id];
                state.replacement_order.insert(replacement_message_id.clone(),state.replacement_order.get(&message_id).copied().unwrap_or(original.seq));
                if state.held_metadata(&message_id,original.seq,&original.value())? {state.replacement_holds.insert(replacement_message_id.clone());}
                if state.released.remove(&message_id) {state.released.insert(replacement_message_id.clone());}
                if let Some(intent)=state.delivery_overrides.remove(&message_id) {state.delivery_overrides.insert(replacement_message_id.clone(),intent);}
                state.intent_sequences.insert(replacement_message_id,event.seq);
            },
            Change::Control(action) => match action {
            SessionInputQueueAction::Pause {} => {
                let cutoff = change.through_seq.filter(|seq| *seq < event.seq)
                    .ok_or_else(|| SessionStoreError::InvalidPayload("queue hold has no prior input cutoff".into()))?;
                state.held_through = Some(state.held_through.unwrap_or(0).max(cutoff));
                state.released=change.except_message_ids.into_iter().collect();
            }
            SessionInputQueueAction::Resume {} => {state.held_through = None;state.replacement_holds.clear();},
            SessionInputQueueAction::Reorder { message_ids } => state.order = message_ids,
            SessionInputQueueAction::Promote { message_id, turn_id } => {
                state.released.insert(message_id.clone());
                state.replacement_holds.remove(&message_id);
                state.intent_sequences.insert(message_id.clone(),event.seq);
                state.delivery_overrides.insert(message_id, SessionInputDeliveryIntent::PreferCurrent { turn_id });
            }
            SessionInputQueueAction::Remove { .. } | SessionInputQueueAction::Clear {} => {},
            },
        }
        state.version = event.seq;
    }
    Ok(state)
}

impl QueueState {
    pub(super) fn intent_sequence(&self,input:&EventId)->Option<u64>{self.intent_sequences.get(input).copied()}
    pub(super) fn held(&self, input: &PendingSessionInput) -> Result<bool, SessionStoreError> {
        self.held_metadata(&input.event.event_id,input.event.seq,&input.input.0)
    }
    fn held_metadata(&self,id:&EventId,seq:u64,routing:&Value)->Result<bool,SessionStoreError> {
        Ok(desktop_delivery_intent(routing)?.is_some() && !self.released.contains(id)
            && (self.replacement_holds.contains(id)||self.held_through.is_some_and(|cutoff| seq<=cutoff)))
    }
    pub(super) fn intent(&self, input: &PendingSessionInput) -> Result<Option<SessionInputDeliveryIntent>, SessionStoreError> {
        self.intent_metadata(&input.event.event_id,&input.input.0)
    }
    fn intent_metadata(&self,id:&EventId,routing:&Value)->Result<Option<SessionInputDeliveryIntent>,SessionStoreError> {
        Ok(self.delivery_overrides.get(id).cloned().or(desktop_delivery_intent(routing)?))
    }
    pub(super) fn sort(&self, inputs: &mut [PendingSessionInput]) -> Result<(),SessionStoreError> {
        // Restore replacement positions across every source before applying
        // the user's composer-only permutation to its original slots.
        inputs.sort_by_key(|input|self.replacement_order.get(&input.event.event_id).copied().unwrap_or(input.event.seq));
        let rank: BTreeMap<_, _> = self.order.iter().enumerate().map(|(index,id)| (id,index)).collect();
        let mut slots=Vec::new();let mut composer=Vec::new();
        for (index,input) in inputs.iter().enumerate() {
            if desktop_delivery_intent(&input.input.0)?.is_some() {slots.push(index);composer.push(input.clone());}
        }
        composer.sort_by_key(|input| (rank.get(&input.event.event_id).copied().unwrap_or(usize::MAX),
            self.replacement_order.get(&input.event.event_id).copied().unwrap_or(input.event.seq)));
        for (index,input) in slots.into_iter().zip(composer) {inputs[index]=input;}
        Ok(())
    }
}

/// Presentation selects the first eligible root using routing metadata only.
/// A delivery read at a model boundary must not hydrate every queued body.
pub(super) async fn first_eligible_root_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,state:&QueueState)
    ->Result<Option<EventId>,SessionStoreError> {
    let owner=require_live_session_tx(tx,session.as_ref()).await?.owner_ref;
    let prefix=session_input::session_input_namespace(&owner,session);
    let query=format!("SELECT input.event_id,input.seq,COALESCE(json_extract(input.inline_json,'$.delivery_intent'),json_extract(CAST(p.body AS TEXT),'$.value.delivery_intent')) FROM agent_events input LEFT JOIN agent_payloads p ON p.payload_id=input.payload_id AND p.session_id=input.session_id{} ORDER BY input.seq",session_input::PENDING_INPUT_PREDICATE);
    let rows=sqlx::query_as::<_,(String,i64,Option<String>)>(&query).bind(session.as_ref()).bind(&prefix).bind(&prefix).bind(false).bind(Option::<&str>::None).fetch_all(&mut **tx).await?;
    let mut eligible=Vec::new();
    for (id,seq,intent) in rows {
        let id=EventId::from(id);let seq=as_u64(seq,"input routing order")?;
        let mut routing=json!({});
        if let Some(intent)=intent {routing["delivery_intent"]=serde_json::from_str(&intent)?;}
        if let Some(intent)=desktop_delivery_intent(&routing)? {validate_delivery_target_tx(tx,session,&intent).await?;}
        if !state.held_metadata(&id,seq,&routing)? {eligible.push((id,seq,routing.get("delivery_intent").is_some()));}
    }
    eligible.sort_by_key(|(id,seq,_)|state.replacement_order.get(id).copied().unwrap_or(*seq));
    let Some((first,_,composer_first))=eligible.first() else {return Ok(None);};
    if !composer_first {return Ok(Some(first.clone()));}
    let rank:BTreeMap<_,_>=state.order.iter().enumerate().map(|(i,id)|(id,i)).collect();
    Ok(eligible.iter().filter(|(_,_,composer)|*composer).min_by_key(|(id,seq,_)|(
        rank.get(id).copied().unwrap_or(usize::MAX),state.replacement_order.get(id).copied().unwrap_or(*seq)))
        .map(|(id,_,_)|id.clone()))
}

pub(super) async fn steering_eligible_tx(
    tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, input: &PendingSessionInput,
    target: &OperationId, state: &QueueState,
) -> Result<bool, SessionStoreError> {
    let (source, root) = AgentSessionStore::turn_accepted_input_tx(tx, session, target).await?;
    let receiver = nomifun_agent_contracts::ChannelInputSource::from_input(&root.0).map_err(SessionStoreError::InvalidPayload)?;
    steering_metadata_eligible(&input.event.event_id,input.event.seq,&input.input.0,source.event_id.as_ref(),receiver.as_ref(),state)
}

fn steering_metadata_eligible(id:&EventId,seq:u64,routing:&Value,target_source:&str,
    receiver:Option<&nomifun_agent_contracts::ChannelInputSource>,state:&QueueState)->Result<bool,SessionStoreError> {
    let intent=state.intent_metadata(id,routing)?;
    if let Some(SessionInputDeliveryIntent::PreferCurrent {turn_id})=&intent {
        validate_uuidv7(turn_id.as_ref(),"observed Turn source")?;
    }
    if state.held_metadata(id,seq,routing)? || routing.get("reply_to").is_some() {return Ok(false);}
    match intent {
        Some(SessionInputDeliveryIntent::Queue {})=>return Ok(false),
        Some(SessionInputDeliveryIntent::PreferCurrent {turn_id}) if turn_id.as_ref()!=target_source=>return Ok(false),
        _=>{},
    }
    let incoming=nomifun_agent_contracts::ChannelInputSource::from_input(routing).map_err(SessionStoreError::InvalidPayload)?;
    Ok(nomifun_agent_contracts::ChannelInputSource::same_destination(receiver,incoming.as_ref()))
}

/// Keyset-scan only routing metadata. Queue bodies and attachment references
/// are never hydrated at a model boundary unless the input is eligible there.
pub(super) async fn pending_steering_inputs_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,
    target:&OperationId,include_blocked:Option<&EventId>,state:&QueueState)->Result<Vec<PendingSessionInput>,SessionStoreError> {
    let owner=require_live_session_tx(tx,session.as_ref()).await?.owner_ref;
    let prefix=session_input::session_input_namespace(&owner,session);
    let (source,root)=AgentSessionStore::turn_accepted_input_tx(tx,session,target).await?;
    let receiver=nomifun_agent_contracts::ChannelInputSource::from_input(&root.0).map_err(SessionStoreError::InvalidPayload)?;
    let extract=|field:&str|format!("COALESCE(json_extract(input.inline_json,'$.{field}'),json_extract(CAST(p.body AS TEXT),'$.value.{field}'))");
    let present=|field:&str|format!("COALESCE(json_type(input.inline_json,'$.{field}'),json_type(CAST(p.body AS TEXT),'$.value.{field}')) IS NOT NULL");
    let query=format!("SELECT input.event_id,input.seq,input.idempotency_key,input.correlation_id,{},{},{},{},{},{} \
        FROM agent_events input LEFT JOIN agent_payloads p ON p.payload_id=input.payload_id AND p.session_id=input.session_id{} \
        AND input.seq>? ORDER BY input.seq LIMIT 128",present("delivery_intent"),extract("delivery_intent"),present("channel_source"),extract("channel_source"),
        present("input_source"),present("reply_to"),session_input::PENDING_INPUT_PREDICATE);
    type RoutingRow=(String,i64,String,String,bool,Option<String>,bool,Option<String>,bool,bool);
    let mut after=0_i64;let mut selected=Vec::new();
    loop {
        let rows=sqlx::query_as::<_,RoutingRow>(&query).bind(session.as_ref()).bind(&prefix).bind(&prefix).bind(false)
            .bind(include_blocked.map(EventId::as_ref)).bind(after).fetch_all(&mut **tx).await?;
        let complete=rows.len()<128;
        for (id,seq,key,correlation,has_delivery_intent,delivery_intent,has_channel,channel,has_input_source,has_reply) in rows {
            after=seq;
            let digest=key.strip_prefix(&prefix).unwrap_or("");
            if correlation!=id || digest.len()!=64 || !digest.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)) {
                return Err(SessionStoreError::InvalidEvent("pending routing metadata lost its accepted identity".into()));
            }
            let mut routing=serde_json::Map::new();
            if has_delivery_intent {routing.insert("delivery_intent".into(),serde_json::from_str(delivery_intent.as_deref().unwrap_or("null"))?);}
            if has_channel {routing.insert("channel_source".into(),serde_json::from_str(channel.as_deref().unwrap_or("null"))?);}
            if has_input_source {routing.insert("input_source".into(),Value::Null);}
            if has_reply {routing.insert("reply_to".into(),Value::Null);}
            let id=EventId::from(id);
            if steering_metadata_eligible(&id,as_u64(seq,"pending routing sequence")?,&Value::Object(routing),source.event_id.as_ref(),receiver.as_ref(),state)? {
                let original=session_input::accepted_input_tx(tx,session,&id).await?;
                if include_blocked==Some(&id) && session_input::cancelled_input_tx(tx,&original.event).await?.is_some() {continue;}
                selected.push(original);if selected.len()==64 {return Ok(selected);}
            }
        }
        if complete {return Ok(selected);}
    }
}

async fn queue_snapshot_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId) -> Result<SessionInputQueueSnapshot, SessionStoreError> {
    let owner = require_live_session_tx(tx, session.as_ref()).await?.owner_ref;
    let context=session_input::InputViewContext::load(tx,session).await?;
    let state=&context.queue;
    let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let prefix=session_input::session_input_namespace(&owner,session);
    // A queue version observes only ordinary inputs and their queue-relevant
    // mutations. It does not walk closed Turn journals or change per token.
    let changed:i64=sqlx::query_scalar("WITH composer_inputs AS (SELECT input.event_id FROM agent_events input LEFT JOIN agent_payloads p ON p.payload_id=input.payload_id AND p.session_id=input.session_id \
        WHERE input.session_id=? AND input.kind='message/user-accepted' AND input.producer_id='session_api' AND input.kind_version=1 AND input.seq>? \
        AND substr(input.idempotency_key,1,length(?))=? AND COALESCE(json_extract(input.inline_json,'$.delivery_intent'),json_extract(CAST(p.body AS TEXT),'$.value.delivery_intent')) IS NOT NULL) \
        SELECT COALESCE(MAX(e.seq),0) FROM agent_events e LEFT JOIN agent_payloads p ON p.payload_id=e.payload_id AND p.session_id=e.session_id \
        WHERE e.session_id=? AND (e.event_id IN (SELECT event_id FROM composer_inputs) \
        OR (e.kind='turn/started' AND e.causation_event_id IN (SELECT event_id FROM composer_inputs)) \
        OR (e.kind IN ('message/input-blocked','turn/steer-accepted') AND COALESCE(json_extract(e.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id')) IN (SELECT event_id FROM composer_inputs)))")
        .bind(session.as_ref()).bind(floor).bind(&prefix).bind(&prefix).bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let boundary:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind IN ('turn/started','turn/completed','turn/failed','turn/cancelled','turn/paused','turn/resume-authorized','message/input-blocked')")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let version = state.version.max(as_u64(changed.max(boundary),"queue mutation sequence")?);
    let mut items = Vec::new();
    let mut originals=session_input::pending_inputs_tx(tx,session,None,true).await?;
    state.sort(&mut originals)?;
    for original in originals {
        let Some(delivery) = state.intent(&original)? else { continue; };
        let input_delivery=serde_json::to_value(session_input::input_view_tx(tx,session,&original.event.event_id,&context).await?)?;
        items.push(SessionInputQueueItem { message_id: original.event.event_id.clone(), held: state.held(&original)?,
            input: original.input, delivery, input_delivery });
    }
    Ok(SessionInputQueueSnapshot { version, items })
}

impl AgentSessionStore {
    pub async fn read_session_input_queue_command_receipt(&self,owner:&PrincipalRef,session:&AgentSessionId,
        key:&IdempotencyKey,command:&Command,expected_version:Option<u64>)->Result<Option<SessionInputQueueSnapshot>,SessionStoreError> {
        if let Command::Control(action)=command {return self.read_session_input_queue_receipt(owner,session,key,action,expected_version).await;}
        let Command::Replace {message_id,content}=command else {unreachable!()};
        let mut tx=self.pool.begin().await?;
        if require_live_session_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let matched=replacement_receipt_tx(&mut tx,session,&queue_command_key(owner,session,key)?,message_id,content,expected_version).await?;
        let result=if matched {Some(queue_snapshot_tx(&mut tx,session).await?)} else {None};
        tx.commit().await?;Ok(result)
    }

    pub async fn update_session_input_queue_command(&self,owner:&PrincipalRef,session:&AgentSessionId,
        key:&IdempotencyKey,command:&Command,expected_version:Option<u64>)->Result<SessionInputQueueSnapshot,SessionStoreError> {
        if let Command::Control(action)=command {return self.update_session_input_queue(owner,session,key,action,expected_version).await;}
        let Command::Replace {message_id,content}=command else {unreachable!()};
        if content.trim().is_empty() || content.len()>16*1024 {return Err(SessionStoreError::InvalidPayload("replacement text requires non-empty content within 16 KiB".into()));}
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        if require_live_session_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let identity=queue_command_key(owner,session,key)?;
        if replacement_receipt_tx(&mut tx,session,&identity,message_id,content,expected_version).await? {
            let snapshot=queue_snapshot_tx(&mut tx,session).await?;db_write_observation.commit(tx).await?;return Ok(snapshot);
        }
        let snapshot=queue_snapshot_tx(&mut tx,session).await?;
        if expected_version.is_some_and(|version|version!=snapshot.version) {return Err(SessionStoreError::Conflict("input queue changed; refresh before editing".into()));}
        if !snapshot.items.iter().any(|item|item.message_id==*message_id) {return Err(SessionStoreError::Conflict("input is no longer pending".into()));}
        let mut replacement=session_input::accepted_input_tx(&mut tx,session,message_id).await?.input;
        replacement.0["content"]=json!(content);
        if canonical_json_bytes(&replacement.0)?.len()>MAX_INLINE_JSON_BYTES {return Err(SessionStoreError::InvalidPayload("replacement input envelope exceeds its bound".into()));}
        let cause=latest_turn_boundary_event_tx(&mut tx,session.as_ref()).await?;
        let id=new_event_id();
        let accepted=SessionEventAppend {agent_session_id:session.clone(),event_id:id.clone(),producer_id:"session_api".into(),
            idempotency_key:session_input::command_key(owner,session,&format!("queue-replacement:{identity}").into())?,
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind("message/user-accepted".into()),kind_version:1,
                correlation_id:CorrelationId::from(id.as_ref()),causation_event_id:Some(cause),payload:SessionEventPayloadRef::InlineJson(replacement)}};
        self.cancel_pending_input_tx(&mut tx,session,message_id).await?;
        self.append_event_tx(&mut tx,&accepted,None).await?;
        self.append_queue_change_tx(&mut tx,session,&identity,QueueChange {action:Change::Replace {message_id:message_id.clone(),replacement_message_id:id},expected_version,through_seq:None,except_message_ids:Vec::new()}).await?;
        let snapshot=queue_snapshot_tx(&mut tx,session).await?;db_write_observation.commit(tx).await?;Ok(snapshot)
    }
    pub async fn session_input_queue(&self, owner: &PrincipalRef, session: &AgentSessionId) -> Result<SessionInputQueueSnapshot, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        if require_live_session_tx(&mut tx, session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let snapshot = queue_snapshot_tx(&mut tx, session).await?;
        tx.commit().await?; Ok(snapshot)
    }

    /// A transport retry may confirm its original command after ordinary
    /// admission has changed. This read grants no new control or dispatch.
    pub async fn read_session_input_queue_receipt(&self,owner:&PrincipalRef,session:&AgentSessionId,
        key:&IdempotencyKey,action:&SessionInputQueueAction,expected_version:Option<u64>,
    )->Result<Option<SessionInputQueueSnapshot>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        if require_live_session_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let identity=queue_command_key(owner,session,key)?;
        let result=if queue_receipt_matches_tx(&mut tx,session,&identity,action,expected_version).await? {
            Some(queue_snapshot_tx(&mut tx,session).await?)
        } else {None};
        tx.commit().await?;Ok(result)
    }

    pub async fn update_session_input_queue(&self, owner: &PrincipalRef, session: &AgentSessionId,
        key: &IdempotencyKey, action: &SessionInputQueueAction, expected_version: Option<u64>,
    ) -> Result<SessionInputQueueSnapshot, SessionStoreError> {
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        if require_live_session_tx(&mut tx, session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let identity = queue_command_key(owner, session, key)?;
        if queue_receipt_matches_tx(&mut tx,session,&identity,action,expected_version).await? {
            let snapshot = queue_snapshot_tx(&mut tx, session).await?;
            db_write_observation.commit(tx).await?; return Ok(snapshot);
        }
        let snapshot = queue_snapshot_tx(&mut tx, session).await?;
        if expected_version.is_some_and(|version| version != snapshot.version) {
            return Err(SessionStoreError::Conflict("input queue changed; refresh before applying this command".into()));
        }
        let pending: BTreeSet<_> = snapshot.items.iter().map(|item| item.message_id.clone()).collect();
        match action {
            SessionInputQueueAction::Remove { message_id } | SessionInputQueueAction::Promote { message_id, .. } => {
                if !pending.contains(message_id) { return Err(SessionStoreError::Conflict("input is no longer pending".into())); }
            }
            SessionInputQueueAction::Reorder { message_ids } => {
                if message_ids.len() != pending.len() || message_ids.iter().cloned().collect::<BTreeSet<_>>() != pending {
                    return Err(SessionStoreError::Conflict("reorder must name every exact pending input once".into()));
                }
            }
            _ => {},
        }
        if let SessionInputQueueAction::Promote { turn_id, .. } = action {
            if let SessionInputQueueAction::Promote {message_id,..}=action
                && snapshot.items.iter().any(|item|item.message_id==*message_id&&item.input_delivery["state"]=="blocked") {
                return Err(SessionStoreError::Conflict("blocked input must be edited or removed before another submission".into()));
            }
            validate_delivery_target_tx(&mut tx, session, &SessionInputDeliveryIntent::PreferCurrent { turn_id: turn_id.clone() }).await?;
            let active_source: Option<String> = sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=(SELECT active_turn_id FROM agent_session_heads WHERE session_id=? AND status='running')")
                .bind(session.as_ref()).bind(session.as_ref()).fetch_optional(&mut *tx).await?;
            if active_source.as_deref() != Some(turn_id.as_ref()) { return Err(SessionStoreError::Conflict("observed Turn is no longer active".into())); }
        }
        let through_seq = if matches!(action, SessionInputQueueAction::Pause {}) {
            Some(head_by_id_tx(&mut tx, session.as_ref()).await?.last_seq)
        } else { None };
        match action {
            SessionInputQueueAction::Remove { message_id } => self.cancel_pending_input_tx(&mut tx, session, message_id).await?,
            SessionInputQueueAction::Clear {} => {
                for item in snapshot.items.iter().filter(|item|item.input_delivery["placement"]=="queued") {
                    self.cancel_pending_input_tx(&mut tx, session, &item.message_id).await?;
                }
            }
            _ => {},
        }
        let except_message_ids=if matches!(action,SessionInputQueueAction::Pause {}) {
            snapshot.items.iter().filter(|item|item.input_delivery["placement"]!="queued").map(|item|item.message_id.clone()).collect()
        } else {Vec::new()};
        self.append_queue_change_tx(&mut tx, session, &identity, QueueChange { action: action.clone().into(), expected_version, through_seq,except_message_ids }).await?;
        let snapshot = queue_snapshot_tx(&mut tx, session).await?;
        db_write_observation.commit(tx).await?; Ok(snapshot)
    }

    async fn append_queue_change_tx(&self, tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
        identity: &str, change: QueueChange,
    ) -> Result<(), SessionStoreError> {
        let cause = latest_turn_boundary_event_tx(tx, session.as_ref()).await?;
        let append = SessionEventAppend { agent_session_id: session.clone(), event_id: new_event_id(), producer_id: "session_api".into(),
            idempotency_key: identity.into(), semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind(QUEUE_CHANGED.into()), kind_version: 1, correlation_id: session.as_ref().into(),
                causation_event_id: Some(cause), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(change)?)),
            }};
        self.append_event_tx(tx, &append, None).await?; Ok(())
    }

    pub(super) async fn cancel_pending_input_tx(&self, tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, input: &EventId) -> Result<(), SessionStoreError> {
        if session_input::input_claimed_tx(tx, session, input).await? { return Err(SessionStoreError::Conflict("input has already been claimed".into())); }
        let identity = format!("session-input-blocked:{}:cancelled_by_owner", input.as_ref());
        let append = SessionEventAppend { agent_session_id: session.clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: identity.into(), semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: SessionEventKind("message/input-blocked".into()), kind_version: 1, correlation_id: input.as_ref().into(), causation_event_id: Some(input.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"input_event_id":input,"reason":"CANCELLED_BY_OWNER"}))),
            }};
        self.append_event_tx(tx, &append, None).await?; Ok(())
    }

    pub(super) async fn hold_session_input_queue_for_stop_tx(&self, tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId,
        target: &OperationId, key: &IdempotencyKey,
    ) -> Result<(), SessionStoreError> {
        let owner=require_live_session_tx(tx,session.as_ref()).await?.owner_ref;
        let prefix=session_input::session_input_namespace(&owner,session);
        // Stop must not hydrate queue bodies, attachments or old native
        // journals before its cancellation fence and exact-owner signal.
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events input LEFT JOIN agent_payloads p ON p.payload_id=input.payload_id AND p.session_id=input.session_id \
            WHERE input.session_id=? AND input.kind='message/user-accepted' AND input.producer_id='session_api' AND input.kind_version=1 \
            AND substr(input.idempotency_key,1,length(?))=? \
            AND input.seq>COALESCE((SELECT MAX(seq) FROM agent_events WHERE session_id=input.session_id AND kind='context/cleared'),0) \
            AND COALESCE(json_extract(input.inline_json,'$.delivery_intent'),json_extract(CAST(p.body AS TEXT),'$.value.delivery_intent')) IS NOT NULL \
            AND NOT EXISTS(SELECT 1 FROM agent_turns t WHERE t.session_id=input.session_id AND t.source_message_id=input.event_id) \
            AND NOT EXISTS(SELECT 1 FROM agent_events c LEFT JOIN agent_payloads cp ON cp.payload_id=c.payload_id AND cp.session_id=c.session_id \
                WHERE c.session_id=input.session_id AND c.kind='turn/steer-accepted' AND c.kind_version=2 AND COALESCE(json_extract(c.inline_json,'$.input_event_id'),json_extract(CAST(cp.body AS TEXT),'$.value.input_event_id'))=input.event_id) \
            AND NOT EXISTS(SELECT 1 FROM agent_events c WHERE c.session_id=input.session_id AND c.event_id='session-input-blocked:'||input.event_id||':cancelled_by_owner'))")
            .bind(session.as_ref()).bind(&prefix).bind(&prefix).fetch_one(&mut **tx).await?;
        if !pending { return Ok(()); }
        // This opt-in control freezes only the existing composer batch. Inputs
        // authored later remain new user intents; no domain input is held.
        let identity = format!("session-input-queue-stop:{}:{}:{}", session.as_ref(), target.as_ref(), digest_payload(key)?.as_ref());
        let cutoff = head_by_id_tx(tx, session.as_ref()).await?.last_seq;
        self.append_queue_change_tx(tx, session, &identity, QueueChange { action: SessionInputQueueAction::Pause {}.into(),
            expected_version: None, through_seq: Some(cutoff),except_message_ids:Vec::new() }).await
    }
}

fn queue_command_key(owner: &PrincipalRef, session: &AgentSessionId, key: &IdempotencyKey) -> Result<String, SessionStoreError> {
    if key.as_ref().trim().is_empty() || key.as_ref().len() > 1024 { return Err(SessionStoreError::InvalidEvent("queue command requires a bounded identity".into())); }
    Ok(format!("session-input-queue:{}:{}:{}", owner.principal_id, session.as_ref(), digest_payload(key)?.as_ref()))
}

async fn queue_receipt_matches_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,identity:&str,
    action:&SessionInputQueueAction,expected_version:Option<u64>)->Result<bool,SessionStoreError> {
    let Some(row)=event_by_producer_key_tx(tx,"session_api",identity).await? else {return Ok(false);};
    let event=event_from_row(row)?;
    if event.agent_session_id!=*session || event.kind.0!=QUEUE_CHANGED || event.kind_version!=1 {
        return Err(SessionStoreError::IdempotencyConflict("queue key belongs to a different original fact".into()));
    }
    let payload=payload_value_for_event_tx(tx,&event).await?;
    validate_event(&event,&payload)?;
    let original:QueueChange=serde_json::from_value(payload)?;
    if original.action!=Change::Control(action.clone()) || original.expected_version!=expected_version {
        return Err(SessionStoreError::IdempotencyConflict("queue key was reused for another command".into()));
    }
    Ok(true)
}

async fn replacement_receipt_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,identity:&str,
    message_id:&EventId,content:&str,expected_version:Option<u64>)->Result<bool,SessionStoreError> {
    let Some(row)=event_by_producer_key_tx(tx,"session_api",identity).await? else {return Ok(false);};
    let event=event_from_row(row)?;
    let payload=payload_value_for_event_tx(tx,&event).await?;
    if event.agent_session_id!=*session || event.kind.0!=QUEUE_CHANGED {return Err(SessionStoreError::IdempotencyConflict("queue key belongs to another fact".into()));}
    validate_event(&event,&payload)?;validate_references_tx(tx,&event,&payload).await?;
    let change:QueueChange=serde_json::from_value(payload)?;
    if change.expected_version==expected_version && let Change::Replace {message_id:old,replacement_message_id}=change.action {
        let replacement=session_input::accepted_input_tx(tx,session,&replacement_message_id).await?;
        if old==*message_id && replacement.input.0["content"].as_str()==Some(content) {return Ok(true);}
    }
    Err(SessionStoreError::IdempotencyConflict("queue key was reused for another replacement".into()))
}

pub(super) fn validate_event(event: &SessionEventRecord, payload: &Value) -> Result<(), SessionStoreError> {
    if event.kind.0 != QUEUE_CHANGED { return Ok(()); }
    let change: QueueChange = serde_json::from_value(payload.clone())?;
    if event.kind_version != 1 || event.producer_id.as_ref() != "session_api"
        || event.correlation_id.as_ref() != event.agent_session_id.as_ref()
        || (matches!(change.action, Change::Control(SessionInputQueueAction::Pause {})) != change.through_seq.is_some())
        || change.through_seq.is_some_and(|seq| seq >= event.seq) {
        return Err(SessionStoreError::InvalidEvent("queue change has invalid original authority or cutoff".into()));
    }
    if !change.except_message_ids.is_empty() && !matches!(change.action,Change::Control(SessionInputQueueAction::Pause {})) {
        return Err(SessionStoreError::InvalidEvent("only explicit queue pause may exempt visible input".into()));
    }
    Ok(())
}

pub(super) async fn validate_references_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,payload:&Value)
    ->Result<(),SessionStoreError> {
    if event.kind.0!=QUEUE_CHANGED {return Ok(());}
    let change:QueueChange=serde_json::from_value(payload.clone())?;
    let mut ids=match &change.action {
        Change::Replace {message_id,replacement_message_id}=>vec![message_id.clone(),replacement_message_id.clone()],
        Change::Control(action)=>match action {
        SessionInputQueueAction::Remove {message_id}|SessionInputQueueAction::Promote {message_id,..}=>vec![message_id.clone()],
        SessionInputQueueAction::Reorder {message_ids}=>message_ids.clone(),
        _=>Vec::new(),
        },
    };
    ids.extend(change.except_message_ids.iter().cloned());
    if ids.iter().collect::<BTreeSet<_>>().len()!=ids.len() {return Err(SessionStoreError::InvalidPayload("queue change duplicates an input identity".into()));}
    for id in ids {
        let original=session_input::accepted_input_tx(tx,&event.agent_session_id,&id).await?;
        if original.event.seq>=event.seq || desktop_delivery_intent(&original.input.0)?.is_none() {
            return Err(SessionStoreError::InvalidEvent("queue change lost its prior ordinary input source".into()));
        }
    }
    if let Change::Replace {message_id,replacement_message_id}=&change.action {
        let original=session_input::accepted_input_tx(tx,&event.agent_session_id,message_id).await?;
        let replacement=session_input::accepted_input_tx(tx,&event.agent_session_id,replacement_message_id).await?;
        let cancelled=session_input::cancelled_input_tx(tx,&original.event).await?
            .ok_or_else(||SessionStoreError::InvalidEvent("replacement lost original cancellation".into()))?;
        let mut inherited=replacement.input.0.clone();inherited["content"]=original.input.0["content"].clone();
        if cancelled.seq>=replacement.event.seq || inherited!=original.input.0 || replacement.event.seq<=original.event.seq {
            return Err(SessionStoreError::InvalidEvent("replacement lost its original envelope or writer order".into()));
        }
    }
    if let Change::Control(SessionInputQueueAction::Promote {turn_id,..})=change.action {
        validate_delivery_target_tx(tx,&event.agent_session_id,&SessionInputDeliveryIntent::PreferCurrent {turn_id}).await?;
    }
    Ok(())
}
