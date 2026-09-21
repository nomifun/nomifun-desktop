//! Anthropic Messages streaming lifecycle, isolated to one Broker attempt.
//! A content-block index selects state; only tool_use.id is a tool call ID.
use crate::adapter::{ProviderWireFrame, parse_usage};
use crate::{
    ChatFinishReason, ChatModelError, ChatModelErrorCode, ChatModelEvent, ChatProtocol,
    ChatRetryDirective, ChatToolCall, ProviderResponseId, ToolCallId,
};
use nomifun_agent_contracts::StrictJsonValue;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const MAX_ARGUMENTS: usize = 256 * 1024;
use crate::ChatProviderReasoning;
use crate::provider_reasoning::{MAX_THINKING_OPAQUE_BYTES, MAX_THINKING_TEXT_BYTES};
const MAX_SIGNATURE: usize = MAX_THINKING_OPAQUE_BYTES;

#[derive(Default)]
pub(crate) struct AnthropicDecoder {
    native: Option<bool>,
    started: bool,
    terminal: bool,
    output_closed: bool,
    incomplete_block: bool,
    next_index: u64,
    active: Option<Block>,
    call_ids: BTreeSet<String>,
    finish: Option<ChatFinishReason>,
    usage: Map<String, Value>,
    budget: crate::wire_budget::WireBudget,
}

enum Block {
    Text,
    Thinking {
        text: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    Tool {
        id: String,
        name: String,
        initial: Value,
        json: String,
    },
}

impl AnthropicDecoder {
    pub(crate) fn decode(
        &mut self,
        frame: &ProviderWireFrame,
    ) -> Option<Result<Vec<ChatModelEvent>, ChatModelError>> {
        let named = frame.event.trim().to_ascii_lowercase();
        let declared = frame.data.get("type").and_then(Value::as_str);
        if named == "json"
            && declared == Some("message")
            && frame.data.get("role").and_then(Value::as_str) == Some("assistant")
        {
            return Some(self.decode_complete_message(&frame.data));
        }
        let kind = if matches!(named.as_str(), "message" | "json") {
            declared.unwrap_or(&named)
        } else {
            &named
        };
        // Heartbeats can precede message_start, but may not select a legacy
        // decoding mode or keep an attempt alive with unbounded no-op frames.
        if kind == "ping" {
            return Some(self.ping(frame, &named, declared));
        }
        let native_frame = matches!(
            declared,
            Some(
                "message_start"
                    | "content_block_start"
                    | "content_block_delta"
                    | "content_block_stop"
                    | "message_delta"
                    | "message_stop"
            )
        ) || frame.data.get("content_block").is_some()
            || frame.data.get("index").is_some()
            || frame.data.pointer("/message/role").is_some();
        if self.native == Some(false) && native_frame {
            return Some(Err(invalid(
                "Anthropic stream mixed native and normalized events",
            )));
        }
        if !*self.native.get_or_insert(native_frame) {
            return None;
        }
        if frame
            .data
            .get("type")
            .is_some_and(|value| !value.is_string())
            || (!matches!(named.as_str(), "message" | "json")
                && declared.is_some_and(|kind| kind != named))
        {
            return Some(Err(invalid(
                "Anthropic event name contradicts its payload type",
            )));
        }
        Some(self.decode_native(kind, &frame.data))
    }

