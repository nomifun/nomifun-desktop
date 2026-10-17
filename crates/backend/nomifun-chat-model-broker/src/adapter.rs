use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::Stream;
use nomifun_agent_contracts::{
    ChatRouteIdentity, ConnectionConfigRef, DigestHex, ModelFailureDiagnostic, StrictJsonValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::contracts::{
    ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelErrorCode,
    ChatModelEvent, ChatModelFeature, ChatModelInput, ChatModelRequest, ChatProtocol, ChatResponseFormat, ChatRole,
    ChatToolChoice, ChatToolResultPart, ChatUsage, ProviderCredentialRef,
    ProviderIdRef, ProviderResponseId, ProviderRoundId, ResolvedChatRoute, ToolCallId,
};
use crate::ports::CredentialLease;
use crate::tool_arguments::{JsonToolArgumentParser, ToolArgumentInput, ToolArgumentParser, proposal_event};
use crate::provider_reasoning::{ChatProviderReasoning, ProviderReasoningRoute,
    gemini_tool_matches_route, gemini_tool_signature, validate_reasoning_signature_origins};

pub type ProviderWireStream =
    Pin<Box<dyn Stream<Item = Result<ProviderWireFrame, ChatModelError>> + Send>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderWireRequest {
    pub protocol: ChatProtocol,
    pub provider_id: ProviderIdRef,
    pub model: String,
    /// Identity of the exact attempted route. For a failover candidate the
    /// preset revision and task remain those of the selected route record,
    /// while `route_id/route_revision` identify that candidate.
    pub route_identity: ChatRouteIdentity,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
    pub credential_ref: ProviderCredentialRef,
    pub route_features: BTreeSet<ChatModelFeature>,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderWireFrame {
    pub event: String,
    #[serde(default)]
    pub data: Value,
    /// Credential-redacted HTTP context for a verified native error frame.
    /// Normal output frames carry no diagnostic metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<ModelFailureDiagnostic>,
}

/// One attempt's frame accumulator. Its state is dropped on success, error,
/// cancellation or receiver closure; it must never cross Session/route or
/// retry boundaries, even when provider response IDs are absent or reused.
pub trait ChatFrameDecoder: Send {
    fn decode_frame(
        &mut self,
        frame: ProviderWireFrame,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError>;

    /// Validated native content may be buffered until its complete reasoning
    /// block is safe to deliver. Callers can renew idle waiting on that progress
    /// without exposing partial blocks or changing the model event contract.
    fn has_deferred_semantic_progress(&self) -> bool {
        false
    }

    /// Last numeric output count actually reported by this attempt. Missing
    /// provider counters remain unknown even though ChatUsage uses zero as
    /// its established projection default.
    fn reported_output_tokens(&self) -> Option<u64> {
        None
    }
}

#[async_trait]
pub trait ProviderTransport: Send + Sync {
    /// Open exactly one provider stream. Transports do not retry; retry and
    /// failover belong exclusively to [`crate::broker::ChatModelBroker`].
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError>;
}

#[async_trait]
pub trait ChatProtocolAdapter: Send + Sync {
    fn protocol(&self) -> ChatProtocol;
    fn name(&self) -> &'static str;
    fn features(&self) -> BTreeSet<ChatModelFeature> {
        protocol_features(self.protocol())
    }
    fn retry_count(&self) -> u8 {
        0
    }
    /// Stateful protocol implementations should supply a fresh decoder for
    /// every transport attempt. None preserves the legacy decode_frame API
    /// for custom stateless adapters; official adapters always return Some.
    fn new_frame_decoder(&self) -> Option<Box<dyn ChatFrameDecoder>> {
        None
    }
    fn new_frame_decoder_for(&self, _request: &ChatModelRequest) -> Option<Box<dyn ChatFrameDecoder>> {
        self.new_frame_decoder()
    }
    fn encode_request(
        &self,
        request: &ChatModelRequest,
        route: &ResolvedChatRoute,
        credential: &CredentialLease,
    ) -> Result<ProviderWireRequest, ChatModelError>;
    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError>;
    /// Legacy direct decoding. Production Broker attempts prefer the fresh
    /// decoder returned above; callers using this stateful API themselves
    /// must provide their own stream isolation and lifecycle.
    fn decode_frame(
        &self,
        frame: ProviderWireFrame,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError>;
}

trait EncodedRequestBody {
    fn into_result(self) -> Result<Value, ChatModelError>;
}

impl EncodedRequestBody for Value {
    fn into_result(self) -> Result<Value, ChatModelError> {
        Ok(self)
    }
}

impl EncodedRequestBody for Result<Value, ChatModelError> {
    fn into_result(self) -> Result<Value, ChatModelError> {
        self
    }
}

/// Resolve an explicit persisted Chat protocol to its serializer. Model names
/// and catalog suggestions never participate in transport selection.
pub fn chat_protocol_for_id(protocol_id: &str) -> Option<ChatProtocol> {
    match protocol_id {
        "anthropic.messages" => Some(ChatProtocol::Anthropic),
        "openai.chat_text" => Some(ChatProtocol::OpenaiChat),
        "openai.responses" => Some(ChatProtocol::OpenaiResponses),
        "gemini.generate_text" => Some(ChatProtocol::Gemini),
        "bedrock.anthropic_messages" => Some(ChatProtocol::Bedrock),
        "vertex.anthropic_messages" => Some(ChatProtocol::Vertex),
        _ => None,
    }
}

/// Request features the serializer can carry. Provider acceptance remains an
/// upstream fact; missing catalog metadata never disables representable input.
pub fn protocol_features(protocol: ChatProtocol) -> BTreeSet<ChatModelFeature> {
    use ChatModelFeature as Feature;

    let mut features = BTreeSet::from([
        Feature::TextInput,
        Feature::TextOutput,
        Feature::ToolCalls,
        Feature::Reasoning,
        Feature::Streaming,
    ]);
    match protocol {
        ChatProtocol::Anthropic => {
            features.extend([
                Feature::ImageInput,
                Feature::ReasoningSignature,
                Feature::PromptCache,
            ]);
        }
        ChatProtocol::OpenaiChat => {
            features.extend([
                Feature::ImageInput,
                Feature::AudioInput,
                Feature::StructuredOutput,
            ]);
        }
        ChatProtocol::OpenaiResponses => {
            features.extend([
                Feature::ImageInput,
                Feature::AudioInput,
                Feature::AudioOutput,
                Feature::PromptCache,
                Feature::StructuredOutput,
                Feature::ProviderRoundState,
                Feature::NativeResponsesItems,
            ]);
        }
        ChatProtocol::Gemini => {
            features.extend([
                Feature::ImageInput,
                Feature::AudioInput,
                Feature::ReasoningSignature,
                Feature::PromptCache,
                Feature::StructuredOutput,
            ]);
        }
        ChatProtocol::Bedrock | ChatProtocol::Vertex => {
            features.extend([
                Feature::ImageInput,
                Feature::ReasoningSignature,
                Feature::PromptCache,
            ]);
        }
    }
    features
}

#[derive(Clone)]
struct AdapterCore {
    protocol: ChatProtocol,
    name: &'static str,
    transport: Arc<dyn ProviderTransport>,
    raw_decode_state: Arc<Mutex<RawDecodeState>>,
    argument_parser: Arc<dyn ToolArgumentParser>,
}

impl AdapterCore {
    fn new(
        protocol: ChatProtocol,
        name: &'static str,
        transport: Arc<dyn ProviderTransport>,
        argument_parser: Arc<dyn ToolArgumentParser>,
    ) -> Self {
        Self {
            protocol,
            name,
            transport,
            raw_decode_state: Arc::new(Mutex::new(RawDecodeState::default())),
            argument_parser,
        }
    }

    fn encode<B: EncodedRequestBody>(
        &self,
        request: &ChatModelRequest,
        route: &ResolvedChatRoute,
        credential: &CredentialLease,
        body: B,
    ) -> Result<ProviderWireRequest, ChatModelError> {
        request
            .validate()
            .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;
        validate_reasoning_signature_origins(&request.input)?;
        for part in request.input.messages.iter().flat_map(|message| &message.content) {
            if let ChatContentPart::ToolCall { provider_metadata, .. } = part
                && !gemini_tool_matches_route(provider_metadata, route)?
            {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::UnsupportedFeature,
                    "Gemini tool signature requires the exact route that produced it",
                    crate::contracts::ChatRetryDirective::Never,
                ));
            }
        }
        if self.protocol != ChatProtocol::OpenaiResponses && request.input.messages.iter().any(|message| {
            message.content.iter().any(|part| matches!(part,
                ChatContentPart::Reasoning { encrypted_content: Some(_), .. }))
        }) {
            return Err(ChatModelError::new(
                ChatModelErrorCode::UnsupportedFeature,
                "opaque Responses reasoning cannot be sent to a different protocol",
                crate::contracts::ChatRetryDirective::Never,
            ));
        }
        if request.input.messages.iter().any(|message| {
            message.content.iter().any(|part| matches!(part, ChatContentPart::ProviderReasoning { block }
                if !block.matches_route(route)))
        }) {
            return Err(ChatModelError::new(
                ChatModelErrorCode::UnsupportedFeature,
                "provider reasoning requires the exact route that produced it",
                crate::contracts::ChatRetryDirective::Never,
            ));
        }
        let body = body.into_result()?;
        route
            .validate()
            .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;
        if route.protocol != self.protocol {
            return Err(ChatModelError::new(
                ChatModelErrorCode::AdapterUnavailable,
                format!(
                    "adapter {} cannot encode {:?} route",
                    self.name, route.protocol
                ),
                crate::contracts::ChatRetryDirective::Never,
            ));
        }
        if !credential.validates_route(route) {
            return Err(ChatModelError::new(
                ChatModelErrorCode::CredentialTargetMismatch,
                "credential reference does not match the resolved provider route",
                crate::contracts::ChatRetryDirective::Never,
            ));
        }
        let route_identity = request.route.with_route(
            route.model_route_id.clone(),
            route.model_route_revision,
        );
        Ok(ProviderWireRequest {
            protocol: self.protocol,
            provider_id: route.provider_id.clone(),
            model: route.model.clone(),
            route_identity,
            connection_config_ref: route.connection_config_ref.clone(),
            config_revision_digest: route.config_revision_digest.clone(),
            credential_ref: route.credential_ref.clone(),
            route_features: route.features.clone(),
            body,
        })
    }

    async fn open_stream(
        &self,
        request: ProviderWireRequest,
        credential: CredentialLease,
    ) -> Result<ProviderWireStream, ChatModelError> {
        self.transport.open_stream(request, credential).await
    }
}

macro_rules! define_adapter {
    (
        $name:ident,
        $protocol:expr,
        $label:literal,
        $encoder:ident
    ) => {
        pub struct $name {
            core: AdapterCore,
        }

        impl $name {
            pub fn new(transport: Arc<dyn ProviderTransport>) -> Self {
                Self::with_argument_parser(transport, Arc::new(JsonToolArgumentParser))
            }

            pub fn with_argument_parser(transport: Arc<dyn ProviderTransport>, argument_parser: Arc<dyn ToolArgumentParser>) -> Self {
                Self { core: AdapterCore::new($protocol, $label, transport, argument_parser) }
            }
        }

        #[async_trait]
        impl ChatProtocolAdapter for $name {
            fn protocol(&self) -> ChatProtocol {
                self.core.protocol
            }

            fn name(&self) -> &'static str {
                self.core.name
            }

            fn new_frame_decoder(&self) -> Option<Box<dyn ChatFrameDecoder>> {
                Some(Box::new(AttemptFrameDecoder {
                    protocol: self.core.protocol,
                    reported_output_tokens: None,
                    raw_decode_state: Arc::new(Mutex::new(RawDecodeState::default())),
                    responses: crate::responses_decoder::ResponsesDecoder::with_argument_parser(false, self.core.argument_parser.clone()),
                    anthropic: crate::anthropic_decoder::AnthropicDecoder::with_argument_parser(self.core.argument_parser.clone()),
                    argument_parser: self.core.argument_parser.clone(),
                }))
            }

            fn new_frame_decoder_for(&self, request: &ChatModelRequest) -> Option<Box<dyn ChatFrameDecoder>> {
                Some(Box::new(AttemptFrameDecoder {
                    protocol: self.core.protocol,
                    reported_output_tokens: None,
                    raw_decode_state: Arc::new(Mutex::new(RawDecodeState::default())),
                    responses: crate::responses_decoder::ResponsesDecoder::with_argument_parser(request.input.preserve_native_responses_items, self.core.argument_parser.clone()),
                    anthropic: crate::anthropic_decoder::AnthropicDecoder::with_argument_parser(self.core.argument_parser.clone()),
                    argument_parser: self.core.argument_parser.clone(),
                }))
            }

            fn encode_request(
                &self,
                request: &ChatModelRequest,
                route: &ResolvedChatRoute,
                credential: &CredentialLease,
            ) -> Result<ProviderWireRequest, ChatModelError> {
                self.core
                    .encode(request, route, credential, $encoder(&request.input, &route.model))
            }

            async fn open_stream(
                &self,
                request: ProviderWireRequest,
                credential: CredentialLease,
            ) -> Result<ProviderWireStream, ChatModelError> {
                self.core.open_stream(request, credential).await
            }

            fn decode_frame(
                &self,
                frame: ProviderWireFrame,
            ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
                decode_frame_for_protocol(
                    self.core.protocol,
                    frame,
                    &self.core.raw_decode_state,
                    self.core.argument_parser.as_ref(),
                )
            }
        }
    };
}

define_adapter!(
    AnthropicAdapter,
    ChatProtocol::Anthropic,
    "anthropic.messages",
    encode_anthropic_request
);
define_adapter!(
    OpenAiChatAdapter,
    ChatProtocol::OpenaiChat,
    "openai.chat",
    encode_openai_chat_request
);
define_adapter!(
    OpenAiResponsesAdapter,
    ChatProtocol::OpenaiResponses,
    "openai.responses",
    encode_openai_responses_request
);
define_adapter!(
    GeminiAdapter,
    ChatProtocol::Gemini,
    "google.gemini",
    encode_gemini_request
);
define_adapter!(
    BedrockAdapter,
    ChatProtocol::Bedrock,
    "amazon.bedrock",
    encode_bedrock_request
);
define_adapter!(
    VertexAdapter,
    ChatProtocol::Vertex,
    "google.vertex",
    encode_vertex_request
);

/// Anthropic-family Messages services (Anthropic, Bedrock, Vertex) reject a
/// root `oneOf`/`anyOf`/`allOf` in a tool's `input_schema`, but canonical tool
/// contracts may carry cross-field unions there. The wire schema keeps the
/// root object and drops only those combinators (hoisting branch fields when
/// the root lists none) while the runtime keeps validating every call against
/// the full canonical schema. Nested schemas, `$defs`, `additionalProperties`,
/// root `not`/`if`/`then`/`else` and descriptions are never touched.
fn anthropic_input_schema(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema.clone();
    };
    if !["oneOf", "anyOf", "allOf"]
        .iter()
        .any(|key| object.contains_key(*key))
    {
        return schema.clone();
    }
    let mut lowered = object.clone();
    let mut one_of_branches: Vec<serde_json::Map<String, Value>> = Vec::new();
    let mut any_of_branches: Vec<serde_json::Map<String, Value>> = Vec::new();
    let mut all_branches: Vec<serde_json::Map<String, Value>> = Vec::new();
    for (key, branches) in [
        ("oneOf", &mut one_of_branches),
        ("anyOf", &mut any_of_branches),
        ("allOf", &mut all_branches),
    ] {
        if let Some(Value::Array(items)) = lowered.remove(key) {
            branches.extend(items.into_iter().filter_map(|branch| match branch {
                Value::Object(map) => Some(map),
                _ => None,
            }));
        }
    }
    let mut properties = lowered
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if properties.is_empty() {
        // Hoist one distinct definition per name; conflicting definitions keep
        // every variant under a nested anyOf, which is still valid.
        for branch in one_of_branches
            .iter()
            .chain(&any_of_branches)
            .chain(&all_branches)
        {
            if let Some(branch_properties) = branch.get("properties").and_then(Value::as_object) {
                for (name, definition) in branch_properties {
                    match properties.entry(name.clone()) {
                        serde_json::map::Entry::Vacant(slot) => {
                            slot.insert(definition.clone());
                        }
                        serde_json::map::Entry::Occupied(mut slot) => {
                            if slot.get() != definition {
                                let existing = slot.get_mut();
                                match existing {
                                    Value::Object(existing)
                                        if existing
                                            .get("anyOf")
                                            .and_then(Value::as_array)
                                            .is_some_and(|options| {
                                                options.iter().all(Value::is_object)
                                            }) =>
                                    {
                                        existing
                                            .get_mut("anyOf")
                                            .and_then(Value::as_array_mut)
                                            .expect("checked above")
                                            .push(definition.clone());
                                    }
                                    _ => {
                                        *existing =
                                            json!({"anyOf": [existing.clone(), definition.clone()]});
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if !properties.is_empty() {
            lowered.insert("properties".to_owned(), Value::Object(properties));
        }
    }
    if !lowered.contains_key("required") {
        let mut required: Option<BTreeSet<String>> = None;
        let mut intersect = |branches: &[serde_json::Map<String, Value>]| {
            if branches.is_empty() {
                return;
            }
            let mut branch_required = branches.iter().map(|branch| {
                branch
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<BTreeSet<String>>()
                    })
                    .unwrap_or_default()
            });
            let mut shared = branch_required.next().unwrap_or_default();
            for set in branch_required {
                shared = shared.intersection(&set).cloned().collect();
            }
            required
                .get_or_insert_with(BTreeSet::new)
                .extend(shared.into_iter());
        };
        intersect(&one_of_branches);
        intersect(&any_of_branches);
        for branch in &all_branches {
            if let Some(names) = branch.get("required").and_then(Value::as_array) {
                required.get_or_insert_with(BTreeSet::new).extend(
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
        if let Some(mut names) = required {
            let available = lowered
                .get("properties")
                .and_then(Value::as_object)
                .map(|properties| {
                    properties.keys().cloned().collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            names.retain(|name| available.contains(name));
            if !names.is_empty() {
                lowered.insert(
                    "required".to_owned(),
                    Value::Array(
                        names.into_iter().map(Value::String).collect(),
                    ),
                );
            }
        }
    }
    Value::Object(lowered)
}

fn encode_anthropic_request(
    input: &ChatModelInput,
    model: &str,
) -> Result<Value, ChatModelError> {
    let max_tokens = input.max_output_tokens.ok_or_else(|| ChatModelError::invalid_request(
        "Anthropic Messages requires an explicit output token ceiling",
    ))?;
    let system = combined_system_text(&input.instructions, &input.messages);
    let mut body = json!({
        "model": model,
        "system": system,
        "messages": anthropic_messages(&input.messages),
        "stream": true,
        "max_tokens": max_tokens,
    });
    if !input.tools.is_empty() {
        body["tools"] = Value::Array(
            input
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": anthropic_input_schema(&tool.input_schema.0),
                    })
                })
                .collect(),
        );
        body["tool_choice"] = anthropic_tool_choice(&input.tool_choice);
        if let Some(parallel) = input.parallel_tool_calls {
            body["tool_choice"]["disable_parallel_tool_use"] = json!(!parallel);
        }
    }
    if let Some(reasoning) = &input.reasoning {
        let budget = reasoning.max_reasoning_tokens.ok_or_else(|| ChatModelError::invalid_request(
            "Anthropic budgeted thinking requires max_reasoning_tokens; adaptive thinking is not configured",
        ))?;
        if budget < 1024 || budget >= max_tokens {
            return Err(ChatModelError::invalid_request("Anthropic thinking budget must be at least 1024 and below max_output_tokens"));
        }
        if reasoning.effort.is_some()
            || matches!(reasoning.summary, crate::contracts::ReasoningSummary::Concise | crate::contracts::ReasoningSummary::Detailed)
        {
            return Err(ChatModelError::new(ChatModelErrorCode::UnsupportedFeature,
                "budgeted Anthropic thinking does not implement explicit effort or summary detail controls",
                crate::contracts::ChatRetryDirective::Never));
        }
        if matches!(input.tool_choice, ChatToolChoice::Required | ChatToolChoice::Specific { .. }) {
            return Err(ChatModelError::invalid_request("budgeted Anthropic thinking cannot force a tool choice"));
        }
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }
    // Cache a static instruction prefix when present. Do not fabricate a
    // user message, cache signed thinking, or promise a server cache hit.
    if !matches!(input.prompt_cache, crate::contracts::PromptCachePolicy::Disabled) && !system.is_empty() {
        body["system"] = json!([{"type":"text", "text":system, "cache_control":{"type":"ephemeral"}}]);
    }
    Ok(body)
}

fn encode_openai_chat_request(
    input: &ChatModelInput,
    model: &str,
) -> Result<Value, ChatModelError> {
    if let Some(reasoning) = &input.reasoning {
        if reasoning.max_reasoning_tokens.is_some()
            || matches!(reasoning.summary, crate::contracts::ReasoningSummary::Concise | crate::contracts::ReasoningSummary::Detailed) {
            return Err(ChatModelError::new(ChatModelErrorCode::UnsupportedFeature,
                "OpenAI Chat cannot encode a separate reasoning token budget or explicit summary detail; use Responses for summaries",
                crate::contracts::ChatRetryDirective::Never));
        }
    }
    let mut messages = Vec::new();
    for instruction in &input.instructions {
        messages.push(json!({"role": "system", "content": instruction}));
    }
    let mut tool_media = Vec::new();
    for message in &input.messages {
        if message.role != ChatRole::Tool && !tool_media.is_empty() {
            messages.push(json!({"role":"user", "content":std::mem::take(&mut tool_media)}));
        }
        messages.extend(openai_messages(message));
        if message.role == ChatRole::Tool {
            for part in &message.content {
                if let ChatContentPart::ToolResult { call_id, output, .. } = part {
                    let media = openai_tool_result_media(output);
                    if !media.is_empty() {
                        tool_media.push(json!({"type":"text", "text":format!("Attachments returned by tool call {}: ", call_id.as_ref())}));
                        tool_media.extend(media);
                    }
                }
            }
        }
    }
    if !tool_media.is_empty() {
        messages.push(json!({"role":"user", "content":tool_media}));
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "store": false,
    });
    insert_if_some(
        &mut body,
        "max_tokens",
        input.max_output_tokens.map(Value::from),
    );
    if !input.tools.is_empty() {
        body["tools"] = Value::Array(
            input
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema.0.clone(),
                        }
                    })
                })
                .collect(),
        );
        body["tool_choice"] = openai_tool_choice(&input.tool_choice);
        insert_if_some(&mut body, "parallel_tool_calls", input.parallel_tool_calls.map(Value::from));
    }
    if let Some(reasoning) = &input.reasoning
        && let Some(effort) = reasoning.effort
    {
        body["reasoning_effort"] = Value::String(effort.as_str().to_owned());
    }
    if !matches!(input.response_format, ChatResponseFormat::Text) {
        body["response_format"] = response_format_value(&input.response_format);
    }
    Ok(body)
}

fn encode_openai_responses_request(
    input: &ChatModelInput,
    model: &str,
) -> Result<Value, ChatModelError> {
    if input.reasoning.as_ref().is_some_and(|reasoning| reasoning.max_reasoning_tokens.is_some()) {
        return Err(ChatModelError::new(ChatModelErrorCode::UnsupportedFeature,
            "OpenAI Responses does not expose a separate reasoning token budget; configure effort or the total output limit",
            crate::contracts::ChatRetryDirective::Never));
    }
    let mut body = json!({
        "model": model,
        "instructions": input.instructions.join("\n\n"),
        "input": responses_items(&input.messages),
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"],
    });
    insert_if_some(
        &mut body,
        "max_output_tokens",
        input.max_output_tokens.map(Value::from),
    );
    if let Some(parent) = &input.provider_round_parent {
        body["previous_response_id"] = Value::String(parent.as_ref().to_owned());
    }
    if !input.tools.is_empty() {
        body["tools"] = Value::Array(
            input
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema.0.clone(),
                        // Platform schemas retain optional fields and are
                        // validated by Kernel. Do not silently rewrite them
                        // into the provider's all-properties-required subset.
                        "strict": false,
                    })
                })
                .collect(),
        );
        body["tool_choice"] = openai_responses_tool_choice(&input.tool_choice);
        insert_if_some(&mut body, "parallel_tool_calls", input.parallel_tool_calls.map(Value::from));
    }
    if !matches!(input.response_format, ChatResponseFormat::Text) {
        body["text"] = json!({
            "format": responses_format_value(&input.response_format)
        });
    }
    if let Some(reasoning) = &input.reasoning {
        let mut options = Map::new();
        if let Some(effort) = reasoning.effort {
            options.insert("effort".into(), Value::String(effort.as_str().into()));
        }
        if reasoning.summary != crate::contracts::ReasoningSummary::None {
            options.insert("summary".into(), Value::String(format!("{:?}", reasoning.summary).to_ascii_lowercase()));
        }
        if !options.is_empty() { body["reasoning"] = Value::Object(options); }
    }
    Ok(body)
}

