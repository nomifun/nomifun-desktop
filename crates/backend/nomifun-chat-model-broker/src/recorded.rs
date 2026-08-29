use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::adapter::ProviderWireFrame;
use crate::contracts::{
    CHAT_MODEL_CONTRACT_VERSION, ChatContentPart, ChatModelEvent, ChatProtocol,
    ChatToolResultPart, PromptCachePolicy, ResolvedChatRoute, ToolCallId,
};

pub const RECORDED_FIXTURE_VERSION: &str = "chat-model-recorded-v1";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedCoverage {
    pub streaming: bool,
    pub tool_round: bool,
    pub reasoning: bool,
    pub prompt_cache: bool,
    pub image_input: bool,
    pub audio_input: bool,
    pub structured_output: bool,
    pub compaction_history: bool,
    pub native_responses_items: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedConformanceFixture {
    pub fixture_version: String,
    pub scenario_id: String,
    pub protocol: ChatProtocol,
    pub route: ResolvedChatRoute,
    pub request: crate::contracts::ChatModelRequest,
    pub coverage: RecordedCoverage,
    pub wire_events: Vec<ProviderWireFrame>,
    pub expected_events: Vec<ChatModelEvent>,
}

impl RecordedConformanceFixture {
    pub fn validate(&self) -> Result<(), RecordedFixtureError> {
        if self.fixture_version != RECORDED_FIXTURE_VERSION {
            return Err(RecordedFixtureError::Version);
        }
        if self.scenario_id.trim().is_empty() {
            return Err(RecordedFixtureError::ScenarioId);
        }
        self.request
            .validate()
            .map_err(|error| RecordedFixtureError::Request(error.to_string()))?;
        self.route
            .validate()
            .map_err(|error| RecordedFixtureError::Route(error.to_string()))?;
        if self.protocol != self.route.protocol {
            return Err(RecordedFixtureError::ProtocolRouteMismatch);
        }
        if self.route.model_route_id != self.request.route.model_route_id
            || self.route.model_route_revision != self.request.route.model_route_revision
        {
            return Err(RecordedFixtureError::RequestRouteMismatch);
        }
        if self.request.contract_version.as_ref() != CHAT_MODEL_CONTRACT_VERSION {
            return Err(RecordedFixtureError::Version);
        }
        if self.wire_events.is_empty() || self.expected_events.is_empty() {
            return Err(RecordedFixtureError::EmptyEvents);
        }
        if self
            .wire_events
            .iter()
            .any(|frame| contains_sensitive_key(&frame.data))
        {
            return Err(RecordedFixtureError::SensitiveWireData);
        }
        validate_expected_events(&self.expected_events)?;
        validate_coverage(self)?;
        Ok(())
    }
}

fn validate_expected_events(events: &[ChatModelEvent]) -> Result<(), RecordedFixtureError> {
    if !events.iter().any(ChatModelEvent::is_semantic_output) {
        return Err(RecordedFixtureError::NoSemanticOutput);
    }
    if !events.last().is_some_and(ChatModelEvent::is_terminal) {
        return Err(RecordedFixtureError::TerminalNotLast);
    }
    if events[..events.len() - 1]
        .iter()
        .any(ChatModelEvent::is_terminal)
    {
        return Err(RecordedFixtureError::DuplicateTerminal);
    }

    let mut tool_names = BTreeMap::<ToolCallId, String>::new();
    let mut usage_count = 0_usize;
    for event in events {
        match event {
            ChatModelEvent::ToolCallDelta {
                call_id, name, ..
            } => {
                if let Some(existing) = tool_names.get(call_id) {
                    if !name.is_empty() && existing != name {
                        return Err(RecordedFixtureError::ToolCorrelation);
                    }
                } else if name.is_empty() {
                    return Err(RecordedFixtureError::ToolCorrelation);
                } else {
                    tool_names.insert(call_id.clone(), name.clone());
                }
            }
            ChatModelEvent::ToolCallCompleted { call } => {
                if let Some(existing) = tool_names.get(&call.call_id)
                    && existing != &call.name
                {
                    return Err(RecordedFixtureError::ToolCorrelation);
                }
                tool_names.insert(call.call_id.clone(), call.name.clone());
            }
            ChatModelEvent::Usage { .. } => {
                usage_count += 1;
                if usage_count > 1 {
                    return Err(RecordedFixtureError::DuplicateUsage);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_coverage(
    fixture: &RecordedConformanceFixture,
) -> Result<(), RecordedFixtureError> {
    let input = &fixture.request.input;
    if fixture.coverage.streaming
        && !fixture.expected_events.iter().any(|event| {
            matches!(
                event,
                ChatModelEvent::OutputTextDelta { .. }
                    | ChatModelEvent::ReasoningDelta { .. }
                    | ChatModelEvent::ToolCallDelta { .. }
            )
        })
    {
        return Err(RecordedFixtureError::Coverage("streaming"));
    }
    if fixture.coverage.tool_round
        && (!request_contains_tool_history(input)
            || !fixture
                .expected_events
                .iter()
                .any(|event| matches!(event, ChatModelEvent::ToolCallCompleted { .. })))
    {
        return Err(RecordedFixtureError::Coverage("tool_round"));
    }
    if fixture.coverage.reasoning
        && !fixture.expected_events.iter().any(|event| {
            matches!(
                event,
                ChatModelEvent::ReasoningDelta { .. }
                    | ChatModelEvent::ReasoningSignature { .. }
            )
        })
    {
        return Err(RecordedFixtureError::Coverage("reasoning"));
    }
    if fixture.coverage.prompt_cache
        && matches!(input.prompt_cache, PromptCachePolicy::Disabled)
    {
        return Err(RecordedFixtureError::Coverage("prompt_cache"));
    }
    if fixture.coverage.image_input && !request_contains_image(input) {
        return Err(RecordedFixtureError::Coverage("image_input"));
    }
    if fixture.coverage.audio_input && !request_contains_audio(input) {
        return Err(RecordedFixtureError::Coverage("audio_input"));
    }
    if fixture.coverage.structured_output
        && matches!(input.response_format, crate::contracts::ChatResponseFormat::Text)
    {
        return Err(RecordedFixtureError::Coverage("structured_output"));
    }
    if fixture.coverage.compaction_history && input.messages.len() < 3 {
        return Err(RecordedFixtureError::Coverage("compaction_history"));
    }
    if fixture.coverage.native_responses_items
        && !fixture.expected_events.iter().any(|event| {
            matches!(event, ChatModelEvent::NativeResponsesItem { .. })
        })
    {
        return Err(RecordedFixtureError::Coverage("native_responses_items"));
    }
    Ok(())
}

fn request_contains_tool_history(input: &crate::contracts::ChatModelInput) -> bool {
    let mut calls = BTreeSet::new();
    let mut results = BTreeSet::new();
    for message in &input.messages {
        for part in &message.content {
            match part {
                ChatContentPart::ToolCall { call_id, .. } => {
                    calls.insert(call_id);
                }
                ChatContentPart::ToolResult { call_id, .. } => {
                    results.insert(call_id);
                }
                _ => {}
            }
        }
    }
    calls.iter().any(|call_id| results.contains(call_id))
}

fn request_contains_image(input: &crate::contracts::ChatModelInput) -> bool {
    input.messages.iter().any(|message| {
        message.content.iter().any(|part| match part {
            ChatContentPart::Image { .. } => true,
            ChatContentPart::ToolResult { output, .. } => output
                .iter()
                .any(|part| matches!(part, ChatToolResultPart::Image { .. })),
            _ => false,
        })
    })
}

fn request_contains_audio(input: &crate::contracts::ChatModelInput) -> bool {
    input.messages.iter().any(|message| {
        message.content.iter().any(|part| match part {
            ChatContentPart::Audio { .. } => true,
            ChatContentPart::ToolResult { output, .. } => output
                .iter()
                .any(|part| matches!(part, ChatToolResultPart::Audio { .. })),
            _ => false,
        })
    })
}

fn contains_sensitive_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "api_key"
                    | "authorization"
                    | "access_token"
                    | "refresh_token"
                    | "client_secret"
                    | "private_key"
                    | "credential"
                    | "credential_material"
            ) || contains_sensitive_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_key),
        _ => false,
    }
}

pub const ANTHROPIC_RECORDED_FIXTURE: &str =
    include_str!("../fixtures/recorded/anthropic.json");
pub const OPENAI_CHAT_RECORDED_FIXTURE: &str =
    include_str!("../fixtures/recorded/openai-chat.json");
pub const OPENAI_RESPONSES_RECORDED_FIXTURE: &str =
    include_str!("../fixtures/recorded/openai-responses.json");
pub const GEMINI_RECORDED_FIXTURE: &str = include_str!("../fixtures/recorded/gemini.json");
pub const BEDROCK_RECORDED_FIXTURE: &str = include_str!("../fixtures/recorded/bedrock.json");
pub const VERTEX_RECORDED_FIXTURE: &str = include_str!("../fixtures/recorded/vertex.json");

pub fn recorded_conformance_fixtures() -> Vec<RecordedConformanceFixture> {
    [
        ANTHROPIC_RECORDED_FIXTURE,
        OPENAI_CHAT_RECORDED_FIXTURE,
        OPENAI_RESPONSES_RECORDED_FIXTURE,
        GEMINI_RECORDED_FIXTURE,
        BEDROCK_RECORDED_FIXTURE,
        VERTEX_RECORDED_FIXTURE,
    ]
    .into_iter()
    .map(|fixture| {
        serde_json::from_str(fixture)
            .expect("recorded model fixture must match RecordedConformanceFixture")
    })
    .collect()
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecordedFixtureError {
    #[error("recorded fixture version is invalid")]
    Version,
    #[error("recorded fixture scenario id is invalid")]
    ScenarioId,
    #[error("recorded fixture request is invalid: {0}")]
    Request(String),
    #[error("recorded fixture route is invalid: {0}")]
    Route(String),
    #[error("recorded fixture protocol and route differ")]
    ProtocolRouteMismatch,
    #[error("recorded fixture request and route differ")]
    RequestRouteMismatch,
    #[error("recorded fixture event lists must be non-empty")]
    EmptyEvents,
    #[error("recorded wire fixture contains credential material")]
    SensitiveWireData,
    #[error("recorded normalized events contain no semantic output")]
    NoSemanticOutput,
    #[error("recorded normalized terminal event is not last")]
    TerminalNotLast,
    #[error("recorded normalized events contain more than one terminal")]
    DuplicateTerminal,
    #[error("recorded normalized tool-call correlation is invalid")]
    ToolCorrelation,
    #[error("recorded normalized usage appears more than once")]
    DuplicateUsage,
    #[error("recorded fixture does not prove declared coverage: {0}")]
    Coverage(&'static str),
}
