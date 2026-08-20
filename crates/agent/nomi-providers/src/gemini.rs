use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use nomi_config::compat::{self, ProviderCompat};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use nomi_types::tool::{ToolDef, truncate_deferred_description};

use crate::anthropic_shared::StreamOutcome;
use crate::{LlmProvider, ProviderError};

const GOOGLE_API_KEY_HEADER: HeaderName = HeaderName::from_static("x-goog-api-key");
const MAX_BUFFERED_SSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_CALLS_PER_TURN: usize = 128;

/// Native Google Gemini `streamGenerateContent` provider.
///
/// This deliberately does not route through the OpenAI compatibility API:
/// Gemini's native wire format is required to preserve function-call IDs and
/// thought signatures across multi-step tool turns.
pub struct GeminiProvider {
    api_keys: Vec<String>,
    current_api_key: AtomicUsize,
    base_url: String,
    compat: ProviderCompat,
}

impl GeminiProvider {
    pub fn new(api_key: &str, base_url: &str, compat: ProviderCompat) -> Self {
        Self {
            api_keys: crate::parse_api_keys(api_key),
            current_api_key: AtomicUsize::new(0),
            base_url: base_url.to_owned(),
            compat,
        }
    }

    fn build_url(&self, model: &str) -> Result<String, ProviderError> {
        let mut url = reqwest::Url::parse(self.base_url.trim()).map_err(|error| {
            ProviderError::Connection(format!("Invalid Gemini base URL: {error}"))
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderError::Connection(
                "Gemini endpoint must be HTTP(S), have a host, and contain no credentials or fragment"
                    .to_owned(),
            ));
        }

        // NomiFun's capability resolver expands `{model}` into a complete,
        // modality-specific endpoint and marks that contract with an explicit
        // empty api_path. Preserve that endpoint (including its query) exactly;
        // standalone/default config leaves api_path unset and uses the native
        // root-to-endpoint construction below.
        if self.compat.api_path.as_deref() == Some("") {
            return Ok(url.into());
        }
        if url.query().is_some() {
            return Err(ProviderError::Connection(
                "Gemini root base URL must not contain a query".to_owned(),
            ));
        }

        let model = model.trim().strip_prefix("models/").unwrap_or(model.trim());
        if model.is_empty()
            || model.contains('/')
            || model.contains('?')
            || model.contains('#')
            || model.chars().any(char::is_whitespace)
        {
            return Err(ProviderError::Connection(
                "Gemini model must be a non-empty model ID, not a URL or path".to_owned(),
            ));
        }

        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                ProviderError::Connection(
                    "Gemini base URL cannot be used as a hierarchical endpoint".to_owned(),
                )
            })?;
            segments.pop_if_empty();
            segments.push("models");
            segments.push(&format!("{model}:streamGenerateContent"));
        }
        url.query_pairs_mut().append_pair("alt", "sse");
        Ok(url.into())
    }

    fn build_headers(api_key: &str) -> Result<HeaderMap, ProviderError> {
        if api_key.trim().is_empty() {
            return Err(ProviderError::Connection(
                "No Gemini API key configured".to_owned(),
            ));
        }
        let mut key = HeaderValue::from_str(api_key).map_err(|error| {
            ProviderError::Connection(format!("Invalid Gemini API key header: {error}"))
        })?;
        key.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(GOOGLE_API_KEY_HEADER, key);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    fn build_request_body(&self, request: &LlmRequest) -> Result<Value, ProviderError> {
        let call_names = collect_tool_call_names(&request.messages)?;
        let mut system_parts = Vec::new();
        if !request.system.is_empty() {
            let system = strip_patterns(&request.system, &self.compat);
            if !system.is_empty() {
                system_parts.push(json!({ "text": system }));
            }
        }

        let mut contents = Vec::new();
        for message in &request.messages {
            if message.role == Role::System {
                append_system_parts(message, &self.compat, &mut system_parts)?;
                continue;
            }

            let role = match message.role {
                Role::Assistant => "model",
                Role::User | Role::Tool => "user",
                Role::System => unreachable!(),
            };
            let parts = build_message_parts(message, &call_names, &self.compat)?;
            if !parts.is_empty() {
                append_content(
                    &mut contents,
                    role,
                    parts,
                    self.compat.merge_same_role(),
                );
            }
        }
        if contents.is_empty() {
            return Err(ProviderError::Parse(
                "Gemini request contains no user or model content".to_owned(),
            ));
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {}
        });
        if let Some(limit) = request.max_tokens {
            body["generationConfig"]["maxOutputTokens"] = json!(limit);
        }
        let has_system_instruction = !system_parts.is_empty();
        if has_system_instruction {
            body["systemInstruction"] = json!({ "parts": system_parts });
        }
        if !request.tools.is_empty() {
            body["tools"] = json!([{
                "functionDeclarations": build_tools(
                    &request.tools,
                    self.compat.sanitize_schema(),
                )?
            }]);
        }
        let mut body = crate::request_body_with_extra(
            &self.compat,
            crate::OutputCeilingLocation::GeminiGenerationConfig,
            body,
        );
        let object = body
            .as_object_mut()
            .expect("typed Gemini request body is an object");
        // Gemini's model is a typed URL path component, not a request-body
        // field. Empty typed option sets override opaque extras.
        object.remove("model");
        if request.tools.is_empty() {
            object.remove("tools");
        }
        if object
            .get("generationConfig")
            .and_then(Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
        {
            object.remove("generationConfig");
        }
        if !has_system_instruction {
            object.remove("systemInstruction");
        }
        Ok(body)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        let url = self.build_url(&request.model)?;
        let body = self.build_request_body(request)?;
        let client = crate::http_client()?;
        let (response, headers) = crate::send_initial_with_key_rotation(
            &client,
            &url,
            &body,
            &self.api_keys,
            &self.current_api_key,
            "gemini",
            Self::build_headers,
        )
        .await
        .map_err(classify_google_error)?;

        let (tx, rx) = mpsc::channel(64);
        let client = client.clone();
        let auto_tool_id = self.compat.auto_tool_id();
        let redactor = nomifun_net::secret_redaction::SecretRedactor::new(&self.api_keys);
        tokio::spawn(async move {
            let outcome = process_sse_stream(response, &tx, auto_tool_id).await;
            crate::retry::finish_stream_with_retry(
                outcome,
                &tx,
                || async {
                    crate::send_initial(&client, &url, &headers, &body, &redactor)
                        .await
                        .map_err(classify_google_error)
                },
                |response| process_sse_stream(response, &tx, auto_tool_id),
            )
            .await;
        });
        Ok(rx)
    }
}