fn encode_gemini_request(
    input: &ChatModelInput,
    _model: &str,
) -> Result<Value, ChatModelError> {
    if matches!(input.prompt_cache, crate::contracts::PromptCachePolicy::Ephemeral) {
        return Err(ChatModelError::new(ChatModelErrorCode::UnsupportedFeature,
            "Gemini ephemeral caching requires an explicit cachedContent lifecycle; this adapter supports automatic caching",
            crate::contracts::ChatRetryDirective::Never));
    }
    let call_names = input
        .tool_call_names()
        .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;
    let mut body = json!({
        "systemInstruction": {
            "parts": [{
                "text": combined_system_text(&input.instructions, &input.messages)
            }]
        },
        "contents": gemini_contents(&input.messages, &call_names)?,
    });
    let mut generation = Map::new();
    if let Some(max_output_tokens) = input.max_output_tokens {
        generation.insert("maxOutputTokens".to_owned(), Value::from(max_output_tokens));
    }
    if let Some(reasoning) = &input.reasoning {
        if reasoning.effort.is_some() && reasoning.max_reasoning_tokens.is_some() {
            return Err(ChatModelError::invalid_request(
                "Gemini thinking cannot combine an effort level and a token budget",
            ));
        }
        let mut thinking = Map::new();
        if let Some(effort) = reasoning.effort {
            let thinking_level = match effort {
            crate::contracts::ReasoningEffort::Minimal => "minimal",
            crate::contracts::ReasoningEffort::Low => "low",
            crate::contracts::ReasoningEffort::Medium => "medium",
            crate::contracts::ReasoningEffort::High => "high",
            crate::contracts::ReasoningEffort::None
            | crate::contracts::ReasoningEffort::XHigh
            | crate::contracts::ReasoningEffort::Max
            | crate::contracts::ReasoningEffort::Ultra => {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::UnsupportedFeature,
                    "Gemini thinking level does not support none, xhigh, max, or ultra",
                    crate::contracts::ChatRetryDirective::Never,
                ));
            }
            };
            thinking.insert("thinkingLevel".into(), Value::String(thinking_level.into()));
        }
        if let Some(budget) = reasoning.max_reasoning_tokens {
            thinking.insert("thinkingBudget".into(), Value::from(budget));
        }
        match reasoning.summary {
            crate::contracts::ReasoningSummary::None => {}
            crate::contracts::ReasoningSummary::Auto => { thinking.insert("includeThoughts".into(), Value::Bool(true)); }
            crate::contracts::ReasoningSummary::Concise | crate::contracts::ReasoningSummary::Detailed => {
                return Err(ChatModelError::new(ChatModelErrorCode::UnsupportedFeature,
                    "Gemini exposes a boolean thought-summary control, not concise or detailed summary levels",
                    crate::contracts::ChatRetryDirective::Never));
            }
        }
        if !thinking.is_empty() { generation.insert("thinkingConfig".into(), Value::Object(thinking)); }
    }
    if !matches!(input.response_format, ChatResponseFormat::Text) {
        generation.insert("responseMimeType".to_owned(), Value::String("application/json".to_owned()));
        if let ChatResponseFormat::JsonSchema { schema, .. } = &input.response_format {
            generation.insert("responseSchema".to_owned(), schema.0.clone());
        }
    }
    body["generationConfig"] = Value::Object(generation);
    if !input.tools.is_empty() {
        body["tools"] = json!([{
            "functionDeclarations": input.tools.iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema.0.clone(),
            })).collect::<Vec<_>>()
        }]);
        body["toolConfig"] = gemini_tool_config(&input.tool_choice);
    }
    Ok(body)
}

