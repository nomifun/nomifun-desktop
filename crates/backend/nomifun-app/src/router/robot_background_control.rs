//! Recording-time, exact-source control of a durable background execution.
//! Targets are frozen before ASR; neither dispatch nor retry chooses a latest
//! execution, Attempt, Turn, native generation or CAS version.

use super::*;
use nomifun_robot::services::{RobotExecutionTarget, RobotTurnRequest};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BackgroundControlKind {
    CancelExecution,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackgroundControlInput {
    kind: BackgroundControlKind,
    target: RobotExecutionTarget,
    binding_version: u64,
    context_floor: u64,
}

fn invalid(detail: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Robot background control: {detail}"))
}

impl NomiCoreSessionOwner {
    /// Called under the original Session recording-context fence.
    pub(crate) async fn capture_robot_background_work(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        binding_version: u64,
        context_floor: u64,
    ) -> Result<(Option<RobotExecutionTarget>, bool), AppError> {
        let session = AgentSessionId::from(request.conversation_id.clone());
        let current = self
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.into(),
                },
                &session,
            )
            .await?;
        let Some(scope) = canonical_companion_origin(owner_id, &current)? else {
            return Ok((None, false));
        };
        if scope.role != nomifun_agent_contracts::CompanionSessionRole::Robot
            || scope.companion_id != request.companion_id
            || scope.robot_id.as_deref() != Some(request.robot_id.as_str())
            || current.session.agent_binding.binding_version != binding_version
        {
            return Err(invalid(
                "recording context differs from its canonical device entry",
            ));
        }
        let rows:Vec<(String,i64,String)>=sqlx::query_as("SELECT execution.execution_id,execution.version,execution.initial_plan_input FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id WHERE link.conversation_id=? AND link.relation='lead' AND link.active=1 AND execution.user_id=? AND execution.deleted_at IS NULL AND execution.status NOT IN ('completed','completed_with_failures','failed','cancelled') ORDER BY execution.created_at LIMIT 65")
            .bind(session.as_ref()).bind(owner_id).fetch_all(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if rows.len() > 64 {
            return Ok((None, true));
        }
        let mut matches = Vec::new();
        for (execution_id, version, plan) in rows {
            let Some(source) = nomifun_agent_execution::execution_source_from_initial_plan(&plan)?
            else {
                continue;
            };
            if source.agent_session_id != request.conversation_id {
                continue;
            }
            if !self
                .robot_background_source_matches(
                    owner_id,
                    request,
                    &source,
                    binding_version,
                    context_floor,
                )
                .await?
            {
                continue;
            }
            matches.push(RobotExecutionTarget {
                execution_id,
                expected_version: version,
                source_turn_operation_id: source.turn_operation_id,
                source_tool_operation_id: source.tool_operation_id,
                source_effect_id: source.effect_id,
                attempt: None,
                attempt_ambiguous: false,
            });
            if matches.len() > 1 {
                return Ok((None, true));
            }
        }
        let Some(mut target) = matches.pop() else {
            return Ok((None, false));
        };
        let (attempt, ambiguous) = self
            .capture_robot_background_attempt(owner_id, &target)
            .await?;
        target.attempt = attempt;
        target.attempt_ambiguous = ambiguous;
        Ok((Some(target), false))
    }

    async fn capture_robot_background_attempt(
        &self,
        owner_id: &str,
        target: &RobotExecutionTarget,
    ) -> Result<
        (
            Option<nomifun_robot::services::RobotExecutionAttemptTarget>,
            bool,
        ),
        AppError,
    > {
        let engine = self
            .execution_progress
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| invalid("Execution owner is unavailable"))?;
        let detail = engine.get(owner_id, &target.execution_id).await?;
        if detail.execution.version != target.expected_version {
            return Ok((None, false));
        }
        let running = detail
            .attempts
            .iter()
            .filter(|attempt| attempt.status == nomifun_common::ExecutionAttemptStatus::Running)
            .collect::<Vec<_>>();
        if running.len() > 1 {
            return Ok((None, true));
        }
        let Some(attempt) = running.into_iter().next() else {
            return Ok((None, false));
        };
        let Some(step) = detail
            .steps
            .iter()
            .find(|step| step.step_id == attempt.step_id)
        else {
            return Err(invalid("running Attempt lost its Step"));
        };
        let Some(child) = attempt.conversation_id.as_ref() else {
            return Ok((None, false));
        };
        let encoded = attempt.runtime_state.as_ref().map(Value::to_string);
        let effects = nomifun_db::AttemptConversationEffects::decode(encoded.as_deref())?;
        let key = effects.current_turn_operation_key(&attempt.attempt_id)?;
        let child_id = AgentSessionId::from(child.clone());
        let current = self
            .canonical
            .store()
            .get_live_session(&child_id)
            .await
            .map_err(agent_session_store_error)?;
        if current.owner_ref.principal_kind != "user" || current.owner_ref.principal_id != owner_id
        {
            return Err(invalid("Attempt Session belongs to another owner"));
        }
        let head = self
            .canonical
            .store()
            .head(&child_id)
            .await
            .map_err(agent_session_store_error)?;
        if head.status != "running" {
            return Ok((None, false));
        }
        let Some(operation) = head.active_turn_id.map(OperationId::from) else {
            return Ok((None, false));
        };
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(&child_id, &operation)
            .await
            .map_err(agent_session_store_error)?;
        let Some(start) = receipt.started_event else {
            return Ok((None, false));
        };
        if start.idempotency_key.as_ref() != format!("user:{owner_id}:{child}:{key}:turn") {
            return Ok((None, false));
        }
        let generation = self
            .canonical
            .store()
            .native_execution_generation(&child_id, &operation)
            .await
            .map_err(agent_session_store_error)?;
        if generation == 0 {
            return Ok((None, false));
        }
        Ok((
            Some(nomifun_robot::services::RobotExecutionAttemptTarget {
                step_id: step.step_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
                expected_step_version: step.version,
                expected_attempt_version: attempt.version,
                agent_session_id: child.clone(),
                turn_operation_id: operation.as_ref().into(),
                binding_version: current.agent_binding.binding_version,
                execution_generation: generation,
            }),
            false,
        ))
    }

    async fn robot_background_source_matches(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        source: &nomifun_agent_execution::AgentExecutionSource,
        binding_version: u64,
        context_floor: u64,
    ) -> Result<bool, AppError> {
        let session = AgentSessionId::from(request.conversation_id.clone());
        if source.agent_session_id != request.conversation_id {
            return Ok(false);
        }
        let observation = self
            .canonical
            .get(
                &PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id.into(),
                },
                &session,
            )
            .await?;
        if observation.session.agent_binding.binding_version != binding_version {
            return Ok(false);
        }
        let (revocation,floor):(i64,i64)=sqlx::query_as("SELECT COALESCE(MAX(seq),0),COALESCE(MAX(CASE WHEN kind='context/cleared' THEN seq END),0) FROM agent_events WHERE session_id=? AND kind IN ('context/cleared','session/agent-binding-changed') AND kind_version=1")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if u64::try_from(floor).ok() != Some(context_floor) {
            return Ok(false);
        }
        let operation = OperationId::from(source.turn_operation_id.clone());
        let receipt = self
            .canonical
            .store()
            .read_turn_receipt(&session, &operation)
            .await
            .map_err(agent_session_store_error)?;
        let Some(start) = receipt.started_event else {
            return Ok(false);
        };
        if u64::try_from(revocation).map_err(invalid)? > start.seq {
            return Ok(false);
        }
        let effect = self
            .canonical
            .store()
            .read_effect(&session, &source.effect_id)
            .await
            .map_err(agent_session_store_error)?
            .ok_or_else(|| invalid("captured execution lost its source Effect"))?;
        // Cancellation is an owner command even when the original delegation
        // acknowledgement became unknown. This does not replay that Effect.
        if effect.turn_id != operation
            || effect.operation_id.as_ref() != source.tool_operation_id
            || effect.owner_domain != "wave5.product"
            || effect.capability_module.as_ref() != "agent.collaboration"
            || !matches!(effect.action_id.as_ref(), "agent/delegate" | "agent/fork")
        {
            return Err(invalid(
                "captured source Effect differs from its original invocation",
            ));
        }
        self.canonical
            .store()
            .committed_tool_call_causation(
                &session,
                &operation,
                &effect.operation_id,
                &effect.capability_module,
                &effect.action_id,
            )
            .await
            .map_err(agent_session_store_error)?;
        let (_, accepted) = self
            .canonical
            .store()
            .turn_accepted_input(&session, &operation)
            .await
            .map_err(agent_session_store_error)?;
        let mut started = self
            .canonical
            .store()
            .resolved_event_payloads(&session, std::slice::from_ref(&start.event_id))
            .await
            .map_err(agent_session_store_error)?;
        let started = started
            .remove(start.event_id.as_ref())
            .ok_or_else(|| invalid("source claim admission is missing"))?;
        if !super::companion_report_delivery::reply_authority_matches(
            &started,
            &nomifun_agent_contracts::AgentHandoffBindingRefV1::from(
                &observation.session.agent_binding,
            ),
            start.seq,
            u64::try_from(revocation).map_err(invalid)?,
        )? {
            return Ok(false);
        }
        let Some(source) = accepted.0.get("input_source").cloned() else {
            return Ok(false);
        };
        let source: nomifun_agent_contracts::AgentInputSource =
            serde_json::from_value(source).map_err(invalid)?;
        Ok(
            matches!(source,nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot{companion_id,robot_id,connection_id,..}
            if companion_id==request.companion_id&&robot_id==request.robot_id&&connection_id==request.connection_id),
        )
    }

    pub(crate) async fn cancel_robot_background_work(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        key: &str,
    ) -> Result<(), AppError> {
        if request.text.trim().is_empty()
            || request.text.len() > 16 * 1024
            || key.trim().is_empty()
            || key.len() > 1024
        {
            return Err(invalid(
                "cancellation requires its complete bounded recording input and stable command key",
            ));
        }
        let captured = request
            .captured_context
            .as_ref()
            .ok_or_else(|| invalid("cancellation has no recording-time context"))?;
        if captured.background_work_ambiguous {
            return Err(invalid(
                "multiple background executions require an explicit choice",
            ));
        }
        let target = captured
            .background_work
            .as_ref()
            .ok_or_else(|| invalid("recording did not capture a background execution"))?;
        let session = AgentSessionId::from(request.conversation_id.clone());
        let _fence = self
            .session_operation_lock(session.as_ref())
            .write_owned()
            .await;
        let source = nomifun_agent_execution::AgentExecutionSource {
            agent_session_id: request.conversation_id.clone(),
            turn_operation_id: target.source_turn_operation_id.clone(),
            tool_operation_id: target.source_tool_operation_id.clone(),
            effect_id: target.source_effect_id.clone(),
        };
        let input = json!({"content":request.text,"files":[],"inject_skills":[],"hidden":false,"origin":"robot","channel_platform":"robot",
            "input_source":nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {companion_id:request.companion_id.clone(),robot_id:request.robot_id.clone(),
                connection_id:request.connection_id.clone(),presented_question_id:request.presented_question_id.clone()},
            "background_control":BackgroundControlInput {kind:BackgroundControlKind::CancelExecution,target:target.clone(),
                binding_version:captured.binding_version,context_floor:captured.context_floor}});
        let digest = digest_payload(&(owner_id, session.as_ref(), key)).map_err(invalid)?;
        let control_key = format!("robot-background-control:{}", digest.as_ref());
        let existing:Option<(String,String)>=sqlx::query_as("SELECT event_id,inline_json FROM agent_events WHERE session_id=? AND producer_id='session_api' AND idempotency_key=? AND kind='message/user-accepted' AND kind_version=1")
            .bind(session.as_ref()).bind(&control_key).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let engine = self
            .execution_progress
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| invalid("Execution owner is unavailable"))?;
        let input_event = if let Some((event_id, stored)) = existing {
            if serde_json::from_str::<Value>(&stored).map_err(invalid)? != input {
                return Err(invalid(
                    "control key was reused with another input or frozen target",
                ));
            }
            if engine
                .lookup_canonical_control_receipt(
                    owner_id,
                    &nomifun_common::AgentExecutionActor::user(owner_id),
                    &target.execution_id,
                    nomifun_api_types::VersionedAgentExecutionCommand {
                        expected_version: target.expected_version,
                    },
                    nomifun_agent_execution::AgentExecutionControlSource {
                        agent_session_id: session.as_ref().into(),
                        input_event_id: event_id.clone(),
                    },
                )
                .await?
                .is_some()
            {
                return Ok(());
            }
            // An accepted input without a successful CAS is not permission to
            // complete its old action after clear/rebind/connection revocation.
            if !self
                .robot_background_source_matches(
                    owner_id,
                    request,
                    &source,
                    captured.binding_version,
                    captured.context_floor,
                )
                .await?
            {
                return Err(invalid(
                    "unexecuted control source was revoked before cancellation",
                ));
            }
            event_id
        } else {
            if !self
                .robot_background_source_matches(
                    owner_id,
                    request,
                    &source,
                    captured.binding_version,
                    captured.context_floor,
                )
                .await?
            {
                return Err(invalid(
                    "recorded source connection, binding or context was revoked",
                ));
            }
            let detail = engine.get(owner_id, &target.execution_id).await?;
            let row:Option<String>=sqlx::query_scalar("SELECT initial_plan_input FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
                .bind(&target.execution_id).bind(owner_id).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
            if nomifun_agent_execution::execution_source_from_initial_plan(
                &row.ok_or_else(|| invalid("recorded Execution disappeared"))?,
            )?
            .as_ref()
                != Some(&source)
                || detail.execution.lead_conversation_id.as_deref() != Some(session.as_ref())
                || detail.execution.version != target.expected_version
            {
                return Err(invalid(
                    "recorded Execution changed; cancellation was not retargeted",
                ));
            }
            let receipt = self
                .canonical
                .store()
                .read_turn_receipt(&session, &source.turn_operation_id.clone().into())
                .await
                .map_err(agent_session_store_error)?;
            let start = receipt
                .started_event
                .ok_or_else(|| invalid("recorded source has no canonical start"))?;
            let event_id = Uuid::now_v7().to_string();
            let appended = self
                .canonical
                .store()
                .append_event(&nomifun_agent_contracts::SessionEventAppend {
                    agent_session_id: session.clone(),
                    event_id: event_id.clone().into(),
                    producer_id: "session_api".into(),
                    idempotency_key: control_key.into(),
                    semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                        kind: nomifun_agent_contracts::SessionEventKind(
                            "message/user-accepted".into(),
                        ),
                        kind_version: 1,
                        correlation_id: event_id.clone().into(),
                        causation_event_id: Some(start.event_id),
                        payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                            StrictJsonValue(input.clone()),
                        ),
                    },
                })
                .await
                .map_err(agent_session_store_error)?;
            if let Some(record) = appended.record {
                let created = self
                    .canonical
                    .store()
                    .session_created_at(&session)
                    .await
                    .map_err(agent_session_store_error)?;
                self.user_events.send_to_user(
                    owner_id,
                    Self::canonical_accepted_input_wire_event(&session, created, &record, &input)?,
                );
            }
            event_id
        };
        // The non-queue command namespace is intentional. Only this exact
        // Execution control consumer may use it; ordinary input draining never
        // recognizes it as an unclaimed Session input.
        engine
            .cancel_from_canonical_control(
                owner_id,
                &nomifun_common::AgentExecutionActor::user(owner_id),
                &target.execution_id,
                nomifun_api_types::VersionedAgentExecutionCommand {
                    expected_version: target.expected_version,
                },
                nomifun_agent_execution::AgentExecutionControlSource {
                    agent_session_id: session.as_ref().into(),
                    input_event_id: input_event,
                },
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn supplement_robot_background_work(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        key: &str,
        _text: &str,
        _correction: bool,
        input_id: &str,
    ) -> Result<(), AppError> {
        let captured = request
            .captured_context
            .as_ref()
            .ok_or_else(|| invalid("supplement has no recording-time context"))?;
        if captured.background_work_ambiguous {
            return Err(invalid(
                "multiple background executions require an explicit choice",
            ));
        }
        let target = captured
            .background_work
            .as_ref()
            .ok_or_else(|| invalid("recording did not capture a background execution"))?;
        if target.attempt_ambiguous {
            return Err(invalid("parallel Attempts require an explicit choice"));
        }
        let attempt=target.attempt.as_ref().ok_or_else(||invalid("the captured background task has no safe running Attempt; use its exact task control or question"))?;
        let session = AgentSessionId::from(request.conversation_id.clone());
        let _fence = self
            .session_operation_lock(session.as_ref())
            .write_owned()
            .await;
        let source = nomifun_agent_execution::AgentExecutionSource {
            agent_session_id: request.conversation_id.clone(),
            turn_operation_id: target.source_turn_operation_id.clone(),
            tool_operation_id: target.source_tool_operation_id.clone(),
            effect_id: target.source_effect_id.clone(),
        };
        if !self
            .robot_background_source_matches(
                owner_id,
                request,
                &source,
                captured.binding_version,
                captured.context_floor,
            )
            .await?
        {
            return Err(invalid(
                "recorded source connection, binding or context was revoked",
            ));
        }
        let child = AgentSessionId::from(attempt.agent_session_id.clone());
        let operation = OperationId::from(attempt.turn_operation_id.clone());
        let native = nomifun_agent_session::NativeTurnMutationFence {
            binding_version: attempt.binding_version,
            execution_generation: attempt.execution_generation,
        };
        let input = SendMessageRequest {
            content: request.text.clone(),
            files: vec![],
            inject_skills: vec![],
            hidden: false,
            origin: None,
            channel_platform: Some("robot".into()),
        };
        if input.content.trim().is_empty()
            || input.content.len() > 16 * 1024
            || input_id.trim().is_empty()
        {
            return Err(invalid(
                "supplement requires the complete bounded recording input",
            ));
        }
        let digest = digest_payload(&(
            owner_id,
            session.as_ref(),
            key,
            input_id,
            &target.execution_id,
            &attempt.attempt_id,
            &attempt.turn_operation_id,
            attempt.execution_generation,
        ))
        .map_err(invalid)?;
        let command = format!("robot-background-steer:{}", digest.as_ref());
        let principal = PrincipalRef {
            principal_kind: "user".into(),
            principal_id: owner_id.into(),
        };
        // A receipt lookup belongs to the original native Turn. Replays never
        // rerun selection or acquire a successor Step/generation.
        if self
            .canonical
            .read_native_steer_receipt(
                &principal,
                &child,
                &command,
                &operation,
                &canonical_turn_input(&input),
                &native,
                false,
            )
            .await?
            .is_some()
        {
            self.voice_steer_exact_native_with_policy(
                owner_id, &child, &command, &operation, input, &native, false,
            )
            .await?;
            return Ok(());
        }
        let engine = self
            .execution_progress
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| invalid("Execution owner is unavailable"))?;
        let detail = engine.get(owner_id, &target.execution_id).await?;
        let plan:Option<String>=sqlx::query_scalar("SELECT initial_plan_input FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
            .bind(&target.execution_id).bind(owner_id).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if detail.execution.version != target.expected_version
            || detail.execution.lead_conversation_id.as_deref() != Some(session.as_ref())
            || nomifun_agent_execution::execution_source_from_initial_plan(
                &plan.ok_or_else(|| invalid("captured Execution disappeared"))?,
            )?
            .as_ref()
                != Some(&source)
        {
            return Err(invalid(
                "captured Execution changed; supplement was not retargeted",
            ));
        }
        let exact_step = detail.steps.iter().any(|step| {
            step.step_id == attempt.step_id
                && step.version == attempt.expected_step_version
                && step.status == nomifun_common::ExecutionStepStatus::Running
        });
        let exact_attempt = detail.attempts.iter().any(|current| {
            current.step_id == attempt.step_id
                && current.attempt_id == attempt.attempt_id
                && current.version == attempt.expected_attempt_version
                && current.status == nomifun_common::ExecutionAttemptStatus::Running
                && current.conversation_id.as_deref() == Some(child.as_ref())
        });
        if !exact_step || !exact_attempt {
            return Err(invalid(
                "captured Step or Attempt changed; supplement was not retargeted",
            ));
        }
        let linked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND execution_id=? AND step_id=? AND attempt_id=? AND relation='attempt' AND active=1)")
            .bind(child.as_ref()).bind(&target.execution_id).bind(&attempt.step_id).bind(&attempt.attempt_id)
            .fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if !linked {
            return Err(invalid("captured Attempt lost its exact Session link"));
        }
        let current = self.canonical.get(&principal, &child).await?;
        if current.head.active_turn_id.as_deref() != Some(operation.as_ref())
            || current.session.agent_binding.binding_version != attempt.binding_version
            || self
                .canonical
                .store()
                .native_execution_generation(&child, &operation)
                .await
                .map_err(agent_session_store_error)?
                != attempt.execution_generation
        {
            return Err(invalid("captured native work ended or was replaced"));
        }
        // Both additions and corrections use the established safe boundary.
        // Public output and already-dispatched tools are never withdrawn, and
        // an unknown Effect never becomes permission to retry the operation.
        self.voice_steer_exact_native_with_policy(
            owner_id, &child, &command, &operation, input, &native, false,
        )
        .await?;
        Ok(())
    }
}
