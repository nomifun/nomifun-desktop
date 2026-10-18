//! Exact Channel presentation receipts in the original Event/Payload chain.
//! They route replies to a delivered question; they never answer it or grant
//! execution. No Channel table, outbox, or model-projected message is involved.
use super::*;
use nomifun_agent_contracts::{ChannelInputQuestionDeliveredV1, CHANNEL_INPUT_QUESTION_DELIVERED};

fn delivery_key(session:&AgentSessionId, platform:&str,plugin:&str,chat:&str,message:&str) -> Result<String,SessionStoreError> {
    Ok(format!("channel-question:{}:{}",session.as_ref(),digest_payload(&(platform,plugin,chat,message))?.as_ref()))
}

impl AgentSessionStore {
    pub async fn record_channel_input_question_delivery(&self,owner:&PrincipalRef,delivery:ChannelInputQuestionDeliveredV1)
        ->Result<SessionEventAck,SessionStoreError> {
        delivery.validate().map_err(SessionStoreError::InvalidPayload)?;
        if &delivery.owner_ref != owner {return Err(SessionStoreError::ExecutionFenced);}
        let session=&delivery.agent_session_id;
        let key=delivery_key(session,&delivery.platform,&delivery.channel_plugin_id,&delivery.chat_id,&delivery.provider_message_id)?;
        let mut tx=self.begin_write_transaction().await?;
        let live=live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(row)=event_by_event_id_tx(&mut tx,&key).await? {
            let event=event_from_row(row)?;
            let previous=read_delivery_tx(&mut tx,&event).await?;
            if previous!=delivery {return Err(SessionStoreError::IdempotencyConflict("Channel message was already delivered for another question".into()));}
            let ack=event_ack(&event);tx.commit().await?;return Ok(ack);
        }
        // The platform ACK can arrive after another surface answered the
        // question. Record its original presentation identity even then; this
        // fact never grants execution and the steering writer still checks the
        // original question association before accepting a reply.
        let wait_id:String=sqlx::query_scalar("SELECT event_id FROM agent_events WHERE session_id=? AND seq=?")
            .bind(session.as_ref()).bind(as_i64(delivery.question_seq,"Channel question sequence")?).fetch_one(&mut *tx).await?;
        let append=SessionEventAppend {agent_session_id:session.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(CHANNEL_INPUT_QUESTION_DELIVERED.into()),kind_version:1,
                correlation_id:delivery.operation_id.as_ref().into(),causation_event_id:Some(wait_id.into()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(&delivery)?))}};
        let ack=required_ack(self.append_event_tx(&mut tx,&append,None).await?)?;
        tx.commit().await?;Ok(ack)
    }

    /// Return the original immutable target even after another question/Turn
    /// starts. Only the existing steering writer decides if it is still valid.
    pub async fn resolve_channel_input_question_delivery(&self,owner:&PrincipalRef,session:&AgentSessionId,
        platform:&str,plugin:&str,chat:&str,message:&str) ->Result<Option<ChannelInputQuestionDeliveredV1>,SessionStoreError> {
        let key=delivery_key(session,platform,plugin,chat,message)?;
        let mut tx=self.pool.begin().await?;
        let live=live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let Some(row)=event_by_event_id_tx(&mut tx,&key).await? else{return Ok(None);};
        let delivery=read_delivery_tx(&mut tx,&event_from_row(row)?).await?;
        if delivery.agent_session_id!=*session || &delivery.owner_ref!=owner || delivery.platform!=platform
            || delivery.channel_plugin_id!=plugin || delivery.chat_id!=chat || delivery.provider_message_id!=message {
            return Err(SessionStoreError::InvalidEvent("Channel delivery lookup differs from its exact presentation identity".into()));
        }
        tx.commit().await?;Ok(Some(delivery))
    }
}

async fn read_delivery_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord) ->Result<ChannelInputQuestionDeliveredV1,SessionStoreError> {
    let payload=payload_value_for_event_tx(tx,event).await?;
    let delivery:ChannelInputQuestionDeliveredV1=serde_json::from_value(payload)?;
    validate_event_tx(tx,event,&delivery).await?;Ok(delivery)
}

