//! Task controls retain one logical Step across execution attempts. Original
//! domain events acknowledge commands; no separate execution ledger exists.
use super::*;

impl AgentExecutionEngine {
    pub(super) async fn control_delegated_task(&self,owner:&str,source:&nomifun_db::AgentExecutionMessageSource,
        command:&nomifun_agent_contracts::AgentCollaborationMessage,control:&nomifun_agent_contracts::AgentCollaborationTaskControl,
        operation:&str,digest:&str)->Result<crate::AgentExecutionMessageReceipt,AppError> {
        use nomifun_agent_contracts::AgentCollaborationTaskAction;
        control.validate().map_err(AppError::BadRequest)?;
        if let AgentCollaborationTaskAction::Restart {expected_attempt_id}=&control.action {
            return self.restart_step_from_message(owner,source,command,control,expected_attempt_id.clone(),operation,digest).await;
        }
        if let Some(receipt)=self.message_receipt(owner,&source.agent_session_id,&source.turn_operation_id,command).await? {return Ok(receipt);}
        let (execution,step)=(&command.execution_id,&command.step_id);
        let actor=match self.authorize_agent_caller(owner,execution,&source.agent_session_id).await {
            Ok(actor)=>actor,
            Err(AppError::Conflict(_)|AppError::Forbidden(_)|AppError::NotFound(_)) if matches!(control.action,AgentCollaborationTaskAction::Retry {})=>{
                // A settled aggregate may have retired its lead link while
                // the exact native parent still owns this source. The retry
                // writer must prove the original command, live source epoch
                // and Step ownership before reactivating that original lead.
                AgentExecutionActor::agent(&source.agent_session_id,None)
            },
            Err(error)=>return Err(error),
        };
        self.reject_automation_external_command(owner,execution,&actor,"control a delegated task in").await?;
        let change=match &control.action {AgentCollaborationTaskAction::Restart {..}=>unreachable!(),AgentCollaborationTaskAction::Retry {}=>"retry_requested",AgentCollaborationTaskAction::Recheck {}=>"recheck_requested",
            AgentCollaborationTaskAction::Stop {}=>"cancel_step",AgentCollaborationTaskAction::ReplaceWithExisting {..}=>"replaced_with_existing"};
        let event=actor_event(&actor,AgentExecutionEventKind::StepChanged,Some(step),None,json!({"change":change,
            "operation_id":operation,"content_digest":digest,"message_source":source,
            "command_request":{"kind":"native_task_control","digest":nomifun_agent_contracts::digest_payload(command).map_err(|error|AppError::Internal(error.to_string()))?}}));
        let rows=match &control.action {
            AgentCollaborationTaskAction::Restart {..}=>unreachable!(),
            AgentCollaborationTaskAction::Retry {}=>self.repository.reset_steps_for_retry(owner,execution,control.expected_execution_version,
                &[RetryAgentExecutionStep {step_id:step.clone(),expected_step_version:control.expected_step_version}],&event).await?,
            AgentCollaborationTaskAction::Recheck {}=>self.repository.request_step_reconciliation(owner,execution,control.expected_execution_version,
                step,control.expected_step_version,&event).await?,
            AgentCollaborationTaskAction::ReplaceWithExisting {replacement_step_id,expected_replacement_step_version}=>{
                let params=self.replacement_params(owner,execution,step,control.expected_step_version,replacement_step_id,*expected_replacement_step_version).await?;
                self.repository.replace_step_with_existing(owner,execution,control.expected_execution_version,&params,&event).await?
            },
            AgentCollaborationTaskAction::Stop {}=>self.repository.cancel_step(owner,execution,control.expected_execution_version,step,control.expected_step_version,
                nomifun_common::AgentExecutionCancellationScope::Task,&event).await?,
        };
        if matches!(control.action,AgentCollaborationTaskAction::Stop {}) {
            self.planning_controls.stop_through(execution,rows.execution.version,Some(&[step.clone()]));
        }
        if matches!(control.action,AgentCollaborationTaskAction::ReplaceWithExisting {..}|AgentCollaborationTaskAction::Stop {}) {
            self.scheduler.request_conversation_cleanup(execution.clone());
        }
        self.scheduler.start(owner.to_owned(),execution.clone());self.publish().await;
        self.message_receipt(owner,&source.agent_session_id,&source.turn_operation_id,command).await?
            .ok_or_else(||AppError::Internal("accepted task control has no original command receipt".into()))
    }

    async fn replacement_params(&self,owner:&str,execution:&str,old:&str,old_version:i64,replacement:&str,replacement_version:i64)
        ->Result<nomifun_db::ReplaceAgentExecutionStepWithExistingParams,AppError> {
        let detail=self.detail(owner,execution).await?;
        let target=current_step(&detail,replacement)?;
        if target.status!=ExecutionStepStatus::Completed||target.version!=replacement_version {
            return Err(AppError::Conflict("replacement task must have its original verified completed result".into()));
        }
        let attempt=detail.attempts.iter().filter(|attempt|attempt.step_id==replacement).max_by_key(|attempt|attempt.attempt_no)
            .ok_or_else(||AppError::Conflict("replacement task has no original Attempt".into()))?;
        let output=self.scheduler.recover_attempt_output(owner,attempt).await?
            .filter(|output|output.disposition==nomifun_db::AgentExecutionAttemptDisposition::Delivered
                && (output.text.as_ref().is_some_and(|text|!text.trim().is_empty())||!output.output_files.is_empty()))
            .ok_or_else(||AppError::Conflict("replacement task has no verified canonical public delivery".into()))?;
        Ok(nomifun_db::ReplaceAgentExecutionStepWithExistingParams {old_step_id:old.into(),expected_old_step_version:old_version,
            replacement_step_id:replacement.into(),expected_replacement_step_version:replacement_version,
            replacement_terminal:nomifun_db::AgentExecutionClosedTurn {conversation_id:output.conversation_id,
                canonical_operation_id:output.canonical_operation_id,terminal_event_id:output.terminal_event_id}})
    }

