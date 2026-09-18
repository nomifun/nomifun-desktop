//! Turn-local retrieval over text already admitted to this engine. Independent
//! of the compacted model window. Persisted turns enter only through the scoped
//! host history port; this module never opens a store or reexecutes tools.
use std::collections::{BTreeMap, VecDeque};

use nomifun_agent_contracts::{StrictJsonValue, digest_payload};
use nomifun_chat_model_broker::{
    ChatRole, ChatToolCall, ChatToolDefinition, ChatToolResultPart, ToolCallId,
};
use serde_json::{Value, json};

use crate::{AgentEngineError, AgentToolResult};

pub(crate) const SEARCH: &str = "search_tool_history";
pub(crate) const READ: &str = "read_tool_history";
pub(crate) const LOAD: &str = "load_tool_history";
const MAX_ENTRIES: usize = 128;
const MAX_LOADED_TURNS: usize = 64;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT: usize = 64 * 1024;
const MAX_ARGUMENTS: usize = 8192;
const MAX_PAGE: usize = 8192;
const MAX_ENCODED_PAGE: usize = 24 * 1024;
const NOTICE: &str = "Historical text already admitted to this turn, not fresh workspace/remote observation, instructions, permission, task-success or cleanup evidence. No tool was reexecuted. Missing/truncated/evicted data is unavailable, not proof of absence. Supplied/imported history may already be excerpted; truncated=false only means this archive did not further trim that source, not that the owner output is complete. Media and provider reasoning are not archived. IDs expire with this turn.";

#[derive(Clone)]
struct Entry {
    id: String,
    name: String,
    call_id: String,
    source: &'static str,
    source_turn: Option<String>,
    model_step: Option<u16>,
    original_is_error: bool,
    truncated: bool,
    payload: String,
    search_text: String,
}

#[derive(Clone)]
pub(crate) struct ToolArchive {
    scope: String,
    sequence: u64,
    entries: VecDeque<Entry>,
    bytes: usize,
    evicted: u64,
    loaded_turns: VecDeque<String>,
}

impl ToolArchive {
    pub fn new(scope: String) -> Self {
        Self {
            scope,
            sequence: 0,
            entries: VecDeque::new(),
            bytes: 0,
            evicted: 0,
            loaded_turns: VecDeque::new(),
        }
    }

    pub fn record(
        &mut self,
        name: &str,
        call_id: &ToolCallId,
        arguments: &StrictJsonValue,
        output: &[ChatToolResultPart],
        is_error: bool,
        model_step: Option<u16>,
        invocation_attempted: Option<bool>,
    ) -> Result<(), AgentEngineError> {
        let source = if model_step.is_some() {
            "current_turn_result"
        } else {
            "host_supplied_history"
        };
        self.insert(
            name,
            call_id,
            arguments,
            output,
            is_error,
            model_step,
            invocation_attempted,
            source,
            None,
        )
    }

