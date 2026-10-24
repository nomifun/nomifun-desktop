//! Source-authored work appends to its original Execution. A completed child
//! accepts a new Step in its own Session without retrying prior work.
use super::*;
use nomifun_db::AgentExecutionSourceAppend;

impl AgentExecutionEngine {
    pub(super) async fn append_source_batch(&self, owner_id:&str, command:&CreateAgentExecutionParams)
        ->Result<Option<AgentExecution>,AppError> {
        let Some(source)=execution_source_from_initial_plan(&command.initial_plan_input)? else {return Ok(None);};
        let actor=AgentExecutionActor::agent(&source.agent_session_id,None);
        loop {
            if let Some(row)=self.repository.find_source_command(owner_id,command).await? {
                self.publish().await;
                return domain_mapper::execution(row,Some(source.agent_session_id.clone())).map(Some);
            }
            let links=self.repository.resolve_conversation_link(owner_id,&source.agent_session_id).await?;
            let mut matching=None;
            let mut reply_turns=None;
            for link in links.iter().filter(|link|link.relation=="lead") {
                let Some(row)=self.repository.get_execution(owner_id,&link.execution_id).await? else {continue;};
                let Some(original)=execution_source_from_initial_plan(&row.initial_plan_input)? else {continue;};
                if original.agent_session_id!=source.agent_session_id {continue;}
                if original.turn_operation_id!=source.turn_operation_id&&reply_turns.is_none() {
                    reply_turns=Some(self.repository.source_reply_turns(owner_id,&source.agent_session_id,&source.turn_operation_id).await?);
                }
                if original.turn_operation_id==source.turn_operation_id
                    ||reply_turns.as_ref().is_some_and(|turns:&Vec<String>|turns.contains(&original.turn_operation_id)) {
                    if row.status==AgentExecutionStatus::Cancelled.as_str() {
                        return Err(AppError::Conflict("the original source delegation was explicitly stopped".into()));
                    }
                    if matching.as_ref().is_none_or(|prior:&nomifun_db::AgentExecutionRow|prior.created_at<row.created_at) {
                        matching=Some(row);
                    }
                }
            }
            let Some(row)=matching else {return Ok(None);};
            if row.status==AgentExecutionStatus::Planning.as_str() {
                let changed=self.changed_for(owner_id,&row.execution_id);
                let notified=changed.notified();tokio::pin!(notified);notified.as_mut().enable();
                let current=self.repository.get_execution(owner_id,&row.execution_id).await?
                    .ok_or_else(||AppError::NotFound("original source Execution is unavailable".into()))?;
                if current.status!=AgentExecutionStatus::Planning.as_str() {continue;}
                tokio::select! {
                    _=self.lifecycle.cancelled()=>return Err(AppError::Conflict("collaboration owner is shutting down".into())),
                    _=notified=>{},
                    _=tokio::time::sleep(Duration::from_secs(30))=>{},
                }
                continue;
            }
            let detail=self.detail(owner_id,&row.execution_id).await?;
            let participants=active_participants(&detail);
            let initial:InitialPlanningCommand=serde_json::from_str(&command.initial_plan_input)
                .map_err(|error|AppError::Internal(format!("invalid source delegation: {error}")))?;
            let plan=match initial {
                InitialPlanningCommand::Explicit {plan,..}=>plan,
                InitialPlanningCommand::Automatic {..}=>self.produce_plan(owner_id,&row.execution_id,
                    PlanningTarget {version:detail.execution.version,delegation:None},&command.goal,&participants).await?,
                _=>return Err(AppError::Conflict("unsupported source batch".into())),
            };
            let materialized=plan_materializer::materialize(plan,&participants)?;
            let result=self.repository.append_source_steps(owner_id,&row.execution_id,detail.execution.version,
                &AgentExecutionSourceAppend::Delegation {command:command.clone()},
                &AppendAgentExecutionStepsParams {new_steps:materialized.steps,new_dependencies:materialized.dependencies},
                &actor_event(&actor,AgentExecutionEventKind::PlanChanged,None,None,json!({}))).await;
            match result {
                Ok(result)=>{
                    self.publish().await;
                    let detail=domain_mapper::detail(result.detail)?;
                    if matches!(detail.execution.status,AgentExecutionStatus::Running|AgentExecutionStatus::WaitingInput) {
                        self.scheduler.start(owner_id.to_owned(),row.execution_id);
                    }
                    return Ok(Some(detail.execution));
                }
                Err(nomifun_db::DbError::Conflict(_))=>{
                    let after=self.detail(owner_id,&row.execution_id).await?;
                    if after.execution.version!=detail.execution.version {continue;}
                    return Err(AppError::Conflict("the source cannot append work in its current state".into()));
                }
                Err(error)=>return Err(error.into()),
            }
        }
    }

