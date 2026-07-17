use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use nomi_config::compat::{self, ProviderCompat};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use nomi_types::tool::{ToolDef, truncate_deferred_description};

use crate::anthropic_shared::StreamOutcome;
use crate::{LlmProvider, ProviderError};

/// Bound sparse provider indices before they reach `Vec` growth. A malformed
/// OpenAI-compatible stream can otherwise request an enormous index in a tiny
/// payload and exhaust the process before terminal validation runs.
const MAX_STRUCTURED_TOOL_CALLS_PER_TURN: usize = 128;

pub struct OpenAIProvider {
    api_keys: Vec<String>,
    current_api_key: AtomicUsize,
    base_url: String,
    compat: ProviderCompat,
    sanitize_tool_schemas: AtomicBool,
}

impl OpenAIProvider {
    pub fn new(api_key: &str, base_url: &str, compat: ProviderCompat) -> Self {
        Self {
            api_keys: crate::parse_api_keys(api_key),
            current_api_key: AtomicUsize::new(0),
            base_url: base_url.to_string(),
            compat,
            sanitize_tool_schemas: AtomicBool::new(false),
        }
    }

    fn should_sanitize_tool_schemas(&self) -> bool {
        self.compat.sanitize_schema() || self.sanitize_tool_schemas.load(Ordering::Acquire)
    }

    fn build_headers(api_key: &str) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        let bearer = format!("Bearer {api_key}");
        let auth = HeaderValue::from_str(&bearer).map_err(|e| {
            ProviderError::Connection(format!("Invalid authorization header: {}", e))
        })?;
        headers.insert(AUTHORIZATION, auth);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    fn build_messages(
        messages: &[Message],
        system: &str,
        compat: &ProviderCompat,
        require_reasoning_content: bool,
    ) -> Vec<Value> {
        let mut result: Vec<Value> = Vec::new();

        // Check if any assistant message in the conversation has thinking content.
        // If so, DeepSeek API requires ALL assistant messages to include
        // reasoning_content (even if empty string).
        let has_any_thinking = messages.iter().any(|m| {
            m.role == Role::Assistant
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Thinking { .. }))
        });

        // System message first
        if !system.is_empty() {
            result.push(json!({
                "role": "system",
                "content": system
            }));
        }

        for msg in messages {
            match msg.role {
                Role::User => {
                    // Check if this contains tool results
                    let has_tool_results = msg
                        .content
                        .iter()
                        .any(|b| matches!(b, ContentBlock::ToolResult { .. }));

                    if has_tool_results {
                        // Each tool result becomes a separate "tool" role message.
                        // The OpenAI wire format has no is_error flag, so failed
                        // results are prefixed textually — otherwise the model
                        // can't tell a tool error from successful output.
                        for block in &msg.content {
                            if let ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                                images,
                            } = block
                            {
                                let content = if *is_error {
                                    format!("[tool error] {content}")
                                } else {
                                    content.clone()
                                };
                                result.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content
                                }));
                                if let Some(img_msg) = tool_images_user_message(
                                    tool_use_id,
                                    images,
                                    compat.supports_image(),
                                ) {
                                    result.push(img_msg);
                                }
                            }
                        }
                    } else {
                        // Check if the message contains any image blocks
                        let has_images = msg
                            .content
                            .iter()
                            .any(|b| matches!(b, ContentBlock::Image { .. }));

                        if has_images {
                            // Multimodal user message: build content array with
                            // text and image_url parts.
                            let mut parts: Vec<Value> = Vec::new();
                            let mut stripped_images = 0usize;
                            for block in &msg.content {
                                match block {
                                    ContentBlock::Text { text } => {
                                        let text = strip_patterns_from_text(text, compat);
                                        if !text.is_empty() {
                                            parts.push(json!({
                                                "type": "text",
                                                "text": text
                                            }));
                                        }
                                    }
                                    ContentBlock::Image { media_type, data } => {
                                        if compat.supports_image() {
                                            parts.push(json!({
                                                "type": "image_url",
                                                "image_url": {
                                                    "url": format!("data:{media_type};base64,{data}")
                                                }
                                            }));
                                        } else {
                                            stripped_images += 1;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if stripped_images > 0 {
                                parts.push(json!({
                                    "type": "text",
                                    "text": "[图片已省略：当前模型不支持图片输入]"
                                }));
                            }
                            result.push(json!({
                                "role": "user",
                                "content": parts
                            }));
                        } else {
                            let text: String = msg
                                .content
                                .iter()
                                .filter_map(|b| {
                                    if let ContentBlock::Text { text } = b {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            let text = strip_patterns_from_text(&text, compat);
                            result.push(json!({
                                "role": "user",
                                "content": text
                            }));
                        }
                    }
                }
                Role::Assistant => {
                    let mut msg_json = json!({ "role": "assistant" });

                    // Preserve reasoning_content for models with thinking mode
                    // (e.g. DeepSeek Reasoner, Kimi K2.5). The API requires
                    // ALL assistant messages to include reasoning_content once
                    // any message in the conversation has it.
                    let thinking: String = msg
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Thinking { thinking, .. } = b {
                                Some(thinking.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    if has_any_thinking || require_reasoning_content {
                        // OpenCode's DeepSeek free endpoint rejects some
                        // multi-turn tool histories when an assistant turn has
                        // no reasoning_content. A single space is intentional:
                        // unlike "", it is accepted as a non-empty placeholder
                        // when persisted/compacted history lost the original
                        // thinking block.
                        let reasoning_content = if require_reasoning_content && thinking.is_empty() {
                            " ".to_owned()
                        } else {
                            thinking
                        };
                        msg_json["reasoning_content"] = json!(reasoning_content);
                    }

                    let text: String = msg
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let text = strip_patterns_from_text(&text, compat);

                    let tool_calls: Vec<Value> = msg
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::ToolUse {
                                id,
                                name,
                                input,
                                extra,
                            } = b
                            {
                                let mut tc_json = json!({
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": serde_json::to_string(input).unwrap_or_default()
                                    }
                                });
                                if let Some(extra_val) = extra {
                                    tc_json["extra_content"] = extra_val.clone();
                                }
                                Some(tc_json)
                            } else {
                                None
                            }
                        })
                        .collect();

                    if !text.is_empty() {
                        msg_json["content"] = json!(text);
                    } else if tool_calls.is_empty() {
                        msg_json["content"] = json!("");
                    }

                    if !tool_calls.is_empty() {
                        msg_json["tool_calls"] = json!(tool_calls);
                    }

                    result.push(msg_json);
                }
                Role::System => {
                    // Already handled above
                }
                Role::Tool => {
                    for block in &msg.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                            images,
                        } = block
                        {
                            let content = if *is_error {
                                format!("[tool error] {content}")
                            } else {
                                content.clone()
                            };
                            result.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content
                            }));
                            if let Some(img_msg) = tool_images_user_message(
                                tool_use_id,
                                images,
                                compat.supports_image(),
                            ) {
                                result.push(img_msg);
                            }
                        }
                    }
                }
            }
        }

        // Dedup tool results: keep last occurrence of each tool_call_id
        if compat.dedup_tool_results() {
            dedup_tool_results(&mut result);
        }

        // Clean orphan tool calls: remove tool_call entries with no matching tool result
        if compat.clean_orphan_tool_calls() {
            clean_orphaned_tool_calls(&mut result);
        }

        // Merge consecutive assistant messages
        if compat.merge_assistant_messages() {
            merge_consecutive_assistant(&mut result);
        }

        result
    }

    fn build_tools(tools: &[ToolDef], sanitize: bool) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                if t.deferred {
                    let short_desc = truncate_deferred_description(&t.description);
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": format!(
                                "(Deferred) {short_desc} — Use ToolSearch to load full schema before calling."
                            ),
                            "parameters": {
                                "type": "object",
                                "properties": {}
                            }
                        }
                    })
                } else {
                    let parameters = if sanitize {
                        compat::sanitize_json_schema(&t.input_schema)
                    } else {
                        t.input_schema.clone()
                    };
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": parameters
                        }
                    })
                }
            })
            .collect()
    }

    fn build_request_body(
        &self,
        request: &LlmRequest,
        sanitize_tool_schemas: bool,
    ) -> Value {
        let max_tokens_field = self
            .compat
            .max_tokens_field
            .as_deref()
            .unwrap_or("max_tokens");

        let mut body = json!({
            "model": request.model,
            "messages": Self::build_messages(
                &request.messages,
                &request.system,
                &self.compat,
                self.compat.require_reasoning_content(),
            ),
            "stream": true,
            "stream_options": { "include_usage": true }
        });
        body[max_tokens_field] = json!(request.max_tokens);

        if !request.tools.is_empty() {
            body["tools"] = json!(Self::build_tools(
                &request.tools,
                sanitize_tool_schemas,
            ));
        }

        if let Some(effort) = &request.reasoning_effort {
            body["reasoning_effort"] = json!(effort);
        }

        body
    }

    async fn send_initial(
        client: &reqwest::Client,
        url: &str,
        headers: &HeaderMap,
        body: &Value,
    ) -> Result<reqwest::Response, ProviderError> {
        crate::retry::with_initial_request_retry(|| async {
            let response = client
                .post(url)
                .headers(headers.clone())
                .json(body)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            let retry_after_ms = crate::parse_retry_after_ms(response.headers()).unwrap_or(5000);
            let body_text = response.text().await.unwrap_or_default();
            if status.as_u16() == 429 {
                return Err(ProviderError::RateLimited {
                    retry_after_ms,
                    message: crate::non_empty_rate_limit_message(body_text),
                });
            }
            Err(ProviderError::Api {
                status: status.as_u16(),
                message: body_text,
            })
        })
        .await
    }

    async fn send_initial_with_key_rotation(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &Value,
    ) -> Result<(reqwest::Response, HeaderMap), ProviderError> {
        let mut last_error = None;
        let key_count = self.api_keys.len();
        let start_index = self.current_api_key.load(Ordering::Acquire) % key_count.max(1);

        for offset in 0..key_count {
            let index = (start_index + offset) % key_count;
            let api_key = &self.api_keys[index];
            let headers = Self::build_headers(api_key)?;
            match Self::send_initial(client, url, &headers, body).await {
                Ok(response) => {
                    self.current_api_key.store(index, Ordering::Release);
                    return Ok((response, headers));
                }
                Err(error) if crate::is_api_key_rotation_error(&error) && offset + 1 < key_count => {
                    let next_index = (index + 1) % key_count;
                    tracing::warn!(
                        target: "nomi_providers",
                        provider = "openai",
                        key_index = index + 1,
                        key_count = self.api_keys.len(),
                        error = %error,
                        "provider rejected API key; trying the next configured key"
                    );
                    self.current_api_key.store(next_index, Ordering::Release);
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ProviderError::Connection("No usable API key configured".to_owned())
        }))
    }
}

