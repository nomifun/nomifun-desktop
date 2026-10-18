//! Canonical transport observations for accepted Channel inputs. No input
//! body, execution grant or outbound retry queue is stored here.
use super::*;
use nomifun_agent_contracts::{ChannelInputSource,ChannelInputDeliverySubject,ChannelInputDeliveryPhase,
    ChannelInputDeliveryStateV1,CHANNEL_INPUT_DELIVERY_STATE};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum ChannelInputDeliveryDisposition { Ready,Confirmed,Unknown }

#[derive(Clone,Debug)]
pub struct RecoverableChannelInput {
    pub agent_session_id:AgentSessionId,
    pub input_event_id:EventId,
    pub source:ChannelInputSource,
}

pub(super) async fn validate_input_source_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,value:&Value)
    ->Result<(),SessionStoreError> {
    let Some(source)=value.get("channel_source") else {return Ok(());};
    let source:ChannelInputSource=serde_json::from_value(source.clone())?;
    source.validate().map_err(SessionStoreError::InvalidPayload)?;
    let authorized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channel_sessions s JOIN channel_users u ON u.channel_user_id=s.channel_user_id \
        JOIN channel_plugins p ON p.channel_plugin_id=s.channel_plugin_id WHERE s.channel_session_id=? AND s.conversation_id=? \
        AND s.chat_id=? AND s.channel_plugin_id=? AND s.chat_kind IN ('direct','group') AND u.authorization_kind='approved' \
        AND u.platform_type=? AND p.type=? AND p.enabled=1 AND p.owner_domain='companion')")
        .bind(&source.channel_session_id).bind(session.as_ref()).bind(&source.chat_id).bind(&source.channel_plugin_id)
        .bind(&source.platform).bind(&source.platform).fetch_one(&mut **tx).await?;
    if !authorized {return Err(SessionStoreError::ExecutionFenced);}
    Ok(())
}

fn scope(state:&ChannelInputDeliveryStateV1)->Result<String,SessionStoreError> {
    Ok(format!("channel-delivery:{}:{}",state.agent_session_id.as_ref(),digest_payload(&(
        &state.operation_id,state.source.destination(),&state.subject))?.as_ref()))
}
fn phase_name(phase:&ChannelInputDeliveryPhase)->&'static str {match phase {
    ChannelInputDeliveryPhase::Started=>"started",ChannelInputDeliveryPhase::Confirmed {..}=>"confirmed",ChannelInputDeliveryPhase::Unknown=>"unknown",
}}

async fn read_state_tx(tx:&mut Transaction<'_,Sqlite>,key:&str)->Result<Option<ChannelInputDeliveryStateV1>,SessionStoreError> {
    let Some(row)=event_by_event_id_tx(tx,key).await? else {return Ok(None);};
    let event=event_from_row(row)?;let state=serde_json::from_value(payload_value_for_event_tx(tx,&event).await?)?;
    validate_event_tx(tx,&event,&state).await?;Ok(Some(state))
}

