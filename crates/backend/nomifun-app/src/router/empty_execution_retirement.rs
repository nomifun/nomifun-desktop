//! Cleanup of an original Attempt's never-admitted Session belongs to the
//! Session lifecycle owner. A caller dropping its await cannot drop cleanup.
use super::*;
use nomifun_agent_session::EmptyExecutionCreationFence;

impl NomiCoreSessionOwner {
    pub(super) async fn discard_empty_execution_creation(
        &self, user:&str, attempt:&str, expected_session:Option<&str>,
    )->Result<(),AppError> {
        let owner=self.native_engines.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(||AppError::Conflict("empty creation lifecycle owner is unavailable".into()))?.session_owner()?;
        if !std::ptr::eq(self,owner.as_ref()) {return Err(AppError::Conflict("empty creation belongs to another lifecycle owner".into()));}
        let (sender,receiver)=tokio::sync::oneshot::channel();
        let (handoff,accepted)=tokio::sync::oneshot::channel();
        let work=owner.retire_empty_execution_creation_owned(user.to_owned(),attempt.to_owned(),expected_session.map(str::to_owned),Some(handoff));
        if !self.background_tasks.spawn(Box::pin(async move {let result=work.await;let _=sender.send(result);})) {
            return Err(AppError::Conflict("Session lifecycle owner closed before empty creation cleanup".into()));
        }
        tokio::select! {
            result=receiver=>result.map_err(|_|AppError::Internal("empty creation owner exited before its receipt".into()))?,
            Ok(())=accepted=>Ok(()),
        }
    }

    fn retire_empty_execution_creation_owned(self:Arc<Self>,user:String,attempt:String,expected_session:Option<String>,mut handoff:Option<tokio::sync::oneshot::Sender<()>>)
        ->std::pin::Pin<Box<dyn Future<Output=Result<(),AppError>>+Send+'static>> {
        Box::pin(async move {
            let principal=PrincipalRef {principal_kind:"user".into(),principal_id:user.clone()};
            let expected_session=expected_session.map(AgentSessionId::from);
            let requested_at=now_ms();let mut fenced=None;let mut backoff=Duration::from_millis(100);
            loop {
                let result=async {
                    if fenced.is_none() {
                        match self.canonical.fence_empty_execution_creation(&principal,&attempt,expected_session.as_ref(),requested_at).await? {
                            EmptyExecutionCreationFence::Missing=>return Ok(true),
                            EmptyExecutionCreationFence::NotEmpty {..}=>return Err(AppError::Conflict("original creation has admitted work or a different identity; Session is retained".into())),
                            EmptyExecutionCreationFence::AwaitingAttemptDisposition {..}=>{
                                // Let the original runner report its error so
                                // the scheduler can commit that disposition.
                                // This ACK transfers cleanup responsibility;
                                // it never says the still-owned Session died.
                                if let Some(handoff)=handoff.take() {let _=handoff.send(());}
                                return Ok(false);
                            }
                            EmptyExecutionCreationFence::Fenced {command,session}=>fenced=Some((command,session)),
                        }
                    }
                    let (command,session)=fenced.as_ref().expect("fenced above");
                    let id=&session.agent_session_id;
                    let _operation=self.session_operation_lock(id.as_ref()).write_owned().await;
                    if let Some(tombstone)=self.canonical.store().inspect_tombstone(id).await.map_err(agent_session_store_error)? {
                        if tombstone.owner_ref!=principal {return Err(AppError::Forbidden("empty creation tombstone belongs to another owner".into()));}
                    } else {
                        // Even a never-admitted Session can have a warm engine.
                        // Retire its exact runtime before managed files vanish.
                        self.runtime_sessions.terminate_and_wait_result(id.as_ref(),Some(nomifun_common::AgentKillReason::ConversationDeleted)).await?;
                        if let Some(engines)=self.native_engines.get().and_then(std::sync::Weak::upgrade) {
                            engines.cleanup_session_processes(&user,id).await?;
                        }
                        self.remove_workspace_for_binding(&user,id,&session.agent_binding).await?;
                        // Finish auxiliary cleanup while `deleting` still
                        // durably owns recovery. Purge is the final ACK.
                        self.remove_idmm_state(id.as_ref()).await?;
                        self.canonical.complete_fenced_delete(command,now_ms()).await?;
                    }
                    // A lost purge ACK is recognized by this exact tombstone;
                    // every cleanup above precedes the durable final boundary.
                    Ok(true)
                }.await;
                match result {
                    Ok(true)=>return Ok(()),
                    Ok(false)=>{},
                    Err(error)=>{
                        if fenced.is_none()&&!matches!(error,AppError::DatabaseUnavailable {..}) {return Err(error);}
                        tracing::warn!(attempt_id=%attempt,cleanup_fenced=fenced.is_some(),error_code=error.error_code(),
                            "original empty creation cleanup remains owned; business work is not retried");
                    }
                }
                // No transaction or Session operation gate survives backoff.
                // `deleting`, or the original terminal Attempt/opening before
                // the fence, supplies the existing cold-recovery obligation.
                tokio::select! {
                    _=self.background_tasks.shutdown_signal()=>return Err(AppError::Conflict("empty creation cleanup handed to lifecycle recovery".into())),
                    _=tokio::time::sleep(backoff)=>{},
                }
                backoff=(backoff*2).min(Duration::from_secs(2));
            }
        })
    }

    pub(super) async fn recover_empty_execution_creations(self:&Arc<Self>)->Result<(),AppError> {
        // This is candidate discovery only. The canonical writer repeats the
        // exact original proof and all emptiness checks before any retirement.
        // Queued work remains owned by its scheduler and is never a candidate.
        let candidates:Vec<(String,String,String)>=sqlx::query_as(
            "SELECT DISTINCT execution.user_id,attempt.attempt_id,opening.session_id \
             FROM agent_execution_attempts attempt \
             JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id \
             JOIN agent_executions execution ON execution.execution_id=attempt.execution_id \
             JOIN agent_events opening ON opening.producer_id='session_api' AND opening.kind='session/opening' \
               AND opening.idempotency_key IN('user:'||execution.user_id||':open:'||attempt.attempt_id, \
                   'user:'||execution.user_id||':open:execution-source:'||attempt.attempt_id) \
             JOIN agent_sessions session ON session.agent_session_id=opening.session_id AND session.state='live' \
             WHERE (attempt.status IN('failed','cancelled','interrupted') \
               OR step.status IN('completed','failed','cancelled','skipped') OR step.superseded_in_revision IS NOT NULL \
               OR execution.status IN('completed','completed_with_failures','failed','cancelled') OR execution.deleted_at IS NOT NULL) \
               AND NOT EXISTS(SELECT 1 FROM conversation_execution_links link WHERE link.conversation_id=opening.session_id)")
            .fetch_all(&self.pool).await.map_err(|error|agent_session_store_error(nomifun_agent_session::SessionStoreError::from(error)))?;
        for (user,attempt,session) in candidates {
            let work=self.clone().retire_empty_execution_creation_owned(user,attempt.clone(),Some(session),None);
            if !self.background_tasks.spawn(Box::pin(async move {
                if let Err(error)=work.await {tracing::warn!(attempt_id=%attempt,error_code=error.error_code(),"empty creation recovery retained original Session");}
            })) {return Err(AppError::Conflict("Session lifecycle owner closed before empty creation recovery".into()));}
        }
        Ok(())
    }
}
