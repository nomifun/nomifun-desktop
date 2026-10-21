//! Exact composer Stop before or after the original input's initial claim.
//! Cancellation and the retained-batch hold use the existing canonical facts.
use super::*;
use nomifun_agent_contracts::{SessionInputQueueAction,SessionInputQueueChangedV1};

#[derive(Debug)]
pub enum SessionInputStop {
    Turn { operation_id: OperationId },
    Input { event: SessionEventRecord, through_seq: u64, duplicate: bool },
}

impl AgentSessionStore {
    pub async fn stop_session_input(&self,owner:&PrincipalRef,session:&AgentSessionId,input:&EventId,key:&str)
        ->Result<SessionInputStop,SessionStoreError> {
        if key.trim().is_empty() || key.len()>256 {return Err(SessionStoreError::InvalidEvent("input Stop requires a bounded identity".into()));}
        let mut tx=self.begin_write_transaction().await?;
        if require_live_session_tx(&mut tx,session.as_ref()).await?.owner_ref!=*owner {return Err(SessionStoreError::ExecutionFenced);}
        // Share the existing composer cancellation namespace across the
        // pre-claim and Turn branches, rather than giving one API key two
        // independently mutable command identities.
        let identity=format!("{}:{}:{}:{}:cancel",owner.principal_kind,owner.principal_id,session.as_ref(),key.trim());
        if let Some(row)=event_by_producer_key_tx(&mut tx,"session_api",&identity).await? {
            let receipt=event_from_row(row)?;
            if receipt.agent_session_id==*session && receipt.kind.0=="turn/cancelled" {
                let payload=payload_value_for_event_tx(&mut tx,&receipt).await?;
                let target=payload.get("target_operation_id").and_then(Value::as_str)
                    .filter(|target|*target==receipt.correlation_id.as_ref())
                    .ok_or_else(||SessionStoreError::InvalidEvent("Turn Stop lost its exact target".into()))?;
                let source:Option<String>=sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?")
                    .bind(session.as_ref()).bind(target).fetch_optional(&mut *tx).await?;
                if source.as_deref()!=Some(input.as_ref()) {return Err(SessionStoreError::IdempotencyConflict("Stop key differs from its original source".into()));}
                let operation_id=OperationId::from(target);tx.commit().await?;
                return Ok(SessionInputStop::Turn {operation_id});
            }
            if receipt.agent_session_id!=*session || receipt.kind.0!="message/input-queue-changed"
                || receipt.causation_event_id.as_ref()!=Some(input) {
                return Err(SessionStoreError::IdempotencyConflict("input Stop key differs from its original source".into()));
            }
            let payload=payload_value_for_event_tx(&mut tx,&receipt).await?;
            session_input_queue::validate_event(&receipt,&payload)?;
            let change:SessionInputQueueChangedV1=serde_json::from_value(payload)?;
            if !matches!(change.action,nomifun_agent_contracts::SessionInputQueueChange::Control(SessionInputQueueAction::Pause {}))
                || !change.except_message_ids.is_empty() {return Err(SessionStoreError::InvalidEvent("input Stop lost its retained-batch hold".into()));}
            let original=session_input::accepted_input_tx(&mut tx,session,input).await?;
            let event=session_input::cancelled_input_tx(&mut tx,&original.event).await?
                .ok_or_else(||SessionStoreError::InvalidEvent("input Stop lost its original cancellation".into()))?;
            tx.commit().await?;
            return Ok(SessionInputStop::Input {event,through_seq:receipt.seq,duplicate:true});
        }
        // A claim that won the writer race is still addressed by the source,
        // never by whichever newer Turn happens to be active when Stop arrives.
        let operations:Vec<String>=sqlx::query_scalar("SELECT turn.operation_id FROM agent_turns turn \
            JOIN agent_events started ON started.event_id=turn.started_event_id AND started.session_id=turn.session_id \
                AND started.kind='turn/started' AND started.correlation_id=turn.operation_id \
            JOIN agent_events input ON input.event_id=turn.source_message_id AND input.session_id=turn.session_id \
                AND input.kind='message/user-accepted' AND started.causation_event_id=input.event_id \
            WHERE turn.session_id=? AND turn.source_message_id=? LIMIT 2")
            .bind(session.as_ref()).bind(input.as_ref()).fetch_all(&mut *tx).await?;
        match operations.as_slice() {
            [operation]=>{let operation_id=OperationId::from(operation.clone());tx.commit().await?;return Ok(SessionInputStop::Turn {operation_id});},
            []=>{},
            _=>return Err(SessionStoreError::InvalidEvent("input Stop source has more than one owning Turn".into())),
        }
        let original=session_input::accepted_input_tx(&mut tx,session,input).await?;
        if session_input_queue::desktop_delivery_intent(&original.input.0)?.is_none() {
            return Err(SessionStoreError::ExecutionFenced);
        }
        if let Some(event)=session_input::cancelled_input_tx(&mut tx,&original.event).await? {
            let through_seq=event.seq;tx.commit().await?;
            return Ok(SessionInputStop::Input {event,through_seq,duplicate:true});
        }
        if session_input::input_claimed_tx(&mut tx,session,input).await? {
            return Err(SessionStoreError::Conflict("input has already joined another Turn as steering".into()));
        }
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&mut *tx).await?;
        if original.event.seq<=as_u64(floor,"input Stop context floor")? {return Err(SessionStoreError::ExecutionFenced);}
        let head=head_by_id_tx(&mut tx,session.as_ref()).await?;
        let queue=session_input_queue::queue_state_tx(&mut tx,session).await?;
        if head.status!="ready" || head.active_turn_id.is_some()
            || session_input_queue::first_eligible_root_tx(&mut tx,session,&queue).await?.as_ref()!=Some(input) {
            return Err(SessionStoreError::Conflict("observed input no longer owns initial preparation".into()));
        }
        let cutoff=head.last_seq;
        self.cancel_pending_input_tx(&mut tx,session,input).await?;
        let event=session_input::cancelled_input_tx(&mut tx,&original.event).await?
            .ok_or_else(||SessionStoreError::InvalidEvent("input Stop did not retain cancellation".into()))?;
        let append=SessionEventAppend {agent_session_id:session.clone(),event_id:new_event_id(),producer_id:"session_api".into(),idempotency_key:identity.into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:SessionEventKind("message/input-queue-changed".into()),kind_version:1,
                correlation_id:CorrelationId::from(session.as_ref()),causation_event_id:Some(input.clone()),
                payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(SessionInputQueueChangedV1 {
                    action:SessionInputQueueAction::Pause {}.into(),expected_version:None,through_seq:Some(cutoff),except_message_ids:Vec::new(),
                })?))}};
        let held=self.append_event_tx(&mut tx,&append,None).await?;
        let through_seq=held.cursor.seq;tx.commit().await?;
        Ok(SessionInputStop::Input {event,through_seq,duplicate:false})
    }
}
