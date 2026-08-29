use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;
use nomifun_agent_contracts::{
    ConnectionConfigRef, DigestHex, ModelRouteId, StrictJsonValue,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::contracts::{
    ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelErrorCode,
    ChatModelEvent, ChatModelFeature, ChatModelRequest, ChatProtocol, ChatResponseFormat, ChatRole,
    ChatToolCall, ChatToolChoice, ChatToolResultPart, ChatUsage, ProviderCredentialRef,
    ProviderIdRef, ProviderResponseId, ProviderRoundId, ResolvedChatRoute, ToolCallId,
};
use crate::ports::CredentialLease;

pub type ProviderWireStream =
    Pin<Box<dyn Stream<Item = Result<ProviderWireFrame, ChatModelError>> + Send>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderWireRequest {
    pub protocol: ChatProtocol,
    pub provider_id: ProviderIdRef,
    pub model: String,
    pub model_route_id: ModelRouteId,
    pub model_route_revision: u64,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
    pub credential_ref: ProviderCredentialRef,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderWireFrame {
    pub event: String,
    #[serde(default)]
    pub data: Value,
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
    fn decode_frame(
        &self,
        frame: ProviderWireFrame,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError>;
}

pub fn protocol_features(protocol: ChatProtocol) -> BTreeSet<ChatModelFeature> {
    use ChatModelFeature as Feature;

    let mut features = BTreeSet::from([
        Feature::TextInput,
        Feature::TextOutput,
        Feature::ToolCalls,
        Feature::Reasoning,
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
}

impl AdapterCore {
    fn new(
        protocol: ChatProtocol,
        name: &'static str,
        transport: Arc<dyn ProviderTransport>,
    ) -> Self {
        Self {
            protocol,
            name,
            transport,
        }
    }

    fn encode(
        &self,
        request: &ChatModelRequest,
        route: &ResolvedChatRoute,
        credential: &CredentialLease,
        body: Value,
    ) -> Result<ProviderWireRequest, ChatModelError> {
        request
            .validate()
            .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;
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
        Ok(ProviderWireRequest {
            protocol: self.protocol,
            provider_id: route.provider_id.clone(),
            model: route.model.clone(),
            model_route_id: route.model_route_id.clone(),
            model_route_revision: route.model_route_revision,
            connection_config_ref: route.connection_config_ref.clone(),
            config_revision_digest: route.config_revision_digest.clone(),
            credential_ref: route.credential_ref.clone(),
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
                Self {
                    core: AdapterCore::new($protocol, $label, transport),
                }
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

            fn encode_request(
                &self,
                request: &ChatModelRequest,
                route: &ResolvedChatRoute,
                credential: &CredentialLease,
            ) -> Result<ProviderWireRequest, ChatModelError> {
                self.core
                    .encode(request, route, credential, $encoder(request, route))
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
                decode_frame_for_protocol(self.core.protocol, frame)
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

fn encode_anthropic_request(
    request: &ChatModelRequest,
    route: &ResolvedChatRoute,
) -> Value {
    let input = &request.input;
    let mut body = json!({
        "model": route.model,
        "system": combined_system_text(&input.instructions, &input.messages),
        "messages": anthropic_messages(&input.messages),
        "stream": true
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
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.input_schema.0.clone(),
                    })
                })
                .collect(),
        );
        body["tool_choice"] = anthropic_tool_choice(&input.tool_choice);
    }
    if let Some(reasoning) = &input.reasoning {
        if let Some(budget) = reasoning.max_reasoning_tokens {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget,
            });
        }
    }
    body
}

fn encode_openai_chat_request(
    request: &ChatModelRequest,
    route: &ResolvedChatRoute,
) -> Value {
    let input = &request.input;
    let mut messages = Vec::new();
    for instruction in &input.instructions {
        messages.push(json!({"role": "system", "content": instruction}));
    }
    messages.extend(input.messages.iter().flat_map(openai_messages));
    let mut body = json!({
        "model": route.model,
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
    }
    if let Some(reasoning) = &input.reasoning
        && let Some(effort) = reasoning.effort
    {
        body["reasoning_effort"] = Value::String(match effort {
            crate::contracts::ReasoningEffort::Low => "low",
            crate::contracts::ReasoningEffort::Medium => "medium",
            crate::contracts::ReasoningEffort::High => "high",
        }
        .to_owned());
    }
    if !matches!(input.response_format, ChatResponseFormat::Text) {
        body["response_format"] = response_format_value(&input.response_format);
    }
    body
}

fn encode_openai_responses_request(
    request: &ChatModelRequest,
    route: &ResolvedChatRoute,
) -> Value {
    let input = &request.input;
    let mut body = json!({
        "model": route.model,
        "instructions": input.instructions.join("\n\n"),
        "input": responses_items(&input.messages),
        "stream": true,
        "store": false,
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
                        "strict": true,
                    })
                })
                .collect(),
        );
        body["tool_choice"] = openai_responses_tool_choice(&input.tool_choice);
    }
    if !matches!(input.response_format, ChatResponseFormat::Text) {
        body["text"] = json!({
            "format": responses_format_value(&input.response_format)
        });
    }
    if let Some(reasoning) = &input.reasoning {
        body["reasoning"] = json!({
            "effort": reasoning.effort.map(|effort| match effort {
                crate::contracts::ReasoningEffort::Low => "low",
                crate::contracts::ReasoningEffort::Medium => "medium",
                crate::contracts::ReasoningEffort::High => "high",
            }),
            "summary": format!("{:?}", reasoning.summary).to_ascii_lowercase(),
        });
    }
    body
}

fn encode_gemini_request(
    request: &ChatModelRequest,
    _route: &ResolvedChatRoute,
) -> Value {
    let input = &request.input;
    let mut body = json!({
        "systemInstruction": {
            "parts": [{
                "text": combined_system_text(&input.instructions, &input.messages)
            }]
        },
        "contents": gemini_contents(&input.messages),
    });
    let mut generation = Map::new();
    if let Some(max_output_tokens) = input.max_output_tokens {
        generation.insert("maxOutputTokens".to_owned(), Value::from(max_output_tokens));
    }
    if let Some(reasoning) = &input.reasoning
        && let Some(effort) = reasoning.effort
    {
        generation.insert(
            "thinkingConfig".to_owned(),
            json!({"thinkingLevel": format!("{effort:?}").to_ascii_lowercase()}),
        );
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
    body
}

fn encode_bedrock_request(
    request: &ChatModelRequest,
    route: &ResolvedChatRoute,
) -> Value {
    let mut body = encode_anthropic_request(request, route);
    body["anthropic_version"] = Value::String("bedrock-2023-05-31".to_owned());
    body
}

fn encode_vertex_request(
    request: &ChatModelRequest,
    route: &ResolvedChatRoute,
) -> Value {
    let mut body = encode_anthropic_request(request, route);
    body["anthropic_version"] = Value::String("vertex-2023-10-16".to_owned());
    body
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
                    "content": openai_tool_result_content(output),
                    "is_error": is_error,
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
        "content": openai_content(&message.content),
    });
    if let Some(round_id) = &message.provider_round_id {
        value["provider_round_id"] = Value::String(round_id.as_ref().to_owned());
    }
    let tool_calls: Vec<Value> = message
        .content
        .iter()
        .filter_map(|part| match part {
            ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                provider_metadata,
            } => Some(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": serde_json::to_string(&arguments.0).unwrap_or_default(),
                },
                "provider_metadata": provider_metadata,
            })),
            _ => None,
        })
        .collect();
    if !tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(tool_calls);
    }
    vec![value]
}

