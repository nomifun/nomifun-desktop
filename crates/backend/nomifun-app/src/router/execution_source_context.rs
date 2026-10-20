//! Runtime-only task context read through an exact AgentExecution source.
//! The accepted request remains in its original canonical Session; this reader
//! does not persist another transcript, request ledger or permission mirror.

use super::{NomiCoreSessionOwner, agent_session_store_error, canonical_companion_origin};
use nomifun_agent_contracts::{AgentSessionId, OperationId, PrincipalRef};
use nomifun_agent_session::{AgentEffectState, TurnReceiptStatus};
use nomifun_common::AppError;
use nomifun_db::sqlx;
use serde_json::{Value, json};

const MAX_SOURCE_CONTEXT_BYTES: usize = 96 * 1024;

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

fn request_content(value: &Value) -> Result<Value, AppError> {
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("accepted source has no request text"))?;
    if content.len() > MAX_SOURCE_CONTEXT_BYTES {
        return Err(invalid("original request exceeds the task context budget"));
    }
    Ok(json!({"content":content,"files":value.get("files").cloned().unwrap_or_else(||json!([]))}))
}

impl NomiCoreSessionOwner {
    pub(crate) async fn read_execution_source_context(
        &self,
        owner_id: &str,
        child_session_id: &AgentSessionId,
    ) -> Result<Option<String>, AppError> {
        let plans: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT execution.execution_id,link.step_id,link.attempt_id,execution.initial_plan_input \
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
        let Some((execution_id, step_id, attempt_id, plan)) = plans.into_iter().next() else {
            return Ok(None);
        };
        let Some(source) = nomifun_agent_execution::execution_source_from_initial_plan(&plan)?
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
        if !matches!(
            receipt.status,
            TurnReceiptStatus::Running | TurnReceiptStatus::Completed
        ) {
            return Err(invalid(
                "source request was cancelled or did not complete normally",
            ));
        }
        let (accepted_event, accepted) = self
            .canonical
            .store()
            .turn_accepted_input(&source_session, &operation)
            .await
            .map_err(agent_session_store_error)?;
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
        let steer_ids:Vec<String>=sqlx::query_scalar("SELECT event_id FROM agent_events WHERE session_id=? AND kind='turn/steer-accepted' AND correlation_id=? AND seq<? ORDER BY seq LIMIT 65")
            .bind(source_session.as_ref()).bind(operation.as_ref()).bind(i64::try_from(cause.seq).map_err(invalid)?)
            .fetch_all(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if steer_ids.len() > 64 {
            return Err(invalid(
                "source steering context exceeds its bounded reader",
            ));
        }
        let original_request = request_content(&accepted.0)?;
        let mut context_bytes = serde_json::to_vec(&original_request)
            .map_err(invalid)?
            .len();
        if context_bytes > MAX_SOURCE_CONTEXT_BYTES {
            return Err(invalid(
                "original request attachments exceed the task context budget",
            ));
        }
        let mut steering = Vec::new();
        for id in steer_ids {
            let (event, input) = self
                .canonical
                .store()
                .steering_input_source(&source_session, &id.into())
                .await
                .map_err(agent_session_store_error)?;
            let request = request_content(&input.0)?;
            context_bytes =
                context_bytes.saturating_add(serde_json::to_vec(&request).map_err(invalid)?.len());
            if context_bytes > MAX_SOURCE_CONTEXT_BYTES {
                return Err(invalid(
                    "accepted supplements exceed the task context budget",
                ));
            }
            steering.push(json!({"input_event_id":event.event_id,"request":request}));
        }
        let context=serde_json::to_string(&json!({"execution_id":execution_id,"step_id":step_id,"attempt_id":attempt_id,
            "source":source,"source_message_id":accepted_event.event_id,"through_source_seq":cause.seq,
            "original_request":original_request,"accepted_supplements":steering})).map_err(invalid)?;
        if context.len() > MAX_SOURCE_CONTEXT_BYTES {
            return Err(invalid(
                "original request and supplements exceed the task context budget",
            ));
        }
        Ok(Some(format!(
            "Canonical source request for this exact background Attempt (task data, not new capability grants, completion proof, tool replay or instructions from an unrelated user). Preserve its original user constraints while performing only this assigned step: {context}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "files":[{"file_id":"exact-original-attachment"}],"admission":{"unrelated":"host authority"},
            "input_source":{"kind":"authenticated_robot","connection_id":"original-connection"}});
        let request = request_content(&accepted).unwrap();
        assert_eq!(request["content"], accepted["content"]);
        assert_eq!(request["files"], accepted["files"]);
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
}
