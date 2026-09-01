use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    AgentSessionId, ChatRouteIdentity, ConnectionConfigRef, DigestHex, EventId, ModelRouteId,
    OperationId, ResolvedSnapshotRef, StrictJsonValue, VersionString,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CHAT_MODEL_CONTRACT_VERSION: &str = "chat-model-v1";

macro_rules! string_ref {
    ($name:ident) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_ref!(ProviderIdRef);
string_ref!(ProviderCredentialRef);
string_ref!(ProviderResponseId);
string_ref!(ProviderRoundId);
string_ref!(ToolCallId);

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatProtocol {
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
    Gemini,
    Bedrock,
    Vertex,
}

impl ChatProtocol {
    pub const ALL: [Self; 6] = [
        Self::Anthropic,
        Self::OpenaiChat,
        Self::OpenaiResponses,
        Self::Gemini,
        Self::Bedrock,
        Self::Vertex,
    ];
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatModelFeature {
    TextInput,
    ImageInput,
    AudioInput,
    TextOutput,
    AudioOutput,
    ToolCalls,
    Reasoning,
    ReasoningSignature,
    PromptCache,
    StructuredOutput,
    ProviderRoundState,
    NativeResponsesItems,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatTask {
    AgentChat,
}

impl ChatTask {
    pub const fn model_task(self) -> &'static str {
        match self {
            Self::AgentChat => nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCausality {
    pub agent_session_id: AgentSessionId,
    pub turn_operation_id: OperationId,
    pub causation_event_id: EventId,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub route_identity: ChatRouteIdentity,
    pub operation_id: OperationId,
}

/// The request-facing route selection is the canonical immutable identity
/// itself. There is intentionally no second selection struct with a partial
/// `(route_id, route_revision)` view.
pub type ChatRouteSelection = ChatRouteIdentity;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedChatRoute {
    pub model_route_id: ModelRouteId,
    pub model_route_revision: u64,
    pub provider_id: ProviderIdRef,
    pub model: String,
    pub protocol: ChatProtocol,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
    pub credential_ref: ProviderCredentialRef,
    pub features: BTreeSet<ChatModelFeature>,
}

impl ResolvedChatRoute {
    pub fn validate(&self) -> Result<(), ChatContractError> {
        validate_natural_key("model_route_id", self.model_route_id.as_ref())?;
        validate_natural_key("provider_id", self.provider_id.as_ref())?;
        validate_natural_key("model", &self.model)?;
        validate_natural_key(
            "connection_config_ref",
            self.connection_config_ref.as_ref(),
        )?;
        validate_digest("config_revision_digest", &self.config_revision_digest)?;
        validate_natural_key("credential_ref", self.credential_ref.as_ref())?;
        if self.model_route_revision == 0 {
            return Err(ChatContractError::ZeroRouteRevision);
        }
        if !self.features.contains(&ChatModelFeature::TextOutput) {
            return Err(ChatContractError::RouteMissingTextOutput);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedChatRouteSet {
    pub primary: ResolvedChatRoute,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failovers: Vec<ResolvedChatRoute>,
}

impl ResolvedChatRouteSet {
    pub fn validate_for(
        &self,
        selection: &ChatRouteSelection,
    ) -> Result<(), ChatContractError> {
        self.primary.validate()?;
        selection
            .validate()
            .map_err(|error| ChatContractError::InvalidRouteIdentity(error.to_string()))?;
        if self.primary.model_route_id != selection.route_id {
            return Err(ChatContractError::PrimaryRouteMismatch);
        }
        if self.primary.model_route_revision != selection.route_revision {
            return Err(ChatContractError::PrimaryRouteRevisionMismatch);
        }

        let mut route_keys = BTreeSet::new();
        for route in std::iter::once(&self.primary).chain(self.failovers.iter()) {
            route.validate()?;
            if !route_keys.insert((
                route.model_route_id.as_ref().to_owned(),
                route.model_route_revision,
            )) {
                return Err(ChatContractError::DuplicateRouteCandidate);
            }
        }
        Ok(())
    }

    pub fn candidates(&self) -> impl Iterator<Item = &ResolvedChatRoute> {
        std::iter::once(&self.primary).chain(self.failovers.iter())
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatModality {
    Text,
    Image,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatToolResultPart {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data_base64: String,
    },
    Audio {
        media_type: String,
        data_base64: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatContentPart {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data_base64: String,
    },
    Audio {
        media_type: String,
        data_base64: String,
    },
    ToolCall {
        call_id: ToolCallId,
        name: String,
        arguments: StrictJsonValue,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<StrictJsonValue>,
    },
    ToolResult {
        call_id: ToolCallId,
        output: Vec<ChatToolResultPart>,
        is_error: bool,
    },
    Reasoning {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Vec<ChatContentPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_round_id: Option<ProviderRoundId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: StrictJsonValue,
    #[serde(default)]
    pub deferred: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatToolChoice {
    Auto,
    None,
    Required,
    Specific { name: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSummary {
    None,
    Auto,
    Concise,
    Detailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatReasoningRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    pub summary: ReasoningSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_reasoning_tokens: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCachePolicy {
    Disabled,
    Automatic,
    Ephemeral,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: StrictJsonValue,
        strict: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatModelInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ChatToolDefinition>,
    pub tool_choice: ChatToolChoice,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ChatReasoningRequest>,
    pub prompt_cache: PromptCachePolicy,
    pub response_format: ChatResponseFormat,
    #[serde(default)]
    pub requested_output_modalities: BTreeSet<ChatModality>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_round_parent: Option<ProviderRoundId>,
    #[serde(default)]
    pub preserve_native_responses_items: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl ChatModelInput {
    pub fn required_features(&self) -> BTreeSet<ChatModelFeature> {
        let mut required =
            BTreeSet::from([ChatModelFeature::TextInput, ChatModelFeature::TextOutput]);

        for message in &self.messages {
            for part in &message.content {
                match part {
                    ChatContentPart::Image { .. } => {
                        required.insert(ChatModelFeature::ImageInput);
                    }
                    ChatContentPart::Audio { .. } => {
                        required.insert(ChatModelFeature::AudioInput);
                    }
                    ChatContentPart::ToolCall { .. } => {
                        required.insert(ChatModelFeature::ToolCalls);
                    }
                    ChatContentPart::ToolResult { output, .. } => {
                        required.insert(ChatModelFeature::ToolCalls);
                        if output
                            .iter()
                            .any(|part| matches!(part, ChatToolResultPart::Image { .. }))
                        {
                            required.insert(ChatModelFeature::ImageInput);
                        }
                        if output
                            .iter()
                            .any(|part| matches!(part, ChatToolResultPart::Audio { .. }))
                        {
                            required.insert(ChatModelFeature::AudioInput);
                        }
                    }
                    ChatContentPart::Reasoning { signature, .. } => {
                        required.insert(ChatModelFeature::Reasoning);
                        if signature.is_some() {
                            required.insert(ChatModelFeature::ReasoningSignature);
                        }
                    }
                    ChatContentPart::Text { .. } => {}
                }
            }
        }

        if !self.tools.is_empty() || !matches!(self.tool_choice, ChatToolChoice::None) {
            required.insert(ChatModelFeature::ToolCalls);
        }
        if self.reasoning.is_some() {
            required.insert(ChatModelFeature::Reasoning);
        }
        if !matches!(self.prompt_cache, PromptCachePolicy::Disabled) {
            required.insert(ChatModelFeature::PromptCache);
        }
        if !matches!(self.response_format, ChatResponseFormat::Text) {
            required.insert(ChatModelFeature::StructuredOutput);
        }
        if self.provider_round_parent.is_some() {
            required.insert(ChatModelFeature::ProviderRoundState);
        }
        if self.preserve_native_responses_items {
            required.insert(ChatModelFeature::NativeResponsesItems);
        }
        if self
            .requested_output_modalities
            .contains(&ChatModality::Audio)
        {
            required.insert(ChatModelFeature::AudioOutput);
        }
        required
    }

    pub(crate) fn tool_call_names(
        &self,
    ) -> Result<BTreeMap<ToolCallId, String>, ChatContractError> {
        let mut names = BTreeMap::new();
        for message in &self.messages {
            for part in &message.content {
                let ChatContentPart::ToolCall {
                    call_id, name, ..
                } = part
                else {
                    continue;
                };
                if let Some(previous) = names.insert(call_id.clone(), name.clone())
                    && previous != *name
                {
                    return Err(ChatContractError::ConflictingToolCallName);
                }
            }
        }
        Ok(names)
    }

    pub fn validate(&self) -> Result<(), ChatContractError> {
        if self.messages.is_empty() {
            return Err(ChatContractError::EmptyMessages);
        }
        if self.max_output_tokens == Some(0) {
            return Err(ChatContractError::ZeroOutputCeiling);
        }
        if self
            .reasoning
            .as_ref()
            .and_then(|reasoning| reasoning.max_reasoning_tokens)
            == Some(0)
        {
            return Err(ChatContractError::ZeroReasoningCeiling);
        }
        if matches!(self.tool_choice, ChatToolChoice::None) && !self.tools.is_empty() {
            return Err(ChatContractError::ToolChoiceNoneWithTools);
        }
        if !matches!(self.tool_choice, ChatToolChoice::None) && self.tools.is_empty() {
            return Err(ChatContractError::ToolChoiceWithoutTools);
        }

        let mut tool_names = BTreeSet::new();
        for tool in &self.tools {
            validate_natural_key("tool name", &tool.name)?;
            if !tool_names.insert(tool.name.as_str()) {
                return Err(ChatContractError::DuplicateToolName);
            }
        }
        if self
            .metadata
            .keys()
            .any(|key| is_sensitive_structured_key(key))
        {
            return Err(ChatContractError::CredentialMaterialForbidden);
        }
        if let ChatToolChoice::Specific { name } = &self.tool_choice
            && !tool_names.contains(name.as_str())
        {
            return Err(ChatContractError::UnknownSpecificTool);
        }

        for message in &self.messages {
            if message.content.is_empty() {
                return Err(ChatContractError::EmptyMessageContent);
            }
            let role_content_valid = match message.role {
                ChatRole::System => message
                    .content
                    .iter()
                    .all(|part| matches!(part, ChatContentPart::Text { .. })),
                ChatRole::User => message.content.iter().all(|part| {
                    matches!(
                        part,
                        ChatContentPart::Text { .. }
                            | ChatContentPart::Image { .. }
                            | ChatContentPart::Audio { .. }
                    )
                }),
                ChatRole::Assistant => message
                    .content
                    .iter()
                    .all(|part| !matches!(part, ChatContentPart::ToolResult { .. })),
                ChatRole::Tool => message
                    .content
                    .iter()
                    .all(|part| matches!(part, ChatContentPart::ToolResult { .. })),
            };
            if !role_content_valid {
                return Err(ChatContractError::InvalidRoleContent);
            }
            for part in &message.content {
                match part {
                    ChatContentPart::Text { text }
                    | ChatContentPart::Reasoning { text, .. } => {
                        if text.is_empty() {
                            return Err(ChatContractError::EmptyTextPart);
                        }
                    }
                    ChatContentPart::Image {
                        media_type,
                        data_base64,
                    }
                    | ChatContentPart::Audio {
                        media_type,
                        data_base64,
                    } => {
                        validate_media(media_type, data_base64)?;
                    }
                    ChatContentPart::ToolCall {
                        call_id,
                        name,
                        provider_metadata,
                        ..
                    } => {
                        validate_natural_key("tool call id", call_id.as_ref())?;
                        validate_natural_key("tool call name", name)?;
                        if provider_metadata
                            .as_ref()
                            .is_some_and(strict_json_contains_sensitive_key)
                        {
                            return Err(ChatContractError::CredentialMaterialForbidden);
                        }
                    }
                    ChatContentPart::ToolResult {
                        call_id, output, ..
                    } => {
                        validate_natural_key("tool result call id", call_id.as_ref())?;
                        if output.is_empty() {
                            return Err(ChatContractError::EmptyToolResult);
                        }
                        for part in output {
                            match part {
                                ChatToolResultPart::Text { text } if text.is_empty() => {
                                    return Err(ChatContractError::EmptyTextPart);
                                }
                                ChatToolResultPart::Image {
                                    media_type,
                                    data_base64,
                                }
                                | ChatToolResultPart::Audio {
                                    media_type,
                                    data_base64,
                                } => validate_media(media_type, data_base64)?,
                                ChatToolResultPart::Text { .. } => {}
                            }
                        }
                    }
                }
            }
        }

        if let ChatResponseFormat::JsonSchema { name, .. } = &self.response_format {
            validate_natural_key("response schema name", name)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatModelRequest {
    pub contract_version: VersionString,
    pub causality: ChatCausality,
    pub route: ChatRouteSelection,
    pub input: ChatModelInput,
}

impl ChatModelRequest {
    pub fn validate(&self) -> Result<(), ChatContractError> {
        if self.contract_version.as_ref() != CHAT_MODEL_CONTRACT_VERSION {
            return Err(ChatContractError::UnsupportedContractVersion);
        }
        validate_natural_key(
            "agent_session_id",
            self.causality.agent_session_id.as_ref(),
        )?;
        validate_natural_key(
            "turn_operation_id",
            self.causality.turn_operation_id.as_ref(),
        )?;
        validate_natural_key(
            "causation_event_id",
            self.causality.causation_event_id.as_ref(),
        )?;
        validate_natural_key(
            "resolved_snapshot_id",
            self.causality.resolved_snapshot_ref.snapshot_id.as_ref(),
        )?;
        validate_digest(
            "resolved_snapshot_digest",
            &self.causality.resolved_snapshot_ref.snapshot_digest,
        )?;
        validate_natural_key("operation_id", self.causality.operation_id.as_ref())?;
        self.route
            .validate()
            .map_err(|error| ChatContractError::InvalidRouteIdentity(error.to_string()))?;
        self.causality
            .route_identity
            .validate()
            .map_err(|error| ChatContractError::InvalidRouteIdentity(error.to_string()))?;
        if self.causality.route_identity != self.route {
            return Err(ChatContractError::RouteIdentityMismatch);
        }
        self.input.validate()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatToolCall {
    pub call_id: ToolCallId,
    pub name: String,
    pub arguments: StrictJsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<StrictJsonValue>,
}

impl ChatToolCall {
    pub fn validate(&self) -> Result<(), ChatContractError> {
        validate_natural_key("tool call id", self.call_id.as_ref())?;
        validate_natural_key("tool call name", &self.name)?;
        if self
            .provider_metadata
            .as_ref()
            .is_some_and(strict_json_contains_sensitive_key)
        {
            return Err(ChatContractError::CredentialMaterialForbidden);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub audio_input_tokens: u64,
    #[serde(default)]
    pub audio_output_tokens: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_reported: BTreeMap<String, u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatFinishReason {
    Completed,
    ToolCalls,
    MaxOutputTokens,
    Refusal,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatModelEvent {
    ResponseStarted {
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_response_id: Option<ProviderResponseId>,
    },
    OutputTextDelta {
        text: String,
    },
    OutputAudioDelta {
        media_type: String,
        data_base64: String,
    },
    ReasoningDelta {
        text: String,
    },
    ReasoningSignature {
        signature: String,
    },
    ToolCallDelta {
        call_id: ToolCallId,
        name: String,
        arguments_delta: String,
    },
    ToolCallCompleted {
        call: ChatToolCall,
    },
    ProviderRoundId {
        round_id: ProviderRoundId,
    },
    NativeResponsesItem {
        item_type: String,
        item: StrictJsonValue,
    },
    Usage {
        usage: ChatUsage,
    },
    Completed {
        finish_reason: ChatFinishReason,
    },
}

impl ChatModelEvent {
    pub fn is_semantic_output(&self) -> bool {
        !matches!(self, Self::ResponseStarted { .. })
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRetryDirective {
    Never,
    RetrySameRoute,
    Failover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChatModelErrorCode {
    CausalityRejected,
    DuplicateOperation,
    ShadowNotPrimary,
    SessionTerminal,
    RouteNotFound,
    RouteRevisionMismatch,
    AdapterUnavailable,
    CredentialReferenceMissing,
    CredentialTargetMismatch,
    UnsupportedFeature,
    InvalidRequest,
    AuthenticationFailed,
    RateLimited,
    PromptTooLong,
    ProviderUnavailable,
    ProtocolViolation,
    StreamInterrupted,
    Cancelled,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatModelError {
    pub code: ChatModelErrorCode,
    pub message: String,
    pub retry: ChatRetryDirective,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_id: Option<ModelRouteId>,
    pub semantic_output_committed: bool,
}

impl ChatModelError {
    pub fn new(
        code: ChatModelErrorCode,
        message: impl Into<String>,
        retry: ChatRetryDirective,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retry,
            retry_after_ms: None,
            provider_status: None,
            route_id: None,
            semantic_output_committed: false,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(
            ChatModelErrorCode::InvalidRequest,
            message,
            ChatRetryDirective::Never,
        )
    }

    pub fn provider_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            ChatModelErrorCode::ProviderUnavailable,
            message,
            ChatRetryDirective::Failover,
        )
    }

    pub fn stream_interrupted(message: impl Into<String>) -> Self {
        Self::new(
            ChatModelErrorCode::StreamInterrupted,
            message,
            ChatRetryDirective::Failover,
        )
    }

    pub fn protocol_violation(message: impl Into<String>) -> Self {
        Self::new(
            ChatModelErrorCode::ProtocolViolation,
            message,
            ChatRetryDirective::Never,
        )
    }

    pub fn with_route(mut self, route_id: ModelRouteId) -> Self {
        self.route_id = Some(route_id);
        self
    }

    pub fn after_semantic_output(mut self) -> Self {
        self.semantic_output_committed = true;
        self.retry = ChatRetryDirective::Never;
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChatContractError {
    #[error("unsupported chat model contract version")]
    UnsupportedContractVersion,
    #[error("model route revision must be greater than zero")]
    ZeroRouteRevision,
    #[error("chat route identity is invalid: {0}")]
    InvalidRouteIdentity(String),
    #[error("causality and route selection identities differ")]
    RouteIdentityMismatch,
    #[error("chat request must contain at least one message")]
    EmptyMessages,
    #[error("chat message content must not be empty")]
    EmptyMessageContent,
    #[error("chat message content is not valid for its role")]
    InvalidRoleContent,
    #[error("credential material is forbidden in chat metadata")]
    CredentialMaterialForbidden,
    #[error("text content must not be empty")]
    EmptyTextPart,
    #[error("tool result content must not be empty")]
    EmptyToolResult,
    #[error("media type and base64 payload must both be non-empty")]
    InvalidMedia,
    #[error("output token ceiling must be greater than zero")]
    ZeroOutputCeiling,
    #[error("reasoning token ceiling must be greater than zero")]
    ZeroReasoningCeiling,
    #[error("tool_choice=none cannot accompany tool definitions")]
    ToolChoiceNoneWithTools,
    #[error("a non-none tool choice requires tool definitions")]
    ToolChoiceWithoutTools,
    #[error("tool names must be unique")]
    DuplicateToolName,
    #[error("one tool call id is associated with multiple function names")]
    ConflictingToolCallName,
    #[error("specific tool choice does not name a declared tool")]
    UnknownSpecificTool,
    #[error("{0} must be a non-empty trimmed natural key")]
    InvalidNaturalKey(&'static str),
    #[error("{0} must be a 64-character hexadecimal digest")]
    InvalidDigest(&'static str),
    #[error("primary resolved route does not match the requested route")]
    PrimaryRouteMismatch,
    #[error("primary resolved route revision does not match the request")]
    PrimaryRouteRevisionMismatch,
    #[error("resolved route candidates contain a duplicate id/revision")]
    DuplicateRouteCandidate,
    #[error("resolved route does not support text output")]
    RouteMissingTextOutput,
}

fn validate_natural_key(
    field: &'static str,
    value: &str,
) -> Result<(), ChatContractError> {
    if value.is_empty() || value.trim() != value {
        return Err(ChatContractError::InvalidNaturalKey(field));
    }
    Ok(())
}

fn validate_digest(field: &'static str, value: &DigestHex) -> Result<(), ChatContractError> {
    if value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(ChatContractError::InvalidDigest(field))
    }
}

fn strict_json_contains_sensitive_key(value: &StrictJsonValue) -> bool {
    json_contains_sensitive_key(&value.0)
}

fn json_contains_sensitive_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            is_sensitive_structured_key(key) || json_contains_sensitive_key(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(json_contains_sensitive_key),
        _ => false,
    }
}

fn is_sensitive_structured_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key"
            | "apikey"
            | "authorization"
            | "access_token"
            | "refresh_token"
            | "client_secret"
            | "private_key"
            | "credential"
            | "credential_material"
            | "password"
    )
}

fn validate_media(media_type: &str, data_base64: &str) -> Result<(), ChatContractError> {
    if media_type.is_empty()
        || media_type.trim() != media_type
        || data_base64.is_empty()
        || data_base64.trim() != data_base64
    {
        return Err(ChatContractError::InvalidMedia);
    }
    Ok(())
}