fn openai_content(content: &[ChatContentPart]) -> Value {
    let parts: Vec<Value> = content
        .iter()
        .filter_map(|part| match part {
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
                "input_audio": {"format": media_type, "data": data_base64}
            })),
            ChatContentPart::Reasoning { text, .. } => {
                Some(json!({"type": "text", "text": text, "role": "reasoning"}))
            }
            ChatContentPart::ToolCall { .. } | ChatContentPart::ToolResult { .. } => None,
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

fn openai_tool_result_content(parts: &[ChatToolResultPart]) -> Value {
    let content = parts
        .iter()
        .map(|part| match part {
            ChatToolResultPart::Text { text } => {
                json!({"type": "text", "text": text})
            }
            ChatToolResultPart::Image {
                media_type,
                data_base64,
            } => json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{media_type};base64,{data_base64}")
                }
            }),
            ChatToolResultPart::Audio {
                media_type,
                data_base64,
            } => json!({
                "type": "input_audio",
                "input_audio": {
                    "format": media_type,
                    "data": data_base64
                }
            }),
        })
        .collect::<Vec<_>>();
    if let [part] = content.as_slice()
        && part.get("type").and_then(Value::as_str) == Some("text")
        && let Some(text) = part.get("text").cloned()
    {
        return text;
    }
    Value::Array(content)
}

fn responses_items(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .flat_map(|message| {
            message.content.iter().map(move |part| match part {
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
                    ..
                } => json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": tool_result_string(output),
                }),
                ChatContentPart::Reasoning {
                    text,
                    signature,
                    encrypted_content,
                } => {
                    let opaque_content = encrypted_content.as_ref().or(signature.as_ref());
                    json!({
                        "type": "reasoning",
                        "summary": [{"type": "summary_text", "text": text}],
                        "encrypted_content": opaque_content,
                    })
                }
            })
        })
        .collect()
}