fn encode_bedrock_request(
    input: &ChatModelInput,
    model: &str,
) -> Result<Value, ChatModelError> {
    let mut body = encode_anthropic_request(input, model)?;
    // Bedrock selects the model and streaming operation in the request URL.
    let object = body.as_object_mut().expect("Messages encoder returns an object");
    object.remove("model");
    object.remove("stream");
    body["anthropic_version"] = Value::String("bedrock-2023-05-31".to_owned());
    Ok(body)
}

fn encode_vertex_request(
    input: &ChatModelInput,
    model: &str,
) -> Result<Value, ChatModelError> {
    let mut body = encode_anthropic_request(input, model)?;
    body.as_object_mut().expect("Messages encoder returns an object").remove("model");
    body["anthropic_version"] = Value::String("vertex-2023-10-16".to_owned());
    Ok(body)
}

/// Encode a stateless, tool-free model input without inventing an Agent Turn
/// or bypassing its causal admission. Routing and credentials stay with the
/// standalone caller; protocol serialization uses the same Agent encoders.
pub fn encode_standalone_chat_input(
    protocol: ChatProtocol,
    model: &str,
    input: &ChatModelInput,
    provider_params: &Value,
    configured_output_limit: Option<i64>,
) -> Result<Value, ChatModelError> {
    if model.trim().is_empty() || !input.tools.is_empty() || input.provider_round_parent.is_some()
        || input.preserve_native_responses_items
        || !matches!(input.tool_choice, ChatToolChoice::Auto | ChatToolChoice::None)
        || input.messages.iter().any(|message| matches!(message.role, ChatRole::Tool) || message.provider_round_id.is_some())
        || input.messages.iter().flat_map(|message| &message.content)
            .any(|part| !matches!(part, ChatContentPart::Text { .. })) {
        return Err(ChatModelError::invalid_request("standalone completion requires stateless text input without tools or native history"));
    }
    let body = match protocol {
        ChatProtocol::Anthropic => encode_anthropic_request(input, model),
        ChatProtocol::OpenaiChat => encode_openai_chat_request(input, model),
        ChatProtocol::OpenaiResponses => encode_openai_responses_request(input, model),
        ChatProtocol::Gemini => encode_gemini_request(input, model),
        ChatProtocol::Bedrock => encode_bedrock_request(input, model),
        ChatProtocol::Vertex => encode_vertex_request(input, model),
    }?;
    let mut body = crate::provider_params::merge_chat_provider_params(body, provider_params, protocol, configured_output_limit)?;
    if let Some(object) = body.as_object_mut() {
        for key in ["tools", "tool_choice", "parallel_tool_calls", "toolConfig", "previous_response_id", "conversation"] {
            object.remove(key);
        }
    }
    Ok(body)
}

/// Encode one bounded verification exchange with the same Agent serializers
/// and provider-parameter merge. Unlike standalone completion it carries
/// function tools plus the exchange's own tool call/result and reasoning
/// replay; it never carries provider-retained round state.
pub fn encode_verification_chat_input(
    protocol: ChatProtocol,
    model: &str,
    input: &ChatModelInput,
    provider_params: &Value,
    configured_output_limit: Option<i64>,
) -> Result<Value, ChatModelError> {
    if model.trim().is_empty()
        || input.provider_round_parent.is_some()
        || input.preserve_native_responses_items
        || !matches!(input.tool_choice, ChatToolChoice::Auto | ChatToolChoice::None)
        || input
            .messages
            .iter()
            .any(|message| message.provider_round_id.is_some())
        || input.messages.iter().flat_map(|message| &message.content).any(|part| {
            matches!(part, ChatContentPart::Image { .. } | ChatContentPart::Audio { .. })
        })
    {
        return Err(ChatModelError::invalid_request(
            "verification input must be exchange-local text/tool content without provider-retained round state",
        ));
    }
    validate_reasoning_signature_origins(input)?;
    for part in input.messages.iter().flat_map(|message| &message.content) {
        match part {
            ChatContentPart::Reasoning {
                encrypted_content: Some(_),
                ..
            } if protocol != ChatProtocol::OpenaiResponses => {
                return Err(ChatModelError::new(
                    ChatModelErrorCode::UnsupportedFeature,
                    "opaque Responses reasoning cannot be sent to a different protocol",
                    crate::contracts::ChatRetryDirective::Never,
                ));
            }
            // One verification target produces and consumes the exchange, so
            // route-digest binding does not apply; only the producing
            // protocol family can carry each provider reasoning kind.
            ChatContentPart::ProviderReasoning { block } => {
                let compatible = match block {
                    ChatProviderReasoning::OpenaiResponsesReasoning { .. } => {
                        protocol == ChatProtocol::OpenaiResponses
                    }
                    _ => protocol.uses_anthropic_messages(),
                };
                if !compatible || block.validate().is_err() {
                    return Err(ChatModelError::new(
                        ChatModelErrorCode::UnsupportedFeature,
                        "provider reasoning is incompatible with the verification protocol",
                        crate::contracts::ChatRetryDirective::Never,
                    ));
                }
            }
            _ => {}
        }
    }
    let body = match protocol {
        ChatProtocol::Anthropic => encode_anthropic_request(input, model),
        ChatProtocol::OpenaiChat => encode_openai_chat_request(input, model),
        ChatProtocol::OpenaiResponses => encode_openai_responses_request(input, model),
        ChatProtocol::Gemini => encode_gemini_request(input, model),
        ChatProtocol::Bedrock => encode_bedrock_request(input, model),
        ChatProtocol::Vertex => encode_vertex_request(input, model),
    }?;
    let mut body = crate::provider_params::merge_chat_provider_params(
        body,
        provider_params,
        protocol,
        configured_output_limit,
    )?;
    // Encoder-owned tools/tool_choice/parallel_tool_calls stay authoritative:
    // they merge as missing-only, and retained-round keys are stripped again
    // after the merge even though provider params can no longer insert them.
    if let Some(object) = body.as_object_mut() {
        for key in ["previous_response_id", "conversation"] {
            object.remove(key);
        }
    }
    Ok(body)
}

/// Every standalone call receives fresh protocol state. This supplies only
/// decoding, never Runtime recovery or retained provider round authority.
pub fn new_standalone_frame_decoder(protocol: ChatProtocol) -> Box<dyn ChatFrameDecoder> {
    let argument_parser = Arc::new(JsonToolArgumentParser);
    Box::new(AttemptFrameDecoder {
        protocol,
        reported_output_tokens: None,
        raw_decode_state: Arc::new(Mutex::new(RawDecodeState::default())),
        responses: crate::responses_decoder::ResponsesDecoder::with_argument_parser(false, argument_parser.clone()),
        anthropic: crate::anthropic_decoder::AnthropicDecoder::with_argument_parser(argument_parser.clone()),
        argument_parser,
    })
}

/// The verification decoder is the standalone protocol decoder plus the
/// broker's canonical event-sequence rules: a verification round replays
/// decoder-produced reasoning and tool state back to the same endpoint, so
/// malformed event sequences must fail exactly like broker consumption.
pub fn new_verification_frame_decoder(protocol: ChatProtocol) -> Box<dyn ChatFrameDecoder> {
    Box::new(VerificationFrameDecoder {
        inner: new_standalone_frame_decoder(protocol),
        sequence: crate::broker::EventSequence::default(),
    })
}

struct VerificationFrameDecoder {
    inner: Box<dyn ChatFrameDecoder>,
    sequence: crate::broker::EventSequence,
}

impl ChatFrameDecoder for VerificationFrameDecoder {
    fn decode_frame(
        &mut self,
        frame: ProviderWireFrame,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let events = self.inner.decode_frame(frame)?;
        for event in &events {
            self.sequence.observe(event)?;
        }
        Ok(events)
    }

    fn has_deferred_semantic_progress(&self) -> bool {
        self.inner.has_deferred_semantic_progress()
    }

    fn reported_output_tokens(&self) -> Option<u64> {
        self.inner.reported_output_tokens()
    }
}

#[cfg(test)]
mod standalone_input_tests {
    use super::*;

    fn input() -> ChatModelInput {
        serde_json::from_value(json!({
            "instructions":["Return concise JSON"], "messages":[{"role":"user","content":[{"type":"text","text":"A completed answer"}]}],
            "tools":[],"tool_choice":{"mode":"none"},"max_output_tokens":4096,
            "prompt_cache":"disabled","response_format":{"type":"text"}
        })).unwrap()
    }

    #[test]
    fn standalone_encoding_shares_protocol_fields_without_tools_or_provider_rounds() {
        for protocol in [ChatProtocol::OpenaiChat,ChatProtocol::OpenaiResponses,ChatProtocol::Anthropic,
            ChatProtocol::Gemini,ChatProtocol::Bedrock,ChatProtocol::Vertex] {
            let body = encode_standalone_chat_input(protocol,"selected-model",&input(),
                &json!({"temperature":0.2,"tools":[{"name":"forbidden"}],"previous_response_id":"old-round"}),Some(2048)).unwrap();
            assert!(body.get("tools").is_none(),"{protocol:?}");
            assert!(body.get("previous_response_id").is_none(),"{protocol:?}");
            match protocol {
                ChatProtocol::Gemini => assert_eq!(body["generationConfig"]["maxOutputTokens"],2048),
                ChatProtocol::OpenaiResponses => assert_eq!(body["max_output_tokens"],2048),
                _ => assert_eq!(body["max_tokens"],2048),
            }
        }
    }

