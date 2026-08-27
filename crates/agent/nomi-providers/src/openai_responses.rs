use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use nomi_config::compat::{self, ProviderCompat};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use nomi_types::tool::{ToolDef, truncate_deferred_description};

use crate::anthropic_shared::StreamOutcome;
use crate::{LlmProvider, ProviderError};

const MAX_OUTPUT_ITEMS: usize = 128;
const MAX_CONTENT_PARTS: usize = 128;
const MAX_SSE_FRAME_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_CONTENT_BYTES: usize = 512 * 1024;
const MAX_ARGUMENT_BYTES: usize = 512 * 1024;
const MAX_REASONING_STATE_BYTES: usize = 512 * 1024;
const MAX_OPAQUE_ID_BYTES: usize = 128;
const MAX_TOOL_NAME_BYTES: usize = 256;
const REASONING_STATE_PREFIX: &str = "openai.responses.reasoning.v1:";

pub struct OpenAIResponsesProvider {
    api_keys: Vec<String>,
    current_api_key: AtomicUsize,
    base_url: String,
    compat: ProviderCompat,
    sanitize_tool_schemas: AtomicBool,
}

impl OpenAIResponsesProvider {
    pub fn new(api_key: &str, base_url: &str, compat: ProviderCompat) -> Self {
        Self {
            api_keys: crate::parse_api_keys(api_key),
            current_api_key: AtomicUsize::new(0),
            base_url: base_url.to_owned(),
            compat,
            sanitize_tool_schemas: AtomicBool::new(false),
        }
    }

    fn should_sanitize_tool_schemas(&self) -> bool {
        self.compat.sanitize_schema() || self.sanitize_tool_schemas.load(Ordering::Acquire)
    }

    fn build_headers(api_key: &str) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        let auth = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|error| {
            ProviderError::Connection(format!("Invalid authorization header: {error}"))
        })?;
        headers.insert(AUTHORIZATION, auth);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    fn endpoint(&self) -> String {
        let path = self.compat.api_path();
        if path.is_empty() {
            return self.base_url.clone();
        }
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    fn retain_round(&self, request: &LlmRequest) -> bool {
        self.compat.chain_rounds() && request.retain_provider_round
    }

    fn chain_parent<'a>(
        &self,
        messages: &'a [Message],
    ) -> Result<Option<(usize, &'a str)>, ProviderError> {
        let Some(index) = messages
            .iter()
            .rposition(|message| message.role == Role::Assistant)
        else {
            return Ok(None);
        };
        let Some(id) = messages[index].provider_round_id.as_deref() else {
            return Ok(None);
        };
        if index + 1 >= messages.len() || !valid_opaque_id(id) {
            return Ok(None);
        }
        // A structural suffix can still serialize to no Responses input at
        // all (for example a System-only tail or a foreign thinking wrapper).
        // Never send previous_response_id together with an empty input delta.
        if build_input(&messages[index + 1..], &self.compat, false)?.is_empty() {
            return Ok(None);
        }
        Ok(Some((index, id)))
    }

    fn build_tools(tools: &[ToolDef], sanitize: bool) -> Vec<Value> {
        tools
            .iter()
            .map(|tool| {
                let (description, parameters) = if tool.deferred {
                    (
                        truncate_deferred_description(&tool.description),
                        json!({ "type": "object", "properties": {} }),
                    )
                } else {
                    (
                        tool.description.clone(),
                        if sanitize {
                            compat::sanitize_json_schema(&tool.input_schema)
                        } else {
                            tool.input_schema.clone()
                        },
                    )
                };
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": description,
                    "parameters": parameters,
                })
            })
            .collect()
    }

    fn build_request_body(
        &self,
        request: &LlmRequest,
        sanitize_tool_schemas: bool,
        parent: Option<(usize, &str)>,
    ) -> Result<Value, ProviderError> {
        let retain = self.retain_round(request);
        let (messages, previous_response_id, full_snapshot) = match parent {
            Some((index, id)) => (&request.messages[index + 1..], Some(id), false),
            None => (request.messages.as_slice(), None, true),
        };
        let input = build_input(messages, &self.compat, full_snapshot)?;
        let max_tokens_field = self
            .compat
            .max_tokens_field
            .as_deref()
            .unwrap_or("max_output_tokens");
        let mut typed = json!({
            "model": request.model,
            "instructions": request.system,
            "input": input,
            "stream": true,
            "store": retain,
            "include": ["reasoning.encrypted_content"],
        });
        if let Some(limit) = request.max_tokens {
            typed[max_tokens_field] = json!(limit);
        }
        if !request.tools.is_empty() {
            typed["tools"] = Value::Array(Self::build_tools(
                &request.tools,
                sanitize_tool_schemas,
            ));
        }
        if let Some(effort) = &request.reasoning_effort {
            typed["reasoning"] = json!({ "effort": effort });
        }
        if let Some(id) = previous_response_id {
            typed["previous_response_id"] = json!(id);
        }

        let mut body = crate::request_body_with_extra(
            &self.compat,
            crate::OutputCeilingLocation::Top {
                dynamic: Some(max_tokens_field),
            },
            typed,
        );
        let object = body
            .as_object_mut()
            .expect("typed Responses request body is an object");
        object.remove("chain_rounds");
        if request.tools.is_empty() {
            object.remove("tools");
        }
        if request.reasoning_effort.is_none() {
            object.remove("reasoning");
        }
        if previous_response_id.is_none() {
            object.remove("previous_response_id");
        }
        Ok(body)
    }

    async fn send_initial_with_key_rotation(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &Value,
    ) -> Result<(reqwest::Response, HeaderMap), ProviderError> {
        crate::send_initial_with_key_rotation(
            client,
            url,
            body,
            &self.api_keys,
            &self.current_api_key,
            "openai.responses",
            Self::build_headers,
        )
        .await
    }
}

fn valid_opaque_id(id: &str) -> bool {
    !id.is_empty()
        && id.trim() == id
        && id.len() <= MAX_OPAQUE_ID_BYTES
        && !id.chars().any(char::is_control)
}

fn strip_patterns(text: &str, compat: &ProviderCompat) -> String {
    let mut output = text.to_owned();
    if let Some(patterns) = &compat.strip_patterns {
        for pattern in patterns {
            output = output.replace(pattern, "");
        }
    }
    output
}

fn image_content(media_type: &str, data: &str) -> Option<Value> {
    media_type.starts_with("image/").then(|| {
        json!({
            "type": "input_image",
            "image_url": format!("data:{media_type};base64,{data}"),
            "detail": "auto",
        })
    })
}

