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
            .ok_or_else(||AppError::NotFound("source input Execution is missing".into()))?;
        let Some(source)=crate::engine::execution_source_from_initial_plan(&row.initial_plan_input)? else {return Ok(());};
        let input=crate::AgentExecutionInput {source,execution_id:detail.execution.execution_id.clone(),
            request_event_sequence:request.sequence,
            step_id:request.step_id.clone().ok_or_else(||AppError::Conflict("input request has no Step".into()))?,
            attempt_id:request.attempt_id.clone().ok_or_else(||AppError::Conflict("input request has no Attempt".into()))?};
        self.session.notify_execution_input_required_idempotent(owner_id,&input).await
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
        if let Some(source) = crate::engine::execution_source_from_initial_plan(&execution_row.initial_plan_input)? {
            if source.agent_session_id != conversation_id {
                return Err(AppError::Conflict("execution report source differs from its lead Session".into()));
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
                attempts.push(crate::AgentExecutionAttemptReport {
                    step_id: attempt.step_id.clone(), attempt_id: attempt.attempt_id.clone(),
                    agent_session_id: session_id.into(), turn_operation_id: output.canonical_operation_id,
                    terminal_event_id, result_ok: output.delivery.result_ok,
                    output_integrity_ok: output.integrity_ok,
                    output_files: output.output_files,
                });
            }
            if detail.execution.status == nomifun_common::AgentExecutionStatus::Completed
                && !verified_completed_delivery {
                return Err(AppError::Conflict("execution completion has no exact canonical delivery".into()));
            }
            let sequence = operation_id.rsplit(':').next().and_then(|value| value.parse().ok())
                .ok_or_else(|| AppError::Conflict("execution report has no terminal execution event".into()))?;
            let report = crate::AgentExecutionReport {
                source, execution_id: detail.execution.execution_id.clone(),
                execution_event_sequence: sequence, status: detail.execution.status.to_string(), attempts,
            };
            return self.session.project_execution_report_idempotent(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use nomifun_api_types::{
        AgentExecutionEvent, AgentResolvedSnapshot, ConversationResponse,
        CreateConversationRequest, ExecutionAttempt,
    };
    use nomifun_common::{
        AgentExecutionActorType, AgentExecutionEventKind, AgentExecutionStatus,
        ExecutionAttemptStatus, generate_id,
    };
    use nomifun_db::{AgentExecutionTurnAuthority, SqliteAgentExecutionRepository};
    use serde_json::json;
    use crate::{AgentExecutionDelivery, AgentExecutionInput, AgentExecutionReport,
        AgentExecutionSource, AgentExecutionTurnOutput};

    #[derive(Default)]
    struct RecordingSessionPort {
        output: Mutex<Option<AgentExecutionTurnOutput>>,
        reads: Mutex<Vec<(String, String, Option<String>)>>,
        reports: Mutex<Vec<(String, String, String, String, AgentExecutionReport)>>,
        inputs: Mutex<Vec<(String, AgentExecutionInput)>>,
    }

    #[async_trait]
    impl AgentExecutionSessionPort for RecordingSessionPort {
        async fn create_idempotent(&self, _: &str, _: CreateConversationRequest,
            _: &str) -> Result<ConversationResponse, AppError> { panic!("report must not create a Session") }
        async fn create_from_agent_snapshot_idempotent(&self, _: &str,
            _: CreateConversationRequest, _: AgentResolvedSnapshot,
            _: &str) -> Result<ConversationResponse, AppError> { panic!("report must not create a Session") }
        async fn discard_unlinked_creation(&self, _: &str, _: &str) -> Result<(), AppError> {
            panic!("report must not discard a Session")
        }
        async fn deliver_turn(&self, _: &str, _: &str, _: &str,
            _: AgentExecutionTurnAuthority, _: SendMessageRequest) -> Result<AgentExecutionDelivery, AppError> {
            panic!("report must not invoke a model Turn")
        }
        async fn delivery_result(&self, _: &str, _: &str,
            _: &str) -> Result<Option<AgentExecutionDelivery>, AppError> { panic!("report must use exact canonical output") }
        async fn read_turn_output(&self, owner: &str, session: &str,
            operation: Option<&str>) -> Result<Option<AgentExecutionTurnOutput>, AppError> {
            self.reads.lock().unwrap().push((owner.into(), session.into(), operation.map(str::to_owned)));
            Ok(self.output.lock().unwrap().clone())
        }
        async fn get(&self, _: &str, _: &str) -> Result<ConversationResponse, AppError> {
            panic!("report must not read mutable Session configuration")
        }
        fn take_turn_tokens(&self, _: &str) -> Option<i64> { panic!("report must not consume runtime state") }
        async fn cancel_for_execution(&self, _: &str, _: &str) -> Result<(), AppError> {
            panic!("report must not cancel a successor Turn")
        }
        async fn cancel_turn_for_execution(&self, _: &str, _: &str, _: &str,
            _: &str) -> Result<(), AppError> { panic!("report must not cancel a Turn") }
        async fn steer_turn(&self, _: &str, _: &str, _: &str,
            _: SendMessageRequest) -> Result<String, AppError> { panic!("report must not steer a Turn") }
        async fn steer_turn_for_execution(&self, _: &str, _: &str, _: &str,
            _: &str, _: SendMessageRequest) -> Result<String, AppError> { panic!("report must not steer a Turn") }
        async fn project_assistant_message_idempotent(&self, _: &str, _: &str,
            _: &str, _: &str, _: &str) -> Result<String, AppError> {
            panic!("source report must retain typed canonical provenance")
        }
        async fn project_execution_report_idempotent(&self, owner: &str, session: &str,
            operation: &str, content: &str, report: &AgentExecutionReport) -> Result<String, AppError> {
            self.reports.lock().unwrap().push((owner.into(), session.into(), operation.into(), content.into(), report.clone()));
            Ok(generate_id())
        }
        async fn notify_execution_input_required_idempotent(&self, owner: &str,
            input: &AgentExecutionInput) -> Result<(), AppError> {
            self.inputs.lock().unwrap().push((owner.into(), input.clone()));
            Ok(())
        }
    }

    struct Fixture {
        database: nomifun_db::Database,
        effects: ProductionConversationEffects,
        session: Arc<RecordingSessionPort>,
        owner: String,
        source: AgentExecutionSource,
        detail: AgentExecutionDetail,
        request: AgentExecutionEvent,
        public_turn_key: String,
    }

    async fn fixture(mode: &str) -> Fixture {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let execution = generate_id();
        let step = generate_id();
        let attempt = generate_id();
        let child_session = generate_id();
        let source = AgentExecutionSource {
            agent_session_id: generate_id(), turn_operation_id: "source-turn".into(),
            tool_operation_id: "source-tool".into(), effect_id: "source-effect".into(),
        };
        let initial_plan = match mode {
            "automatic" => json!({"mode":mode,"source":source}),
            "explicit" => json!({"mode":mode,"source":source,"plan":{"steps":[]}}),
            _ => panic!("fixture must exercise generic planning modes"),
        };
        sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,summary,created_at,updated_at) VALUES(?,?,'background task','completed','automatic','adaptive','ask_user','disabled',1,?,'canonical task result',1,1)")
            .bind(&execution).bind(&owner).bind(initial_plan.to_string())
            .execute(database.pool()).await.unwrap();
        let public_turn_key = format!("{attempt}:decision-continuation");
        sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,step_id,attempt_id,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,7,'attempt_changed',?,?,'system',?,?,1)")
            .bind(&execution).bind(&step).bind(&attempt).bind(&owner)
            .bind(json!({"turn_operation_key":public_turn_key}).to_string())
            .execute(database.pool()).await.unwrap();
        let repository = Arc::new(SqliteAgentExecutionRepository::new(database.pool().clone()));
        let row = repository.get_execution(&owner, &execution).await.unwrap().unwrap();
        let detail = AgentExecutionDetail {
            execution: crate::domain_mapper::execution(row, Some(source.agent_session_id.clone())).unwrap(),
            participants: vec![], steps: vec![], dependencies: vec![],
            attempts: vec![ExecutionAttempt {
                attempt_id: attempt.clone(), execution_id: execution.clone(), step_id: step.clone(),
                attempt_no: 1, participant_id: None, conversation_id: Some(child_session),
                status: ExecutionAttemptStatus::Completed, trigger_reason: "initial".into(),
                effective_config: json!({}), question: None, error: None,
                output_summary: Some("canonical task result".into()), output_files: vec![],
                tokens: None, retry_after: None, runtime_state: None,
                started_at: Some(1), finished_at: Some(2), version: 1, created_at: 1, updated_at: 2,
            }],
        };
        let request = AgentExecutionEvent {
            execution_id: execution, sequence: 9,
            event_type: AgentExecutionEventKind::DecisionRequested,
            step_id: Some(step), attempt_id: Some(attempt),
            actor_type: AgentExecutionActorType::System, actor_id: None,
            actor_conversation_id: None, actor_attempt_id: None,
            on_behalf_of_user_id: owner.clone(), payload: json!({"question":"Which file?"}), created_at: 2,
        };
        let session = Arc::new(RecordingSessionPort::default());
        let effects = ProductionConversationEffects { session: session.clone(), repository };
        Fixture { database, effects, session, owner, source, detail, request, public_turn_key }
    }

    fn verified_output() -> AgentExecutionTurnOutput {
        AgentExecutionTurnOutput {
            canonical_operation_id: "canonical-child-turn".into(),
            terminal_event_id: Some("canonical-child-terminal".into()),
            output_files: vec!["report.txt".into()], integrity_ok: true,
            delivery: AgentExecutionDelivery {
                message_id: generate_id(), replayed: false, completed: true,
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {
                    message_ids: vec!["canonical-child-message".into()],
                }),
                user_input_request: None, result_ok: Some(true), result_text: Some("canonical task result".into()),
                result_error: None, result_error_code: None, result_error_retryable: None,
            },
        }
    }

    #[tokio::test]
    async fn generic_source_reports_use_exact_canonical_attempts_without_another_model_turn() {
        for mode in ["automatic", "explicit"] {
            let fixture = fixture(mode).await;
            *fixture.session.output.lock().unwrap() = Some(verified_output());
            let execution = &fixture.detail.execution.execution_id;
            let operation = format!("exec-lead-report:{execution}:event:11");
            fixture.effects.report_lead(&fixture.owner, &fixture.detail, &operation).await.unwrap();
            assert_eq!(*fixture.session.reads.lock().unwrap(), vec![(
                fixture.owner.clone(), fixture.detail.attempts[0].conversation_id.clone().unwrap(),
                Some(fixture.public_turn_key.clone()),
            )]);
            let reports = fixture.session.reports.lock().unwrap();
            assert_eq!(reports.len(), 1);
            let (owner, session, key, content, report) = &reports[0];
            assert_eq!(owner, &fixture.owner);
            assert_eq!(session, &fixture.source.agent_session_id);
            assert_eq!(key, &operation);
            assert_eq!(content, "canonical task result");
            assert_eq!(report.source, fixture.source);
            assert_eq!(&report.execution_id, execution);
            assert_eq!(report.execution_event_sequence, 11);
            assert_eq!(report.status, "completed");
            assert_eq!(report.attempts.len(), 1);
            let attempt = &report.attempts[0];
            assert_eq!(attempt.step_id, fixture.detail.attempts[0].step_id);
            assert_eq!(attempt.attempt_id, fixture.detail.attempts[0].attempt_id);
            assert_eq!(attempt.agent_session_id, fixture.detail.attempts[0].conversation_id.clone().unwrap());
            assert_eq!(attempt.turn_operation_id, "canonical-child-turn");
            assert_eq!(attempt.terminal_event_id, "canonical-child-terminal");
            assert_eq!(attempt.result_ok, Some(true));
            assert!(attempt.output_integrity_ok);
            assert_eq!(attempt.output_files, ["report.txt"]);
            drop(reports);
            fixture.database.close().await;
        }
    }

    #[tokio::test]
    async fn generic_source_waiting_input_notifies_the_original_request_with_exact_decision_identity() {
        for mode in ["automatic", "explicit"] {
            let mut fixture = fixture(mode).await;
            fixture.detail.execution.status = AgentExecutionStatus::WaitingInput;
            fixture.detail.attempts[0].status = ExecutionAttemptStatus::WaitingInput;
            fixture.effects.report_lead_input(&fixture.owner, &fixture.detail, &fixture.request).await.unwrap();
            assert_eq!(*fixture.session.inputs.lock().unwrap(), vec![(fixture.owner.clone(), AgentExecutionInput {
                source: fixture.source, execution_id: fixture.detail.execution.execution_id.clone(),
                request_event_sequence: fixture.request.sequence,
                step_id: fixture.request.step_id.clone().unwrap(),
                attempt_id: fixture.request.attempt_id.clone().unwrap(),
            })]);
            assert!(fixture.session.reports.lock().unwrap().is_empty());
            assert!(fixture.session.reads.lock().unwrap().is_empty());
            fixture.database.close().await;
        }
    }

    #[tokio::test]
    async fn generic_source_completion_cannot_project_success_without_exact_canonical_delivery() {
        for mode in ["automatic", "explicit"] {
            let fixture = fixture(mode).await;
            let operation = format!("exec-lead-report:{}:event:11", fixture.detail.execution.execution_id);
            let error = fixture.effects.report_lead(&fixture.owner, &fixture.detail, &operation).await.unwrap_err();
            assert!(matches!(error, AppError::Conflict(_)));
            assert!(fixture.session.reports.lock().unwrap().is_empty());
            fixture.database.close().await;
        }
    }
}