    pub(super) async fn followup_completed_child(&self,owner_id:&str,message_source:&nomifun_db::AgentExecutionMessageSource,
        execution_id:&str,step_id:&str,content:&str,operation:&str,digest:&str)->Result<crate::AgentExecutionMessageReceipt,AppError> {
        let sender_session=message_source.agent_session_id.as_str();
        loop {
            let detail=self.detail(owner_id,execution_id).await?;
            let predecessor=current_step(&detail,step_id)?;
            if predecessor.status!=ExecutionStepStatus::Completed||predecessor.kind!=ExecutionStepKind::Agent {
                return Err(AppError::Conflict("only a completed child accepts a new followup work unit".into()));
            }
            let participants=active_participants(&detail);
            let participant_index=participants.iter().position(|participant|
                predecessor.assigned_participant_id.as_deref()==Some(participant.participant_id.as_str()))
                .ok_or_else(||AppError::Conflict("the original child configuration is no longer active".into()))?;
            let plan=PlannedExecution {steps:vec![PlannedExecutionStep {
                title:predecessor.title.clone(),spec:content.into(),profile:predecessor.profile.clone(),
                kind:ExecutionStepKind::Agent,agent_mode:Some(nomifun_common::AgentStepMode::Normal),depends_on:vec![],
                participant_index:Some(participant_index),assignment_rationale:Some("explicit input to the original child".into()),
                role:predecessor.role.clone(),tool_policy:predecessor.tool_policy,fanout_group:None,control_policy:None,
                failure_policy:predecessor.failure_policy,
            }]};
            let mut materialized=plan_materializer::materialize(plan,&participants)?;
            materialized.steps[0].spec=content.to_owned();
            // Preserve the exact original resource ceiling, including fields
            // whose optional JSON representation predates the current DTO.
            let row=self.repository.get_step(owner_id,execution_id,step_id).await?
                .ok_or_else(||AppError::NotFound("completed child Step".into()))?;
            materialized.steps[0].profile=row.profile;
            let links=self.repository.resolve_conversation_link(owner_id,sender_session).await?;
            let mut attempts=links.iter().filter(|link|link.execution_id==execution_id&&link.active
                &&matches!(link.relation.as_str(),"attempt"|"automation"));
            let attempt=attempts.next().and_then(|link|link.attempt_id.clone());
            if attempts.next().is_some() {return Err(AppError::Conflict("ambiguous followup sender Attempt".into()));}
            let actor=AgentExecutionActor::agent(sender_session,attempt);
            let result=self.repository.append_source_steps(owner_id,execution_id,detail.execution.version,
                &AgentExecutionSourceAppend::ChildFollowup {message_source:message_source.clone(),
                    original_step_id:step_id.into(),operation_id:operation.into(),content_digest:digest.into()},
                &AppendAgentExecutionStepsParams {new_steps:materialized.steps,new_dependencies:vec![]},
                &actor_event(&actor,AgentExecutionEventKind::PlanChanged,Some(step_id),None,json!({}))).await;
            match result {
                Ok(result)=>{
                    self.publish().await;
                    let receipt=crate::AgentExecutionMessageReceipt {execution_id:execution_id.into(),step_ids:result.added_step_ids,
                        sequence:result.detail.execution.event_sequence,operation_id:operation.into(),replayed:false};
                    let detail=domain_mapper::detail(result.detail)?;
                    if matches!(detail.execution.status,AgentExecutionStatus::Running|AgentExecutionStatus::WaitingInput) {
                        self.scheduler.start(owner_id.to_owned(),execution_id.to_owned());
                    }
                    return Ok(receipt);
                }
                Err(nomifun_db::DbError::Conflict(_))=>{
                    let after=self.detail(owner_id,execution_id).await?;
                    if after.execution.version!=detail.execution.version {continue;}
                    return Err(AppError::Conflict("the child cannot accept a new input in its current state".into()));
                }
                Err(error)=>return Err(error.into()),
            }
        }
    }
}