    fn decode_complete_message(
        &mut self,
        message: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if self.started || self.terminal {
            return Err(invalid("Duplicate complete Anthropic response"));
        }
        let id = identity(message, "id")?;
        let usage = field(message, "usage")?;
        let content = field(message, "content")?
            .as_array()
            .ok_or_else(|| invalid("Complete Anthropic content is not an array"))?;
        let stop_reason = field(message, "stop_reason")?
            .as_str()
            .ok_or_else(|| invalid("Complete Anthropic response lacks a stop reason"))?;
        let mut events = self.decode_native(
            "message_start",
            &serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "stop_reason": null,
                    "usage": usage,
                }
            }),
        )?;
        for (index, block) in content.iter().enumerate() {
            events.extend(self.decode_native(
                "content_block_start",
                &serde_json::json!({
                    "type": "content_block_start",
                    "index": index,
                    "content_block": block,
                }),
            )?);
            events.extend(self.decode_native(
                "content_block_stop",
                &serde_json::json!({
                    "type": "content_block_stop",
                    "index": index,
                }),
            )?);
        }
        events.extend(self.decode_native(
            "message_delta",
            &serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                "usage": usage,
            }),
        )?);
        events.extend(self.decode_native(
            "message_stop",
            &serde_json::json!({"type": "message_stop"}),
        )?);
        Ok(events)
    }

    fn ping(
        &mut self,
        frame: &ProviderWireFrame,
        named: &str,
        declared: Option<&str>,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        self.budget.admit(&frame.data)?;
        if self.terminal
            || frame
                .data
                .get("type")
                .is_some_and(|value| value.as_str() != Some("ping"))
            || (!matches!(named, "message" | "json") && declared.is_some_and(|kind| kind != named))
            || !frame
                .data
                .as_object()
                .is_some_and(|object| object.keys().all(|key| key == "type"))
        {
            return Err(invalid("Invalid Anthropic heartbeat"));
        }
        Ok(Vec::new())
    }

    fn decode_native(
        &mut self,
        kind: &str,
        data: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        self.budget.admit(data)?;
        if self.terminal {
            return Err(invalid("Anthropic event after message_stop"));
        }
        if let Some(error) = crate::provider_errors::decode(ChatProtocol::Anthropic, kind, data) {
            return Err(error);
        }
        if kind == "message_start" {
            if self.started {
                return Err(invalid("Duplicate Anthropic message_start"));
            }
            let message = field(data, "message")?;
            if string(message, "type")? != "message"
                || string(message, "role")? != "assistant"
                || !field(message, "content")?
                    .as_array()
                    .is_some_and(Vec::is_empty)
                || message
                    .get("stop_reason")
                    .is_some_and(|value| !value.is_null())
            {
                return Err(invalid("Invalid initial Anthropic message"));
            }
            let id = identity(message, "id")?.to_owned();
            let usage = field(message, "usage")?;
            for key in ["input_tokens", "output_tokens"] {
                if field(usage, key)?.as_u64().is_none() {
                    return Err(invalid("Missing initial Anthropic token count"));
                }
            }
            self.merge_usage(usage)?;
            self.started = true;
            return Ok(vec![ChatModelEvent::ResponseStarted {
                provider_response_id: Some(ProviderResponseId(id)),
            }]);
        }
        if !self.started {
            return Err(invalid("Anthropic content before message_start"));
        }
        match kind {
            "content_block_start" => self.start_block(data),
            "content_block_delta" => self.block_delta(data),
            "content_block_stop" => self.stop_block(data),
            "message_delta" => {
                if self.active.is_some() || self.finish.is_some() {
                    return Err(invalid(
                        "Anthropic message_delta before block closure or after stop reason",
                    ));
                }
                let delta = field(data, "delta")?;
                if !delta.is_object() {
                    return Err(invalid("Invalid Anthropic message delta"));
                }
                self.output_closed = true;
                if let Some(reason) = delta.get("stop_reason").filter(|value| !value.is_null()) {
                    self.finish = Some(match reason.as_str() {
                        Some("end_turn" | "stop_sequence") if self.call_ids.is_empty() => {
                            ChatFinishReason::Completed
                        }
                        Some("tool_use") if !self.call_ids.is_empty() => {
                            ChatFinishReason::ToolCalls
                        }
                        Some("max_tokens") => ChatFinishReason::MaxOutputTokens,
                        Some("refusal") => ChatFinishReason::Refusal,
                        Some("pause_turn") => {
                            return Err(unsupported(
                                "Anthropic server-tool pause is not a local tool continuation",
                            ));
                        }
                        _ => {
                            return Err(invalid(
                                "Anthropic stop reason is unknown or contradicts tool output",
                            ));
                        }
                    });
                    if self.incomplete_block
                        && self.finish != Some(ChatFinishReason::MaxOutputTokens)
                    {
                        return Err(invalid(
                            "Anthropic incomplete block without an output-limit terminal",
                        ));
                    }
                }
                self.merge_usage(field(data, "usage")?)?;
                Ok(Vec::new())
            }
            "message_stop" => {
                if self.active.is_some() {
                    return Err(invalid("Anthropic message stopped with an open block"));
                }
                let finish_reason = self
                    .finish
                    .ok_or_else(|| invalid("Anthropic message_stop lacks a stop reason"))?;
                self.terminal = true;
                Ok(vec![
                    ChatModelEvent::Usage {
                        usage: parse_usage(&Value::Object(std::mem::take(&mut self.usage))),
                    },
                    ChatModelEvent::Completed { finish_reason },
                ])
            }
            _ => Err(unsupported("Unsupported native Anthropic event")),
        }
    }

    fn start_block(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if self.active.is_some()
            || self.finish.is_some()
            || self.output_closed
            || self.incomplete_block
            || self.next_index >= 128
            || index(data)? != self.next_index
        {
            return Err(invalid(
                "Anthropic content block is duplicate, interleaved or excessive",
            ));
        }
        let block = field(data, "content_block")?;
        let mut events = Vec::new();
        self.active = Some(match string(block, "type")? {
            "text" => {
                let text = string(block, "text")?;
                if !text.is_empty() {
                    events.push(ChatModelEvent::OutputTextDelta {
                        text: text.to_owned(),
                    });
                }
                Block::Text
            }
            "thinking" => {
                let text = string(block, "thinking")?;
                if text.len() > MAX_THINKING_TEXT_BYTES {
                    return Err(invalid("Anthropic thinking text limit exceeded"));
                }
                let signature = match block.get("signature") {
                    None => String::new(),
                    Some(Value::String(value)) if value.len() <= MAX_SIGNATURE => value.clone(),
                    _ => return Err(invalid("Invalid initial Anthropic thinking signature")),
                };
                Block::Thinking {
                    text: text.to_owned(),
                    signature,
                }
            }
            "tool_use" => {
                let id = identity(block, "id")?.to_owned();
                let name = identity(block, "name")?.to_owned();
                if self.call_ids.len() >= 64 || !self.call_ids.insert(id.clone()) {
                    return Err(invalid("Duplicate or excessive Anthropic tool call"));
                }
                let initial = field(block, "input")?;
                if !initial.is_object() {
                    return Err(invalid("Anthropic initial tool input is not an object"));
                }
                // WireBudget has already bounded the complete frame. Bound
                // the argument object as well before retaining another copy.
                crate::wire_budget::encoded_size(initial, MAX_ARGUMENTS)?;
                events.push(ChatModelEvent::ToolCallDelta {
                    call_id: ToolCallId(id.clone()),
                    name: name.clone(),
                    arguments_delta: String::new(),
                });
                Block::Tool {
                    id,
                    name,
                    initial: initial.clone(),
                    json: String::new(),
                }
            }
            "redacted_thinking" => {
                let data = string(block, "data")?;
                if data.is_empty() || data.len() > MAX_THINKING_OPAQUE_BYTES {
                    return Err(invalid("Invalid Anthropic redacted thinking block"));
                }
                Block::RedactedThinking {
                    data: data.to_owned(),
                }
            }
            _ => {
                return Err(unsupported(
                    "Anthropic provider-hosted content block is not a local tool",
                ));
            }
        });
        Ok(events)
    }

    fn block_delta(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if index(data)? != self.next_index {
            return Err(invalid("Anthropic delta targets a different content block"));
        }
        let delta = field(data, "delta")?;
        let kind = string(delta, "type")?;
        let block = self
            .active
            .as_mut()
            .ok_or_else(|| invalid("Anthropic delta has no open block"))?;
        let mut events = Vec::new();
        match (block, kind) {
            (Block::Text, "text_delta") => {
                let text = string(delta, "text")?;
                if !text.is_empty() {
                    events.push(ChatModelEvent::OutputTextDelta {
                        text: text.to_owned(),
                    });
                }
            }
            (Block::Thinking { text, signature }, "thinking_delta") => {
                if !signature.is_empty() {
                    return Err(invalid("Anthropic thinking text after signature"));
                }
                let delta = string(delta, "thinking")?;
                if text.len().saturating_add(delta.len()) > MAX_THINKING_TEXT_BYTES {
                    return Err(invalid("Anthropic thinking text limit exceeded"));
                }
                text.push_str(delta);
            }
            (Block::Thinking { signature, .. }, "signature_delta") => {
                let value = string(delta, "signature")?;
                if signature.len().saturating_add(value.len()) > MAX_SIGNATURE {
                    return Err(invalid("Anthropic thinking signature limit exceeded"));
                }
                signature.push_str(value);
            }
            (
                Block::Tool {
                    id,
                    name,
                    initial,
                    json,
                },
                "input_json_delta",
            ) => {
                if !initial.as_object().is_some_and(Map::is_empty) {
                    return Err(invalid(
                        "Anthropic tool supplied both initial input and JSON deltas",
                    ));
                }
                let text = string(delta, "partial_json")?;
                if json.len().saturating_add(text.len()) > MAX_ARGUMENTS {
                    return Err(invalid("Anthropic streamed arguments limit exceeded"));
                }
                json.push_str(text);
                if !text.is_empty() {
                    events.push(ChatModelEvent::ToolCallDelta {
                        call_id: ToolCallId(id.clone()),
                        name: name.clone(),
                        arguments_delta: text.to_owned(),
                    });
                }
            }
            _ => {
                return Err(unsupported(
                    "Anthropic delta cannot be represented by its active content block",
                ));
            }
        }
        Ok(events)
    }

    fn stop_block(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if index(data)? != self.next_index {
            return Err(invalid("Anthropic stop targets a different content block"));
        }
        let block = self
            .active
            .take()
            .ok_or_else(|| invalid("Anthropic block stopped more than once"))?;
        self.next_index += 1;
        Ok(match block {
            Block::Text => Vec::new(),
            Block::Thinking { text, signature } => {
                if signature.is_empty() {
                    self.incomplete_block = true;
                    return Ok(Vec::new());
                }
                vec![ChatModelEvent::ProviderReasoningBlock {
                    block: ChatProviderReasoning::AnthropicThinking { text, signature, route_digest: None },
                }]
            }
            Block::RedactedThinking { data } => vec![ChatModelEvent::ProviderReasoningBlock {
                block: ChatProviderReasoning::AnthropicRedactedThinking { data, route_digest: None },
            }],
            Block::Tool {
                id,
                name,
                initial,
                json,
            } => {
                let arguments = if json.is_empty() {
                    initial
                } else {
                    match serde_json::from_str(&json) {
                        Ok(value) => value,
                        Err(error) if error.is_eof() => {
                            self.incomplete_block = true;
                            return Ok(Vec::new());
                        }
                        Err(_) => {
                            return Err(invalid("Anthropic tool block ended with invalid JSON"));
                        }
                    }
                };
                if !arguments.is_object() {
                    return Err(invalid("Anthropic tool arguments must be an object"));
                }
                vec![ChatModelEvent::ToolCallCompleted {
                    call: ChatToolCall {
                        call_id: ToolCallId(id),
                        name,
                        arguments: StrictJsonValue(arguments),
                        provider_metadata: None,
                    },
                }]
            }
        })
    }

    fn merge_usage(&mut self, value: &Value) -> Result<(), ChatModelError> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("Invalid Anthropic usage object"))?;
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "input_tokens"
                    | "output_tokens"
                    | "cache_creation_input_tokens"
                    | "cache_read_input_tokens"
            ) {
                let number = value
                    .as_u64()
                    .ok_or_else(|| invalid("Invalid Anthropic token count"))?;
                if self
                    .usage
                    .get(key)
                    .and_then(Value::as_u64)
                    .is_some_and(|previous| number < previous)
                {
                    return Err(invalid("Anthropic cumulative usage decreased"));
                }
            }
            // Counts are cumulative snapshots, not independent billable deltas.
            self.usage.insert(key.clone(), value.clone());
        }
        Ok(())
    }
}

