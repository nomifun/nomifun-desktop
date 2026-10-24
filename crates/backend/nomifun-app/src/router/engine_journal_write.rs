//! Retry only the same immutable native fact while its original caller waits.
//! A typed SQLite contention error never restarts a model/tool or a Turn.
use super::*;
use nomifun_agent_session::{NativeCheckpoint,NativeCheckpointWrite,SessionStoreError,SessionEventAppendResult};
use std::time::Duration;

fn contention(error:&SessionStoreError)->bool {
    matches!(error,SessionStoreError::Sqlite(error) if nomifun_db::database_unavailable_reason(error).is_some())
}

pub(super) fn store_error(error:SessionStoreError)->AppError {
    if let SessionStoreError::Sqlite(sql)=&error&&let Some(reason)=nomifun_db::database_unavailable_reason(sql) {
        AppError::DatabaseUnavailable {reason}
    } else {failure(error)}
}

async fn backoff(journal:&Journal,error:SessionStoreError,delay:&mut Duration)->Result<(),SessionStoreError> {
    tokio::select! {
        biased;
        _=journal.cancellation.cancelled()=>Err(error),
        _=tokio::time::sleep(*delay)=>{*delay=delay.saturating_mul(2).min(Duration::from_secs(1));Ok(())},
    }
}

fn confirmed(record:nomifun_agent_contracts::SessionEventRecord)->SessionEventAppendResult {
    let cursor=nomifun_agent_contracts::SessionEventCursor {agent_session_id:record.agent_session_id.clone(),seq:record.seq};
    let ack=nomifun_agent_contracts::SessionEventAck {agent_session_id:record.agent_session_id.clone(),event_id:record.event_id.clone(),seq:record.seq,cursor:cursor.clone()};
    SessionEventAppendResult {cursor,record:Some(record),ack:Some(ack),persisted:true,duplicate:true}
}

impl Journal {
    pub(super) async fn append_immutable(&self,append:&SessionEventAppend,payload:Option<&SessionPayloadRecord>,kind:EngineJournalWrite)
        ->Result<SessionEventAppendResult,SessionStoreError> {
        let mut delay=Duration::from_millis(25);
        let mut confirm_first=false;
        loop {
            let receipt=self.settlement.lock().await.clone();
            if confirm_first {
                let found=match receipt.as_ref() {
                    Some(receipt)=>self.store.confirm_native_settlement_event(receipt,append,payload).await,
                    None=>self.store.confirm_native_owner_event(&self.lease,append,payload).await,
                };
                match found {
                    Ok(Some(record))=>return Ok(confirmed(record)),
                    Ok(None)=>{},
                    Err(error) if contention(&error)=>{backoff(self,error,&mut delay).await?;continue;},
                    Err(error)=>return Err(error),
                }
            }
            let result=if let Some(receipt)=receipt.as_ref() {
                self.store.append_native_settlement_observation(receipt,append,payload).await
            } else if kind==EngineJournalWrite::Progress {
                self.store.append_native_event(&self.lease,append,payload).await
            } else {self.store.append_native_observation(&self.lease,append,payload).await};
            match result {
                Err(error) if contention(&error)=>{
                    #[cfg(test)]
                    self.contention_observed.notify_one();
                    confirm_first=true;
                    backoff(self,error,&mut delay).await?;
                }
                result=>return result,
            }
        }
    }

    pub(super) async fn save_immutable_checkpoint(&self,append:&SessionEventAppend,request:NativeCheckpointWrite)
        ->Result<NativeCheckpoint,SessionStoreError> {
        let digest=digest_bytes(&canonical_json_bytes(&request.state.0)?);
        let revision=request.expected_revision.checked_add(1).ok_or_else(||SessionStoreError::Conflict("checkpoint revision exhausted".into()))?;
        let mut delay=Duration::from_millis(25);
        let mut confirm_first=false;
        loop {
            if confirm_first {
                let observed=async {
                    let event=self.store.confirm_native_owner_event(&self.lease,append,None).await?;
                    let checkpoint=self.store.load_native_checkpoint(&request.owner,&self.session,&self.operation).await?;
                    if let Some(checkpoint)=checkpoint.as_ref()&&checkpoint.revision==revision {
                        if checkpoint.digest!=digest||checkpoint.state!=request.state||checkpoint.execution_fence!=request.execution_fence
                            ||event.as_ref().map(|event|event.seq)!=Some(checkpoint.through_seq) {
                            return Err(SessionStoreError::IdempotencyConflict("original checkpoint confirmation differs from its immutable state or metadata".into()));
                        }
                        return Ok(Some(checkpoint.clone()));
                    }
                    if event.is_some() {
                        if self.store.read_turn_receipt(&self.session,&self.operation).await?.status==TurnReceiptStatus::Cancelled {return Err(SessionStoreError::ExecutionFenced);}
                        return Err(SessionStoreError::InvalidEvent("checkpoint metadata exists without its exact checkpoint state".into()));
                    }
                    Ok(None)
                }.await;
                match observed {
                    Ok(Some(checkpoint))=>return Ok(checkpoint),
                    Ok(None)=>{},
                    Err(error) if contention(&error)=>{backoff(self,error,&mut delay).await?;continue;},
                    Err(error)=>return Err(error),
                }
            }
            match self.store.save_native_checkpoint(append,request.clone()).await {
                Err(error) if contention(&error)=>{
                    #[cfg(test)]
                    self.contention_observed.notify_one();
                    confirm_first=true;backoff(self,error,&mut delay).await?;
                },
                result=>return result,
            }
        }
    }
}
