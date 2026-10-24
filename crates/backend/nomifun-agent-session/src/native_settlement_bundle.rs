//! The final native record and its canonical exit share one writer snapshot.
//! In particular, an intervening user Pause cannot leave a premature Runtime
//! terminal committed while its canonical counterpart was rejected.
use super::*;

async fn acknowledge_tx(store:&AgentSessionStore,tx:&mut Transaction<'_,Sqlite>,receipt:&NativeTurnSettlementReceipt)
    ->Result<(),SessionStoreError> {
    let identity=format!("{}:settled",receipt.event_id.as_ref());
    store.append_event_tx(tx,&native_pause::native_control_event(&receipt.session,&receipt.operation,identity,SETTLED,
        receipt.event_id.clone(),json!({"execution_generation":receipt.generation})),None).await?;
    Ok(())
}

impl AgentSessionStore {
    pub async fn append_native_settlement_terminal_bundle(&self,receipt:&NativeTurnSettlementReceipt,
        runtime:&SessionEventAppend,runtime_payload:Option<&SessionPayloadRecord>,message:Option<&SessionEventAppend>,terminal:Option<&SessionEventAppend>)
        ->Result<NativeTurnSettlementBundleReceipt,SessionStoreError> {
        validate_event_append(runtime)?;
        if runtime.agent_session_id!=receipt.session||runtime.producer_id.as_ref()!="runtime_supervisor"||runtime.semantic_event.kind_version!=1
            ||runtime.semantic_event.kind.0!="runtime/progress-recorded"||runtime.semantic_event.correlation_id.as_ref()!=receipt.operation.as_ref() {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(payload)=runtime_payload {validate_supplied_payload(runtime,payload)?;validate_payload_contents(payload)?;}
        let body=match (&runtime.semantic_event.payload,runtime_payload.map(|payload|&payload.body)) {
            (SessionEventPayloadRef::InlineJson(value),_)=>value.0.clone(),
            (SessionEventPayloadRef::Stored(_),Some(SessionPayloadBody::Json(value)))=>value.0.clone(),
            _=>return Err(SessionStoreError::InvalidPayload("original terminal runtime Payload is missing".into())),
        };
        let event=&body["event"];
        let kind=event["event"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
        let (mut tx, db_write_observation) = self.begin_write_transaction().await?;
        if !original_cause_tx(&mut tx,receipt,runtime.semantic_event.causation_event_id.as_ref()).await? {return Err(SessionStoreError::ExecutionFenced);}
        let done=format!("{}:settled",receipt.event_id.as_ref());
        if let Some(done)=event_by_event_id_tx(&mut tx,&done).await?.map(event_from_row).transpose()? {
            let request=event_by_event_id_tx(&mut tx,receipt.event_id.as_ref()).await?.map(event_from_row).transpose()?.ok_or(SessionStoreError::ExecutionFenced)?;
            let saved=receipt_tx(&mut tx,request).await?;
            if done.agent_session_id!=receipt.session||done.causation_event_id.as_ref()!=Some(&receipt.event_id)||saved.outcome!=receipt.outcome
                ||saved.session!=receipt.session||saved.operation!=receipt.operation||saved.generation!=receipt.generation {return Err(SessionStoreError::ExecutionFenced);}
            let original=terminal_matches(receipt,kind,event,true);
            if !original {
                let steps_match=event.get("model_steps")==receipt.outcome.0.get("model_steps");
                let stopped:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns t JOIN agent_events e ON e.session_id=t.session_id AND e.event_id=t.terminal_event_id WHERE t.session_id=? AND t.operation_id=? AND t.state='cancelled' AND e.kind='turn/cancelled' AND e.correlation_id=t.operation_id)")
                    .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).fetch_one(&mut *tx).await?;
                let user_pause:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND correlation_id=? AND kind='turn/pause-requested' AND kind_version=1 AND producer_id='runtime_supervisor' AND causation_event_id=? AND seq>=? AND seq<?)")
                    .bind(receipt.session.as_ref()).bind(receipt.operation.as_ref()).bind(receipt.started.as_ref()).bind(as_i64(receipt.generation,"pause generation")?).bind(as_i64(done.seq,"settlement acknowledgement cursor")?).fetch_one(&mut *tx).await?;
                if !steps_match||!((kind=="turn_cancelled"&&stopped)||(kind=="turn_paused"&&event["reason"]=="EXECUTION_USER_REQUESTED"&&user_pause)) {return Err(SessionStoreError::ExecutionFenced);}
            }
            db_write_observation.commit(tx).await?;return Ok(NativeTurnSettlementBundleReceipt {runtime_seq:None});
        }
        let state=current_tx(&mut tx,receipt).await?;
        if !cleanup_proven_tx(&mut tx,receipt).await? {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        if matches!(state.as_str(),"cancelled"|"completed"|"failed") {
            acknowledge_tx(self,&mut tx,receipt).await?;
            db_write_observation.commit(tx).await?;return Ok(NativeTurnSettlementBundleReceipt {runtime_seq:None});
        }
        let user_pause=requires_user_pause_tx(&mut tx,receipt).await?;
        let matches=if user_pause {
            kind=="turn_paused"&&event["reason"]=="EXECUTION_USER_REQUESTED"&&event.get("model_steps")==receipt.outcome.0.get("model_steps")
        } else {terminal_matches(receipt,kind,event,true)};
        if !matches {return Err(SessionStoreError::ExecutionFenced);}
        if let Some(message)=message {
            if message.agent_session_id!=receipt.session||message.producer_id.as_ref()!="runtime_supervisor"||message.semantic_event.kind.0!="message/completed"
                ||message.semantic_event.kind_version!=1||!original_cause_tx(&mut tx,receipt,message.semantic_event.causation_event_id.as_ref()).await? {return Err(SessionStoreError::ExecutionFenced);}
        }
        if kind=="turn_paused" {
            if terminal.is_some() {return Err(SessionStoreError::ExecutionFenced);}
            let reason=event["reason"].as_str().ok_or(SessionStoreError::ExecutionFenced)?;
            let runtime=required_ack(self.append_event_tx(&mut tx,runtime,runtime_payload).await?)?;
            if let Some(message)=message {self.append_event_tx(&mut tx,message,None).await?;}
            let previous=receipt.pause.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
            if previous.reason!=reason||!previous.cleanup_proven||pending_user_pause_tx(&mut tx,receipt).await?.is_some() {
                self.pause_native_execution_tx(&mut tx,&receipt.session,&receipt.operation,previous.execution_fence,reason,true).await?;
            }
            acknowledge_tx(self,&mut tx,receipt).await?;
            db_write_observation.commit(tx).await?;return Ok(NativeTurnSettlementBundleReceipt {runtime_seq:Some(runtime.seq)});
        }
        let terminal=terminal.ok_or(SessionStoreError::ExecutionFenced)?;
        if terminal.agent_session_id!=receipt.session||terminal.producer_id.as_ref()!="runtime_supervisor"||terminal.semantic_event.kind_version!=1
            ||terminal.semantic_event.correlation_id.as_ref()!=receipt.operation.as_ref() {return Err(SessionStoreError::ExecutionFenced);}
        let SessionEventPayloadRef::InlineJson(terminal_body)=&terminal.semantic_event.payload else {return Err(SessionStoreError::ExecutionFenced);};
        if kind=="turn_cancelled" {
            if message.is_some()||terminal.semantic_event.kind.0!="turn/cancelled"||terminal.semantic_event.causation_event_id.as_ref()!=Some(&receipt.started)
                ||terminal_body.0["operation_id"].as_str()!=Some(receipt.operation.as_ref()) {return Err(SessionStoreError::ExecutionFenced);}
        } else {
            let message=message.ok_or(SessionStoreError::ExecutionFenced)?;
            if terminal.semantic_event.causation_event_id.as_ref()!=Some(&message.event_id)
                ||!terminal_matches(receipt,&terminal.semantic_event.kind.0,&terminal_body.0,false) {return Err(SessionStoreError::ExecutionFenced);}
            let unsettled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_effects WHERE session_id=? AND state IN ('pending','unknown'))")
                .bind(receipt.session.as_ref()).fetch_one(&mut *tx).await?;
            if unsettled {return Err(SessionStoreError::RecoveryRequiresReconciliation);}
        }
        let runtime=required_ack(self.append_event_tx(&mut tx,runtime,runtime_payload).await?)?;
        if let Some(message)=message {self.append_event_tx(&mut tx,message,None).await?;}
        self.append_event_tx(&mut tx,terminal,None).await?;
        acknowledge_tx(self,&mut tx,receipt).await?;
        db_write_observation.commit(tx).await?;Ok(NativeTurnSettlementBundleReceipt {runtime_seq:Some(runtime.seq)})
    }
}
