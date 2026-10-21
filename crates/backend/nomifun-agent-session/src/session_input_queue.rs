//! Queue control is a projection of accepted input and reference-only changes.
//! It never owns another message body, task ledger, or runtime.
use super::*;
use nomifun_agent_contracts::{SessionInputDeliveryIntent, SessionInputQueueAction, SessionInputQueueChangedV1 as QueueChange};
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

pub(super) async fn queue_state_tx(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId) -> Result<QueueState, SessionStoreError> {
    let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    let rows = sqlx::query_as::<_, StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id FROM agent_events WHERE session_id=? AND kind='message/input-queue-changed' AND seq>? ORDER BY seq")
        .bind(session.as_ref()).bind(floor).fetch_all(&mut **tx).await?;
    let mut state = QueueState { version: as_u64(floor, "queue context floor")?, ..Default::default() };
    for row in rows {
        let event = event_from_row(row)?;
        let payload=payload_value_for_event_tx(tx, &event).await?;
        validate_event(&event,&payload)?;
        validate_references_tx(tx,&event,&payload).await?;
        let change: QueueChange = serde_json::from_value(payload)?;
        match change.action {
            SessionInputQueueAction::Pause {} => {
                let cutoff = change.through_seq.filter(|seq| *seq < event.seq)
                    .ok_or_else(|| SessionStoreError::InvalidPayload("queue hold has no prior input cutoff".into()))?;
                state.held_through = Some(state.held_through.unwrap_or(0).max(cutoff));
                state.released.clear();
            }
            SessionInputQueueAction::Resume {} => state.held_through = None,
            SessionInputQueueAction::Reorder { message_ids } => state.order = message_ids,
            SessionInputQueueAction::Promote { message_id, turn_id } => {
                state.released.insert(message_id.clone());
                state.delivery_overrides.insert(message_id, SessionInputDeliveryIntent::PreferCurrent { turn_id });
            }
            SessionInputQueueAction::Remove { .. } | SessionInputQueueAction::Clear {} => {},
        }
        state.version = event.seq;
    }
    Ok(state)
}

impl QueueState {
    pub(super) fn held(&self, input: &PendingSessionInput) -> Result<bool, SessionStoreError> {
        self.held_metadata(&input.event.event_id,input.event.seq,&input.input.0)
    }
    fn held_metadata(&self,id:&EventId,seq:u64,routing:&Value)->Result<bool,SessionStoreError> {
        Ok(desktop_delivery_intent(routing)?.is_some() && !self.released.contains(id)
            && self.held_through.is_some_and(|cutoff| seq<=cutoff))
    }
    pub(super) fn intent(&self, input: &PendingSessionInput) -> Result<Option<SessionInputDeliveryIntent>, SessionStoreError> {
        self.intent_metadata(&input.event.event_id,&input.input.0)
    }
    fn intent_metadata(&self,id:&EventId,routing:&Value)->Result<Option<SessionInputDeliveryIntent>,SessionStoreError> {
        Ok(self.delivery_overrides.get(id).cloned().or(desktop_delivery_intent(routing)?))
    }
    pub(super) fn sort(&self, inputs: &mut [PendingSessionInput]) -> Result<(),SessionStoreError> {
        let rank: BTreeMap<_, _> = self.order.iter().enumerate().map(|(index,id)| (id,index)).collect();
        let mut slots=Vec::new();let mut composer=Vec::new();
        for (index,input) in inputs.iter().enumerate() {
            if desktop_delivery_intent(&input.input.0)?.is_some() {slots.push(index);composer.push(input.clone());}
        }
        composer.sort_by_key(|input| (rank.get(&input.event.event_id).copied().unwrap_or(usize::MAX), input.event.seq));
        for (index,input) in slots.into_iter().zip(composer) {inputs[index]=input;}
        Ok(())
    }
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
    let state = queue_state_tx(tx, session).await?;
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
    let version = state.version.max(as_u64(changed,"queue mutation sequence")?);
    let mut items = Vec::new();
    for original in session_input::pending_inputs_tx(tx,session,None,true).await? {
        let Some(delivery) = state.intent(&original)? else { continue; };
        let (input_delivery, _) = session_input::delivery_state_tx(tx, session, &original.event.event_id).await?;
        items.push(SessionInputQueueItem { message_id: original.event.event_id.clone(), held: state.held(&original)?,
            input: original.input, delivery, input_delivery });
    }
    let rank: BTreeMap<_, _> = state.order.iter().enumerate().map(|(index,id)| (id,index)).collect();
    // The input query already supplies canonical order for newly accepted items.
    items.sort_by_key(|item| rank.get(&item.message_id).copied().unwrap_or(usize::MAX));
    Ok(SessionInputQueueSnapshot { version, items })
}