    #[test]
    fn standalone_encoding_rejects_native_context_and_does_not_invent_output_or_schema_parameters() {
        let mut value = input();
        value.provider_round_parent = Some("retained-round".into());
        assert!(encode_standalone_chat_input(ChatProtocol::OpenaiChat,"model",&value,&json!({}),None).is_err());
        value.provider_round_parent = None;
        value.max_output_tokens = None;
        let body = encode_standalone_chat_input(ChatProtocol::OpenaiChat,"model",&value,&json!({}),None).unwrap();
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn standalone_decoder_distinguishes_missing_output_usage_from_reported_zero() {
        for protocol in [ChatProtocol::OpenaiChat,ChatProtocol::OpenaiResponses,ChatProtocol::Anthropic,
            ChatProtocol::Gemini,ChatProtocol::Bedrock,ChatProtocol::Vertex] {
            let mut decoder = new_standalone_frame_decoder(protocol);
            let frame = |data| ProviderWireFrame { event:"usage".into(),data,diagnostic:None };
            decoder.decode_frame(frame(json!({"input_tokens":100}))).unwrap();
            assert_eq!(decoder.reported_output_tokens(),None,"{protocol:?}");
            decoder.decode_frame(frame(json!({"output_tokens":0}))).unwrap();
            assert_eq!(decoder.reported_output_tokens(),Some(0),"{protocol:?}");
            decoder.decode_frame(frame(json!({"output_tokens":123}))).unwrap();
            assert_eq!(decoder.reported_output_tokens(),Some(123),"{protocol:?}");
            decoder.decode_frame(frame(json!({"input_tokens":200}))).unwrap();
            assert_eq!(decoder.reported_output_tokens(),Some(123),"{protocol:?}");
        }
        for alias in ["output_tokens","completion_tokens","output","candidatesTokenCount"] {
            let data=json!({alias:37});
            assert_eq!(output_tokens_from_usage(&data),Some(37));
            assert_eq!(parse_usage(&data).output_tokens,37);
        }
        assert_eq!(output_tokens_from_usage(&json!({"input_tokens":100})),None);
        assert_eq!(parse_usage(&json!({"input_tokens":100})).output_tokens,0);
    }

    fn tool_input(schema: Value) -> ChatModelInput {
        let mut value = input();
        value.tool_choice = ChatToolChoice::Auto;
        value.tools = serde_json::from_value(json!([
            {"name":"probe","description":"probe tool","input_schema":schema}
        ]))
        .unwrap();
        value
    }

    #[test]
    fn anthropic_tool_schemas_drop_only_root_combinators() {
        let union = json!({
            "type":"object","additionalProperties":false,
            "properties":{"cmd":{"type":"string"},"command":{"type":"string"},
                "args":{"type":"array","items":{"type":"string"}}},
            "oneOf":[{"required":["cmd"],"not":{"required":["command"]}},
                {"required":["command"],"not":{"required":["cmd"]}}]
        });
        let mut expected = union.clone();
        expected.as_object_mut().unwrap().remove("oneOf");
        let body = encode_anthropic_request(&tool_input(union),"model").unwrap();
        assert_eq!(body["tools"][0]["input_schema"],expected);

        let conditional = json!({
            "type":"object","properties":{"format":{"type":"string"},
                "offset":{"type":"integer"}},"required":["format"],
            "allOf":[{"if":{"required":["start_line"]},"then":{"not":{"required":["offset"]}}}]
        });
        let mut expected = conditional.clone();
        expected.as_object_mut().unwrap().remove("allOf");
        let body = encode_anthropic_request(&tool_input(conditional),"model").unwrap();
        assert_eq!(body["tools"][0]["input_schema"],expected);

        let untouched = json!({
            "type":"object","properties":{"mode":{"oneOf":[{"const":"a"},{"const":"b"}]},
                "limit":{"type":"integer"}},"not":{"required":["limit","other"]},
            "if":{"required":["mode"]},"then":{"properties":{"limit":{"minimum":1}}},
            "else":{"properties":{"limit":{"maximum":1}}}
        });
        let body = encode_anthropic_request(&tool_input(untouched.clone()),"model").unwrap();
        assert_eq!(body["tools"][0]["input_schema"],untouched);
    }

    #[test]
    fn anthropic_tool_schemas_hoist_branch_fields_when_the_root_lists_none() {
        let union = json!({
            "type":"object",
            "oneOf":[{"type":"object","properties":{"a":{"type":"string"}},"required":["a","c"]},
                {"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"string"}},
                    "required":["a"]}]
        });
        let body = encode_anthropic_request(&tool_input(union),"model").unwrap();
        assert_eq!(body["tools"][0]["input_schema"],json!({
            "type":"object",
            "properties":{"a":{"anyOf":[{"type":"string"},{"type":"integer"}]},
                "b":{"type":"string"}},
            "required":["a"]
        }));

        let conjunction = json!({
            "type":"object",
            "allOf":[{"properties":{"a":{"type":"string"}},"required":["a"]},
                {"properties":{"b":{"type":"string"}},"required":["b"]}]
        });
        let body = encode_anthropic_request(&tool_input(conjunction),"model").unwrap();
        assert_eq!(body["tools"][0]["input_schema"],json!({
            "type":"object",
            "properties":{"a":{"type":"string"},"b":{"type":"string"}},
            "required":["a","b"]
        }));
    }

    #[test]
    fn only_the_anthropic_family_lowers_tool_schemas() {
        let union = json!({
            "type":"object","properties":{"cmd":{"type":"string"}},
            "oneOf":[{"required":["cmd"]},{"required":["command"]}]
        });
        for encode in [encode_bedrock_request as fn(&ChatModelInput,&str) -> _,
            encode_vertex_request] {
            let body = encode(&tool_input(union.clone()),"model").unwrap();
            assert!(body["tools"][0]["input_schema"].get("oneOf").is_none());
        }
        let body = encode_openai_chat_request(&tool_input(union.clone()),"model").unwrap();
        assert!(body["tools"][0]["function"]["parameters"].get("oneOf").is_some());
        let body = encode_openai_responses_request(&tool_input(union.clone()),"model").unwrap();
        assert!(body["tools"][0]["parameters"].get("oneOf").is_some());
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use crate::contracts::{ChatToolCall, ChatUsage};

    fn verification_input() -> ChatModelInput {
        serde_json::from_value(json!({
            "instructions":["verify the echo tool"],
            "messages":[
                {"role":"user","content":[{"type":"text","text":"call the tool"}]},
                {"role":"assistant","content":[
                    {"type":"provider_reasoning","block":{"kind":"anthropic_thinking","text":"thinking","signature":"sig"}},
                    {"type":"tool_call","call_id":"call_1","name":"nomifun_protocol_echo","arguments":{"value":"ok"}}
                ]},
                {"role":"tool","content":[{"type":"tool_result","call_id":"call_1",
                    "output":[{"type":"text","text":"{\"value\":\"ok\"}"}],"is_error":false}]}
            ],
            "tools":[{"name":"nomifun_protocol_echo","description":"echo",
                "input_schema":{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}}],
            "tool_choice":{"mode":"auto"},"max_output_tokens":512,
            "prompt_cache":"disabled","response_format":{"type":"text"}
        })).unwrap()
    }

    fn without_provider_reasoning(input: &mut ChatModelInput) {
        input.messages[1]
            .content
            .retain(|part| !matches!(part, ChatContentPart::ProviderReasoning { .. }));
    }

    #[test]
    fn verification_encoding_keeps_tools_and_tool_history_for_every_protocol() {
        for (protocol, anthropic_family) in [
            (ChatProtocol::Anthropic, true), (ChatProtocol::Bedrock, true), (ChatProtocol::Vertex, true),
            (ChatProtocol::OpenaiChat, false), (ChatProtocol::OpenaiResponses, false), (ChatProtocol::Gemini, false),
        ] {
            let mut input = verification_input();
            if !anthropic_family {
                // Anthropic signed thinking can replay only on Messages-family routes.
                without_provider_reasoning(&mut input);
            }
            let body = encode_verification_chat_input(protocol, "verified-model", &input, &json!({}), Some(2048)).unwrap();
            let wire = body.to_string();
            assert!(wire.contains("nomifun_protocol_echo"), "{protocol:?} lost the tool surface: {wire}");
            assert!(wire.contains("call_1"), "{protocol:?} lost the tool call/result history: {wire}");
            match protocol {
                ChatProtocol::Anthropic | ChatProtocol::Bedrock | ChatProtocol::Vertex => {
                    let content = body["messages"][1]["content"].as_array().unwrap();
                    assert_eq!(content[0]["type"], "thinking", "{protocol:?}: {wire}");
                    assert_eq!(content[0]["signature"], "sig", "{protocol:?}: {wire}");
                    assert_eq!(content[1]["type"], "tool_use", "{protocol:?}: {wire}");
                    assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result", "{protocol:?}: {wire}");
                    assert!(body.get("tools").is_some(), "{protocol:?}");
                }
                ChatProtocol::OpenaiChat => {
                    // Instructions serialize as a leading system message.
                    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "call_1", "{protocol:?}: {wire}");
                    assert_eq!(body["messages"][3]["role"], "tool", "{protocol:?}: {wire}");
                    assert_eq!(body["tools"][0]["function"]["name"], "nomifun_protocol_echo");
                    assert_eq!(body["tool_choice"], "auto");
                }
                ChatProtocol::OpenaiResponses => {
                    let input_items = body["input"].as_array().unwrap();
                    assert!(input_items.iter().any(|item| item["type"] == "function_call" && item["call_id"] == "call_1"), "{wire}");
                    assert!(input_items.iter().any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_1"), "{wire}");
                    assert_eq!(body["tools"][0]["name"], "nomifun_protocol_echo");
                }
                ChatProtocol::Gemini => {
                    let contents = body["contents"].as_array().unwrap();
                    assert!(
                        contents[1]["parts"].as_array().unwrap().iter()
                            .any(|part| part["functionCall"]["name"] == "nomifun_protocol_echo"),
                        "{wire}"
                    );
                    assert!(contents[2]["parts"].as_array().unwrap().iter()
                        .any(|part| part.get("functionResponse").is_some()), "{wire}");
                    assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "nomifun_protocol_echo");
                }
            }
            assert!(body.get("previous_response_id").is_none(), "{protocol:?}");
            assert!(body.get("conversation").is_none(), "{protocol:?}");
        }
    }

    #[test]
    fn verification_encoding_rejects_retained_round_state_and_non_text_parts() {
        for mutate in [
            |input: &mut ChatModelInput| input.provider_round_parent = Some("round-1".into()),
            |input: &mut ChatModelInput| input.messages[0].provider_round_id = Some("round-0".into()),
            |input: &mut ChatModelInput| input.preserve_native_responses_items = true,
            |input: &mut ChatModelInput| input.tool_choice = ChatToolChoice::Required,
            |input: &mut ChatModelInput| input.messages[0].content =
                vec![ChatContentPart::Image { media_type: "image/png".into(), data_base64: "AA==".into() }],
            |input: &mut ChatModelInput| input.messages[0].content =
                vec![ChatContentPart::Audio { media_type: "audio/wav".into(), data_base64: "AA==".into() }],
        ] {
            let mut input = verification_input();
            without_provider_reasoning(&mut input);
            mutate(&mut input);
            assert!(encode_verification_chat_input(ChatProtocol::OpenaiChat, "model", &input, &json!({}), None).is_err());
        }
        assert!(encode_verification_chat_input(ChatProtocol::OpenaiChat, "  ", &verification_input(), &json!({}), None).is_err());
    }

    #[test]
    fn verification_encoding_enforces_reasoning_protocol_compatibility() {
        let mut input = verification_input();
        without_provider_reasoning(&mut input);
        // Generic signed reasoning has no producing-route evidence anywhere.
        input.messages[1].content = vec![ChatContentPart::Reasoning {
            text: "thinking".into(), signature: Some("sig".into()), encrypted_content: None }];
        for protocol in [ChatProtocol::Anthropic, ChatProtocol::OpenaiResponses] {
            assert_eq!(
                encode_verification_chat_input(protocol, "model", &input, &json!({}), None).unwrap_err().code,
                ChatModelErrorCode::UnsupportedFeature
            );
        }
        // Opaque Responses ciphertext is OpenaiResponses-only.
        input.messages[1].content = vec![ChatContentPart::Reasoning {
            text: "thinking".into(), signature: None, encrypted_content: Some("cipher".into()) }];
        assert!(encode_verification_chat_input(ChatProtocol::OpenaiResponses, "model", &input, &json!({}), None).is_ok());
        assert_eq!(
            encode_verification_chat_input(ChatProtocol::Anthropic, "model", &input, &json!({}), None).unwrap_err().code,
            ChatModelErrorCode::UnsupportedFeature
        );
        // Provider reasoning blocks must match the protocol family.
        let responses_block = ChatContentPart::ProviderReasoning {
            block: ChatProviderReasoning::OpenaiResponsesReasoning {
                item: json!({"id":"rs_1","type":"reasoning","content":[],"summary":[],"encrypted_content":"c"}),
                route_digest: None,
            },
        };
        input.messages[1].content = vec![responses_block.clone()];
        assert!(encode_verification_chat_input(ChatProtocol::OpenaiResponses, "model", &input, &json!({}), None).is_ok());
        assert_eq!(
            encode_verification_chat_input(ChatProtocol::Anthropic, "model", &input, &json!({}), None).unwrap_err().code,
            ChatModelErrorCode::UnsupportedFeature
        );
    }

    #[test]
    fn verification_encoding_merges_provider_params_like_standalone() {
        let mut input = verification_input();
        without_provider_reasoning(&mut input);
        let body = encode_verification_chat_input(ChatProtocol::OpenaiChat, "model", &input,
            &json!({"max_tokens_field":"max_completion_tokens","temperature":0.2,"stream_options":{"include_usage":false}}),
            Some(4096)).unwrap();
        assert_eq!(body["max_completion_tokens"], 512);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["stream_options"]["include_usage"], false);
        let body = encode_verification_chat_input(ChatProtocol::OpenaiChat, "model", &input,
            &json!({"max_tokens_field":"max_completion_tokens"}), Some(256)).unwrap();
        assert_eq!(body["max_completion_tokens"], 256, "configured ceiling caps the requested output");
    }

    #[test]
    fn verification_provider_params_cannot_reinsert_round_state_or_replace_tools() {
        let mut input = verification_input();
        without_provider_reasoning(&mut input);
        for protocol in [ChatProtocol::OpenaiChat, ChatProtocol::OpenaiResponses, ChatProtocol::Anthropic] {
            let body = encode_verification_chat_input(protocol, "model", &input,
                &json!({"previous_response_id":"round-x","conversation":"conv-1",
                    "tools":[{"name":"injected"}],"tool_choice":"required","parallel_tool_calls":false}),
                None).unwrap();
            assert!(body.get("previous_response_id").is_none(), "{protocol:?}");
            assert!(body.get("conversation").is_none(), "{protocol:?}");
            let wire = body.to_string();
            assert!(wire.contains("nomifun_protocol_echo"), "{protocol:?} lost encoder tools: {wire}");
            assert!(!wire.contains("injected"), "{protocol:?} took provider-param tools: {wire}");
        }
    }

    struct StubDecoder {
        batches: std::collections::VecDeque<Result<Vec<ChatModelEvent>, ChatModelError>>,
    }

    impl ChatFrameDecoder for StubDecoder {
        fn decode_frame(&mut self, _frame: ProviderWireFrame) -> Result<Vec<ChatModelEvent>, ChatModelError> {
            self.batches.pop_front().unwrap_or_else(|| Ok(Vec::new()))
        }
    }

    fn verification_decoder(batches: Vec<Result<Vec<ChatModelEvent>, ChatModelError>>) -> VerificationFrameDecoder {
        VerificationFrameDecoder {
            inner: Box::new(StubDecoder { batches: batches.into() }),
            sequence: crate::broker::EventSequence::default(),
        }
    }

    fn frame() -> ProviderWireFrame {
        ProviderWireFrame { event: "message".into(), data: json!({}), diagnostic: None }
    }

    fn usage() -> ChatModelEvent {
        ChatModelEvent::Usage { usage: ChatUsage::default() }
    }

    #[test]
    fn verification_decoder_applies_broker_sequence_rules() {
        // Duplicate canonical usage is rejected exactly like broker consumption.
        let mut decoder = verification_decoder(vec![Ok(vec![usage()]), Ok(vec![usage()])]);
        decoder.decode_frame(frame()).unwrap();
        let error = decoder.decode_frame(frame()).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
        // Any event after the canonical terminal is rejected.
        let mut decoder = verification_decoder(vec![
            Ok(vec![ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed }]),
            Ok(vec![ChatModelEvent::OutputTextDelta { text: "late".into(), source: None }]),
        ]);
        decoder.decode_frame(frame()).unwrap();
        let error = decoder.decode_frame(frame()).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
        // Provider reasoning blocks pass through block.validate().
        let mut decoder = verification_decoder(vec![Ok(vec![ChatModelEvent::ProviderReasoningBlock {
            block: ChatProviderReasoning::AnthropicThinking { text: "t".into(), signature: String::new(), route_digest: None },
        }])]);
        let error = decoder.decode_frame(frame()).unwrap_err();
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
    }

    #[test]
    fn verification_decoder_passes_valid_streams_and_delegates() {
        let call = ChatToolCall {
            call_id: ToolCallId("call_1".into()),
            name: "nomifun_protocol_echo".into(),
            arguments: StrictJsonValue(json!({"value":"ok"})),
            provider_metadata: None,
        };
        let mut decoder = new_verification_frame_decoder(ChatProtocol::OpenaiChat);
        let events = decoder
            .decode_frame(ProviderWireFrame {
                event: "message".into(),
                data: json!({"id":"r1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1",
                    "type":"function","function":{"name":"nomifun_protocol_echo","arguments":"{\"value\":\"ok\"}"}}]},
                    "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":1}}),
                diagnostic: None,
            })
            .unwrap();
        assert!(events.iter().any(|event| matches!(event,
            ChatModelEvent::ToolCallCompleted { call: completed } if completed == &call)));
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })));
    }
}

