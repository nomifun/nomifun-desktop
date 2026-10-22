//! Runtime-only task context read through an exact AgentExecution source.
//! The accepted request remains in its original canonical Session; this reader
//! does not persist another transcript, request ledger or permission mirror.

use super::{NomiCoreSessionOwner, agent_session_store_error, canonical_companion_origin};
use nomifun_agent_contracts::{AgentSessionId, OperationId, PrincipalRef};
use nomifun_agent_session::{AgentEffectState, TurnReceiptStatus};
use nomifun_api_types::SendMessageRequest;
use nomifun_common::AppError;
use nomifun_db::sqlx;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_SOURCE_CONTEXT_BYTES: usize = 96 * 1024;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInputSelections {
    files: Vec<String>,
    inject_skills: Vec<String>,
}

impl SourceInputSelections {
    fn extend(&mut self, value: &Value) -> Result<(), AppError> {
        let next: Self = serde_json::from_value(json!({
            "files":value.get("files").cloned().unwrap_or_else(||json!([])),
            "inject_skills":value.get("inject_skills").cloned().unwrap_or_else(||json!([])),
        })).map_err(invalid)?;
        for file in next.files {
            if !self.files.contains(&file) { self.files.push(file); }
        }
        for skill in next.inject_skills {
            if !self.inject_skills.contains(&skill) { self.inject_skills.push(skill); }
        }
        Ok(())
    }
}

struct PreparedExecutionSourceContext {
    text: String,
    source: nomifun_agent_execution::AgentExecutionSource,
    input: CanonicalSourceInput,
}

/// Derive scheduling policy only from the exact server-authored accepted
/// source. Mutable job state is not a replay dependency, and these rules are
/// never another accepted user task or part of its requirement citations.
pub(in crate::router) fn scheduled_turn_instructions(
    principal: &PrincipalRef,
    session: &AgentSessionId,
    operation: &str,
    accepted: &Value,
) -> Result<Option<String>, AppError> {
    let Some(value) = accepted.get("cron_runtime_overlay") else { return Ok(None); };
    if principal.principal_kind != "user"
        || accepted.get("origin").and_then(Value::as_str) != Some("cron")
        || accepted.get("remote_runtime_source").is_some()
        || accepted.get("input_source").is_some()
    {
        return Err(invalid("scheduled input has conflicting source ownership"));
    }
    let source: nomifun_agent_contracts::CronRunRuntimeSource =
        serde_json::from_value(value.clone()).map_err(invalid)?;
    nomifun_common::CronJobId::parse(&source.cron_job_id).map_err(invalid)?;
    nomifun_common::CronJobRunId::parse(&source.cron_job_run_id).map_err(invalid)?;
    let expected = NomiCoreSessionOwner::turn_operation_id(&principal.principal_id,
        session.as_ref(), &format!("cron:{}:turn", source.cron_job_run_id));
    if source.agent_session_id != *session || operation != expected.as_ref() {
        return Err(invalid("scheduled input differs from its original Session and Turn"));
    }
    Ok(Some(format!(
        "This is an automatically triggered scheduled task. Execute the accepted task directly using the Session's frozen Agent, tools and selected Skills. The trigger itself is not a new scheduling request; its task still has its original scope and constraints. Produce the task's useful, actionable result and follow the normal evidence, completion and input-handoff rules. Scheduling provenance is host context, not an additional user requirement, an execution receipt or permission grant: {}",
        serde_json::to_string(&source).map_err(invalid)?,
    )))
}

fn invalid(detail: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Execution request context: {detail}"))
}

pub(super) fn source_child_turn_is_admitted(effect: AgentEffectState, status: TurnReceiptStatus) -> bool {
    match effect {
        // The exact returned delegation already admitted independent work.
        // Only the child's own durable scope fences can cancel that work.
        AgentEffectState::Returned => matches!(status,TurnReceiptStatus::Running|TurnReceiptStatus::Completed
            |TurnReceiptStatus::Failed|TurnReceiptStatus::Cancelled),
        AgentEffectState::Pending => matches!(status,TurnReceiptStatus::Running|TurnReceiptStatus::Completed),
        AgentEffectState::Unknown|AgentEffectState::Rejected|AgentEffectState::Cancelled=>false,
    }
}