fn user_content(message: &Message, compat: &ProviderCompat) -> Vec<Value> {
    let mut content = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text { text } => {
                let text = strip_patterns(text, compat);
                if !text.is_empty() {
                    content.push(json!({ "type": "input_text", "text": text }));
                }
            }
            ContentBlock::Image { media_type, data } if compat.supports_image() => {
                if let Some(image) = image_content(media_type, data) {
                    content.push(image);
                } else {
                    content.push(json!({
                        "type": "input_text",
                        "text": format!("[unsupported attachment omitted: {media_type}]")
                    }));
                }
            }
            ContentBlock::Image { media_type, .. } => content.push(json!({
                "type": "input_text",
                "text": format!("[image omitted because this model does not support images: {media_type}]")
            })),
            ContentBlock::ToolUse { .. }
            | ContentBlock::ToolResult { .. }
            | ContentBlock::Thinking { .. } => {}
        }
    }
    content
}

fn tool_output(
    call_id: &str,
    content: &str,
    is_error: bool,
    images: &[nomi_types::tool::ToolImage],
    compat: &ProviderCompat,
) -> Value {
    let text = if is_error {
        format!("[tool error]\n{content}")
    } else {
        crate::compatibility_gateway_safe_tool_result(content)
    };
    let usable_images: Vec<Value> = if compat.supports_image() {
        images
            .iter()
            .filter_map(|image| image_content(&image.media_type, &image.data))
            .collect()
    } else {
        Vec::new()
    };
    let unsupported = images.len().saturating_sub(usable_images.len());
    let output = if usable_images.is_empty() {
        if unsupported == 0 {
            Value::String(text)
        } else {
            Value::String(format!(
                "{text}\n[{unsupported} tool attachment(s) omitted]"
            ))
        }
    } else {
        let mut parts = vec![json!({ "type": "input_text", "text": text })];
        parts.extend(usable_images);
        if unsupported > 0 {
            parts.push(json!({
                "type": "input_text",
                "text": format!("[{unsupported} unsupported tool attachment(s) omitted]")
            }));
        }
        Value::Array(parts)
    };
    json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    })
}

fn build_input(
    messages: &[Message],
    compat: &ProviderCompat,
    full_snapshot: bool,
) -> Result<Vec<Value>, ProviderError> {
    let mut input = Vec::new();
    for message in messages {
        match message.role {
            Role::System => {}
            Role::User | Role::Tool => {
                for block in &message.content {
                    if let ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                        images,
                    } = block
                    {
                        input.push(tool_output(
                            tool_use_id,
                            content,
                            *is_error,
                            images,
                            compat,
                        ));
                    }
                }
                let content = user_content(message, compat);
                if !content.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            Role::Assistant => {
                for block in &message.content {
                    if let ContentBlock::Thinking {
                        signature: Some(signature),
                        ..
                    } = block
                    {
                        input.extend(decode_reasoning_state(signature)?);
                    }
                }
                let text = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                let text = strip_patterns(&text, compat);
                if !text.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": text,
                    }));
                }
                for block in &message.content {
                    if let ContentBlock::ToolUse {
                        id, name, input: arguments, ..
                    } = block
                    {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": serde_json::to_string(arguments).map_err(|error| {
                                ProviderError::Config(format!(
                                    "could not serialize persisted tool arguments: {error}"
                                ))
                            })?,
                        }));
                    }
                }
            }
        }
    }
    if full_snapshot {
        if compat.dedup_tool_results() {
            dedup_function_outputs(&mut input);
        }
        if compat.clean_orphan_tool_calls() {
            clean_orphan_function_pairs(&mut input);
        }
    }
    Ok(input)
}

fn item_call_id(item: &Value, kind: &str) -> Option<String> {
    if item.get("type").and_then(Value::as_str) != Some(kind) {
        return None;
    }
    item.get("call_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn dedup_function_outputs(input: &mut Vec<Value>) {
    let mut last = HashMap::new();
    for (index, item) in input.iter().enumerate() {
        if let Some(call_id) = item_call_id(item, "function_call_output") {
            last.insert(call_id, index);
        }
    }
    let mut index = 0usize;
    input.retain(|item| {
        let keep = item_call_id(item, "function_call_output")
            .is_none_or(|call_id| last.get(&call_id) == Some(&index));
        index += 1;
        keep
    });
}

fn clean_orphan_function_pairs(input: &mut Vec<Value>) {
    let calls: HashSet<String> = input
        .iter()
        .filter_map(|item| item_call_id(item, "function_call"))
        .collect();
    let outputs: HashSet<String> = input
        .iter()
        .filter_map(|item| item_call_id(item, "function_call_output"))
        .collect();
    input.retain(|item| {
        if let Some(call_id) = item_call_id(item, "function_call") {
            outputs.contains(&call_id)
        } else if let Some(call_id) = item_call_id(item, "function_call_output") {
            calls.contains(&call_id)
        } else {
            true
        }
    });
}

fn encode_reasoning_state(items: &[Value]) -> Result<Option<String>, ProviderError> {
    if items.is_empty() {
        return Ok(None);
    }
    let encoded = serde_json::to_string(items).map_err(|error| {
        ProviderError::Parse(format!("could not encode Responses reasoning state: {error}"))
    })?;
    if encoded.len() > MAX_REASONING_STATE_BYTES {
        return Err(ProviderError::Parse(format!(
            "Responses reasoning state exceeded the {MAX_REASONING_STATE_BYTES}-byte safety limit"
        )));
    }
    Ok(Some(format!("{REASONING_STATE_PREFIX}{encoded}")))
}

fn decode_reasoning_state(signature: &str) -> Result<Vec<Value>, ProviderError> {
    let Some(encoded) = signature.strip_prefix(REASONING_STATE_PREFIX) else {
        return Ok(Vec::new());
    };
    if encoded.len() > MAX_REASONING_STATE_BYTES {
        return Err(ProviderError::Config(
            "persisted Responses reasoning state exceeds its safety limit".into(),
        ));
    }
    let items: Vec<Value> = serde_json::from_str(encoded).map_err(|error| {
        ProviderError::Config(format!("persisted Responses reasoning state is invalid: {error}"))
    })?;
    if items.len() > MAX_OUTPUT_ITEMS {
        return Err(ProviderError::Config(
            "persisted Responses reasoning state contains too many items".into(),
        ));
    }
    for item in &items {
        validate_reasoning_item(item).map_err(|error| ProviderError::Config(error.to_string()))?;
    }
    Ok(items)
}

fn validate_reasoning_item(item: &Value) -> Result<(), ProviderError> {
    let object = item
        .as_object()
        .ok_or_else(|| ProviderError::Parse("Responses reasoning item is not an object".into()))?;
    if object.get("type").and_then(Value::as_str) != Some("reasoning") {
        return Err(ProviderError::Parse(
            "Responses reasoning state contains a non-reasoning item".into(),
        ));
    }
    let id = required_str(object, "id", "reasoning item")?;
    if !valid_opaque_id(id) {
        return Err(ProviderError::Parse(
            "Responses reasoning item has an invalid id".into(),
        ));
    }
    let encrypted = required_str(object, "encrypted_content", "reasoning item")?;
    if encrypted.is_empty() || encrypted.len() > MAX_REASONING_STATE_BYTES {
        return Err(ProviderError::Parse(
            "Responses reasoning item has invalid encrypted_content".into(),
        ));
    }
    Ok(())
}

fn required_str<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, ProviderError> {
    object.get(field).and_then(Value::as_str).ok_or_else(|| {
        ProviderError::Parse(format!("Responses {context} is missing string field `{field}`"))
    })
}

#[async_trait]
impl LlmProvider for OpenAIResponsesProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        let client = crate::http_client()?;
        let url = self.endpoint();
        let retain = self.retain_round(request);
        let mut parent = if retain {
            self.chain_parent(&request.messages)?
        } else {
            None
        };
        let mut sanitize_tool_schemas = self.should_sanitize_tool_schemas();
        let mut learned_schema_fallback = false;
        let mut attempted = HashSet::new();

        // Each compatibility transition is monotonic. A stale parent removes
        // the cursor exactly once; an incompatible schema switches to the
        // conservative schema exactly once. Therefore at most three distinct
        // request bodies can be sent for one logical call.
        let (response, headers, body) = loop {
            if !attempted.insert((parent.is_some(), sanitize_tool_schemas)) {
                return Err(ProviderError::Config(
                    "Responses compatibility negotiation attempted the same request twice".into(),
                ));
            }
            if attempted.len() > 3 {
                return Err(ProviderError::Config(
                    "Responses compatibility negotiation exceeded its safety bound".into(),
                ));
            }

            let body = self.build_request_body(request, sanitize_tool_schemas, parent)?;
            tracing::debug!(
                target: "nomi_providers",
                provider = "openai.responses",
                chained = parent.is_some(),
                sanitize_tool_schemas,
                "sending Responses request"
            );
            match self
                .send_initial_with_key_rotation(&client, &url, &body)
                .await
            {
                Ok((response, headers)) => break (response, headers, body),
                Err(error) if parent.is_some() && error.is_stale_previous_response() => {
                    tracing::warn!(
                        target: "nomi_providers",
                        provider = "openai.responses",
                        "provider rejected a stale previous_response_id; retrying once with a full snapshot"
                    );
                    parent = None;
                }
                Err(error)
                    if !request.tools.is_empty()
                        && !sanitize_tool_schemas
                        && error.is_tool_schema_incompatible() =>
                {
                    tracing::warn!(
                        target: "nomi_providers",
                        provider = "openai.responses",
                        "provider rejected tool schemas; retrying once with conservative schema roots"
                    );
                    sanitize_tool_schemas = true;
                    learned_schema_fallback = true;
                }
                Err(ProviderError::Api {
                    status: 404,
                    message,
                }) => {
                    return Err(ProviderError::Api {
                        status: 404,
                        message: format!(
                            "{message}\nThe configured endpoint did not expose the OpenAI Responses API; expected a POST /responses endpoint."
                        ),
                    });
                }
                Err(error) => return Err(error),
            }
        };

        if learned_schema_fallback {
            self.sanitize_tool_schemas.store(true, Ordering::Release);
        }

        let (tx, rx) = mpsc::channel(64);
        let client = client.clone();
        let url_clone = url.clone();
        let redactor = nomifun_net::secret_redaction::SecretRedactor::new(&self.api_keys);
        tokio::spawn(async move {
            let outcome = process_sse_stream(response, &tx, retain).await;
            crate::retry::finish_stream_with_retry(
                outcome,
                &tx,
                || {
                    crate::retry::send_and_check(
                        &client,
                        &url_clone,
                        &headers,
                        &body,
                        &redactor,
                    )
                },
                |response| process_sse_stream(response, &tx, retain),
            )
            .await;
        });
        Ok(rx)
    }
}

