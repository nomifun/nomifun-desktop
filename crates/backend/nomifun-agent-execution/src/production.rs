//! Canonical production assembly for [`AgentExecutionEngine`].
//!
//! The application supplies infrastructure services once. Planning, attempt
//! execution, realtime publication and conversation effects are assembled here
//! so no caller can construct a partial lifecycle or depend on those internal
//! strategies directly.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{AgentExecutionDetail, SendMessageRequest};
use nomifun_common::AppError;
use nomifun_conversation::{AgentExecutionConversationPort, ConversationService};
use nomifun_db::{
    IAgentExecutionRepository, IAgentExecutionTemplateRepository, IProviderRepository,
};
use nomifun_preset::PresetService;
use nomifun_realtime::UserEventSink;

use crate::attempt_runner::ConversationAttemptRunner;
use crate::engine::{AgentExecutionEngine, AgentExecutionEngineDeps};
use crate::event_publisher::AgentExecutionEventPublisher;
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
    pub preset_service: Arc<PresetService>,
    pub realtime: Arc<dyn UserEventSink>,
    pub conversation: ConversationService,
    pub runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    pub encryption_key: [u8; 32],
    pub workspace_root: PathBuf,
}

struct ProductionConversationEffects {
    conversation: ConversationService,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
    execution_port: AgentExecutionConversationPort,
}

#[async_trait]
impl ConversationEffects for ProductionConversationEffects {
    async fn cancel_attempt(&self, owner_id: &str, conversation_id: &str) -> Result<(), AppError> {
        self.conversation
            .cancel_for_execution(owner_id, conversation_id, &self.runtime_registry)
            .await
    }
    async fn steer_attempt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        text: &str,
    ) -> Result<(), AppError> {
        self.execution_port
            .steer_turn(
                owner_id,
                conversation_id,
                operation_id,
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
        _operation_id: &str,
    ) -> Result<(), AppError> {
        self.conversation
            .cancel_for_execution(owner_id, conversation_id, &self.runtime_registry)
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
        // The persisted terminal summary is already the synthesis/sole
        // business output selected by the scheduler. Project it as the final
        // assistant message; never feed it back through the lead model.
        self.conversation
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
        let publisher = AgentExecutionEventPublisher::new(config.realtime);
        let attempt_runner = Arc::new(ConversationAttemptRunner::new(
            config.conversation.clone(),
            config.runtime_registry.clone(),
        ));
        // The immutable participant snapshot supplies the actual lead model;
        // absence stays typed and fails explicitly in the planner.
        let planner: Arc<dyn PlanProducer> = Arc::new(LlmPlanProducer::new(
            config.provider_repository.clone(),
            config.encryption_key,
            config.workspace_root.clone(),
            None,
        ));
        let execution_port = config
            .conversation
            .agent_execution_port(config.runtime_registry.clone());
        let conversation_effects = Arc::new(ProductionConversationEffects {
            conversation: config.conversation,
            runtime_registry: config.runtime_registry,
            execution_port,
        });
        let deps = AgentExecutionEngineDeps::new(
            config.repository,
            config.template_repository,
            config.provider_repository,
            config.preset_service,
            planner,
            attempt_runner,
            conversation_effects,
            publisher,
            config.workspace_root,
        );
        Self::from_dependencies(deps)
    }
}
