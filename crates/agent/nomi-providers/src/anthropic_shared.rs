// Shared Anthropic message/tool building and SSE parsing logic.
// Used by AnthropicProvider, BedrockProvider, and VertexProvider.

use serde_json::{Value, json};
use tokio::sync::mpsc;

use nomi_types::llm::LlmEvent;
use nomi_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use nomi_types::tool::{ToolDef, truncate_deferred_description};

use super::ProviderError;
use nomi_config::compat::ProviderCompat;

/// Convert internal Message format to Anthropic API message format.
/// Compat flags control merging and alternation behavior.
pub fn build_messages(messages: &[Message], compat: &ProviderCompat) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();

    for msg in messages {
        let role_str = match msg.role {
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
            Role::System => continue, // system is top-level in Anthropic
        };

        let mut content: Vec<Value> = msg
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => json!({
                    "type": "text",
                    "text": text
                }),
                ContentBlock::ToolUse {
                    id, name, input, ..
                } => {
                    let tool_id = if id.is_empty() && compat.auto_tool_id() {
                        generate_tool_id()
                    } else {
                        id.clone()
                    };
                    json!({
                        "type": "tool_use",
                        "id": tool_id,
                        "name": name,
                        "input": input
                    })
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    images,
                } => {
                    if images.is_empty() {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                            "is_error": is_error
                        })
                    } else {
                        // Multimodal result: content becomes an array of
                        // image blocks followed by the text block.
                        let mut blocks: Vec<Value> = images
                            .iter()
                            .map(|img| {
                                json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": img.media_type,
                                        "data": img.data
                                    }
                                })
                            })
                            .collect();
                        if !content.is_empty() {
                            blocks.push(json!({ "type": "text", "text": content }));
                        }
                        json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": blocks,
                            "is_error": is_error
                        })
                    }
                }
                ContentBlock::Thinking {
                    thinking,
                    signature,
                } => {
                    let mut value = json!({
                        "type": "thinking",
                        "thinking": thinking
                    });
                    if let Some(signature) = signature {
                        value["signature"] = json!(signature);
                    }
                    value
                }
                ContentBlock::Image { media_type, data } => json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                }),
            })
            .collect();

        // Strip patterns from text content
        if let Some(patterns) = &compat.strip_patterns {
            for item in &mut content {
                if item["type"] == "text"
                    && let Some(text) = item["text"].as_str()
                {
                    let mut cleaned = text.to_string();
                    for pattern in patterns {
                        cleaned = cleaned.replace(pattern, "");
                    }
                    item["text"] = json!(cleaned);
                }
            }
        }

        // Merge consecutive messages with the same role (if enabled)
        if compat.merge_same_role()
            && let Some(last) = result.last_mut()
            && last["role"].as_str() == Some(role_str)
            && let Some(arr) = last["content"].as_array_mut()
        {
            arr.extend(content);
            continue;
        }

        result.push(json!({
            "role": role_str,
            "content": content
        }));
    }

    // Ensure user/assistant alternation (if enabled)
    if compat.ensure_alternation() {
        ensure_message_alternation(&mut result);
    }

    result
}

/// Insert filler messages to ensure strict user/assistant alternation.
fn ensure_message_alternation(messages: &mut Vec<Value>) {
    if messages.is_empty() {
        return;
    }

    // If first message is assistant, prepend a user filler
    if messages[0]["role"].as_str() == Some("assistant") {
        messages.insert(
            0,
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "."}]
            }),
        );
    }

    // Walk through and insert fillers where alternation is broken
    let mut i = 1;
    while i < messages.len() {
        let prev_role = messages[i - 1]["role"].as_str().unwrap_or("");
        let curr_role = messages[i]["role"].as_str().unwrap_or("");
        if prev_role == curr_role {
            let filler_role = if curr_role == "user" {
                "assistant"
            } else {
                "user"
            };
            messages.insert(
                i,
                json!({
                    "role": filler_role,
                    "content": [{"type": "text", "text": "."}]
                }),
            );
            i += 1; // skip the filler we just inserted
        }
        i += 1;
    }
}

/// Generate a unique tool ID when missing. UUIDv7 (time-ordered + random) is
/// collision-free even for ids produced within the same millisecond.
fn generate_tool_id() -> String {
    format!("toolu_{}", uuid::Uuid::now_v7().simple())
}

/// Convert internal ToolDef format to Anthropic API tool format.
/// Deferred tools emit a minimal schema to reduce input token usage;
/// the caller must invoke ToolSearch to retrieve the full schema.
pub fn build_tools(tools: &[ToolDef]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            if t.deferred {
                let short_desc = truncate_deferred_description(&t.description);
                json!({
                    "name": t.name,
                    "description": format!(
                        "(Deferred) {short_desc} — Use ToolSearch to load full schema before calling."
                    ),
                    "input_schema": {
                        "type": "object",
                        "properties": {}
                    }
                })
            } else {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema
                })
            }
        })
        .collect()
}