#[derive(Debug)]
enum OutputItemState {
    Message {
        id: String,
        text: BTreeMap<usize, String>,
        refusal: BTreeMap<usize, String>,
    },
    Function {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        announced: bool,
        last_preview: Option<Value>,
    },
    Reasoning {
        id: String,
        summary: String,
        completed: Option<Value>,
    },
}

impl OutputItemState {
    fn id(&self) -> &str {
        match self {
            Self::Message { id, .. } | Self::Function { id, .. } | Self::Reasoning { id, .. } => id,
        }
    }
}

struct ResponsesStreamState {
    retain: bool,
    created: bool,
    response_id: Option<String>,
    store: Option<bool>,
    last_sequence: Option<u64>,
    items: BTreeMap<usize, OutputItemState>,
    done_items: HashSet<usize>,
    terminal_seen: bool,
}

impl ResponsesStreamState {
    fn new(retain: bool) -> Self {
        Self {
            retain,
            created: false,
            response_id: None,
            store: None,
            last_sequence: None,
            items: BTreeMap::new(),
            done_items: HashSet::new(),
            terminal_seen: false,
        }
    }

    fn process(&mut self, event_name: &str, payload: &Value) -> Result<Vec<LlmEvent>, ProviderError> {
        if self.terminal_seen {
            return Err(ProviderError::Parse(format!(
                "Responses emitted `{event_name}` after its terminal event"
            )));
        }
        let object = payload.as_object().ok_or_else(|| {
            ProviderError::Parse(format!("Responses `{event_name}` payload is not an object"))
        })?;
        if object.get("type").and_then(Value::as_str) != Some(event_name) {
            return Err(ProviderError::Parse(format!(
                "Responses SSE event name `{event_name}` does not match its JSON type"
            )));
        }
        self.check_sequence(object)?;

        if !self.created && event_name != "response.created" && event_name != "error" {
            return Err(ProviderError::Parse(format!(
                "Responses emitted `{event_name}` before response.created"
            )));
        }

        match event_name {
            "response.created" => self.on_created(object),
            "response.queued" | "response.in_progress" => {
                self.check_progress_response(object)?;
                Ok(Vec::new())
            }
            "response.output_item.added" => self.on_item_added(object),
            "response.output_item.done" => self.on_item_done(object),
            "response.content_part.added" | "response.content_part.done" => {
                self.on_content_part(object)?;
                Ok(Vec::new())
            }
            "response.output_text.delta" => self.on_message_delta(object, false),
            "response.refusal.delta" => self.on_message_delta(object, true),
            "response.output_text.done" => self.on_message_done(object, false),
            "response.refusal.done" => self.on_message_done(object, true),
            "response.function_call_arguments.delta" => self.on_arguments_delta(object),
            "response.function_call_arguments.done" => self.on_arguments_done(object),
            "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done" => {
                self.check_item_reference(object, "reasoning")?;
                Ok(Vec::new())
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.on_reasoning_delta(object)
            }
            "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
                self.on_reasoning_done(object)
            }
            "response.completed" | "response.incomplete" | "response.failed" => {
                self.on_terminal(event_name, object)
            }
            "error" => Err(response_error(object)),
            _ => Err(ProviderError::Parse(format!(
                "Responses emitted unsupported event `{event_name}`"
            ))),
        }
    }

    fn check_sequence(&mut self, object: &Map<String, Value>) -> Result<(), ProviderError> {
        let Some(sequence) = object.get("sequence_number") else {
            return Ok(());
        };
        let sequence = sequence.as_u64().ok_or_else(|| {
            ProviderError::Parse("Responses sequence_number is not an unsigned integer".into())
        })?;
        if self.last_sequence.is_some_and(|last| sequence <= last) {
            return Err(ProviderError::Parse(format!(
                "Responses sequence_number did not increase ({sequence})"
            )));
        }
        self.last_sequence = Some(sequence);
        Ok(())
    }

    fn on_created(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        if self.created {
            return Err(ProviderError::Parse(
                "Responses emitted response.created more than once".into(),
            ));
        }
        let response = response_object(object)?;
        self.capture_response_identity(response)?;
        let status = required_str(response, "status", "created response")?;
        if !matches!(status, "queued" | "in_progress") {
            return Err(ProviderError::Parse(format!(
                "Responses response.created has terminal status `{status}`"
            )));
        }
        self.created = true;
        Ok(Vec::new())
    }

    fn check_progress_response(&mut self, object: &Map<String, Value>) -> Result<(), ProviderError> {
        let response = response_object(object)?;
        self.match_response_identity(response)
    }

    fn capture_response_identity(&mut self, response: &Map<String, Value>) -> Result<(), ProviderError> {
        let id = required_str(response, "id", "response")?;
        if !valid_opaque_id(id) {
            return Err(ProviderError::Parse("Responses returned an invalid response id".into()));
        }
        let store = response.get("store").and_then(Value::as_bool).ok_or_else(|| {
            ProviderError::Parse("Responses response is missing boolean field `store`".into())
        })?;
        if store != self.retain {
            return Err(ProviderError::Parse(format!(
                "Responses acknowledged store={store}, but the request required store={} ",
                self.retain
            )));
        }
        self.response_id = Some(id.to_owned());
        self.store = Some(store);
        Ok(())
    }

    fn match_response_identity(&self, response: &Map<String, Value>) -> Result<(), ProviderError> {
        let id = required_str(response, "id", "response")?;
        if self.response_id.as_deref() != Some(id) {
            return Err(ProviderError::Parse(
                "Responses changed response id within one stream".into(),
            ));
        }
        let store = response.get("store").and_then(Value::as_bool).ok_or_else(|| {
            ProviderError::Parse("Responses response is missing boolean field `store`".into())
        })?;
        if self.store != Some(store) || store != self.retain {
            return Err(ProviderError::Parse(
                "Responses changed the acknowledged store policy within one stream".into(),
            ));
        }
        Ok(())
    }

    fn on_item_added(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        if self.items.contains_key(&index) {
            return Err(ProviderError::Parse(format!(
                "Responses added output item {index} more than once"
            )));
        }
        let item = object.get("item").and_then(Value::as_object).ok_or_else(|| {
            ProviderError::Parse("Responses output_item.added is missing object field `item`".into())
        })?;
        self.items.insert(index, parse_added_item(item)?);
        Ok(Vec::new())
    }

    fn on_item_done(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        if self.done_items.contains(&index) {
            return Err(ProviderError::Parse(format!(
                "Responses completed output item {index} more than once"
            )));
        }
        let item = object.get("item").and_then(Value::as_object).ok_or_else(|| {
            ProviderError::Parse("Responses output_item.done is missing object field `item`".into())
        })?;
        validate_done_item_status(item)?;
        let events = self.reconcile_item(index, item)?;
        self.done_items.insert(index);
        Ok(events)
    }

    fn on_content_part(&self, object: &Map<String, Value>) -> Result<(), ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "content part")?;
        let item = self.items.get(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses content part references unknown item {index}"))
        })?;
        if item.id() != item_id || !matches!(item, OutputItemState::Message { .. }) {
            return Err(ProviderError::Parse(
                "Responses content part does not match its message item".into(),
            ));
        }
        let _ = content_index(object)?;
        let part = object.get("part").and_then(Value::as_object).ok_or_else(|| {
            ProviderError::Parse("Responses content part is missing object field `part`".into())
        })?;
        match part.get("type").and_then(Value::as_str) {
            Some("output_text" | "refusal") => Ok(()),
            other => Err(ProviderError::Parse(format!(
                "Responses emitted unsupported message content part {other:?}"
            ))),
        }
    }

    fn on_message_delta(
        &mut self,
        object: &Map<String, Value>,
        refusal: bool,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let (index, content_index) = item_and_content_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "message delta")?;
        let delta = required_str(object, "delta", "message delta")?;
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses message delta references unknown item {index}"))
        })?;
        let OutputItemState::Message { id, text, refusal: refusals } = item else {
            return Err(ProviderError::Parse(
                "Responses message delta references a non-message item".into(),
            ));
        };
        if id != item_id {
            return Err(ProviderError::Parse(
                "Responses message delta changed its item id".into(),
            ));
        }
        let aggregate_bytes = message_content_bytes(text, refusals)?;
        if aggregate_bytes.saturating_add(delta.len()) > MAX_MESSAGE_CONTENT_BYTES {
            return Err(ProviderError::Parse(format!(
                "Responses message content exceeded the {MAX_MESSAGE_CONTENT_BYTES}-byte aggregate safety limit"
            )));
        }
        let target = if refusal { refusals } else { text };
        let accumulated = target.entry(content_index).or_default();
        accumulated.push_str(delta);
        Ok((!delta.is_empty())
            .then(|| LlmEvent::TextDelta(delta.to_owned()))
            .into_iter()
            .collect())
    }

    fn on_message_done(
        &mut self,
        object: &Map<String, Value>,
        refusal: bool,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let (index, content_index) = item_and_content_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "message done")?;
        let field = if refusal { "refusal" } else { "text" };
        let completed = required_str(object, field, "message done")?;
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses message done references unknown item {index}"))
        })?;
        let OutputItemState::Message { id, text, refusal: refusals } = item else {
            return Err(ProviderError::Parse(
                "Responses message done references a non-message item".into(),
            ));
        };
        if id != item_id {
            return Err(ProviderError::Parse(
                "Responses message done changed its item id".into(),
            ));
        }
        let aggregate_bytes = message_content_bytes(text, refusals)?;
        let target = if refusal { refusals } else { text };
        let streamed = target.entry(content_index).or_default();
        if !streamed.is_empty() && streamed != completed {
            return Err(ProviderError::Parse(
                "Responses message done did not match its streamed content".into(),
            ));
        }
        if streamed.is_empty() && !completed.is_empty() {
            if aggregate_bytes.saturating_add(completed.len()) > MAX_MESSAGE_CONTENT_BYTES {
                return Err(ProviderError::Parse(format!(
                    "Responses message content exceeded the {MAX_MESSAGE_CONTENT_BYTES}-byte aggregate safety limit"
                )));
            }
            *streamed = completed.to_owned();
            return Ok(vec![LlmEvent::TextDelta(completed.to_owned())]);
        }
        Ok(Vec::new())
    }

    fn on_arguments_delta(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "function arguments delta")?;
        let delta = required_str(object, "delta", "function arguments delta")?;
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses function delta references unknown item {index}"))
        })?;
        let OutputItemState::Function {
            id,
            call_id,
            name,
            arguments,
            announced,
            last_preview,
        } = item
        else {
            return Err(ProviderError::Parse(
                "Responses function delta references a non-function item".into(),
            ));
        };
        if id != item_id {
            return Err(ProviderError::Parse(
                "Responses function delta changed its item id".into(),
            ));
        }
        if arguments.len().saturating_add(delta.len()) > MAX_ARGUMENT_BYTES {
            return Err(ProviderError::Parse(
                "Responses function arguments exceeded their safety limit".into(),
            ));
        }
        arguments.push_str(delta);
        let preview = serde_json::from_str::<Value>(arguments)
            .ok()
            .filter(Value::is_object);
        let should_emit = !*announced || (preview.is_some() && preview != *last_preview);
        if !should_emit {
            return Ok(Vec::new());
        }
        *announced = true;
        *last_preview = preview.clone();
        Ok(vec![LlmEvent::ToolUseDelta {
            id: call_id.clone(),
            name: name.clone(),
            input: preview,
        }])
    }

    fn on_arguments_done(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "function arguments done")?;
        let completed_name = required_str(object, "name", "function arguments done")?;
        let completed = required_str(object, "arguments", "function arguments done")?;
        if completed.len() > MAX_ARGUMENT_BYTES {
            return Err(ProviderError::Parse(
                "Responses function arguments exceeded their safety limit".into(),
            ));
        }
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses function done references unknown item {index}"))
        })?;
        let OutputItemState::Function {
            id,
            call_id,
            name,
            arguments,
            announced,
            last_preview,
        } = item
        else {
            return Err(ProviderError::Parse(
                "Responses function done references a non-function item".into(),
            ));
        };
        if id != item_id
            || name != completed_name
            || (!arguments.is_empty() && arguments != completed)
        {
            return Err(ProviderError::Parse(
                "Responses function done did not match its streamed item".into(),
            ));
        }
        if arguments.is_empty() {
            *arguments = completed.to_owned();
        }
        let preview = serde_json::from_str::<Value>(completed)
            .ok()
            .filter(Value::is_object);
        if !*announced || (preview.is_some() && preview != *last_preview) {
            *announced = true;
            *last_preview = preview.clone();
            return Ok(vec![LlmEvent::ToolUseDelta {
                id: call_id.clone(),
                name: name.clone(),
                input: preview,
            }]);
        }
        Ok(Vec::new())
    }

    fn on_reasoning_delta(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "reasoning delta")?;
        let delta = required_str(object, "delta", "reasoning delta")?;
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses reasoning delta references unknown item {index}"))
        })?;
        let OutputItemState::Reasoning { id, summary, .. } = item else {
            return Err(ProviderError::Parse(
                "Responses reasoning delta references a non-reasoning item".into(),
            ));
        };
        if id != item_id || summary.len().saturating_add(delta.len()) > MAX_REASONING_STATE_BYTES {
            return Err(ProviderError::Parse(
                "Responses reasoning delta is inconsistent or too large".into(),
            ));
        }
        summary.push_str(delta);
        Ok((!delta.is_empty())
            .then(|| LlmEvent::ThinkingDelta(delta.to_owned()))
            .into_iter()
            .collect())
    }

    fn on_reasoning_done(&mut self, object: &Map<String, Value>) -> Result<Vec<LlmEvent>, ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "reasoning done")?;
        let completed = required_str(object, "text", "reasoning done")?;
        let item = self.items.get_mut(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses reasoning done references unknown item {index}"))
        })?;
        let OutputItemState::Reasoning { id, summary, .. } = item else {
            return Err(ProviderError::Parse(
                "Responses reasoning done references a non-reasoning item".into(),
            ));
        };
        if id != item_id || (!summary.is_empty() && summary != completed) {
            return Err(ProviderError::Parse(
                "Responses reasoning done did not match its streamed content".into(),
            ));
        }
        if summary.is_empty() && !completed.is_empty() {
            *summary = completed.to_owned();
            return Ok(vec![LlmEvent::ThinkingDelta(completed.to_owned())]);
        }
        Ok(Vec::new())
    }

    fn check_item_reference(
        &self,
        object: &Map<String, Value>,
        expected: &str,
    ) -> Result<(), ProviderError> {
        let index = output_index(object)?;
        self.ensure_item_open(index)?;
        let item_id = required_str(object, "item_id", "item reference")?;
        let item = self.items.get(&index).ok_or_else(|| {
            ProviderError::Parse(format!("Responses event references unknown item {index}"))
        })?;
        let matches_kind = matches!((expected, item), ("reasoning", OutputItemState::Reasoning { .. }));
        if !matches_kind || item.id() != item_id {
            return Err(ProviderError::Parse(
                "Responses event does not match its output item".into(),
            ));
        }
        Ok(())
    }

    fn ensure_item_open(&self, index: usize) -> Result<(), ProviderError> {
        if self.done_items.contains(&index) {
            return Err(ProviderError::Parse(format!(
                "Responses emitted another item event after output item {index} was done"
            )));
        }
        Ok(())
    }

    fn reconcile_item(
        &mut self,
        index: usize,
        item: &Map<String, Value>,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let item_type = required_str(item, "type", "output item")?;
        let id = required_str(item, "id", "output item")?;
        if !valid_opaque_id(id) {
            return Err(ProviderError::Parse("Responses output item has an invalid id".into()));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = self.items.entry(index) {
            entry.insert(parse_added_item(item)?);
        }
        let state = self.items.get_mut(&index).expect("item was inserted");
        if state.id() != id {
            return Err(ProviderError::Parse(
                "Responses changed an output item id".into(),
            ));
        }
        match (item_type, state) {
            ("message", OutputItemState::Message { text, refusal, .. }) => {
                let content = item.get("content").and_then(Value::as_array).ok_or_else(|| {
                    ProviderError::Parse("Responses message item is missing content".into())
                })?;
                if content.len() > MAX_CONTENT_PARTS {
                    return Err(ProviderError::Parse(format!(
                        "Responses message contains more than {MAX_CONTENT_PARTS} content parts"
                    )));
                }
                let mut events = Vec::new();
                let mut terminal_text = HashSet::new();
                let mut terminal_refusal = HashSet::new();
                let mut terminal_bytes = 0usize;
                for (content_index, part) in content.iter().enumerate() {
                    let part = part.as_object().ok_or_else(|| {
                        ProviderError::Parse("Responses message content is not an object".into())
                    })?;
                    let (target, field) = match part.get("type").and_then(Value::as_str) {
                        Some("output_text") => {
                            terminal_text.insert(content_index);
                            (&mut *text, "text")
                        }
                        Some("refusal") => {
                            terminal_refusal.insert(content_index);
                            (&mut *refusal, "refusal")
                        }
                        other => {
                            return Err(ProviderError::Parse(format!(
                                "Responses emitted unsupported message content {other:?}"
                            )));
                        }
                    };
                    let completed = required_str(part, field, "message content")?;
                    terminal_bytes = terminal_bytes.checked_add(completed.len()).ok_or_else(|| {
                        ProviderError::Parse(
                            "Responses terminal message content byte count overflowed".into(),
                        )
                    })?;
                    if terminal_bytes > MAX_MESSAGE_CONTENT_BYTES {
                        return Err(ProviderError::Parse(format!(
                            "Responses terminal message exceeded the {MAX_MESSAGE_CONTENT_BYTES}-byte aggregate safety limit"
                        )));
                    }
                    let streamed = target.entry(content_index).or_default();
                    if !streamed.is_empty() && streamed != completed {
                        return Err(ProviderError::Parse(
                            "Responses terminal message did not match streamed content".into(),
                        ));
                    }
                    if streamed.is_empty() && !completed.is_empty() {
                        *streamed = completed.to_owned();
                        events.push(LlmEvent::TextDelta(completed.to_owned()));
                    }
                }
                if text.keys().any(|index| !terminal_text.contains(index))
                    || refusal
                        .keys()
                        .any(|index| !terminal_refusal.contains(index))
                {
                    return Err(ProviderError::Parse(
                        "Responses terminal message omitted streamed content".into(),
                    ));
                }
                Ok(events)
            }
            (
                "function_call",
                OutputItemState::Function {
                    call_id,
                    name,
                    arguments,
                    announced,
                    last_preview,
                    ..
                },
            ) => {
                let terminal_call_id = required_str(item, "call_id", "function call")?;
                let terminal_name = required_str(item, "name", "function call")?;
                let terminal_arguments = required_str(item, "arguments", "function call")?;
                validate_call_identity(terminal_call_id, terminal_name)?;
                if call_id != terminal_call_id
                    || name != terminal_name
                    || (!arguments.is_empty() && arguments != terminal_arguments)
                {
                    return Err(ProviderError::Parse(
                        "Responses terminal function call did not match streamed arguments".into(),
                    ));
                }
                if terminal_arguments.len() > MAX_ARGUMENT_BYTES {
                    return Err(ProviderError::Parse(
                        "Responses function arguments exceeded their safety limit".into(),
                    ));
                }
                if arguments.is_empty() {
                    *arguments = terminal_arguments.to_owned();
                }
                if !*announced {
                    *announced = true;
                    let preview = serde_json::from_str::<Value>(arguments)
                        .ok()
                        .filter(Value::is_object);
                    *last_preview = preview.clone();
                    return Ok(vec![LlmEvent::ToolUseDelta {
                        id: call_id.clone(),
                        name: name.clone(),
                        input: preview,
                    }]);
                }
                Ok(Vec::new())
            }
            ("reasoning", OutputItemState::Reasoning { completed, .. }) => {
                let value = Value::Object(item.clone());
                validate_reasoning_item(&value)?;
                if completed.as_ref().is_some_and(|existing| existing != &value) {
                    return Err(ProviderError::Parse(
                        "Responses terminal reasoning item did not match output_item.done".into(),
                    ));
                }
                *completed = Some(value);
                Ok(Vec::new())
            }
            _ => Err(ProviderError::Parse(
                "Responses output item changed type within one stream".into(),
            )),
        }
    }

    fn on_terminal(
        &mut self,
        event_name: &str,
        object: &Map<String, Value>,
    ) -> Result<Vec<LlmEvent>, ProviderError> {
        let response = response_object(object)?;
        self.match_response_identity(response)?;
        let status = required_str(response, "status", "terminal response")?;
        let expected = match event_name {
            "response.completed" => "completed",
            "response.incomplete" => "incomplete",
            "response.failed" => "failed",
            _ => unreachable!(),
        };
        if status != expected {
            return Err(ProviderError::Parse(format!(
                "Responses terminal event `{event_name}` carried status `{status}`"
            )));
        }
        let output = response.get("output").and_then(Value::as_array).ok_or_else(|| {
            ProviderError::Parse("Responses terminal response is missing output".into())
        })?;
        if output.len() > MAX_OUTPUT_ITEMS {
            return Err(ProviderError::Parse(
                "Responses terminal response contains too many output items".into(),
            ));
        }
        let mut events = Vec::new();
        for (index, item) in output.iter().enumerate() {
            let item = item.as_object().ok_or_else(|| {
                ProviderError::Parse("Responses terminal output item is not an object".into())
            })?;
            if event_name == "response.completed" {
                validate_completed_item_status(item)?;
            }
            events.extend(self.reconcile_item(index, item)?);
        }
        if self.items.len() != output.len() {
            return Err(ProviderError::Parse(
                "Responses terminal output did not contain every streamed item".into(),
            ));
        }
        let usage = parse_usage(response)?;
        let has_refusal = self.items.values().any(|item| match item {
            OutputItemState::Message { refusal, .. } => refusal.values().any(|text| !text.is_empty()),
            _ => false,
        });
        let functions: Vec<(String, String, String)> = self
            .items
            .values()
            .filter_map(|item| match item {
                OutputItemState::Function {
                    call_id,
                    name,
                    arguments,
                    ..
                } => Some((call_id.clone(), name.clone(), arguments.clone())),
                _ => None,
            })
            .collect();
        let reasoning: Vec<Value> = self
            .items
            .values()
            .filter_map(|item| match item {
                OutputItemState::Reasoning { completed, .. } => completed.clone(),
                _ => None,
            })
            .collect();

        let incomplete_reason = response
            .get("incomplete_details")
            .and_then(Value::as_object)
            .and_then(|details| details.get("reason"))
            .and_then(Value::as_str);

        match event_name {
            "response.completed" if has_refusal => {
                events.push(LlmEvent::Done {
                    stop_reason: StopReason::Refusal,
                    usage,
                });
            }
            "response.completed" => {
                if let Some(signature) = encode_reasoning_state(&reasoning)? {
                    events.push(LlmEvent::ThinkingSignature(signature));
                }
                for (call_id, name, arguments) in &functions {
                    let input = crate::parse_tool_call_arguments(
                        "OpenAI Responses",
                        name,
                        call_id,
                        arguments,
                    )
                    .map_err(ProviderError::Parse)?;
                    events.push(LlmEvent::ToolUse {
                        id: call_id.clone(),
                        name: name.clone(),
                        input,
                        extra: None,
                    });
                }
                if self.retain {
                    events.push(LlmEvent::ProviderRoundId(
                        self.response_id.clone().expect("created response has an id"),
                    ));
                }
                events.push(LlmEvent::Done {
                    stop_reason: if functions.is_empty() {
                        StopReason::EndTurn
                    } else {
                        StopReason::ToolUse
                    },
                    usage,
                });
            }
            "response.incomplete" if incomplete_reason == Some("content_filter") || has_refusal => {
                events.push(LlmEvent::Done {
                    stop_reason: StopReason::Refusal,
                    usage,
                });
            }
            "response.incomplete" if incomplete_reason == Some("max_output_tokens") => {
                for (call_id, name, arguments) in &functions {
                    events.push(LlmEvent::ToolUseTruncated {
                        id: call_id.clone(),
                        name: name.clone(),
                        argument_bytes: arguments.len(),
                    });
                }
                if functions.is_empty() && self.retain {
                    events.push(LlmEvent::ProviderRoundId(
                        self.response_id.clone().expect("created response has an id"),
                    ));
                }
                events.push(LlmEvent::Done {
                    stop_reason: StopReason::MaxTokens,
                    usage,
                });
            }
            "response.incomplete" => {
                return Err(ProviderError::Api {
                    status: 422,
                    message: format!(
                        "Responses ended incomplete for unsupported reason `{}`",
                        incomplete_reason.unwrap_or("missing")
                    ),
                });
            }
            "response.failed" => {
                return Err(terminal_response_error(response));
            }
            _ => unreachable!(),
        }
        self.terminal_seen = true;
        Ok(events)
    }
}