impl AgentSessionStore {
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
        let mut tx = self.begin_write_transaction().await?;
        if require_live_session_tx(&mut tx, session.as_ref()).await?.owner_ref != *owner { return Err(SessionStoreError::ExecutionFenced); }
        let identity = queue_command_key(owner, session, key)?;
        if queue_receipt_matches_tx(&mut tx,session,&identity,action,expected_version).await? {
            let snapshot = queue_snapshot_tx(&mut tx, session).await?;
            tx.commit().await?; return Ok(snapshot);
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
                for item in &snapshot.items { self.cancel_pending_input_tx(&mut tx, session, &item.message_id).await?; }
            }
            _ => {},
        }
        self.append_queue_change_tx(&mut tx, session, &identity, QueueChange { action: action.clone(), expected_version, through_seq }).await?;
        let snapshot = queue_snapshot_tx(&mut tx, session).await?;
        tx.commit().await?; Ok(snapshot)
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

    async fn cancel_pending_input_tx(&self, tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, input: &EventId) -> Result<(), SessionStoreError> {
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
        self.append_queue_change_tx(tx, session, &identity, QueueChange { action: SessionInputQueueAction::Pause {},
            expected_version: None, through_seq: Some(cutoff) }).await
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
    if original.action!=*action || original.expected_version!=expected_version {
        return Err(SessionStoreError::IdempotencyConflict("queue key was reused for another command".into()));
    }
    Ok(true)
}

pub(super) fn validate_event(event: &SessionEventRecord, payload: &Value) -> Result<(), SessionStoreError> {
    if event.kind.0 != QUEUE_CHANGED { return Ok(()); }
    let change: QueueChange = serde_json::from_value(payload.clone())?;
    if event.kind_version != 1 || event.producer_id.as_ref() != "session_api"
        || event.correlation_id.as_ref() != event.agent_session_id.as_ref()
        || (matches!(change.action, SessionInputQueueAction::Pause {}) != change.through_seq.is_some())
        || change.through_seq.is_some_and(|seq| seq >= event.seq) {
        return Err(SessionStoreError::InvalidEvent("queue change has invalid original authority or cutoff".into()));
    }
    Ok(())
}

pub(super) async fn validate_references_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,payload:&Value)
    ->Result<(),SessionStoreError> {
    if event.kind.0!=QUEUE_CHANGED {return Ok(());}
    let change:QueueChange=serde_json::from_value(payload.clone())?;
    let ids=match &change.action {
        SessionInputQueueAction::Remove {message_id}|SessionInputQueueAction::Promote {message_id,..}=>vec![message_id.clone()],
        SessionInputQueueAction::Reorder {message_ids}=>message_ids.clone(),
        _=>Vec::new(),
    };
    if ids.iter().collect::<BTreeSet<_>>().len()!=ids.len() {return Err(SessionStoreError::InvalidPayload("queue change duplicates an input identity".into()));}
    for id in ids {
        let original=session_input::accepted_input_tx(tx,&event.agent_session_id,&id).await?;
        if original.event.seq>=event.seq || desktop_delivery_intent(&original.input.0)?.is_none() {
            return Err(SessionStoreError::InvalidEvent("queue change lost its prior ordinary input source".into()));
        }
    }
    if let SessionInputQueueAction::Promote {turn_id,..}=change.action {
        validate_delivery_target_tx(tx,&event.agent_session_id,&SessionInputDeliveryIntent::PreferCurrent {turn_id}).await?;
    }
    Ok(())
}