fn request_content(value: &Value) -> Result<Value, AppError> {
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("accepted source has no request text"))?;
    if content.len() > MAX_SOURCE_CONTEXT_BYTES {
        return Err(invalid("original request exceeds the task context budget"));
    }
    Ok(json!({"content":content,"files":value.get("files").cloned().unwrap_or_else(||json!([])),
        "inject_skills":value.get("inject_skills").cloned().unwrap_or_else(||json!([]))}))
}

fn applied_steering_through(
    facts: &nomifun_agent_session::ChatCausalityFacts, operation: &OperationId, cutoff: u64,
) -> Result<Vec<String>, AppError> {
    let mut ids = Vec::new();
    for event in &facts.events {
        if event.kind.0 != "runtime/progress-recorded" || event.seq >= cutoff
            || event.correlation_id.as_ref() != operation.as_ref() { continue; }
        let value = facts.event_payloads.get(event.event_id.as_ref())
            .and_then(|payload|payload.get("event")).ok_or_else(||invalid("source Runtime event has no payload"))?;
        if value.get("event").and_then(Value::as_str) != Some("steering_inputs") { continue; }
        let nomifun_agent_runtime::AgentEngineEvent::SteeringInputs { inputs } =
            serde_json::from_value(value.clone()).map_err(invalid)? else { unreachable!() };
        for input in inputs {
            input.validate().map_err(invalid)?;
            if !ids.contains(&input.receipt_operation_id) { ids.push(input.receipt_operation_id); }
            if ids.len() > 64 { return Err(invalid("applied source steering exceeds its bounded reader")); }
        }
    }
    Ok(ids)
}

/// The exact input visible to this original tool invocation. A later accepted
/// queue item or presentation proof cannot become the command's authority.
pub(in crate::router) async fn canonical_input_at_tool(
    store:&nomifun_agent_session::AgentSessionStore,session:&AgentSessionId,turn:&OperationId,tool:&OperationId,
    capability:&nomifun_agent_contracts::CapabilityId,action:&nomifun_agent_contracts::ActionId,
) ->Result<(nomifun_agent_contracts::SessionEventRecord,nomifun_agent_contracts::StrictJsonValue),AppError> {
    let cause=store.committed_tool_call_causation(session,turn,tool,capability,action).await.map_err(agent_session_store_error)?;
    let facts=store.turn_output_facts(session,turn).await.map_err(agent_session_store_error)?;
    if let Some(receipt)=applied_steering_through(&facts,turn,cause.seq)?.last() {
        if !facts.events.iter().any(|event|event.event_id.as_ref()==receipt&&event.kind.0=="turn/steer-accepted"
            &&event.correlation_id.as_ref()==turn.as_ref()&&event.seq<cause.seq) {
            return Err(invalid("applied command input has no original exact-Turn steering receipt"));
        }
        store.steering_input_source(session,&receipt.clone().into()).await.map_err(agent_session_store_error)
    } else {store.turn_accepted_input(session,turn).await.map_err(agent_session_store_error)}
}

pub(in crate::router) struct CanonicalSourceInput {
    source_message_id: nomifun_agent_contracts::EventId,
    original_request: Value,
    accepted_supplements: Vec<Value>,
    selections: SourceInputSelections,
}

impl CanonicalSourceInput {
    pub(in crate::router) fn prepare_request(&self, request: &mut SendMessageRequest) {
        request.files=self.selections.files.clone();
        request.inject_skills=self.selections.inject_skills.clone();
    }
}

