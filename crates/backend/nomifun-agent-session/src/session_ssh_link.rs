//! Exact host resource-close facts in the canonical Session chain. An old
//! process's missing pool is still unknown; these readers never dial/adopt it.
use super::*;
use nomifun_agent_contracts::{SessionSshLinkCloseV1,SessionSshLinkClosedV1,SESSION_SSH_LINK_CLOSE_REQUESTED,SESSION_SSH_LINK_CLOSED};

fn request_key(close:&SessionSshLinkCloseV1)->String {
    format!("ssh-link-close:{}:{}:{}:{}",close.agent_session_id.as_ref(),close.pool_instance_id,close.ssh_host_id,close.instance_id)
}
fn result_key(close:&SessionSshLinkCloseV1)->String {format!("{}:settled",request_key(close))}

pub(super) async fn validate_event_tx(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,value:&Value)->Result<(),SessionStoreError> {
    let (close,closed)=match event.kind.0.as_str() {
        SESSION_SSH_LINK_CLOSE_REQUESTED=>(serde_json::from_value::<SessionSshLinkCloseV1>(value.clone())?,None),
        SESSION_SSH_LINK_CLOSED=>{let closed:SessionSshLinkClosedV1=serde_json::from_value(value.clone())?;
            closed.validate().map_err(SessionStoreError::InvalidPayload)?;(closed.close.clone(),Some(closed))},
        _=>return Ok(()),
    };
    close.validate().map_err(SessionStoreError::InvalidPayload)?;
    let live=live_from_row(session_row_by_id_tx(tx,event.agent_session_id.as_ref()).await?)?;
    let key=if closed.is_some(){result_key(&close)}else{request_key(&close)};
    if close.agent_session_id!=event.agent_session_id||close.owner_ref!=live.owner_ref||event.producer_id.as_ref()!="session_api"
        ||event.kind_version!=1||event.event_id.as_ref()!=key||event.correlation_id.as_ref()!=event.agent_session_id.as_ref() {
        return Err(SessionStoreError::ExecutionFenced);
    }
    let cause=event.causation_event_id.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
    let source=event_by_event_id_tx(tx,cause.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
    if source.agent_session_id!=event.agent_session_id||source.seq>=event.seq {return Err(SessionStoreError::ExecutionFenced);}
    if closed.is_some() {
        let original=payload_value_for_event_tx(tx,&source).await?;
        if source.kind.0!=SESSION_SSH_LINK_CLOSE_REQUESTED||source.event_id.as_ref()!=request_key(&close)
            ||serde_json::from_value::<SessionSshLinkCloseV1>(original.clone())?!=close {
            return Err(SessionStoreError::ExecutionFenced);
        }
        Box::pin(validate_event_tx(tx,&source,&original)).await?;
    } else if source.kind.0!="session/opening" {return Err(SessionStoreError::ExecutionFenced);}
    Ok(())
}

impl AgentSessionStore {
    pub async fn record_session_ssh_link_close_requested(&self,owner:&PrincipalRef,close:SessionSshLinkCloseV1)
        ->Result<SessionEventAck,SessionStoreError> {
        close.validate().map_err(SessionStoreError::InvalidPayload)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let live=live_from_row(session_row_by_id_tx(&mut tx,close.agent_session_id.as_ref()).await?)?;
        if &close.owner_ref!=owner||live.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let source:EventId=sqlx::query_scalar::<_,String>("SELECT event_id FROM agent_events WHERE session_id=? AND kind='session/opening' ORDER BY seq LIMIT 1")
            .bind(close.agent_session_id.as_ref()).fetch_one(&mut *tx).await?.into();
        let key=request_key(&close);
        let append=SessionEventAppend {agent_session_id:close.agent_session_id.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(SESSION_SSH_LINK_CLOSE_REQUESTED.into()),kind_version:1,
                correlation_id:close.agent_session_id.as_ref().into(),causation_event_id:Some(source),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(close)?))}};
        let ack=required_ack(self.append_event_tx_with_policy(&mut tx,&append,None,AppendSessionStatePolicy::SshLinkClose).await?)?;
        db_write_observation.commit(tx).await?;Ok(ack)
    }

    pub async fn record_session_ssh_link_closed(&self,owner:&PrincipalRef,closed:SessionSshLinkClosedV1)->Result<SessionEventAck,SessionStoreError> {
        closed.validate().map_err(SessionStoreError::InvalidPayload)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        let close=&closed.close;
        let live=live_from_row(session_row_by_id_tx(&mut tx,close.agent_session_id.as_ref()).await?)?;
        if &close.owner_ref!=owner||live.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        let key=result_key(close);
        let append=SessionEventAppend {agent_session_id:close.agent_session_id.clone(),event_id:key.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind(SESSION_SSH_LINK_CLOSED.into()),kind_version:1,
                correlation_id:close.agent_session_id.as_ref().into(),causation_event_id:Some(request_key(close).into()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(closed)?))}};
        let ack=required_ack(self.append_event_tx_with_policy(&mut tx,&append,None,AppendSessionStatePolicy::SshLinkClose).await?)?;
        db_write_observation.commit(tx).await?;Ok(ack)
    }

    /// Metadata-only owner read of exact requested resources. A missing result
    /// remains unknown even after restart; it never authorizes a new SSH link.
    pub async fn session_ssh_link_closes(&self,owner:&PrincipalRef,session:&AgentSessionId)
        ->Result<Vec<(SessionSshLinkCloseV1,Option<SessionSshLinkClosedV1>)>,SessionStoreError> {
        let mut tx=self.pool.begin().await?;
        let live=live_from_row(session_row_by_id_tx(&mut tx,session.as_ref()).await?)?;
        if &live.owner_ref!=owner {return Err(SessionStoreError::ExecutionFenced);}
        let mut cursor=0i64;let mut observations=Vec::new();
        loop {
            let rows:Vec<StoredEventRow>=sqlx::query_as("SELECT session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json,payload_id \
                FROM agent_events WHERE session_id=? AND kind='session/ssh-link-close-requested' AND seq>? ORDER BY seq LIMIT 32")
                .bind(session.as_ref()).bind(cursor).fetch_all(&mut *tx).await?;
            if rows.is_empty(){break;}
            for row in rows {
                cursor=cursor.max(row.seq);let event=event_from_row(row)?;let value=payload_value_for_event_tx(&mut tx,&event).await?;
                validate_event_tx(&mut tx,&event,&value).await?;
                let close:SessionSshLinkCloseV1=serde_json::from_value(value)?;
                let result=if let Some(row)=event_by_event_id_tx(&mut tx,&result_key(&close)).await? {
                    let event=event_from_row(row)?;let value=payload_value_for_event_tx(&mut tx,&event).await?;
                    validate_event_tx(&mut tx,&event,&value).await?;Some(serde_json::from_value(value)?)
                }else{None};
                observations.push((close,result));
            }
        }
        tx.commit().await?;Ok(observations)
    }
}