/// Generate a unique tool call ID in OpenAI `call_xxx` format. UUIDv7
/// (time-ordered + random) is collision-free even within the same instant.
fn generate_call_id() -> String {
    format!("call_{}", uuid::Uuid::now_v7().simple())
}

/// Build a follow-up user message carrying a tool result's images.
///
/// The OpenAI wire format only allows string content in `tool` role
/// messages, so images ride in a separate user message right after the
/// tool result, labelled with the originating call id.
fn tool_images_user_message(
    tool_use_id: &str,
    images: &[nomi_types::tool::ToolImage],
    supports_image: bool,
) -> Option<Value> {
    if images.is_empty() || !supports_image {
        return None;
    }
    let mut parts: Vec<Value> = vec![json!({
        "type": "text",
        "text": format!("[images from tool call {tool_use_id}]")
    })];
    parts.extend(images.iter().map(|img| {
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", img.media_type, img.data) }
        })
    }));
    Some(json!({ "role": "user", "content": parts }))
}

/// Strip configured patterns from text content
fn strip_patterns_from_text(text: &str, compat: &ProviderCompat) -> String {
    match &compat.strip_patterns {
        Some(patterns) if !patterns.is_empty() => {
            let mut result = text.to_string();
            for pattern in patterns {
                result = result.replace(pattern, "");
            }
            result
        }
        _ => text.to_string(),
    }
}

/// Deduplicate tool results: keep last occurrence of each tool_call_id
fn dedup_tool_results(messages: &mut Vec<Value>) {
    use std::collections::HashMap;

    // Find the last index of each tool_call_id
    let mut last_index: HashMap<String, usize> = HashMap::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg["role"].as_str() == Some("tool")
            && let Some(id) = msg["tool_call_id"].as_str()
        {
            last_index.insert(id.to_string(), i);
        }
    }

    // Keep only the last occurrence
    let mut seen: HashMap<String, bool> = HashMap::new();
    let mut to_remove = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if msg["role"].as_str() == Some("tool")
            && let Some(id) = msg["tool_call_id"].as_str()
            && let Some(&last_i) = last_index.get(id)
        {
            if i != last_i && !seen.contains_key(id) {
                to_remove.push(i);
            }
            if i == last_i {
                seen.insert(id.to_string(), true);
            }
        }
    }

    // Remove in reverse order to preserve indices
    for i in to_remove.into_iter().rev() {
        messages.remove(i);
    }
}

/// Remove tool_call entries from assistant messages that have no corresponding tool result
fn clean_orphaned_tool_calls(messages: &mut [Value]) {
    use std::collections::HashSet;

    let answered_ids: HashSet<String> = messages
        .iter()
        .filter(|m| m["role"].as_str() == Some("tool"))
        .filter_map(|m| m["tool_call_id"].as_str().map(String::from))
        .collect();

    for msg in messages.iter_mut() {
        if msg["role"].as_str() == Some("assistant")
            && let Some(tcs) = msg["tool_calls"].as_array_mut()
        {
            tcs.retain(|tc| {
                tc["id"]
                    .as_str()
                    .map(|id| answered_ids.contains(id))
                    .unwrap_or(true)
            });
            if tcs.is_empty() {
                msg.as_object_mut().unwrap().remove("tool_calls");
            }
        }
    }
}

/// Merge consecutive assistant messages into one
fn merge_consecutive_assistant(messages: &mut Vec<Value>) {
    let mut i = 0;
    while i + 1 < messages.len() {
        if messages[i]["role"].as_str() == Some("assistant")
            && messages[i + 1]["role"].as_str() == Some("assistant")
        {
            let next = messages.remove(i + 1);

            // Merge text content
            let curr_text = messages[i]["content"].as_str().unwrap_or("").to_string();
            let next_text = next["content"].as_str().unwrap_or("").to_string();
            let merged_text = match (curr_text.is_empty(), next_text.is_empty()) {
                (true, true) => String::new(),
                (true, false) => next_text,
                (false, true) => curr_text,
                (false, false) => format!("{}{}", curr_text, next_text),
            };

            if !merged_text.is_empty() {
                messages[i]["content"] = json!(merged_text);
            }

            // Merge reasoning_content
            let curr_rc = messages[i]["reasoning_content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let next_rc = next["reasoning_content"].as_str().unwrap_or("").to_string();
            let merged_rc = match (curr_rc.is_empty(), next_rc.is_empty()) {
                (true, true) => String::new(),
                (true, false) => next_rc,
                (false, true) => curr_rc,
                (false, false) => format!("{}{}", curr_rc, next_rc),
            };

            if !merged_rc.is_empty() {
                messages[i]["reasoning_content"] = json!(merged_rc);
            }

            // Merge tool_calls
            if let Some(next_tcs) = next["tool_calls"].as_array() {
                let curr_tcs = messages[i]
                    .as_object_mut()
                    .unwrap()
                    .entry("tool_calls")
                    .or_insert_with(|| json!([]));
                if let Some(arr) = curr_tcs.as_array_mut() {
                    arr.extend(next_tcs.iter().cloned());
                }
            }

            // Don't increment i - check the merged result against the next message
        } else {
            i += 1;
        }
    }
}

/// State for accumulating tool call deltas by index
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
    extra: Option<Value>,
    announced: bool,
    last_progress_signature: String,
}

struct StreamState {
    tool_calls: Vec<ToolCallAccumulator>,
    input_tokens: u64,
    output_tokens: u64,
    /// Cache-read (prompt-cache hit) tokens reported by the provider, if any.
    /// Informational: surfaced into the Done event's usage so the cache-hit rate
    /// is observable for domestic OpenAI-compatible providers (DeepSeek/GLM/Qwen/…)
    /// that do automatic prefix caching. 0 when the provider reports none.
    cache_read_tokens: u64,
    /// Deferred Done event: populated when finish_reason arrives, emitted on
    /// [DONE] so the final usage-only chunk has a chance to update token counts.
    pending_done: Option<LlmEvent>,
    /// Final structured calls are staged alongside Done and atomically released
    /// only after the legal usage-only tail and explicit `[DONE]`. A clean EOF
    /// may complete a non-tool terminal but never commits side-effecting calls.
    pending_tool_calls: Vec<LlmEvent>,
    /// Once finish_reason appears, only usage-only metadata may follow before
    /// `[DONE]`; content, tool deltas, or a second finish poison the turn.
    finish_seen: bool,
    /// A malformed SSE payload makes the rest of the provider turn
    /// untrustworthy. Once poisoned, no later chunk or `[DONE]` sentinel may
    /// resurrect accumulated calls or commit a terminal Done.
    fatal_error: bool,
}

impl StreamState {
    fn new() -> Self {
        Self {
            tool_calls: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            pending_done: None,
            pending_tool_calls: Vec::new(),
            finish_seen: false,
            fatal_error: false,
        }
    }

    fn poison(&mut self, message: impl Into<String>) -> Vec<LlmEvent> {
        self.tool_calls.clear();
        self.pending_tool_calls.clear();
        self.pending_done = None;
        self.fatal_error = true;
        vec![LlmEvent::Error(message.into())]
    }

    fn fatal_error(&self) -> bool {
        self.fatal_error
    }

    /// Atomically emit staged structured calls and the deferred Done event with
    /// up-to-date token counts.
    ///
    /// OpenAI sends usage in a separate trailing chunk (choices:[]) *after* the
    /// chunk that carries `finish_reason`. We defer the Done event until [DONE]
    /// so that token counts are always accurate.
    fn drain_terminal_events(&mut self) -> Vec<LlmEvent> {
        if self.fatal_error {
            self.pending_tool_calls.clear();
            self.pending_done = None;
            return Vec::new();
        }
        let Some(pending) = self.pending_done.take() else {
            self.pending_tool_calls.clear();
            return Vec::new();
        };
        let done = match pending {
            LlmEvent::Done { stop_reason, .. } => LlmEvent::Done {
                stop_reason,
                usage: TokenUsage {
                    input_tokens: self.input_tokens,
                    output_tokens: self.output_tokens,
                    cache_creation_tokens: 0,
                    cache_read_tokens: self.cache_read_tokens,
                },
            },
            other => other,
        };
        let mut events = std::mem::take(&mut self.pending_tool_calls);
        events.push(done);
        events
    }

    #[cfg(test)]
    fn flush_done(&mut self) -> Option<LlmEvent> {
        self.drain_terminal_events()
            .into_iter()
            .find(|event| matches!(event, LlmEvent::Done { .. }))
    }