/// Input data at an already-proven command boundary. The caller separately
/// proves delegation ownership; this reader grants no capabilities or Session
/// control and never substitutes a later accepted input or steering command.
pub(in crate::router) async fn canonical_source_input(
    store:&nomifun_agent_session::AgentSessionStore, session:&AgentSessionId, operation:&OperationId, cutoff:u64,
) ->Result<CanonicalSourceInput,AppError> {
    let facts=store.turn_output_facts(session,operation).await.map_err(agent_session_store_error)?;
    let (accepted_event,accepted)=store.turn_accepted_input(session,operation).await.map_err(agent_session_store_error)?;
    let steer_ids=applied_steering_through(&facts,operation,cutoff)?;
    let original_request=request_content(&accepted.0)?;
    let mut selections=SourceInputSelections::default();selections.extend(&accepted.0)?;
    let mut context_bytes=serde_json::to_vec(&original_request).map_err(invalid)?.len();
    let mut supplements=Vec::new();
    for id in steer_ids {
        let(event,input)=store.steering_input_source(session,&id.into()).await.map_err(agent_session_store_error)?;
        let request=request_content(&input.0)?;selections.extend(&input.0)?;
        context_bytes=context_bytes.saturating_add(serde_json::to_vec(&request).map_err(invalid)?.len());
        if context_bytes>MAX_SOURCE_CONTEXT_BYTES {return Err(invalid("accepted source supplements exceed the task context budget"));}
        supplements.push(json!({"input_event_id":event.event_id,"request":request}));
    }
    if context_bytes>MAX_SOURCE_CONTEXT_BYTES {return Err(invalid("original source input exceeds the task context budget"));}
    Ok(CanonicalSourceInput {source_message_id:accepted_event.event_id,original_request,accepted_supplements:supplements,selections})
}