fn insert_if_some(object: &mut Value, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        object[key] = value;
    }
}

fn combined_system_text(instructions: &[String], messages: &[ChatMessage]) -> String {
    instructions
        .iter()
        .map(String::as_str)
        .chain(messages.iter().flat_map(|message| {
            message.content.iter().filter_map(|part| {
                if !matches!(message.role, ChatRole::System) {
                    return None;
                }
                match part {
                    ChatContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                }
            })
        }))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn anthropic_messages(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter(|message| !matches!(message.role, ChatRole::System))
        .map(|message| {
            json!({
                "role": match message.role {
                    ChatRole::User | ChatRole::Tool => "user",
                    ChatRole::Assistant => "assistant",
                    ChatRole::System => "user",
                },
                "content": anthropic_content(&message.content),
            })
        })
        .collect()
}

fn anthropic_content(content: &[ChatContentPart]) -> Vec<Value> {
    content
        .iter()
        .map(|part| match part {
            ChatContentPart::Text { text } => json!({"type": "text", "text": text}),
            ChatContentPart::Image {
                media_type,
                data_base64,
            } => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": media_type,
                    "data": data_base64,
                }
            }),
            ChatContentPart::Audio {
                media_type,
                data_base64,
            } => json!({
                "type": "input_audio",
                "media_type": media_type,
                "data": data_base64,
            }),
            ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": arguments.0.clone(),
            }),
            ChatContentPart::ToolResult {
                call_id,
                output,
                is_error,
            } => json!({
                "type": "tool_result",
                "tool_use_id": call_id,
                "content": anthropic_tool_result_content(output),
                "is_error": is_error,
            }),
            ChatContentPart::Reasoning {
                text,
                signature,
                ..
            } => json!({
                "type": "thinking",
                "thinking": text,
                "signature": signature,
            }),
            ChatContentPart::ProviderReasoning { block } => match block {
                crate::ChatProviderReasoning::AnthropicThinking { text, signature, .. } =>
                    json!({"type":"thinking", "thinking":text, "signature":signature}),
                crate::ChatProviderReasoning::AnthropicRedactedThinking { data, .. } =>
                    json!({"type":"redacted_thinking", "data":data}),
                // Exact route validation rejects this before transport.
                crate::ChatProviderReasoning::OpenaiResponsesReasoning { .. } => Value::Null,
            },
        })
        .collect()
}

fn openai_messages(message: &ChatMessage) -> Vec<Value> {
    if matches!(message.role, ChatRole::Tool) {
        return message
            .content
            .iter()
            .filter_map(|part| match part {
                ChatContentPart::ToolResult {
                    call_id,
                    output,
                    is_error,
                } => Some(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": openai_tool_result_content(output, *is_error),
                })),
                _ => None,
            })
            .collect();
    }

    let role = match message.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    };
    let mut value = json!({
        "role": role,
        "content": openai_content(&message.content, message.role == ChatRole::Assistant),
    });
    let tool_calls: Vec<Value> = message
        .content
        .iter()
        .filter_map(|part| match part {
            ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => Some(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&arguments.0).unwrap_or_default(),
                },
            })),
            _ => None,
        })
        .collect();
    if !tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(tool_calls);
        if value["content"].as_array().is_some_and(Vec::is_empty) {
            value["content"] = Value::Null;
        }
    }
    if message.role == ChatRole::Assistant
        && value["content"].as_array().is_some_and(Vec::is_empty)
        && value.get("tool_calls").is_none() {
        return vec![];
    }
    vec![value]
}

fn openai_content(content: &[ChatContentPart], assistant: bool) -> Value {
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|part| match part {
            ChatContentPart::Text { text } if assistant && text == "[Private reasoning omitted from replay]" => None,
            ChatContentPart::Text { text } => Some(json!({"type": "text", "text": text})),
            ChatContentPart::Image {
                media_type,
                data_base64,
            } => Some(json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{media_type};base64,{data_base64}")}
            })),
            ChatContentPart::Audio {
                media_type,
                data_base64,
            } => Some(json!({
                "type": "input_audio",
                "input_audio": {"format": openai_audio_format(media_type), "data": data_base64}
            })),
            ChatContentPart::Reasoning { .. } => {
                // Chat text parts have no private-reasoning role. A sole
                // part is flattened below, and mixed parts are ordinary
                // assistant content too. Never turn thinking/pretend calls
                // into public factual history on this compatible protocol.
                None
            }
            // AdapterCore rejects this request before transport; never turn
            // opaque provider state into ordinary visible text.
            ChatContentPart::ProviderReasoning { .. }
            | ChatContentPart::ToolCall { .. } | ChatContentPart::ToolResult { .. } => None,
        })
        .collect();
    if let [part] = parts.as_slice()
        && part.get("type").and_then(Value::as_str) == Some("text")
        && let Some(text) = part.get("text").cloned()
    {
        return text;
    }
    Value::Array(parts)
}

fn openai_tool_result_content(parts: &[ChatToolResultPart], is_error: bool) -> Value {
    // Chat tool messages accept text only. Multimodal tool outputs are sent
    // after all responses in the tool batch as a user attachment message.
    let text = parts.iter().filter_map(|part| match part {
        ChatToolResultPart::Text { text } => Some(text.as_str()),
        _ => None,
    }).collect::<Vec<_>>().join("\n");
    let text = if is_error { format!("[tool error]\n{text}") } else { text };
    Value::String(text)
}

fn openai_tool_result_media(parts: &[ChatToolResultPart]) -> Vec<Value> {
    let content = parts
        .iter()
        .filter_map(|part| match part {
            ChatToolResultPart::Text { .. } => None,
            ChatToolResultPart::Image {
                media_type,
                data_base64,
            } => Some(json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{media_type};base64,{data_base64}")
                }
            })),
            ChatToolResultPart::Audio {
                media_type,
                data_base64,
            } => Some(json!({
                "type": "input_audio",
                "input_audio": {
                    "format": openai_audio_format(media_type),
                    "data": data_base64
                }
            })),
        })
        .collect::<Vec<_>>();
    content
}

fn openai_audio_format(media_type: &str) -> &str {
    match media_type {
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        value => value,
    }
}

fn responses_items(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .flat_map(|message| {
            let exact_null_reasoning = message.content.iter().any(|part| matches!(part,
                ChatContentPart::ProviderReasoning {block:crate::ChatProviderReasoning::OpenaiResponsesReasoning {item,..}}
                    if item.get("content").is_some_and(Value::is_null)));
            message.content.iter().filter(move |part| !exact_null_reasoning || !matches!(part,
                ChatContentPart::Reasoning {signature:None,encrypted_content:None,..}))
            .map(move |part| match part {
                ChatContentPart::Text { text } => json!({
                    "type": "message",
                    "role": match message.role {
                        ChatRole::Assistant => "assistant",
                        ChatRole::System => "system",
                        ChatRole::Tool | ChatRole::User => "user",
                    },
                    "content": [{
                        "type": match message.role {
                            ChatRole::Assistant => "output_text",
                            ChatRole::System | ChatRole::Tool | ChatRole::User => "input_text",
                        },
                        "text": text
                    }],
                }),
                ChatContentPart::Image {
                    media_type,
                    data_base64,
                } => json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_image",
                        "image_url": format!("data:{media_type};base64,{data_base64}")
                    }]
                }),
                ChatContentPart::Audio {
                    media_type,
                    data_base64,
                } => json!({
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "audio": {"format": media_type, "data": data_base64}
                    }]
                }),
                ChatContentPart::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": serde_json::to_string(&arguments.0).unwrap_or_default(),
                }),
                ChatContentPart::ToolResult {
                    call_id,
                    output,
                    is_error,
                } => json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": responses_tool_output(output, *is_error),
                }),
                ChatContentPart::Reasoning {
                    text,
                    signature,
                    encrypted_content,
                } => {
                    let opaque_content = encrypted_content.as_ref().or(signature.as_ref());
                    json!({
                        "type": "reasoning",
                        "summary": if text.is_empty() { Vec::<Value>::new() } else { vec![json!({"type": "summary_text", "text": text})] },
                        "encrypted_content": opaque_content,
                    })
                }
                // AdapterCore rejects typed foreign reasoning before using
                // this body. No Anthropic signature -> encrypted_content cast.
                ChatContentPart::ProviderReasoning { block } => match block {
                    crate::ChatProviderReasoning::OpenaiResponsesReasoning { item, .. } => item.clone(),
                    _ => Value::Null,
                },
            })
        })
        .collect()
}

fn responses_tool_output(output: &[ChatToolResultPart], is_error: bool) -> Value {
    let has_media = output.iter().any(|part| !matches!(part, ChatToolResultPart::Text { .. }));
    if !has_media {
        let text = tool_result_string(output);
        return Value::String(if is_error { format!("Tool reported failure:\n{text}") } else { text });
    }
    let mut parts = Vec::new();
    if is_error { parts.push(json!({"type":"input_text", "text":"Tool reported failure; the following output is an observation, not success."})); }
    parts.extend(output.iter().map(|part| match part {
        ChatToolResultPart::Text { text } => json!({"type":"input_text", "text":text}),
        ChatToolResultPart::Image { media_type, data_base64 } => json!({
            "type":"input_image", "image_url":format!("data:{media_type};base64,{data_base64}")
        }),
        ChatToolResultPart::Audio { media_type, data_base64 } => json!({
            "type":"input_audio", "audio_url":format!("data:{media_type};base64,{data_base64}")
        }),
    }));
    Value::Array(parts)
}

fn gemini_contents(
    messages: &[ChatMessage],
    call_names: &BTreeMap<ToolCallId, String>,
) -> Result<Vec<Value>, ChatModelError> {
    messages
        .iter()
        .filter(|message| !matches!(message.role, ChatRole::System))
        .map(|message| {
            let role = match message.role {
                ChatRole::Assistant => "model",
                ChatRole::User | ChatRole::Tool => "user",
                ChatRole::System => "user",
            };
            let parts = message
                .content
                .iter()
                .map(|part| gemini_part(part, call_names))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({
                "role": role,
                "parts": parts
            }))
        })
        .collect()
}

fn gemini_part(
    part: &ChatContentPart,
    call_names: &BTreeMap<ToolCallId, String>,
) -> Result<Value, ChatModelError> {
    match part {
        ChatContentPart::Text { text } => Ok(json!({"text": text})),
        ChatContentPart::Image {
            media_type,
            data_base64,
        } => Ok(json!({"inlineData": {"mimeType": media_type, "data": data_base64}})),
        ChatContentPart::Audio {
            media_type,
            data_base64,
        } => Ok(json!({"inlineData": {"mimeType": media_type, "data": data_base64}})),
        ChatContentPart::ToolCall {
            call_id,
            name,
            arguments,
            provider_metadata,
        } => {
            let mut part = json!({"functionCall": {
                "id": call_id,
                "name": name,
                "args": arguments.0.clone()
            }});
            if let Some(signature) = gemini_tool_signature(provider_metadata)? {
                part["thoughtSignature"] = json!(signature);
            }
            Ok(part)
        }
        ChatContentPart::ToolResult {
            call_id,
            output,
            is_error,
        } => {
            let name = call_names.get(call_id).ok_or_else(|| {
                ChatModelError::invalid_request(
                    "Gemini function response has no matching function call",
                )
            })?;
            Ok(json!({"functionResponse": {
                "id": call_id,
                "name": name,
                "response": {
                    "content": tool_result_string(output),
                    "is_error": is_error,
                }
            }}))
        }
        ChatContentPart::Reasoning { text, .. } => Ok(json!({"text": text, "thought": true})),
        ChatContentPart::ProviderReasoning { .. } => Err(ChatModelError::new(
            ChatModelErrorCode::UnsupportedFeature,
            "typed Anthropic reasoning cannot be encoded as Gemini content",
            crate::contracts::ChatRetryDirective::Never,
        )),
    }
}

fn anthropic_tool_result_content(parts: &[ChatToolResultPart]) -> Value {
    Value::Array(
        parts
            .iter()
            .map(|part| match part {
                ChatToolResultPart::Text { text } => {
                    json!({"type": "text", "text": text})
                }
                ChatToolResultPart::Image {
                    media_type,
                    data_base64,
                } => json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data_base64
                    }
                }),
                ChatToolResultPart::Audio {
                    media_type,
                    data_base64,
                } => json!({
                    "type": "input_audio",
                    "media_type": media_type,
                    "data": data_base64
                }),
            })
            .collect(),
    )
}

