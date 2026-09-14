//! Bounded, attempt-local native Responses lifecycle decoding. Item identity is
//! not tool-call identity. Completed snapshots corroborate streamed data; they
//! never authorize execution of an incomplete or differently named call.
use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::StrictJsonValue;
use serde_json::Value;

use crate::adapter::{ProviderWireFrame, parse_usage};
use crate::contracts::{
    ChatFinishReason, ChatModelError, ChatModelEvent, ChatProtocol, ChatToolCall,
    ProviderResponseId, ToolCallId,
};

const MAX_ITEMS: usize = 128;
const MAX_PARTS: usize = 128;
const MAX_ARGUMENTS: usize = 256 * 1024;

pub(crate) struct ResponsesDecoder {
    native: Option<bool>,
    preserve: bool,
    response_id: Option<String>,
    sequence: Option<u64>,
    items: Vec<Item>,
    calls: BTreeSet<String>,
    budget: crate::wire_budget::WireBudget,
    terminal: bool,
    refusal: bool,
    incomplete_item: bool,
}

struct Item {
    id: String,
    kind: String,
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    arguments_done: bool,
    parts: BTreeMap<(String, usize), String>,
    finished_parts: BTreeSet<(String, usize)>,
    added_parts: BTreeSet<(String, usize)>,
    closed_parts: BTreeSet<(String, usize)>,
    snapshot: Option<Value>,
}

impl ResponsesDecoder {
    pub(crate) fn new(preserve: bool) -> Self {
        Self {
            native: None,
            preserve,
            response_id: None,
            sequence: None,
            items: Vec::new(),
            calls: BTreeSet::new(),
            budget: crate::wire_budget::WireBudget::default(),
            terminal: false,
            refusal: false,
            incomplete_item: false,
        }
    }

    pub(crate) fn decode(
        &mut self,
        frame: &ProviderWireFrame,
    ) -> Option<Result<Vec<ChatModelEvent>, ChatModelError>> {
        let named = frame.event.trim().to_ascii_lowercase();
        if frame
            .data
            .get("type")
            .is_some_and(|value| !value.is_string())
        {
            return Some(Err(invalid("Invalid Responses event type")));
        }
        let declared = frame.data.get("type").and_then(Value::as_str);
        let native_frame = declared.is_some_and(|kind| kind.starts_with("response."))
            || frame.data.get("response").is_some()
            || frame.data.get("output_index").is_some();
        if self.native == Some(false) && native_frame {
            return Some(Err(invalid(
                "Responses stream mixed native and normalized events",
            )));
        }
        let native = *self.native.get_or_insert(native_frame);
        if !native {
            return None;
        }
        let kind = if matches!(named.as_str(), "message" | "json") {
            declared.unwrap_or(&named)
        } else {
            if declared.is_some_and(|kind| kind != named) {
                return Some(Err(invalid(
                    "Responses event name contradicts its payload type",
                )));
            }
            &named
        };
        Some(self.decode_native(kind, &frame.data))
    }

