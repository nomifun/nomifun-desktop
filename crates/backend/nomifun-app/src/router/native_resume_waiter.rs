//! The original Session owner advances durable continuation intents only after
//! the original Execution reservation is available. No input is re-delivered.
use super::*;

impl NomiCoreSessionOwner {
    pub(in crate::router) async fn schedule_requested_native_resumes(self: &Arc<Self>) -> Result<usize, AppError> {
        let mut after = 0;
        let mut scheduled = 0;
        loop {
            let pending = self.canonical.store().pending_native_resume_requests(after,500).await.map_err(agent_session_store_error)?;
            let Some(last) = pending.last() else { break; };
            after = last.rowid;
            for item in pending {
                self.enqueue_requested_native_resume(item.owner,item.agent_session_id,item.request)?;
                scheduled += 1;
            }
        }
        Ok(scheduled)
    }

    pub(in crate::router) fn enqueue_requested_native_resume(self: &Arc<Self>, principal: PrincipalRef,
        session: AgentSessionId, request: NativeResumeRequest) -> Result<(), AppError> {
        // This is the existing process-local Session lock registry, not another
        // durable queue. The original event remains the only intent authority.
        let key = format!("native-resume-request:{}:{}",session.as_ref(),nomifun_agent_contracts::digest_bytes(request.idempotency_key.as_bytes()).as_ref());
        let Ok(flight) = self.session_operation_lock(&key).try_write_owned() else { return Ok(()); };
        let weak = Arc::downgrade(self);
        let tasks = self.background_tasks.clone();
        if !self.background_tasks.spawn(Box::pin(async move {
            let _flight = flight;
            loop {
                let Some(owner) = weak.upgrade() else { return; };
                let store = owner.canonical.store();
                match store.native_resume_request_receipt(&principal,&session,&request).await {
                    Ok(None) => return,
                    Ok(Some(_)) => {}
                    Err(error) => {
                        tracing::warn!(session_id=session.as_ref(),%error,"queued continuation receipt could not be read");
                        drop(owner);
                        tokio::select! { _ = tasks.shutdown_signal() => return, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
                        continue;
                    }
                }
                let available = store.native_resume_capacity_available(&principal,&session,&request).await;
                let mut blocked = None;
                match available {
                    Ok(false) => {}
                    Ok(true) => {
                        let _operation = owner.session_operation_lock(session.as_ref()).write_owned().await;
                        match owner.prepare_native_resume(&principal,&session,&request).await {
                            Ok(prepared) => match store.commit_native_resume(&principal,&session,&request,prepared).await {
                                Ok(receipt) => {
                                    drop(_operation);
                                    if let Err(error) = owner.enqueue_authorized_native_resume(session.clone(),request.operation_id.clone(),receipt.seq).await {
                                        tracing::warn!(session_id=session.as_ref(),%error,"authorized continuation awaits original native recovery owner");
                                    }
                                    return;
                                }
                                Err(nomifun_agent_session::SessionStoreError::ExecutionCapacityPending) => {}
                                Err(nomifun_agent_session::SessionStoreError::Sqlite(error)) => {
                                    tracing::warn!(session_id=session.as_ref(),%error,"queued continuation commit remains unconfirmed");
                                }
                                Err(error) => blocked = Some(error.to_string()),
                            },
                            Err(error) => blocked = Some(error.to_string()),
                        }
                    }
                    Err(nomifun_agent_session::SessionStoreError::Sqlite(error)) => {
                        tracing::warn!(session_id=session.as_ref(),%error,"queued continuation capacity could not be read");
                    }
                    Err(error) => blocked = Some(error.to_string()),
                }
                if let Some(reason) = blocked {
                    match store.block_native_resume_request(&principal,&session,&request,&reason).await {
                        Ok(()) => return,
                        Err(error) => tracing::warn!(session_id=session.as_ref(),%error,"continuation control result remains unconfirmed"),
                    }
                }
                drop(owner);
                tokio::select! {
                    _ = tasks.shutdown_signal() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        })) { return Err(AppError::Conflict("continuation request is durable but its local owner is shutting down".into())); }
        Ok(())
    }
}