fn tool_result_string(parts: &[ChatToolResultPart]) -> String {
    if parts
        .iter()
        .all(|part| matches!(part, ChatToolResultPart::Text { .. }))
    {
        return parts
            .iter()
            .filter_map(|part| match part {
                ChatToolResultPart::Text { text } => Some(text.as_str()),
                ChatToolResultPart::Image { .. } | ChatToolResultPart::Audio { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    serde_json::to_string(parts).unwrap_or_default()
}

fn anthropic_tool_choice(choice: &ChatToolChoice) -> Value {
    match choice {
        ChatToolChoice::Auto => json!({"type": "auto"}),
        ChatToolChoice::Required => json!({"type": "any"}),
        ChatToolChoice::Specific { name } => json!({"type": "tool", "name": name}),
        ChatToolChoice::None => Value::Null,
    }
}

fn openai_tool_choice(choice: &ChatToolChoice) -> Value {
    match choice {
        ChatToolChoice::Auto => Value::String("auto".to_owned()),
        ChatToolChoice::Required => Value::String("required".to_owned()),
        ChatToolChoice::Specific { name } => json!({
            "type": "function",
            "function": {"name": name}
        }),
        ChatToolChoice::None => Value::String("none".to_owned()),
    }
}

fn openai_responses_tool_choice(choice: &ChatToolChoice) -> Value {
    match choice {
        ChatToolChoice::Auto => Value::String("auto".to_owned()),
        ChatToolChoice::Required => Value::String("required".to_owned()),
        ChatToolChoice::Specific { name } => json!({
            "type": "function",
            "name": name
        }),
        ChatToolChoice::None => Value::String("none".to_owned()),
    }
}

fn gemini_tool_config(choice: &ChatToolChoice) -> Value {
    let function_calling_config = match choice {
        ChatToolChoice::Auto => json!({"mode": "AUTO"}),
        ChatToolChoice::None => json!({"mode": "NONE"}),
        ChatToolChoice::Required => json!({"mode": "ANY"}),
        ChatToolChoice::Specific { name } => json!({
            "mode": "ANY",
            "allowedFunctionNames": [name]
        }),
    };
    json!({"functionCallingConfig": function_calling_config})
}

fn response_format_value(format: &ChatResponseFormat) -> Value {
    match format {
        ChatResponseFormat::Text => json!({"type": "text"}),
        ChatResponseFormat::JsonObject => json!({"type": "json_object"}),
        ChatResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => json!({
            "type": "json_schema",
            "json_schema": {
                "name": name,
                "schema": schema.0.clone(),
                "strict": strict,
            }
        }),
    }
}

fn responses_format_value(format: &ChatResponseFormat) -> Value {
    match format {
        ChatResponseFormat::Text => json!({"type": "text"}),
        ChatResponseFormat::JsonObject => json!({"type": "json_object"}),
        ChatResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => json!({
            "type": "json_schema",
            "name": name,
            "schema": schema.0.clone(),
            "strict": strict,
        }),
    }
}

struct AttemptFrameDecoder {
    protocol: ChatProtocol,
    reported_output_tokens: Option<u64>,
    raw_decode_state: Arc<Mutex<RawDecodeState>>,
    responses: crate::responses_decoder::ResponsesDecoder,
    anthropic: crate::anthropic_decoder::AnthropicDecoder,
    argument_parser: Arc<dyn ToolArgumentParser>,
}

impl ChatFrameDecoder for AttemptFrameDecoder {
    fn reported_output_tokens(&self) -> Option<u64> {
        self.reported_output_tokens
    }

    fn has_deferred_semantic_progress(&self) -> bool {
        match self.protocol {
            ChatProtocol::OpenaiResponses => self.responses.has_deferred_semantic_progress(),
            protocol if protocol.uses_anthropic_messages() => self.anthropic.has_deferred_semantic_progress(),
            _ => false,
        }
    }

    fn decode_frame(
        &mut self,
        frame: ProviderWireFrame,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let source = match self.protocol {
            ChatProtocol::OpenaiResponses => frame.data.get("response").unwrap_or(&frame.data),
            protocol if protocol.uses_anthropic_messages() => frame.data.get("message").unwrap_or(&frame.data),
            ChatProtocol::Gemini => frame.data.get("usageMetadata").unwrap_or(&frame.data),
            _ => &frame.data,
        };
        let reported = output_tokens_from_usage(source);
        let result = self.decode_events(frame);
        if result.is_ok() && let Some(reported) = reported {
            self.reported_output_tokens = Some(reported);
        }
        result
    }
}

impl AttemptFrameDecoder {
    fn decode_events(&mut self, frame: ProviderWireFrame) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        // Cloud exception headers are normalized outside Messages. Handle them
        // even after the native decoder has already selected a content lifecycle.
        if let Some(error) = crate::provider_errors::decode_with_context(
            self.protocol, &frame.event.trim().to_ascii_lowercase(), &frame.data,
            frame.diagnostic.as_ref(),
        ) {
            return Err(error);
        }
        if self.protocol == ChatProtocol::OpenaiResponses {
            if let Some(events) = self.responses.decode(&frame) {
                return events;
            }
        }
        if self.protocol.uses_anthropic_messages() {
            if let Some(events) = self.anthropic.decode(&frame) {
                return events;
            }
        }
        decode_frame_for_protocol(self.protocol, frame, &self.raw_decode_state, self.argument_parser.as_ref())
    }
}

fn decode_frame_for_protocol(
    protocol: ChatProtocol,
    frame: ProviderWireFrame,
    raw_decode_state: &Arc<Mutex<RawDecodeState>>,
    argument_parser: &dyn ToolArgumentParser,
) -> Result<Vec<ChatModelEvent>, ChatModelError> {
    let event_name = frame.event.trim().to_ascii_lowercase();
    if event_name.is_empty() {
        return Err(ChatModelError::protocol_violation(
            "provider frame has an empty event name",
        ));
    }
    if let Some(error) = crate::provider_errors::decode_with_context(
        protocol, &event_name, &frame.data, frame.diagnostic.as_ref(),
    ) {
        return Err(error);
    }

    if protocol == ChatProtocol::OpenaiChat
        && (frame.data.get("choices").is_some()
            || (event_name == "done"
                && raw_decode_state
                    .lock()
                    .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?
                    .openai
                    .values()
                    .any(|state| state.response_started))
            || (is_terminal_frame(&frame.data)
                && raw_decode_state
                    .lock()
                    .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?
                    .openai
                    .values()
                    .any(|state| state.response_started)))
    {
        return decode_openai_chat_raw_frame(frame, raw_decode_state, argument_parser);
    }
    if protocol == ChatProtocol::OpenaiChat
        && event_name == "done"
        && frame.data.as_object().is_some_and(Map::is_empty)
    {
        return Ok(Vec::new());
    }
    if protocol == ChatProtocol::Gemini
        && (frame.data.get("candidates").is_some()
            || frame.data.get("usageMetadata").is_some()
            || frame
                .data
                .get("promptFeedback")
                .and_then(|feedback| feedback.get("blockReason"))
                .and_then(Value::as_str)
                .is_some()
            || (event_name == "done"
                && raw_decode_state
                    .lock()
                    .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?
                    .gemini
                    .values()
                    .any(|state| state.response_started))
            || (is_terminal_frame(&frame.data)
                && raw_decode_state
                    .lock()
                    .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?
                    .gemini
                    .values()
                    .any(|state| state.response_started)))
    {
        return decode_gemini_raw_frame(frame, raw_decode_state, argument_parser);
    }
    if protocol == ChatProtocol::Gemini
        && event_name == "done"
        && frame.data.as_object().is_some_and(Map::is_empty)
    {
        return Ok(Vec::new());
    }

    let mut events = Vec::new();
    match event_name.as_str() {
        "response.created"
        | "message_start"
        | "response.start"
        | "stream.start"
        | "start" => {
            let id = string_at(&frame.data, &["id", "response.id", "message.id"])
                .map(ProviderResponseId);
            events.push(ChatModelEvent::ResponseStarted {
                provider_response_id: id,
            });
        }
        "text.delta"
        | "output_text.delta"
        | "content_block_delta"
        | "message.delta"
        | "response.output_text.delta" => {
            if let Some(text) = text_delta(&frame.data) {
                events.push(ChatModelEvent::OutputTextDelta { text, source: None });
            }
            if let Some(reasoning) = reasoning_delta(&frame.data) {
                events.push(ChatModelEvent::ReasoningDelta { text: reasoning });
            }
            if let Some(signature) = string_at(
                &frame.data,
                &["signature", "delta.signature", "reasoning.signature"],
            ) {
                events.push(ChatModelEvent::ReasoningSignature { signature });
            }
        }
        "audio.delta" | "response.audio.delta" | "response.output_audio.delta" => {
            let data_base64 = string_at(
                &frame.data,
                &["data_base64", "delta", "audio.delta", "audio.data"],
            )
            .ok_or_else(|| {
                ChatModelError::protocol_violation(
                    "provider audio delta is missing base64 data",
                )
            })?;
            let media_type = string_at(
                &frame.data,
                &["media_type", "audio.media_type", "format"],
            )
            .unwrap_or_else(|| "audio/pcm".to_owned());
            events.push(ChatModelEvent::OutputAudioDelta {
                media_type,
                data_base64,
            });
        }
        "reasoning.delta"
        | "thinking.delta"
        | "response.reasoning_summary_text.delta" => {
            if let Some(text) = text_delta(&frame.data) {
                events.push(ChatModelEvent::ReasoningDelta { text });
            }
        }
        "reasoning.signature" | "thinking.signature" | "signature.delta" => {
            if let Some(signature) =
                string_at(&frame.data, &["signature", "delta.signature", "encrypted_content"])
            {
                events.push(ChatModelEvent::ReasoningSignature { signature });
            }
        }
        "tool_call.delta"
        | "function_call.delta"
        | "response.function_call_arguments.delta"
        | "content_block_tool_delta" => {
            let call_id = string_at(
                &frame.data,
                &["call_id", "id", "tool_call_id", "delta.id", "tool.id"],
            )
            .ok_or_else(|| {
                ChatModelError::protocol_violation("tool-call delta is missing call id")
            })?;
            let name = string_at(&frame.data, &["name", "function.name", "delta.name"])
                .unwrap_or_default();
            let arguments_delta = string_at(
                &frame.data,
                &["arguments_delta", "arguments", "delta.arguments", "function.arguments"],
            )
            .unwrap_or_default();
            events.push(ChatModelEvent::ToolCallDelta {
                call_id: ToolCallId(call_id),
                name,
                arguments_delta,
            });
        }
        "tool_call.completed"
        | "function_call.completed"
        | "response.function_call.done"
        | "content_block_tool_done" => {
            events.push(parse_tool_call(&frame.data, argument_parser)?);
        }
        "response.output_item.added" | "output_item.added" | "native.item" => {
            let item_type = string_at(&frame.data, &["item.type", "type", "output.type"])
                .unwrap_or_else(|| "unknown".to_owned());
            let item = frame
                .data
                .get("item")
                .or_else(|| frame.data.get("output"))
                .cloned()
                .unwrap_or(frame.data.clone());
            events.push(ChatModelEvent::NativeResponsesItem {
                item_type,
                item: StrictJsonValue(item),
            });
        }
        "provider.round" | "response.completed.round" => {
            let round_id = string_at(&frame.data, &["round_id", "response.id", "id"])
                .ok_or_else(|| ChatModelError::protocol_violation("round frame is missing id"))?;
            events.push(ChatModelEvent::ProviderRoundId {
                round_id: ProviderRoundId(round_id),
            });
        }
        "usage" | "response.usage" | "message.usage" | "response.completed.usage" => {
            events.push(ChatModelEvent::Usage {
                usage: parse_usage(&frame.data),
            });
        }
        "response.completed"
        | "message_stop"
        | "stream.end"
        | "done"
        | "finish"
        | "response.incomplete" => {
            let finish_reason = parse_finish_reason(protocol, &frame.data, &event_name);
            events.push(ChatModelEvent::Completed { finish_reason });
        }
        _ => {
            if let Some(text) = text_delta(&frame.data) {
                events.push(ChatModelEvent::OutputTextDelta { text, source: None });
            } else if let Some(usage) = usage_from_nested(&frame.data) {
                events.push(ChatModelEvent::Usage { usage });
            } else if is_terminal_frame(&frame.data) {
                events.push(ChatModelEvent::Completed {
                    finish_reason: parse_finish_reason(protocol, &frame.data, &event_name),
                });
            } else {
                return Err(ChatModelError::protocol_violation(format!(
                    "unsupported {:?} provider event `{}`",
                    protocol, frame.event
                )));
            }
        }
    }
    Ok(events)
}

#[derive(Default)]
struct RawDecodeState {
    openai: BTreeMap<String, OpenAiRawStreamState>,
    gemini: BTreeMap<String, GeminiRawStreamState>,
}

#[derive(Default)]
struct OpenAiRawStreamState {
    response_started: bool,
    tools: BTreeMap<u64, RawToolCallState>,
    usage_seen: bool,
    pending_usage: Option<ChatUsage>,
    pending_finish: Option<ChatFinishReason>,
}

#[derive(Default)]
struct GeminiRawStreamState {
    response_started: bool,
    has_tool_calls: bool,
    pending_usage: Option<ChatUsage>,
    tools: BTreeMap<ToolCallId, RawGeminiToolCall>,
}

struct RawGeminiToolCall {
    name: String,
    arguments: Value,
    provider_metadata: Option<StrictJsonValue>,
}

#[derive(Default)]
struct RawToolCallState {
    call_id: String,
    name: String,
    arguments: String,
    emitted_arguments: usize,
}

const OPENAI_ANONYMOUS_STREAM: &str = "__openai_anonymous__";

fn decode_openai_chat_raw_frame(
    frame: ProviderWireFrame,
    raw_decode_state: &Arc<Mutex<RawDecodeState>>,
    argument_parser: &dyn ToolArgumentParser,
) -> Result<Vec<ChatModelEvent>, ChatModelError> {
    let mut state = raw_decode_state
        .lock()
        .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?;
    let explicit_response_key = frame
        .data
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    // One attempt carries one Chat Completions response. Azure-style gateways
    // open it with an id-less prelude (`id: ""`, `choices: []`, e.g.
    // prompt_filter_results) and may interleave id-less chunks; id-less frames
    // already join the single open stream, and symmetrically the first
    // identified frame adopts a single anonymous stream instead of opening a
    // second response. A frame whose id contradicts an already identified
    // stream still opens a separate stream, which the event-sequence rules
    // reject.
    if let Some(response_key) = explicit_response_key.as_deref()
        && state.openai.len() == 1
        && let Some(anonymous) = state.openai.remove(OPENAI_ANONYMOUS_STREAM)
    {
        state.openai.insert(response_key.to_owned(), anonymous);
    }
    let response_key = match explicit_response_key.clone() {
        Some(response_key) => response_key,
        None if state.openai.len() <= 1 => state
            .openai
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| OPENAI_ANONYMOUS_STREAM.to_owned()),
        None => {
            return Err(ChatModelError::protocol_violation(
                "OpenAI Chat frame without response id cannot be correlated with multiple active streams",
            ));
        }
    };
    let stream = state.openai.entry(response_key.clone()).or_default();
    let mut events = Vec::new();
    if !stream.response_started {
        events.push(ChatModelEvent::ResponseStarted {
            provider_response_id: explicit_response_key.map(ProviderResponseId),
        });
        stream.response_started = true;
    }

    let choices = match frame.data.get("choices") {
        Some(value) => Some(value.as_array().ok_or_else(|| {
            ChatModelError::protocol_violation("OpenAI Chat choices is not an array")
        })?),
        None => None,
    };
    if let Some(choices) = choices {
        if choices.len() > 1 {
            return Err(ChatModelError::protocol_violation(
                "OpenAI Chat streamed response contains more than one choice",
            ));
        }
        if let Some(choice) = choices.first() {
            let choice = choice.as_object().ok_or_else(|| {
                ChatModelError::protocol_violation("OpenAI Chat choice is not an object")
            })?;
            let finish_reason = match choice.get("finish_reason") {
                None | Some(Value::Null) => None,
                Some(Value::String(reason)) if !reason.is_empty() => Some(reason.as_str()),
                Some(Value::String(_)) => None,
                Some(_) => {
                    return Err(ChatModelError::protocol_violation(
                        "OpenAI Chat finish_reason is not a string or null",
                    ));
                }
            };
            let delta = choice
                .get("delta")
                .or_else(|| choice.get("message"))
                .filter(|value| !value.is_null());
            let delta = match delta {
                Some(value) => Some(value.as_object().ok_or_else(|| {
                    ChatModelError::protocol_violation(
                        "OpenAI Chat choice delta is not an object",
                    )
                })?),
                None if finish_reason.is_some() => None,
                None => {
                    return Err(ChatModelError::protocol_violation(
                        "OpenAI Chat choice is missing an object delta",
                    ));
                }
            };

            if let Some(delta) = delta {
                if let Some(text) = delta.get("content").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    events.push(ChatModelEvent::OutputTextDelta {
                        text: text.to_owned(),
                        source: None,
                    });
                }
                if let Some(reasoning) = extract_openai_raw_reasoning(delta) {
                    events.push(ChatModelEvent::ReasoningDelta { text: reasoning });
                }

                let tool_calls = delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| calls.iter().collect::<Vec<_>>());
                let tool_calls = tool_calls.or_else(|| {
                    delta
                        .get("function_call")
                        .filter(|value| value.is_object())
                        .map(|call| vec![call])
                });
                if let Some(tool_calls) = tool_calls {
                    for (position, tool_call) in tool_calls.into_iter().enumerate() {
                        let tool_call = tool_call.as_object().ok_or_else(|| {
                            ChatModelError::protocol_violation(
                                "OpenAI Chat tool call is not an object",
                            )
                        })?;
                        let index = tool_call
                            .get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or(position as u64);
                        let function = tool_call
                            .get("function")
                            .and_then(Value::as_object)
                            .unwrap_or(tool_call);
                        let entry = stream.tools.entry(index).or_default();
                        if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                            if !id.is_empty() {
                                entry.call_id = id.to_owned();
                            }
                        }
                        if entry.call_id.is_empty() {
                            entry.call_id = format!("openai-tool-{index}");
                        }
                        if let Some(name) = function.get("name").and_then(Value::as_str)
                            && !name.is_empty()
                        {
                            entry.name = name.to_owned();
                        }
                        if let Some(arguments) = function.get("arguments") {
                            append_raw_arguments(&mut entry.arguments, arguments);
                        }
                        if entry.name.is_empty() {
                            continue;
                        }
                        let arguments_delta =
                            entry.arguments[entry.emitted_arguments..].to_owned();
                        entry.emitted_arguments = entry.arguments.len();
                        events.push(ChatModelEvent::ToolCallDelta {
                            call_id: ToolCallId(entry.call_id.clone()),
                            name: entry.name.clone(),
                            arguments_delta,
                        });
                    }
                }
            }

            if let Some(reason) = finish_reason {
                stream.pending_finish = Some(parse_finish_reason_text(reason));
            }
        }
    }

    let raw_usage = frame
        .data
        .get("usage")
        .filter(|value| !value.is_null())
        .map(parse_usage);
    if let Some(usage) = raw_usage {
        stream.pending_usage = Some(usage);
    }

    let done_marker = frame.event.eq_ignore_ascii_case("done");
    if let Some(value) = frame.data.get("finish_reason") {
        match value {
            Value::Null => {}
            Value::String(reason) if !reason.is_empty() => {
                stream.pending_finish = Some(parse_finish_reason_text(reason));
            }
            Value::String(_) => {}
            _ => {
                return Err(ChatModelError::protocol_violation(
                    "OpenAI Chat finish_reason is not a string or null",
                ));
            }
        }
    }
    let choices_are_empty = choices.is_some_and(|choices| choices.is_empty());
    let json_response = frame.event.eq_ignore_ascii_case("json");
    let should_complete = done_marker
        || json_response
        || is_terminal_frame(&frame.data)
        || (choices_are_empty && stream.pending_finish.is_some())
        || (stream.pending_finish.is_some()
            && frame.data.get("usage").is_some_and(|value| !value.is_null()));
    if should_complete {
        let finish_reason = stream.pending_finish.take().unwrap_or_else(|| {
            if stream.tools.is_empty() { ChatFinishReason::Completed } else { ChatFinishReason::ToolCalls }
        });
        // A length/refusal terminal cannot complete a partially generated
        // function. Engines may discard the proposed batch, never execute it.
        if matches!(finish_reason, ChatFinishReason::Completed | ChatFinishReason::ToolCalls) {
            append_openai_raw_tool_completions(stream, &mut events, argument_parser);
        }
        if let Some(usage) = stream.pending_usage.take()
            && !stream.usage_seen
        {
            events.push(ChatModelEvent::Usage { usage });
            stream.usage_seen = true;
        }
        events.push(ChatModelEvent::Completed { finish_reason });
        state.openai.remove(&response_key);
    }
    Ok(events)
}