fn parse_added_item(item: &Map<String, Value>) -> Result<OutputItemState, ProviderError> {
    let item_type = required_str(item, "type", "output item")?;
    let id = required_str(item, "id", "output item")?;
    if !valid_opaque_id(id) {
        return Err(ProviderError::Parse("Responses output item has an invalid id".into()));
    }
    match item_type {
        "message" => {
            if item.get("role").and_then(Value::as_str) != Some("assistant") {
                return Err(ProviderError::Parse(
                    "Responses output message has a non-assistant role".into(),
                ));
            }
            Ok(OutputItemState::Message {
                id: id.to_owned(),
                text: BTreeMap::new(),
                refusal: BTreeMap::new(),
            })
        }
        "function_call" => {
            let call_id = required_str(item, "call_id", "function call")?;
            let name = required_str(item, "name", "function call")?;
            validate_call_identity(call_id, name)?;
            let arguments = item
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if arguments.len() > MAX_ARGUMENT_BYTES {
                return Err(ProviderError::Parse(
                    "Responses function arguments exceeded their safety limit".into(),
                ));
            }
            Ok(OutputItemState::Function {
                id: id.to_owned(),
                call_id: call_id.to_owned(),
                name: name.to_owned(),
                arguments: arguments.to_owned(),
                announced: false,
                last_preview: None,
            })
        }
        "reasoning" => Ok(OutputItemState::Reasoning {
            id: id.to_owned(),
            summary: String::new(),
            completed: None,
        }),
        other => Err(ProviderError::Parse(format!(
            "Responses emitted unsupported output item type `{other}`"
        ))),
    }
}

