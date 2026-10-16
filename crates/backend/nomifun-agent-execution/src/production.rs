//! Canonical production assembly for [`AgentExecutionEngine`].
//!
//! The application supplies infrastructure services once. Planning, attempt
//! execution, realtime publication and conversation effects are assembled here
//! so no caller can construct a partial lifecycle or depend on those internal
//! strategies directly.

use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_api_types::{AgentExecutionDetail, SendMessageRequest};
use nomifun_common::AppError;
use nomifun_db::{
    IAgentExecutionRepository, IAgentExecutionTemplateRepository, IProviderRepository,
};
use nomifun_realtime::UserEventSink;
use nomifun_model_invoke::ModelInvokeService;

use crate::attempt_runner::{AgentExecutionSessionPort, AgentSessionAttemptRunner};
use crate::engine::{AgentExecutionEngine, AgentExecutionEngineDeps};
use crate::event_publisher::AgentExecutionEventPublisher;
use crate::lifecycle::AgentExecutionLifecycle;
use crate::planner::{LlmPlanProducer, PlanProducer};
use crate::scheduler::ConversationEffects;

/// Infrastructure required by the one supported production engine.
///
/// This is intentionally the only public construction contract. Planner,
/// scheduler and attempt-executor choices are implementation details of the
/// engine crate rather than application-level concepts.
pub struct AgentExecutionEngineConfig {
    pub repository: Arc<dyn IAgentExecutionRepository>,
    pub template_repository: Arc<dyn IAgentExecutionTemplateRepository>,
    pub provider_repository: Arc<dyn IProviderRepository>,
    pub provider_model_repository: Arc<dyn nomifun_db::IProviderModelRepository>,
    pub provider_model_capability_repository:
        Arc<dyn nomifun_db::IProviderModelCapabilityRepository>,
    pub realtime: Arc<dyn UserEventSink>,
    pub session: Arc<dyn AgentExecutionSessionPort>,
    pub model_invoke: Arc<ModelInvokeService>,
    pub workspace_root: PathBuf,
    pub lifecycle: AgentExecutionLifecycle,
}

struct ProductionConversationEffects {
    session: Arc<dyn AgentExecutionSessionPort>,
    repository: Arc<dyn IAgentExecutionRepository>,
}