    pub async fn recheck_step(&self,owner:&str,actor:&AgentExecutionActor,execution:&str,step:&str,request:RetryExecutionStepRequest)
        ->Result<AgentExecutionDetail,AppError> {
        let engine=self.clone();let owner=owner.to_owned();let actor=actor.clone();let execution=execution.to_owned();let step=step.to_owned();
        self.lifecycle.spawn(async move {engine.recheck_step_inner(&owner,&actor,&execution,&step,request).await})
            .await.map_err(|error|AppError::Internal(format!("task reconciliation owner failed: {error}")))?
    }

    async fn recheck_step_inner(&self,owner:&str,actor:&AgentExecutionActor,execution:&str,step:&str,request:RetryExecutionStepRequest)
        ->Result<AgentExecutionDetail,AppError> {
        let command=json!({"kind":"recheck_step","owner_id":owner,"execution_id":execution,"step_id":step,"request":request,"actor":actor});
        let operation=format!("recheck-step:{}",nomifun_agent_contracts::digest_payload(&command).map_err(|error|AppError::Internal(error.to_string()))?.as_ref());
        let event=actor_event(actor,AgentExecutionEventKind::StepChanged,Some(step),None,json!({"change":"recheck_requested","operation_id":operation,"command_request":command}));
        let rows=self.repository.request_step_reconciliation(owner,execution,request.expected_execution_version,step,request.expected_step_version,&event).await?;
        self.scheduler.start(owner.into(),execution.into());self.publish().await;domain_mapper::detail(rows)
    }

    pub async fn replace_step_with_existing(&self,owner:&str,actor:&AgentExecutionActor,execution:&str,step:&str,
        request:nomifun_api_types::ReplaceExecutionStepWithExistingRequest)->Result<AgentExecutionDetail,AppError> {
        let engine=self.clone();let owner=owner.to_owned();let actor=actor.clone();let execution=execution.to_owned();let step=step.to_owned();
        self.lifecycle.spawn(async move {engine.replace_step_with_existing_inner(&owner,&actor,&execution,&step,request).await})
            .await.map_err(|error|AppError::Internal(format!("task replacement owner failed: {error}")))?
    }

    async fn replace_step_with_existing_inner(&self,owner:&str,actor:&AgentExecutionActor,execution:&str,step:&str,
        request:nomifun_api_types::ReplaceExecutionStepWithExistingRequest)->Result<AgentExecutionDetail,AppError> {
        let command=json!({"kind":"replace_step_with_existing","owner_id":owner,"execution_id":execution,"step_id":step,"request":request,"actor":actor});
        let operation=format!("replace-step:{}",nomifun_agent_contracts::digest_payload(&command).map_err(|error|AppError::Internal(error.to_string()))?.as_ref());
        if self.task_command_was_accepted(owner,execution,&operation,&command).await? {return self.detail(owner,execution).await;}
        let params=self.replacement_params(owner,execution,step,request.expected_step_version,&request.replacement_step_id,request.expected_replacement_step_version).await?;
        let event=actor_event(actor,AgentExecutionEventKind::PlanChanged,Some(step),None,json!({"change":"replaced_with_existing","operation_id":operation,"command_request":command}));
        let rows=self.repository.replace_step_with_existing(owner,execution,request.expected_execution_version,&params,&event).await?;
        self.scheduler.request_conversation_cleanup(execution.into());self.scheduler.start(owner.into(),execution.into());self.publish().await;
        domain_mapper::detail(rows)
    }

    pub async fn collaboration_has_step_job(&self,owner:&str,execution:&str,step:&str,attempt:Option<&str>,observed_step_version:Option<i64>)
        ->Result<bool,AppError> {
        self.repository.get_step(owner,execution,step).await?
            .ok_or_else(||AppError::NotFound("delegated task".into()))?;
        Ok(self.scheduler.has_step_job(execution,step,attempt,observed_step_version))
    }

    pub async fn collaboration_has_dispatch_owner(&self,owner:&str,execution:&str)->Result<bool,AppError> {
        self.repository.get_execution(owner,execution).await?.ok_or_else(||AppError::NotFound("delegated execution".into()))?;
        // Initial plan construction is an existing tracked producer before a
        // scheduler generation exists. Only its live guard counts, never a
        // Planning status or a persisted lease.
        Ok(self.scheduler.has_dispatch_owner(execution)||self.planning_controls.has_live_owner(execution)||self.has_restart_owner(execution))
    }

    pub(super) async fn task_command_was_accepted(&self,owner:&str,execution:&str,operation:&str,request:&serde_json::Value)
        ->Result<bool,AppError> {
        let mut cursor=0;
        loop {
            let events=self.events(owner,execution,Some(cursor),Some(200)).await?;
            for event in &events {
                if event.payload.get("operation_id").and_then(serde_json::Value::as_str)==Some(operation) {
                    if event.payload.get("command_request")!=Some(request) {
                        return Err(AppError::Conflict("task command identity was reused with different content".into()));
                    }
                    return Ok(true);
                }
            }
            if events.len()<200 {return Ok(false);}
            cursor=events.last().expect("full page").sequence;
        }
    }
}