async fn state_for_input_tx(tx:&mut Transaction<'_,Sqlite>,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,
    turn_source:&str,subject:ChannelInputDeliverySubject)->Result<ChannelInputDeliveryStateV1,SessionStoreError> {
    let original=session_input::accepted_input_tx(tx,session,input).await?;
    if live_session_by_id_tx(tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
    let source:ChannelInputSource=serde_json::from_value(original.input.0.get("channel_source").cloned()
        .ok_or_else(||SessionStoreError::InvalidEvent("accepted input has no trusted Channel source".into()))?)?;
    let target:Option<(String,Option<String>)>=sqlx::query_as("SELECT t.operation_id,t.terminal_event_id FROM agent_turns t WHERE t.session_id=? AND t.source_message_id=? AND \
        (t.source_message_id=? OR EXISTS(SELECT 1 FROM agent_events c LEFT JOIN agent_payloads p ON p.payload_id=c.payload_id AND p.session_id=c.session_id \
         WHERE c.session_id=t.session_id AND c.correlation_id=t.operation_id AND c.kind='turn/steer-accepted' AND c.kind_version=2 \
         AND COALESCE(json_extract(c.inline_json,'$.input_event_id'),json_extract(CAST(p.body AS TEXT),'$.value.input_event_id'))=?))")
        .bind(session.as_ref()).bind(turn_source).bind(input.as_ref()).bind(input.as_ref()).fetch_optional(&mut **tx).await?;
    let Some((operation,terminal))=target else {return Err(SessionStoreError::ExecutionFenced);};
    match &subject {
        ChannelInputDeliverySubject::Terminal if terminal.is_none()=>return Err(SessionStoreError::ExecutionFenced),
        ChannelInputDeliverySubject::Question {question_seq}=>{
            session_input::question_source_tx(tx,session,turn_source,*question_seq).await?;
        },_=>{},
    }
    let state=ChannelInputDeliveryStateV1 {agent_session_id:session.clone(),owner_ref:owner.clone(),input_event_id:input.clone(),
        operation_id:operation.into(),source,subject,phase:ChannelInputDeliveryPhase::Started};
    state.validate().map_err(SessionStoreError::InvalidPayload)?;Ok(state)
}

async fn question_receipts_tx(tx:&mut Transaction<'_,Sqlite>,state:&ChannelInputDeliveryStateV1)->Result<Vec<String>,SessionStoreError> {
    let ChannelInputDeliverySubject::Question {question_seq}=state.subject else {return Ok(Vec::new());};
    let rows=sqlx::query_as::<_,StoredEventRow>("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
        FROM agent_events WHERE session_id=? AND kind='channel/input-question-delivered' AND correlation_id=?")
        .bind(state.agent_session_id.as_ref()).bind(state.operation_id.as_ref()).fetch_all(&mut **tx).await?;
    let mut ids=Vec::new();
    for row in rows {
        let event=event_from_row(row)?;let proof:nomifun_agent_contracts::ChannelInputQuestionDeliveredV1=serde_json::from_value(payload_value_for_event_tx(tx,&event).await?)?;
        channel_input_delivery::validate_event_tx(tx,&event,&proof).await?;
        if proof.question_seq==question_seq && proof.platform==state.source.platform && proof.channel_plugin_id==state.source.channel_plugin_id
            && proof.chat_id==state.source.chat_id {ids.push(proof.provider_message_id);}
    }
    Ok(ids)
}

impl AgentSessionStore {
    async fn append_channel_delivery_tx(&self,tx:&mut Transaction<'_,Sqlite>,state:ChannelInputDeliveryStateV1)->Result<(),SessionStoreError> {
        let key=format!("{}:{}",scope(&state)?,phase_name(&state.phase));
        let event=SessionEventAppend {agent_session_id:state.agent_session_id.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(CHANNEL_INPUT_DELIVERY_STATE.into()),kind_version:1,
                correlation_id:state.operation_id.as_ref().into(),causation_event_id:Some(state.input_event_id.clone()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(state)?))}};
        self.append_event_tx(tx,&event,None).await?;Ok(())
    }

    /// A durable Started observation is never retried after a process cut. A
    /// confirmed question's existing provider proof may settle that cut safely.
    pub async fn begin_channel_input_delivery(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,turn_source:&str,
        subject:ChannelInputDeliverySubject)->Result<ChannelInputDeliveryDisposition,SessionStoreError> {
        let mut tx=self.begin_write_transaction().await?;
        let mut state=state_for_input_tx(&mut tx,owner,session,input,turn_source,subject).await?;
        let key=scope(&state)?;
        if read_state_tx(&mut tx,&format!("{key}:confirmed")).await?.is_some() {tx.commit().await?;return Ok(ChannelInputDeliveryDisposition::Confirmed);}
        let proofs=question_receipts_tx(&mut tx,&state).await?;
        if !proofs.is_empty() {
            state.phase=ChannelInputDeliveryPhase::Confirmed {provider_message_ids:proofs};
            self.append_channel_delivery_tx(&mut tx,state).await?;tx.commit().await?;return Ok(ChannelInputDeliveryDisposition::Confirmed);
        }
        if read_state_tx(&mut tx,&format!("{key}:started")).await?.is_some() || read_state_tx(&mut tx,&format!("{key}:unknown")).await?.is_some() {
            if read_state_tx(&mut tx,&format!("{key}:unknown")).await?.is_none() {state.phase=ChannelInputDeliveryPhase::Unknown;self.append_channel_delivery_tx(&mut tx,state).await?;}
            tx.commit().await?;return Ok(ChannelInputDeliveryDisposition::Unknown);
        }
        let original=session_input::accepted_input_tx(&mut tx,session,input).await?;
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if original.event.seq<=as_u64(floor,"Channel delivery context floor")? {return Err(SessionStoreError::ExecutionFenced);}
        validate_input_source_tx(&mut tx,session,&original.input.0).await?;
        self.append_channel_delivery_tx(&mut tx,state).await?;tx.commit().await?;Ok(ChannelInputDeliveryDisposition::Ready)
    }

    pub async fn finish_channel_input_delivery(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,turn_source:&str,
        subject:ChannelInputDeliverySubject,confirmed:bool,provider_message_ids:Vec<String>)->Result<(),SessionStoreError> {
        let mut tx=self.begin_write_transaction().await?;
        let mut state=state_for_input_tx(&mut tx,owner,session,input,turn_source,subject).await?;let key=scope(&state)?;
        if read_state_tx(&mut tx,&format!("{key}:confirmed")).await?.is_some() {tx.commit().await?;return Ok(());}
        if read_state_tx(&mut tx,&format!("{key}:started")).await?.is_none() || read_state_tx(&mut tx,&format!("{key}:unknown")).await?.is_some() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        state.phase=if confirmed {ChannelInputDeliveryPhase::Confirmed {provider_message_ids}} else {ChannelInputDeliveryPhase::Unknown};
        self.append_channel_delivery_tx(&mut tx,state).await?;tx.commit().await?;Ok(())
    }

    pub async fn recoverable_channel_inputs(&self,owner:&PrincipalRef)->Result<Vec<RecoverableChannelInput>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let mut inputs=Vec::new();let mut after_session=String::new();let mut after_seq=0_i64;
        let namespace=format!("session-input:{}:{}:",owner.principal_kind,owner.principal_id);
        loop {
            // Exclude acknowledged/unknown original inputs before resolving
            // their bodies. Aliases of the same Turn/receiver share its one
            // delivery scope. Only candidate identities cross this query.
            let rows:Vec<(String,String,i64)>=sqlx::query_as("WITH owner_sessions AS MATERIALIZED ( \
                SELECT agent_session_id FROM agent_sessions WHERE state='live' \
                    AND json_extract(owner_ref_json,'$.principal_kind')=? AND json_extract(owner_ref_json,'$.principal_id')=?), \
                delivery_payload AS (SELECT d.session_id,d.correlation_id,COALESCE(d.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')) AS body \
                    FROM agent_events d JOIN owner_sessions s ON s.agent_session_id=d.session_id \
                    LEFT JOIN agent_payloads p ON p.payload_id=d.payload_id AND p.session_id=d.session_id \
                    WHERE d.kind='channel/input-delivery-state' AND d.kind_version=1 AND d.producer_id='session_api'), \
                delivery AS MATERIALIZED (SELECT session_id,correlation_id,json_extract(body,'$.input_event_id') AS input_id, \
                    json_extract(body,'$.source.platform') AS platform,json_extract(body,'$.source.channel_plugin_id') AS plugin_id, \
                    json_extract(body,'$.source.chat_id') AS chat_id FROM delivery_payload \
                    WHERE json_extract(body,'$.subject.kind')='terminal' AND json_extract(body,'$.phase.state') IN ('confirmed','unknown')), \
                claims AS MATERIALIZED (SELECT t.session_id,t.source_message_id AS input_id,t.operation_id FROM agent_turns t \
                    JOIN owner_sessions s ON s.agent_session_id=t.session_id WHERE t.source_message_id IS NOT NULL \
                    UNION ALL SELECT c.session_id,COALESCE(json_extract(c.inline_json,'$.input_event_id'), \
                        json_extract(CAST(p.body AS TEXT),'$.value.input_event_id')),c.correlation_id \
                    FROM agent_events c JOIN owner_sessions s ON s.agent_session_id=c.session_id \
                    LEFT JOIN agent_payloads p ON p.payload_id=c.payload_id AND p.session_id=c.session_id \
                    WHERE c.kind='turn/steer-accepted' AND c.kind_version=2) \
                SELECT i.session_id,i.event_id,i.seq FROM agent_events i JOIN owner_sessions s ON s.agent_session_id=i.session_id \
                LEFT JOIN agent_payloads ip ON ip.payload_id=i.payload_id AND ip.session_id=i.session_id \
                WHERE i.kind='message/user-accepted' AND i.kind_version=1 AND i.producer_id='session_api' \
                AND substr(i.idempotency_key,1,length(?||i.session_id||':'))=(?||i.session_id||':') \
                AND (i.session_id>? OR (i.session_id=? AND i.seq>?)) \
                AND i.seq>COALESCE((SELECT MAX(seq) FROM agent_events c WHERE c.session_id=i.session_id AND c.kind='context/cleared'),0) \
                AND CASE WHEN EXISTS(SELECT 1 FROM delivery d WHERE d.session_id=i.session_id AND d.input_id=i.event_id) THEN 0 ELSE \
                    COALESCE(json_type(i.inline_json,'$.channel_source'),json_type(CAST(ip.body AS TEXT),'$.value.channel_source')) IS NOT NULL \
                    AND NOT EXISTS(SELECT 1 FROM claims c JOIN delivery d ON d.session_id=c.session_id AND d.correlation_id=c.operation_id \
                        WHERE c.session_id=i.session_id AND c.input_id=i.event_id \
                        AND d.platform=COALESCE(json_extract(i.inline_json,'$.channel_source.platform'),json_extract(CAST(ip.body AS TEXT),'$.value.channel_source.platform')) \
                        AND d.plugin_id=COALESCE(json_extract(i.inline_json,'$.channel_source.channel_plugin_id'),json_extract(CAST(ip.body AS TEXT),'$.value.channel_source.channel_plugin_id')) \
                        AND d.chat_id=COALESCE(json_extract(i.inline_json,'$.channel_source.chat_id'),json_extract(CAST(ip.body AS TEXT),'$.value.channel_source.chat_id'))) END \
                ORDER BY i.session_id,i.seq LIMIT 64")
                .bind(&owner.principal_kind).bind(&owner.principal_id).bind(&namespace).bind(&namespace).bind(&after_session).bind(&after_session).bind(after_seq)
                .fetch_all(&mut *tx).await?;
            if rows.is_empty() {break;}
            for(session,id,seq)in rows {
                after_session=session.clone();after_seq=seq;
                let session:AgentSessionId=session.into();let input:EventId=id.into();
                let original=session_input::accepted_input_tx(&mut tx,&session,&input).await?;
                let source=ChannelInputSource::from_input(&original.input.0).map_err(SessionStoreError::InvalidPayload)?
                    .ok_or_else(||SessionStoreError::InvalidPayload("Channel source is missing".into()))?;
                let(delivery,_)=session_input::delivery_state_tx(&mut tx,&session,&input).await?;
                if matches!(delivery["state"].as_str(),Some("blocked"|"discarded")) {continue;}
                inputs.push(RecoverableChannelInput {agent_session_id:session,input_event_id:input,source});
            }
        }
        tx.commit().await?;Ok(inputs)
    }

}

