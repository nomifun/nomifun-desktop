//! Local recovery of one exited scheduler job. A failed observer or settlement
//! never becomes an aggregate barrier and never resubmits an admitted input.
use super::*;

pub(super) fn control_resolution(detail:&AgentExecutionDetail,step:&ExecutionStep)->ControlResolution {
    let dependencies=detail.dependencies.iter().filter(|edge|edge.superseded_in_revision.is_none()
        &&edge.blocked_step_id==step.step_id).filter_map(|edge|
        detail.steps.iter().find(|candidate|candidate.step_id==edge.blocker_step_id)).collect::<Vec<_>>();
    control_steps::evaluate(step,&dependencies,&detail.attempts)
}

impl ExecutionScheduler {
    /// Keep the existing reservation while observing only its exact Attempt.
    /// This future is polled beside healthy jobs, controls, Stop and lease loss.
    pub(super) fn recover_failed_step(&self,owner_id:String,execution_id:String,step_id:String,
        pending_version:i64,mut attempt_id:Option<String>,mut lease:AgentExecutionLeaseToken,first_error:AppError)
        ->BoxFuture<'static,()> {
        let scheduler=self.clone();
        async move {
            tracing::warn!(%execution_id,%step_id,%first_error,"scheduler job transferred to its original Step receipt owner");
            let mut recoverable=scheduler_error_is_recoverable(&first_error);
            let error_code=first_error.error_code().to_owned();
            let mut original_failure=Some(first_error);
            let changed=scheduler.inner.deps.publisher.changed_for(&owner_id,&execution_id);
            let mut delay=EFFECT_RETRY_MIN;
            loop {
                tokio::select! {
                    _=scheduler.inner.deps.lifecycle.cancelled()=>return,
                    _=changed.notified()=>{},
                    _=tokio::time::sleep(delay)=>{},
                }
                let result=async {
                    let detail=scheduler.detail(&owner_id,&execution_id).await?;
                    if !matches!(detail.execution.status,AgentExecutionStatus::Running|AgentExecutionStatus::WaitingInput|AgentExecutionStatus::Paused) {
                        return Ok(true);
                    }
                    let Some(step)=detail.steps.iter().find(|step|step.step_id==step_id&&step.superseded_in_revision.is_none()) else {return Ok(true);};
                    let current=detail.attempts.iter().filter(|attempt|attempt.step_id==step_id).max_by_key(|attempt|attempt.attempt_no);
                    if attempt_id.is_none() {
                        if let Some(attempt)=current.filter(|attempt|(attempt.status==ExecutionAttemptStatus::Queued
                            || step.kind!=ExecutionStepKind::Agent&&attempt.status==ExecutionAttemptStatus::Running)
                            &&pending_version.checked_add(1)==Some(step.version)) {
                            // create_attempt's unique expected-version CAS committed,
                            // but its reply was lost before the runner was called.
                            attempt_id=Some(attempt.attempt_id.clone());
                            scheduler.bind_step_job_attempt(&execution_id,&step_id,&attempt.attempt_id)?;
                        } else if step.status==ExecutionStepStatus::Pending && step.version==pending_version
                            &&current.is_none_or(|attempt|!matches!(attempt.status,ExecutionAttemptStatus::Queued|ExecutionAttemptStatus::Running)) {
                            if !recoverable {
                                scheduler.inner.deps.repository.transition_step_status(&owner_id,&execution_id,&step_id,
                                    detail.execution.version,step.version,Some(&lease),ExecutionStepStatus::Failed,
                                    &system_event(AgentExecutionEventKind::StepChanged,Some(&step_id),None,
                                        json!({"status":"failed","reason":"scheduler_pre_admission_failure","error_code":error_code}))).await?;
                                scheduler.notify_publication();
                            }
                            // No Attempt was admitted. Returning releases only this
                            // local dispatch slot, never an existing business input.
                            return Ok(true);
                        } else {return Ok(true);}
                    }
                    let Some(attempt)=current.filter(|attempt|Some(&attempt.attempt_id)==attempt_id.as_ref()) else {return Ok(true);};
                    if !matches!(attempt.status,ExecutionAttemptStatus::Queued|ExecutionAttemptStatus::Running) {return Ok(true);}
                    let Some(assigned)=scheduler.step_job_lease(&execution_id,&step_id,&lease) else {
                        // Only a new scheduler's successful writer acquisition
                        // can hand settlement authority back to this owner.
                        scheduler.start(owner_id.clone(),execution_id.clone());
                        return Ok(false);
                    };
                    lease=assigned;
                    if detail.execution.status==AgentExecutionStatus::Paused {return Ok(false);}
                    if attempt.status==ExecutionAttemptStatus::Queued {
                        if !recoverable {
                            scheduler.settle_agent_outcome(&owner_id,&execution_id,&step_id,&attempt.attempt_id,
                                Err(original_failure.take().ok_or_else(||AppError::Conflict("original preparation failure was already transferred".into()))?),
                                attempt.attempt_no+1,AttemptSettlementFence {step_version:step.version,attempt_version:attempt.version},Some(&lease)).await?;
                            return Ok(true);
                        }
                        let has_initial=attempt.runtime_state.as_ref().is_some_and(|state|
                            state.get("pending_conversation_effects").and_then(serde_json::Value::as_array)
                                .is_some_and(|effects|effects.iter().any(|effect|effect.get("kind").and_then(serde_json::Value::as_str)==Some("initial_input"))));
                        if !has_initial {
                            let mut observed=ScheduledAgentJobs::new();let mut in_flight=HashSet::new();
                            scheduler.recover_interrupted_scope(&owner_id,&execution_id,&lease,&mut observed,&mut in_flight,
                                Some((&step_id,&attempt.attempt_id))).await?;
                            return Ok(true);
                        }
                        let saved=scheduler.inner.deps.repository.get_step_detail(&owner_id,&execution_id,&step_id).await?
                            .ok_or_else(||AppError::NotFound("original queued Step".into()))?;
                        if saved.current_attempt.as_ref().is_none_or(|current|current.attempt.attempt_id!=attempt.attempt_id) {return Ok(true);}
                        // The returned sender no longer owns an active invocation.
                        // Continue only its exact persisted reservation/input.
                        if let Err(error)=scheduler.execute_agent_step_admitted(
                            &owner_id,&execution_id,step.clone(),&lease,&mut attempt_id,Some(saved)).await {
                            if !scheduler_error_is_recoverable(&error) {
                                // A later permanent preparation refusal cannot
                                // inherit the first temporary observation's
                                // retry classification. Retain its exact error,
                                // then reread: only a still-Queued original
                                // Attempt may settle it as a preparation error.
                                // A committed Running input follows its native
                                // receipt instead; no accepted input is deleted.
                                recoverable=false;
                                original_failure=Some(error);
                                return Ok(false);
                            }
                            return Err(error);
                        }
                        return Ok(true);
                    }
                    if step.kind!=ExecutionStepKind::Agent {
                        let saved=scheduler.inner.deps.repository.get_step_detail(&owner_id,&execution_id,&step_id).await?
                            .ok_or_else(||AppError::NotFound("original control Step".into()))?;
                        if saved.current_attempt.as_ref().is_none_or(|current|current.attempt.attempt_id!=attempt.attempt_id) {return Ok(true);}
                        scheduler.settle_control_step(&owner_id,&detail,step,&lease,&saved,control_resolution(&detail,step)).await?;
                        return Ok(true);
                    }
                    let mut observed=ScheduledAgentJobs::new();
                    let mut in_flight=HashSet::new();
                    scheduler.recover_interrupted_scope(&owner_id,&execution_id,&lease,&mut observed,&mut in_flight,
                        Some((&step_id,&attempt.attempt_id))).await?;
                    debug_assert!(observed.is_empty(),"local recovery must not recursively install an observer");
                    let fresh=scheduler.detail(&owner_id,&execution_id).await?;
                    let Some(original)=fresh.attempts.iter().find(|candidate|candidate.attempt_id==attempt.attempt_id) else {return Ok(true);};
                    if original.status!=ExecutionAttemptStatus::Running {return Ok(true);}
                    if scheduler.inner.conversation_controls.contains_key(&original.attempt_id)
                        || owns_recoverable_pending_effects(&fresh,original) {
                        // The durable control FIFO now owns this invocation.
                        // Release our local Step slot so that its exact queued
                        // decision can be delivered; retaining it would make
                        // that owner wait for this observer forever.
                        return Ok(true);
                    }
                    let current_step=fresh.steps.iter().find(|candidate|candidate.step_id==step_id)
                        .ok_or_else(||AppError::NotFound("original running Step".into()))?;
                    let session=original.conversation_id.as_deref().ok_or_else(||AppError::Conflict("original running Attempt has no Session".into()))?;
                    let key=scheduler.inner.deps.repository.execution_attempt_turn_operation_key(
                        &owner_id,&execution_id,&step_id,&original.attempt_id).await?;
                    let outcome=scheduler.inner.deps.attempt_runner.observe_existing_turn(&owner_id,session,&key).await?;
                    scheduler.settle_agent_outcome(&owner_id,&execution_id,&step_id,&original.attempt_id,Ok(outcome),original.attempt_no+1,
                        AttemptSettlementFence{step_version:current_step.version,attempt_version:original.version},Some(&lease)).await?;
                    Ok::<bool,AppError>(true)
                }.await;
                match result {
                    Ok(true)=>return,
                    Ok(false)=>{},
                    Err(error)=>tracing::warn!(%execution_id,%step_id,%error,"original Step receipt recovery remains pending; siblings continue"),
                }
                delay=next_effect_retry_delay(delay);
            }
        }.boxed()
    }
}