fn gemini_contents(messages: &[ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter(|message| !matches!(message.role, ChatRole::System))
        .map(|message| {
            let role = match message.role {
                ChatRole::Assistant => "model",
                ChatRole::User | ChatRole::Tool => "user",
                ChatRole::System => "user",
            };
            json!({
                "role": role,
                "parts": message.content.iter().map(gemini_part).collect::<Vec<_>>()
            })
        })
        .collect()
}

fn gemini_part(part: &ChatContentPart) -> Value {
    match part {
        ChatContentPart::Text { text } => json!({"text": text}),
        ChatContentPart::Image {
            media_type,
            data_base64,
        } => json!({"inlineData": {"mimeType": media_type, "data": data_base64}}),
        ChatContentPart::Audio {
            media_type,
            data_base64,
        } => json!({"inlineData": {"mimeType": media_type, "data": data_base64}}),
        ChatContentPart::ToolCall {
            name, arguments, ..
        } => json!({"functionCall": {"name": name, "args": arguments.0.clone()}}),
        ChatContentPart::ToolResult {
            call_id,
            output,
            is_error,
        } => json!({"functionResponse": {
            "name": call_id,
            "response": {
                "content": tool_result_string(output),
                "is_error": is_error,
            }
        }}),
        ChatContentPart::Reasoning { text, .. } => json!({"text": text}),
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

fn decode_frame_for_protocol(
    protocol: ChatProtocol,
    frame: ProviderWireFrame,
) -> Result<Vec<ChatModelEvent>, ChatModelError> {
    let event_name = frame.event.trim().to_ascii_lowercase();
    if event_name.is_empty() {
        return Err(ChatModelError::protocol_violation(
            "provider frame has an empty event name",
        ));
    }
    if event_name == "error" || event_name.ends_with(".error") {
        let message = string_at(
            &frame.data,
            &["message", "error.message", "error", "detail"],
        )
        .unwrap_or_else(|| "provider stream error".to_owned());
        return Err(ChatModelError::provider_unavailable(message));
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
                events.push(ChatModelEvent::OutputTextDelta { text });
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
            events.push(ChatModelEvent::ToolCallCompleted {
                call: parse_tool_call(&frame.data)?,
            });
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
                events.push(ChatModelEvent::OutputTextDelta { text });
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

fn parse_tool_call(value: &Value) -> Result<ChatToolCall, ChatModelError> {
    let call_id = string_at(value, &["call_id", "id", "tool_call_id", "tool.id"])
        .ok_or_else(|| ChatModelError::protocol_violation("completed tool call is missing id"))?;
    let name = string_at(value, &["name", "function.name", "tool.name"])
        .ok_or_else(|| ChatModelError::protocol_violation("completed tool call is missing name"))?;
    let arguments = value
        .get("arguments")
        .or_else(|| value.get("input"))
        .or_else(|| value.get("function").and_then(|f| f.get("arguments")))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let arguments = if let Value::String(arguments) = arguments {
        serde_json::from_str(&arguments).unwrap_or_else(|_| json!({"raw": arguments}))
    } else {
        arguments
    };
    Ok(ChatToolCall {
        call_id: ToolCallId(call_id),
        name,
        arguments: StrictJsonValue(arguments),
        provider_metadata: value
            .get("provider_metadata")
            .cloned()
            .map(StrictJsonValue),
    })
}

fn parse_usage(value: &Value) -> ChatUsage {
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
        output_tokens: first_u64(
            source,
            &["output_tokens", "completion_tokens", "output", "candidatesTokenCount"],
        ),
        reasoning_tokens: first_u64(
            source,
            &["reasoning_tokens", "reasoning", "thoughtsTokenCount"],
        ),
        cache_write_tokens: first_u64(
            source,
            &["cache_write_tokens", "cache_creation_input_tokens", "cacheCreationInputTokens"],
        ),
        cache_read_tokens: first_u64(
            source,
            &["cache_read_tokens", "cache_read_input_tokens", "cacheReadInputTokens"],
        ),
        audio_input_tokens: first_u64(source, &["audio_input_tokens", "audioInputTokens"]),
        audio_output_tokens: first_u64(source, &["audio_output_tokens", "audioOutputTokens"]),
        provider_reported,
    }
}

fn usage_from_nested(value: &Value) -> Option<ChatUsage> {
    value.get("usage").map(parse_usage)
}

fn first_u64(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
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
    if reason.contains("tool") || reason == "function_call" {
        ChatFinishReason::ToolCalls
    } else if reason.contains("length")
        || reason.contains("max")
        || reason == "max_output_tokens"
    {
        ChatFinishReason::MaxOutputTokens
    } else if reason.contains("refusal") || reason.contains("safety") || reason.contains("blocked") {
        ChatFinishReason::Refusal
    } else if reason.contains("cancel") || reason.contains("abort") {
        ChatFinishReason::Cancelled
    } else if event_name == "response.incomplete" {
        ChatFinishReason::MaxOutputTokens
    } else {
        ChatFinishReason::Completed
    }
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