    fn get_or_create_tool(&mut self, index: usize) -> &mut ToolCallAccumulator {
        while self.tool_calls.len() <= index {
            self.tool_calls.push(ToolCallAccumulator {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
                extra: None,
                announced: false,
                last_progress_signature: String::new(),
            });
        }
        &mut self.tool_calls[index]
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        let url = format!("{}{}", self.base_url, self.compat.api_path());
        let client = crate::http_client();

        let sanitize_tool_schemas = self.should_sanitize_tool_schemas();
        let mut body = self.build_request_body(request, sanitize_tool_schemas);

        tracing::debug!(target: "nomi_providers", body = %serde_json::to_string_pretty(&body).unwrap_or_default(), "outgoing request");

        let (response, headers) = match self
            .send_initial_with_key_rotation(&client, &url, &body)
            .await
        {
            Ok(result) => result,
            Err(error)
                if !request.tools.is_empty()
                    && !sanitize_tool_schemas
                    && error.is_tool_schema_incompatible() =>
            {
                let ProviderError::Api { status, .. } = &error else {
                    unreachable!("schema classifier only accepts API errors");
                };
                tracing::warn!(
                    target: "nomi_providers",
                    provider = "openai",
                    status,
                    "provider rejected tool schemas; retrying with Bedrock-compatible schema roots"
                );
                body = self.build_request_body(request, true);
                let (response, headers) = self
                    .send_initial_with_key_rotation(&client, &url, &body)
                    .await?;
                self.sanitize_tool_schemas.store(true, Ordering::Release);
                (response, headers)
            }
            Err(error) => return Err(error),
        };

        let (tx, rx) = mpsc::channel(64);
        let auto_tool_id = self.compat.auto_tool_id();
        let client = client.clone();
        let url_clone = url.clone();

        tokio::spawn(async move {
            match process_sse_stream(response, &tx, auto_tool_id).await {
                StreamOutcome::Ok => {}
                StreamOutcome::FailedPartial(e) => {
                    let _ = tx.send(LlmEvent::Error(e.to_string())).await;
                }
                StreamOutcome::FailedEmpty(e) => {
                    if e.is_retryable() {
                        let mut backoff = std::time::Duration::from_secs(1);
                        let mut final_err = Some(e);
                        for attempt in 1..=crate::retry::MAX_STREAM_RETRIES {
                            backoff = crate::retry::backoff_sleep(attempt, backoff).await;
                            match crate::retry::send_and_check(&client, &url_clone, &headers, &body)
                                .await
                            {
                                Ok(resp) => {
                                    let outcome = process_sse_stream(resp, &tx, auto_tool_id).await;
                                    match crate::retry::evaluate_outcome(outcome, attempt) {
                                        Ok(None) => {
                                            final_err = None;
                                            break;
                                        }
                                        Ok(Some(e)) => {
                                            final_err = Some(e);
                                            break;
                                        }
                                        Err(_) => continue,
                                    }
                                }
                                Err(e) if attempt == crate::retry::MAX_STREAM_RETRIES => {
                                    final_err = Some(e);
                                    break;
                                }
                                Err(_) => continue,
                            }
                        }
                        if let Some(err) = final_err {
                            let _ = tx.send(LlmEvent::Error(err.to_string())).await;
                        }
                    } else {
                        let _ = tx.send(LlmEvent::Error(e.to_string())).await;
                    }
                }
            }
        });

        Ok(rx)
    }
}

async fn process_sse_stream(
    response: reqwest::Response,
    tx: &mpsc::Sender<LlmEvent>,
    auto_tool_id: bool,
) -> StreamOutcome {
    use futures::StreamExt;

    let mut state = StreamState::new();
    let mut buffer = String::new();
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

        // Process complete lines
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                tracing::debug!(target: "nomi_providers", chunk = %data, "sse chunk received");
                if data == "[DONE]" {
                    if !state.finish_seen {
                        for event in state.poison(
                            "OpenAI-compatible provider sent [DONE] before finish_reason",
                        ) {
                            let _ = tx.send(event).await;
                        }
                        return StreamOutcome::Ok;
                    }
                    // Atomically release staged calls and Done now that the
                    // legal usage-only tail has updated token counts.
                    for event in state.drain_terminal_events() {
                        if tx.send(event).await.is_err() {
                            return StreamOutcome::Ok;
                        }
                    }
                    return StreamOutcome::Ok;
                }

                let events = parse_sse_chunk(data, &mut state, auto_tool_id);
                for event in events {
                    if matches!(
                        event,
                        LlmEvent::TextDelta(_)
                            | LlmEvent::ThinkingDelta(_)
                            | LlmEvent::ToolUseDelta { .. }
                            | LlmEvent::ToolUse { .. }
                    ) {
                        emitted_content = true;
                    }
                    if tx.send(event).await.is_err() {
                        return StreamOutcome::Ok;
                    }
                }
                if state.fatal_error() {
                    // The parser already emitted the one actionable Error.
                    // Stop consuming immediately so a later valid-looking
                    // finish chunk or [DONE] cannot commit this turn.
                    return StreamOutcome::Ok;
                }
            }
        }
    }

    // EOF may terminate a final SSE line without a newline. Parse that line
    // before deciding whether the stream is clean; otherwise an invalid or
    // truncated tail after finish_reason could be silently ignored while its
    // already-staged tool calls were committed.
    let trailing = buffer.trim();
    if !trailing.is_empty() && !trailing.starts_with(':') {
        let Some(data) = trailing.strip_prefix("data: ") else {
            for event in state.poison(
                "OpenAI-compatible stream ended with an invalid trailing SSE line",
            ) {
                let _ = tx.send(event).await;
            }
            return StreamOutcome::Ok;
        };
        if data == "[DONE]" {
            if !state.finish_seen {
                for event in
                    state.poison("OpenAI-compatible provider sent [DONE] before finish_reason")
                {
                    let _ = tx.send(event).await;
                }
                return StreamOutcome::Ok;
            }
            for event in state.drain_terminal_events() {
                if tx.send(event).await.is_err() {
                    return StreamOutcome::Ok;
                }
            }
            return StreamOutcome::Ok;
        }

        for event in parse_sse_chunk(data, &mut state, auto_tool_id) {
            if matches!(
                event,
                LlmEvent::TextDelta(_)
                    | LlmEvent::ThinkingDelta(_)
                    | LlmEvent::ToolUseDelta { .. }
                    | LlmEvent::ToolUse { .. }
            ) {
                emitted_content = true;
            }
            if tx.send(event).await.is_err() {
                return StreamOutcome::Ok;
            }
        }
        if state.fatal_error() {
            return StreamOutcome::Ok;
        }
    }

    // Some OpenAI-compatible servers close after finish_reason without an
    // explicit `[DONE]`. A clean EOF may complete a side-effect-free terminal,
    // but structured tool calls require the protocol's explicit commit marker:
    // a connection truncated immediately after finish_reason must not execute.
    if state.finish_seen {
        if !state.pending_tool_calls.is_empty() {
            for event in state.poison(
                "OpenAI-compatible stream ended before [DONE] with structured tool calls pending",
            ) {
                let _ = tx.send(event).await;
            }
            return StreamOutcome::Ok;
        }
        for event in state.drain_terminal_events() {
            if tx.send(event).await.is_err() {
                return StreamOutcome::Ok;
            }
        }
        StreamOutcome::Ok
    } else {
        let error = ProviderError::Connection(
            "OpenAI-compatible stream ended before finish_reason".to_string(),
        );
        if emitted_content {
            StreamOutcome::FailedPartial(error)
        } else {
            StreamOutcome::FailedEmpty(error)
        }
    }
}

const TOOL_PROGRESS_PREVIEW_FIELDS: &[&str] = &[
    "file_path",
    "filePath",
    "path",
    "file_name",
    "fileName",
    "relative_path",
    "relativePath",
    "dir",
    "glob",
    "command",
    "cmd",
    "script",
    "pattern",
    "query",
    "url",
    "skill",
];

fn tool_argument_value_progress_preview(input: &Value) -> Option<Value> {
    let Value::Object(map) = input else {
        return None;
    };

    let mut preview = serde_json::Map::new();
    for key in TOOL_PROGRESS_PREVIEW_FIELDS {
        if let Some(value) = map.get(*key)
            && is_small_preview_value(value)
        {
            preview.insert((*key).to_string(), value.clone());
        }
    }

    if preview.is_empty() {
        None
    } else {
        Some(Value::Object(preview))
    }
}

fn tool_argument_progress_preview(arguments: &str) -> Option<Value> {
    let mut preview = serde_json::Map::new();

    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(arguments) {
        return tool_argument_value_progress_preview(&Value::Object(map));
    } else {
        for key in TOOL_PROGRESS_PREVIEW_FIELDS {
            if let Some(value) = extract_json_string_field(arguments, key) {
                preview.insert((*key).to_string(), Value::String(value));
            }
        }
    }

    if preview.is_empty() {
        None
    } else {
        Some(Value::Object(preview))
    }
}

fn is_small_preview_value(value: &Value) -> bool {
    match value {
        Value::String(s) => s.len() <= 2_000,
        Value::Number(_) | Value::Bool(_) => true,
        _ => false,
    }
}

fn extract_json_string_field(arguments: &str, key: &str) -> Option<String> {
    let quoted_key = format!("\"{key}\"");
    let mut search_from = 0usize;

    while let Some(relative_pos) = arguments[search_from..].find(&quoted_key) {
        let mut cursor = search_from + relative_pos + quoted_key.len();
        cursor = skip_json_whitespace(arguments, cursor);
        if arguments[cursor..].chars().next()? != ':' {
            search_from = cursor;
            continue;
        }
        cursor += ':'.len_utf8();
        cursor = skip_json_whitespace(arguments, cursor);
        if arguments[cursor..].chars().next()? != '"' {
            search_from = cursor;
            continue;
        }
        cursor += '"'.len_utf8();

        let mut escaped = false;
        for (offset, ch) in arguments[cursor..].char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                let end = cursor + offset;
                let raw = &arguments[cursor..end];
                let quoted = format!("\"{raw}\"");
                return serde_json::from_str::<String>(&quoted)
                    .ok()
                    .or_else(|| Some(raw.to_string()));
            }
        }

        return None;
    }

    None
}

/// Atomically finalize structured OpenAI tool calls.
///
/// If any call has malformed arguments, return an error and emit none of the
/// calls. This prevents a valid parallel call from being executed alongside a
/// malformed sibling and, critically, prevents malformed JSON from becoming
/// an executable `{}` payload.
fn finalize_structured_tool_calls(
    state: &mut StreamState,
    auto_tool_id: bool,
) -> Result<Vec<LlmEvent>, String> {
    let calls = std::mem::take(&mut state.tool_calls);
    let mut events = Vec::with_capacity(calls.len());

    for tc in calls {
        let id = if tc.id.is_empty() && auto_tool_id {
            generate_call_id()
        } else {
            tc.id
        };
        let input = crate::parse_tool_call_arguments(
            "OpenAI-compatible provider",
            &tc.name,
            &id,
            &tc.arguments,
        )?;
        events.push(LlmEvent::ToolUse {
            id,
            name: tc.name,
            input,
            extra: tc.extra,
        });
    }

    Ok(events)
}