pub(super) async fn validate_event_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,delivery:&ChannelInputQuestionDeliveredV1)
    ->Result<(),SessionStoreError> {
    delivery.validate().map_err(SessionStoreError::InvalidPayload)?;
    if event.kind.0!=CHANNEL_INPUT_QUESTION_DELIVERED || event.kind_version!=1 || event.producer_id.as_ref()!="session_api"
        || event.agent_session_id!=delivery.agent_session_id || event.correlation_id.as_ref()!=delivery.operation_id.as_ref()
        || event.event_id.as_ref()!=delivery_key(&delivery.agent_session_id,&delivery.platform,&delivery.channel_plugin_id,&delivery.chat_id,&delivery.provider_message_id)? {
        return Err(SessionStoreError::InvalidEvent("Channel delivery event identity differs".into()));
    }
    let live=live_session_by_id_tx(tx,event.agent_session_id.as_ref()).await?;
    if live.owner_ref!=delivery.owner_ref {return Err(SessionStoreError::ExecutionFenced);}
    let wait_id=event.causation_event_id.as_ref().ok_or_else(||SessionStoreError::InvalidEvent("Channel delivery has no original question".into()))?;
    let wait=event_by_event_id_tx(tx,wait_id.as_ref()).await?.map(event_from_row).transpose()?
        .ok_or_else(||SessionStoreError::InvalidEvent("Channel delivery original question is missing".into()))?;
    let payload=payload_value_for_event_tx(tx,&wait).await?;
    let request:nomifun_agent_contracts::AgentUserInputRequest=serde_json::from_value(
        payload.pointer("/event/request").cloned().ok_or_else(||SessionStoreError::InvalidEvent("Channel source question is missing".into()))?)?;
    request.validate().map_err(SessionStoreError::InvalidPayload)?;
    if wait.agent_session_id!=event.agent_session_id || wait.seq!=delivery.question_seq || wait.seq>=event.seq
        || wait.kind.0!="runtime/progress-recorded" || wait.kind_version!=1 || wait.producer_id.as_ref()!="runtime_supervisor"
        || wait.correlation_id!=event.correlation_id || payload.pointer("/event/event").and_then(Value::as_str)!=Some("user_input_requested")
        || request.request_id.as_ref()!=delivery.request_id {
        return Err(SessionStoreError::InvalidEvent("Channel delivery differs from its committed source question".into()));
    }
    let identity:Option<(String,Option<String>,Option<String>)>=sqlx::query_as("SELECT started_event_id,source_message_id,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(event.agent_session_id.as_ref()).bind(delivery.operation_id.as_ref()).fetch_optional(&mut **tx).await?;
    let Some((start_id,source,terminal_id))=identity else {return Err(SessionStoreError::InvalidEvent("Channel delivery original Turn is missing".into()));};
    if source.as_deref()!=Some(delivery.source_message_id.as_ref()) {
        return Err(SessionStoreError::InvalidEvent("Channel delivery differs from its original Turn source".into()));
    }
    let start=event_by_event_id_tx(tx,&start_id).await?.map(event_from_row).transpose()?
        .ok_or_else(||SessionStoreError::InvalidEvent("Channel delivery original Turn start is missing".into()))?;
    let start_payload=payload_value_for_event_tx(tx,&start).await?;
    if start.agent_session_id!=event.agent_session_id || start.kind.0!="turn/started" || start.kind_version!=1
        || start.correlation_id!=event.correlation_id || start.seq>=wait.seq
        || !wait.causation_event_id.as_ref().is_some_and(|cause|cause==&start.event_id || cause.as_ref()==delivery.source_message_id.as_ref())
        || start_payload.get("source_message_id").and_then(Value::as_str)!=source.as_deref() {
        return Err(SessionStoreError::InvalidEvent("Channel delivery original Turn start differs".into()));
    }
    if let Some(terminal_id)=terminal_id {
        let terminal=event_by_event_id_tx(tx,&terminal_id).await?.map(event_from_row).transpose()?
            .ok_or_else(||SessionStoreError::InvalidEvent("Channel delivery original Turn terminal is missing".into()))?;
        if terminal.agent_session_id!=event.agent_session_id || terminal.correlation_id!=event.correlation_id
            || !is_turn_terminal_kind(&terminal.kind.0) || terminal.seq<=wait.seq {
            return Err(SessionStoreError::InvalidEvent("Channel delivery source question is outside its original Turn".into()));
        }
    }

    Ok(())
}
