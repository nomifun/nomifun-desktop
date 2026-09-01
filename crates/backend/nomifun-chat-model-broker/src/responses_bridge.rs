use std::collections::{BTreeMap, BTreeSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use nomifun_agent_contracts::{ChatRouteIdentity, ModelRouteId, StrictJsonValue, VersionString};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::broker::{BrokerEventEnvelope, ChatBrokerPort};
use crate::contracts::{
    CHAT_MODEL_CONTRACT_VERSION, ChatCausality, ChatContentPart, ChatFinishReason, ChatMessage,
    ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatModelInput, ChatModelRequest,
    ChatModality, ChatReasoningRequest, ChatResponseFormat, ChatRetryDirective, ChatRole,
    ChatToolCall, ChatToolChoice, ChatToolDefinition,
    ChatToolResultPart, ChatUsage, PromptCachePolicy, ProviderResponseId, ProviderRoundId,
    ToolCallId,
};

const RESPONSES_BRIDGE_STREAM_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsesRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponsesInputContent {
    InputText {
        text: String,
    },
    InputImage {
        media_type: String,
        data_base64: String,
    },
    InputAudio {
        media_type: String,
        data_base64: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResponsesInputItem {
    Message {
        role: ResponsesRole,
        content: Vec<ResponsesInputContent>,
    },
    FunctionCall {
        call_id: ToolCallId,
        name: String,
        arguments: StrictJsonValue,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<StrictJsonValue>,
    },
    FunctionCallOutput {
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
pub struct ResponsesBridgeRequest {
    pub bridge_version: VersionString,
    pub causality: ChatCausality,
    pub model_route_id: ModelRouteId,
    pub model_route_revision: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    pub input: Vec<ResponsesInputItem>,
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
    pub previous_response_id: Option<ProviderRoundId>,
    #[serde(default)]
    pub preserve_native_responses_items: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    pub store: bool,
}

impl ResponsesBridgeRequest {
    pub fn into_chat_request(self) -> Result<ChatModelRequest, ChatModelError> {
        if self.bridge_version.as_ref() != CHAT_MODEL_CONTRACT_VERSION {
            return Err(ChatModelError::invalid_request(
                "Responses Bridge version does not match the chat model contract",
            ));
        }
        if self.store {
            return Err(ChatModelError::invalid_request(
                "local Responses Bridge is stateless and requires store=false",
            ));
        }
        let messages = self
            .input
            .into_iter()
            .map(responses_item_to_message)
            .collect::<Result<Vec<_>, _>>()?;
        let route = ChatRouteIdentity::new(
            self.causality.route_identity.preset_revision_id.clone(),
            self.causality.route_identity.model_task.clone(),
            self.model_route_id.clone(),
            self.model_route_revision,
        );
        if route != self.causality.route_identity {
            return Err(ChatModelError::invalid_request(
                "Responses Bridge route fields do not match the immutable causality identity",
            ));
        }
        let request = ChatModelRequest {
            contract_version: VersionString(CHAT_MODEL_CONTRACT_VERSION.to_owned()),
            route,
            causality: self.causality,
            input: ChatModelInput {
                instructions: self.instructions,
                messages,
                tools: self.tools,
                tool_choice: self.tool_choice,
                max_output_tokens: self.max_output_tokens,
                reasoning: self.reasoning,
                prompt_cache: self.prompt_cache,
                response_format: self.response_format,
                requested_output_modalities: self.requested_output_modalities,
                provider_round_parent: self.previous_response_id,
                preserve_native_responses_items: self.preserve_native_responses_items,
                metadata: self.metadata,
            },
        };
        request
            .validate()
            .map_err(|error| ChatModelError::invalid_request(error.to_string()))?;
        Ok(request)
    }
}

fn responses_item_to_message(item: ResponsesInputItem) -> Result<ChatMessage, ChatModelError> {
    let (role, content) = match item {
        ResponsesInputItem::Message { role, content } => {
            let role = match role {
                ResponsesRole::System => ChatRole::System,
                ResponsesRole::User => ChatRole::User,
                ResponsesRole::Assistant => ChatRole::Assistant,
            };
            let content = content
                .into_iter()
                .map(|part| match part {
                    ResponsesInputContent::InputText { text } => ChatContentPart::Text { text },
                    ResponsesInputContent::InputImage {
                        media_type,
                        data_base64,
                    } => ChatContentPart::Image {
                        media_type,
                        data_base64,
                    },
                    ResponsesInputContent::InputAudio {
                        media_type,
                        data_base64,
                    } => ChatContentPart::Audio {
                        media_type,
                        data_base64,
                    },
                })
                .collect();
            (role, content)
        }
        ResponsesInputItem::FunctionCall {
            call_id,
            name,
            arguments,
            provider_metadata,
        } => (
            ChatRole::Assistant,
            vec![ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                provider_metadata,
            }],
        ),
        ResponsesInputItem::FunctionCallOutput {
            call_id,
            output,
            is_error,
        } => (
            ChatRole::Tool,
            vec![ChatContentPart::ToolResult {
                call_id,
                output,
                is_error,
            }],
        ),
        ResponsesInputItem::Reasoning {
            text,
            signature,
            encrypted_content,
        } => (
            ChatRole::Assistant,
            vec![ChatContentPart::Reasoning {
                text,
                signature,
                encrypted_content,
            }],
        ),
    };
    if content.is_empty() {
        return Err(ChatModelError::invalid_request(
            "Responses input message content must not be empty",
        ));
    }
    Ok(ChatMessage {
        role,
        content,
        provider_round_id: None,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ResponsesBridgeEvent {
    #[serde(rename = "response.created")]
    ResponseCreated {
        response_id: ProviderResponseId,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        response_id: ProviderResponseId,
        delta: String,
    },
    #[serde(rename = "response.output_audio.delta")]
    OutputAudioDelta {
        response_id: ProviderResponseId,
        media_type: String,
        data_base64: String,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningDelta {
        response_id: ProviderResponseId,
        delta: String,
    },
    #[serde(rename = "response.reasoning_signature")]
    ReasoningSignature {
        response_id: ProviderResponseId,
        signature: String,
    },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        response_id: ProviderResponseId,
        call_id: ToolCallId,
        name: String,
        delta: String,
    },
    #[serde(rename = "response.function_call.done")]
    FunctionCallDone {
        response_id: ProviderResponseId,
        call: ChatToolCall,
    },
    #[serde(rename = "response.output_item.added")]
    NativeItem {
        response_id: ProviderResponseId,
        item_type: String,
        item: StrictJsonValue,
    },
    #[serde(rename = "response.provider_round")]
    ProviderRound {
        response_id: ProviderResponseId,
        round_id: ProviderRoundId,
    },
    #[serde(rename = "response.usage")]
    Usage {
        response_id: ProviderResponseId,
        usage: ChatUsage,
    },
    #[serde(rename = "response.completed")]
    Completed {
        response_id: ProviderResponseId,
        finish_reason: ChatFinishReason,
    },
    #[serde(rename = "response.failed")]
    Failed {
        response_id: ProviderResponseId,
        error: ChatModelError,
    },
}

pub struct ResponsesBridgeStream {
    receiver: mpsc::Receiver<ResponsesBridgeEvent>,
}

impl Stream for ResponsesBridgeStream {
    type Item = ResponsesBridgeEvent;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}

pub struct ResponsesBridge {
    broker: Arc<dyn ChatBrokerPort>,
}

impl ResponsesBridge {
    pub fn new(broker: Arc<dyn ChatBrokerPort>) -> Self {
        Self { broker }
    }

    pub const fn retry_count(&self) -> u8 {
        0
    }

    pub async fn open_stream(
        &self,
        request: ResponsesBridgeRequest,
    ) -> Result<ResponsesBridgeStream, ChatModelError> {
        let chat_request = request.into_chat_request()?;
        let mut broker_stream = self.broker.open_chat_stream(chat_request).await?;
        let response_id = ProviderResponseId(format!("resp_{}", Uuid::now_v7().simple()));
        let (sender, receiver) = mpsc::channel(RESPONSES_BRIDGE_STREAM_CAPACITY);

        tokio::spawn(async move {
            if sender
                .send(ResponsesBridgeEvent::ResponseCreated {
                    response_id: response_id.clone(),
                })
                .await
                .is_err()
            {
                return;
            }

            let mut terminal = false;
            while let Some(item) = broker_stream.next().await {
                match item {
                    Ok(envelope) => {
                        let Some(event) = map_broker_event(response_id.clone(), envelope) else {
                            continue;
                        };
                        terminal = matches!(event, ResponsesBridgeEvent::Completed { .. });
                        if sender.send(event).await.is_err() || terminal {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender
                            .send(ResponsesBridgeEvent::Failed {
                                response_id: response_id.clone(),
                                error,
                            })
                            .await;
                        return;
                    }
                }
            }

            if !terminal {
                let _ = sender
                    .send(ResponsesBridgeEvent::Failed {
                        response_id,
                        error: ChatModelError::new(
                            ChatModelErrorCode::StreamInterrupted,
                            "broker stream ended without a terminal event",
                            ChatRetryDirective::Never,
                        ),
                    })
                    .await;
            }
        });

        Ok(ResponsesBridgeStream { receiver })
    }
}

fn map_broker_event(
    response_id: ProviderResponseId,
    envelope: BrokerEventEnvelope,
) -> Option<ResponsesBridgeEvent> {
    match envelope.event {
        ChatModelEvent::ResponseStarted { .. } => None,
        ChatModelEvent::OutputTextDelta { text } => Some(ResponsesBridgeEvent::OutputTextDelta {
            response_id,
            delta: text,
        }),
        ChatModelEvent::OutputAudioDelta {
            media_type,
            data_base64,
        } => Some(ResponsesBridgeEvent::OutputAudioDelta {
            response_id,
            media_type,
            data_base64,
        }),
        ChatModelEvent::ReasoningDelta { text } => Some(ResponsesBridgeEvent::ReasoningDelta {
            response_id,
            delta: text,
        }),
        ChatModelEvent::ReasoningSignature { signature } => {
            Some(ResponsesBridgeEvent::ReasoningSignature {
                response_id,
                signature,
            })
        }
        ChatModelEvent::ToolCallDelta {
            call_id,
            name,
            arguments_delta,
        } => Some(ResponsesBridgeEvent::FunctionCallArgumentsDelta {
            response_id,
            call_id,
            name,
            delta: arguments_delta,
        }),
        ChatModelEvent::ToolCallCompleted { call } => {
            Some(ResponsesBridgeEvent::FunctionCallDone { response_id, call })
        }
        ChatModelEvent::ProviderRoundId { round_id } => {
            Some(ResponsesBridgeEvent::ProviderRound {
                response_id,
                round_id,
            })
        }
        ChatModelEvent::NativeResponsesItem { item_type, item } => {
            Some(ResponsesBridgeEvent::NativeItem {
                response_id,
                item_type,
                item,
            })
        }
        ChatModelEvent::Usage { usage } => Some(ResponsesBridgeEvent::Usage {
            response_id,
            usage,
        }),
        ChatModelEvent::Completed { finish_reason } => Some(ResponsesBridgeEvent::Completed {
            response_id,
            finish_reason,
        }),
    }
}