fn strip_patterns(text: &str, compat: &ProviderCompat) -> String {
    let mut result = text.to_owned();
    if let Some(patterns) = &compat.strip_patterns {
        for pattern in patterns {
            result = result.replace(pattern, "");
        }
    }
    result
}

fn append_system_parts(
    message: &Message,
    compat: &ProviderCompat,
    target: &mut Vec<Value>,
) -> Result<(), ProviderError> {
    for block in &message.content {
        match block {
            ContentBlock::Text { text } => {
                let text = strip_patterns(text, compat);
                if !text.is_empty() {
                    target.push(json!({ "text": text }));
                }
            }
            _ => {
                return Err(ProviderError::Parse(
                    "Gemini system messages may contain text only".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn collect_tool_call_names(messages: &[Message]) -> Result<HashMap<String, String>, ProviderError> {
    let mut names = HashMap::new();
    for message in messages {
        for block in &message.content {
            let ContentBlock::ToolUse { id, name, .. } = block else {
                continue;
            };
            if id.trim().is_empty()
                || id.trim() != id
                || name.trim().is_empty()
                || name.trim() != name
            {
                return Err(ProviderError::Parse(
                    "Gemini tool-call history contains an empty call ID or function name"
                        .to_owned(),
                ));
            }
            match names.insert(id.clone(), name.clone()) {
                Some(previous) if previous != *name => {
                    return Err(ProviderError::Parse(format!(
                        "Gemini tool-call ID '{id}' is associated with multiple function names"
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(names)
}

fn build_message_parts(
    message: &Message,
    call_names: &HashMap<String, String>,
    compat: &ProviderCompat,
) -> Result<Vec<Value>, ProviderError> {
    let mut parts = Vec::new();
    for block in &message.content {
        match (message.role, block) {
            (_, ContentBlock::Text { text }) => {
                let text = strip_patterns(text, compat);
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
            }
            (Role::User | Role::Tool, ContentBlock::Image { media_type, data }) => {
                if compat.supports_image() {
                    parts.push(inline_data_part(media_type, data));
                } else {
                    parts.push(json!({
                        "text": "[Image omitted: this model is configured without image input support]"
                    }));
                }
            }
            (
                Role::Assistant,
                ContentBlock::Thinking {
                    thinking,
                    signature,
                },
            ) => {
                if !thinking.is_empty() || signature.is_some() {
                    let mut part = json!({ "text": thinking, "thought": true });
                    if let Some(signature) = signature {
                        part["thoughtSignature"] = json!(signature);
                    }
                    parts.push(part);
                }
            }
            (
                Role::Assistant,
                ContentBlock::ToolUse {
                    id,
                    name,
                    input,
                    extra,
                },
            ) => {
                if id.trim().is_empty()
                    || id.trim() != id
                    || name.trim().is_empty()
                    || name.trim() != name
                    || !input.is_object()
                {
                    return Err(ProviderError::Parse(
                        "Gemini function calls require non-empty IDs/names and object arguments"
                            .to_owned(),
                    ));
                }
                let mut part = json!({
                    "functionCall": {
                        "id": id,
                        "name": name,
                        "args": input
                    }
                });
                if let Some(signature) = extra_thought_signature(extra)? {
                    part["thoughtSignature"] = json!(signature);
                }
                parts.push(part);
            }
            (
                Role::User | Role::Tool,
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    images,
                },
            ) => {
                let name = call_names.get(tool_use_id).ok_or_else(|| {
                    ProviderError::Parse(format!(
                        "Gemini function response '{tool_use_id}' has no matching function call"
                    ))
                })?;
                let result = serde_json::from_str::<Value>(content)
                    .unwrap_or_else(|_| Value::String(content.clone()));
                let response = if *is_error {
                    json!({ "error": result })
                } else {
                    json!({ "result": result })
                };
                let mut function_response = json!({
                    "id": tool_use_id,
                    "name": name,
                    "response": response
                });
                if compat.supports_image() {
                    let image_parts: Vec<Value> = images
                        .iter()
                        .filter(|image| image.media_type.starts_with("image/"))
                        .map(|image| inline_data_part(&image.media_type, &image.data))
                        .collect();
                    if !image_parts.is_empty() {
                        function_response["parts"] = json!(image_parts);
                    }
                }
                parts.push(json!({ "functionResponse": function_response }));
            }
            _ => {
                return Err(ProviderError::Parse(format!(
                    "Gemini cannot encode a {:?} content block in a {:?} message",
                    block, message.role
                )));
            }
        }
    }
    Ok(parts)
}

fn extra_thought_signature(extra: &Option<Value>) -> Result<Option<&str>, ProviderError> {
    let Some(extra) = extra else {
        return Ok(None);
    };
    for field in ["thoughtSignature", "thought_signature"] {
        if let Some(value) = extra.get(field) {
            return value.as_str().map(Some).ok_or_else(|| {
                ProviderError::Parse(format!(
                    "Gemini tool metadata field '{field}' must be a string"
                ))
            });
        }
    }
    Ok(None)
}

fn inline_data_part(media_type: &str, data: &str) -> Value {
    json!({
        "inlineData": {
            "mimeType": media_type,
            "data": data
        }
    })
}

fn append_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>, merge: bool) {
    if merge
        && let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut)
    {
        existing.extend(parts);
        return;
    }
    contents.push(json!({ "role": role, "parts": parts }));
}

fn build_tools(tools: &[ToolDef], sanitize: bool) -> Result<Vec<Value>, ProviderError> {
    let mut names = HashSet::new();
    tools
        .iter()
        .map(|tool| {
            if tool.name.trim().is_empty()
                || tool.name.trim() != tool.name
                || !names.insert(tool.name.as_str())
            {
                return Err(ProviderError::Parse(
                    "Gemini tool declarations require unique, non-empty names".to_owned(),
                ));
            }
            if tool.deferred {
                let description = truncate_deferred_description(&tool.description);
                Ok(json!({
                    "name": tool.name,
                    "description": format!(
                        "(Deferred) {description} - Use ToolSearch to load the full schema before calling."
                    ),
                    "parameters": {
                        "type": "object",
                        "properties": {}
                    }
                }))
            } else {
                let parameters = if sanitize {
                    compat::sanitize_json_schema(&tool.input_schema)
                } else {
                    tool.input_schema.clone()
                };
                Ok(json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": parameters
                }))
            }
        })
        .collect()
}

#[derive(Debug)]
struct PendingToolCall {
    id: String,
    name: String,
    input: Value,
    extra: Option<Value>,
}

struct GeminiStreamState {
    pending_calls: Vec<PendingToolCall>,
    call_ids: HashSet<String>,
    input_tokens: u64,
    output_tokens: u64,
    reasoning_tokens: u64,
    cache_read_tokens: u64,
    finish_reason: Option<String>,
    stop_reason: Option<StopReason>,
    emitted_content: bool,
}

impl GeminiStreamState {
    fn new() -> Self {
        Self {
            pending_calls: Vec::new(),
            call_ids: HashSet::new(),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            finish_reason: None,
            stop_reason: None,
            emitted_content: false,
        }
    }

    fn outcome(&self, error: ProviderError) -> StreamOutcome {
        if self.emitted_content {
            StreamOutcome::FailedPartial(error)
        } else {
            StreamOutcome::FailedEmpty(error)
        }
    }

    fn process_response(
        &mut self,
        response: &Value,
        auto_tool_id: bool,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let object = response.as_object().ok_or_else(|| {
            ProviderError::Parse("Gemini returned a non-object SSE payload".to_owned())
        })?;
        if let Some(error) = object.get("error").filter(|value| !value.is_null()) {
            return Err(google_stream_error(error));
        }
        update_usage(object, self)?;

        if let Some(feedback) = object.get("promptFeedback") {
            let feedback = feedback.as_object().ok_or_else(|| {
                ProviderError::Parse("Gemini returned non-object promptFeedback".to_owned())
            })?;
            if let Some(reason) = feedback.get("blockReason").and_then(Value::as_str) {
                return Err(ProviderError::Api {
                    status: 400,
                    message: format!("Gemini blocked the prompt: {reason}"),
                });
            }
        }

        let Some(candidates) = object.get("candidates") else {
            if object.contains_key("usageMetadata") || object.contains_key("promptFeedback") {
                return Ok(Vec::new());
            }
            return Err(ProviderError::Parse(
                "Gemini SSE payload has neither candidates nor accounting metadata".to_owned(),
            ));
        };
        let candidates = candidates.as_array().ok_or_else(|| {
            ProviderError::Parse("Gemini returned non-array candidates".to_owned())
        })?;
        if candidates.is_empty() {
            if object.contains_key("usageMetadata") {
                return Ok(Vec::new());
            }
            return Err(ProviderError::Parse(
                "Gemini returned an empty candidates array before completion".to_owned(),
            ));
        }
        if candidates.len() != 1 {
            return Err(ProviderError::Parse(format!(
                "Gemini returned {} candidates; Nomi requires exactly one",
                candidates.len()
            )));
        }

        let candidate = candidates[0].as_object().ok_or_else(|| {
            ProviderError::Parse("Gemini returned a non-object candidate".to_owned())
        })?;
        let parts = candidate
            .get("content")
            .and_then(Value::as_object)
            .and_then(|content| content.get("parts"));
        if self.finish_reason.is_some()
            && parts
                .and_then(Value::as_array)
                .is_some_and(|parts| !parts.is_empty())
        {
            return Err(ProviderError::Parse(
                "Gemini returned content after a terminal finishReason".to_owned(),
            ));
        }

        let mut events = Vec::new();
        if let Some(parts) = parts {
            let parts = parts.as_array().ok_or_else(|| {
                ProviderError::Parse("Gemini returned non-array content parts".to_owned())
            })?;
            for part in parts {
                events.extend(self.process_part(part, auto_tool_id)?);
            }
        }

        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str)
            && reason != "FINISH_REASON_UNSPECIFIED"
        {
            self.record_finish_reason(
                reason,
                candidate.get("finishMessage").and_then(Value::as_str),
            )?;
        }
        Ok(events)
    }

    fn process_part(
        &mut self,
        part: &Value,
        auto_tool_id: bool,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let part = part.as_object().ok_or_else(|| {
            ProviderError::Parse("Gemini returned a non-object content part".to_owned())
        })?;
        let has_text = part.contains_key("text");
        let has_call = part.contains_key("functionCall");
        if has_text && has_call {
            return Err(ProviderError::Parse(
                "Gemini returned a part with multiple mutually exclusive payloads".to_owned(),
            ));
        }

        let signature = optional_string(part, "thoughtSignature")?
            .or(optional_string(part, "thought_signature")?);
        if has_call {
            let call = part["functionCall"].as_object().ok_or_else(|| {
                ProviderError::Parse("Gemini returned non-object functionCall".to_owned())
            })?;
            let name = call
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty() && name.trim() == *name)
                .ok_or_else(|| {
                    ProviderError::Parse(
                        "Gemini returned a functionCall without a name".to_owned(),
                    )
                })?;
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty() && id.trim() == *id)
                .map(str::to_owned)
                .or_else(|| auto_tool_id.then(generate_call_id))
                .ok_or_else(|| {
                    ProviderError::Parse(
                        "Gemini returned a functionCall without an ID".to_owned(),
                    )
                })?;
            let input = match call.get("args") {
                None | Some(Value::Null) => json!({}),
                Some(value) if value.is_object() => value.clone(),
                Some(_) => {
                    return Err(ProviderError::Parse(format!(
                        "Gemini returned non-object arguments for function '{name}'"
                    )));
                }
            };
            if self.pending_calls.len() >= MAX_TOOL_CALLS_PER_TURN {
                return Err(ProviderError::Parse(format!(
                    "Gemini exceeded the maximum of {MAX_TOOL_CALLS_PER_TURN} tool calls in one turn"
                )));
            }
            if !self.call_ids.insert(id.clone()) {
                return Err(ProviderError::Parse(format!(
                    "Gemini reused functionCall ID '{id}' in one turn"
                )));
            }
            self.pending_calls.push(PendingToolCall {
                id,
                name: name.to_owned(),
                input,
                extra: signature.map(|value| json!({ "thoughtSignature": value })),
            });
            return Ok(Vec::new());
        }

        let mut events = Vec::new();
        if has_text {
            let text = part["text"].as_str().ok_or_else(|| {
                ProviderError::Parse("Gemini returned a non-string text part".to_owned())
            })?;
            let thought = match part.get("thought") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(ProviderError::Parse(
                        "Gemini returned a non-boolean thought marker".to_owned(),
                    ));
                }
            };
            if !text.is_empty() {
                events.push(if thought {
                    LlmEvent::ThinkingDelta(text.to_owned())
                } else {
                    LlmEvent::TextDelta(text.to_owned())
                });
            }
        } else if signature.is_none() {
            return Err(ProviderError::Parse(
                "Gemini returned an unsupported content part".to_owned(),
            ));
        }
        if let Some(signature) = signature {
            events.push(LlmEvent::ThinkingSignature(signature.to_owned()));
        }
        Ok(events)
    }

    fn record_finish_reason(
        &mut self,
        reason: &str,
        detail: Option<&str>,
    ) -> Result<(), ProviderError> {
        if let Some(previous) = &self.finish_reason {
            if previous == reason {
                return Ok(());
            }
            return Err(ProviderError::Parse(format!(
                "Gemini changed finishReason from '{previous}' to '{reason}'"
            )));
        }

        let stop_reason = match reason {
            "STOP" => {
                if self.pending_calls.is_empty() {
                    StopReason::EndTurn
                } else {
                    StopReason::ToolUse
                }
            }
            "MAX_TOKENS" => StopReason::MaxTokens,
            blocked => {
                let suffix = detail
                    .filter(|detail| !detail.trim().is_empty())
                    .map(|detail| format!(": {detail}"))
                    .unwrap_or_default();
                return Err(ProviderError::Api {
                    status: 400,
                    message: format!("Gemini stopped generation with {blocked}{suffix}"),
                });
            }
        };
        self.finish_reason = Some(reason.to_owned());
        self.stop_reason = Some(stop_reason);
        Ok(())
    }

    fn terminal_events(&mut self) -> Result<Vec<LlmEvent>, ProviderError> {
        let stop_reason = self.stop_reason.ok_or_else(|| {
            ProviderError::StreamTruncated(
                "Gemini stream ended before a terminal finishReason".to_owned(),
            )
        })?;
        let mut events = Vec::with_capacity(self.pending_calls.len() + 1);
        // A response the ceiling cut off never executes its staged calls, even
        // though Gemini stages them fully parsed — the same policy the OpenAI
        // `length` and Anthropic `max_tokens` arms apply. Report each as a
        // non-executable truncation fact so a resumable round knows what was
        // reached for, instead of failing the whole turn with a parse error.
        let truncated = matches!(stop_reason, StopReason::MaxTokens);
        for call in std::mem::take(&mut self.pending_calls) {
            if truncated {
                let argument_bytes =
                    serde_json::to_string(&call.input).map_or(0, |json| json.len());
                events.push(LlmEvent::ToolUseTruncated {
                    id: call.id,
                    name: call.name,
                    argument_bytes,
                });
                continue;
            }
            events.push(LlmEvent::ToolUse {
                id: call.id,
                name: call.name,
                input: call.input,
                extra: call.extra,
            });
        }
        events.push(LlmEvent::Done {
            stop_reason,
            usage: TokenUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                reasoning_tokens: self.reasoning_tokens,
                cache_creation_tokens: 0,
                cache_read_tokens: self.cache_read_tokens,
            },
        });
        Ok(events)
    }
}

fn generate_call_id() -> String {
    format!("gemini_call_{}", uuid::Uuid::now_v7().simple())
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ProviderError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(ProviderError::Parse(format!(
            "Gemini returned non-string field '{field}'"
        ))),
    }
}

fn update_usage(
    response: &Map<String, Value>,
    state: &mut GeminiStreamState,
) -> Result<(), ProviderError> {
    let Some(usage) = response.get("usageMetadata") else {
        return Ok(());
    };
    let usage = usage.as_object().ok_or_else(|| {
        ProviderError::Parse("Gemini returned non-object usageMetadata".to_owned())
    })?;
    state.input_tokens = usage_u64(usage, "promptTokenCount")?.unwrap_or(state.input_tokens);
    state.cache_read_tokens =
        usage_u64(usage, "cachedContentTokenCount")?.unwrap_or(state.cache_read_tokens);
    let candidate_tokens =
        usage_u64(usage, "candidatesTokenCount")?.unwrap_or(state.output_tokens);
    let thought_tokens = usage_u64(usage, "thoughtsTokenCount")?.unwrap_or(0);
    state.reasoning_tokens = thought_tokens;
    state.output_tokens = candidate_tokens.checked_add(thought_tokens).ok_or_else(|| {
        ProviderError::Parse("Gemini returned overflowing output token usage".to_owned())
    })?;
    Ok(())
}

fn usage_u64(usage: &Map<String, Value>, field: &str) -> Result<Option<u64>, ProviderError> {
    match usage.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            ProviderError::Parse(format!("Gemini returned non-integer usage field '{field}'"))
        }),
    }
}