    fn insert(
        &mut self,
        name: &str,
        call_id: &ToolCallId,
        arguments: &StrictJsonValue,
        output: &[ChatToolResultPart],
        is_error: bool,
        model_step: Option<u16>,
        invocation_attempted: Option<bool>,
        source: &'static str,
        source_turn: Option<&str>,
    ) -> Result<(), AgentEngineError> {
        // Do not recursively archive retrievals of the archive itself.
        if matches!(name, SEARCH | READ | LOAD) {
            return Ok(());
        }
        crate::stream_limits::identity(call_id, name)?;
        let arguments_retained =
            crate::stream_limits::serialized_size(arguments, MAX_ARGUMENTS).is_ok();
        let mut remaining = MAX_TEXT;
        let mut text_parts = Vec::new();
        let mut search_text = String::new();
        let mut truncated = !arguments_retained;
        let mut omitted_media_parts = 0usize;
        for (index, part) in output.iter().enumerate() {
            match part {
                ChatToolResultPart::Text { text } => {
                    let retained = prefix(text, remaining);
                    let partial = retained.len() != text.len();
                    truncated |= partial;
                    // Limit metadata too, including results with many empty parts.
                    if text_parts.len() < 64 {
                        if !search_text.is_empty() {
                            search_text.push('\n');
                        }
                        search_text.push_str(retained);
                        text_parts.push(json!({"part":index,"original_bytes":text.len(),
                            "truncated":partial,"text":retained}));
                        remaining -= retained.len();
                    } else {
                        truncated = true;
                    }
                }
                ChatToolResultPart::Image { .. } | ChatToolResultPart::Audio { .. } => {
                    omitted_media_parts += 1;
                }
            }
        }
        let payload = json!({"source":source,"model_step":model_step,"call_id":call_id,"tool":name,
            "source_turn":source_turn,"source_may_be_bounded":source != "current_turn_result",
            "original_is_error":is_error,"invocation_attempted":invocation_attempted,
            "arguments":if arguments_retained { Some(&arguments.0) } else { None },
            "arguments_omitted":!arguments_retained,"text_parts":text_parts,"original_parts":output.len(),
            "omitted_media_parts":omitted_media_parts,"truncated":truncated}).to_string();
        if payload.len() > 512 * 1024 {
            return Err(invalid("tool archive record exceeds its projected bound"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("tool archive sequence exhausted"))?;
        let id = digest_payload(&(
            "agent-tool-archive-v1",
            &self.scope,
            self.sequence,
            &payload,
        ))
        .map_err(|error| invalid(&error.to_string()))?
        .as_ref()
        .to_owned();
        let entry = Entry {
            id,
            name: name.into(),
            call_id: call_id.as_ref().into(),
            source,
            source_turn: source_turn.map(str::to_owned),
            model_step,
            original_is_error: is_error,
            truncated,
            payload,
            search_text,
        };
        let size = entry.bytes();
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(size) > MAX_BYTES {
            let Some(old) = self.entries.pop_front() else {
                return Err(invalid("tool archive entry exceeds capacity"));
            };
            self.bytes -= old.bytes();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.bytes += size;
        self.entries.push_back(entry);
        Ok(())
    }

    pub fn context(&self) -> String {
        format!(
            "Nomi tool history archive: {} retained results, {} older records evicted. Use search_tool_history to find previous tool text by literal substring (empty query lists); read_tool_history pages an exact returned ID. If load_tool_history is available, it imports one persisted older turn at a time using a platform-checked receipt cursor, then search this archive. This bounded archive survives model-window compaction only within this turn; it is not an automatically complete Conversation index. {}",
            self.entries.len(),
            self.evicted,
            NOTICE
        )
    }

    pub async fn load(
        &mut self,
        call: &ChatToolCall,
        port: &dyn crate::AgentHistoryPort,
        causality: &nomifun_chat_model_broker::ChatCausality,
        binding: &crate::EngineBinding,
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<AgentToolResult, AgentEngineError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Load {
            before_turn: Option<String>,
        }
        let invalid_args = || {
            AgentToolResult::text(
                call.call_id.clone(),
                "Use optional before_turn from the preceding load; omit instead of null. The cursor must belong to this Session before the current turn.",
                true,
            )
        };
        if call
            .arguments
            .0
            .get("before_turn")
            .is_some_and(Value::is_null)
        {
            return Ok(invalid_args());
        }
        let Ok(args) = serde_json::from_value::<Load>(call.arguments.0.clone()) else {
            return Ok(invalid_args());
        };
        if args
            .before_turn
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Ok(invalid_args());
        }
        let page = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            result = port.read_previous(causality, args.before_turn.as_deref()) => match result {
                Ok(page) => page,
                Err(AgentEngineError::Cancelled) => return Err(AgentEngineError::Cancelled),
                Err(_) => return Ok(AgentToolResult::text(call.call_id.clone(), "Historical read unavailable or outside its scoped budget. No historical tools were executed and no archive was changed.", true)),
            },
        };
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let failed_cursor = page
            .turn
            .as_ref()
            .filter(|turn| {
                !turn.operation_id.is_empty()
                    && turn.operation_id.len() <= 256
                    && !turn.operation_id.chars().any(char::is_control)
            })
            .map(|turn| (turn.operation_id.clone(), page.has_older));
        let result = self.import(page, binding);
        Ok(match result {
            Ok(value) => AgentToolResult::text(call.call_id.clone(), value.to_string(), false),
            Err(_) => AgentToolResult::text(call.call_id.clone(), json!({
                "notice":"Historical turn has incompatible or incomplete provenance; no archive was changed. Do not infer tool execution, cleanup or permission to replay.",
                "status":"rejected_history","source_turn":failed_cursor.as_ref().map(|(id, _)| id),
                "next_before_turn":failed_cursor.as_ref().filter(|(_, older)| *older).map(|(id, _)| id),
            }).to_string(), true),
        })
    }

    fn import(
        &mut self,
        page: crate::AgentHistoryPage,
        binding: &crate::EngineBinding,
    ) -> Result<Value, AgentEngineError> {
        let Some(turn) = page.turn else {
            if page.has_older {
                return Err(invalid("empty history page cannot hide a continuation"));
            }
            return Ok(
                json!({"notice":NOTICE,"status":"end","imported_records":0,"has_older":false}),
            );
        };
        if turn.operation_id.is_empty()
            || turn.operation_id.len() > 256
            || turn.operation_id.chars().any(char::is_control)
            || turn.receipt_status.len() > 64
            || turn.events.len() > 4096
            || turn.requirement.role != ChatRole::User
        {
            return Err(invalid("invalid historical turn envelope"));
        }
        crate::stream_limits::serialized_size(&turn.events, 8 * 1024 * 1024)?;
        crate::stream_limits::serialized_size(&turn.requirement, 8 * 1024 * 1024)?;
        let cursor = if page.has_older {
            Some(turn.operation_id.as_str())
        } else {
            None
        };
        if turn.events.is_empty() {
            return Ok(
                json!({"notice":NOTICE,"status":"no_engine_records","source_turn":turn.operation_id,
                "receipt_status":turn.receipt_status,"imported_records":0,"has_older":page.has_older,"next_before_turn":cursor}),
            );
        }
        let Some(crate::AgentEngineEvent::TurnStarted {
            binding: recorded,
            turn_operation_id,
        }) = turn.events.first()
        else {
            return Err(invalid("historical turn has no engine root"));
        };
        if recorded != binding || turn_operation_id.as_ref() != turn.operation_id {
            return Err(invalid("historical turn belongs to another exact binding"));
        }
        crate::history::validate_archive_turn(turn.requirement, &turn.events)?;
        if self.loaded_turns.contains(&turn.operation_id) {
            return Ok(json!({"notice":NOTICE,"status":"already_loaded",
                "source_turn":turn.operation_id,"imported_records":0,
                "has_older":page.has_older,"next_before_turn":cursor,
                "next":"Search retained records. Earlier imports may have been evicted; repeated loading does not refresh them while this source is in the bounded recent-load index."}));
        }
        // Validate the entire source first, then publish the complete derived
        // archive update. Read failures must not leave partial imports/evictions.
        let mut candidate = self.clone();
        let initial_sequence = candidate.sequence;
        let initial_evicted = candidate.evicted;
        let mut calls = BTreeMap::new();
        for event in &turn.events {
            match event {
                crate::AgentEngineEvent::ToolCallCompleted { step, call } if *step > 0 => {
                    calls.insert(&call.call_id, call);
                }
                crate::AgentEngineEvent::ToolCompleted { step, result } if *step > 0 => {
                    let call = calls
                        .remove(&result.call_id)
                        .ok_or_else(|| invalid("historical result has no exact call"))?;
                    candidate.insert(
                        &call.name,
                        &call.call_id,
                        &call.arguments,
                        &result.output,
                        result.is_error,
                        Some(*step),
                        None,
                        "persisted_turn_result",
                        Some(&turn.operation_id),
                    )?;
                }
                _ => {}
            }
        }
        let result = json!({"notice":NOTICE,"status":"loaded","source_turn":turn.operation_id,
            "receipt_status":turn.receipt_status,"imported_records":candidate.sequence - initial_sequence,
            "evicted_records":candidate.evicted - initial_evicted,"retained_records":candidate.entries.len(),
            "has_older":page.has_older,"next_before_turn":cursor,
            "next":"Search the archive; imported records may themselves be truncated/evicted. Persisted observations are not original output restoration or cleanup proof."});
        if candidate.loaded_turns.len() == MAX_LOADED_TURNS {
            candidate.loaded_turns.pop_front();
        }
        candidate.loaded_turns.push_back(turn.operation_id);
        *self = candidate;
        Ok(result)
    }

    pub fn handle(&self, call: &ChatToolCall) -> AgentToolResult {
        let result = match call.name.as_str() {
            SEARCH => self.search(&call.arguments.0),
            READ => self.read(&call.arguments.0),
            _ => Err("Unknown tool history control"),
        };
        match result {
            Ok(value) => AgentToolResult::text(call.call_id.clone(), value.to_string(), false),
            Err(message) => AgentToolResult::text(call.call_id.clone(), message, true),
        }
    }

    fn search(&self, input: &Value) -> Result<Value, &'static str> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Search {
            query: String,
            #[serde(default)]
            call_id: Option<String>,
            #[serde(default)]
            after_id: Option<String>,
            #[serde(default = "search_limit")]
            limit: usize,
        }
        if ["after_id", "call_id"]
            .iter()
            .any(|key| input.get(*key).is_some_and(Value::is_null))
        {
            return Err("Omit after_id/call_id instead of null");
        }
        let args: Search = serde_json::from_value(input.clone())
            .map_err(|_| "Use query and optional call_id/after_id/limit")?;
        if args
            .call_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err("call_id must be a nonempty exact ID of at most 256 bytes");
        }
        if args.query.len() > 256 || !(1..=8).contains(&args.limit) {
            return Err("Query is at most 256 bytes; limit is 1..8");
        }
        let end = match args.after_id {
            None => self.entries.len(),
            Some(id) => self
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or("History cursor expired or unknown; restart search without after_id")?,
        };
        let matches = self
            .entries
            .iter()
            .take(end)
            .rev()
            .filter(|entry| args.call_id.as_ref().is_none_or(|id| id == &entry.call_id))
            .filter(|entry| {
                entry.search_text.contains(&args.query) || entry.payload.contains(&args.query)
            })
            .collect::<Vec<_>>();
        let mut count = args.limit.min(matches.len());
        loop {
            let hits = matches.iter().take(count).map(|entry| {
                let (preview, preview_kind) = if let Some(position) = entry.search_text.find(&args.query) {
                    (prefix(&entry.search_text[position..], 192), "retained_text")
                } else {
                    let position = entry.payload.find(&args.query).unwrap_or(0);
                    (prefix(&entry.payload[position..], 192), "projected_json")
                };
                json!({"id":entry.id,"tool":entry.name,"call_id":entry.call_id,"source":entry.source,"source_turn":entry.source_turn,
                    "model_step":entry.model_step,"original_is_error":entry.original_is_error,
                    "truncated":entry.truncated,"total_bytes":entry.payload.len(),
                    "preview_kind":preview_kind,"preview":preview})
            }).collect::<Vec<_>>();
            let value = json!({"notice":NOTICE,"search":"case-sensitive literal in retained text or projected JSON; latest archive insertion first, not chronological execution order across imported turns",
                "retained_records":self.entries.len(),"evicted_records":self.evicted,
                "hits":hits,"has_more":count < matches.len(),
                "next_after_id":if count < matches.len() && count > 0 { Some(&matches[count - 1].id) } else { None }});
            if fits(&value) {
                return Ok(value);
            }
            if count <= 1 {
                return Err("History search metadata exceeds output bound");
            }
            count -= 1;
        }
    }

    fn read(&self, input: &Value) -> Result<Value, &'static str> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Read {
            id: String,
            #[serde(default)]
            offset: usize,
            #[serde(default = "page_limit")]
            limit: usize,
        }
        let args: Read = serde_json::from_value(input.clone())
            .map_err(|_| "Use id and optional byte offset/limit")?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == args.id)
            .ok_or("History record expired, evicted or unknown; search the current archive")?;
        if !(4..=MAX_PAGE).contains(&args.limit)
            || args.offset > entry.payload.len()
            || !entry.payload.is_char_boundary(args.offset)
        {
            return Err("Use preceding next_offset at a UTF-8 boundary and limit 4..8192");
        }
        let mut limit = args.limit;
        loop {
            let text = prefix(&entry.payload[args.offset..], limit);
            let end = args.offset + text.len();
            let value = json!({"notice":NOTICE,"id":entry.id,"source":entry.source,"source_turn":entry.source_turn,"original_is_error":entry.original_is_error,
                "truncated":entry.truncated,"offset":args.offset,"end_offset":end,"total_bytes":entry.payload.len(),
                "next_offset":if end < entry.payload.len() { Some(end) } else { None },"eof":end == entry.payload.len(),
                "json_fragment":text});
            if fits(&value) {
                return Ok(value);
            }
            if limit <= 4 {
                return Err("History page metadata exceeds output bound");
            }
            limit = (limit / 2).max(4);
        }
    }
}

