//! One authenticated tool read in one SQLite snapshot. Body bytes are resolved
//! only by the canonical Store and never copied to a message or runtime cache.
use super::*;
use nomifun_agent_contracts::{ToolDetailAvailability as Availability, ToolDetailFormat as Format,
    ToolDetailPage, ToolDetailQuery, ToolDetailRetention, ToolDetailSection as Section};
use nomifun_agent_contracts::chat_model::{ChatToolCall, ChatToolResultPart, ToolCallId};
use nomifun_engine_core::EngineToolResult;
use serde::{Deserialize, Serialize};

const MAX_DETAIL_IDENTITIES: i64 = 16;
const MAX_DETAIL_PAGE_BYTES: u32 = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    session: String,
    turn: String,
    call: String,
    call_event: String,
    result_seq: Option<u64>,
    through_seq: u64,
    section: Section,
    digest: DigestHex,
    offset: u64,
}

#[derive(sqlx::FromRow)]
struct Turn {
    operation_id: String,
    started_event_id: String,
    terminal_event_id: Option<String>,
    finished_at: Option<i64>,
    state: String,
}

#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Observation {
    ToolCallCompleted { step: u16, call: ChatToolCall },
    ToolStarted { step: u16, call_id: ToolCallId, capability_id: String, action_id: String },
    ToolCompleted { step: u16, result: EngineToolResult },
    HostToolSettled { call_id: ToolCallId, result: EngineToolResult },
    HostToolDispatch { dispatch: Dispatch },
}

#[derive(Deserialize)]
struct Dispatch { call_id: ToolCallId, capability_id: String, action_id: String }

fn invalid(detail: &str) -> SessionStoreError {
    SessionStoreError::InvalidEvent(format!("canonical tool detail: {detail}"))
}

fn bad_query(detail: &str) -> SessionStoreError { SessionStoreError::InvalidSession(detail.into()) }

fn observe_action(identity: &mut Option<(String, String)>, capability: String, action: String) -> Result<(), SessionStoreError> {
    let observed = (capability, action);
    if identity.as_ref().is_some_and(|original| original != &observed) {
        return Err(invalid("host dispatch and runtime admission identities differ"));
    }
    *identity = Some(observed);
    Ok(())
}