fn google_stream_error(error: &Value) -> ProviderError {
    let status = error
        .get("code")
        .and_then(Value::as_u64)
        .and_then(|code| u16::try_from(code).ok())
        .unwrap_or(500);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string());
    classify_google_error(ProviderError::Api { status, message })
}

fn classify_google_error(error: ProviderError) -> ProviderError {
    match error {
        ProviderError::Api { status, message } => {
            let message = google_error_message(&message);
            if is_prompt_too_long(status, &message) {
                ProviderError::PromptTooLong(message)
            } else if status == 429 {
                ProviderError::RateLimited {
                    retry_after_ms: 5000,
                    message,
                }
            } else {
                ProviderError::Api { status, message }
            }
        }
        ProviderError::RateLimited {
            retry_after_ms,
            message,
        } => ProviderError::RateLimited {
            retry_after_ms,
            message: google_error_message(&message),
        },
        other => other,
    }
}

fn google_error_message(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|json| {
            json.get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| raw.to_owned())
}

fn is_prompt_too_long(status: u16, message: &str) -> bool {
    if !matches!(status, 400 | 413) {
        return false;
    }
    let message = message.to_ascii_lowercase();
    (message.contains("input token count") || message.contains("context length"))
        && (message.contains("exceed") || message.contains("maximum"))
}

