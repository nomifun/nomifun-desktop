//! Bounded, attempt-local native Responses lifecycle decoding. Item identity is
//! not tool-call identity. Completed snapshots corroborate streamed data; they
//! never authorize execution of an incomplete or differently named call.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use crate::tool_arguments::{ToolArgumentInput, ToolArgumentParser, proposal_event};

use nomifun_agent_contracts::StrictJsonValue;
use serde_json::Value;

use crate::adapter::{ProviderWireFrame, parse_usage};
use crate::contracts::{
    ChatFinishReason, ChatModelError, ChatModelEvent, ChatOutputTextSource, ChatProtocol,
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
    argument_parser: Arc<dyn ToolArgumentParser>,
    // Standalone idle observation only; summary/opaque event delivery remains
    // atomic and no additional Agent fact or recovery state is produced.
    last_frame_semantic_progress: bool,
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
    pub(crate) fn with_argument_parser(preserve: bool, argument_parser: Arc<dyn ToolArgumentParser>) -> Self {
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
            argument_parser,
            last_frame_semantic_progress: false,
        }
    }

    pub(crate) fn has_deferred_semantic_progress(&self) -> bool { self.last_frame_semantic_progress }

    pub(crate) fn decode(
        &mut self,
        frame: &ProviderWireFrame,
    ) -> Option<Result<Vec<ChatModelEvent>, ChatModelError>> {
        self.last_frame_semantic_progress = false;
        let named = frame.event.trim().to_ascii_lowercase();
        if frame
            .data
            .get("type")
            .is_some_and(|value| !value.is_string())
        {
            return Some(Err(invalid("Invalid Responses event type")));
        }
        let declared = frame.data.get("type").and_then(Value::as_str);
        if named == "json"
            && frame.data.get("object").and_then(Value::as_str) == Some("response")
        {
            let result = self.decode_complete_response(&frame.data);
            if result.is_err() { self.last_frame_semantic_progress = false; }
            return Some(result);
        }
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
        if named == "done" && self.native == Some(true) {
            return Some(if self.terminal { Ok(Vec::new()) } else {
                Err(invalid("Responses sentinel cannot replace a terminal response"))
            });
        }
        let kind = if matches!(named.as_str(), "" | "message" | "json") {
            declared.unwrap_or(&named)
        } else {
            if declared.is_some_and(|kind| kind != named) {
                return Some(Err(invalid(
                    "Responses event name contradicts its payload type",
                )));
            }
            &named
        };
        let result = self.decode_native(kind, &frame.data);
        if result.is_err() { self.last_frame_semantic_progress = false; }
        Some(result)
    }

    fn decode_complete_response(
        &mut self,
        response: &Value,
    ) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        if self.response_id.is_some() || self.terminal {
            return Err(invalid("Duplicate complete Responses response"));
        }
        let status = string(response, "status")?;
        if !matches!(status, "completed" | "incomplete") {
            return Err(invalid("Complete Responses response has a nonterminal status"));
        }
        let mut events = self.decode_native(
            "response.created",
            &serde_json::json!({
                "type": "response.created",
                "response": {"id": identity(response, "id")?},
            }),
        )?;
        for (output_index, item) in array(response, "output")?.iter().enumerate() {
            let kind = identity(item, "type")?;
            let item_id = identity(item, "id")?;
            let mut initial = item.clone();
            match kind {
                "message" => {
                    initial["content"] = Value::Array(Vec::new());
                }
                "reasoning" => {
                    initial["summary"] = Value::Array(Vec::new());
                }
                _ => {}
            }
            events.extend(self.decode_native(
                "response.output_item.added",
                &serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": initial,
                }),
            )?);
            if kind == "message" {
                for (content_index, part) in array(item, "content")?.iter().enumerate() {
                    let part_type = identity(part, "type")?;
                    let (event, field, text) = match part_type {
                        "output_text" => (
                            "response.output_text.done",
                            "text",
                            string(part, "text")?,
                        ),
                        "refusal" => (
                            "response.refusal.done",
                            "refusal",
                            string(part, "refusal")?,
                        ),
                        _ => return Err(invalid("Unsupported complete Responses content part")),
                    };
                    let mut data = serde_json::json!({
                        "type": event,
                        "output_index": output_index,
                        "item_id": item_id,
                        "content_index": content_index,
                    });
                    data[field] = Value::String(text.to_owned());
                    events.extend(self.decode_native(event, &data)?);
                }
            } else if kind == "reasoning" {
                let empty = Vec::new();
                let summary = if item.get("summary").is_none() { &empty } else { array(item, "summary")? };
                for (summary_index, part) in summary.iter().enumerate() {
                    if identity(part, "type")? != "summary_text" {
                        return Err(invalid("Unsupported complete Responses reasoning part"));
                    }
                    events.extend(self.decode_native(
                        "response.reasoning_summary_text.done",
                        &serde_json::json!({
                            "type": "response.reasoning_summary_text.done",
                            "output_index": output_index,
                            "item_id": item_id,
                            "summary_index": summary_index,
                            "text": string(part, "text")?,
                        }),
                    )?);
                }
            }
            events.extend(self.decode_native(
                "response.output_item.done",
                &serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item,
                }),
            )?);
        }
        let terminal = if status == "completed" {
            "response.completed"
        } else {
            "response.incomplete"
        };
        events.extend(self.decode_native(
            terminal,
            &serde_json::json!({"type": terminal, "response": response}),
        )?);
        Ok(events)
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
        if data.get("response_id").is_some()
            && data.get("response_id").and_then(Value::as_str) != self.response_id.as_deref()
        {
            return Err(invalid("Responses response_id changed"));
        }
        let mut normalized = data.clone();
        if matches!(kind, "response.content_part.added" | "response.content_part.done" | "response.content_part.delta")
            && normalized.get("item_id").is_none()
        {
            if normalized.get("response_id").and_then(Value::as_str) != self.response_id.as_deref() {
                return Err(invalid("Responses content event omitted item and response identity"));
            }
            let item = self.items.get(index(data, "output_index")?)
                .ok_or_else(|| invalid("Unknown Responses content item"))?;
            normalized["item_id"] = Value::String(item.id.clone());
        }
        let kind = if kind == "response.content_part.delta" {
            let item = self.items.get(index(data, "output_index")?)
                .ok_or_else(|| invalid("Unknown Responses content item"))?;
            let content_index = index(data, "content_index")?;
            let part_kind = item.added_parts.iter().find(|(_, index)| *index == content_index)
                .map(|(kind, _)| kind.as_str());
            match part_kind {
                Some("output_text") => "response.output_text.delta",
                Some("refusal") => "response.refusal.delta",
                Some("reasoning_text") if item.kind == "reasoning" => "response.reasoning_text.delta",
                _ => return Err(invalid("Responses content delta has no announced text part")),
            }
        } else if kind == "response.done" {
            let status = string(field(data, "response")?, "status")?;
            let terminal = match status {
                "completed" => "response.completed",
                "incomplete" => "response.incomplete",
                _ => return Err(invalid("Responses response.done is not a supported terminal")),
            };
            if normalized["response"].get("output").is_none() {
                let output = self.items.iter().map(|item| item.snapshot.clone()
                    .ok_or_else(|| invalid("Responses terminal omitted an unfinished item")))
                    .collect::<Result<Vec<_>, _>>()?;
                normalized["response"]["output"] = Value::Array(output);
            }
            terminal
        } else { kind };
        let data = &normalized;
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
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.delta"
            | "response.reasoning_text.done" => self.text_event(kind, data),
            "response.content_part.added"
            | "response.content_part.done"
            | "response.reasoning_part.added"
            | "response.reasoning_part.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done" => {
                let summary = kind.starts_with("response.reasoning_summary");
                let item = self.open_item(data)?;
                let raw_reasoning = !summary && item.kind == "reasoning";
                let index = index(
                    data,
                    if summary {
                        "summary_index"
                    } else {
                        "content_index"
                    },
                )?;
                if index >= MAX_PARTS
                    || item.kind != if summary || raw_reasoning { "reasoning" } else { "message" }
                    || (kind.starts_with("response.reasoning_part") && !raw_reasoning)
                {
                    return Err(invalid("Invalid Responses content part"));
                }
                let part = field(data, "part")?;
                let (part_kind, text) = if raw_reasoning {
                    if string(part,"type")? != "reasoning_text" {
                        return Err(invalid("Invalid raw Responses reasoning part"));
                    }
                    ("reasoning_text",string(part,"text")?)
                } else { part_text(part, summary)? };
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
            // OpenAI-compatible backends add response-level event kinds
            // (e.g. `response.metadata`); vendors require clients to tolerate
            // them. Content authority stays with the strictly validated
            // output_item.done snapshots and terminal corroboration, so
            // ignoring an unknown kind cannot drop admitted content — an
            // unrepresentable part inside a snapshot still fails loudly.
            // All generic checks above (budget, terminal, provider errors,
            // sequence numbers, creation and response identity) still apply.
            _ => Ok(Vec::new()),
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
        } else if item.kind == "reasoning" && value.get("summary").is_some_and(|summary| {
            !summary.as_array().is_some_and(Vec::is_empty)
        }) {
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
        let raw_reasoning = kind.starts_with("response.reasoning_text");
        let reasoning = raw_reasoning || kind.starts_with("response.reasoning_summary_text");
        let refusal = kind.starts_with("response.refusal");
        let part_kind = if raw_reasoning {
            "reasoning_text"
        } else if reasoning {
            "summary_text"
        } else if refusal {
            "refusal"
        } else {
            "output_text"
        };
        let output_index = index(data, "output_index")? as u32;
        let item = self.open_item(data)?;
        if item.kind != if reasoning { "reasoning" } else { "message" } {
            return Err(invalid("Text event has incompatible item type"));
        }
        let part_index = index(
            data,
            if reasoning && !raw_reasoning {
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
        let source = ChatOutputTextSource {
            item_id: item.id.clone(), output_index: Some(output_index), content_index: part_index as u32,
        };
        self.last_frame_semantic_progress |= reasoning && !delta.is_empty();
        // Reasoning is delivered atomically with its opaque continuation at
        // item.done, so empty summaries and consecutive blocks stay distinct.
        Ok(if reasoning || delta.is_empty() {
            Vec::new()
        } else {
            vec![ChatModelEvent::OutputTextDelta { text: delta, source: Some(source) }]
        })
    }

    fn finish_item(&mut self, data: &Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        let value = field(data, "item")?;
        let output_index = index(data, "output_index")? as u32;
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
                    events.push(proposal_event(self.argument_parser.as_ref(),
                        ToolCallId(item.call_id.clone().ok_or_else(|| invalid("Missing call identity"))?),
                        item.name.clone().ok_or_else(|| invalid("Missing function name"))?,
                        ToolArgumentInput::JsonText(final_args), None));
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
                let empty = Vec::new();
                let parts = if reasoning && value.get("summary").is_none() {
                    &empty
                } else { array(value, if reasoning { "summary" } else { "content" })? };
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
                            source: Some(ChatOutputTextSource { item_id: item.id.clone(),
                                output_index: Some(output_index), content_index: index as u32 }),
                        });
                    }
                    if kind == "refusal" {
                        self.refusal = true;
                    }
                    expected.insert(key, text.to_owned());
                    summary.push(text);
                }
                let null_raw = reasoning && value.get("content").is_some_and(Value::is_null)
                    && value.get("encrypted_content").is_none_or(Value::is_null);
                let raw = if reasoning { value.get("content").filter(|value| !value.is_null()) } else { None };
                if let Some(raw) = raw {
                    let raw = raw.as_array().ok_or_else(|| invalid("Invalid raw Responses reasoning content"))?;
                    if raw.len() > MAX_PARTS { return Err(invalid("Raw reasoning part count limit exceeded")); }
                    for (index, part) in raw.iter().enumerate() {
                        if string(part, "type")? != "reasoning_text" {
                            return Err(invalid("Unsupported raw Responses reasoning part"));
                        }
                        let text = string(part, "text")?;
                        let key = ("reasoning_text".to_owned(), index);
                        if item.parts.get(&key).is_some_and(|streamed| streamed != text) {
                            return Err(invalid("Completed raw reasoning contradicts its deltas"));
                        }
                        expected.insert(key, text.to_owned());
                    }
                }
                if null_raw {
                    // StepFun closes reasoning with content:null even after
                    // streaming actual reasoning_text parts. Corroborate the
                    // stream locally; never insert that text into its snapshot.
                    for (key,text) in &item.parts {
                        if key.0 == "reasoning_text" { expected.insert(key.clone(),text.clone()); }
                    }
                    for key in item.added_parts.iter().chain(&item.closed_parts) {
                        if key.0 == "reasoning_text" { expected.entry(key.clone()).or_default(); }
                    }
                    let incomplete = matches!(value.get("status").and_then(Value::as_str),Some("incomplete" | "in_progress"));
                    for key in expected.keys() {
                        if !incomplete && key.0 == "reasoning_text" && !item.finished_parts.contains(key) && !item.closed_parts.contains(key) {
                            return Err(invalid("Null reasoning snapshot closed an unfinished text part"));
                        }
                    }
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
                    if null_raw || raw.is_some_and(|content| content.as_array().is_some_and(|parts| !parts.is_empty())) {
                        // Incomplete reasoning is observed but cannot become
                        // replayable continuation. The terminal retains the
                        // normal output-limit/refusal classification.
                        if matches!(value.get("status").and_then(Value::as_str), Some("incomplete" | "in_progress")) {
                            self.last_frame_semantic_progress = true;
                        } else {
                        if null_raw {
                            let mut text = item.parts.iter().filter(|((kind,_),_)| kind == "reasoning_text")
                                .map(|(_,text)|text.as_str()).collect::<Vec<_>>().join("\n\n");
                            if text.is_empty() { text = summary.join("\n\n"); }
                            if !text.is_empty() { events.push(ChatModelEvent::ReasoningDelta {text}); }
                        }
                        let block = crate::ChatProviderReasoning::OpenaiResponsesReasoning {
                            item:value.clone(), route_digest:None,
                        };
                        block.validate().map_err(invalid)?;
                        self.last_frame_semantic_progress = true;
                        events.push(ChatModelEvent::ProviderReasoningBlock { block });
                        }
                    } else {
                        let encrypted_content = optional_text(value, "encrypted_content")?;
                        self.last_frame_semantic_progress |= encrypted_content.is_some();
                        let text = summary.join("\n\n");
                        if !text.is_empty() || encrypted_content.is_some() {
                            events.push(ChatModelEvent::ReasoningBlock {
                                text,
                                encrypted_content,
                            });
                        }
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
        if output.len() != self.items.len() {
            return Err(invalid(
                "Responses terminal omitted or changed an output item",
            ));
        }
        for (value, item) in output.iter().zip(&self.items) {
            // Reasoning encrypted_content is re-encrypted on every upstream
            // serialization, so the same item legitimately arrives with new
            // ciphertext. It is excluded from corroboration for every item
            // type; continuation authority stays the output_item.done
            // snapshot and the terminal ciphertext is never read.
            if value
                .get("encrypted_content")
                .is_some_and(|ciphertext| !ciphertext.is_null() && !ciphertext.is_string())
            {
                return Err(invalid(
                    "Responses terminal omitted or changed an output item",
                ));
            }
            let corroborated = item
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| item_eq_without_ciphertext(snapshot, value));
            if !corroborated {
                return Err(invalid(
                    "Responses terminal omitted or changed an output item",
                ));
            }
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
/// Terminal output items corroborate the output_item.done snapshots exactly
/// except for the opaque top-level `encrypted_content` member, which upstream
/// re-encryption may legitimately change between serializations.
fn item_eq_without_ciphertext(snapshot: &Value, terminal: &Value) -> bool {
    fn strip(value: &Value) -> Value {
        match value.as_object() {
            Some(object) => Value::Object(
                object
                    .iter()
                    .filter(|(key, _)| key.as_str() != "encrypted_content")
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            None => value.clone(),
        }
    }
    strip(snapshot) == strip(terminal)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::ProviderWireFrame;

    fn native(decoder: &mut ResponsesDecoder, kind: &str, data: Value) -> Result<Vec<ChatModelEvent>, ChatModelError> {
        decoder.decode(&ProviderWireFrame {event:kind.into(),data,diagnostic:None}).unwrap()
    }

    fn stepfun_frames(reasoning_part: &str) -> Vec<Value> {
        let reasoning = serde_json::json!({"id":"reasoning", "type":"reasoning", "summary":[],
            "content":null, "encrypted_content":null,"status":null});
        let message = serde_json::json!({"id":"message", "type":"message", "role":"assistant", "status":"completed",
            "content":[{"type":"output_text","text":"OK","annotations":[]}]});
        vec![
            serde_json::json!({"type":"response.created","response":{"id":"resp_step","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":0,"item":reasoning}),
            serde_json::json!({"type":format!("response.{reasoning_part}.added"),"output_index":0,"item_id":"reasoning","content_index":0,"part":{"type":"reasoning_text","text":""}}),
            serde_json::json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"reasoning","content_index":0,"delta":"Actual reasoning"}),
            serde_json::json!({"type":"response.reasoning_text.done","output_index":0,"item_id":"reasoning","content_index":0,"text":"Actual reasoning"}),
            serde_json::json!({"type":format!("response.{reasoning_part}.done"),"output_index":0,"item_id":"reasoning","content_index":0,"part":{"type":"reasoning_text","text":"Actual reasoning"}}),
            serde_json::json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            serde_json::json!({"type":"response.output_item.added","output_index":1,"item":{"id":"message","type":"message","role":"assistant","status":"in_progress","content":[]}}),
            serde_json::json!({"type":"response.content_part.added","output_index":1,"item_id":"message","content_index":0,"part":{"type":"output_text","text":""}}),
            serde_json::json!({"type":"response.output_text.delta","output_index":1,"item_id":"message","content_index":0,"delta":"OK"}),
            serde_json::json!({"type":"response.output_text.done","output_index":1,"item_id":"message","content_index":0,"text":"OK"}),
            serde_json::json!({"type":"response.content_part.done","output_index":1,"item_id":"message","content_index":0,"part":{"type":"output_text","text":"OK"}}),
            serde_json::json!({"type":"response.output_item.done","output_index":1,"item":message}),
            serde_json::json!({"type":"response.completed","response":{"id":"resp_step","status":"completed","output":[reasoning,message]}}),
        ]
    }

    #[test]
    fn stepfun_live_reasoning_content_parts_and_documented_alias_keep_exact_null_snapshot() {
        for part in ["content_part","reasoning_part"] {
            let mut decoder = ResponsesDecoder::with_argument_parser(false, std::sync::Arc::new(crate::JsonToolArgumentParser));
            let mut events = Vec::new();
            let frames = stepfun_frames(part);
            for frame in &frames {
                events.extend(native(&mut decoder,frame["type"].as_str().unwrap(),frame.clone()).unwrap());
            }
            assert!(events.iter().any(|event| matches!(event,ChatModelEvent::ReasoningDelta {text} if text == "Actual reasoning")));
            let block = events.iter().find_map(|event| match event {
                ChatModelEvent::ProviderReasoningBlock {block} => Some(block), _ => None,
            }).unwrap();
            assert_eq!(block,&crate::ChatProviderReasoning::OpenaiResponsesReasoning {item:frames[6]["item"].clone(),route_digest:None});
            assert!(events.iter().any(|event| matches!(event,ChatModelEvent::OutputTextDelta {text, .. } if text == "OK")));
            assert!(events.last().unwrap().is_terminal());
        }
    }

    #[test]
    fn streamed_text_deltas_share_the_verified_item_and_content_source() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let mut frames = stepfun_frames("content_part");
        frames[9]["delta"] = serde_json::json!("O");
        let mut suffix = frames[9].clone();
        suffix["delta"] = serde_json::json!("K");
        frames.insert(10, suffix);
        let events: Vec<_> = frames.into_iter().flat_map(|frame| {
            native(&mut decoder, frame["type"].as_str().unwrap(), frame.clone()).unwrap()
        }).collect();
        let text: Vec<_> = events.iter().filter_map(|event| match event {
            ChatModelEvent::OutputTextDelta {text,source} => Some((text.as_str(),source.clone())), _ => None,
        }).collect();
        let source = Some(ChatOutputTextSource {
            item_id:"message".into(), output_index:Some(1), content_index:0,
        });
        assert_eq!(text, vec![("O",source.clone()), ("K",source)]);
    }

    #[test]
    fn stepfun_reasoning_part_identity_kind_and_finished_text_are_still_verified() {
        for (index,key,replacement) in [
            (2,"item_id",serde_json::json!("wrong")),
            (2,"part",serde_json::json!({"type":"output_text","text":""})),
            (5,"part",serde_json::json!({"type":"reasoning_text","text":"contradiction"})),
            (5,"content_index",serde_json::json!(5000)),
        ] {
            let mut decoder = ResponsesDecoder::with_argument_parser(false,std::sync::Arc::new(crate::JsonToolArgumentParser));
            let mut frames = stepfun_frames("content_part");
            frames[index][key] = replacement;
            let mut failed = false;
            for frame in frames {
                if native(&mut decoder,frame["type"].as_str().unwrap(),frame.clone()).is_err() { failed = true; break; }
            }
            assert!(failed,"accepted invalid StepFun event at {index}/{key}");
        }
    }

    fn compatible_frames() -> Vec<Value> {
        vec![
            serde_json::json!({"type":"response.created","response":{"id":"resp_compat","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","response_id":"resp_compat","output_index":0,
                "item":{"id":"message","type":"message","role":"assistant","content":[]}}),
            serde_json::json!({"type":"response.content_part.added","response_id":"resp_compat","output_index":0,"content_index":0,
                "part":{"type":"output_text","text":""}}),
            serde_json::json!({"type":"response.content_part.delta","response_id":"resp_compat","output_index":0,"content_index":0,"delta":"hello"}),
            serde_json::json!({"type":"response.output_item.done","response_id":"resp_compat","output_index":0,
                "item":{"id":"message","type":"message","role":"assistant","status":"completed",
                    "content":[{"type":"output_text","text":"hello"}]}}),
            serde_json::json!({"type":"response.done","response":{"id":"resp_compat","status":"completed",
                "usage":{"input_tokens":12,"output_tokens":3}}}),
        ]
    }

    #[test]
    fn compatible_data_only_frames_require_real_item_snapshots_and_a_terminal() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let mut events = Vec::new();
        for frame in compatible_frames() {
            events.extend(native(&mut decoder, "message", frame).unwrap());
        }
        assert!(events.iter().any(|event| matches!(event,
            ChatModelEvent::OutputTextDelta { text, .. } if text == "hello")));
        assert!(events.iter().any(|event| matches!(event, ChatModelEvent::Completed { .. })));
        assert!(native(&mut decoder, "done", serde_json::json!({})).unwrap().is_empty());
        assert!(native(&mut decoder, "message", compatible_frames()[3].clone()).is_err());

        for mutation in 0..4 {
            let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
            let mut frames = compatible_frames();
            match mutation {
                0 => { frames.remove(4); }
                1 => { frames[3]["response_id"] = serde_json::json!("other"); }
                2 => { frames[3]["content_index"] = serde_json::json!(1); }
                3 => { frames.pop(); }
                _ => unreachable!(),
            }
            let mut failed = false;
            for frame in frames {
                match native(&mut decoder, "message", frame) {
                    Ok(events) => assert!(!events.iter().any(|event| matches!(event, ChatModelEvent::Completed { .. }))),
                    Err(_) => { failed = true; break; }
                }
            }
            if !failed { assert!(native(&mut decoder, "done", serde_json::json!({})).is_err()); }
        }
    }

    #[test]
    fn raw_reasoning_must_match_its_stream_and_does_not_forge_a_summary_or_cipher() {
        for changed in [false, true] {
            let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
            native(&mut decoder,"response.created",serde_json::json!({"type":"response.created","response":{"id":"resp_raw"}})).unwrap();
            native(&mut decoder,"response.output_item.added",serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"raw","type":"reasoning","content":[]}})).unwrap();
            native(&mut decoder,"response.reasoning_text.delta",serde_json::json!({"type":"response.reasoning_text.delta",
                "output_index":0,"item_id":"raw","content_index":0,"delta":"actual text"})).unwrap();
            assert!(decoder.has_deferred_semantic_progress());
            let item=serde_json::json!({"id":"raw","type":"reasoning","content":[{
                "type":"reasoning_text","text":if changed {"changed text"} else {"actual text"}
            }]});
            let result=native(&mut decoder,"response.output_item.done",serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item}));
            if changed { assert!(result.is_err()); }
            else {
                assert!(matches!(result.unwrap().as_slice(),[ChatModelEvent::ProviderReasoningBlock {
                    block:crate::ChatProviderReasoning::OpenaiResponsesReasoning {item:actual,route_digest:None}
                }] if actual==&item));
            }
        }
    }

    #[test]
    fn incomplete_raw_reasoning_is_not_replayable_and_keeps_the_output_limit_terminal() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        native(&mut decoder,"response.created",serde_json::json!({"type":"response.created","response":{"id":"resp_cutoff"}})).unwrap();
        native(&mut decoder,"response.output_item.added",serde_json::json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"raw_cutoff","type":"reasoning","content":[]}})).unwrap();
        let item=serde_json::json!({"id":"raw_cutoff","type":"reasoning","status":"incomplete",
            "content":[{"type":"reasoning_text","text":"unfinished reasoning"}]});
        let events=native(&mut decoder,"response.output_item.done",serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item})).unwrap();
        assert!(!events.iter().any(|event| matches!(event, ChatModelEvent::ProviderReasoningBlock {..})));
        let events=native(&mut decoder,"response.incomplete",serde_json::json!({"type":"response.incomplete","response":{
            "id":"resp_cutoff","status":"incomplete","output":[item],"incomplete_details":{"reason":"max_output_tokens"}
        }})).unwrap();
        assert!(matches!(events.last(),Some(ChatModelEvent::Completed {finish_reason:ChatFinishReason::MaxOutputTokens})));
    }

    fn reasoning_decoder() -> ResponsesDecoder {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        native(&mut decoder,"response.created",serde_json::json!({"type":"response.created","response":{"id":"resp_progress"}})).unwrap();
        native(&mut decoder,"response.output_item.added",serde_json::json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"thought","type":"reasoning","summary":[]}})).unwrap();
        decoder
    }

    #[test]
    fn deferred_summary_progress_keeps_atomic_delivery_and_ignores_empty_or_repeated_snapshots() {
        let mut decoder = reasoning_decoder();
        let events = native(&mut decoder,"response.reasoning_summary_text.delta",serde_json::json!({"type":"response.reasoning_summary_text.delta",
            "output_index":0,"item_id":"thought","summary_index":0,"delta":"grounded summary"})).unwrap();
        assert!(events.is_empty(), "summary remains atomic until the reasoning item closes");
        assert!(decoder.has_deferred_semantic_progress());
        native(&mut decoder,"response.reasoning_summary_text.delta",serde_json::json!({"type":"response.reasoning_summary_text.delta",
            "output_index":0,"item_id":"thought","summary_index":0,"delta":""})).unwrap();
        assert!(!decoder.has_deferred_semantic_progress());
        native(&mut decoder,"response.reasoning_summary_text.done",serde_json::json!({"type":"response.reasoning_summary_text.done",
            "output_index":0,"item_id":"thought","summary_index":0,"text":"grounded summary"})).unwrap();
        assert!(!decoder.has_deferred_semantic_progress());
        let item = serde_json::json!({"id":"thought","type":"reasoning","summary":[{"type":"summary_text","text":"grounded summary"}]});
        let events = native(&mut decoder,"response.output_item.done",serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item})).unwrap();
        assert!(!decoder.has_deferred_semantic_progress());
        assert!(matches!(events.as_slice(), [ChatModelEvent::ReasoningBlock {text,encrypted_content:None}] if text == "grounded summary"));
        native(&mut decoder,"response.completed",serde_json::json!({"type":"response.completed","response":{
            "id":"resp_progress","status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":1}}})).unwrap();
        assert!(!decoder.has_deferred_semantic_progress(), "corroborating terminal snapshots contain no new reasoning");
    }

    #[test]
    fn new_opaque_reasoning_counts_as_progress_but_invalid_or_duplicate_frames_do_not() {
        let mut decoder = reasoning_decoder();
        let item = serde_json::json!({"id":"thought","type":"reasoning","summary":[],"encrypted_content":"private-opaque-data"});
        native(&mut decoder,"response.output_item.done",serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item})).unwrap();
        assert!(decoder.has_deferred_semantic_progress());
        assert!(native(&mut decoder,"response.output_item.done",serde_json::json!({"type":"response.output_item.done","output_index":0,"item":item})).is_err());
        assert!(!decoder.has_deferred_semantic_progress());
        let mut decoder = reasoning_decoder();
        native(&mut decoder,"response.reasoning_summary_text.delta",serde_json::json!({"type":"response.reasoning_summary_text.delta",
            "output_index":0,"item_id":"thought","summary_index":0,"delta":"valid summary"})).unwrap();
        assert!(decoder.has_deferred_semantic_progress());
        assert!(native(&mut decoder,"response.reasoning_summary_text.delta",serde_json::json!({"type":"response.reasoning_summary_text.delta",
            "output_index":0,"item_id":"wrong-thought","summary_index":0,"delta":"wrong identity"})).is_err());
        assert!(!decoder.has_deferred_semantic_progress());
        let mut decoder = ResponsesDecoder::with_argument_parser(false,Arc::new(crate::JsonToolArgumentParser));
        assert!(native(&mut decoder,"json",serde_json::json!({"id":"invalid_composite","object":"response","status":"completed",
            "output":[{"id":"thought","type":"reasoning","summary":[],"encrypted_content":"valid opaque"},
                {"id":"bad","type":"unsupported"}],"usage":{"input_tokens":1,"output_tokens":1}})).is_err());
        assert!(!decoder.has_deferred_semantic_progress());
    }

    #[test]
    fn complete_json_response_is_projected_through_the_native_lifecycle() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let events = decoder
            .decode(&ProviderWireFrame {
                diagnostic: None,
                event: "json".into(),
                data: serde_json::json!({
                    "id": "resp_1",
                    "object": "response",
                    "status": "completed",
                    "output": [{
                        "id": "msg_1",
                        "type": "message",
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": "hello", "annotations": []}]
                    }],
                    "usage": {"input_tokens": 3, "output_tokens": 4, "total_tokens": 7}
                }),
            })
            .expect("complete JSON response is handled")
            .unwrap();
        assert!(matches!(events.first(), Some(ChatModelEvent::ResponseStarted { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            ChatModelEvent::OutputTextDelta { text, source: Some(source) }
                if text == "hello" && source.item_id == "msg_1"
                    && source.output_index == Some(0) && source.content_index == 0
        )));
        assert!(matches!(
            events.last(),
            Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })
        ));
    }

    #[test]
    fn native_text_sources_keep_distinct_messages_and_content_blocks() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let events = native(&mut decoder, "json", serde_json::json!({
            "id":"response_sources", "object":"response", "status":"completed",
            "output":[
                {"id":"message_a", "type":"message", "role":"assistant", "status":"completed",
                    "content":[{"type":"output_text", "text":"first"}, {"type":"output_text", "text":"second"}]},
                {"id":"message_b", "type":"message", "role":"assistant", "status":"completed",
                    "content":[{"type":"output_text", "text":"third"}]}
            ]
        })).unwrap();
        let sources: Vec<_> = events.into_iter().filter_map(|event| match event {
            ChatModelEvent::OutputTextDelta { source, .. } => source, _ => None,
        }).collect();
        assert_eq!(sources, vec![
            ChatOutputTextSource { item_id:"message_a".into(), output_index:Some(0), content_index:0 },
            ChatOutputTextSource { item_id:"message_a".into(), output_index:Some(0), content_index:1 },
            ChatOutputTextSource { item_id:"message_b".into(), output_index:Some(1), content_index:0 },
        ]);
    }

    /// Gateways re-encrypt reasoning encrypted_content at every serialization,
    /// so added, done and terminal legitimately carry different ciphertext.
    fn gateway_reasoning_tool_stream() -> Vec<Value> {
        vec![
            serde_json::json!({"type":"response.created",
                "response":{"id":"resp_gw","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-a"}}),
            serde_json::json!({"type":"response.output_item.done","output_index":0,
                "item":{"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-b"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":1,
                "item":{"id":"fc_1","type":"function_call","status":"in_progress",
                    "arguments":"","call_id":"call_1","name":"get_time"}}),
            serde_json::json!({"type":"response.function_call_arguments.delta","output_index":1,
                "item_id":"fc_1","delta":"{}"}),
            serde_json::json!({"type":"response.function_call_arguments.done","output_index":1,
                "item_id":"fc_1","arguments":"{}"}),
            serde_json::json!({"type":"response.output_item.done","output_index":1,
                "item":{"id":"fc_1","type":"function_call","status":"completed",
                    "arguments":"{}","call_id":"call_1","name":"get_time"}}),
            serde_json::json!({"type":"response.completed",
                "response":{"id":"resp_gw","status":"completed","output":[
                    {"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-c"},
                    {"id":"fc_1","type":"function_call","status":"completed",
                        "arguments":"{}","call_id":"call_1","name":"get_time"}
                ],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}),
        ]
    }

    #[test]
    fn reencrypted_terminal_reasoning_ciphertext_still_corroborates_the_done_snapshot() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let mut events = Vec::new();
        for frame in gateway_reasoning_tool_stream() {
            let kind = frame["type"].as_str().unwrap().to_owned();
            events.extend(native(&mut decoder, &kind, frame).unwrap());
        }
        assert!(events.iter().any(|event| matches!(event,
            ChatModelEvent::ReasoningBlock { encrypted_content, .. }
                if encrypted_content.as_deref() == Some("cipher-b"))),
            "continuation ciphertext comes from output_item.done, never the terminal");
        assert!(events.iter().any(|event| matches!(event,
            ChatModelEvent::ToolCallCompleted { call } if call.name == "get_time")));
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed {
            finish_reason: ChatFinishReason::ToolCalls })));
    }

    #[test]
    fn terminal_output_changes_beyond_ciphertext_are_still_rejected() {
        for case in 0..6 {
            let mut frames = gateway_reasoning_tool_stream();
            {
                let output = frames[7]["response"]["output"].as_array_mut().unwrap();
                match case {
                    0 => { output.truncate(1); }
                    1 => output.reverse(),
                    2 => output[1]["arguments"] = serde_json::json!("{\"tz\":\"UTC\"}"),
                    3 => output[0]["id"] = serde_json::json!("rs_other"),
                    4 => output[0]["summary"] =
                        serde_json::json!([{"type":"summary_text","text":"changed"}]),
                    5 => output[0]["encrypted_content"] = serde_json::json!(123),
                    _ => unreachable!(),
                }
            }
            let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
            let mut failed = false;
            for frame in frames {
                let kind = frame["type"].as_str().unwrap().to_owned();
                if native(&mut decoder, &kind, frame).is_err() {
                    failed = true;
                    break;
                }
            }
            assert!(failed, "terminal mutation {case} must be rejected");
        }
    }

    /// The codex gateway shape: a normal function_call lifecycle plus a
    /// response.metadata event the upstream adds before the terminal.
    fn codex_metadata_stream() -> Vec<Value> {
        vec![
            serde_json::json!({"type":"response.created",
                "response":{"id":"resp_codex","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"fc_1","type":"function_call","status":"in_progress",
                    "arguments":"","call_id":"call_1","name":"get_time"}}),
            serde_json::json!({"type":"response.function_call_arguments.done","output_index":0,
                "item_id":"fc_1","arguments":"{}"}),
            serde_json::json!({"type":"response.output_item.done","output_index":0,
                "item":{"id":"fc_1","type":"function_call","status":"completed",
                    "arguments":"{}","call_id":"call_1","name":"get_time"}}),
            serde_json::json!({"type":"response.metadata","sequence_number":10,
                "response_id":"resp_codex",
                "metadata":{"openai_chatgpt_moderation_metadata":{"severity":"low"}}}),
            serde_json::json!({"type":"response.completed",
                "response":{"id":"resp_codex","status":"completed","output":[
                    {"id":"fc_1","type":"function_call","status":"completed",
                        "arguments":"{}","call_id":"call_1","name":"get_time"}
                ]}}),
        ]
    }

    #[test]
    fn unknown_response_level_event_kinds_are_ignored_before_the_terminal() {
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let mut events = Vec::new();
        for frame in codex_metadata_stream() {
            let kind = frame["type"].as_str().unwrap().to_owned();
            events.extend(native(&mut decoder, &kind, frame).unwrap());
        }
        assert!(events.iter().any(|event| matches!(event,
            ChatModelEvent::ToolCallCompleted { call } if call.name == "get_time")));
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed {
            finish_reason: ChatFinishReason::ToolCalls })));
    }

    #[test]
    fn unknown_item_scoped_events_are_ignored_without_dropping_streamed_text() {
        let message = serde_json::json!({"id":"message","type":"message","role":"assistant",
            "status":"completed","content":[{"type":"output_text","text":"hello"}]});
        let frames = vec![
            serde_json::json!({"type":"response.created","response":{"id":"resp_ann","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"message","type":"message","role":"assistant","status":"in_progress","content":[]}}),
            serde_json::json!({"type":"response.content_part.added","output_index":0,"item_id":"message",
                "content_index":0,"part":{"type":"output_text","text":""}}),
            serde_json::json!({"type":"response.output_text.delta","output_index":0,"item_id":"message",
                "content_index":0,"delta":"hel"}),
            // An item-scoped event kind this decoder does not model yet.
            serde_json::json!({"type":"response.output_text.annotation.added","output_index":0,"item_id":"message",
                "content_index":0,"annotation":{"type":"url_citation","url":"https://example.invalid"}}),
            serde_json::json!({"type":"response.output_text.delta","output_index":0,"item_id":"message",
                "content_index":0,"delta":"lo"}),
            serde_json::json!({"type":"response.output_text.done","output_index":0,"item_id":"message",
                "content_index":0,"text":"hello"}),
            serde_json::json!({"type":"response.content_part.done","output_index":0,"item_id":"message",
                "content_index":0,"part":{"type":"output_text","text":"hello"}}),
            serde_json::json!({"type":"response.output_item.done","output_index":0,"item":message}),
            serde_json::json!({"type":"response.completed","response":{"id":"resp_ann","status":"completed",
                "output":[message]}}),
        ];
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        let mut events = Vec::new();
        for frame in &frames {
            let kind = frame["type"].as_str().unwrap().to_owned();
            events.extend(native(&mut decoder, &kind, frame.clone()).unwrap());
        }
        let text: String = events.iter().filter_map(|event| match event {
            ChatModelEvent::OutputTextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(text, "hello", "streamed text must survive ignored events");
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed {
            finish_reason: ChatFinishReason::Completed })));
    }

    #[test]
    fn unknown_event_kinds_keep_identity_sequence_and_lifecycle_guards() {
        // A different response_id is still a stream violation.
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        for frame in &codex_metadata_stream()[..4] {
            let kind = frame["type"].as_str().unwrap().to_owned();
            native(&mut decoder, &kind, frame.clone()).unwrap();
        }
        assert!(native(&mut decoder,"response.metadata",serde_json::json!({"type":"response.metadata",
            "sequence_number":11,"response_id":"other","metadata":{}})).is_err());
        // A non-advancing sequence number is still a stream violation.
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        for frame in &codex_metadata_stream()[..5] {
            let kind = frame["type"].as_str().unwrap().to_owned();
            native(&mut decoder, &kind, frame.clone()).unwrap();
        }
        assert!(native(&mut decoder,"response.metadata",serde_json::json!({"type":"response.metadata",
            "sequence_number":10,"response_id":"resp_codex","metadata":{}})).is_err());
        // Nothing unknown survives the terminal.
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        for frame in &codex_metadata_stream() {
            let kind = frame["type"].as_str().unwrap().to_owned();
            native(&mut decoder, &kind, frame.clone()).unwrap();
        }
        assert!(native(&mut decoder,"response.metadata",serde_json::json!({"type":"response.metadata",
            "response_id":"resp_codex","metadata":{}})).is_err());
        // Nothing runs before response.created.
        let mut decoder = ResponsesDecoder::with_argument_parser(false, Arc::new(crate::JsonToolArgumentParser));
        assert!(native(&mut decoder,"response.metadata",serde_json::json!({"type":"response.metadata",
            "response_id":"resp_codex","metadata":{}})).is_err());
    }
}
