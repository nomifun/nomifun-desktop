//! NomiFun-owned compaction orchestration and metadata.

use std::sync::Arc;

use futures::StreamExt;
use nomifun_chat_model_broker::{
    ChatFinishReason, ChatModelEvent, ChatModelRequest, ChatResponseFormat, ChatToolChoice,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::engine::EngineBinding;
use crate::error::CodingEngineError;
use crate::model::CodingModelPort;

const DEFAULT_MAX_COMPACTION_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct CodingCompactionRequest {
    pub model_request: ChatModelRequest,
    pub source_event_cursor: u64,
    pub workspace_summary: String,
    pub completed_tools: Vec<String>,
    pub outstanding_work: String,
    pub retained_facts: Vec<String>,
    pub max_summary_bytes: usize,
}

impl CodingCompactionRequest {
    pub fn new(
        model_request: ChatModelRequest,
        source_event_cursor: u64,
        workspace_summary: impl Into<String>,
        completed_tools: Vec<String>,
        outstanding_work: impl Into<String>,
        retained_facts: Vec<String>,
    ) -> Self {
        Self {
            model_request,
            source_event_cursor,
            workspace_summary: workspace_summary.into(),
            completed_tools,
            outstanding_work: outstanding_work.into(),
            retained_facts,
            max_summary_bytes: DEFAULT_MAX_COMPACTION_BYTES,
        }
    }

    pub fn with_max_summary_bytes(mut self, max_summary_bytes: usize) -> Self {
        self.max_summary_bytes = max_summary_bytes;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingCompactionSummary {
    pub source_event_cursor: u64,
    pub agent_session_id: String,
    pub runtime_binding_id: String,
    pub engine_family_id: String,
    pub engine_build_id: String,
    pub engine_build_digest: String,
    pub snapshot_id: String,
    pub snapshot_digest: String,
    pub task_summary: String,
    pub workspace_summary: String,
    pub completed_tools: Vec<String>,
    pub outstanding_work: String,
    pub retained_facts: Vec<String>,
}

impl CodingCompactionSummary {
    pub fn new(
        binding: &EngineBinding,
        source_event_cursor: u64,
        task_summary: impl Into<String>,
        workspace_summary: impl Into<String>,
        completed_tools: Vec<String>,
        outstanding_work: impl Into<String>,
        retained_facts: Vec<String>,
    ) -> Result<Self, CodingEngineError> {
        let summary = Self {
            source_event_cursor,
            agent_session_id: binding.agent_session_id().as_ref().to_owned(),
            runtime_binding_id: binding.runtime_binding_id().as_ref().to_owned(),
            engine_family_id: binding.family_id().as_ref().to_owned(),
            engine_build_id: binding.build_id().as_ref().to_owned(),
            engine_build_digest: binding.build_digest().as_ref().to_owned(),
            snapshot_id: binding
                .resolved_snapshot_ref()
                .snapshot_id
                .as_ref()
                .to_owned(),
            snapshot_digest: binding
                .resolved_snapshot_ref()
                .snapshot_digest
                .as_ref()
                .to_owned(),
            task_summary: task_summary.into(),
            workspace_summary: workspace_summary.into(),
            completed_tools,
            outstanding_work: outstanding_work.into(),
            retained_facts,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if self.agent_session_id.trim().is_empty()
            || self.runtime_binding_id.trim().is_empty()
            || self.engine_family_id.trim().is_empty()
            || self.engine_build_id.trim().is_empty()
            || self.engine_build_digest.len() != 64
            || self.snapshot_id.trim().is_empty()
            || self.snapshot_digest.len() != 64
            || self.task_summary.trim().is_empty()
            || self.outstanding_work.trim().is_empty()
        {
            return Err(CodingEngineError::Compaction(
                "compaction summary must retain identity, task and outstanding work".to_owned(),
            ));
        }
        Ok(())
    }
}

pub async fn run_compaction(
    binding: &EngineBinding,
    model: Arc<dyn CodingModelPort>,
    mut request: CodingCompactionRequest,
    cancellation: CancellationToken,
) -> Result<CodingCompactionSummary, CodingEngineError> {
    if request.max_summary_bytes == 0
        || request.max_summary_bytes > 4 * 1024 * 1024
        || request.outstanding_work.trim().is_empty()
    {
        return Err(CodingEngineError::Compaction(
            "compaction limits and outstanding work must be explicit".to_owned(),
        ));
    }
    if &request.model_request.causality.agent_session_id != binding.agent_session_id()
        || &request.model_request.causality.resolved_snapshot_ref
            != binding.resolved_snapshot_ref()
    {
        return Err(CodingEngineError::TurnBindingMismatch {
            field: "compaction_identity",
        });
    }
    request.model_request.input.tools.clear();
    request.model_request.input.tool_choice = ChatToolChoice::None;
    request.model_request.input.reasoning = None;
    request.model_request.input.response_format = ChatResponseFormat::Text;
    request
        .model_request
        .input
        .metadata
        .insert("nomifun_task".to_owned(), "agent_compaction".to_owned());
    request.model_request.input.preserve_native_responses_items = false;
    request
        .model_request
        .input
        .requested_output_modalities
        .clear();
    request
        .model_request
        .validate()
        .map_err(|error| CodingEngineError::Compaction(error.to_string()))?;

    if cancellation.is_cancelled() {
        return Err(CodingEngineError::Cancelled);
    }
    let open_stream = model.open_stream(request.model_request, cancellation.clone());
    let mut stream = tokio::select! {
        _ = cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
        result = open_stream => result.map_err(CodingEngineError::from_model_error)?,
    };
    let mut summary = String::new();
    let mut terminal = None;
    while terminal.is_none() {
        let item = tokio::select! {
            _ = cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
            item = stream.next() => item,
        };
        let Some(item) = item else {
            return Err(CodingEngineError::ModelStreamEndedWithoutTerminal);
        };
        match item.map_err(CodingEngineError::from_model_error)? {
            ChatModelEvent::OutputTextDelta { text } => {
                if text.is_empty() {
                    return Err(CodingEngineError::Compaction(
                        "compaction emitted an empty text delta".to_owned(),
                    ));
                }
                summary.push_str(&text);
                if summary.len() > request.max_summary_bytes {
                    return Err(CodingEngineError::ContextTooLarge {
                        limit: request.max_summary_bytes,
                        actual: summary.len(),
                    });
                }
            }
            ChatModelEvent::Completed { finish_reason } => terminal = Some(finish_reason),
            ChatModelEvent::ResponseStarted { .. }
            | ChatModelEvent::ReasoningDelta { .. }
            | ChatModelEvent::ReasoningSignature { .. }
            | ChatModelEvent::ProviderRoundId { .. }
            | ChatModelEvent::Usage { .. } => {}
            ChatModelEvent::ToolCallDelta { .. } | ChatModelEvent::ToolCallCompleted { .. } => {
                return Err(CodingEngineError::Compaction(
                    "compaction route attempted a Tool Call".to_owned(),
                ));
            }
            ChatModelEvent::NativeResponsesItem { .. }
            | ChatModelEvent::OutputAudioDelta { .. } => {
                return Err(CodingEngineError::Compaction(
                    "compaction route emitted an unsupported modality".to_owned(),
                ));
            }
        }
    }
    match terminal.expect("loop exits only after terminal") {
        ChatFinishReason::Completed if !summary.trim().is_empty() => {}
        ChatFinishReason::Cancelled => return Err(CodingEngineError::Cancelled),
        finish_reason => {
            return Err(CodingEngineError::Compaction(format!(
                "compaction ended with {finish_reason:?}"
            )));
        }
    }

    CodingCompactionSummary::new(
        binding,
        request.source_event_cursor,
        summary,
        request.workspace_summary,
        request.completed_tools,
        request.outstanding_work,
        request.retained_facts,
    )
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::stream;
    use super::*;
    use crate::engine::{
        CodingEngine, CodingEngineBuild, CodingRuntimeProfile, EngineBuildId, EngineFamilyId,
    };
    use nomifun_agent_contracts::{
        AgentSessionId, ChatRouteIdentity, DigestHex, EventId, ModelRouteId, OperationId,
        ResolvedSnapshotId, ResolvedSnapshotRef, RuntimeBindingId, VersionString,
    };
    use nomifun_chat_model_broker::{
        ChatCausality, ChatContentPart, ChatModelError, ChatModelEvent, ChatModelInput,
        ChatResponseFormat, ChatRole, PromptCachePolicy,
    };
    use std::collections::BTreeSet;

    fn binding() -> EngineBinding {
        CodingEngine::new(CodingEngineBuild {
            family_id: EngineFamilyId::from("nomifun.coding"),
            build_id: EngineBuildId::from("build-1"),
            build_digest: DigestHex::from("a".repeat(64)),
            display_name: "Coding".to_owned(),
            supported_profiles: vec![CodingRuntimeProfile::Coding],
        })
        .unwrap()
        .bind(
            AgentSessionId::from("session"),
            RuntimeBindingId::from("binding"),
            CodingRuntimeProfile::Coding,
            ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
        )
        .unwrap()
    }

    fn model_request(binding: &EngineBinding) -> ChatModelRequest {
        let route = ChatRouteIdentity::new(
            "preset@1",
            "agent_chat",
            ModelRouteId::from("route"),
            1,
        );
        ChatModelRequest {
            contract_version: VersionString::from(
                nomifun_chat_model_broker::CHAT_MODEL_CONTRACT_VERSION,
            ),
            causality: ChatCausality {
                agent_session_id: binding.agent_session_id().clone(),
                turn_operation_id: OperationId::from("compact-turn"),
                causation_event_id: EventId::from("compact-input"),
                resolved_snapshot_ref: binding.resolved_snapshot_ref().clone(),
                route_identity: route.clone(),
                operation_id: OperationId::from("compact-model"),
            },
            route,
            input: ChatModelInput {
                instructions: vec!["Summarize the Coding session.".to_owned()],
                messages: vec![nomifun_chat_model_broker::ChatMessage {
                    role: ChatRole::User,
                    content: vec![ChatContentPart::Text {
                        text: "history".to_owned(),
                    }],
                    provider_round_id: None,
                }],
                tools: Vec::new(),
                tool_choice: ChatToolChoice::None,
                max_output_tokens: Some(100),
                reasoning: None,
                prompt_cache: PromptCachePolicy::Disabled,
                response_format: ChatResponseFormat::Text,
                requested_output_modalities: BTreeSet::new(),
                provider_round_parent: None,
                preserve_native_responses_items: false,
                metadata: Default::default(),
            },
        }
    }

    struct CompactionModel;

    #[async_trait]
    impl CodingModelPort for CompactionModel {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<crate::model::CodingModelStream, ChatModelError> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "Keep exact bindings.".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ])))
        }
    }

    #[test]
    fn summary_retains_exact_runtime_identity() {
        let binding = binding();
        let summary = CodingCompactionSummary::new(
            &binding,
            9,
            "Fix compiler",
            "workspace is clean",
            vec!["read_file".to_owned()],
            "run tests",
            vec!["tool schema is frozen".to_owned()],
        )
        .unwrap();
        assert_eq!(summary.source_event_cursor, 9);
        assert_eq!(summary.engine_build_digest, binding.build_digest().as_ref());
        assert!(summary.validate().is_ok());
    }

    #[tokio::test]
    async fn compaction_uses_the_model_port_without_tools_or_private_history() {
        let binding = binding();
        let summary = run_compaction(
            &binding,
            Arc::new(CompactionModel),
            CodingCompactionRequest::new(
                model_request(&binding),
                12,
                "workspace changed",
                vec!["read_file".to_owned()],
                "run tests",
                vec!["EngineBinding is immutable".to_owned()],
            ),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(summary.task_summary, "Keep exact bindings.");
        assert_eq!(summary.source_event_cursor, 12);
        assert_eq!(
            summary.runtime_binding_id,
            binding.runtime_binding_id().as_ref()
        );
    }
}