async fn process_sse_stream(
    response: reqwest::Response,
    tx: &mpsc::Sender<LlmEvent>,
    auto_tool_id: bool,
) -> StreamOutcome {
    let mut state = GeminiStreamState::new();
    let mut buffer = Vec::new();
    let mut stream = response.bytes_stream();
    let mut done_sentinel = false;

    'stream: while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => return state.outcome(ProviderError::from(error)),
        };
        buffer.extend_from_slice(&chunk);
        if buffer.len() > MAX_BUFFERED_SSE_BYTES {
            return state.outcome(ProviderError::Parse(format!(
                "Gemini SSE buffer exceeded {MAX_BUFFERED_SSE_BYTES} bytes"
            )));
        }

        while let Some((offset, delimiter_len)) = find_sse_boundary(&buffer) {
            let mut tail = buffer.split_off(offset + delimiter_len);
            let event = std::mem::replace(&mut buffer, std::mem::take(&mut tail));
            let event = &event[..offset];
            if event.len() > MAX_SSE_EVENT_BYTES {
                return state.outcome(ProviderError::Parse(format!(
                    "Gemini SSE event exceeded {MAX_SSE_EVENT_BYTES} bytes"
                )));
            }
            let data = match sse_data(event) {
                Ok(Some(data)) => data,
                Ok(None) => continue,
                Err(error) => return state.outcome(error),
            };
            if data.trim() == "[DONE]" {
                done_sentinel = true;
                break 'stream;
            }
            let json = match serde_json::from_str::<Value>(&data) {
                Ok(json) => json,
                Err(error) => {
                    return state.outcome(ProviderError::Parse(format!(
                        "Gemini returned malformed SSE JSON: {error}"
                    )));
                }
            };
            let events = match state.process_response(&json, auto_tool_id) {
                Ok(events) => events,
                Err(error) => return state.outcome(error),
            };
            for event in events {
                if matches!(
                    event,
                    LlmEvent::TextDelta(_)
                        | LlmEvent::ThinkingDelta(_)
                        | LlmEvent::ThinkingSignature(_)
                ) {
                    state.emitted_content = true;
                }
                if tx.send(event).await.is_err() {
                    return StreamOutcome::Ok;
                }
            }
        }
    }

    if !done_sentinel && !buffer.iter().all(u8::is_ascii_whitespace) {
        return state.outcome(ProviderError::Connection(
            "Gemini stream ended with an unterminated SSE event".to_owned(),
        ));
    }
    let events = match state.terminal_events() {
        Ok(events) => events,
        Err(error) => return state.outcome(error),
    };
    for event in events {
        if tx.send(event).await.is_err() {
            return StreamOutcome::Ok;
        }
    }
    StreamOutcome::Ok
}