#[async_trait]
impl ConversationEffects for ProductionConversationEffects {
    async fn report_lead_input(&self,owner_id:&str,detail:&AgentExecutionDetail,
        request:&nomifun_api_types::AgentExecutionEvent)->Result<(),AppError> {
        let row=self.repository.get_execution(owner_id,&detail.execution.execution_id).await?
            .ok_or_else(||AppError::NotFound("directed input Execution is missing".into()))?;
        let Some(source)=crate::engine::directed_delegation_source(&row.initial_plan_input)? else {return Ok(());};
        let input=crate::DirectedDelegationInput {source,execution_id:detail.execution.execution_id.clone(),
            request_event_sequence:request.sequence,
            step_id:request.step_id.clone().ok_or_else(||AppError::Conflict("input request has no Step".into()))?,
            attempt_id:request.attempt_id.clone().ok_or_else(||AppError::Conflict("input request has no Attempt".into()))?};
        self.session.notify_directed_input_required_idempotent(owner_id,&input).await
    }
    async fn cancel_attempt(&self, owner_id: &str, conversation_id: &str) -> Result<(), AppError> {
        self.session
            .cancel_for_execution(owner_id, conversation_id)
            .await
    }
    async fn steer_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
        text: &str,
    ) -> Result<(), AppError> {
        self.session
            .steer_turn_for_execution(
                owner_id,
                conversation_id,
                operation_id,
                target_operation_id,
                SendMessageRequest {
                    content: text.to_owned(),
                    files: vec![],
                    inject_skills: vec![],
                    hidden: false,
                    origin: Some("agent_execution".to_owned()),
                    channel_platform: None,
                },
            )
            .await
            .map(|_| ())
    }
    async fn stop_attempt_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
    ) -> Result<(), AppError> {
        self.session
            .cancel_turn_for_execution(owner_id, conversation_id, operation_id, target_operation_id)
            .await
    }
    async fn report_lead(
        &self,
        owner_id: &str,
        detail: &AgentExecutionDetail,
        operation_id: &str,
    ) -> Result<(), AppError> {
        let Some(conversation_id) = detail.execution.lead_conversation_id.as_deref() else {
            return Ok(());
        };
        let result = detail
            .execution
            .summary
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("执行已结束，但没有生成汇总。");
        let execution_row = self.repository.get_execution(owner_id, &detail.execution.execution_id).await?
            .ok_or_else(|| AppError::NotFound(format!("Agent Execution {}", detail.execution.execution_id)))?;
        if let Some(source) = crate::engine::directed_delegation_source(&execution_row.initial_plan_input)? {
            if source.agent_session_id != conversation_id {
                return Err(AppError::Conflict("directed report source differs from its lead Session".into()));
            }
            let mut turn_keys = HashMap::new();
            let mut cursor = 0;
            loop {
                let events = self.repository.list_events(owner_id, &detail.execution.execution_id, cursor, 500).await?;
                for event in &events {
                    if let Some(attempt) = &event.attempt_id {
                        let payload: serde_json::Value = serde_json::from_str(&event.payload)
                            .map_err(|error| AppError::Internal(format!("invalid execution report source event: {error}")))?;
                        if let Some(key) = payload.get("turn_operation_key").and_then(serde_json::Value::as_str) {
                            turn_keys.insert(attempt.clone(), key.to_owned());
                        }
                    }
                }
                let Some(last) = events.last() else { break; };
                cursor = last.sequence;
                if events.len() < 500 { break; }
            }
            let mut attempts = Vec::new();
            let mut verified_completed_delivery = false;
            for attempt in detail.attempts.iter().filter(|attempt| attempt.status.is_terminal()) {
                let Some(session_id) = attempt.conversation_id.as_deref() else { continue; };
                let key = turn_keys.get(&attempt.attempt_id).cloned()
                    .unwrap_or_else(|| format!("{}:initial-turn", attempt.attempt_id));
                let Some(output) = self.session.read_turn_output(owner_id, session_id, Some(&key)).await? else { continue; };
                let Some(terminal_event_id) = output.terminal_event_id else { continue; };
                if !output.delivery.completed { continue; }
                verified_completed_delivery |= attempt.status == nomifun_common::ExecutionAttemptStatus::Completed
                    && output.delivery.result_ok == Some(true) && output.delivery.user_input_request.is_none();
                attempts.push(crate::DirectedDelegationAttemptReport {
                    step_id: attempt.step_id.clone(), attempt_id: attempt.attempt_id.clone(),
                    agent_session_id: session_id.into(), turn_operation_id: output.canonical_operation_id,
                    terminal_event_id, result_ok: output.delivery.result_ok,
                    output_integrity_ok: output.integrity_ok,
                    output_files: output.output_files,
                });
            }
            if detail.execution.status == nomifun_common::AgentExecutionStatus::Completed
                && !verified_completed_delivery {
                return Err(AppError::Conflict("directed completion has no exact canonical delivery".into()));
            }
            let sequence = operation_id.rsplit(':').next().and_then(|value| value.parse().ok())
                .ok_or_else(|| AppError::Conflict("directed report has no terminal execution event".into()))?;
            let report = crate::DirectedDelegationReport {
                source, execution_id: detail.execution.execution_id.clone(),
                execution_event_sequence: sequence, status: detail.execution.status.to_string(), attempts,
            };
            return self.session.project_directed_execution_report_idempotent(
                owner_id, conversation_id, operation_id, result, &report,
            ).await.map(|_| ());
        }
        // The persisted terminal summary is already the synthesis/sole
        // business output selected by the scheduler. Project it as the final
        // assistant message; never feed it back through the lead model. Do not
        // cancel by Session identity here: the original delegation turn may
        // have ended and a later user turn may now own the active slot.
        self.session
            .project_assistant_message_idempotent(
                owner_id,
                conversation_id,
                operation_id,
                result,
                "agent_execution_report",
            )
            .await
            .map(|_| ())
    }
}

impl AgentExecutionEngine {
    /// Construct the canonical production engine.
    pub fn new(config: AgentExecutionEngineConfig) -> Self {
        let publisher = AgentExecutionEventPublisher::new(config.realtime)
            .with_lifecycle(config.lifecycle.clone());
        let session = config.session;
        let attempt_runner = Arc::new(AgentSessionAttemptRunner::new(session.clone()));
        // The immutable participant snapshot supplies the actual lead model;
        // absence stays typed and fails explicitly in the planner.
        let planner: Arc<dyn PlanProducer> = Arc::new(LlmPlanProducer::new(
            config.provider_model_repository.clone(),
            config.model_invoke,
            config.workspace_root.clone(),
        ));
        let conversation_effects = Arc::new(ProductionConversationEffects {
            session: session.clone(),
            repository: config.repository.clone(),
        });
        let deps = AgentExecutionEngineDeps::new(
            config.repository,
            config.template_repository,
            config.provider_repository,
            config.provider_model_repository,
            config.provider_model_capability_repository,
            planner,
            attempt_runner,
            conversation_effects,
            publisher,
            config.workspace_root,
            config.lifecycle,
            session,
        );
        Self::from_dependencies(deps)
    }
}
