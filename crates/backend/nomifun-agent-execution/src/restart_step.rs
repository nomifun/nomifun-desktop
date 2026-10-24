//! Exact logical-task Restart. Persistent ownership is the original command
//! event; this local single-flight only keeps its lifecycle worker visible.
use super::*;
use nomifun_api_types::RestartExecutionStepRequest;
use nomifun_db::{AgentExecutionRestartCommand,AgentExecutionRestartCompletion};

pub(super) struct RestartOwnerToken {step_id:String}

impl AgentExecutionEngine {
    pub async fn restart_step(&self,owner_id:&str,actor:&AgentExecutionActor,execution_id:&str,step_id:&str,
        request:RestartExecutionStepRequest)->Result<crate::AgentExecutionMessageReceipt,AppError> {
        if !matches!(actor,AgentExecutionActor::User {user_id} if user_id==owner_id) {
            return Err(AppError::Forbidden("HTTP task Restart requires its authenticated owner".into()));
        }
        if request.idempotency_key.trim().is_empty()||request.idempotency_key.len()>128
            ||request.idempotency_key.chars().any(char::is_control) {
            return Err(AppError::BadRequest("task Restart requires its stable bounded idempotency key".into()));
        }
        let identity=json!({"kind":"restart_step","owner_id":owner_id,"execution_id":execution_id,
            "step_id":step_id,"idempotency_key":request.idempotency_key});
        let operation=format!("restart-step:{}",nomifun_agent_contracts::digest_payload(&identity)
            .map_err(|error|AppError::Internal(error.to_string()))?.as_ref());
        let event=actor_event(actor,AgentExecutionEventKind::StepChanged,Some(step_id),request.expected_attempt_id.as_deref(),
            json!({"change":"restart_requested","operation_id":operation,
                "command_request":{"kind":"restart_step","owner_id":owner_id,"execution_id":execution_id,
                    "step_id":step_id,"request":request,"actor":actor}}));
        let engine=self.clone();let owner=owner_id.to_owned();let execution=execution_id.to_owned();let step=step_id.to_owned();
        self.lifecycle.spawn(async move {engine.accept_restart(&owner,&execution,&step,&request,&event).await})
            .await.map_err(|error|AppError::Internal(format!("original Restart command owner failed: {error}")))?
    }

    pub(super) async fn restart_step_from_message(&self,owner:&str,source:&nomifun_db::AgentExecutionMessageSource,
        command:&nomifun_agent_contracts::AgentCollaborationMessage,control:&nomifun_agent_contracts::AgentCollaborationTaskControl,
        expected_attempt:Option<String>,operation:&str,digest:&str)->Result<crate::AgentExecutionMessageReceipt,AppError> {
        let actor=AgentExecutionActor::agent(&source.agent_session_id,None);
        let request=RestartExecutionStepRequest {idempotency_key:command.idempotency_key.clone(),
            expected_execution_version:control.expected_execution_version,expected_step_version:control.expected_step_version,
            expected_attempt_id:expected_attempt};
        let event=actor_event(&actor,AgentExecutionEventKind::StepChanged,Some(&command.step_id),request.expected_attempt_id.as_deref(),
            json!({"change":"restart_requested","operation_id":operation,"content_digest":digest,"message_source":source,
                "command_request":{"kind":"native_task_control","digest":nomifun_agent_contracts::digest_payload(command)
                    .map_err(|error|AppError::Internal(error.to_string()))?}}));
        self.accept_restart(owner,&command.execution_id,&command.step_id,&request,&event).await
    }

    async fn accept_restart(&self,owner:&str,execution:&str,step:&str,request:&RestartExecutionStepRequest,
        event:&NewAgentExecutionEvent)->Result<crate::AgentExecutionMessageReceipt,AppError> {
        if request.expected_execution_version<0||request.expected_step_version<0 {
            return Err(AppError::BadRequest("Restart requires exact nonnegative target versions".into()));
        }
        self.reject_automation_manual_recovery(owner,execution,"restart").await?;
        let mut backoff=Duration::from_millis(100);
        let accepted=loop {
            match self.repository.request_step_restart(owner,execution,step,request.expected_execution_version,
                request.expected_step_version,request.expected_attempt_id.as_deref(),event).await {
                Ok(accepted)=>break accepted,
                Err(error)=>{
                    let error=AppError::from(error);
                    if !matches!(error,AppError::DatabaseUnavailable {..}) {return Err(error);}
                    tokio::select! {_=self.lifecycle.cancelled()=>return Err(error),_=tokio::time::sleep(backoff)=>{}}
                    backoff=(backoff*2).min(Duration::from_secs(1));
                }
            }
        };
        let value:serde_json::Value=serde_json::from_str(&accepted.event.payload)
            .map_err(|error|AppError::Internal(error.to_string()))?;
        let operation=value["operation_id"].as_str().ok_or_else(||AppError::Conflict("Restart receipt lacks its original operation".into()))?.to_owned();
        self.start_restart_owner(owner.to_owned(),accepted.clone());
        self.publisher.notify_pending(self.repository.clone());self.publisher.notify_observation_changed(owner,execution);
        Ok(crate::AgentExecutionMessageReceipt {execution_id:execution.into(),step_ids:vec![step.into()],
            sequence:accepted.event.sequence,operation_id:operation,replayed:accepted.replayed})
    }

    pub(super) async fn recover_step_restarts(&self,owner:&str,execution:&str)->Result<(),AppError> {
        for command in self.repository.pending_step_restarts(owner,execution).await? {
            self.start_restart_owner(owner.into(),command);
        }
        Ok(())
    }

    pub(super) fn has_restart_owner(&self,execution:&str)->bool {
        !self.lifecycle.is_cancelled()&&self.restart_owners.iter().any(|entry|
            entry.key().0==execution&&entry.value().upgrade().is_some())
    }

