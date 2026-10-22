//! Canonical production assembly for [`AgentExecutionEngine`].
//!
//! The application supplies infrastructure services once. Planning, attempt
//! execution, realtime publication and conversation effects are assembled here
//! so no caller can construct a partial lifecycle or depend on those internal
//! strategies directly.

use std::path::PathBuf;
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
    async fn cancel_attempt(&self, cleanup: &nomifun_db::PendingConversationCleanup) -> Result<(), AppError> {
        let target=match &cleanup.target {
            nomifun_db::ConversationCleanupTurnTarget::Exact {operation_id}=>operation_id,
            nomifun_db::ConversationCleanupTurnTarget::Unconfirmed {reason}=>return Err(AppError::Conflict(reason.clone())),
        };
        self.session.cancel_attempt_for_execution(&cleanup.user_id, &cleanup.conversation_id,
            &cleanup.attempt_id, target).await
    }
    async fn steer_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
        text: &str,
    ) -> Result<crate::AgentExecutionSteerDelivery, AppError> {
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
            // The parent keeps its original Turn and reads these settled domain
            // results through the canonical collaboration port. Publishing them
            // here as its final answer would bypass integration and duplicate
            // the parent's normal Runtime output.
            return Ok(());
        }
        // The persisted terminal summary is already the synthesis/sole
        // business output of a directly created, non-source Execution. Keep
        // that UI summary projection without creating a hidden model Turn or
        // cancelling another Turn that now owns the lead Session's slot.
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
pub(crate) mod tests {
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
    use crate::{AgentExecutionDelivery, AgentExecutionInput,
        AgentExecutionSource, AgentExecutionTurnOutput};

    #[derive(Default)]
    struct RecordingSessionPort {
        reads: Mutex<Vec<(String, String, Option<String>)>>,
        projections: Mutex<Vec<(String, String, String, String, String)>>,
        inputs: Mutex<Vec<(String, AgentExecutionInput)>>,
    }

    pub(crate) fn planning_engine(pool: nomifun_db::SqlitePool, planner: Arc<dyn PlanProducer>,
        lifecycle: AgentExecutionLifecycle, data_dir: PathBuf) -> AgentExecutionEngine {
        struct Sink;
        impl UserEventSink for Sink {
            fn send_to_user(&self, _: &str, _: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
        }
        let repository = Arc::new(SqliteAgentExecutionRepository::new(pool.clone()));
        let session = Arc::new(RecordingSessionPort::default());
        AgentExecutionEngine::from_dependencies(AgentExecutionEngineDeps::new(
            repository.clone(),
            Arc::new(nomifun_db::SqliteAgentExecutionTemplateRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelCapabilityRepository::new(pool)),
            planner, Arc::new(AgentSessionAttemptRunner::new(session.clone())),
            Arc::new(ProductionConversationEffects { session: session.clone(), repository }),
            AgentExecutionEventPublisher::new(Arc::new(Sink)).with_lifecycle(lifecycle.clone()),
            data_dir, lifecycle, session,
        ))
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
            Ok(None)
        }
        async fn get(&self, _: &str, _: &str) -> Result<ConversationResponse, AppError> {
            panic!("report must not read mutable Session configuration")
        }
        fn take_turn_tokens(&self, _: &str) -> Option<i64> { panic!("report must not consume runtime state") }
        async fn cancel_attempt_for_execution(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), AppError> {
            panic!("report must not cancel a successor Turn")
        }
        async fn cancel_turn_for_execution(&self, _: &str, _: &str, _: &str,
            _: &str) -> Result<(), AppError> { panic!("report must not cancel a Turn") }
        async fn steer_turn(&self, _: &str, _: &str, _: &str,
            _: SendMessageRequest) -> Result<String, AppError> { panic!("report must not steer a Turn") }
        async fn steer_turn_for_execution(&self, _: &str, _: &str, _: &str,
            _: &str, _: SendMessageRequest) -> Result<crate::AgentExecutionSteerDelivery, AppError> { panic!("report must not steer a Turn") }
        async fn project_assistant_message_idempotent(&self, owner: &str, session: &str,
            operation: &str, content: &str, origin: &str) -> Result<String, AppError> {
            self.projections.lock().unwrap().push((owner.into(), session.into(), operation.into(), content.into(), origin.into()));
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
        sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,max_parallel,initial_plan_input,summary,created_at,updated_at) VALUES(?,?,'background task','completed','adaptive','ask_user','disabled',1,?,'canonical task result',1,1)")
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
            automation_control:None,
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
        Fixture { database, effects, session, owner, source, detail, request }
    }

    #[tokio::test]
    async fn source_results_stay_in_the_domain_until_the_parent_integrates_them() {
        for mode in ["automatic", "explicit"] {
            let fixture = fixture(mode).await;
            let operation = format!("exec-lead-report:{}:event:11", fixture.detail.execution.execution_id);
            fixture.effects.report_lead(&fixture.owner, &fixture.detail, &operation).await.unwrap();
            assert!(fixture.session.projections.lock().unwrap().is_empty(),
                "a child's settled result cannot become the parent's final answer");
            assert!(fixture.session.reads.lock().unwrap().is_empty(),
                "the collaboration consumer reads exact results only when requested by the parent");
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
            assert!(fixture.session.projections.lock().unwrap().is_empty());
            assert!(fixture.session.reads.lock().unwrap().is_empty());
            fixture.database.close().await;
        }
    }

    #[tokio::test]
    async fn non_source_execution_keeps_its_direct_public_summary() {
        let fixture = fixture("automatic").await;
        sqlx::query("UPDATE agent_executions SET initial_plan_input=? WHERE execution_id=?")
            .bind(json!({"mode":"automatic"}).to_string()).bind(&fixture.detail.execution.execution_id)
            .execute(fixture.database.pool()).await.unwrap();
        let operation = format!("exec-lead-report:{}:event:11", fixture.detail.execution.execution_id);
        fixture.effects.report_lead(&fixture.owner, &fixture.detail, &operation).await.unwrap();
        assert_eq!(*fixture.session.projections.lock().unwrap(), vec![(fixture.owner.clone(),
            fixture.source.agent_session_id.clone(), operation, "canonical task result".into(), "agent_execution_report".into())]);
        assert!(fixture.session.reads.lock().unwrap().is_empty());
        fixture.database.close().await;
    }
}