    fn decode_native(
        &mut self,
        kind: &str,
        data: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        self.budget.admit(data)?;
        if self.terminal {
            return Err(invalid("Responses event after terminal"));
        }
        if let Some(error) =
            crate::provider_errors::decode(ChatProtocol::OpenaiResponses, kind, data)
        {
            return Err(error);
        }
        if let Some(number) = data.get("sequence_number") {
            let number = number
                .as_u64()
                .ok_or_else(|| invalid("Invalid Responses sequence number"))?;
            if self.sequence.is_some_and(|previous| number <= previous) {
                return Err(invalid("Responses sequence did not advance"));
            }
            self.sequence = Some(number);
        }
        if kind == "response.created" {
            if self.response_id.is_some() {
                return Err(invalid("Duplicate Responses creation"));
            }
            let response = field(data, "response")?;
            let id = identity(response, "id")?.to_owned();
            self.response_id = Some(id.clone());
            return Ok(vec![ChatModelEvent::ResponseStarted {
                provider_response_id: Some(ProviderResponseId(id)),
            }]);
        }
        if self.response_id.is_none() {
            return Err(invalid("Responses event before creation"));
        }
        match kind {
            "response.in_progress" | "response.queued" => {
                self.check_response(field(data, "response")?)?;
                Ok(Vec::new())
            }
            "response.output_item.added" => self.add_item(data),
            "response.output_item.done" => self.finish_item(data),
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                let item = self.open_item(data)?;
                if item.kind != "function_call" || item.arguments_done {
                    return Err(invalid("Arguments event outside an open function call"));
                }
                let done = kind.ends_with(".done");
                let value = string(data, if done { "arguments" } else { "delta" })?;
                let delta = if done {
                    value
                        .strip_prefix(item.arguments.as_str())
                        .ok_or_else(|| invalid("Final function arguments contradict deltas"))?
                } else {
                    value
                };
                if item.arguments.len().saturating_add(delta.len()) > MAX_ARGUMENTS {
                    return Err(invalid("Function arguments limit exceeded"));
                }
                let delta = delta.to_owned();
                item.arguments.push_str(&delta);
                item.arguments_done = done;
                Ok(if delta.is_empty() {
                    Vec::new()
                } else {
                    vec![item.tool_delta(delta)?]
                })
            }
            "response.output_text.delta"
            | "response.output_text.done"
            | "response.refusal.delta"
            | "response.refusal.done"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_summary_text.done" => self.text_event(kind, data),
            "response.content_part.added"
            | "response.content_part.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done" => {
                let reasoning = kind.starts_with("response.reasoning_summary");
                let item = self.open_item(data)?;
                let index = index(
                    data,
                    if reasoning {
                        "summary_index"
                    } else {
                        "content_index"
                    },
                )?;
                if index >= MAX_PARTS
                    || item.kind != if reasoning { "reasoning" } else { "message" }
                {
                    return Err(invalid("Invalid Responses content part"));
                }
                let part = field(data, "part")?;
                let (part_kind, text) = part_text(part, reasoning)?;
                let key = (part_kind.to_owned(), index);
                if kind.ends_with(".done") {
                    if !item.closed_parts.insert(key.clone())
                        || item.parts.get(&key).map(String::as_str).unwrap_or("") != text
                    {
                        return Err(invalid("Completed content part contradicts streamed text"));
                    }
                } else {
                    if !text.is_empty()
                        || item.parts.contains_key(&key)
                        || item.closed_parts.contains(&key)
                        || !item.added_parts.insert(key.clone())
                    {
                        return Err(invalid(
                            "Invalid or duplicate initial Responses content part",
                        ));
                    }
                }
                Ok(Vec::new())
            }
            "response.completed" | "response.incomplete" => self.finish_response(kind, data),
            _ => Err(invalid("Unsupported native Responses event")),
        }
    }

    fn check_response(&self, response: &Value) -> Result<(), ChatModelError> {
        if Some(identity(response, "id")?) != self.response_id.as_deref() {
            return Err(invalid("Responses identity changed"));
        }
        Ok(())
    }

    fn add_item(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if self.items.len() >= MAX_ITEMS || index(data, "output_index")? != self.items.len() {
            return Err(invalid(
                "Responses item index is duplicate, missing or excessive",
            ));
        }
        let value = field(data, "item")?;
        let id = identity(value, "id")?.to_owned();
        let kind = identity(value, "type")?.to_owned();
        if self.items.iter().any(|item| item.id == id) {
            return Err(invalid("Duplicate Responses item identity"));
        }
        if !matches!(kind.as_str(), "message" | "function_call" | "reasoning") && !self.preserve {
            return Err(invalid(
                "Native provider-hosted item is not admitted by this consumer",
            ));
        }
        let mut item = Item {
            id,
            kind,
            call_id: None,
            name: None,
            arguments: String::new(),
            arguments_done: false,
            parts: BTreeMap::new(),
            finished_parts: BTreeSet::new(),
            added_parts: BTreeSet::new(),
            closed_parts: BTreeSet::new(),
            snapshot: None,
        };
        let mut events = Vec::new();
        if item.kind == "function_call" {
            let call_id = identity(value, "call_id")?.to_owned();
            if self.calls.len() >= 64 || !self.calls.insert(call_id.clone()) {
                return Err(invalid("Duplicate or excessive Responses tool call"));
            }
            item.call_id = Some(call_id);
            item.name = Some(identity(value, "name")?.to_owned());
            let args = string(value, "arguments")?;
            if args.len() > MAX_ARGUMENTS {
                return Err(invalid("Function arguments limit exceeded"));
            }
            item.arguments = args.to_owned();
            events.push(item.tool_delta(args.to_owned())?);
        } else if item.kind == "message" {
            if string(value, "role")? != "assistant" || !array(value, "content")?.is_empty() {
                return Err(invalid("Invalid initial Responses assistant message"));
            }
        } else if item.kind == "reasoning" && !array(value, "summary")?.is_empty() {
            return Err(invalid("Invalid initial Responses reasoning summary"));
        }
        self.items.push(item);
        Ok(events)
    }

    fn open_item(&mut self, data: &Value) -> Result<&mut Item, ChatModelError> {
        let index = index(data, "output_index")?;
        let item = self
            .items
            .get_mut(index)
            .ok_or_else(|| invalid("Unknown Responses output index"))?;
        if item.snapshot.is_some() || identity(data, "item_id")? != item.id {
            return Err(invalid(
                "Responses event targets a closed or different item",
            ));
        }
        for (key, expected) in [
            ("call_id", item.call_id.as_deref()),
            ("name", item.name.as_deref()),
        ] {
            if data.get(key).is_some() && Some(identity(data, key)?) != expected {
                return Err(invalid("Responses event contradicts its function identity"));
            }
        }
        Ok(item)
    }

    fn text_event(
        &mut self,
        kind: &str,
        data: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let reasoning = kind.starts_with("response.reasoning_summary_text");
        let refusal = kind.starts_with("response.refusal");
        let part_kind = if reasoning {
            "summary_text"
        } else if refusal {
            "refusal"
        } else {
            "output_text"
        };
        let item = self.open_item(data)?;
        if item.kind != if reasoning { "reasoning" } else { "message" } {
            return Err(invalid("Text event has incompatible item type"));
        }
        let part_index = index(
            data,
            if reasoning {
                "summary_index"
            } else {
                "content_index"
            },
        )?;
        if part_index >= MAX_PARTS {
            return Err(invalid("Responses content index limit exceeded"));
        }
        let key = (part_kind.to_owned(), part_index);
        if item.finished_parts.contains(&key) || item.closed_parts.contains(&key) {
            return Err(invalid("Text event after part completion"));
        }
        let done = kind.ends_with(".done");
        let text = string(
            data,
            if done {
                if refusal { "refusal" } else { "text" }
            } else {
                "delta"
            },
        )?;
        let current = item.parts.entry(key.clone()).or_default();
        let delta = if done {
            text.strip_prefix(current.as_str())
                .ok_or_else(|| invalid("Completed text contradicts its deltas"))?
        } else {
            text
        };
        let delta = delta.to_owned();
        current.push_str(&delta);
        if done {
            item.finished_parts.insert(key);
        }
        // Reasoning is delivered atomically with its opaque continuation at
        // item.done, so empty summaries and consecutive blocks stay distinct.
        Ok(if reasoning || delta.is_empty() {
            Vec::new()
        } else {
            vec![ChatModelEvent::OutputTextDelta { text: delta }]
        })
    }

    fn finish_item(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let value = field(data, "item")?;
        let item = self
            .items
            .get_mut(index(data, "output_index")?)
            .ok_or_else(|| invalid("Unknown completed Responses item"))?;
        if item.snapshot.is_some()
            || identity(value, "id")? != item.id
            || identity(value, "type")? != item.kind
        {
            return Err(invalid("Completed Responses item identity mismatch"));
        }
        let mut events = Vec::new();
        match item.kind.as_str() {
            "function_call" => {
                let complete = match string(value, "status")? {
                    "completed" => true,
                    "incomplete" | "in_progress" => false,
                    _ => return Err(invalid("Invalid final function status")),
                };
                if Some(identity(value, "call_id")?) != item.call_id.as_deref()
                    || Some(identity(value, "name")?) != item.name.as_deref()
                {
                    return Err(invalid("Completed function identity or status mismatch"));
                }
                let final_args = string(value, "arguments")?;
                if final_args.len() > MAX_ARGUMENTS
                    || (item.arguments_done && final_args != item.arguments)
                {
                    return Err(invalid("Completed function arguments mismatch"));
                }
                let suffix = final_args
                    .strip_prefix(item.arguments.as_str())
                    .ok_or_else(|| invalid("Completed arguments contradict deltas"))?;
                if !suffix.is_empty() {
                    events.push(item.tool_delta(suffix.to_owned())?);
                }
                if !complete {
                    // Keep the closed snapshot for terminal corroboration,
                    // but never manufacture a completed call from its JSON.
                    self.incomplete_item = true;
                } else {
                    let arguments: Value = serde_json::from_str(final_args)
                        .map_err(|_| invalid("Function arguments are not JSON"))?;
                    if !arguments.is_object() {
                        return Err(invalid("Function arguments must be an object"));
                    }
                    events.push(ChatModelEvent::ToolCallCompleted {
                        call: ChatToolCall {
                            call_id: ToolCallId(
                                item.call_id
                                    .clone()
                                    .ok_or_else(|| invalid("Missing call identity"))?,
                            ),
                            name: item
                                .name
                                .clone()
                                .ok_or_else(|| invalid("Missing function name"))?,
                            arguments: StrictJsonValue(arguments),
                            provider_metadata: None,
                        },
                    });
                }
            }
            "message" | "reasoning" => {
                let reasoning = item.kind == "reasoning";
                if !reasoning {
                    if string(value, "role")? != "assistant" {
                        return Err(invalid("Invalid completed assistant message role"));
                    }
                    match string(value, "status")? {
                        "completed" => {}
                        "incomplete" | "in_progress" => self.incomplete_item = true,
                        _ => return Err(invalid("Invalid completed assistant message status")),
                    }
                } else if let Some(status) = value.get("status").filter(|value| !value.is_null()) {
                    match status.as_str() {
                        Some("completed") => {}
                        Some("incomplete" | "in_progress") => self.incomplete_item = true,
                        _ => return Err(invalid("Invalid reasoning item status")),
                    }
                }
                let parts = array(value, if reasoning { "summary" } else { "content" })?;
                if parts.len() > MAX_PARTS {
                    return Err(invalid("Responses content count limit exceeded"));
                }
                let mut expected = BTreeMap::new();
                let mut summary = Vec::new();
                for (index, part) in parts.iter().enumerate() {
                    let (kind, text) = part_text(part, reasoning)?;
                    let key = (kind.to_owned(), index);
                    if let Some(streamed) = item.parts.get(&key) {
                        if streamed != text {
                            return Err(invalid("Completed item contradicts streamed content"));
                        }
                    } else if !reasoning && !text.is_empty() {
                        events.push(ChatModelEvent::OutputTextDelta {
                            text: text.to_owned(),
                        });
                    }
                    if kind == "refusal" {
                        self.refusal = true;
                    }
                    expected.insert(key, text.to_owned());
                    summary.push(text);
                }
                if item
                    .parts
                    .keys()
                    .chain(&item.added_parts)
                    .chain(&item.closed_parts)
                    .any(|key| !expected.contains_key(key))
                {
                    return Err(invalid("Completed item omitted streamed content"));
                }
                if reasoning {
                    let encrypted_content = optional_text(value, "encrypted_content")?;
                    let text = summary.join("\n\n");
                    if !text.is_empty() || encrypted_content.is_some() {
                        events.push(ChatModelEvent::ReasoningBlock {
                            text,
                            encrypted_content,
                        });
                    }
                }
            }
            _ => {}
        }
        if self.preserve {
            events.push(ChatModelEvent::NativeResponsesItem {
                item_type: item.kind.clone(),
                item: StrictJsonValue(value.clone()),
            });
        }
        item.snapshot = Some(value.clone());
        Ok(events)
    }

    fn finish_response(
        &mut self,
        kind: &str,
        data: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let response = field(data, "response")?;
        self.check_response(response)?;
        let complete = kind == "response.completed";
        if complete && self.incomplete_item {
            return Err(invalid(
                "Responses success terminal contains an incomplete item",
            ));
        }
        if string(response, "status")? != if complete { "completed" } else { "incomplete" } {
            return Err(invalid("Responses terminal status mismatch"));
        }
        let output = array(response, "output")?;
        if output.len() != self.items.len()
            || output
                .iter()
                .zip(&self.items)
                .any(|(value, item)| item.snapshot.as_ref() != Some(value))
        {
            return Err(invalid(
                "Responses terminal omitted or changed an output item",
            ));
        }
        let finish_reason = if !complete {
            match response
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
            {
                Some("max_output_tokens") => ChatFinishReason::MaxOutputTokens,
                Some("content_filter") => ChatFinishReason::Refusal,
                _ => return Err(invalid("Unsupported Responses incomplete reason")),
            }
        } else if self.refusal {
            ChatFinishReason::Refusal
        } else if !self.calls.is_empty() {
            ChatFinishReason::ToolCalls
        } else {
            ChatFinishReason::Completed
        };
        let mut events = Vec::new();
        if let Some(usage) = response.get("usage").filter(|value| !value.is_null()) {
            if !usage.is_object() {
                return Err(invalid("Invalid Responses usage"));
            }
            for key in ["input_tokens", "output_tokens", "total_tokens"] {
                if usage.get(key).is_some_and(|value| value.as_u64().is_none()) {
                    return Err(invalid("Invalid Responses token count"));
                }
            }
            events.push(ChatModelEvent::Usage {
                usage: parse_usage(usage),
            });
        }
        // Requests use store:false. A response ID is observability, not proof
        // of server-retained continuation; do not emit ProviderRoundId here.
        events.push(ChatModelEvent::Completed { finish_reason });
        self.terminal = true;
        Ok(events)
    }
}