/// State machine for accumulating SSE content blocks
pub struct StreamState {
    /// A single valid message_start must precede all content and terminal
    /// events. This prevents an arbitrary suffix of a truncated/malformed stream
    /// from being treated as a complete tool turn.
    message_started: bool,
    /// Current block type being accumulated
    pub current_block_type: Option<String>,
    /// Accumulated tool input JSON fragments
    pub tool_input_json: String,
    /// Whether `tool_input_json` currently came from the `input` value on
    /// `content_block_start`. Official Anthropic streams start with `input: {}`
    /// and then send authoritative `input_json_delta` fragments; compatible
    /// providers sometimes put the complete input object in the start event.
    pub tool_input_from_start: bool,
    /// Tool use ID for current block
    pub tool_id: String,
    /// Tool name for current block
    pub tool_name: String,
    /// Input tokens from message_start
    pub input_tokens: u64,
    /// Output tokens accumulated
    pub output_tokens: u64,
    /// Cache creation tokens (prompt caching)
    pub cache_creation_tokens: u64,
    /// Cache read tokens (prompt caching)
    pub cache_read_tokens: u64,
    /// Fully parsed tool calls are held here until the provider confirms a
    /// successful `tool_use` terminal reason *and* closes the message with
    /// `message_stop`. Emitting them at `content_block_stop` or
    /// `message_delta` is too early: a later malformed/truncated tail must never
    /// leave an executable call in the engine.
    pending_tool_calls: Vec<LlmEvent>,
    /// Done is staged at `message_delta` and atomically released with any tool
    /// calls only when the protocol's `message_stop` commit marker arrives.
    pending_done: Option<LlmEvent>,
    /// Whether a valid `message_stop` commit marker has been observed.
    terminal_seen: bool,
    /// A protocol error was already emitted.  Pending calls are discarded and
    /// later events must not resurrect them.
    fatal_error: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            message_started: false,
            current_block_type: None,
            tool_input_json: String::new(),
            tool_input_from_start: false,
            tool_id: String::new(),
            tool_name: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            pending_tool_calls: Vec::new(),
            pending_done: None,
            terminal_seen: false,
            fatal_error: false,
        }
    }

    fn reset_current_block(&mut self) {
        self.current_block_type = None;
        self.tool_input_json.clear();
        self.tool_input_from_start = false;
        self.tool_id.clear();
        self.tool_name.clear();
    }

    fn protocol_error(&mut self, message: impl Into<String>) -> Vec<LlmEvent> {
        self.pending_tool_calls.clear();
        self.pending_done = None;
        self.reset_current_block();
        self.fatal_error = true;
        vec![LlmEvent::Error(message.into())]
    }

    pub(crate) fn terminal_seen(&self) -> bool {
        self.terminal_seen
    }

    pub(crate) fn fatal_error(&self) -> bool {
        self.fatal_error
    }
}

/// Outcome of SSE stream processing — distinguishes "failed before any content
/// was emitted" (safe to retry) from "failed after partial content" (not safe).
pub enum StreamOutcome {
    Ok,
    FailedEmpty(ProviderError),
    FailedPartial(ProviderError),
}

/// Find the earliest SSE event-block boundary (the blank line separating
/// events) in `buf`, returning its byte offset and delimiter length.
///
/// Supports LF (`\n\n`), CRLF (`\r\n\r\n`), and bare-CR (`\r\r`) framing so the
/// parser works against gateways that do not use the Anthropic API's `\n\n`
/// framing. Returns the boundary with the smallest offset; if a partial
/// delimiter sits at the end of the buffer (e.g. a chunk split mid-`\r\n\r\n`),
/// none match yet and the caller waits for more bytes — same as the original
/// `\n\n` logic.
fn find_sse_event_boundary(buf: &str) -> Option<(usize, usize)> {
    [
        buf.find("\r\n\r\n").map(|i| (i, 4)),
        buf.find("\n\n").map(|i| (i, 2)),
        buf.find("\r\r").map(|i| (i, 2)),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|&(offset, _)| offset)
}

/// Process the SSE stream from an Anthropic-compatible API
pub async fn process_sse_stream(
    response: reqwest::Response,
    tx: &mpsc::Sender<LlmEvent>,
) -> StreamOutcome {
    use futures::StreamExt;

    let mut state = StreamState::new();
    let mut buffer = String::new();
    let mut current_event_type = String::new();
    let mut stream = response.bytes_stream();
    let mut emitted_content = false;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let err = ProviderError::Connection(e.to_string());
                return if emitted_content {
                    StreamOutcome::FailedPartial(err)
                } else {
                    StreamOutcome::FailedEmpty(err)
                };
            }
        };
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        // Process complete SSE events. Per the SSE spec an event is terminated
        // by a blank line, which may be framed with LF ("\n\n"), CRLF
        // ("\r\n\r\n"), or bare CR ("\r\r"). The Anthropic official API uses
        // "\n\n", but some Anthropic-compatible gateways (e.g. new-api / one-api
        // proxies) emit "\r\n\r\n"; matching only "\n\n" there finds no event
        // boundary, parses zero events, and yields a silent empty response.
        while let Some((event_end, delim_len)) = find_sse_event_boundary(&buffer) {
            let event_block = buffer[..event_end].to_string();
            buffer = buffer[event_end + delim_len..].to_string();

            for line in event_block.lines() {
                if let Some(event_type) = line.strip_prefix("event: ") {
                    current_event_type = event_type.to_string();
                } else if let Some(data) = line.strip_prefix("data: ") {
                    tracing::debug!(target: "nomi_providers", chunk = %data, "sse chunk received");
                    let events = parse_sse_data(&current_event_type, data, &mut state);
                    for event in events {
                        if matches!(
                            event,
                            LlmEvent::TextDelta(_)
                                | LlmEvent::ThinkingDelta(_)
                                | LlmEvent::ThinkingSignature(_)
                                | LlmEvent::ToolUse { .. }
                        ) {
                            emitted_content = true;
                        }
                        if tx.send(event).await.is_err() {
                            return StreamOutcome::Ok; // receiver dropped
                        }
                    }
                    if state.fatal_error() || state.terminal_seen() {
                        // `message_stop` is the protocol commit point. Once it
                        // (or a protocol Error) is sent, a later socket reset
                        // must not change the outcome.
                        return StreamOutcome::Ok;
                    }
                }
            }
        }
    }

    if state.fatal_error() || state.terminal_seen() {
        StreamOutcome::Ok
    } else {
        // A closed HTTP body is not itself a successful Anthropic turn. Even a
        // complete `message_delta` can be followed by a malformed/truncated
        // tail; only `message_stop` commits the response.
        let error = ProviderError::Connection(
            "Anthropic-compatible stream ended before message_stop".to_string(),
        );
        if emitted_content {
            StreamOutcome::FailedPartial(error)
        } else {
            StreamOutcome::FailedEmpty(error)
        }
    }
}