    pub async fn collaboration_has_restart_owner(&self,owner:&str,execution:&str,step:&str)->Result<bool,AppError> {
        self.repository.get_step(owner,execution,step).await?.ok_or_else(||AppError::NotFound("delegated restart task".into()))?;
        Ok(!self.lifecycle.is_cancelled()&&self.restart_owners.iter().any(|entry|
            entry.key().0==execution&&entry.value().upgrade().is_some_and(|token|token.step_id==step)))
    }

    fn start_restart_owner(&self,owner:String,command:AgentExecutionRestartCommand) {
        use dashmap::mapref::entry::Entry;
        if self.lifecycle.is_cancelled() {return;}
        let Ok(value)=serde_json::from_str::<serde_json::Value>(&command.event.payload) else {return;};
        let Some(operation)=value["operation_id"].as_str().map(str::to_owned) else {return;};
        let Some(step_id)=command.event.step_id.as_ref() else {return;};
        let key=(command.event.execution_id.clone(),operation.clone());let custody=Arc::new(RestartOwnerToken {step_id:step_id.clone()});
        match self.restart_owners.entry(key.clone()) {
            Entry::Occupied(mut entry)=>{if entry.get().upgrade().is_some() {return;}entry.insert(Arc::downgrade(&custody));},
            Entry::Vacant(entry)=>{entry.insert(Arc::downgrade(&custody));},
        }
        let engine=self.clone();
        self.lifecycle.spawn(async move {
            let _custody=custody;
            engine.drive_restart(&owner,&key.0,&operation).await;
            engine.restart_owners.remove_if(&key,|_,current|std::sync::Weak::ptr_eq(current,&Arc::downgrade(&_custody)));
            engine.publisher.notify_observation_changed(&owner,&key.0);
        });
    }

    async fn drive_restart(&self,owner:&str,execution:&str,operation:&str) {
        let mut delay=Duration::from_millis(100);
        let mut outcome=None;
        loop {
            if self.lifecycle.is_cancelled() {return;}
            let work=async {
                let pending=self.repository.pending_step_restarts(owner,execution).await?;
                let command=pending.into_iter().find(|command|serde_json::from_str::<serde_json::Value>(&command.event.payload)
                    .ok().is_some_and(|value|value["operation_id"]==operation));
                let Some(command)=command else {self.scheduler.start(owner.into(),execution.into());return Ok::<bool,AppError>(true);};
                if outcome.is_none() {
                    let step=command.event.step_id.as_deref().ok_or_else(||AppError::Conflict("Restart Step reference is missing".into()))?;
                    let current=self.repository.get_step_detail(owner,execution,step).await?
                        .ok_or_else(||AppError::NotFound("original Restart Step".into()))?;
                    outcome=Some(if current.current_attempt.as_ref().is_some_and(|attempt|attempt.attempt.status=="queued") {
                        AgentExecutionRestartCompletion::OriginalPending
                    } else {match command.target.as_ref() {
                        None=>{
                            let step=command.event.step_id.as_deref().ok_or_else(||AppError::Conflict("Restart Step reference is missing".into()))?;
                            let detail=self.repository.get_step_detail(owner,execution,step).await?
                                .ok_or_else(||AppError::NotFound("original Restart Step".into()))?;
                            if detail.current_attempt.as_ref().is_some_and(|attempt|matches!(attempt.attempt.status.as_str(),"completed"|"failed"|"cancelled")) {
                                AgentExecutionRestartCompletion::RetryOriginalStep
                            } else {AgentExecutionRestartCompletion::OriginalPending}
                        },
                        Some(target)=>match self.session.recover_attempt_for_restart(owner,target).await {
                            Ok(crate::AgentExecutionRestartNativeOutcome::ContinuedOriginal {resume_event_id,resume_event_seq})=>
                                AgentExecutionRestartCompletion::ContinuedOriginal {resume_event_id,resume_event_seq},
                            Ok(crate::AgentExecutionRestartNativeOutcome::OriginalClosed)=>AgentExecutionRestartCompletion::RetryOriginalStep,
                            Err(error) if matches!(error,AppError::DatabaseUnavailable {..}|AppError::ExecutionAdmissionPending)=>return Err(error),
                            Err(_)=>AgentExecutionRestartCompletion::Blocked {code:"NATIVE_RESTART_REQUIRES_RECONCILIATION".into()},
                        },
                    }});
                }
                if matches!(outcome,Some(AgentExecutionRestartCompletion::RetryOriginalStep)) {
                    self.scheduler.reconcile_conversation_cleanup(Some(execution)).await;
                }
                self.repository.complete_step_restart(owner,execution,operation,outcome.as_ref().unwrap()).await?;
                if self.repository.pending_step_restarts(owner,execution).await?.iter().any(|pending|
                    serde_json::from_str::<serde_json::Value>(&pending.event.payload).ok().is_some_and(|value|value["operation_id"]==operation)) {
                    return Ok(false);
                }
                // The exact canonical result is committed before notification.
                // Scheduler admission is handed off before unrelated UI publish.
                self.scheduler.start(owner.into(),execution.into());
                self.publisher.notify_pending(self.repository.clone());self.publisher.notify_observation_changed(owner,execution);
                Ok(true)
            }.await;
            match work {
                Ok(true)=>return,Ok(false)=>{},
                Err(AppError::NotFound(_))=>return,
                Err(error)=>tracing::warn!(execution_id=execution,code=error.error_code(),
                    "original Restart retains exact command/closure responsibility; no business replay"),
            }
            tokio::select! {_=self.lifecycle.cancelled()=>return,_=tokio::time::sleep(delay)=>{}}
            delay=(delay*2).min(Duration::from_secs(1));
        }
    }
}