fn validate_done_item_status(item: &Map<String, Value>) -> Result<(), ProviderError> {
    let item_type = required_str(item, "type", "done output item")?;
    match item.get("status") {
        Some(Value::String(status)) if matches!(status.as_str(), "completed" | "incomplete") => {
            Ok(())
        }
        // Reasoning items do not consistently carry a status. Their opaque
        // encrypted payload is checked against the terminal response instead.
        None if item_type == "reasoning" => Ok(()),
        Some(Value::String(status)) => Err(ProviderError::Parse(format!(
            "Responses output_item.done contains `{item_type}` item with status `{status}`"
        ))),
        Some(_) => Err(ProviderError::Parse(format!(
            "Responses `{item_type}` item has a non-string status"
        ))),
        None => Err(ProviderError::Parse(format!(
            "Responses output_item.done contains `{item_type}` item without status"
        ))),
    }
}

fn validate_completed_item_status(item: &Map<String, Value>) -> Result<(), ProviderError> {
    let item_type = required_str(item, "type", "completed output item")?;
    match item.get("status") {
        Some(Value::String(status)) if status == "completed" => Ok(()),
        // Reasoning items in the current Responses wire do not consistently
        // carry a status. Their encrypted payload and terminal equality are
        // validated separately; a present non-completed status is still fatal.
        None if item_type == "reasoning" => Ok(()),
        Some(Value::String(status)) => Err(ProviderError::Parse(format!(
            "Responses completed round contains `{item_type}` item with status `{status}`"
        ))),
        Some(_) => Err(ProviderError::Parse(format!(
            "Responses `{item_type}` item has a non-string status"
        ))),
        None => Err(ProviderError::Parse(format!(
            "Responses completed round contains `{item_type}` item without status"
        ))),
    }
}

