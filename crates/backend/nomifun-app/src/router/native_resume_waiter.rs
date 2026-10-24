//! The original Session owner advances durable continuation intents only after
//! the original Execution reservation is available. No input is re-delivered.
use super::*;

fn admission_temporarily_unavailable(error:&AppError)->bool {
    matches!(error,AppError::DatabaseUnavailable {..}|AppError::ExecutionAdmissionPending)
}

#[cfg(test)]
type PreparationFailure = (AppError, Arc<tokio::sync::Notify>);
#[cfg(test)]
static PREPARATION_FAILURES: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeMap<String,PreparationFailure>>> = std::sync::OnceLock::new();

impl NomiCoreSessionOwner {
    #[cfg(test)]
    pub(in crate::router) async fn hold_native_resume_attachment_for_test(&self,session:&AgentSessionId)
        ->tokio::sync::OwnedRwLockWriteGuard<()> {
        self.session_operation_lock(&format!("native-recovery:{}",session.as_ref())).write_owned().await
    }

    #[cfg(test)]
    pub(in crate::router) async fn join_queued_native_resume_for_test(&self,session:&AgentSessionId,request:&NativeResumeRequest) {
        let key=format!("native-resume-request:{}:{}",session.as_ref(),nomifun_agent_contracts::digest_bytes(request.idempotency_key.as_bytes()).as_ref());
        let _joined=self.session_operation_lock(&key).write_owned().await;
    }

    #[cfg(test)]
    pub(in crate::router) fn inject_queued_native_resume_preparation_error(session:&str,error:AppError)->Arc<tokio::sync::Notify> {
        let reached=Arc::new(tokio::sync::Notify::new());
        PREPARATION_FAILURES.get_or_init(Default::default).lock().unwrap()
            .insert(session.to_owned(),(error,reached.clone()));
        reached
    }

    async fn prepare_queued_native_resume(&self,principal:&PrincipalRef,session:&AgentSessionId,
        request:&NativeResumeRequest)->Result<nomifun_agent_session::NativeResumePreparation,AppError> {
        #[cfg(test)]
        {
            let failure=PREPARATION_FAILURES.get_or_init(Default::default).lock().unwrap().remove(session.as_ref());
            if let Some((error,reached))=failure { reached.notify_one(); return Err(error); }
        }
        self.prepare_native_resume(principal,session,request).await
    }

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
                match store.native_resume_request_receipt(&principal,&session,&request).await.map_err(agent_session_store_error) {
                    Ok(None) => return,
                    Ok(Some(_)) => {}
                    Err(error) if admission_temporarily_unavailable(&error) => {
                        tracing::warn!(session_id=session.as_ref(),%error,"queued continuation receipt could not be read");
                        drop(owner);
                        tokio::select! { _ = tasks.shutdown_signal() => return, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
                        continue;
                    }
                    Err(error)=>{
                        // A deleted Session or revoked owner cannot write a
                        // control result onto this request. Retire this local
                        // flight; do not retain a permanent failure forever.
                        tracing::warn!(session_id=session.as_ref(),%error,"queued continuation receipt authority is no longer valid; original local owner retired");
                        return;
                    }
                }
                // Capacity, read-only preparation and the exact canonical
                // commit share one typed infrastructure classification. The
                // latter still rechecks every fence and original request;
                // retrying admission never replays model/tool work.
                let admitted:Result<Option<nomifun_agent_session::NativeResumeReceipt>,AppError>=async {
                    if !store.native_resume_capacity_available(&principal,&session,&request).await
                        .map_err(agent_session_store_error)? { return Ok(None); }
                    let _operation = owner.session_operation_lock(session.as_ref()).write_owned().await;
                    let prepared=owner.prepare_queued_native_resume(&principal,&session,&request).await?;
                    store.commit_native_resume(&principal,&session,&request,prepared).await
                        .map(Some).map_err(agent_session_store_error)
                }.await;
                match admitted {
                    Ok(Some(receipt))=>{
                        if let Err(error)=owner.enqueue_authorized_native_resume(session.clone(),request.operation_id.clone(),receipt.seq).await {
                            tracing::warn!(session_id=session.as_ref(),%error,"authorized continuation awaits original native recovery owner");
                        }
                        return;
                    }
                    Ok(None)=>{},
                    Err(error) if admission_temporarily_unavailable(&error)=>{
                        tracing::warn!(session_id=session.as_ref(),%error,"original queued continuation admission remains temporarily unavailable");
                    }
                    Err(error)=>{
                        match store.block_native_resume_request(&principal,&session,&request,&error.to_string()).await {
                            Ok(())=>return,
                            Err(error)=>tracing::warn!(session_id=session.as_ref(),%error,"continuation control result remains unconfirmed"),
                        }
                    }
                }
                // No Session admission lock survives this backoff. Stop can
                // win its durable writer; the next receipt read then exits
                // this same request without authorizing continuation.
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