impl Item {
    fn tool_delta(&self, arguments_delta: String) -> Result<ChatModelEvent, ChatModelError> {
        Ok(ChatModelEvent::ToolCallDelta {
            call_id: ToolCallId(
                self.call_id
                    .clone()
                    .ok_or_else(|| invalid("Missing call identity"))?,
            ),
            name: self
                .name
                .clone()
                .ok_or_else(|| invalid("Missing function name"))?,
            arguments_delta,
        })
    }
}

fn invalid(message: &str) -> ChatModelError {
    ChatModelError::protocol_violation(message)
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, ChatModelError> {
    value
        .get(key)
        .ok_or_else(|| invalid("Missing native Responses field"))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ChatModelError> {
    field(value, key)?
        .as_str()
        .ok_or_else(|| invalid("Invalid native Responses string"))
}
fn identity<'a>(value: &'a Value, key: &str) -> Result<&'a str, ChatModelError> {
    let value = string(value, key)?;
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid("Invalid native Responses identity"));
    }
    Ok(value)
}
fn index(value: &Value, key: &str) -> Result<usize, ChatModelError> {
    field(value, key)?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| invalid("Invalid native Responses index"))
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, ChatModelError> {
    field(value, key)?
        .as_array()
        .ok_or_else(|| invalid("Invalid native Responses array"))
}
fn optional_text(value: &Value, key: &str) -> Result<Option<String>, ChatModelError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() => Ok(Some(text.clone())),
        _ => Err(invalid("Invalid opaque Responses continuation")),
    }
}
fn part_text(value: &Value, reasoning: bool) -> Result<(&str, &str), ChatModelError> {
    let kind = string(value, "type")?;
    match (reasoning, kind) {
        (true, "summary_text") | (false, "output_text") => Ok((kind, string(value, "text")?)),
        (false, "refusal") => Ok((kind, string(value, "refusal")?)),
        _ => Err(invalid("Unsupported Responses content part")),
    }
}