fn validate_call_identity(call_id: &str, name: &str) -> Result<(), ProviderError> {
    if !valid_opaque_id(call_id) || name.is_empty() || name.len() > MAX_TOOL_NAME_BYTES {
        return Err(ProviderError::Parse(
            "Responses function call has an invalid call id or name".into(),
        ));
    }
    Ok(())
}

fn response_object(object: &Map<String, Value>) -> Result<&Map<String, Value>, ProviderError> {
    object.get("response").and_then(Value::as_object).ok_or_else(|| {
        ProviderError::Parse("Responses event is missing object field `response`".into())
    })
}

fn output_index(object: &Map<String, Value>) -> Result<usize, ProviderError> {
    let value = object.get("output_index").and_then(Value::as_u64).ok_or_else(|| {
        ProviderError::Parse("Responses event is missing unsigned `output_index`".into())
    })?;
    let index = usize::try_from(value).map_err(|_| {
        ProviderError::Parse("Responses output_index does not fit this platform".into())
    })?;
    if index >= MAX_OUTPUT_ITEMS {
        return Err(ProviderError::Parse(format!(
            "Responses output_index exceeds the {MAX_OUTPUT_ITEMS}-item safety limit"
        )));
    }
    Ok(index)
}

fn content_index(object: &Map<String, Value>) -> Result<usize, ProviderError> {
    let value = object.get("content_index").and_then(Value::as_u64).ok_or_else(|| {
        ProviderError::Parse("Responses event is missing unsigned `content_index`".into())
    })?;
    let index = usize::try_from(value).map_err(|_| {
        ProviderError::Parse("Responses content_index does not fit this platform".into())
    })?;
    if index >= MAX_CONTENT_PARTS {
        return Err(ProviderError::Parse(format!(
            "Responses content_index exceeds the {MAX_CONTENT_PARTS}-part safety limit"
        )));
    }
    Ok(index)
}