impl NomiCoreSessionOwner {
    /// Deliver the selections visible to an admitted parent message, including
    /// messages to an already-running child. The immutable command event is
    /// the boundary; later inputs and the parent's current runtime are irrelevant.
    pub(in crate::router) async fn prepare_execution_steering_input(
        &self, owner_id: &str, child_session: &AgentSessionId, operation_id: &str,
        mut request: SendMessageRequest,
    ) -> Result<SendMessageRequest, AppError> {
        let commands: Vec<(String,String,String)> = sqlx::query_as(
            "SELECT event.execution_id,event.step_id,json_extract(event.payload,'$.message_source') \
             FROM agent_execution_events event \
             JOIN agent_executions execution ON execution.execution_id=event.execution_id \
             JOIN agent_execution_attempts attempt ON attempt.execution_id=event.execution_id \
               AND attempt.step_id=event.step_id AND attempt.attempt_id=event.attempt_id \
             JOIN conversation_execution_links link ON link.execution_id=attempt.execution_id \
               AND link.step_id=attempt.step_id AND link.attempt_id=attempt.attempt_id \
             WHERE execution.user_id=? AND link.conversation_id=? \
               AND link.relation IN ('attempt','automation') \
               AND event.event_type='step_changed' \
               AND json_extract(event.payload,'$.change')='conversation_effect_requested' \
               AND json_extract(event.payload,'$.effect')='steer' \
               AND json_extract(event.payload,'$.operation_id')=? \
               AND json_type(event.payload,'$.message_source')='object' LIMIT 2")
            .bind(owner_id).bind(child_session.as_ref()).bind(operation_id)
            .fetch_all(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if commands.len()>1 {return Err(invalid("steering command has multiple original owners"));}
        let Some((execution,step,source))=commands.into_iter().next() else {return Ok(request);};
        let source:nomifun_db::AgentExecutionMessageSource=serde_json::from_str(&source).map_err(invalid)?;
        let mut tx=self.pool.begin().await.map_err(|error|AppError::Internal(error.to_string()))?;
        let digest=nomifun_agent_contracts::digest_bytes(request.content.as_bytes());
        let (cutoff,_)=nomifun_db::agent_message_command_bound_tx(&mut tx,&source,&execution,&step,operation_id,digest.as_ref()).await?;
        tx.commit().await.map_err(|error|AppError::Internal(error.to_string()))?;
        let input=canonical_source_input(self.canonical.store(),&source.agent_session_id.into(),
            &source.turn_operation_id.into(),cutoff).await?;
        input.prepare_request(&mut request);
        Ok(request)
    }

    /// Explicit question replies can continue only their exact prior task.
    /// The same transaction-scoped proof is used by command admission.
    pub(crate) async fn collaboration_reply_source_turn(
        &self, owner_id: &str, session: &AgentSessionId, current: &OperationId,
    ) -> Result<Option<OperationId>, AppError> {
        Ok(self.collaboration_source_turns(owner_id,session,current).await?.into_iter().next())
    }

    /// The shared reader checks typed question/terminal identity, exact binding,
    /// clear floor and decreasing accepted-input ancestry. It grants no Session-
    /// wide authority and never selects latest work or creates a Runtime.
    pub(crate) async fn collaboration_source_turns(
        &self, owner_id: &str, session: &AgentSessionId, current: &OperationId,
    ) -> Result<Vec<OperationId>, AppError> {
        let mut tx=self.pool.begin().await.map_err(|error|AppError::Internal(error.to_string()))?;
        let sources=nomifun_db::agent_reply_source_turns_tx(&mut tx,owner_id,session.as_ref(),current.as_ref()).await?;
        tx.commit().await.map_err(|error|AppError::Internal(error.to_string()))?;
        Ok(sources.into_iter().map(OperationId::from).collect())
    }
    pub(crate) async fn read_execution_source_context(
        &self,
        owner_id: &str,
        child_session_id: &AgentSessionId,
    ) -> Result<Option<String>, AppError> {
        Ok(self.execution_source_context(owner_id, child_session_id).await?.map(|prepared|prepared.text))
    }

    /// Keep the delegated task body and source provenance separate. Only exact
    /// accepted attachment/Skill selections enter the child's ordinary input
    /// preparation; its own immutable binding remains the permission ceiling.
    pub(crate) async fn prepare_execution_child_input(
        &self, owner_id: &str, child_session_id: &AgentSessionId,
        source: Option<&nomifun_agent_execution::AgentExecutionSource>,
        mut request: SendMessageRequest,
    ) -> Result<SendMessageRequest, AppError> {
        if !request.files.is_empty() || !request.inject_skills.is_empty()
            || request.origin.as_deref()!=Some("agent_execution") {
            return Err(invalid("child selections must come from its exact delegated source"));
        }
        let prepared = match self.execution_source_context(owner_id, child_session_id).await? {
            Some(prepared)=>prepared,
            None if source.is_none()=>return Ok(request),
            None=>return Err(invalid("child has no current source Attempt")),
        };
        if source.is_some_and(|source|prepared.source != *source) {
            return Err(invalid("child input source differs from its admitted Attempt"));
        }
        prepared.input.prepare_request(&mut request);
        Ok(request)
    }

    async fn execution_source_context(
        &self, owner_id: &str, child_session_id: &AgentSessionId,
    ) -> Result<Option<PreparedExecutionSourceContext>, AppError> {
        let plans: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT execution.execution_id,link.step_id,link.attempt_id \
             FROM conversation_execution_links link \
             JOIN agent_executions execution ON execution.execution_id=link.execution_id \
             JOIN agent_execution_attempts attempt ON attempt.execution_id=link.execution_id AND attempt.step_id=link.step_id AND attempt.attempt_id=link.attempt_id \
             JOIN agent_sessions session ON session.agent_session_id=link.conversation_id \
             WHERE link.conversation_id=? AND link.relation='attempt' AND link.active=1 \
               AND execution.user_id=? AND execution.deleted_at IS NULL AND attempt.status='running' \
               AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
               AND json_extract(session.owner_ref_json,'$.principal_id')=? LIMIT 2")
            .bind(child_session_id.as_ref()).bind(owner_id).bind(owner_id)
            .fetch_all(&self.pool).await.map_err(|error| AppError::Internal(error.to_string()))?;
        if plans.len() > 1 {
            return Err(invalid("child belongs to multiple active Attempts"));
        }
        let Some((execution_id, step_id, attempt_id)) = plans.into_iter().next() else {
            return Ok(None);
        };
        let repository=nomifun_db::SqliteAgentExecutionRepository::new(self.pool.clone());
        let message_source=nomifun_db::IAgentExecutionRepository::step_message_source(
            &repository,owner_id,&execution_id,&step_id,
        ).await?;
        if message_source.is_none() && nomifun_db::IAgentExecutionRepository::continuation_target(
            &repository,owner_id,&execution_id,&step_id,
        ).await?.is_some() {
            // A direct UI followup is its own accepted input. Reusing a child
            // Session does not re-inject the original delegation's selections.
            return Ok(None);
        }
        let Some(source) = nomifun_db::IAgentExecutionRepository::step_source(
            &repository,owner_id,&execution_id,&step_id,
        ).await?
        else {
            return Ok(None);
        };
        let source_session = AgentSessionId::from(source.agent_session_id.clone());
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner_id.into(),
        };
        let source_observation = self.canonical.get(&principal, &source_session).await?;
        let child_observation = self.canonical.get(&principal, child_session_id).await?;
        let source_companion = canonical_companion_origin(owner_id, &source_observation)?;
        let child_companion = canonical_companion_origin(owner_id, &child_observation)?;
        if source_companion
            .as_ref()
            .map(|scope| scope.companion_id.as_str())
            != child_companion
                .as_ref()
                .map(|scope| scope.companion_id.as_str())
        {
            return Err(invalid(
                "child and original request belong to different Companions",
            ));
        }
        let operation = OperationId::from(source.turn_operation_id.clone());
        let effect = self
            .canonical
            .store()
            .read_effect(&source_session, &source.effect_id)
            .await
            .map_err(agent_session_store_error)?
            .ok_or_else(|| invalid("exact source Effect is missing"))?;
        if effect.turn_id != operation
            || effect.operation_id.as_ref() != source.tool_operation_id
            || effect.owner_domain != "wave5.product"
            || effect.capability_module.as_ref() != "agent.collaboration"
            || !matches!(effect.action_id.as_ref(), "agent/delegate" | "agent/fork")
            || !matches!(
                effect.state,
                AgentEffectState::Pending | AgentEffectState::Returned
            )
        {
            return Err(invalid(
                "source differs from the admitted collaboration Effect",
            ));
        }
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(&source_session, &operation)
            .await
            .map_err(agent_session_store_error)?;
        if !source_child_turn_is_admitted(effect.state,receipt.status) {
            return Err(invalid(
                "source delegation has no confirmed independent child admission",
            ));
        }
        let cause = self
            .canonical
            .store()
            .committed_tool_call_causation(
                &source_session,
                &operation,
                &effect.operation_id,
                &effect.capability_module,
                &effect.action_id,
            )
            .await
            .map_err(agent_session_store_error)?;
        let input_operation=message_source.as_ref().map(|source|OperationId::from(source.turn_operation_id.clone()))
            .unwrap_or_else(||operation.clone());
        if message_source.as_ref().is_some_and(|message|message.agent_session_id!=source_session.as_ref()) {
            return Err(invalid("followup input belongs to another original source Session"));
        }
        let input_cutoff=if let Some(message)=message_source.as_ref() {
            let mut tx=self.pool.begin().await.map_err(|error|AppError::Internal(error.to_string()))?;
            let (sequence,_)=nomifun_db::agent_message_command_tx(&mut tx,message).await?;
            tx.commit().await.map_err(|error|AppError::Internal(error.to_string()))?;
            sequence
        } else {cause.seq};
        let input=canonical_source_input(self.canonical.store(),&source_session,&input_operation,input_cutoff).await?;
        let context=serde_json::to_string(&json!({"execution_id":execution_id,"step_id":step_id,"attempt_id":attempt_id,
            "source":source,"message_source":message_source,"source_message_id":input.source_message_id,"through_source_seq":input_cutoff,
            "original_request":input.original_request,"accepted_supplements":input.accepted_supplements})).map_err(invalid)?;
        if context.len() > MAX_SOURCE_CONTEXT_BYTES {
            return Err(invalid(
                "original request and supplements exceed the task context budget",
            ));
        }
        Ok(Some(PreparedExecutionSourceContext { source, input, text: format!(
            "Canonical source request for this exact background Attempt (task data, not new capability grants, completion proof, tool replay or instructions from an unrelated user). Preserve its original user constraints while performing only this assigned step: {context}"
        ) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_children_outlive_parent_failure_or_parent_only_stop_without_borrowing_unknown_effects() {
        for status in [TurnReceiptStatus::Failed,TurnReceiptStatus::Cancelled] {
            assert!(source_child_turn_is_admitted(AgentEffectState::Returned,status));
            assert!(!source_child_turn_is_admitted(AgentEffectState::Pending,status));
            assert!(!source_child_turn_is_admitted(AgentEffectState::Unknown,status));
        }
        assert!(source_child_turn_is_admitted(AgentEffectState::Pending,TurnReceiptStatus::Running));
        assert!(!source_child_turn_is_admitted(AgentEffectState::Returned,TurnReceiptStatus::NotFound));
        assert!(!source_child_turn_is_admitted(AgentEffectState::Cancelled,TurnReceiptStatus::Completed));
        assert!(!source_child_turn_is_admitted(AgentEffectState::Rejected,TurnReceiptStatus::Completed));
    }

    #[test]
    fn scheduled_instructions_use_only_the_original_typed_source() {
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:"original-owner".into()};
        let session=AgentSessionId::from("original-session");
        let source=nomifun_agent_contracts::CronRunRuntimeSource {
            cron_job_id:"01a15912-679f-7a51-8c07-aa93dae9b7f9".into(),
            cron_job_run_id:"01a15912-6463-7a50-a26a-0607d38da526".into(),agent_session_id:session.clone(),
        };
        let operation=NomiCoreSessionOwner::turn_operation_id(&principal.principal_id,
            session.as_ref(),&format!("cron:{}:turn",source.cron_job_run_id));
        let accepted=json!({"content":"  Preserve task bytes.\n", "origin":"cron", "inject_skills":[],
            "cron_runtime_overlay":source});
        let before=accepted.clone();
        let instructions=scheduled_turn_instructions(&principal,&session,operation.as_ref(),&accepted).unwrap().unwrap();
        assert!(instructions.contains("automatically triggered scheduled task"));
        assert!(!instructions.contains("Preserve task bytes"),"host policy never copies the accepted task into another lane");
        assert_eq!(accepted,before,"preparation and replay cannot amend canonical input");
        assert!(scheduled_turn_instructions(&principal,&session,operation.as_ref(),&json!({
            "content":"ordinary user says cron", "origin":"cron"})).unwrap().is_none(),
            "an origin string cannot manufacture a trusted scheduled source");
        assert!(scheduled_turn_instructions(&principal,&"other-session".into(),operation.as_ref(),&accepted).is_err());
        assert!(scheduled_turn_instructions(&principal,&session,"other-turn",&accepted).is_err());
        assert!(scheduled_turn_instructions(&PrincipalRef {principal_id:"other-owner".into(),..principal.clone()},
            &session,operation.as_ref(),&accepted).is_err());
        let mut conflicting=accepted.clone();conflicting["input_source"]=json!({"kind":"authenticated_robot"});
        assert!(scheduled_turn_instructions(&principal,&session,operation.as_ref(),&conflicting).is_err());
    }

    #[test]
    fn task_source_preserves_original_constraints_and_attachment_references() {
        let accepted = json!({"content":"Create the report, preserve existing files, and ask before sending it.",
            "files":["exact-original-attachment"],"inject_skills":["explicit-review"],"admission":{"unrelated":"host authority"},
            "input_source":{"kind":"authenticated_robot","connection_id":"original-connection"}});
        let request = request_content(&accepted).unwrap();
        assert_eq!(request["content"], accepted["content"]);
        assert_eq!(request["files"], accepted["files"]);
        assert_eq!(request["inject_skills"], accepted["inject_skills"]);
        assert!(
            request.get("admission").is_none(),
            "request data cannot manufacture host admission authority"
        );
        assert!(
            request_content(&json!({"content":"x".repeat(MAX_SOURCE_CONTEXT_BYTES+1)})).is_err(),
            "an oversized user constraint is rejected rather than silently truncated"
        );
        assert!(request_content(&json!({"content":null})).is_err());
    }

    #[test]
    fn child_selections_keep_source_order_and_do_not_turn_metadata_into_authority() {
        let mut selections = SourceInputSelections::default();
        selections.extend(&json!({"files":["image-a"],"inject_skills":["review"],
            "admission":{"capabilities":["forged"]},"workspace":"other-root"})).unwrap();
        selections.extend(&json!({"files":["image-a","image-b"],"inject_skills":["review","verify"]})).unwrap();
        assert_eq!(selections.files, ["image-a","image-b"]);
        assert_eq!(selections.inject_skills, ["review","verify"]);
        assert!(selections.extend(&json!({"files":[{"path":"unvalidated"}]})).is_err());
        assert!(selections.extend(&json!({"inject_skills":"review"})).is_err());
        assert_eq!(selections.files, ["image-a","image-b"], "failed selection decoding is atomic");
    }
}