fn invalid(message: &str) -> ChatModelError {
    ChatModelError::protocol_violation(message)
}
fn unsupported(message: &str) -> ChatModelError {
    ChatModelError::new(
        ChatModelErrorCode::UnsupportedFeature,
        message,
        ChatRetryDirective::Never,
    )
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, ChatModelError> {
    value
        .get(key)
        .ok_or_else(|| invalid("Missing native Anthropic field"))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ChatModelError> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| invalid("Invalid native Anthropic string"))
}
fn identity<'a>(value: &'a Value, key: &str) -> Result<&'a str, ChatModelError> {
    let text = string(value, key)?;
    if text.is_empty()
        || text.trim() != text
        || text.len() > 256
        || text.chars().any(char::is_control)
    {
        return Err(invalid("Invalid native Anthropic identity"));
    }
    Ok(text)
}
fn index(value: &Value) -> Result<u64, ChatModelError> {
    field(value, "index")?
        .as_u64()
        .ok_or_else(|| invalid("Invalid Anthropic content block index"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_json_message_is_projected_through_the_native_lifecycle() {
        let mut decoder = AnthropicDecoder::default();
        let events = decoder
            .decode(&ProviderWireFrame {
                event: "json".into(),
                data: serde_json::json!({
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "hello"},
                        {"type": "tool_use", "id": "tool_1", "name": "search", "input": {"q": "x"}}
                    ],
                    "stop_reason": "tool_use",
                    "usage": {"input_tokens": 3, "output_tokens": 4}
                }),
            })
            .expect("complete JSON response is handled")
            .unwrap();
        assert!(matches!(events.first(), Some(ChatModelEvent::ResponseStarted { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            ChatModelEvent::OutputTextDelta { text } if text == "hello"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            ChatModelEvent::ToolCallCompleted { call } if call.name == "search"
        )));
        assert!(matches!(
            events.last(),
            Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })
        ));
    }
}