fn message_content_bytes(
    text: &BTreeMap<usize, String>,
    refusal: &BTreeMap<usize, String>,
) -> Result<usize, ProviderError> {
    text.values()
        .chain(refusal.values())
        .try_fold(0usize, |total, content| {
            total.checked_add(content.len()).ok_or_else(|| {
                ProviderError::Parse("Responses message content byte count overflowed".into())
            })
        })
}

fn item_and_content_index(object: &Map<String, Value>) -> Result<(usize, usize), ProviderError> {
    Ok((output_index(object)?, content_index(object)?))
}

fn parse_usage(response: &Map<String, Value>) -> Result<TokenUsage, ProviderError> {
    let Some(usage) = response.get("usage") else {
        return Ok(TokenUsage::default());
    };
    if usage.is_null() {
        return Ok(TokenUsage::default());
    }
    let usage = usage.as_object().ok_or_else(|| {
        ProviderError::Parse("Responses usage is not an object".into())
    })?;
    let input_tokens = optional_u64(usage, "input_tokens")?;
    let output_tokens = optional_u64(usage, "output_tokens")?;
    let cache_read_tokens = usage
        .get("input_tokens_details")
        .and_then(Value::as_object)
        .map(|details| optional_u64(details, "cached_tokens"))
        .transpose()?
        .unwrap_or_default();
    let reasoning_tokens = usage
        .get("output_tokens_details")
        .and_then(Value::as_object)
        .map(|details| optional_u64(details, "reasoning_tokens"))
        .transpose()?
        .unwrap_or_default();
    Ok(TokenUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_creation_tokens: 0,
        cache_read_tokens,
    })
}