fn skip_json_whitespace(input: &str, mut index: usize) -> usize {
    while let Some(ch) = input[index..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn maybe_tool_progress_event(
    acc: &mut ToolCallAccumulator,
    auto_tool_id: bool,
) -> Option<LlmEvent> {
    if acc.name.trim().is_empty() {
        return None;
    }

    if acc.id.trim().is_empty() {
        if auto_tool_id {
            acc.id = generate_call_id();
        } else {
            return None;
        }
    }

    let input = tool_argument_progress_preview(&acc.arguments);
    let signature = input
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_default();

    if !acc.announced || (!signature.is_empty() && signature != acc.last_progress_signature) {
        acc.announced = true;
        acc.last_progress_signature = signature;
        Some(LlmEvent::ToolUseDelta {
            id: acc.id.clone(),
            name: acc.name.clone(),
            input,
        })
    } else {
        None
    }
}

/// Extract one reasoning delta from the OpenAI-compatible variants used by
/// different gateways. Prefer the scalar fields to avoid duplicating output
/// when a provider includes both a normalized field and `reasoning_details`.
fn extract_reasoning_delta(delta: &Value) -> Option<String> {
    for field in ["reasoning_content", "reasoning"] {
        if let Some(text) = delta[field].as_str().filter(|text| !text.is_empty()) {
            return Some(text.to_string());
        }
    }

    let mut reasoning = String::new();
    for detail in delta["reasoning_details"].as_array()? {
        let text = detail["text"]
            .as_str()
            .filter(|text| !text.is_empty())
            .or_else(|| {
                detail["content"]
                    .as_str()
                    .filter(|content| !content.is_empty())
            });
        if let Some(text) = text {
            reasoning.push_str(text);
        }
    }

    (!reasoning.is_empty()).then_some(reasoning)
}

fn optional_usage_u64(
    usage: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, String> {
    match usage.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            format!(
                "OpenAI-compatible provider returned non-integer usage field '{field}'"
            )
        }),
    }
}

fn update_stream_usage(json: &Value, state: &mut StreamState) -> Result<(), String> {
    let usage = match json.get("usage") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::Object(usage)) => usage,
        Some(_) => {
            return Err(
                "OpenAI-compatible provider returned a non-object usage payload".to_string(),
            );
        }
    };

    let base_prompt = optional_usage_u64(usage, "prompt_tokens")?.unwrap_or(state.input_tokens);
    let cache_hit = optional_usage_u64(usage, "prompt_cache_hit_tokens")?.unwrap_or(0);
    state.input_tokens = base_prompt.checked_add(cache_hit).ok_or_else(|| {
        "OpenAI-compatible provider returned overflowing prompt token usage".to_string()
    })?;
    state.output_tokens =
        optional_usage_u64(usage, "completion_tokens")?.unwrap_or(state.output_tokens);

    let detail_cached = match usage.get("prompt_tokens_details") {
        None | Some(Value::Null) => 0,
        Some(Value::Object(details)) => {
            optional_usage_u64(details, "cached_tokens")?.unwrap_or(0)
        }
        Some(_) => {
            return Err(
                "OpenAI-compatible provider returned non-object prompt_tokens_details"
                    .to_string(),
            );
        }
    };
    let cached = if cache_hit > 0 {
        cache_hit
    } else {
        detail_cached
    };
    if cached > 0 {
        state.cache_read_tokens = cached;
    }

    Ok(())
}