fn event_requires_valid_json(event_type: &str) -> bool {
    matches!(
        event_type,
        "message_start"
            | "content_block_start"
            | "content_block_delta"
            | "content_block_stop"
            | "message_delta"
            | "message_stop"
            | "ping"
            | "error"
    )
}

/// Parse a single SSE data payload into zero or more LlmEvents.
///
/// Tool calls are deliberately *not* emitted at `content_block_stop`. They are
/// parsed and staged there. A later `message_delta` must confirm
/// `stop_reason: "tool_use"`, and only the final `message_stop` atomically
/// releases them. This keeps a complete-looking `{}` placeholder from escaping
/// when an argument delta was malformed, the stream was truncated, or the
/// actual terminal reason was `max_tokens`.
pub fn parse_sse_data(event_type: &str, data: &str, state: &mut StreamState) -> Vec<LlmEvent> {
    if state.fatal_error {
        return Vec::new();
    }

    if state.pending_done.is_some() && !matches!(event_type, "message_stop" | "ping") {
        return state.protocol_error(format!(
            "Anthropic-compatible provider emitted '{event_type}' after terminal message_delta but before message_stop"
        ));
    }

    let json: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(error) if event_requires_valid_json(event_type) => {
            return state.protocol_error(format!(
                "Anthropic-compatible provider returned malformed JSON for {event_type}: {error}"
            ));
        }
        Err(_) => return Vec::new(),
    };

    if let Some(payload_type) = json.get("type")
        && payload_type.as_str() != Some(event_type)
    {
        return state.protocol_error(format!(
            "Anthropic-compatible provider event '{event_type}' carried a mismatched payload type"
        ));
    }

    let requires_message_start = matches!(
        event_type,
        "content_block_start"
            | "content_block_delta"
            | "content_block_stop"
            | "message_delta"
            | "message_stop"
    );
    if requires_message_start && !state.message_started {
        return state.protocol_error(format!(
            "Anthropic-compatible provider emitted '{event_type}' before message_start"
        ));
    }

    match event_type {
        "message_start" => {
            if state.message_started {
                return state.protocol_error(
                    "Anthropic-compatible provider returned more than one message_start",
                );
            }
            let Some(message) = json.get("message").and_then(Value::as_object) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned message_start without an object message",
                );
            };
            state.message_started = true;
            if let Some(usage) = message.get("usage") {
                state.input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                state.cache_creation_tokens =
                    usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                state.cache_read_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            }
            Vec::new()
        }

        "content_block_start" => {
            if state.terminal_seen {
                return state.protocol_error(
                    "Anthropic-compatible provider started a content block after the terminal event",
                );
            }
            if state.current_block_type.is_some() {
                return state.protocol_error(
                    "Anthropic-compatible provider started a new content block before stopping the previous block",
                );
            }
            let Some(block) = json.get("content_block").and_then(Value::as_object) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned content_block_start without an object content_block",
                );
            };
            let Some(block_type) = block.get("type").and_then(Value::as_str) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned content_block_start without a block type",
                );
            };

            if block_type == "tool_use" {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let input = block.get("input");
                if input.is_some_and(|value| !value.is_object()) {
                    return state.protocol_error(format!(
                        "Anthropic-compatible provider returned non-object start input for tool '{name}' ({id})"
                    ));
                }

                state.tool_id = id;
                state.tool_name = name;
                state.tool_input_json.clear();
                state.tool_input_from_start = false;
                if let Some(input) = input {
                    // Official Anthropic streams put the placeholder `{}` here;
                    // compatible providers may put the complete object here.
                    // The first real delta remains authoritative and clears it.
                    state.tool_input_json = input.to_string();
                    state.tool_input_from_start = true;
                }
            }
            state.current_block_type = Some(block_type.to_string());
            Vec::new()
        }

        "content_block_delta" => {
            let Some(active_block_type) = state.current_block_type.clone() else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned a content block delta without an active block",
                );
            };
            let Some(delta) = json.get("delta").and_then(Value::as_object) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned content_block_delta without an object delta",
                );
            };
            let Some(delta_type) = delta.get("type").and_then(Value::as_str) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned content_block_delta without a delta type",
                );
            };

            match delta_type {
                "text_delta" if active_block_type != "text" => state.protocol_error(
                    "Anthropic-compatible provider returned text_delta outside a text block",
                ),
                "text_delta" => match delta.get("text").and_then(Value::as_str) {
                    Some(text) => vec![LlmEvent::TextDelta(text.to_string())],
                    None => state.protocol_error(
                        "Anthropic-compatible provider returned text_delta without string text",
                    ),
                },
                "input_json_delta" => {
                    if state.current_block_type.as_deref() != Some("tool_use") {
                        return state.protocol_error(
                            "Anthropic-compatible provider returned tool input outside a tool_use block",
                        );
                    }
                    let Some(partial) = delta.get("partial_json").and_then(Value::as_str) else {
                        return state.protocol_error(
                            "Anthropic-compatible provider returned input_json_delta without string partial_json",
                        );
                    };
                    if !partial.is_empty() {
                        if state.tool_input_from_start {
                            state.tool_input_json.clear();
                            state.tool_input_from_start = false;
                        }
                        state.tool_input_json.push_str(partial);
                    }
                    Vec::new()
                }
                "thinking_delta" | "signature_delta" if active_block_type != "thinking" => {
                    state.protocol_error(format!(
                        "Anthropic-compatible provider returned {delta_type} outside a thinking block"
                    ))
                }
                "thinking_delta" => match delta.get("thinking").and_then(Value::as_str) {
                    Some(thinking) => vec![LlmEvent::ThinkingDelta(thinking.to_string())],
                    None => state.protocol_error(
                        "Anthropic-compatible provider returned thinking_delta without string thinking",
                    ),
                },
                "signature_delta" => match delta.get("signature").and_then(Value::as_str) {
                    Some(signature) => vec![LlmEvent::ThinkingSignature(signature.to_string())],
                    None => state.protocol_error(
                        "Anthropic-compatible provider returned signature_delta without string signature",
                    ),
                },
                _ if state.current_block_type.as_deref() == Some("tool_use") => state
                    .protocol_error(format!(
                        "Anthropic-compatible provider returned unexpected '{delta_type}' inside a tool_use block"
                    )),
                _ => Vec::new(),
            }
        }

        "content_block_stop" => {
            let Some(block_type) = state.current_block_type.clone() else {
                return state.protocol_error(
                    "Anthropic-compatible provider stopped a content block that was never started",
                );
            };
            if block_type == "tool_use" {
                match crate::parse_tool_call_arguments(
                    "Anthropic-compatible provider",
                    &state.tool_name,
                    &state.tool_id,
                    &state.tool_input_json,
                ) {
                    Ok(input) => state.pending_tool_calls.push(LlmEvent::ToolUse {
                        id: state.tool_id.clone(),
                        name: state.tool_name.clone(),
                        input,
                        extra: None,
                    }),
                    Err(error) => return state.protocol_error(error),
                }
            }
            state.reset_current_block();
            Vec::new()
        }

        "message_delta" => {
            if state.terminal_seen || state.pending_done.is_some() {
                return state.protocol_error(
                    "Anthropic-compatible provider returned more than one terminal message_delta",
                );
            }
            let Some(delta) = json.get("delta").and_then(Value::as_object) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned message_delta without an object delta",
                );
            };
            let Some(stop_reason) = delta.get("stop_reason").and_then(Value::as_str) else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned message_delta without a stop_reason",
                );
            };
            if let Some(usage) = json.get("usage") {
                state.output_tokens = usage["output_tokens"]
                    .as_u64()
                    .unwrap_or(state.output_tokens);
            }
            let usage = TokenUsage {
                input_tokens: state.input_tokens,
                output_tokens: state.output_tokens,
                cache_creation_tokens: state.cache_creation_tokens,
                cache_read_tokens: state.cache_read_tokens,
            };

            match stop_reason {
                "tool_use" => {
                    if state.current_block_type.is_some() {
                        return state.protocol_error(
                            "Anthropic-compatible provider terminated with tool_use before stopping the active content block",
                        );
                    }
                    if state.pending_tool_calls.is_empty() {
                        return state.protocol_error(
                            "Anthropic-compatible provider terminated with tool_use but supplied no complete tool calls",
                        );
                    }
                    state.pending_done = Some(LlmEvent::Done {
                        stop_reason: StopReason::ToolUse,
                        usage,
                    });
                    Vec::new()
                }
                "end_turn" | "stop_sequence" => {
                    if state.current_block_type.is_some() || !state.pending_tool_calls.is_empty() {
                        return state.protocol_error(
                            "Anthropic-compatible provider ended the turn after supplying uncommitted tool calls",
                        );
                    }
                    state.pending_done = Some(LlmEvent::Done {
                        stop_reason: StopReason::EndTurn,
                        usage,
                    });
                    Vec::new()
                }
                "max_tokens" => {
                    // Even a syntactically complete earlier call belongs to a
                    // truncated response and must not execute. This mirrors the
                    // OpenAI `finish_reason: length` policy.
                    state.pending_tool_calls.clear();
                    state.reset_current_block();
                    state.pending_done = Some(LlmEvent::Done {
                        stop_reason: StopReason::MaxTokens,
                        usage,
                    });
                    Vec::new()
                }
                other => state.protocol_error(format!(
                    "Anthropic-compatible provider returned unsupported stop_reason '{other}'"
                )),
            }
        }

        "message_stop" => {
            if state.terminal_seen {
                return state.protocol_error(
                    "Anthropic-compatible provider returned more than one message_stop",
                );
            }
            if json.get("type").and_then(Value::as_str) != Some("message_stop") {
                return state.protocol_error(
                    "Anthropic-compatible provider returned message_stop without a matching payload type",
                );
            }
            if state.current_block_type.is_some() {
                return state.protocol_error(
                    "Anthropic-compatible provider stopped the message with an active content block",
                );
            }
            let Some(done) = state.pending_done.take() else {
                return state.protocol_error(
                    "Anthropic-compatible provider returned message_stop before terminal message_delta",
                );
            };

            let terminal_is_tool_use = matches!(
                done,
                LlmEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    ..
                }
            );
            if terminal_is_tool_use != !state.pending_tool_calls.is_empty() {
                return state.protocol_error(
                    "Anthropic-compatible provider terminal shape changed before message_stop",
                );
            }

            state.terminal_seen = true;
            let mut events = std::mem::take(&mut state.pending_tool_calls);
            events.push(done);
            events
        }

        "ping" => {
            if json.get("type").and_then(Value::as_str) != Some("ping") {
                return state.protocol_error(
                    "Anthropic-compatible provider returned ping without a matching payload type",
                );
            }
            Vec::new()
        }

        "error" => {
            let message = json
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown API error")
                .to_string();
            state.protocol_error(message)
        }

        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nomi_types::tool::ToolDef;
    use serde_json::json;

    fn started_state() -> StreamState {
        let mut state = StreamState::new();
        let events = parse_sse_data(
            "message_start",
            r#"{"type":"message_start","message":{"usage":{"input_tokens":1}}}"#,
            &mut state,
        );
        assert!(events.is_empty());
        state
    }

    fn start_content_block(state: &mut StreamState, block_type: &str) {
        let payload = json!({
            "type": "content_block_start",
            "content_block": {"type": block_type}
        })
        .to_string();
        assert!(parse_sse_data("content_block_start", &payload, state).is_empty());
    }

    #[test]
    fn generate_tool_id_is_unique_and_prefixed() {
        // Two ids generated back-to-back (same millisecond) must differ — the old
        // timestamp-hash scheme produced identical ids on same-ms collisions.
        let a = generate_tool_id();
        let b = generate_tool_id();
        assert_ne!(a, b, "tool ids must be unique even in quick succession");
        assert!(a.starts_with("toolu_"), "id: {a}");
    }

    /// Compat with merge but no alternation — matches pre-compat behavior
    fn default_compat() -> ProviderCompat {
        ProviderCompat {
            merge_same_role: Some(true),
            ..Default::default()
        }
    }

    // --- build_messages tests ---

    #[test]
    fn test_build_messages_text_only() {
        let messages = vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "Hello".to_string(),
            }],
        )];
        let result = build_messages(&messages, &default_compat());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Hello");
    }

    #[test]
    fn test_build_messages_with_tool_use() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                input: json!({"cmd": "ls"}),
                extra: None,
            }],
        )];
        let result = build_messages(&messages, &default_compat());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "assistant");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["id"], "call_1");
        assert_eq!(content[0]["name"], "bash");
        assert_eq!(content[0]["input"]["cmd"], "ls");
    }

    #[test]
    fn test_build_messages_with_tool_result() {
        let messages = vec![Message::new(
            Role::Tool,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "file list".to_string(),
                is_error: false,
                images: Vec::new(),
            }],
        )];
        let result = build_messages(&messages, &default_compat());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user"); // Tool maps to "user"
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[0]["content"], "file list");
        assert_eq!(content[0]["is_error"], false);
    }

    #[test]
    fn test_build_messages_tool_result_with_images() {
        let messages = vec![Message::new(
            Role::Tool,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "screenshot taken".to_string(),
                is_error: false,
                images: vec![nomi_types::tool::ToolImage {
                    media_type: "image/png".to_string(),
                    data: "aGVsbG8=".to_string(),
                }],
            }],
        )];
        let result = build_messages(&messages, &default_compat());
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        // content becomes a block array: image first, then text
        let blocks = content[0]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["type"], "base64");
        assert_eq!(blocks[0]["source"]["media_type"], "image/png");
        assert_eq!(blocks[0]["source"]["data"], "aGVsbG8=");
        assert_eq!(blocks[1]["type"], "text");
        assert_eq!(blocks[1]["text"], "screenshot taken");
    }

    #[test]
    fn test_build_messages_user_image_block() {
        let messages = vec![Message::new(
            Role::User,
            vec![
                ContentBlock::Text {
                    text: "What is in this image?".to_string(),
                },
                ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "aGVsbG8=".to_string(),
                },
            ],
        )];
        let result = build_messages(&messages, &default_compat());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        // First block is text
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "What is in this image?");
        // Second block is the image
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aGVsbG8=");
    }

    #[test]
    fn test_build_messages_with_thinking() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::Thinking {
                thinking: "Let me think...".to_string(),
                signature: None,
            }],
        )];
        let result = build_messages(&messages, &default_compat());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "assistant");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Let me think...");
    }

    #[test]
    fn test_build_messages_with_thinking_signature() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::Thinking {
                thinking: "Let me think...".to_string(),
                signature: Some("sig-123".to_string()),
            }],
        )];

        let result = build_messages(&messages, &default_compat());

        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "thinking");
        assert_eq!(content[0]["thinking"], "Let me think...");
        assert_eq!(content[0]["signature"], "sig-123");
    }

    // --- compat-driven behavior tests ---

    #[test]
    fn test_ensure_alternation_inserts_user_filler_before_assistant() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::Text { text: "hi".into() }],
        )];
        let compat = ProviderCompat {
            ensure_alternation: Some(true),
            merge_same_role: Some(true),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[1]["role"], "assistant");
    }

    #[test]
    fn test_ensure_alternation_disabled_no_filler() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::Text { text: "hi".into() }],
        )];
        let compat = ProviderCompat {
            ensure_alternation: Some(false),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "assistant");
    }

    #[test]
    fn test_merge_same_role_enabled_merges_consecutive_user() {
        let messages = vec![
            Message::new(Role::User, vec![ContentBlock::Text { text: "a".into() }]),
            Message::new(Role::User, vec![ContentBlock::Text { text: "b".into() }]),
        ];
        let compat = ProviderCompat {
            merge_same_role: Some(true),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        assert_eq!(result.len(), 1);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
    }

    #[test]
    fn test_merge_same_role_disabled_keeps_separate() {
        let messages = vec![
            Message::new(Role::User, vec![ContentBlock::Text { text: "a".into() }]),
            Message::new(Role::User, vec![ContentBlock::Text { text: "b".into() }]),
        ];
        let compat = ProviderCompat {
            merge_same_role: Some(false),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_auto_tool_id_generates_id_when_empty() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: String::new(),
                name: "bash".into(),
                input: json!({}),
                extra: None,
            }],
        )];
        let compat = ProviderCompat {
            auto_tool_id: Some(true),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        let content = result[0]["content"].as_array().unwrap();
        let id = content[0]["id"].as_str().unwrap();
        assert!(id.starts_with("toolu_"));
    }

    #[test]
    fn test_auto_tool_id_preserves_existing_id() {
        let messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "existing_id".into(),
                name: "bash".into(),
                input: json!({}),
                extra: None,
            }],
        )];
        let compat = ProviderCompat {
            auto_tool_id: Some(true),
            ..Default::default()
        };
        let result = build_messages(&messages, &compat);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["id"], "existing_id");
    }

    // --- build_tools tests ---

    #[test]
    fn test_build_tools_single() {
        // arrange
        let schema = json!({
            "type": "object",
            "properties": {
                "cmd": { "type": "string" }
            },
            "required": ["cmd"]
        });
        let tools = vec![ToolDef {
            name: "bash".to_string(),
            description: "Run a shell command".to_string(),
            input_schema: schema.clone(),
            deferred: false,
        }];
        // act
        let result = build_tools(&tools);
        // assert
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "bash");
        assert_eq!(result[0]["description"], "Run a shell command");
        assert_eq!(result[0]["input_schema"], schema);
    }

    #[test]
    fn test_build_tools_empty() {
        // arrange
        let tools: Vec<ToolDef> = vec![];
        // act
        let result = build_tools(&tools);
        // assert
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_tools_deferred_has_empty_schema() {
        let tools = vec![
            ToolDef {
                name: "Read".into(),
                description: "Read a file".into(),
                input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
                deferred: false,
            },
            ToolDef {
                name: "DelegateTool".into(),
                description: "Delegate tasks to Agents".into(),
                input_schema: json!({"type": "object", "properties": {"agents": {"type": "array"}}}),
                deferred: true,
            },
        ];
        let result = build_tools(&tools);

        // Core tool has full input_schema
        assert!(
            result[0]["input_schema"]["properties"]
                .get("path")
                .is_some()
        );

        // Deferred tool has empty input_schema and modified description
        assert!(
            result[1]["input_schema"]["properties"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        let desc = result[1]["description"].as_str().unwrap();
        assert!(desc.contains("ToolSearch"));
    }

    // --- parse_sse_data tests ---

    #[test]
    fn tool_sequence_without_message_start_cannot_commit() {
        let mut state = StreamState::new();
        let start = parse_sse_data(
            "content_block_start",
            r#"{"type":"content_block_start","content_block":{"type":"tool_use","id":"call_suffix","name":"update_base","input":{"kb_id":"kb_1"}}}"#,
            &mut state,
        );
        let delta = parse_sse_data(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            &mut state,
        );
        let stop = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );

        assert!(start.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(delta.is_empty());
        assert!(stop.is_empty());
        assert!(state.pending_tool_calls.is_empty());
    }

    #[test]
    fn duplicate_message_start_is_a_protocol_error() {
        let mut state = started_state();

        let events = parse_sse_data(
            "message_start",
            r#"{"type":"message_start","message":{"usage":{"input_tokens":2}}}"#,
            &mut state,
        );

        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("more than one message_start"))
        ));
        assert!(state.fatal_error());
    }

    #[test]
    fn content_delta_without_active_block_is_a_protocol_error() {
        let mut state = started_state();

        let events = parse_sse_data(
            "content_block_delta",
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"orphan"}}"#,
            &mut state,
        );

        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("without an active block"))
        ));
        assert!(state.fatal_error());
    }

    #[test]
    fn test_parse_anthropic_event_text_delta() {
        // arrange
        let mut state = started_state();
        start_content_block(&mut state, "text");
        let data = r#"{"delta":{"type":"text_delta","text":"Hello"}}"#;
        // act
        let events = parse_sse_data("content_block_delta", data, &mut state);
        // assert
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::TextDelta(t) => assert_eq!(t, "Hello"),
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn test_parse_anthropic_event_tool_use() {
        // arrange
        let mut state = started_state();
        // step 1: content_block_start with tool_use type
        let start_events = parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"id1","name":"bash","input":{}}}"#,
            &mut state,
        );
        assert!(start_events.is_empty());
        // step 2: content_block_delta with input_json_delta
        let delta_events = parse_sse_data(
            "content_block_delta",
            r#"{"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":\"ls\"}"}}"#,
            &mut state,
        );
        assert!(delta_events.is_empty());
        // step 3: a stopped block is staged, not executable yet
        let stopped = parse_sse_data("content_block_stop", r#"{}"#, &mut state);
        assert!(stopped.is_empty());
        // step 4: the successful terminal reason is still only staged
        let terminal = parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            &mut state,
        );
        assert!(terminal.is_empty());
        // step 5: message_stop atomically commits the call and Done
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );
        assert_eq!(events.len(), 2);
        match &events[0] {
            LlmEvent::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "id1");
                assert_eq!(name, "bash");
                assert_eq!(input["cmd"], "ls");
            }
            _ => panic!("expected ToolUse"),
        }
        assert!(matches!(
            events[1],
            LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                ..
            }
        ));
    }

    #[test]
    fn malformed_anthropic_tool_input_emits_error_not_tool_use() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_bad","name":"update_base","input":{}}}"#,
            &mut state,
        );
        parse_sse_data(
            "content_block_delta",
            r#"{"delta":{"type":"input_json_delta","partial_json":"{\"kb_id\":]"}}"#,
            &mut state,
        );

        let events = parse_sse_data("content_block_stop", r#"{}"#, &mut state);

        assert_eq!(events.len(), 1);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. })),
            "malformed input must never become an executable tool call"
        );
        match &events[0] {
            LlmEvent::Error(message) => {
                assert!(message.contains("malformed JSON arguments"));
                assert!(message.contains("update_base"));
                assert!(message.contains("call_bad"));
            }
            other => panic!("expected explicit Error, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_missing_tool_name_or_id_emits_error_not_tool_use() {
        for (id, name, expected) in [
            ("call_missing_name", "", "missing function name"),
            ("", "update_base", "without a call id"),
        ] {
            let mut state = started_state();
            let start = json!({
                "content_block": {
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": {}
                }
            })
            .to_string();
            parse_sse_data("content_block_start", &start, &mut state);

            let events = parse_sse_data("content_block_stop", r#"{}"#, &mut state);

            assert!(
                events
                    .iter()
                    .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
            );
            assert!(events.iter().any(
                |event| matches!(event, LlmEvent::Error(message) if message.contains(expected))
            ));
        }
    }

    #[test]
    fn anthropic_start_event_can_carry_complete_tool_input() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_full","name":"update_base","input":{"kb_id":"kb_1"}}}"#,
            &mut state,
        );

        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());
        assert!(parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"tool_use"}}"#,
            &mut state,
        )
        .is_empty());
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );

        match events.first() {
            Some(LlmEvent::ToolUse {
                id, name, input, ..
            }) => {
                assert_eq!(id, "call_full");
                assert_eq!(name, "update_base");
                assert_eq!(input["kb_id"], "kb_1");
            }
            other => panic!("expected one ToolUse with complete start input, got {other:?}"),
        }
    }

    #[test]
    fn anthropic_explicit_empty_object_remains_a_valid_tool_input() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_no_args","name":"list_bases","input":{}}}"#,
            &mut state,
        );

        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());
        assert!(parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"tool_use"}}"#,
            &mut state,
        )
        .is_empty());
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );

        match events.first() {
            Some(LlmEvent::ToolUse { input, .. }) => {
                assert_eq!(input, &json!({}));
            }
            other => panic!("expected a valid no-argument ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn terminal_tail_allows_only_valid_ping_before_message_stop() {
        for (ping, should_commit) in [
            (r#"{"type":"ping"}"#, true),
            (r#"{"type":"ping""#, false),
        ] {
            let mut state = started_state();
            parse_sse_data(
                "content_block_start",
                r#"{"type":"content_block_start","content_block":{"type":"tool_use","id":"call_ping","name":"list_bases","input":{}}}"#,
                &mut state,
            );
            parse_sse_data(
                "content_block_stop",
                r#"{"type":"content_block_stop"}"#,
                &mut state,
            );
            assert!(parse_sse_data(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
                &mut state,
            )
            .is_empty());

            let ping_events = parse_sse_data("ping", ping, &mut state);
            let terminal = parse_sse_data(
                "message_stop",
                r#"{"type":"message_stop"}"#,
                &mut state,
            );

            if should_commit {
                assert!(ping_events.is_empty());
                assert!(matches!(
                    terminal.as_slice(),
                    [LlmEvent::ToolUse { .. }, LlmEvent::Done {
                        stop_reason: StopReason::ToolUse,
                        ..
                    }]
                ));
            } else {
                assert!(ping_events
                    .iter()
                    .any(|event| matches!(event, LlmEvent::Error(_))));
                assert!(terminal.is_empty());
            }
        }
    }

    #[test]
    fn anthropic_max_tokens_discards_even_a_complete_staged_tool_call() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_complete","name":"update_base","input":{"kb_id":"kb_1"}}}"#,
            &mut state,
        );
        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());

        assert!(parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":99}}"#,
            &mut state,
        )
        .is_empty());
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );

        assert_eq!(events.len(), 1);
        assert!(events.iter().all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
        assert!(matches!(
            events[0],
            LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                ..
            }
        ));
    }

    #[test]
    fn malformed_recognized_tool_delta_poison_cannot_fall_back_to_start_placeholder() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_bad_delta","name":"update_base","input":{}}}"#,
            &mut state,
        );

        let malformed = parse_sse_data(
            "content_block_delta",
            r#"{"delta":{"type":"input_json_delta","partial_json":"{\"kb_id\":\"kb_1\"}""#,
            &mut state,
        );
        assert!(malformed.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(malformed.iter().all(|event| !matches!(event, LlmEvent::ToolUse { .. })));

        // Later block/terminal events cannot resurrect the `{}` from start.
        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());
        assert!(
            parse_sse_data(
                "message_delta",
                r#"{"delta":{"stop_reason":"tool_use"}}"#,
                &mut state,
            )
            .is_empty()
        );
    }

    #[test]
    fn malformed_tool_delta_shape_is_an_error_not_an_empty_call() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_bad_shape","name":"update_base","input":{}}}"#,
            &mut state,
        );

        let events = parse_sse_data(
            "content_block_delta",
            r#"{"delta":{"type":"input_json_delta","partial_json":null}}"#,
            &mut state,
        );

        assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(events.iter().all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
    }

    #[test]
    fn end_turn_cannot_commit_a_staged_tool_call() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_wrong_terminal","name":"list_bases","input":{}}}"#,
            &mut state,
        );
        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());

        let events = parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"end_turn"}}"#,
            &mut state,
        );

        assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(events.iter().all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
    }

    #[tokio::test]
    async fn clean_eof_without_terminal_never_releases_a_staged_tool_call() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_eof\",\"name\":\"update_base\",\"input\":{\"kb_id\":\"kb_1\"}}}\n\n",
            "event: content_block_stop\n",
            "data: {}\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx).await;
        drop(tx);

        assert!(matches!(outcome, StreamOutcome::FailedEmpty(_)));
        while let Some(event) = rx.recv().await {
            assert!(!matches!(event, LlmEvent::ToolUse { .. }));
        }
    }

    #[tokio::test]
    async fn eof_after_tool_use_message_delta_never_commits_the_staged_call() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_eof_terminal\",\"name\":\"update_base\",\"input\":{\"kb_id\":\"kb_1\"}}}\n\n",
            "event: content_block_stop\n",
            "data: {}\n\n",
            "event: message_delta\n",
            "data: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::FailedEmpty(_)));
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::ToolUse { .. } | LlmEvent::Done { .. }
        )));
    }

    #[tokio::test]
    async fn malformed_tail_after_tool_use_message_delta_discards_the_staged_call() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_bad_tail\",\"name\":\"update_base\",\"input\":{\"kb_id\":\"kb_1\"}}}\n\n",
            "event: content_block_stop\n",
            "data: {}\n\n",
            "event: message_delta\n",
            "data: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"illegal tail\"}}\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::ToolUse { .. } | LlmEvent::Done { .. }
        )));
    }

    #[tokio::test]
    async fn clean_message_stop_atomically_commits_tool_call_and_done() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
            "event: content_block_start\n",
            "data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_commit\",\"name\":\"update_base\",\"input\":{\"kb_id\":\"kb_1\"}}}\n\n",
            "event: content_block_stop\n",
            "data: {}\n\n",
            "event: message_delta\n",
            "data: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert!(matches!(events.first(), Some(LlmEvent::ToolUse { id, .. }) if id == "call_commit"));
        assert!(matches!(
            events.last(),
            Some(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage
            }) if usage.output_tokens == 7
        ));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_parse_anthropic_event_stop() {
        // arrange
        let mut state = started_state();
        let data = r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        // act: message_delta stages; message_stop commits.
        assert!(parse_sse_data("message_delta", data, &mut state).is_empty());
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );
        // assert
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::Done { stop_reason, usage } => {
                assert_eq!(*stop_reason, StopReason::EndTurn);
                assert_eq!(usage.output_tokens, 42);
            }
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn anthropic_stop_sequence_is_a_clean_end_turn() {
        let mut state = started_state();
        assert!(parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"stop_sequence"},"usage":{"output_tokens":5}}"#,
            &mut state,
        )
        .is_empty());
        let events = parse_sse_data(
            "message_stop",
            r#"{"type":"message_stop"}"#,
            &mut state,
        );

        assert!(matches!(
            events.as_slice(),
            [LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                ..
            }]
        ));
    }

    #[test]
    fn unsupported_anthropic_stop_reasons_fail_closed_without_tools() {
        for reason in ["pause_turn", "refusal", "future_reason"] {
            let mut state = started_state();
            let data = json!({ "delta": { "stop_reason": reason } }).to_string();
            let events = parse_sse_data("message_delta", &data, &mut state);

            assert!(
                events.iter().any(
                    |event| matches!(event, LlmEvent::Error(message) if message.contains(reason))
                ),
                "{reason} must surface an explicit provider error"
            );
            assert!(events.iter().all(|event| !matches!(
                event,
                LlmEvent::Done { .. } | LlmEvent::ToolUse { .. }
            )));
            assert!(state.fatal_error());
        }
    }

    #[test]
    fn unsupported_anthropic_stop_reason_discards_a_staged_call() {
        let mut state = started_state();
        parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"call_pause","name":"update_base","input":{"kb_id":"kb_1"}}}"#,
            &mut state,
        );
        assert!(parse_sse_data("content_block_stop", r#"{}"#, &mut state).is_empty());

        let events = parse_sse_data(
            "message_delta",
            r#"{"delta":{"stop_reason":"pause_turn"}}"#,
            &mut state,
        );

        assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::Done { .. } | LlmEvent::ToolUse { .. }
        )));
    }

    #[test]
    fn test_parse_anthropic_event_thinking() {
        // arrange
        let mut state = started_state();
        start_content_block(&mut state, "thinking");
        let data = r#"{"delta":{"type":"thinking_delta","thinking":"reasoning step"}}"#;
        // act
        let events = parse_sse_data("content_block_delta", data, &mut state);
        // assert
        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::ThinkingDelta(t) => assert_eq!(t, "reasoning step"),
            _ => panic!("expected ThinkingDelta"),
        }
    }

    #[test]
    fn test_parse_anthropic_event_thinking_signature() {
        let mut state = started_state();
        start_content_block(&mut state, "thinking");
        let data = r#"{"delta":{"type":"signature_delta","signature":"sig-123"}}"#;

        let events = parse_sse_data("content_block_delta", data, &mut state);

        assert_eq!(events.len(), 1);
        match &events[0] {
            LlmEvent::ThinkingSignature(signature) => assert_eq!(signature, "sig-123"),
            _ => panic!("expected ThinkingSignature"),
        }
    }

    #[test]
    fn test_parse_anthropic_event_unknown_type() {
        // arrange
        let mut state = started_state();
        let data = r#"{}"#;
        // act
        let events = parse_sse_data("unknown_event", data, &mut state);
        // assert
        assert!(events.is_empty());
    }

    // --- find_sse_event_boundary tests ---

    #[test]
    fn test_sse_boundary_lf() {
        // LF framing: "event: x\ndata: y\n\n..."
        assert_eq!(find_sse_event_boundary("a\n\nb"), Some((1, 2)));
    }

    #[test]
    fn test_sse_boundary_crlf() {
        // CRLF framing used by some gateways: "...\r\n\r\n..."
        assert_eq!(find_sse_event_boundary("a\r\n\r\nb"), Some((1, 4)));
    }

    #[test]
    fn test_sse_boundary_cr() {
        assert_eq!(find_sse_event_boundary("a\r\rb"), Some((1, 2)));
    }

    #[test]
    fn test_sse_boundary_none_when_incomplete() {
        // A chunk split mid-delimiter must not match yet.
        assert_eq!(find_sse_event_boundary("data: {}\r\n\r"), None);
        assert_eq!(find_sse_event_boundary("data: {}\n"), None);
    }

    #[test]
    fn test_sse_boundary_picks_earliest() {
        // When both framings appear, the earliest offset wins.
        let buf = "a\n\nb\r\n\r\nc";
        assert_eq!(find_sse_event_boundary(buf), Some((1, 2)));
    }

    #[test]
    fn test_crlf_event_block_parses_text_delta() {
        // End-to-end: a CRLF-framed event block yields a TextDelta, proving the
        // boundary fix lets `.lines()` split the inner CRLF lines correctly.
        let mut state = started_state();
        start_content_block(&mut state, "text");
        let buf = "event: content_block_delta\r\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\r\n\r\n";
        let (end, len) = find_sse_event_boundary(buf).expect("boundary found");
        let block = &buf[..end];
        let mut event_type = String::new();
        let mut got = Vec::new();
        for line in block.lines() {
            if let Some(t) = line.strip_prefix("event: ") {
                event_type = t.to_string();
            } else if let Some(data) = line.strip_prefix("data: ") {
                got = parse_sse_data(&event_type, data, &mut state);
            }
        }
        assert_eq!(len, 4);
        assert_eq!(got.len(), 1);
        match &got[0] {
            LlmEvent::TextDelta(t) => assert_eq!(t, "hi"),
            _ => panic!("expected TextDelta from CRLF-framed event"),
        }
    }
}