impl Entry {
    fn bytes(&self) -> usize {
        self.payload.len()
            + self.search_text.len()
            + self.id.len()
            + self.name.len()
            + self.call_id.len()
            + self.source_turn.as_ref().map_or(0, String::len)
            + 128
    }
}
fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn fits(value: &Value) -> bool {
    crate::stream_limits::serialized_size(&value.to_string(), MAX_ENCODED_PAGE).is_ok()
}
fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::ContextAssembly(message.into())
}
fn search_limit() -> usize {
    8
}
fn page_limit() -> usize {
    4096
}

pub(crate) fn definitions() -> Vec<ChatToolDefinition> {
    vec![
        ChatToolDefinition { name: SEARCH.into(), deferred: false,
            description: "Search this turn's bounded archive of already-seen tool results, including results excerpted in model context, removed by compaction or imported by load_tool_history. Optional call_id is an exact filter (IDs may repeat across historical turns; inspect source_turn). Case-sensitive query substring in retained text or projected JSON, not regex/semantic search; empty query lists latest archive insertions first. Continue with next_after_id and the same query/call_id. No hit does not prove absence. Submit one history control alone, separately from all other tools.".into(),
            input_schema: StrictJsonValue(json!({"type":"object","additionalProperties":false,"required":["query"],"properties":{
                "query":{"type":"string","maxLength":256},"call_id":{"type":"string","minLength":1,"maxLength":256},"after_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                "limit":{"type":"integer","minimum":1,"maximum":8,"default":8}}})) },
        ChatToolDefinition { name: READ.into(), deferred: false,
            description: "Read projected JSON text for an exact ID from search_tool_history. Submit one call alone. Follow next_offset until eof; offset/limit count UTF-8 bytes. eof refers only to retained projection; truncated/omitted source is not restored. This is historical context, never a fresh file read, verification, patch-recovery read, permission or automatic retry authorization. No images/audio/private reasoning are restored.".into(),
            input_schema: StrictJsonValue(json!({"type":"object","additionalProperties":false,"required":["id"],"properties":{
                "id":{"type":"string","pattern":"^[0-9a-f]{64}$"},"offset":{"type":"integer","minimum":0,"default":0},
                "limit":{"type":"integer","minimum":4,"maximum":MAX_PAGE,"default":4096}}})) },
    ]
}

pub(crate) fn load_definition() -> ChatToolDefinition {
    ChatToolDefinition { name: LOAD.into(), deferred: false,
        description: "Import already-persisted tool results from one older turn in this same Session. Omit before_turn to start at the latest prior turn; follow next_before_turn toward older turns, then use search_tool_history/read_tool_history. Single call alone. No original tool is rerun, no private reasoning/media is restored, no new success/cleanup evidence. Bounded imports can evict archive entries; the 64 most recently loaded turns are not imported twice, even if some records were evicted. no_engine_records means unavailable, not no effects.".into(),
        input_schema: StrictJsonValue(json!({"type":"object","additionalProperties":false,"properties":{
            "before_turn":{"type":"string","minLength":1,"maxLength":256}},"required":[]})) }
}