fn parse_sse_chunk(data: &str, state: &mut StreamState, auto_tool_id: bool) -> Vec<LlmEvent> {
    if state.fatal_error {
        return Vec::new();
    }

    let mut events = Vec::new();

    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(error) => {
            return state.poison(format!(
                "OpenAI-compatible provider returned malformed SSE JSON: {error}"
            ));
        }
    };

    if !json.is_object() {
        return state.poison(
            "OpenAI-compatible provider returned a non-object SSE payload",
        );
    }

    let post_finish_usage_only = state.finish_seen;
    if post_finish_usage_only {
        let is_usage_only = json.get("usage").is_some_and(Value::is_object)
            && json
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty);
        if !is_usage_only {
            return state.poison(
                "OpenAI-compatible provider emitted non-usage data after finish_reason",
            );
        }
        if let Err(error) = update_stream_usage(&json, state) {
            return state.poison(error);
        }
        return events;
    }

    if let Err(error) = update_stream_usage(&json, state) {
        return state.poison(error);
    }

    let Some(choices) = json.get("choices").and_then(Value::as_array) else {
        return state.poison(
            "OpenAI-compatible provider returned an SSE payload without a choices array",
        );
    };
    if choices.len() != 1 {
        return state.poison(format!(
            "OpenAI-compatible provider returned {} choices in a streamed response; expected exactly one",
            choices.len()
        ));
    }
    let Some(choice) = choices[0].as_object() else {
        return state.poison(
            "OpenAI-compatible provider returned a non-object streamed choice",
        );
    };
    let Some(delta_value) = choice.get("delta") else {
        return state.poison(
            "OpenAI-compatible provider returned a streamed choice without an object delta",
        );
    };
    let Some(delta) = delta_value.as_object() else {
        return state.poison(
            "OpenAI-compatible provider returned a streamed choice without an object delta",
        );
    };
    let finish_reason = match choice.get("finish_reason") {
        None | Some(Value::Null) => None,
        Some(Value::String(reason)) => Some(reason.as_str()),
        Some(_) => {
            return state.poison(
                "OpenAI-compatible provider returned a non-string finish_reason",
            );
        }
    };

    for field in ["role", "content", "reasoning_content", "reasoning"] {
        if let Some(value) = delta.get(field)
            && !value.is_null()
            && !value.is_string()
        {
            return state.poison(format!(
                "OpenAI-compatible provider returned non-string delta field '{field}'"
            ));
        }
    }
    if let Some(details) = delta.get("reasoning_details")
        && !details.is_null()
    {
        let Some(details) = details.as_array() else {
            return state.poison(
                "OpenAI-compatible provider returned non-array reasoning_details",
            );
        };
        for detail in details {
            let Some(detail) = detail.as_object() else {
                return state.poison(
                    "OpenAI-compatible provider returned a non-object reasoning detail",
                );
            };
            for field in ["text", "content"] {
                if let Some(value) = detail.get(field)
                    && !value.is_null()
                    && !value.is_string()
                {
                    return state.poison(format!(
                        "OpenAI-compatible provider returned non-string reasoning detail field '{field}'"
                    ));
                }
            }
        }
    }

    // Reasoning content (OpenAI reasoning models)
    if let Some(reasoning) = extract_reasoning_delta(delta_value) {
        events.push(LlmEvent::ThinkingDelta(reasoning));
    }

    // Text content
    if let Some(content) = delta.get("content").and_then(Value::as_str)
        && !content.is_empty()
    {
        events.push(LlmEvent::TextDelta(content.to_string()));
    }

    // Tool calls
    let tool_calls = match delta.get("tool_calls") {
        None | Some(Value::Null) => None,
        Some(Value::Array(tool_calls)) => Some(tool_calls),
        Some(_) => {
            return state.poison(
                "OpenAI-compatible provider returned non-array delta.tool_calls",
            );
        }
    };
    if let Some(tool_calls) = tool_calls {
        for tc in tool_calls {
            let Some(tc) = tc.as_object() else {
                return state.poison(
                    "OpenAI-compatible provider returned a non-object tool_calls item",
                );
            };
            let Some(raw_index) = tc.get("index").and_then(Value::as_u64) else {
                return state.poison(
                    "OpenAI-compatible provider returned a tool_calls item without a non-negative integer index",
                );
            };
            if raw_index >= MAX_STRUCTURED_TOOL_CALLS_PER_TURN as u64 {
                return state.poison(format!(
                    "OpenAI-compatible provider returned tool-call index {raw_index}; maximum supported index is {}",
                    MAX_STRUCTURED_TOOL_CALLS_PER_TURN - 1
                ));
            }
            let index = raw_index as usize;
            if let Some(kind) = tc.get("type")
                && kind.as_str() != Some("function")
            {
                return state.poison(
                    "OpenAI-compatible provider returned a tool_calls item with a non-function type",
                );
            }
            let id = match tc.get("id") {
                None => None,
                Some(Value::String(id)) if !id.trim().is_empty() => Some(id.as_str()),
                Some(_) => {
                    return state.poison(
                        "OpenAI-compatible provider returned an empty or non-string tool call id",
                    );
                }
            };
            let Some(function) = tc.get("function").and_then(Value::as_object) else {
                return state.poison(
                    "OpenAI-compatible provider returned a tool_calls item without an object function",
                );
            };
            let name = match function.get("name") {
                None => None,
                Some(Value::String(name)) if !name.trim().is_empty() => Some(name.as_str()),
                Some(_) => {
                    return state.poison(
                        "OpenAI-compatible provider returned an empty or non-string function name",
                    );
                }
            };
            let arguments = match function.get("arguments") {
                None => None,
                Some(Value::String(arguments)) => Some(arguments.as_str()),
                Some(_) => {
                    return state.poison(
                        "OpenAI-compatible provider returned non-string function arguments",
                    );
                }
            };

            if let Some(existing) = state.tool_calls.get(index) {
                if let Some(id) = id
                    && !existing.id.is_empty()
                    && existing.id != id
                {
                    return state.poison(format!(
                        "OpenAI-compatible provider changed the id for tool-call index {index}"
                    ));
                }
                if let Some(name) = name
                    && !existing.name.is_empty()
                    && existing.name != name
                {
                    return state.poison(format!(
                        "OpenAI-compatible provider changed the function name for tool-call index {index}"
                    ));
                }
            }

            let acc = state.get_or_create_tool(index);

            if let Some(id) = id {
                acc.id = id.to_string();
            }
            if let Some(name) = name {
                acc.name = name.to_string();
            }
            if let Some(arguments) = arguments {
                acc.arguments.push_str(arguments);
            }
            if let Some(extra) = tc.get("extra_content").filter(|v| !v.is_null()) {
                acc.extra = Some(extra.clone());
            }
            if let Some(event) = maybe_tool_progress_event(acc, auto_tool_id) {
                events.push(event);
            }
        }
    }

    // Defer final structured calls and Done until [DONE] so the trailing
    // usage-only chunk can update token counts first.
    if let Some(finish_reason) = finish_reason {
        state.finish_seen = true;
        match finish_reason {
            "tool_calls" => {
                if state.tool_calls.is_empty() {
                    return state.poison(
                        "OpenAI-compatible provider finished with tool_calls but supplied no complete structured tool call",
                    );
                }
                match finalize_structured_tool_calls(state, auto_tool_id) {
                    Ok(tool_events) => {
                        state.pending_tool_calls = tool_events;
                        state.pending_done = Some(LlmEvent::Done {
                            stop_reason: StopReason::ToolUse,
                            usage: TokenUsage::default(),
                        });
                    }
                    Err(error) => return state.poison(error),
                }
            }
            "stop" => {
                if !state.tool_calls.is_empty() {
                    return state.poison(
                        "OpenAI-compatible provider finished with stop after supplying structured tool-call data",
                    );
                }
                state.pending_done = Some(LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                });
            }
            "length" => {
                // A length-truncated argument stream is never safe to execute,
                // even if the accumulated JSON happens to parse. Report the
                // actual terminal condition and discard all incomplete call
                // accumulators; the caller can retry with a larger token budget.
                state.tool_calls.clear();
                state.pending_tool_calls.clear();
                state.pending_done = Some(LlmEvent::Done {
                    stop_reason: StopReason::MaxTokens,
                    usage: TokenUsage::default(),
                });
            }
            other => {
                return state.poison(format!(
                    "OpenAI-compatible provider returned unsupported finish_reason '{other}'"
                ));
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::{extract_reasoning_delta, parse_sse_chunk, StreamState};
    use nomi_types::llm::LlmEvent;
    use nomi_types::message::StopReason;
    use serde_json::json;

    #[test]
    fn extracts_reasoning_content_delta() {
        let delta = json!({"reasoning_content": "thinking"});
        assert_eq!(extract_reasoning_delta(&delta).as_deref(), Some("thinking"));
    }

    #[test]
    fn extracts_reasoning_delta() {
        let delta = json!({"reasoning": "thinking"});
        assert_eq!(extract_reasoning_delta(&delta).as_deref(), Some("thinking"));
    }

    #[test]
    fn extracts_reasoning_details_text_and_content() {
        let delta = json!({
            "reasoning_details": [
                {"type": "reasoning.text", "text": "first "},
                {"type": "reasoning.text", "content": "second"},
                {"text": "", "content": " third"}
            ]
        });
        assert_eq!(
            extract_reasoning_delta(&delta).as_deref(),
            Some("first second third")
        );
    }

    #[test]
    fn scalar_reasoning_field_takes_precedence_over_details() {
        let delta = json!({
            "reasoning_content": "once",
            "reasoning": "duplicate",
            "reasoning_details": [{"text": "duplicate"}]
        });
        assert_eq!(extract_reasoning_delta(&delta).as_deref(), Some("once"));
    }

    #[test]
    fn reasoning_variants_emit_thinking_deltas() {
        for chunk in [
            r#"{"choices":[{"delta":{"reasoning_content":"from content"},"finish_reason":null,"index":0}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"from reasoning"},"finish_reason":null,"index":0}]}"#,
            r#"{"choices":[{"delta":{"reasoning_details":[{"text":"from "},{"content":"details"}]},"finish_reason":null,"index":0}]}"#,
        ] {
            let mut state = StreamState::new();
            let events = parse_sse_chunk(chunk, &mut state, false);
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, LlmEvent::ThinkingDelta(text) if !text.is_empty())),
                "expected ThinkingDelta for chunk: {chunk}"
            );
        }
    }

    #[test]
    fn literal_tool_call_markup_is_exact_text_and_never_a_tool() {
        for literal in [
            r#"<tool_call>{"name":"counted_tool","arguments":{}}</tool_call>"#,
            r#"<tool_call>not json</tool_call>"#,
            r#"<tool_call>{"name":"counted_tool""#,
        ] {
            let mut state = StreamState::new();
            let chunk = json!({
                "choices": [{
                    "delta": { "content": literal },
                    "finish_reason": "stop",
                    "index": 0
                }]
            })
            .to_string();

            let events = parse_sse_chunk(&chunk, &mut state, true);
            let text = events
                .iter()
                .filter_map(|event| match event {
                    LlmEvent::TextDelta(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();

            assert_eq!(text, literal);
            assert!(events.iter().all(|event| !matches!(
                event,
                LlmEvent::ToolUse { .. } | LlmEvent::ToolUseDelta { .. } | LlmEvent::Error(_)
            )));
            assert!(matches!(
                state.drain_terminal_events().as_slice(),
                [LlmEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    ..
                }]
            ));
        }
    }

    #[test]
    fn split_literal_tool_tags_round_trip_exactly_without_progress() {
        let literal =
            r#"prefix <tool_call>{"name":"counted_tool","arguments":{}}</tool_call> suffix"#;
        for split in [1, 8, 15, 27, 48, literal.len() - 3] {
            let mut state = StreamState::new();
            let first = json!({
                "choices": [{
                    "delta": { "content": &literal[..split] },
                    "finish_reason": null,
                    "index": 0
                }]
            })
            .to_string();
            let second = json!({
                "choices": [{
                    "delta": { "content": &literal[split..] },
                    "finish_reason": "stop",
                    "index": 0
                }]
            })
            .to_string();

            let mut events = parse_sse_chunk(&first, &mut state, true);
            events.extend(parse_sse_chunk(&second, &mut state, true));
            let text = events
                .iter()
                .filter_map(|event| match event {
                    LlmEvent::TextDelta(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();

            assert_eq!(text, literal, "split at byte {split}");
            assert!(events.iter().all(|event| !matches!(
                event,
                LlmEvent::ToolUse { .. } | LlmEvent::ToolUseDelta { .. } | LlmEvent::Error(_)
            )));
        }
    }

    #[test]
    fn reasoning_tool_markup_is_exact_thinking_and_never_a_tool() {
        let literal = r#"<tool_call>{"name":"counted_tool","arguments":{}}</tool_call>"#;
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": { "reasoning_content": literal },
                "finish_reason": "stop",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, true);
        assert!(matches!(
            events.as_slice(),
            [LlmEvent::ThinkingDelta(text)] if text == literal
        ));
        assert!(matches!(
            state.drain_terminal_events().as_slice(),
            [LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                ..
            }]
        ));
    }

    #[test]
    fn tool_calls_finish_without_structured_delta_is_an_error() {
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": {
                    "content": r#"<tool_call>{"name":"counted_tool","arguments":{}}</tool_call>"#
                },
                "finish_reason": "tool_calls",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, true);
        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("no complete structured tool call"))
        ));
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn sparse_tool_call_index_is_rejected_before_vector_growth() {
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": u64::MAX,
                        "id": "call_sparse",
                        "function": {"name": "Read", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, false);

        assert!(matches!(
            events.as_slice(),
            [LlmEvent::Error(message)] if message.contains("tool-call index")
        ));
        assert!(state.fatal_error());
        assert!(state.tool_calls.is_empty());
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn missing_or_invalid_tool_call_index_is_rejected() {
        for item in [
            json!({
                "id": "call_missing_index",
                "function": {"name": "Read", "arguments": "{}"}
            }),
            json!({
                "index": "0",
                "id": "call_string_index",
                "function": {"name": "Read", "arguments": "{}"}
            }),
            json!({
                "index": -1,
                "id": "call_negative_index",
                "function": {"name": "Read", "arguments": "{}"}
            }),
        ] {
            let mut state = StreamState::new();
            let chunk = json!({
                "choices": [{
                    "delta": {"tool_calls": [item]},
                    "finish_reason": "tool_calls",
                    "index": 0
                }]
            })
            .to_string();

            let events = parse_sse_chunk(&chunk, &mut state, false);

            assert!(matches!(
                events.as_slice(),
                [LlmEvent::Error(message)] if message.contains("integer index")
            ));
            assert!(state.fatal_error());
            assert!(state.drain_terminal_events().is_empty());
        }
    }

    #[test]
    fn malformed_envelope_cannot_be_washed_by_a_later_valid_finish() {
        for malformed in [
            r#"{}"#,
            r#"[]"#,
            r#"{"choices":[]}"#,
            r#"{"choices":[null]}"#,
            r#"{"choices":[{"delta":null,"finish_reason":null}]}"#,
        ] {
            let mut state = StreamState::new();
            let partial = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_partial","type":"function","function":{"name":"Read","arguments":"{"}}]},"finish_reason":null,"index":0}]}"#;
            parse_sse_chunk(partial, &mut state, false);

            let malformed_events = parse_sse_chunk(malformed, &mut state, false);
            let finish = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
            let later_events = parse_sse_chunk(finish, &mut state, false);

            assert!(malformed_events
                .iter()
                .any(|event| matches!(event, LlmEvent::Error(_))),
                "payload should poison: {malformed}"
            );
            assert!(later_events.is_empty());
            assert!(state.drain_terminal_events().is_empty());
        }
    }

    #[test]
    fn changed_tool_identity_for_one_index_is_rejected() {
        for second in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_other","function":{"arguments":"}"}}]},"finish_reason":"tool_calls","index":0}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"Write","arguments":"}"}}]},"finish_reason":"tool_calls","index":0}]}"#,
        ] {
            let mut state = StreamState::new();
            let first = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_same","type":"function","function":{"name":"Read","arguments":"{"}}]},"finish_reason":null,"index":0}]}"#;
            parse_sse_chunk(first, &mut state, false);

            let events = parse_sse_chunk(second, &mut state, false);

            assert!(events.iter().any(
                |event| matches!(event, LlmEvent::Error(message) if message.contains("changed the"))
            ));
            assert!(state.drain_terminal_events().is_empty());
        }
    }

    #[test]
    fn length_finish_never_executes_partial_structured_tool_call() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_write","type":"function","function":{"name":"Write","arguments":"{\"file_path\":\"/tmp/index.html\",\"content\":\"<html><body>hello"}}]},"finish_reason":"length","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, true);

        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. })),
            "length-truncated arguments must never execute"
        );
        assert!(state.tool_calls.is_empty());
        assert!(matches!(
            state.pending_done,
            Some(LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                ..
            })
        ));
    }

    #[test]
    fn length_finish_does_not_execute_even_complete_tool_arguments() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_complete","type":"function","function":{"name":"Read","arguments":"{\"path\":\"/tmp/file\"}"}}]},"finish_reason":"length","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, true);

        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
        );
        assert!(matches!(
            state.pending_done,
            Some(LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                ..
            })
        ));
    }

    #[test]
    fn length_finish_treats_text_markup_as_text_and_never_a_tool() {
        let literal =
            r#"<tool_call>{"name":"Write","arguments":{"file_path":"/tmp/index.html""#;
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": { "reasoning_content": literal },
                "finish_reason": "length",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, true);

        assert!(matches!(
            events.as_slice(),
            [LlmEvent::ThinkingDelta(text)] if text == literal
        ));
        assert!(matches!(
            state.drain_terminal_events().as_slice(),
            [LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                ..
            }]
        ));
    }

    #[test]
    fn post_finish_content_poison_clears_staged_structured_calls() {
        let mut state = StreamState::new();
        let finish = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_staged","type":"function","function":{"name":"Read","arguments":"{\"path\":\"/tmp/file\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let finish_events = parse_sse_chunk(finish, &mut state, true);

        assert!(finish_events
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
        assert_eq!(state.pending_tool_calls.len(), 1);

        let tail = r#"{"choices":[{"delta":{"content":"illegal tail"},"finish_reason":null,"index":0}]}"#;
        let tail_events = parse_sse_chunk(tail, &mut state, true);

        assert!(tail_events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("after finish_reason"))
        ));
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn second_finish_after_finish_reason_poison_clears_staged_calls() {
        let mut state = StreamState::new();
        let finish = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_staged","type":"function","function":{"name":"Read","arguments":"{\"path\":\"/tmp/file\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        parse_sse_chunk(finish, &mut state, true);
        assert_eq!(state.pending_tool_calls.len(), 1);

        let second_finish =
            r#"{"choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#;
        let events = parse_sse_chunk(second_finish, &mut state, true);

        assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn usage_only_tail_preserves_staged_calls_and_updates_done_usage() {
        let mut state = StreamState::new();
        let finish = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_staged","type":"function","function":{"name":"Read","arguments":"{\"path\":\"/tmp/file\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        parse_sse_chunk(finish, &mut state, true);

        let usage = r#"{"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7}}"#;
        assert!(parse_sse_chunk(usage, &mut state, true).is_empty());

        let terminal = state.drain_terminal_events();
        assert!(matches!(terminal.first(), Some(LlmEvent::ToolUse { .. })));
        assert!(matches!(
            terminal.last(),
            Some(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage
            }) if usage.input_tokens == 11 && usage.output_tokens == 7
        ));
    }

    #[tokio::test]
    async fn stream_without_done_sentinel_still_emits_done() {
        use super::{StreamOutcome, process_sse_stream};
        // Some OpenAI-compatible servers (vLLM, local deployments) close the
        // connection after the final chunk without sending the `[DONE]`
        // sentinel. A side-effect-free EndTurn may still complete.
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        );
        let http_resp = http::Response::builder()
            .status(200)
            .body(body.to_string())
            .unwrap();
        let response = reqwest::Response::from(http_resp);

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let outcome = process_sse_stream(response, &tx, false).await;
        drop(tx);

        let mut saw_done = false;
        while let Some(ev) = rx.recv().await {
            if matches!(ev, LlmEvent::Done { .. }) {
                saw_done = true;
            }
        }
        assert!(saw_done, "stream ending without [DONE] must still emit a Done");
        assert!(matches!(outcome, StreamOutcome::Ok));
    }

    #[tokio::test]
    async fn structured_tool_call_requires_explicit_done_sentinel() {
        use super::{StreamOutcome, process_sse_stream};

        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_eof\",\"type\":\"function\",\"function\":{\"name\":\"update_base\",\"arguments\":\"{\\\"kb_id\\\":\\\"kb_1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let outcome = process_sse_stream(response, &tx, false).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("before [DONE]"))
        ));
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::ToolUse { .. } | LlmEvent::Done { .. }
        )));
    }

    #[tokio::test]
    async fn explicit_done_sentinel_commits_structured_tool_call() {
        use super::{StreamOutcome, process_sse_stream};

        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_done\",\"type\":\"function\",\"function\":{\"name\":\"update_base\",\"arguments\":\"{\\\"kb_id\\\":\\\"kb_1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let outcome = process_sse_stream(response, &tx, false).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LlmEvent::ToolUse { .. }))
                .count(),
            1
        );
        assert!(events
            .iter()
            .any(|event| matches!(event, LlmEvent::ToolUse { id, .. } if id == "call_done")));
        assert!(matches!(
            events.last(),
            Some(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn unterminated_tail_after_finish_cannot_commit_staged_tool_call() {
        use super::{StreamOutcome, process_sse_stream};

        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_staged\",\"type\":\"function\",\"function\":{\"name\":\"update_base\",\"arguments\":\"{\\\"kb_id\\\":\\\"kb_1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":["
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let outcome = process_sse_stream(response, &tx, false).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LlmEvent::Error(_)))
                .count(),
            1
        );
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::ToolUse { .. } | LlmEvent::Done { .. }
        )));
    }

    #[tokio::test]
    async fn malformed_json_stream_stops_before_later_tool_finish_and_done() {
        use super::{StreamOutcome, process_sse_stream};

        let body = concat!(
            "data: {\"choices\":[\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_late\",\"type\":\"function\",\"function\":{\"name\":\"update_base\",\"arguments\":\"{\\\"kb_id\\\":\\\"kb_1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let outcome = process_sse_stream(response, &tx, true).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LlmEvent::Error(_)))
                .count(),
            1
        );
        assert!(events.iter().all(|event| !matches!(
            event,
            LlmEvent::ToolUse { .. } | LlmEvent::ToolUseDelta { .. } | LlmEvent::Done { .. }
        )));
    }

    #[tokio::test]
    async fn stream_literal_tool_tags_round_trip_across_delta_splits() {
        use super::{StreamOutcome, process_sse_stream};

        for literal in [
            r#"<tool_call>{"name":"counted_tool","arguments":{}}</tool_call>"#,
            r#"<tool_call>not json</tool_call>"#,
            r#"<tool_call>{"name":"counted_tool""#,
        ] {
            for split in [1, 7, literal.len() / 2, literal.len() - 1] {
                let first = json!({
                    "choices": [{
                        "delta": { "content": &literal[..split] },
                        "finish_reason": null,
                        "index": 0
                    }]
                })
                .to_string();
                let second = json!({
                    "choices": [{
                        "delta": { "content": &literal[split..] },
                        "finish_reason": "stop",
                        "index": 0
                    }]
                })
                .to_string();
                let body = format!("data: {first}\n\ndata: {second}\n\ndata: [DONE]\n\n");
                let response = reqwest::Response::from(
                    http::Response::builder()
                        .status(200)
                        .body(body)
                        .unwrap(),
                );
                let (tx, mut rx) = tokio::sync::mpsc::channel(16);

                let outcome = process_sse_stream(response, &tx, true).await;
                drop(tx);
                let mut events = Vec::new();
                while let Some(event) = rx.recv().await {
                    events.push(event);
                }
                let text = events
                    .iter()
                    .filter_map(|event| match event {
                        LlmEvent::TextDelta(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<String>();

                assert!(matches!(outcome, StreamOutcome::Ok));
                assert_eq!(text, literal, "split at byte {split}");
                assert!(events.iter().all(|event| !matches!(
                    event,
                    LlmEvent::ToolUse { .. }
                        | LlmEvent::ToolUseDelta { .. }
                        | LlmEvent::Error(_)
                )));
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| matches!(
                            event,
                            LlmEvent::Done {
                                stop_reason: StopReason::EndTurn,
                                ..
                            }
                        ))
                        .count(),
                    1
                );
            }
        }
    }

    #[tokio::test]
    async fn post_finish_content_stream_clears_staged_call_and_emits_no_done() {
        use super::{StreamOutcome, process_sse_stream};

        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_staged\",\"type\":\"function\",\"function\":{\"name\":\"Read\",\"arguments\":\"{\\\"path\\\":\\\"/tmp/file\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"illegal tail\"},\"finish_reason\":null}]}\n\n",
            "data: [DONE]\n\n",
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(body.to_string())
                .unwrap(),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let outcome = process_sse_stream(response, &tx, true).await;
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

    #[test]
    fn tool_images_ride_in_follow_up_user_message() {
        use nomi_types::message::{ContentBlock, Message, Role};
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
        let compat = nomi_config::compat::ProviderCompat::openai_defaults();
        let result = OpenAIProvider::build_messages(&messages, "", &compat, false);
        // tool message first, then a user message carrying the image
        assert_eq!(result[0]["role"], "tool");
        assert_eq!(result[0]["content"], "screenshot taken");
        assert_eq!(result[1]["role"], "user");
        let parts = result[1]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert!(
            parts[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn user_message_image_block_produces_image_url_content() {
        use nomi_types::message::{ContentBlock, Message, Role};
        let messages = vec![Message::new(
            Role::User,
            vec![
                ContentBlock::Text {
                    text: "Describe this image".to_string(),
                },
                ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "aGVsbG8=".to_string(),
                },
            ],
        )];
        let compat = nomi_config::compat::ProviderCompat::openai_defaults();
        let result = OpenAIProvider::build_messages(&messages, "", &compat, false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Describe this image");
        assert_eq!(content[1]["type"], "image_url");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .ends_with("aGVsbG8=")
        );
    }

    #[test]
    fn strips_user_image_when_supports_image_false() {
        use nomi_types::message::{ContentBlock, Message, Role};
        let compat = ProviderCompat {
            supports_image: Some(false),
            ..Default::default()
        };
        let messages = vec![Message::new(
            Role::User,
            vec![
                ContentBlock::Text { text: "看这张图".into() },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "AAAA".into(),
                },
            ],
        )];
        let out = OpenAIProvider::build_messages(&messages, "", &compat, false);
        let s = serde_json::to_string(&out).unwrap();
        assert!(!s.contains("image_url"), "不应出现 image_url: {s}");
        assert!(s.contains("图片已省略"), "应出现占位: {s}");
    }

    #[test]
    fn keeps_user_image_when_supports_image_true() {
        use nomi_types::message::{ContentBlock, Message, Role};
        let compat = ProviderCompat::default(); // supports_image() == true
        let messages = vec![Message::new(
            Role::User,
            vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data: "AAAA".into(),
            }],
        )];
        let out = OpenAIProvider::build_messages(&messages, "", &compat, false);
        let s = serde_json::to_string(&out).unwrap();
        assert!(s.contains("image_url"), "应保留 image_url: {s}");
    }

    use super::*;

    fn no_compat() -> ProviderCompat {
        ProviderCompat::default()
    }

    fn openai_compat() -> ProviderCompat {
        ProviderCompat::openai_defaults()
    }

    fn simple_request() -> LlmRequest {
        LlmRequest {
            model: "gpt-4o-mini".into(),
            system: String::new(),
            messages: vec![Message::new(
                Role::User,
                vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            )],
            tools: vec![],
            max_tokens: 16,
            thinking: None,
            reasoning_effort: None,
        }
    }

    async fn drain_stream(mut rx: tokio::sync::mpsc::Receiver<LlmEvent>) {
        while rx.recv().await.is_some() {}
    }

    #[test]
    fn deepseek_free_multiturn_tool_history_gets_reasoning_placeholder() {
        let mut compat = openai_compat();
        compat.require_reasoning_content = Some(true);
        let provider = OpenAIProvider::new("key", "http://localhost", compat);
        let mut request = simple_request();
        request.model = "deepseek-v4-flash-free".into();
        request.messages = vec![
            Message::new(
                Role::Assistant,
                vec![ContentBlock::ToolUse {
                    id: "call_1".into(),
                    name: "read".into(),
                    input: json!({"path": "README.md"}),
                    extra: None,
                }],
            ),
            Message::new(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "contents".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
        ];

        let body = provider.build_request_body(&request, provider.should_sanitize_tool_schemas());
        let assistant = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["reasoning_content"], " ");
        assert!(assistant["tool_calls"].is_array());
    }

    #[test]
    fn reasoning_placeholder_requires_explicit_provider_compat() {
        let provider = OpenAIProvider::new("key", "http://localhost", openai_compat());
        let mut request = simple_request();
        request.model = "other-free".into();
        request.messages = vec![Message::new(
            Role::Assistant,
            vec![ContentBlock::Text {
                text: "answer".into(),
            }],
        )];

        let body = provider.build_request_body(&request, provider.should_sanitize_tool_schemas());
        assert!(
            body["messages"][0].get("reasoning_content").is_none(),
            "unrelated models must retain normal OpenAI message semantics"
        );
    }

    #[tokio::test]
    async fn stream_reuses_shared_http_client() {
        use crate::http_client_build_count;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .expect(2)
            .mount(&server)
            .await;

        let provider = OpenAIProvider::new("key", &server.uri(), openai_compat());

        // First call may trigger the one-time lazy build (0 if another test in
        // this binary already initialized the process-wide shared client).
        drain_stream(provider.stream(&simple_request()).await.unwrap()).await;
        let after_first = http_client_build_count();

        // A second call must NOT rebuild — the shared client (and its keep-alive
        // connection pool) is reused across requests and providers.
        drain_stream(provider.stream(&simple_request()).await.unwrap()).await;
        assert_eq!(
            http_client_build_count(),
            after_first,
            "shared HTTP client must be reused, not rebuilt per call"
        );
        assert!(
            after_first <= 1,
            "shared HTTP client must be built at most once per process, got {after_first}"
        );
    }

    // --- max_tokens_field ---

    #[test]
    fn test_max_tokens_field_default() {
        let provider = OpenAIProvider::new("key", "http://localhost", openai_compat());
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system: String::new(),
            messages: vec![],
            tools: vec![],
            max_tokens: 1024,
            thinking: None,
            reasoning_effort: None,
        };
        let body = provider.build_request_body(&req, provider.should_sanitize_tool_schemas());
        assert_eq!(body["max_tokens"], 1024);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_max_tokens_field_custom() {
        let compat = ProviderCompat {
            max_tokens_field: Some("max_completion_tokens".into()),
            ..Default::default()
        };
        let provider = OpenAIProvider::new("key", "http://localhost", compat);
        let req = LlmRequest {
            model: "gpt-4o".into(),
            system: String::new(),
            messages: vec![],
            tools: vec![],
            max_tokens: 2048,
            thinking: None,
            reasoning_effort: None,
        };
        let body = provider.build_request_body(&req, provider.should_sanitize_tool_schemas());
        assert_eq!(body["max_completion_tokens"], 2048);
        assert!(body.get("max_tokens").is_none());
    }

    // --- merge_assistant_messages ---

    #[test]
    fn test_merge_assistant_messages_enabled() {
        let messages = vec![
            Message::new(
                Role::Assistant,
                vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            ),
            Message::new(
                Role::Assistant,
                vec![ContentBlock::Text {
                    text: " world".into(),
                }],
            ),
        ];
        let result = OpenAIProvider::build_messages(&messages, "", &openai_compat(), false);
        let assistant_msgs: Vec<_> = result.iter().filter(|m| m["role"] == "assistant").collect();
        assert_eq!(assistant_msgs.len(), 1);
        assert_eq!(assistant_msgs[0]["content"], "hello world");
    }

    #[test]
    fn test_merge_assistant_messages_disabled() {
        let messages = vec![
            Message::new(
                Role::Assistant,
                vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            ),
            Message::new(
                Role::Assistant,
                vec![ContentBlock::Text {
                    text: " world".into(),
                }],
            ),
        ];
        let result = OpenAIProvider::build_messages(&messages, "", &no_compat(), false);
        let assistant_msgs: Vec<_> = result.iter().filter(|m| m["role"] == "assistant").collect();
        assert_eq!(assistant_msgs.len(), 2);
    }

    // --- clean_orphan_tool_calls ---

    #[test]
    fn test_clean_orphan_tool_calls_enabled() {
        let messages = vec![
            Message::new(
                Role::Assistant,
                vec![
                    ContentBlock::ToolUse {
                        id: "tc1".into(),
                        name: "bash".into(),
                        input: json!({}),
                        extra: None,
                    },
                    ContentBlock::ToolUse {
                        id: "tc2".into(),
                        name: "read".into(),
                        input: json!({}),
                        extra: None,
                    },
                ],
            ),
            Message::new(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "tc1".into(),
                    content: "ok".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
            // tc2 has no result -> orphan
        ];
        let result = OpenAIProvider::build_messages(&messages, "", &openai_compat(), false);
        let assistant = result.iter().find(|m| m["role"] == "assistant").unwrap();
        let tcs = assistant["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["id"], "tc1");
    }

    #[test]
    fn test_clean_orphan_tool_calls_disabled() {
        let messages = vec![
            Message::new(
                Role::Assistant,
                vec![
                    ContentBlock::ToolUse {
                        id: "tc1".into(),
                        name: "bash".into(),
                        input: json!({}),
                        extra: None,
                    },
                    ContentBlock::ToolUse {
                        id: "tc2".into(),
                        name: "read".into(),
                        input: json!({}),
                        extra: None,
                    },
                ],
            ),
            Message::new(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "tc1".into(),
                    content: "ok".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
        ];
        let result = OpenAIProvider::build_messages(&messages, "", &no_compat(), false);
        let assistant = result.iter().find(|m| m["role"] == "assistant").unwrap();
        let tcs = assistant["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 2);
    }

    // --- dedup_tool_results ---

    #[test]
    fn test_dedup_tool_results_enabled() {
        let messages = vec![
            Message::new(
                Role::Assistant,
                vec![ContentBlock::ToolUse {
                    id: "tc1".into(),
                    name: "bash".into(),
                    input: json!({}),
                    extra: None,
                }],
            ),
            Message::new(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "tc1".into(),
                    content: "first".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
            Message::new(
                Role::Tool,
                vec![ContentBlock::ToolResult {
                    tool_use_id: "tc1".into(),
                    content: "second".into(),
                    is_error: false,
                    images: Vec::new(),
                }],
            ),
        ];
        let result = OpenAIProvider::build_messages(&messages, "", &openai_compat(), false);
        let tool_msgs: Vec<_> = result.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(tool_msgs.len(), 1);
        assert_eq!(tool_msgs[0]["content"], "second");
    }

    // --- usage token parsing ---

    #[test]
    fn test_usage_from_trailing_chunk() {
        // OpenAI sends usage in a trailing chunk where choices:[] — the Done
        // event must carry the token counts from that chunk, not zeros.
        let mut state = StreamState::new();

        // chunk 1: finish_reason + text delta, no usage
        let chunk1 = r#"{"choices":[{"delta":{"content":"hi"},"finish_reason":"stop"}]}"#;
        let events = parse_sse_chunk(chunk1, &mut state, false);
        // TextDelta is emitted immediately; Done is deferred.
        assert!(
            events.iter().all(|e| !matches!(e, LlmEvent::Done { .. })),
            "Done should be deferred, not emitted with finish_reason chunk"
        );
        assert!(state.pending_done.is_some());

        // chunk 2: trailing usage-only chunk (choices:[])
        let chunk2 = r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let events2 = parse_sse_chunk(chunk2, &mut state, false);
        assert!(events2.is_empty());
        assert_eq!(state.input_tokens, 10);
        assert_eq!(state.output_tokens, 5);

        // [DONE] — flush with final counts
        let done = state.flush_done().expect("pending_done should be Some");
        match done {
            LlmEvent::Done { stop_reason, usage } => {
                assert_eq!(stop_reason, StopReason::EndTurn);
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn test_usage_in_finish_chunk() {
        // Some providers/models include usage in the same chunk as finish_reason.
        // Counts should still be correct after flush.
        let mut state = StreamState::new();

        // No text delta here, only finish_reason + usage in the same chunk.
        let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":8,"completion_tokens":3}}"#;
        let events = parse_sse_chunk(chunk, &mut state, false);
        assert!(
            events.iter().all(|e| !matches!(e, LlmEvent::Done { .. })),
            "Done should be deferred even when usage is in the finish chunk"
        );
        assert_eq!(state.output_tokens, 3);

        let done = state.flush_done().unwrap();
        match done {
            LlmEvent::Done { usage, .. } => {
                assert_eq!(usage.output_tokens, 3);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn test_build_tools_deferred_has_empty_parameters() {
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
        let result = OpenAIProvider::build_tools(&tools, false);

        // Core tool has full parameters
        let read_params = &result[0]["function"]["parameters"];
        assert!(read_params["properties"].get("path").is_some());

        // Deferred tool has empty parameters and modified description
        let spawn_params = &result[1]["function"]["parameters"];
        assert!(spawn_params["properties"].as_object().unwrap().is_empty());
        let spawn_desc = result[1]["function"]["description"].as_str().unwrap();
        assert!(spawn_desc.contains("ToolSearch"));
    }

    #[test]
    fn test_request_body_uses_explicit_schema_sanitize_snapshot() {
        let provider = OpenAIProvider::new("key", "http://localhost", openai_compat());
        let mut request = simple_request();
        request.tools.push(ToolDef {
            name: "Read".into(),
            description: "Read a file".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "oneOf": [{ "required": ["path"] }]
            }),
            deferred: false,
        });

        provider.sanitize_tool_schemas.store(true, Ordering::Release);

        let unsanitized = provider.build_request_body(&request, false);
        assert!(
            unsanitized["tools"][0]["function"]["parameters"]
                .get("oneOf")
                .is_some()
        );

        let sanitized = provider.build_request_body(&request, true);
        assert!(
            sanitized["tools"][0]["function"]["parameters"]
                .get("oneOf")
                .is_none()
        );
    }

    #[test]
    fn usage_includes_prompt_cache_hit_tokens() {
        // DeepSeek reports prompt_cache_hit_tokens separately;
        // input_tokens should be the sum of prompt_tokens + prompt_cache_hit_tokens
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":500,"completion_tokens":100,"prompt_cache_hit_tokens":999500}}"#;
        let _ = parse_sse_chunk(chunk, &mut state, false);

        assert_eq!(state.input_tokens, 1_000_000);
        assert_eq!(state.output_tokens, 100);
    }

    #[test]
    fn usage_with_prompt_tokens_details_cached() {
        // OpenAI standard: prompt_tokens already includes cached_tokens (it's the total)
        // prompt_tokens_details.cached_tokens is informational only
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1000000,"completion_tokens":100,"prompt_tokens_details":{"cached_tokens":999000}}}"#;
        let _ = parse_sse_chunk(chunk, &mut state, false);

        // prompt_tokens is already the full total for OpenAI
        assert_eq!(state.input_tokens, 1_000_000);
        assert_eq!(state.output_tokens, 100);
    }

    #[test]
    fn usage_without_cache_fields_unchanged() {
        // Provider that only sends prompt_tokens (no cache fields)
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":50000,"completion_tokens":200}}"#;
        let _ = parse_sse_chunk(chunk, &mut state, false);

        assert_eq!(state.input_tokens, 50_000);
        assert_eq!(state.output_tokens, 200);
    }

    #[test]
    fn tool_calls_with_stop_finish_reason_are_rejected() {
        // A non-tool terminal cannot promote earlier structured fragments into
        // executable calls.
        let mut state = StreamState::new();

        // chunk 1: tool call delta (name + partial args)
        let chunk1 = r#"{"choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"extra_content":{},"function":{"arguments":"{\"skill\":\"test\",\"args\":\"hello\"}","name":"Skill"},"id":"call_abc123","type":"function"}]},"index":0}]}"#;
        let events1 = parse_sse_chunk(chunk1, &mut state, false);
        let progress = events1
            .iter()
            .find(|e| matches!(e, LlmEvent::ToolUseDelta { .. }))
            .expect("tool call deltas should announce running work before finish_reason");
        if let LlmEvent::ToolUseDelta { id, name, input } = progress {
            assert_eq!(id, "call_abc123");
            assert_eq!(name, "Skill");
            assert_eq!(input.as_ref().unwrap()["skill"], "test");
        }
        assert_eq!(state.tool_calls.len(), 1);
        assert_eq!(state.tool_calls[0].name, "Skill");

        // chunk 2: finish_reason:"stop" (not "tool_calls")
        let chunk2 = r#"{"choices":[{"delta":{"role":"assistant"},"finish_reason":"stop","index":0}],"usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120}}"#;
        let events2 = parse_sse_chunk(chunk2, &mut state, false);

        assert!(events2.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("finished with stop"))
        ));
        assert!(events2
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. } | LlmEvent::Done { .. })));
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn malformed_single_chunk_tool_arguments_emit_error_not_tool_use() {
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_bad",
                        "function": {
                            "name": "update_base",
                            "arguments": "{\"kb_id\":]"
                        }
                    }]
                },
                "finish_reason": "tool_calls",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, false);

        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. })),
            "malformed arguments must never become an executable tool call: {events:?}"
        );
        let message = events
            .iter()
            .find_map(|event| match event {
                LlmEvent::Error(message) => Some(message),
                _ => None,
            })
            .expect("malformed arguments should surface an explicit provider error");
        assert!(message.contains("malformed JSON arguments"));
        assert!(message.contains("update_base"));
        assert!(message.contains("call_bad"));
        assert!(state.pending_done.is_none());
    }

    #[test]
    fn malformed_streamed_tool_arguments_emit_error_after_aggregation() {
        let mut state = StreamState::new();
        let first = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_split_bad",
                        "function": {
                            "name": "create_base",
                            "arguments": "{\"name\":"
                        }
                    }]
                },
                "finish_reason": null,
                "index": 0
            }]
        })
        .to_string();
        let second = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": { "arguments": "]}" }
                    }]
                },
                "finish_reason": "tool_calls",
                "index": 0
            }]
        })
        .to_string();

        let first_events = parse_sse_chunk(&first, &mut state, false);
        assert!(
            first_events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
        );
        let final_events = parse_sse_chunk(&second, &mut state, false);

        assert!(
            final_events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
        );
        assert!(final_events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("call_split_bad"))
        ));
        assert!(state.pending_done.is_none());
    }

    #[test]
    fn non_string_tool_arguments_are_rejected() {
        let mut state = StreamState::new();
        let chunk = json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_object_args",
                        "function": {
                            "name": "update_base",
                            "arguments": { "kb_id": "kb_1" }
                        }
                    }]
                },
                "finish_reason": "tool_calls",
                "index": 0
            }]
        })
        .to_string();

        let events = parse_sse_chunk(&chunk, &mut state, false);
        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("non-string function arguments"))
        ));
        assert!(events
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. } | LlmEvent::Done { .. })));
        assert!(state.drain_terminal_events().is_empty());
    }

    #[test]
    fn tool_call_argument_stream_emits_file_target_preview_before_finish() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_write_1","function":{"name":"Write","arguments":"{\"file_path\":\"/tmp/snake.html\",\"content\":\""}}]},"finish_reason":null,"index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, false);

        let progress = events
            .iter()
            .find(|e| matches!(e, LlmEvent::ToolUseDelta { .. }))
            .expect("Write should be announced while arguments are still streaming");
        if let LlmEvent::ToolUseDelta { id, name, input } = progress {
            assert_eq!(id, "call_write_1");
            assert_eq!(name, "Write");
            assert_eq!(input.as_ref().unwrap()["file_path"], "/tmp/snake.html");
            assert!(
                input.as_ref().unwrap().get("content").is_none(),
                "large write content must not be pushed as a progress preview"
            );
        }
    }

    #[test]
    fn auto_tool_id_is_stable_between_progress_and_final_tool_use() {
        let mut state = StreamState::new();

        let chunk1 = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"Bash","arguments":"{\"command\":\"bun test\"}"}}]},"finish_reason":null,"index":0}]}"#;
        let events1 = parse_sse_chunk(chunk1, &mut state, true);
        let progress_id = events1
            .iter()
            .find_map(|e| match e {
                LlmEvent::ToolUseDelta { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("auto-id providers should still emit a stable progress event");

        let chunk2 = r#"{"choices":[{"delta":{},"finish_reason":"tool_calls","index":0}]}"#;
        let events2 = parse_sse_chunk(chunk2, &mut state, true);
        assert!(events2
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
        let terminal = state.drain_terminal_events();
        let final_id = terminal
            .iter()
            .find_map(|e| match e {
                LlmEvent::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .expect("final tool use should be emitted");

        assert_eq!(progress_id, final_id);
    }

    #[test]
    fn stop_without_tool_calls_unchanged() {
        // Standard stop without tool calls should still produce EndTurn.
        let mut state = StreamState::new();

        let chunk =
            r#"{"choices":[{"delta":{"content":"done"},"finish_reason":"stop","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, false);

        let text_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, LlmEvent::TextDelta(_)))
            .collect();
        assert_eq!(text_events.len(), 1);

        let done = state.flush_done().unwrap();
        match done {
            LlmEvent::Done { stop_reason, .. } => {
                assert_eq!(stop_reason, StopReason::EndTurn);
            }
            other => panic!("expected Done with EndTurn, got {other:?}"),
        }
    }

    #[test]
    fn test_auto_tool_id_generates_id_when_empty() {
        let mut state = StreamState::new();

        // Simulate a provider that returns tool_calls without an id field
        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, true);
        assert!(events
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
        let terminal = state.drain_terminal_events();

        let tool_use = terminal
            .iter()
            .find(|e| matches!(e, LlmEvent::ToolUse { .. }))
            .expect("should emit ToolUse event");

        if let LlmEvent::ToolUse { id, name, .. } = tool_use {
            assert!(!id.is_empty(), "id should be auto-generated, not empty");
            assert!(id.starts_with("call_"), "id should have call_ prefix");
            assert_eq!(name, "get_weather");
        }
    }

    #[test]
    fn test_auto_tool_id_preserves_existing_id() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_existing_123","function":{"name":"read_file","arguments":"{}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, true);
        assert!(events
            .iter()
            .all(|event| !matches!(event, LlmEvent::ToolUse { .. })));
        let terminal = state.drain_terminal_events();

        let tool_use = terminal
            .iter()
            .find(|e| matches!(e, LlmEvent::ToolUse { .. }))
            .expect("should emit ToolUse event");

        if let LlmEvent::ToolUse { id, .. } = tool_use {
            assert_eq!(id, "call_existing_123", "existing id should be preserved");
        }
    }

    #[test]
    fn missing_tool_id_emits_error_when_auto_id_is_disabled() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"get_weather","arguments":"{}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, false);

        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
        );
        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("without a call id"))
        ));
    }

    #[test]
    fn missing_tool_name_emits_error() {
        let mut state = StreamState::new();

        let chunk = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_missing_name","function":{"arguments":"{}"}}]},"finish_reason":"tool_calls","index":0}]}"#;
        let events = parse_sse_chunk(chunk, &mut state, false);

        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. }))
        );
        assert!(events.iter().any(
            |event| matches!(event, LlmEvent::Error(message) if message.contains("missing function name") && message.contains("call_missing_name"))
        ));
    }
}
