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
    /// Current block type being accumulated
    pub current_block_type: Option<String>,
    /// Accumulated tool input JSON fragments
    pub tool_input_json: String,
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
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            current_block_type: None,
            tool_input_json: String::new(),
            tool_id: String::new(),
            tool_name: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        }
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
                }
            }
        }
    }

    StreamOutcome::Ok
}

/// Parse a single SSE data payload into zero or more LlmEvents
pub fn parse_sse_data(event_type: &str, data: &str, state: &mut StreamState) -> Vec<LlmEvent> {
    let mut events = Vec::new();

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return events,
    };

    match event_type {
        "message_start" => {
            if let Some(usage) = json.get("message").and_then(|m| m.get("usage")) {
                state.input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                state.cache_creation_tokens =
                    usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                state.cache_read_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            }
        }

        "content_block_start" => {
            let block = &json["content_block"];
            let block_type = block["type"].as_str().unwrap_or("");
            state.current_block_type = Some(block_type.to_string());

            if block_type == "tool_use" {
                state.tool_id = block["id"].as_str().unwrap_or("").to_string();
                state.tool_name = block["name"].as_str().unwrap_or("").to_string();
                state.tool_input_json.clear();
            }
        }

        "content_block_delta" => {
            let delta = &json["delta"];
            let delta_type = delta["type"].as_str().unwrap_or("");

            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta["text"].as_str() {
                        events.push(LlmEvent::TextDelta(text.to_string()));
                    }
                }
                "input_json_delta" => {
                    if let Some(partial) = delta["partial_json"].as_str() {
                        state.tool_input_json.push_str(partial);
                    }
                }
                "thinking_delta" => {
                    if let Some(thinking) = delta["thinking"].as_str() {
                        events.push(LlmEvent::ThinkingDelta(thinking.to_string()));
                    }
                }
                "signature_delta" => {
                    if let Some(signature) = delta["signature"].as_str() {
                        events.push(LlmEvent::ThinkingSignature(signature.to_string()));
                    }
                }
                _ => {}
            }
        }

        "content_block_stop" => {
            if state.current_block_type.as_deref() == Some("tool_use") {
                let input: Value = serde_json::from_str(&state.tool_input_json)
                    .unwrap_or(Value::Object(serde_json::Map::new()));
                events.push(LlmEvent::ToolUse {
                    id: state.tool_id.clone(),
                    name: state.tool_name.clone(),
                    input,
                    extra: None,
                });
                state.tool_input_json.clear();
            }
            state.current_block_type = None;
        }

        "message_delta" => {
            let delta = &json["delta"];
            let stop_reason = match delta["stop_reason"].as_str() {
                Some("end_turn") => StopReason::EndTurn,
                Some("tool_use") => StopReason::ToolUse,
                Some("max_tokens") => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };

            if let Some(usage) = json.get("usage") {
                state.output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
            }

            events.push(LlmEvent::Done {
                stop_reason,
                usage: TokenUsage {
                    input_tokens: state.input_tokens,
                    output_tokens: state.output_tokens,
                    cache_creation_tokens: state.cache_creation_tokens,
                    cache_read_tokens: state.cache_read_tokens,
                },
            });
        }

        "message_stop" => {
            // Stream complete, no action needed
        }

        "error" => {
            let msg = json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            events.push(LlmEvent::Error(msg.to_string()));
        }

        _ => {}
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    use nomi_types::tool::ToolDef;
    use serde_json::json;

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
                name: "SpawnTool".into(),
                description: "Spawn sub-agents".into(),
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
    fn test_parse_anthropic_event_text_delta() {
        // arrange
        let mut state = StreamState::new();
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
        let mut state = StreamState::new();
        // step 1: content_block_start with tool_use type
        let start_events = parse_sse_data(
            "content_block_start",
            r#"{"content_block":{"type":"tool_use","id":"id1","name":"bash"}}"#,
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
        // step 3: content_block_stop emits the ToolUse event
        let events = parse_sse_data("content_block_stop", r#"{}"#, &mut state);
        // assert
        assert_eq!(events.len(), 1);
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
    }

    #[test]
    fn test_parse_anthropic_event_stop() {
        // arrange
        let mut state = StreamState::new();
        let data = r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        // act
        let events = parse_sse_data("message_delta", data, &mut state);
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
    fn test_parse_anthropic_event_thinking() {
        // arrange
        let mut state = StreamState::new();
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
        let mut state = StreamState::new();
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
        let mut state = StreamState::new();
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
        let mut state = StreamState::new();
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