fn append_openai_raw_tool_completions(
    stream: &mut OpenAiRawStreamState,
    events: &mut Vec<ChatModelEvent>,
    argument_parser: &dyn ToolArgumentParser,
) {
    for tool in stream.tools.values() {
        events.push(proposal_event(argument_parser, ToolCallId(tool.call_id.clone()), tool.name.clone(),
            ToolArgumentInput::JsonText(&tool.arguments), None));
    }
}

fn decode_gemini_raw_frame(
    frame: ProviderWireFrame,
    raw_decode_state: &Arc<Mutex<RawDecodeState>>,
    argument_parser: &dyn ToolArgumentParser,
) -> Result<Vec<ChatModelEvent>, ChatModelError> {
    let mut state = raw_decode_state
        .lock()
        .map_err(|_| ChatModelError::protocol_violation("raw decoder state poisoned"))?;
    let explicit_response_key = frame
        .data
        .get("responseId")
        .or_else(|| frame.data.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let response_key = match explicit_response_key.clone() {
        Some(response_key) => response_key,
        None if state.gemini.len() <= 1 => state
            .gemini
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "__gemini_anonymous__".to_owned()),
        None => {
            return Err(ChatModelError::protocol_violation(
                "Gemini frame without response id cannot be correlated with multiple active streams",
            ));
        }
    };
    let stream = state.gemini.entry(response_key.clone()).or_default();
    let mut events = Vec::new();
    let mut finish_reason = None;
    let mut has_tool_calls = stream.has_tool_calls;
    if !stream.response_started {
        if let Some(response_id) = explicit_response_key {
            events.push(ChatModelEvent::ResponseStarted {
                provider_response_id: Some(ProviderResponseId(response_id)),
            });
        }
        // Gemini generateContent may omit responseId. Preserve the established
        // canonical event sequence in that case while marking the stream so a
        // later terminal marker can still be correlated.
        stream.response_started = true;
    }

    let candidates = match frame.data.get("candidates") {
        Some(value) => Some(value.as_array().ok_or_else(|| {
            ChatModelError::protocol_violation("Gemini candidates is not an array")
        })?),
        None => None,
    };
    if let Some(candidates) = candidates {
        if candidates.len() > 1 {
            return Err(ChatModelError::protocol_violation(
                "Gemini response contains more than one candidate",
            ));
        }
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            let candidate = candidate.as_object().ok_or_else(|| {
                ChatModelError::protocol_violation("Gemini candidate is not an object")
            })?;
            let parts = match candidate.get("content") {
                None | Some(Value::Null) => None,
                Some(Value::Object(content)) => match content.get("parts") {
                    None | Some(Value::Null) => None,
                    Some(parts) => Some(parts.as_array().ok_or_else(|| {
                        ChatModelError::protocol_violation(
                            "Gemini content parts is not an array",
                        )
                    })?),
                },
                Some(_) => {
                    return Err(ChatModelError::protocol_violation(
                        "Gemini candidate content is not an object",
                    ));
                }
            };
            if let Some(parts) = parts {
                for (part_index, part) in parts.iter().enumerate() {
                    let part = part.as_object().ok_or_else(|| {
                        ChatModelError::protocol_violation("Gemini content part is not an object")
                    })?;
                    if part.get("text").is_some() && part.get("functionCall").is_some() {
                        return Err(ChatModelError::protocol_violation(
                            "Gemini content part contains both text and functionCall",
                        ));
                    }
                    if let Some(text) = part.get("text").and_then(Value::as_str)
                        && !text.is_empty()
                    {
                        let event = if part.get("thought").and_then(Value::as_bool) == Some(true) {
                            ChatModelEvent::ReasoningDelta {
                                text: text.to_owned(),
                            }
                        } else {
                            ChatModelEvent::OutputTextDelta {
                                text: text.to_owned(),
                                source: None,
                            }
                        };
                        events.push(event);
                    }
                    let signature = match part
                        .get("thoughtSignature")
                        .or_else(|| part.get("thought_signature"))
                    {
                        None | Some(Value::Null) => None,
                        Some(Value::String(signature)) => Some(signature),
                        Some(_) => {
                            return Err(ChatModelError::protocol_violation(
                                "Gemini thoughtSignature is not a string or null",
                            ));
                        }
                    };
                    if part.get("functionCall").is_none()
                        && let Some(signature) = signature.filter(|signature| !signature.is_empty())
                    {
                        events.push(ChatModelEvent::ReasoningSignature {
                            signature: signature.to_owned(),
                        });
                    }
                    if let Some(function_call) = part.get("functionCall") {
                        has_tool_calls = true;
                        let function_call = function_call.as_object().ok_or_else(|| {
                            ChatModelError::protocol_violation(
                                "Gemini functionCall is not an object",
                            )
                        })?;
                        let name = function_call
                            .get("name")
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .ok_or_else(|| {
                                ChatModelError::protocol_violation(
                                    "Gemini functionCall is missing name",
                                )
                            })?;
                        let call_id = match function_call.get("id") {
                            None | Some(Value::Null) => {
                                format!("gemini-tool-{candidate_index}-{part_index}")
                            }
                            Some(Value::String(id)) if !id.is_empty() => id.to_owned(),
                            Some(Value::String(_)) => {
                                format!("gemini-tool-{candidate_index}-{part_index}")
                            }
                            Some(_) => {
                                return Err(ChatModelError::protocol_violation(
                                    "Gemini functionCall id is not a string or null",
                                ));
                            }
                        };
                        let arguments = match function_call.get("args") {
                            None | Some(Value::Null) => json!({}),
                            Some(value) => value.clone(),
                        };
                        let tool_id = ToolCallId(call_id.clone());
                        if let Some(existing) = stream.tools.get(&tool_id)
                            && existing.name != name {
                            return Err(ChatModelError::protocol_violation(
                                "Gemini functionCall name changed for one call id",
                            ));
                        }
                        stream.tools.insert(tool_id, RawGeminiToolCall {
                            name: name.to_owned(), arguments,
                            provider_metadata: signature.map(|signature|
                                StrictJsonValue(json!({"thoughtSignature": signature}))),
                        });
                    }
                }
            }
            if let Some(value) = candidate
                .get("finishReason")
                .or_else(|| candidate.get("finish_reason"))
            {
                match value {
                    Value::Null => {}
                    Value::String(reason) if !is_unspecified_finish_reason(reason) => {
                        finish_reason = Some(parse_finish_reason_text(reason));
                    }
                    Value::String(_) => {}
                    _ => {
                        return Err(ChatModelError::protocol_violation(
                            "Gemini finishReason is not a string or null",
                        ));
                    }
                }
            }
        }
    }
    if let Some(usage) = frame.data.get("usageMetadata").filter(|value| !value.is_null()) {
        stream.pending_usage = Some(parse_usage(usage));
    }
    if frame
        .data
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
        .and_then(Value::as_str)
        .is_some()
    {
        return Err(ChatModelError::provider_unavailable(
            "Gemini blocked the prompt",
        ));
    }
    if candidates.is_some_and(|candidates| candidates.is_empty())
        && frame.data.get("usageMetadata").is_none()
        && !is_terminal_frame(&frame.data)
    {
        return Err(ChatModelError::protocol_violation(
            "Gemini raw frame has an empty candidates array without usage metadata",
        ));
    }
    let done_marker = frame.event.eq_ignore_ascii_case("done");
    if let Some(value) = frame
        .data
        .get("finishReason")
        .or_else(|| frame.data.get("finish_reason"))
    {
        match value {
            Value::Null => {}
            Value::String(reason) if !is_unspecified_finish_reason(reason) => {
                finish_reason = Some(parse_finish_reason_text(reason));
            }
            Value::String(_) => {}
            _ => {
                return Err(ChatModelError::protocol_violation(
                    "Gemini finishReason is not a string or null",
                ));
            }
        }
    }
    let json_response = frame.event.eq_ignore_ascii_case("json");
    let terminal = finish_reason.is_some()
        || done_marker
        || is_terminal_frame(&frame.data)
        || (json_response && candidates.is_some());
    if let Some(mut finish_reason) = finish_reason {
        if has_tool_calls && finish_reason == ChatFinishReason::Completed {
            finish_reason = ChatFinishReason::ToolCalls;
        }
        if matches!(finish_reason, ChatFinishReason::Completed | ChatFinishReason::ToolCalls) {
            events.extend(stream.tools.iter().map(|(id, tool)| proposal_event(argument_parser,
                id.clone(), tool.name.clone(), ToolArgumentInput::JsonValue(&tool.arguments), tool.provider_metadata.clone())));
        } else if finish_reason == ChatFinishReason::MaxOutputTokens {
            // Preserve unadmitted proposal identities for exact truncation
            // discard without claiming that their parameters completed.
            events.extend(stream.tools.iter().map(|(id, tool)| ChatModelEvent::ToolCallDelta {
                call_id: id.clone(), name: tool.name.clone(), arguments_delta: String::new(),
            }));
        }
        if let Some(usage) = stream.pending_usage.take() {
            events.push(ChatModelEvent::Usage { usage });
        }
        events.push(ChatModelEvent::Completed { finish_reason });
    } else if terminal {
        let finish_reason = if has_tool_calls {
            ChatFinishReason::ToolCalls
        } else {
            ChatFinishReason::Completed
        };
        if matches!(finish_reason, ChatFinishReason::Completed | ChatFinishReason::ToolCalls) {
            events.extend(stream.tools.iter().map(|(id, tool)| proposal_event(argument_parser,
                id.clone(), tool.name.clone(), ToolArgumentInput::JsonValue(&tool.arguments), tool.provider_metadata.clone())));
        }
        if let Some(usage) = stream.pending_usage.take() {
            events.push(ChatModelEvent::Usage { usage });
        }
        events.push(ChatModelEvent::Completed { finish_reason });
    }
    stream.has_tool_calls = has_tool_calls;
    if terminal {
        state.gemini.remove(&response_key);
    }
    Ok(events)
}