fn optional_u64(object: &Map<String, Value>, field: &str) -> Result<u64, ProviderError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value.as_u64().ok_or_else(|| {
            ProviderError::Parse(format!("Responses usage field `{field}` is not unsigned"))
        }),
    }
}

fn response_error(object: &Map<String, Value>) -> ProviderError {
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Responses stream reported an error");
    let code = object
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    ProviderError::Api {
        status: if code.contains("server") { 500 } else { 422 },
        message: format!("{message} (code: {code})"),
    }
}

fn terminal_response_error(response: &Map<String, Value>) -> ProviderError {
    let error = response.get("error").and_then(Value::as_object);
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Responses request failed");
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    ProviderError::Api {
        status: if code.contains("server") { 500 } else { 422 },
        message: format!("{message} (code: {code})"),
    }
}

fn find_sse_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let mut best = None;
    for delimiter in [&b"\r\n\r\n"[..], &b"\n\n"[..], &b"\r\r"[..]] {
        if let Some(index) = buffer.windows(delimiter.len()).position(|window| window == delimiter)
            && best.is_none_or(|(current, _)| index < current)
        {
            best = Some((index, delimiter.len()));
        }
    }
    best
}

fn parse_sse_frame(frame: &[u8]) -> Result<(String, Value), ProviderError> {
    let frame = std::str::from_utf8(frame).map_err(|error| {
        ProviderError::Parse(format!("Responses SSE frame is not valid UTF-8: {error}"))
    })?;
    let normalized = frame.replace("\r\n", "\n").replace('\r', "\n");
    let mut event_name = None;
    let mut data = Vec::new();
    for line in normalized.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => {
                if event_name.replace(value.to_owned()).is_some() {
                    return Err(ProviderError::Parse(
                        "Responses SSE frame contains more than one event field".into(),
                    ));
                }
            }
            "data" => data.push(value),
            "id" | "retry" => {}
            other => {
                return Err(ProviderError::Parse(format!(
                    "Responses SSE frame contains unsupported field `{other}`"
                )));
            }
        }
    }
    let event_name = event_name.ok_or_else(|| {
        ProviderError::Parse("Responses SSE frame is missing its named event".into())
    })?;
    if data.is_empty() {
        return Err(ProviderError::Parse(
            "Responses SSE frame is missing its data payload".into(),
        ));
    }
    let data = data.join("\n");
    if data == "[DONE]" {
        return Err(ProviderError::Parse(
            "Responses used a Chat Completions [DONE] sentinel instead of a terminal Responses event"
                .into(),
        ));
    }
    let payload = serde_json::from_str(&data).map_err(|error| {
        ProviderError::Parse(format!("Responses SSE data is invalid JSON: {error}"))
    })?;
    Ok((event_name, payload))
}

async fn process_sse_stream(
    response: reqwest::Response,
    tx: &mpsc::Sender<LlmEvent>,
    retain: bool,
) -> StreamOutcome {
    use futures::StreamExt;

    let mut state = ResponsesStreamState::new(retain);
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut emitted_content = false;
    let mut saw_frame = false;
    // Keep terminal-derived events private until the body reaches a clean
    // EOF. A trailing poison frame must not be able to follow an already
    // committed Done/cursor pair.
    let mut pending_terminal_events: Option<Vec<LlmEvent>> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                return stream_failure(ProviderError::from(error), emitted_content);
            }
        };
        buffer.extend_from_slice(&chunk);
        if buffer.len() > MAX_SSE_FRAME_BYTES && find_sse_boundary(&buffer).is_none() {
            return stream_failure(
                ProviderError::Parse(format!(
                    "Responses SSE frame exceeded the {MAX_SSE_FRAME_BYTES}-byte safety limit"
                )),
                emitted_content,
            );
        }

        while let Some((boundary, delimiter_len)) = find_sse_boundary(&buffer) {
            if boundary > MAX_SSE_FRAME_BYTES {
                return stream_failure(
                    ProviderError::Parse(format!(
                        "Responses SSE frame exceeded the {MAX_SSE_FRAME_BYTES}-byte safety limit"
                    )),
                    emitted_content,
                );
            }
            let frame = buffer[..boundary].to_vec();
            buffer.drain(..boundary + delimiter_len);
            if frame.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            saw_frame = true;
            let (event_name, payload) = match parse_sse_frame(&frame) {
                Ok(parsed) => parsed,
                Err(error) => return stream_failure(error, emitted_content),
            };
            let events = match state.process(&event_name, &payload) {
                Ok(events) => events,
                Err(error) => return stream_failure(error, emitted_content),
            };
            if state.terminal_seen {
                pending_terminal_events = Some(events);
                continue;
            }
            for event in events {
                emitted_content |= matches!(
                    event,
                    LlmEvent::TextDelta(_)
                        | LlmEvent::ThinkingDelta(_)
                        | LlmEvent::ToolUseDelta { .. }
                        | LlmEvent::ToolUse { .. }
                        | LlmEvent::ToolUseTruncated { .. }
                );
                if tx.send(event).await.is_err() {
                    return StreamOutcome::Ok;
                }
            }
        }
        if buffer.len() > MAX_SSE_FRAME_BYTES {
            return stream_failure(
                ProviderError::Parse(format!(
                    "Responses SSE frame exceeded the {MAX_SSE_FRAME_BYTES}-byte safety limit"
                )),
                emitted_content,
            );
        }
    }

    if !buffer.iter().all(u8::is_ascii_whitespace) {
        return stream_failure(
            ProviderError::Parse(
                "Responses stream ended in the middle of an SSE frame".into(),
            ),
            emitted_content,
        );
    }
    if let Some(events) = pending_terminal_events {
        for event in events {
            if tx.send(event).await.is_err() {
                break;
            }
        }
        return StreamOutcome::Ok;
    }

    let error = if saw_frame {
        ProviderError::StreamTruncated(
            "Responses stream ended before a terminal response event".into(),
        )
    } else {
        ProviderError::Parse(
            "Responses endpoint returned a non-streaming body; named SSE events were required".into(),
        )
    };
    stream_failure(error, emitted_content)
}

fn stream_failure(error: ProviderError, emitted_content: bool) -> StreamOutcome {
    if emitted_content {
        StreamOutcome::FailedPartial(error)
    } else {
        StreamOutcome::FailedEmpty(error)
    }
}