impl AgentSessionStore {
    /// Read already committed invocation data. This method never acquires an
    /// engine, native owner, lease, current tool binding, or execution port.
    pub async fn tool_detail_page(&self, owner: &PrincipalRef, session: &AgentSessionId,
        query: &ToolDetailQuery) -> Result<ToolDetailPage, SessionStoreError> {
        validate_uuidv7(&query.turn_id, "tool detail turn_id")?;
        if query.call_id.is_empty() || query.call_id.len() > 256
            || query.call_id.trim() != query.call_id || query.call_id.chars().any(char::is_control)
            || query.call_id.starts_with("agent-instructions:") {
            return Err(bad_query("invalid tool detail call identity"));
        }
        let limit = query.limit.unwrap_or(16 * 1024);
        if !(4..=MAX_DETAIL_PAGE_BYTES).contains(&limit) { return Err(bad_query("tool detail limit must be between 4 and 65536 bytes")); }
        let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
        if let Some(cursor) = &cursor {
            if cursor.version != 1 || cursor.session != session.as_ref() || cursor.turn != query.turn_id
                || cursor.call != query.call_id || cursor.section != query.section
                || query.through_seq.is_some_and(|seq| seq != cursor.through_seq)
                || query.expected_result_seq.is_some_and(|seq| Some(seq) != cursor.result_seq) {
                return Err(bad_query("tool detail cursor belongs to another source"));
            }
        }
        let mut tx = self.pool.begin().await?;
        let live = require_live_session_tx(&mut tx, session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::Conflict("AgentSession owner mismatch".into())); }
        let head = head_by_id_tx(&mut tx, session.as_ref()).await?;
        let through = cursor.as_ref().map(|cursor| cursor.through_seq).or(query.through_seq).unwrap_or(head.last_seq);
        if through > head.last_seq { return Err(bad_query("tool detail cursor is ahead of the committed Session")); }
        let turn = sqlx::query_as::<_, Turn>(
            "SELECT operation_id,started_event_id,terminal_event_id,finished_at,state FROM agent_turns WHERE session_id=? AND source_message_id=?")
            .bind(session.as_ref()).bind(&query.turn_id).fetch_optional(&mut *tx).await?
            .ok_or_else(|| SessionStoreError::NotFound("tool detail Turn not found".into()))?;
        let (root, _) = Self::turn_accepted_input_tx(&mut tx, session, &turn.operation_id.as_str().into()).await?;
        if root.event_id.as_ref() != query.turn_id { return Err(invalid("Turn source differs from its accepted root")); }
        let start = event_by_event_id_tx(&mut tx, &turn.started_event_id).await?
            .map(event_from_row).transpose()?.ok_or_else(|| invalid("Turn start is missing"))?;
        let mut terminal_seq = None;
        let mut closed = false;
        if let Some(id) = &turn.terminal_event_id {
            let terminal = event_by_event_id_tx(&mut tx, id).await?
                .map(event_from_row).transpose()?.ok_or_else(|| invalid("Turn terminal is missing"))?;
            let expected = match turn.state.as_str() {
                "completed" => "turn/completed", "failed" => "turn/failed",
                "cancelled" | "interrupted" => "turn/cancelled", _ => return Err(invalid("Turn terminal state differs")),
            };
            if terminal.agent_session_id != *session || terminal.correlation_id.as_ref() != turn.operation_id
                || terminal.kind.0 != expected || terminal.kind_version != 1 || terminal.seq <= start.seq
                || turn.finished_at != Some(as_i64(terminal.seq, "tool detail terminal")?) {
                return Err(invalid("Turn terminal identity differs"));
            }
            closed = terminal.seq <= through;
            terminal_seq = Some(terminal.seq);
        } else if !matches!(turn.state.as_str(), "accepted" | "running") {
            return Err(invalid("closed Turn has no terminal"));
        }
        let rows = sqlx::query_as::<_, StoredEventRow>(
            "SELECT e.session_id,e.seq,e.event_id,e.producer_id,e.idempotency_key,e.kind,e.kind_version,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id \
             FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id \
             WHERE e.session_id=? AND e.kind='runtime/progress-recorded' AND e.producer_id='runtime_supervisor' AND e.correlation_id=? \
               AND e.seq>? AND e.seq<=? \
               AND ((json_extract(COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')),'$.event.event') \
                 IN ('tool_call_completed','tool_started','tool_completed','host_tool_settled','host_tool_dispatch') \
               AND COALESCE(json_extract(COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')),'$.event.call.call_id'), \
                 json_extract(COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')),'$.event.result.call_id'), \
                 json_extract(COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')),'$.event.call_id'), \
                 json_extract(COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')),'$.event.dispatch.call_id'))=?) \
                 OR (e.payload_id IS NOT NULL AND p.payload_id IS NULL)) \
             ORDER BY e.seq LIMIT ?")
            .bind(session.as_ref()).bind(&turn.operation_id).bind(as_i64(start.seq, "tool detail start")?)
            .bind(as_i64(through, "tool detail upper")?).bind(&query.call_id).bind(MAX_DETAIL_IDENTITIES + 1)
            .fetch_all(&mut *tx).await?;
        if rows.len() > MAX_DETAIL_IDENTITIES as usize { return Err(invalid("too many matching invocation identities")); }
        let events = rows.into_iter().map(event_from_row).collect::<Result<Vec<_>, _>>()?;
        let payloads = self.resolved_event_payloads_in_transaction(&mut tx, session,
            &events.iter().map(|event| event.event_id.clone()).collect::<Vec<_>>()).await?;
        let mut proposal = None;
        let mut result: Option<(u64, EngineToolResult)> = None;
        let mut action_identity = None;
        for event in events {
            if event.kind_version != 1 || !matches!(event.causation_event_id.as_ref(), Some(id) if id == &root.event_id || id == &start.event_id) {
                return Err(invalid("observation differs from its exact Turn causation"));
            }
            let value = payloads.get(event.event_id.as_ref()).and_then(|payload| payload.get("event"))
                .ok_or_else(|| invalid("observation payload is missing"))?;
            match serde_json::from_value::<Observation>(value.clone())? {
                Observation::ToolCallCompleted { step, call } => {
                    if terminal_seq.is_some_and(|terminal| event.seq >= terminal) { return Err(invalid("proposal follows its Turn terminal")); }
                    if proposal.is_some() || call.call_id.as_ref() != query.call_id || !call.arguments.0.is_object() {
                        return Err(invalid("invocation proposal is ambiguous"));
                    }
                    if call.name == "finish_without_reply" { return Err(SessionStoreError::NotFound("internal tool detail is unavailable".into())); }
                    proposal = Some((event.event_id, step, call));
                }
                Observation::ToolStarted { step, call_id, capability_id, action_id } => {
                    if terminal_seq.is_some_and(|terminal| event.seq >= terminal) { return Err(invalid("admission follows its Turn terminal")); }
                    let Some((_, original_step, _)) = &proposal else { return Err(invalid("admission precedes its proposal")); };
                    if step != *original_step || call_id.as_ref() != query.call_id { return Err(invalid("admission differs from its proposal")); }
                    observe_action(&mut action_identity, capability_id, action_id)?;
                }
                Observation::ToolCompleted { step, result: output } => {
                    if terminal_seq.is_some_and(|terminal| event.seq >= terminal) { return Err(invalid("runtime result follows its Turn terminal")); }
                    let Some((_, original_step, _)) = &proposal else { return Err(invalid("result precedes its proposal")); };
                    if step != *original_step || output.call_id.as_ref() != query.call_id { return Err(invalid("result differs from its proposal")); }
                    result = Some((event.seq, output));
                }
                Observation::HostToolSettled { call_id, result: output } => {
                    // User Stop commits its cancellation fence before the
                    // original owner settles. That exact late receipt remains
                    // readable; later process cumulative output is not selected.
                    if proposal.is_none() || call_id.as_ref() != query.call_id || output.call_id != call_id {
                        return Err(invalid("host result differs from its invocation"));
                    }
                    result = Some((event.seq, output));
                }
                Observation::HostToolDispatch { dispatch } => {
                    if proposal.is_none() || dispatch.call_id.as_ref() != query.call_id
                        || terminal_seq.is_some_and(|terminal| event.seq >= terminal) {
                        return Err(invalid("host dispatch differs from its invocation"));
                    }
                    observe_action(&mut action_identity, dispatch.capability_id, dispatch.action_id)?;
                }
            }
        }
        let (call_event, _, call) = proposal.ok_or_else(|| SessionStoreError::NotFound("tool invocation not found in this Turn".into()))?;
        let result_seq = result.as_ref().map(|(seq, _)| *seq);
        if query.expected_result_seq.is_some_and(|seq| Some(seq) != result_seq) {
            return Err(SessionStoreError::Conflict("tool detail result differs from the displayed observation".into()));
        }
        let mut retention = None;
        let mut working_directory = None;
        let (content, format, availability, non_text_parts) = match query.section {
            Section::Arguments => {
                let script = action_identity.as_ref().and_then(|(capability, action)| match (capability.as_str(), action.as_str()) {
                    ("workspace.process", "workspace.process/start") => call.arguments.0.get("cmd"),
                    ("ssh", "ssh/exec" | "ssh/sudo") => call.arguments.0.get("command"),
                    _ => None,
                }).and_then(Value::as_str);
                working_directory = script.and_then(|_| call.arguments.0.get("cwd")).and_then(Value::as_str).map(str::to_owned);
                match script {
                    Some(script) => (script.to_owned(), Format::Text, Availability::Available, 0),
                    None => (serde_json::to_string_pretty(&call.arguments.0)?, Format::Json, Availability::Available, 0),
                }
            },
            Section::Output => match result {
                Some((_, output)) => {
                    let non_text = output.output.iter().filter(|part| !matches!(part, ChatToolResultPart::Text { .. })).count() as u32;
                    let mut text = output.output_text();
                    // Formatting only, after exact native admission. Preserve
                    // generic/MCP JSON verbatim; never infer its execution status.
                    let native_process = action_identity.as_ref().is_some_and(|(capability, action)| capability == "workspace.process"
                        && matches!(action.as_str(), "workspace.process/start" | "workspace.process/poll"));
                    if native_process && output.output.len() == 1 {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if let Some(original) = value.pointer("/output/text").and_then(Value::as_str) { text = original.to_owned(); }
                            if let Some(output) = value.get("output").and_then(|output|
                                serde_json::from_value::<nomifun_engine_core::EngineProcessOutput>(output.clone()).ok()) {
                                retention = Some(ToolDetailRetention {next_cursor:output.next_cursor,retained_bytes:output.retained_bytes as u64,
                                    dropped_bytes:output.dropped_bytes,source_encoding:output.source_encoding,decode_errors:output.decode_errors});
                            }
                        }
                    }
                    (text, Format::Text, Availability::Available, non_text)
                }
                None => (String::new(), Format::Text, if closed { Availability::Unconfirmed } else { Availability::Pending }, 0),
            },
        };
        let digest = digest_bytes(content.as_bytes());
        let offset = cursor.as_ref().map_or(0, |cursor| cursor.offset);
        if let Some(cursor) = &cursor {
            if cursor.call_event != call_event.as_ref() || cursor.result_seq != result_seq || cursor.digest != digest {
                return Err(SessionStoreError::Conflict("tool detail cursor no longer matches its immutable source".into()));
            }
        }
        let begin = usize::try_from(offset).map_err(|_| bad_query("tool detail offset overflows"))?;
        if begin > content.len() || !content.is_char_boundary(begin) { return Err(bad_query("invalid tool detail UTF-8 offset")); }
        let mut end = content.len().min(begin.saturating_add(limit as usize));
        while !content.is_char_boundary(end) { end -= 1; }
        let eof = end == content.len();
        let next_cursor = if eof { None } else { Some(encode_cursor(&Cursor {
            version: 1, session: session.as_ref().into(), turn: query.turn_id.clone(), call: query.call_id.clone(),
            call_event: call_event.as_ref().into(), result_seq, through_seq: through, section: query.section,
            digest, offset: end as u64,
        })?) };
        let page = ToolDetailPage { turn_id: query.turn_id.clone(), call_id: query.call_id.clone(), call_event_id: call_event.0,
            through_seq: through, result_seq, section: query.section, availability, content: content[begin..end].into(), format,
            offset, total_bytes: content.len() as u64, next_cursor, eof, non_text_parts, retention, working_directory };
        tx.commit().await?;
        Ok(page)
    }
}

fn decode_cursor(value: &str) -> Result<Cursor, SessionStoreError> {
    if value.len() > 4096 { return Err(bad_query("tool detail cursor is too long")); }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value).map_err(|_| bad_query("invalid tool detail cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| bad_query("invalid tool detail cursor"))
}

fn encode_cursor(cursor: &Cursor) -> Result<String, SessionStoreError> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor)?))
}