fn extract_openai_raw_reasoning(delta: &Map<String, Value>) -> Option<String> {
    ["reasoning_content", "reasoning"]
        .iter()
        .find_map(|key| delta.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod private_chat_replay_tests {
    use super::*;

    #[test]
    fn stepfun_nullable_native_reasoning_replay_does_not_reconstruct_its_streamed_text() {
        let item = json!({"id":"rs_step", "type":"reasoning", "content":null,"summary":[],"encrypted_content":null,"status":null});
        let message = ChatMessage {role:ChatRole::Assistant,provider_round_id:None,content:vec![
            ChatContentPart::Reasoning {text:"ACTUAL_PRIVATE_STREAM_TEXT".into(),signature:None,encrypted_content:None},
            ChatContentPart::ProviderReasoning {block:crate::ChatProviderReasoning::OpenaiResponsesReasoning {
                item:item.clone(),route_digest:None}},
            ChatContentPart::Text {text:"Visible answer".into()},
        ]};
        let encoded = responses_items(&[message]);
        assert_eq!(encoded.len(),2);
        assert_eq!(encoded[0],item);
        assert_eq!(encoded[1]["content"][0]["text"],"Visible answer");
        assert!(!serde_json::to_string(&encoded).unwrap().contains("ACTUAL_PRIVATE_STREAM_TEXT"));
    }

    #[test]
    fn chat_tool_history_uses_only_wire_fields_and_defers_media_until_the_batch_finishes() {
        let mut input = crate::recorded_conformance_fixtures().into_iter()
            .find(|fixture| fixture.protocol == ChatProtocol::OpenaiChat).unwrap().request.input;
        input.instructions.clear();
        input.messages = serde_json::from_value(json!([
            {"role":"assistant", "provider_round_id":"local-round", "content":[
                {"type":"tool_call","call_id":"one","name":"read","arguments":{},"provider_metadata":{"local":"metadata"}},
                {"type":"tool_call","call_id":"two","name":"read","arguments":{}}
            ]},
            {"role":"tool", "content":[{"type":"tool_result","call_id":"one","is_error":true,"output":[
                {"type":"text","text":"first failure"},
                {"type":"image","media_type":"image/png","data_base64":"aW1hZ2U="}
            ]}]},
            {"role":"tool", "content":[{"type":"tool_result","call_id":"two","is_error":false,"output":[
                {"type":"text","text":"second result"},
                {"type":"audio","media_type":"audio/wav","data_base64":"YXVkaW8="}
            ]}]},
            {"role":"user", "content":[{"type":"text","text":"Continue"}]}
        ])).unwrap();
        let body = encode_openai_chat_request(&input,"selected").unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.iter().map(|message| message["role"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["assistant","tool","tool","user","user"]);
        assert!(messages[0].get("provider_round_id").is_none());
        assert!(messages[0]["content"].is_null());
        assert!(messages[0]["tool_calls"][0].get("provider_metadata").is_none());
        assert_eq!(messages[1]["content"],"[tool error]\nfirst failure");
        assert!(messages[1].get("is_error").is_none());
        assert_eq!(messages[2]["content"],"second result");
        assert_eq!(messages[3]["content"][1]["image_url"]["url"],"data:image/png;base64,aW1hZ2U=");
        assert_eq!(messages[3]["content"][3]["input_audio"],json!({"format":"wav","data":"YXVkaW8="}));
        assert_eq!(messages[4]["content"],"Continue");
        input.messages.pop();
        let terminal = encode_openai_chat_request(&input,"selected").unwrap();
        assert_eq!(terminal["messages"].as_array().unwrap().len(),4);
        assert_eq!(terminal["messages"][3]["role"],"user");
    }

    #[test]
    fn chat_user_audio_uses_api_format_names() {
        let message = ChatMessage { role:ChatRole::User, provider_round_id:None,
            content:vec![ChatContentPart::Audio {media_type:"audio/mpeg".into(),data_base64:"YXVkaW8=".into()}] };
        assert_eq!(openai_messages(&message)[0]["content"][0]["input_audio"]["format"],"mp3");
    }

    #[test]
    fn openai_chat_private_thinking_never_becomes_public_assistant_content() {
        let private = ChatContentPart::Reasoning {
            text: "PRIVATE_NOT_EXECUTED<tool_call><function=write_file>FAKE_BODY</function></tool_call>".into(),
            signature: Some("PRIVATE_SIGNATURE".into()), encrypted_content: None,
        };
        let message = |content| ChatMessage { role: ChatRole::Assistant, content, provider_round_id: None };
        let only = openai_messages(&message(vec![private.clone()]));
        assert!(only.is_empty());
        assert!(!serde_json::to_string(&only).unwrap().contains("PRIVATE_NOT_EXECUTED"));
        assert!(!serde_json::to_string(&only).unwrap().contains("FAKE_BODY"));
        let mixed = openai_messages(&message(vec![private,
            ChatContentPart::Text { text:"Visible progress remains exact.".into() },
            ChatContentPart::ToolCall { call_id:"actual-call".into(),name:"write_file".into(),
                arguments:nomifun_agent_contracts::StrictJsonValue(json!({"path":"actual.txt","content":"actual\n"})),provider_metadata:None },
        ]));
        let encoded=serde_json::to_string(&mixed).unwrap();
        assert!(!encoded.contains("PRIVATE_NOT_EXECUTED")&&!encoded.contains("PRIVATE_SIGNATURE")&&!encoded.contains("FAKE_BODY"));
        assert!(encoded.contains("Visible progress remains exact."));
        assert_eq!(mixed[0]["tool_calls"][0]["function"]["name"],"write_file");
        assert_eq!(serde_json::from_str::<Value>(mixed[0]["tool_calls"][0]["function"]["arguments"].as_str().unwrap()).unwrap(),
            json!({"path":"actual.txt","content":"actual\n"}));
        let marker=ChatContentPart::Text {text:"[Private reasoning omitted from replay]".into()};
        assert!(openai_messages(&message(vec![marker.clone()])).is_empty());
        let user=ChatMessage {role:ChatRole::User,content:vec![marker],provider_round_id:None};
        assert_eq!(openai_messages(&user)[0]["content"],"[Private reasoning omitted from replay]");
    }
}

fn append_raw_arguments(target: &mut String, value: &Value) {
    if let Some(value) = raw_argument_string(value) {
        target.push_str(&value);
    }
}

fn raw_argument_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Null => None,
        value => Some(value.to_string()),
    }
}

fn string_at(value: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for component in path.split('.') {
            current = current.get(component)?;
        }
        current.as_str().map(str::to_owned)
    })
}

fn text_delta(value: &Value) -> Option<String> {
    string_at(
        value,
        &[
            "text",
            "delta",
            "delta.text",
            "delta.content",
            "content",
            "output_text",
            "part.text",
        ],
    )
}

fn reasoning_delta(value: &Value) -> Option<String> {
    string_at(
        value,
        &[
            "reasoning",
            "reasoning_content",
            "thinking",
            "delta.reasoning",
            "delta.reasoning_content",
            "delta.thinking",
        ],
    )
}

fn parse_tool_call(value: &Value, argument_parser: &dyn ToolArgumentParser) -> Result<ChatModelEvent, ChatModelError> {
    let call_id = string_at(value, &["call_id", "id", "tool_call_id", "tool.id"])
        .ok_or_else(|| ChatModelError::protocol_violation("completed tool call is missing id"))?;
    let name = string_at(value, &["name", "function.name", "tool.name"])
        .ok_or_else(|| ChatModelError::protocol_violation("completed tool call is missing name"))?;
    let arguments = value.get("arguments").or_else(|| value.get("input"))
        .or_else(|| value.get("function").and_then(|f| f.get("arguments")))
        .cloned().unwrap_or_else(|| json!({}));
    let input = if let Value::String(text) = &arguments {
        ToolArgumentInput::JsonText(text)
    } else { ToolArgumentInput::JsonValue(&arguments) };
    Ok(proposal_event(argument_parser, ToolCallId(call_id), name, input,
        value.get("provider_metadata").cloned().map(StrictJsonValue)))
}

pub(crate) fn parse_usage(value: &Value) -> ChatUsage {
    let source = value.get("usage").unwrap_or(value);
    let mut provider_reported = std::collections::BTreeMap::new();
    if let Some(object) = source.as_object() {
        for (key, value) in object {
            if let Some(number) = value.as_u64()
                && !matches!(
                    key.as_str(),
                    "input_tokens"
                        | "output_tokens"
                        | "reasoning_tokens"
                        | "cache_write_tokens"
                        | "cache_read_tokens"
                        | "audio_input_tokens"
                        | "audio_output_tokens"
                        | "prompt_tokens"
                        | "completion_tokens"
                        | "total_tokens"
                        | "input"
                        | "output"
                        | "promptTokenCount"
                        | "candidatesTokenCount"
                        | "thoughtsTokenCount"
                        | "cacheCreationInputTokens"
                        | "cacheReadInputTokens"
                        | "cachedContentTokenCount"
                        | "audioInputTokens"
                        | "audioOutputTokens"
                )
            {
                provider_reported.insert(key.clone(), number);
            }
        }
    }
    ChatUsage {
        input_tokens: first_u64(
            source,
            &["input_tokens", "prompt_tokens", "input", "promptTokenCount"],
        ),
        output_tokens: output_tokens_from_usage(value).unwrap_or(0),
        reasoning_tokens: first_u64(source, &["reasoning_tokens", "reasoning", "thoughtsTokenCount"])
            .max(nested_u64(
                source,
                &["completion_tokens_details", "reasoning_tokens"],
            ))
            .max(nested_u64(source, &["output_tokens_details", "reasoning_tokens"])),
        cache_write_tokens: first_u64(
            source,
            &["cache_write_tokens", "cache_creation_input_tokens", "cacheCreationInputTokens"],
        ),
        cache_read_tokens: first_u64(
            source,
            &[
                "cache_read_tokens",
                "cache_read_input_tokens",
                "cacheReadInputTokens",
                "cachedContentTokenCount",
            ],
        )
        .max(nested_u64(
            source,
            &["prompt_tokens_details", "cached_tokens"],
        ))
        .max(nested_u64(source, &["input_tokens_details", "cached_tokens"])),
        audio_input_tokens: first_u64(source, &["audio_input_tokens", "audioInputTokens"]),
        audio_output_tokens: first_u64(source, &["audio_output_tokens", "audioOutputTokens"]),
        provider_reported,
    }
}

fn usage_from_nested(value: &Value) -> Option<ChatUsage> {
    value.get("usage").map(parse_usage)
}

fn output_tokens_from_usage(value: &Value) -> Option<u64> {
    let source = value.get("usage").unwrap_or(value);
    ["output_tokens", "completion_tokens", "output", "candidatesTokenCount"].iter()
        .find_map(|key| source.get(*key).and_then(Value::as_u64))
}

fn first_u64(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn nested_u64(value: &Value, path: &[&str]) -> u64 {
    let Some(nested) = path
        .iter()
        .try_fold(value, |current, key| current.get(*key))
    else {
        return 0;
    };
    nested.as_u64().unwrap_or(0)
}

fn parse_finish_reason(
    _protocol: ChatProtocol,
    value: &Value,
    event_name: &str,
) -> ChatFinishReason {
    let reason = string_at(
        value,
        &[
            "finish_reason",
            "stop_reason",
            "finishReason",
            "response.status",
            "status",
        ],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();
    if reason.contains("unexpected_tool") {
        ChatFinishReason::Refusal
    } else if reason.contains("tool") || reason == "function_call" {
        ChatFinishReason::ToolCalls
    } else if reason.contains("length")
        || reason.contains("max")
        || reason == "max_output_tokens"
    {
        ChatFinishReason::MaxOutputTokens
    } else if reason.contains("refusal")
        || reason.contains("safety")
        || reason.contains("blocked")
        || reason.contains("content_filter")
        || reason.contains("recitation")
        || reason.contains("blocklist")
        || reason.contains("image_safety")
        || reason.contains("prohibited")
        || reason.contains("spii")
        || reason.contains("unexpected_tool")
        || reason.contains("malformed")
    {
        ChatFinishReason::Refusal
    } else if reason.contains("cancel") || reason.contains("abort") {
        ChatFinishReason::Cancelled
    } else if event_name == "response.incomplete" {
        ChatFinishReason::MaxOutputTokens
    } else {
        ChatFinishReason::Completed
    }
}

fn parse_finish_reason_text(reason: &str) -> ChatFinishReason {
    parse_finish_reason(
        ChatProtocol::OpenaiChat,
        &json!({"finish_reason": reason}),
        "finish",
    )
}

fn is_unspecified_finish_reason(reason: &str) -> bool {
    matches!(
        reason.to_ascii_lowercase().as_str(),
        ""
            | "finish_reason_unspecified"
            | "unspecified"
            | "none"
            | "null"
    )
}

fn is_terminal_frame(value: &Value) -> bool {
    value
        .get("done")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("terminal")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}