fn find_sse_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    [b"\r\n\r\n".as_slice(), b"\n\n", b"\r\r"]
        .into_iter()
        .filter_map(|delimiter| {
            buffer
                .windows(delimiter.len())
                .position(|window| window == delimiter)
                .map(|offset| (offset, delimiter.len()))
        })
        .min_by_key(|(offset, _)| *offset)
}

fn sse_data(event: &[u8]) -> Result<Option<String>, ProviderError> {
    let event = std::str::from_utf8(event).map_err(|error| {
        ProviderError::Parse(format!("Gemini returned invalid UTF-8 SSE data: {error}"))
    })?;
    let mut data = Vec::new();
    for line in event.split(['\n', '\r']) {
        let Some(value) = line.strip_prefix("data:") else {
            continue;
        };
        data.push(value.strip_prefix(' ').unwrap_or(value));
    }
    if data.is_empty() {
        Ok(None)
    } else {
        Ok(Some(data.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_url_uses_model_path_and_sse_query() {
        let provider = GeminiProvider::new(
            "key",
            "https://generativelanguage.googleapis.com/v1beta/",
            ProviderCompat::gemini_defaults(),
        );
        assert_eq!(
            provider.build_url("models/gemini-3.6-flash").unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.6-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn exact_capability_endpoint_is_not_appended_twice() {
        let endpoint = "https://gateway.example/v1beta/models/gemini-custom:streamGenerateContent?alt=sse&route=one";
        let provider = GeminiProvider::new(
            "key",
            endpoint,
            ProviderCompat {
                api_path: Some(String::new()),
                ..ProviderCompat::gemini_defaults()
            },
        );
        assert_eq!(provider.build_url("ignored-model").unwrap(), endpoint);
    }

    #[test]
    fn url_rejects_model_path_injection() {
        let provider = GeminiProvider::new(
            "key",
            "https://generativelanguage.googleapis.com/v1beta",
            ProviderCompat::gemini_defaults(),
        );
        assert!(provider.build_url("../other:generateContent").is_err());
    }

    #[test]
    fn google_error_classifies_context_overflow() {
        let error = classify_google_error(ProviderError::Api {
            status: 400,
            message: json!({
                "error": {
                    "message": "The input token count exceeds the maximum number of tokens allowed"
                }
            })
            .to_string(),
        });
        assert!(matches!(error, ProviderError::PromptTooLong(_)));
    }

    #[test]
    fn sse_parser_accepts_multiline_data_and_crlf() {
        let event = b"event: message\r\ndata: {\"candidates\": []}\r\n";
        assert_eq!(
            sse_data(event).unwrap().as_deref(),
            Some("{\"candidates\": []}")
        );
        assert_eq!(find_sse_boundary(b"one\r\n\r\ntwo"), Some((3, 4)));
    }

    #[tokio::test]
    async fn incomplete_stream_never_commits_staged_function_call() {
        let frame = json!({
            "candidates": [{
                "content": { "parts": [{
                    "functionCall": {
                        "id": "dangerous-call",
                        "name": "Delete",
                        "args": { "path": "/important" }
                    }
                }] }
            }]
        });
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(format!("data: {frame}\n\n"))
                .unwrap(),
        );
        let (tx, mut rx) = mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx, true).await;
        drop(tx);

        assert!(matches!(
            outcome,
            StreamOutcome::FailedEmpty(ProviderError::StreamTruncated(_))
        ));
        assert!(rx.recv().await.is_none());
    }

    /// A ceiling that lands after Gemini has already staged a complete function
    /// call used to fail the whole turn with an opaque parse error. It is now a
    /// resumable MaxTokens carrying a non-executable truncation fact — the same
    /// policy the OpenAI `length` and Anthropic `max_tokens` arms apply, and
    /// strictly more recoverable than an error.
    #[tokio::test]
    async fn max_tokens_with_a_staged_function_call_is_truncated_not_an_error() {
        let frame = json!({
            "candidates": [{
                "content": { "parts": [{
                    "functionCall": {
                        "id": "call_write",
                        "name": "Write",
                        "args": { "path": "/tmp/a.html" }
                    }
                }] },
                "finishReason": "MAX_TOKENS"
            }]
        });
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(200)
                .body(format!("data: {frame}\n\n"))
                .unwrap(),
        );
        let (tx, mut rx) = mpsc::channel(8);

        let outcome = process_sse_stream(response, &tx, true).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert!(matches!(outcome, StreamOutcome::Ok));
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, LlmEvent::ToolUse { .. })),
            "a call staged in a truncated response must never execute"
        );
        assert!(matches!(
            &events[0],
            LlmEvent::ToolUseTruncated { id, name, argument_bytes }
                if id == "call_write" && name == "Write" && *argument_bytes > 0
        ));
        assert!(matches!(
            events.last(),
            Some(LlmEvent::Done {
                stop_reason: StopReason::MaxTokens,
                ..
            })
        ));
    }

    #[test]
    fn streamed_429_is_rate_limited() {
        let error = google_stream_error(&json!({
            "code": 429,
            "message": "quota exhausted",
            "status": "RESOURCE_EXHAUSTED"
        }));
        assert!(matches!(
            error,
            ProviderError::RateLimited {
                retry_after_ms: 5000,
                message
            } if message == "quota exhausted"
        ));
    }
}
