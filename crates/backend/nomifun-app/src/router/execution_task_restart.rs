//! Native part of one durable logical-task restart. The domain command fixes
//! the original Attempt/Turn; this adapter cannot select a later task or mint
//! a model-facing checkpoint authorization.
use super::*;

impl NomiCoreSessionOwner {
    pub(super) async fn recover_execution_attempt_for_restart(&self, owner_id:&str,
        target:&nomifun_agent_execution::AgentExecutionRestartTarget)
        ->Result<nomifun_agent_execution::AgentExecutionRestartNativeOutcome,AppError> {
        use nomifun_agent_execution::AgentExecutionRestartNativeOutcome;
        let reject=||AppError::Conflict("restart no longer addresses its original owned task command".into());
        let event:Option<(String,String,Option<String>)>=sqlx::query_as(
            "SELECT event.payload,event.actor_type,event.actor_id FROM agent_execution_events event \
             JOIN agent_executions execution ON execution.execution_id=event.execution_id \
             WHERE event.execution_id=? AND execution.user_id=? AND execution.deleted_at IS NULL \
               AND event.sequence=? AND event.step_id=? AND event.attempt_id=? \
               AND json_extract(event.payload,'$.change')='restart_requested' \
               AND json_extract(event.payload,'$.operation_id')=? \
               AND NOT EXISTS(SELECT 1 FROM agent_execution_events resolved \
                 WHERE resolved.execution_id=event.execution_id \
                   AND json_extract(resolved.payload,'$.change')='restart_resolved' \
                   AND json_extract(resolved.payload,'$.operation_id')=json_extract(event.payload,'$.operation_id') \
                   AND json_extract(resolved.payload,'$.disposition') IN('blocked','superseded'))")
            .bind(&target.execution_id).bind(owner_id).bind(target.command_sequence)
            .bind(&target.step_id).bind(&target.attempt_id).bind(&target.command_operation_id)
            .fetch_optional(&self.pool).await.map_err(|error|AppError::from(nomifun_db::DbError::from(error)))?;
        let (payload,actor,actor_id)=event.ok_or_else(reject)?;
        let payload:Value=serde_json::from_str(&payload).map_err(|_|reject())?;
        let frozen:nomifun_agent_execution::AgentExecutionRestartTarget=serde_json::from_value(
            payload.get("restart_target").cloned().ok_or_else(reject)?).map_err(|_|reject())?;
        if &frozen!=target {return Err(reject());}
        let session=AgentSessionId::from(target.conversation_id.clone());
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        self.canonical.get(&principal,&session).await?;
        if let Some(pause)=target.pause.as_ref() {
            // Ordinary parent-agent control retains its existing management
            // scope. It cannot impersonate an owner's native resume grant.
            if actor!="user"||actor_id.as_deref()!=Some(owner_id) {
                return Err(AppError::Conflict("A paused native task requires its owner's explicit Restart or Continue command".into()));
            }
            let request=nomifun_agent_session::NativeResumeRequest {
                operation_id:target.canonical_operation_id.clone().into(),
                idempotency_key:format!("execution-task-restart:{}",target.command_operation_id),
                expected_pause_revision:pause.revision,
                expected_checkpoint_revision:pause.checkpoint_revision,
                expected_checkpoint_digest:pause.checkpoint_digest.clone()
                    .ok_or_else(||AppError::Conflict("original paused task has no checkpoint digest".into()))?.into(),
                budget:nomifun_agent_contracts::NativeBudgetIncrease {retry_stall_guards:true,..Default::default()},
                cleanup_attestation:None,
            };
            let owner=self.native_engines.get().and_then(std::sync::Weak::upgrade)
                .ok_or_else(||AppError::Conflict("the original runtime owner is unavailable".into()))?
                .session_owner()?;
            // Replays are read before preparation by the shared command port.
            // Neither a new pause nor a later checkpoint replaces this target.
            let receipt=owner.submit_native_resume(principal,session,request).await?;
            let (event,sequence)=match receipt {
                nomifun_agent_session::NativeResumeCommandReceipt::Authorized {receipt}=>(receipt.authorization_event_id,receipt.seq),
                nomifun_agent_session::NativeResumeCommandReceipt::Queued {receipt}=>(receipt.request_event_id,receipt.seq),
            };
            return Ok(AgentExecutionRestartNativeOutcome::ContinuedOriginal {resume_event_id:event.as_ref().into(),resume_event_seq:sequence});
        }
        // An explicit restart of a live invocation stops only this original
        // operation. The final domain writer must still prove its terminal,
        // known effects and native cleanup before admitting another Attempt.
        <Self as nomifun_agent_execution::AgentExecutionSessionPort>::cancel_attempt_for_execution(
            self,owner_id,&target.conversation_id,&target.attempt_id,&target.canonical_operation_id).await?;
        Ok(AgentExecutionRestartNativeOutcome::OriginalClosed)
    }
}