pub(super) async fn validate_event_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,state:&ChannelInputDeliveryStateV1)->Result<(),SessionStoreError> {
    state.validate().map_err(SessionStoreError::InvalidPayload)?;
    let original=session_input::accepted_input_tx(tx,&state.agent_session_id,&state.input_event_id).await?;
    if event.kind.0!=CHANNEL_INPUT_DELIVERY_STATE || event.kind_version!=1 || event.producer_id.as_ref()!="session_api"
        || event.agent_session_id!=state.agent_session_id || event.correlation_id.as_ref()!=state.operation_id.as_ref()
        || event.causation_event_id.as_ref()!=Some(&state.input_event_id) || original.event.seq>=event.seq
        || event.event_id.as_ref()!=format!("{}:{}",scope(state)?,phase_name(&state.phase))
        || original.input.0.get("channel_source")!=Some(&serde_json::to_value(&state.source)?)
        || live_session_by_id_tx(tx,state.agent_session_id.as_ref()).await?.owner_ref!=state.owner_ref {
        return Err(SessionStoreError::InvalidEvent("Channel delivery state differs from its original accepted input".into()));
    }
    let turn_source:String=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(state.agent_session_id.as_ref()).bind(state.operation_id.as_ref()).fetch_one(&mut **tx).await?;
    let expected=state_for_input_tx(tx,&state.owner_ref,&state.agent_session_id,&state.input_event_id,&turn_source,state.subject.clone()).await?;
    if expected.operation_id!=state.operation_id {return Err(SessionStoreError::ExecutionFenced);}
    Ok(())
}
