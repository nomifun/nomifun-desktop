//! Rebuildable timeline references, read in the same snapshot as source messages.
//! No text, groups, or UI disclosure state is written back to canonical storage.
use super::*;
use nomifun_agent_contracts::{MessageTimeline, MessageTimelineSegment, MessageTimelineUpdate,
    TimelineSegmentKind as Kind, TimelineSegmentState as State};

const MAX_TIMELINE_EVENTS: usize = 1_000_000;

#[derive(sqlx::FromRow)]
pub(super) struct TimelineTurn {
    operation_id: String,
    source_message_id: String,
    started_event_id: String,
    first_seq: i64,
    last_seq: Option<i64>,
    state: String,
}

/// A bounded field selection from the original event, not another persisted
/// event vocabulary. Unselected message bodies never leave SQLite on paging.
#[derive(Debug, sqlx::FromRow)]
struct Fact {
    seq: i64,
    event_id: String,
    kind: String,
    kind_version: i64,
    producer_id: String,
    correlation_id: String,
    causation_event_id: Option<String>,
    source_root: Option<String>,
    content: Option<String>,
    text_source_json: Option<String>,
    has_content: i64,
    runtime_kind: Option<String>,
    runtime_step: Option<i64>,
    call_id: Option<String>,
    result_error: Option<i64>,
    tool_name: Option<String>,
}

fn invalid(detail: &str) -> SessionStoreError {
    SessionStoreError::InvalidEvent(format!("canonical message timeline: {detail}"))
}

#[derive(Debug)]
struct Source {
    segments: Vec<MessageTimelineSegment>,
    offset: u64,
    reasoning_step: Option<i64>,
}
impl Source {
    fn text() -> Self { Self { segments: Vec::new(), offset: 0, reasoning_step: None } }
}

#[derive(Debug)]
struct Tool {
    segment: MessageTimelineSegment,
    owner: Option<String>,
    visible: bool,
    step: Option<i64>,
}

struct Timeline {
    root: String,
    operation: String,
    started_event: String,
    group: String,
    sources: BTreeMap<String, Source>,
    tools: BTreeMap<String, Tool>,
    narration_active: Option<(String, usize)>,
    reasoning_active: Option<(String, usize)>,
    activity: Option<(Kind, Option<i64>)>,
    // The active narration anchor is needed even when paging did not select
    // its body. It is always the first part event, never the current page edge.
    narration: Option<(String, String, Option<nomifun_agent_contracts::ChatOutputTextSource>)>,
    reasoning_start: Option<(String, u64)>,
    withdrawn: BTreeSet<String>,
    internal_silent_finishes: BTreeSet<String>,
}

impl Timeline {
    fn new(turn: &TimelineTurn) -> Self {
        Self { root: turn.source_message_id.clone(), operation: turn.operation_id.clone(), started_event: turn.started_event_id.clone(),
            group: format!("process:{}", turn.source_message_id), sources: BTreeMap::new(),
            tools: BTreeMap::new(), narration_active: None, reasoning_active: None, activity: None, narration: None,
            reasoning_start: None, withdrawn: BTreeSet::new(), internal_silent_finishes: BTreeSet::new() }
    }

    fn settle_active(&mut self, active: Option<(String, usize)>, seq: u64, state: State) {
        if let Some((source, index)) = active
            && let Some(segment) = self.sources.get_mut(&source).and_then(|source| source.segments.get_mut(index))
            && segment.state == State::Running {
            segment.state = state;
            segment.last_seq = segment.last_seq.max(seq);
        }
    }

    fn close_reasoning(&mut self, seq: u64, state: State) {
        let active = self.reasoning_active.take();
        self.settle_active(active, seq, state);
        self.reasoning_start = None;
    }

    fn close(&mut self, seq: u64, state: State) {
        let active = self.narration_active.take();
        self.settle_active(active, seq, state);
        self.close_reasoning(seq, state);
        self.activity = None;
        self.narration = None;
    }

    fn activity(&mut self, kind: Kind, step: Option<i64>, seq: u64) {
        if self.activity.is_some_and(|previous| previous.1.zip(step).is_some_and(|(old, new)| old != new)) {
            self.close(seq, State::Completed);
        } else if self.activity.is_some_and(|previous| previous.0 != kind) {
            // Activity transitions end a reasoning disclosure, not a body.
            // Providers may interleave reasoning with a single text block or
            // deliver an atomic reasoning block after that body has started.
            self.close_reasoning(seq, State::Completed);
        }
        let step = step.or_else(|| self.activity.filter(|previous| previous.0 == kind).and_then(|previous| previous.1));
        self.activity = Some((kind, step));
    }

    fn text(&mut self, fact: &Fact, kind: Kind) -> Result<(), SessionStoreError> {
        if fact.source_root.as_deref() != Some(self.root.as_str()) { return Ok(()); }
        if kind == Kind::Narration && self.withdrawn.contains(&fact.correlation_id) { return Ok(()); }
        let seq = as_u64(fact.seq, "timeline text sequence")?;
        let text_source = fact.text_source_json.as_deref().map(serde_json::from_str::<nomifun_agent_contracts::ChatOutputTextSource>)
            .transpose().map_err(|_| invalid("invalid typed text source"))?;
        if let Some(source) = &text_source { source.validate().map_err(|_| invalid("invalid typed text source identity"))?; }
        if kind == Kind::Narration && fact.has_content != 0 {
            let same = self.narration.as_ref().is_some_and(|(source, _, identity)|
                source == &fact.correlation_id && identity == &text_source);
            if !same {
                let activity = self.activity;
                self.close(seq, State::Completed);
                self.activity = activity;
                self.narration = Some((fact.correlation_id.clone(), fact.event_id.clone(), text_source));
            }
        }
        let Some(text) = &fact.content else {
            if kind == Kind::Narration && let Some((_, anchor, _)) = &self.narration {
                self.group = format!("process:{anchor}");
            }
            return Ok(());
        };
        let activity = self.activity.filter(|activity| activity.0 == kind);
        let source = self.sources.entry(fact.correlation_id.clone()).or_insert_with(Source::text);
        if kind == Kind::Reasoning { source.reasoning_step = activity.and_then(|activity| activity.1); }
        let start = source.offset;
        let units = text.chars().map(|character| character.len_utf16() as u64).sum::<u64>();
        source.offset = start.checked_add(units).ok_or_else(|| invalid("UTF-16 range overflow"))?;
        if text.is_empty() { return Ok(()); }
        let active = if kind == Kind::Narration { &self.narration_active } else { &self.reasoning_active };
        let continuing = active.as_ref().filter(|(id, _)| id == &fact.correlation_id).map(|(_, index)| *index);
        if let Some(index) = continuing {
            let segment = &mut source.segments[index];
            if segment.kind != kind || segment.end_utf16 != Some(start) { return Err(invalid("noncontiguous source segment")); }
            segment.last_seq = seq;
            segment.end_utf16 = Some(source.offset);
        } else {
            if kind == Kind::Reasoning {
                let active = self.reasoning_active.take();
                self.settle_active(active, seq, State::Completed);
            }
            let source = self.sources.get_mut(&fact.correlation_id).expect("source inserted above");
            let index = source.segments.len();
            source.segments.push(MessageTimelineSegment {
                segment_id: fact.event_id.clone(), group_id: self.group.clone(), first_seq: seq, last_seq: seq,
                kind, state: if kind == Kind::Reasoning && activity.and_then(|activity| activity.1).is_none() {
                    State::Completed
                } else { State::Running },
                start_utf16: Some(start), end_utf16: Some(source.offset), call_id: None,
            });
            if kind == Kind::Narration {
                self.narration_active = Some((fact.correlation_id.clone(), index));
            } else {
                self.reasoning_active = Some((fact.correlation_id.clone(), index));
            }
        }
        if kind == Kind::Narration && let Some((_, anchor, _)) = &self.narration {
            self.group = format!("process:{anchor}");
        }
        Ok(())
    }

    fn tool(&mut self, fact: &Fact, owner: bool, visible: bool) -> Result<(), SessionStoreError> {
        let Some(call_id) = &fact.call_id else { return Err(invalid("tool identity missing")); };
        if call_id.starts_with("agent-instructions:") || self.internal_silent_finishes.contains(call_id) { return Ok(()); }
        if owner && fact.causation_event_id.as_deref() != Some(self.started_event.as_str()) { return Ok(()); }
        if self.tools.get(call_id).is_some_and(|tool| tool.step.zip(fact.runtime_step).is_some_and(|(original,current)| original != current)) {
            return Ok(());
        }
        let seq = as_u64(fact.seq, "timeline tool sequence")?;
        // Call starts split text runs, while result/settlement observations
        // below never do. A later result cannot move the original call's group.
        if !self.tools.contains_key(call_id) { self.close(seq, State::Completed); }
        let tool = self.tools.entry(call_id.clone()).or_insert_with(|| Tool {
            segment: MessageTimelineSegment {
                segment_id: fact.event_id.clone(), group_id: self.group.clone(), first_seq: seq, last_seq: seq,
                kind: Kind::Tool, state: State::Running, start_utf16: None, end_utf16: None, call_id: Some(call_id.clone()),
            }, owner: None, visible: false, step: fact.runtime_step,
        });
        if tool.step.is_none() { tool.step = fact.runtime_step; }
        if owner {
            if tool.owner.as_ref().is_some_and(|previous| previous != &fact.correlation_id) {
                return Err(invalid("tool changed its owner projection"));
            }
            tool.owner = Some(fact.correlation_id.clone());
        }
        tool.visible |= visible;
        tool.segment.last_seq = seq;
        Ok(())
    }

    fn result(&mut self, fact: &Fact) -> Result<(), SessionStoreError> {
        let Some(call_id) = &fact.call_id else { return Err(invalid("result identity missing")); };
        if self.internal_silent_finishes.contains(call_id) { return Ok(()); }
        let Some(tool) = self.tools.get_mut(call_id) else { return Ok(()); };
        if tool.step.zip(fact.runtime_step).is_some_and(|(original,current)| original != current) { return Ok(()); }
        if fact.kind == "tool/result-recorded" && tool.owner.as_deref() != Some(fact.correlation_id.as_str()) {
            return Ok(());
        }
        tool.segment.last_seq = as_u64(fact.seq, "timeline result sequence")?;
        if let Some(is_error) = fact.result_error {
            tool.segment.state = if is_error == 0 { State::Completed } else { State::Failed };
            tool.visible = true;
        }
        Ok(())
    }

    fn accept(&mut self, fact: &Fact) -> Result<(), SessionStoreError> {
        // Runtime lifecycle is owned by the formal supervisor. An event from
        // another producer cannot close/reopen an activity or settle its tool.
        if fact.kind == "runtime/progress-recorded" && fact.producer_id != "runtime_supervisor" {
            return Ok(());
        }
        if fact.kind_version != 1 { return Err(invalid("unsupported canonical event version")); }
        if fact.kind=="runtime/progress-recorded" && fact.correlation_id==self.operation {
            if fact.tool_name.as_deref()==Some("finish_without_reply")
                && matches!(fact.runtime_kind.as_deref(),Some("tool_call_delta"|"tool_call_completed")) {
                if let Some(call)=&fact.call_id { self.internal_silent_finishes.insert(call.clone()); }
                return Ok(());
            }
            if fact.call_id.as_ref().is_some_and(|call|self.internal_silent_finishes.contains(call))
                && matches!(fact.runtime_kind.as_deref(),Some("tool_call_delta"|"tool_call_completed"|"tool_started"|"tool_completed"|"host_tool_settled")) {
                return Ok(());
            }
        }
        let seq = as_u64(fact.seq, "timeline sequence")?;
        match fact.kind.as_str() {
            "message/content-part" => self.text(fact, Kind::Narration)?,
            "thinking/content-part" => self.text(fact, Kind::Reasoning)?,
            "message/completed" => {
                if self.narration_active.as_ref().is_some_and(|(source, _)| source == &fact.correlation_id) {
                    self.close(seq, State::Completed);
                }
            }
            "tool/call-started" => self.tool(fact, true, true)?,
            "tool/result-recorded" => self.result(fact)?,
            "turn/steer-accepted" => {
                self.close(seq, State::Completed);
                self.group = format!("process:{}", fact.event_id);
            }
            "runtime/progress-recorded" if fact.correlation_id == self.operation => {
                match fact.runtime_kind.as_deref() {
                    Some("reasoning_delta") => {
                        self.activity(Kind::Reasoning, fact.runtime_step, seq);
                        if self.reasoning_start.is_none() { self.reasoning_start = Some((fact.event_id.clone(), seq)); }
                    }
                    Some("output_text_delta") => self.activity(Kind::Narration, fact.runtime_step, seq),
                    Some("completion_delivered") => {
                        self.close(seq, State::Completed);
                        self.activity(Kind::Narration, fact.runtime_step, seq);
                    }
                    Some("tool_call_completed") => self.tool(fact, false, false)?,
                    Some("tool_started") => self.tool(fact, false, true)?,
                    Some("tool_completed" | "host_tool_settled") => self.result(fact)?,
                    Some("voice_model_step_superseded") => {
                        self.close(seq, State::Completed);
                        if let Some(step) = fact.runtime_step.and_then(|step| u16::try_from(step).ok()) {
                            let source = nomifun_agent_contracts::canonical_assistant_step_message_id(&self.root, step)
                                .map_err(SessionStoreError::InvalidEvent)?;
                            self.withdrawn.insert(source.clone());
                            self.sources.entry(source).or_insert_with(Source::text).segments.clear();
                        }
                    }
                    Some("model_step_started" | "execution_resumed" | "tool_call_delta" | "model_output_truncated"
                        | "model_response_rejected" | "delivery_review_superseded"
                        | "user_input_requested") => self.close(seq, State::Completed),
                    Some("silent_completion" | "turn_completed" | "turn_input_required") => self.close(seq, State::Completed),
                    Some("turn_failed") => self.close(seq, State::Failed),
                    Some("turn_cancelled") => self.close(seq, State::Cancelled),
                    Some("turn_paused") => self.close(seq, State::Interrupted),
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(mut self, turn: &TimelineTurn, through: u64) -> Result<Vec<MessageTimelineUpdate>, SessionStoreError> {
        if self.reasoning_active.is_none() && let Some((Kind::Reasoning, Some(step))) = self.activity
            && let Some((event_id, seq)) = &self.reasoning_start
            && let Some((source_id, source)) = self.sources.iter_mut().find(|(_, source)| source.reasoning_step == Some(step)) {
            // The journal may preserve an empty activity start after its bounded
            // reasoning body budget is exhausted. A zero-length reference
            // expresses that real activity without reusing an earlier body.
            let index = source.segments.len();
            source.segments.push(MessageTimelineSegment {
                segment_id: event_id.clone(), group_id: self.group.clone(), first_seq: *seq, last_seq: *seq,
                kind: Kind::Reasoning, state: State::Running,
                start_utf16: Some(source.offset), end_utf16: Some(source.offset), call_id: None,
            });
            self.reasoning_active = Some((source_id.clone(), index));
        }
        if let Some(end) = turn.last_seq.filter(|end| *end >= 0 && *end as u64 <= through) {
            self.close(as_u64(end, "timeline Turn end")?, match turn.state.as_str() {
                "completed" => State::Completed, "failed" => State::Failed,
                "cancelled" => State::Cancelled, _ => State::Interrupted,
            });
            for tool in self.tools.values_mut().filter(|tool| tool.segment.state == State::Running) {
                // Missing result is an interrupted observation, never inferred
                // tool success or failure from the enclosing Turn's outcome.
                tool.segment.state = State::Interrupted;
                tool.segment.last_seq = tool.segment.last_seq.max(end as u64);
            }
        }
        let mut updates = self.sources.into_iter().map(|(source, value)| MessageTimelineUpdate {
            source_message_id: source,
            call_id: None,
            timeline: MessageTimeline { through_seq: through, segments: value.segments },
        }).collect::<Vec<_>>();
        for (call_id, tool) in self.tools.into_iter().filter(|(_, tool)| tool.visible) {
            updates.push(MessageTimelineUpdate {
                source_message_id: tool.owner.unwrap_or_else(|| self.root.clone()), call_id: Some(call_id),
                timeline: MessageTimeline { through_seq: through, segments: vec![tool.segment] },
            });
        }
        Ok(updates)
    }
}

pub(super) async fn for_root(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, root: &str, through: u64)
    -> Result<Vec<MessageTimelineUpdate>, SessionStoreError> {
    let turn = sqlx::query_as::<_, TimelineTurn>(
        "SELECT operation_id,source_message_id,started_event_id,COALESCE(started_at,accepted_at) AS first_seq,finished_at AS last_seq,state \
         FROM agent_turns WHERE session_id=? AND source_message_id=?")
        .bind(session.as_ref()).bind(root).fetch_optional(&mut **tx).await?;
    match turn { Some(turn) => read_turn(tx, session, &turn, through, None).await, None => Ok(Vec::new()) }
}

async fn read_turn(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, turn: &TimelineTurn,
    through: u64, selected: Option<&BTreeSet<String>>) -> Result<Vec<MessageTimelineUpdate>, SessionStoreError> {
    if turn.first_seq < 0 || turn.last_seq.is_some_and(|end| end < turn.first_seq)
        || turn.operation_id.is_empty() || turn.source_message_id.is_empty() {
        return Err(invalid("invalid owning Turn boundary"));
    }
    let mut query = sqlx::QueryBuilder::<Sqlite>::new(
        "WITH facts AS (SELECT e.seq,e.event_id,e.kind,e.kind_version,e.producer_id,e.correlation_id,e.causation_event_id, \
         COALESCE(e.inline_json,json_extract(CAST(p.body AS TEXT),'$.value')) AS fact \
         FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id \
         WHERE e.session_id=");
    let upper = turn.last_seq.unwrap_or(as_i64(through, "timeline head")?).min(as_i64(through, "timeline head")?);
    query.push_bind(session.as_ref()).push(" AND e.seq>=").push_bind(turn.first_seq)
        .push(" AND e.seq<=").push_bind(upper)
        .push(" AND e.kind IN ('message/content-part','thinking/content-part','message/completed', \
        'tool/call-started','tool/result-recorded','runtime/progress-recorded','turn/steer-accepted')) \
        SELECT seq,event_id,kind,kind_version,producer_id,correlation_id,causation_event_id,json_extract(fact,'$.turn_id') AS source_root, CASE WHEN ");
    if let Some(selected) = selected {
        query.push("correlation_id IN (");
        let mut ids = query.separated(",");
        for id in selected { ids.push_bind(id); }
        ids.push_unseparated(")");
    } else { query.push("1"); }
    query.push(" THEN json_extract(fact,'$.content') END AS content, json_extract(fact,'$.source') AS text_source_json, \
        COALESCE(length(json_extract(fact,'$.content'))>0,0) AS has_content, \
        json_extract(fact,'$.event.event') AS runtime_kind,json_extract(fact,'$.event.step') AS runtime_step, \
        COALESCE(json_extract(fact,'$.call_id'),json_extract(fact,'$.event.call.call_id'), \
        json_extract(fact,'$.event.call_id'),json_extract(fact,'$.event.result.call_id')) AS call_id, \
        CASE WHEN kind='tool/result-recorded' AND json_type(fact,'$.error') IS NOT NULL \
                  AND json_type(fact,'$.error')<>'null' THEN 1 \
             WHEN json_type(fact,'$.event.result.is_error') IN ('true','false') \
             THEN json_extract(fact,'$.event.result.is_error') \
             WHEN json_type(fact,'$.output.is_error') IN ('true','false') \
             THEN json_extract(fact,'$.output.is_error') \
             WHEN kind='tool/result-recorded' AND json_type(fact,'$.output') IS NOT NULL \
             THEN 0 END AS result_error, \
        COALESCE(json_extract(fact,'$.event.call.name'),json_extract(fact,'$.event.name')) AS tool_name FROM facts ORDER BY seq LIMIT ")
        .push_bind((MAX_TIMELINE_EVENTS + 1) as i64);
    let facts = query.build_query_as::<Fact>().fetch_all(&mut **tx).await?;
    if facts.len() > MAX_TIMELINE_EVENTS { return Err(invalid("event budget exceeded")); }
    let mut timeline = Timeline::new(turn);
    for fact in facts { timeline.accept(&fact)?; }
    timeline.finish(turn, through)
}

pub(super) async fn hydrate(tx: &mut Transaction<'_, Sqlite>, session: &AgentSessionId, through: u64,
    rows: &mut [MessageProjection]) -> Result<(), SessionStoreError> {
    let selected = rows.iter().filter(|row| matches!(row.presentation_intent.as_str(), "message" | "thinking"))
        .filter_map(|row| row.projection.get("correlation_id").and_then(Value::as_str).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let relevant = rows.iter().filter(|row| matches!(row.presentation_intent.as_str(), "message" | "thinking" | "tool"))
        .filter(|row| row.presentation_intent == "tool" || row.projection.get("turn_id").and_then(Value::as_str).is_some())
        .collect::<Vec<_>>();
    if relevant.is_empty() { return Ok(()); }
    let mut query = sqlx::QueryBuilder::<Sqlite>::new(
        "SELECT operation_id,source_message_id,started_event_id,COALESCE(started_at,accepted_at) AS first_seq,finished_at AS last_seq,state \
         FROM agent_turns WHERE session_id=");
    query.push_bind(session.as_ref()).push(" AND source_message_id IS NOT NULL AND (");
    {
        let mut conditions = query.separated(" OR ");
        for row in relevant {
            conditions.push("(COALESCE(started_at,accepted_at)<=").push_bind_unseparated(as_i64(row.first_seq,"timeline source")?)
                .push_unseparated(" AND COALESCE(finished_at,").push_bind_unseparated(as_i64(through,"timeline head")?)
                .push_unseparated(")>=").push_bind_unseparated(as_i64(row.first_seq,"timeline source")?).push_unseparated(")");
        }
    }
    query.push(")");
    for turn in query.build_query_as::<TimelineTurn>().fetch_all(&mut **tx).await? {
        let updates = read_turn(tx, session, &turn, through, Some(&selected)).await?;
        for row in rows.iter_mut().filter(|row| row.first_seq >= turn.first_seq as u64
            && row.first_seq <= turn.last_seq.unwrap_or(through as i64) as u64) {
            if matches!(row.presentation_intent.as_str(), "message" | "thinking")
                && row.projection.get("turn_id").and_then(Value::as_str)
                    .is_some_and(|root| root != turn.source_message_id) {
                return Err(invalid("source projection differs from its exact canonical Turn"));
            }
            let id = row.projection.get("correlation_id").and_then(Value::as_str);
            let call = row.projection.pointer("/tool_summary/call_id").and_then(Value::as_str);
            let update = updates.iter().find(|update| match &update.call_id {
                Some(call_id) => row.presentation_intent == "tool" && call == Some(call_id.as_str()),
                None => row.presentation_intent != "tool" && id == Some(update.source_message_id.as_str()),
            });
            if let Some(update) = update {
                if update.call_id.is_none() {
                    let body = row.projection.get("content").and_then(Value::as_str)
                        .ok_or_else(|| invalid("referenced source body missing"))?;
                    let length = body.chars().map(|character| character.len_utf16() as u64).sum::<u64>();
                    if update.timeline.segments.iter().any(|segment| match (segment.start_utf16, segment.end_utf16) {
                        (Some(start), Some(end)) => start > end || end > length,
                        _ => true,
                    }) { return Err(invalid("text range exceeds its returned source body")); }
                }
                row.projection["timeline"] = serde_json::to_value(&update.timeline)?;
                if row.presentation_intent == "thinking" {
                    row.projection["state"] = json!(if update.timeline.segments.last()
                        .is_some_and(|segment| segment.state == State::Running) { "streaming" } else { "recorded" });
                }
                // This remains the canonical source fingerprint. IDMM uses
                // that exact value for its question CAS; read-only timeline
                // and lifecycle metadata must never create a new authority.
            }
        }
    }
    if rows.iter().any(|row| row.presentation_intent == "thinking"
        && row.projection.get("timeline").is_none()) {
        return Err(invalid("reasoning source has no exact canonical Turn"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROOT: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    fn turn() -> TimelineTurn {
        TimelineTurn { operation_id: "turn-operation".into(), source_message_id: ROOT.into(), started_event_id: "start".into(), first_seq: 1, last_seq: None, state: "running".into() }
    }
    fn fact(seq: i64, kind: &str, source: &str, content: Option<&str>) -> Fact {
        Fact { seq, event_id: format!("event-{seq}"), kind: kind.into(), correlation_id: source.into(),
            kind_version: 1, producer_id: "runtime_supervisor".into(),
            causation_event_id: Some("start".into()),
            source_root: Some(ROOT.into()), content: content.map(str::to_owned), text_source_json: None, has_content: i64::from(content.is_some_and(|text| !text.is_empty())),
            runtime_kind: None, runtime_step: None, call_id: None, result_error: None, tool_name: None }
    }
    fn runtime(seq: i64, name: &str, step: i64) -> Fact {
        Fact { runtime_kind: Some(name.into()), runtime_step: Some(step), ..fact(seq, "runtime/progress-recorded", "turn-operation", None) }
    }
    fn tool(seq: i64, name: &str, call: &str) -> Fact {
        Fact { call_id: Some(call.into()), ..runtime(seq, name, 1) }
    }
    fn ordered(updates: &[MessageTimelineUpdate]) -> Vec<&MessageTimelineSegment> {
        let mut segments = updates.iter().flat_map(|update| &update.timeline.segments).collect::<Vec<_>>();
        segments.sort_by_key(|segment| segment.first_seq); segments
    }
    fn source<'a>(updates: &'a [MessageTimelineUpdate], id: &str) -> &'a MessageTimeline {
        &updates.iter().find(|update| update.source_message_id == id && update.call_id.is_none()).unwrap().timeline
    }

    #[test]
    fn interleaved_same_step_reasoning_and_tools_retain_four_exact_segments() {
        let mut timeline = Timeline::new(&turn());
        for event in [runtime(2,"reasoning_delta",1), fact(3,"thinking/content-part","thought",Some("先看")),
            tool(4,"tool_call_completed","one"), tool(5,"tool_started","one"), runtime(6,"reasoning_delta",1),
            fact(7,"thinking/content-part","thought",Some("再查😀")), tool(8,"tool_call_completed","two"), tool(9,"tool_started","two")] {
            timeline.accept(&event).unwrap();
        }
        let updates = timeline.finish(&turn(),9).unwrap();
        let segments = ordered(&updates);
        assert_eq!(segments.iter().map(|segment| segment.kind).collect::<Vec<_>>(), [Kind::Reasoning,Kind::Tool,Kind::Reasoning,Kind::Tool]);
        assert_eq!((segments[0].start_utf16,segments[0].end_utf16),(Some(0),Some(2)));
        assert_eq!((segments[2].start_utf16,segments[2].end_utf16),(Some(2),Some(6)));
        assert_eq!(segments[0].segment_id,"event-3");
        assert_eq!(segments[2].segment_id,"event-7");
        assert!(segments.iter().all(|segment| segment.group_id == format!("process:{ROOT}")));
    }

    #[test]
    fn silent_finish_control_never_splits_public_text_or_reveals_its_result_as_a_tool() {
        for failed in [0,1] {
            let mut timeline=Timeline::new(&turn());
            for event in [
                fact(2,"message/content-part","answer",Some("A")),
                Fact {tool_name:Some("finish_without_reply".into()),..tool(3,"tool_call_delta","quiet-finish")},
                Fact {tool_name:Some("finish_without_reply".into()),..tool(4,"tool_call_completed","quiet-finish")},
                Fact {tool_name:None,result_error:Some(failed),..tool(5,"tool_completed","quiet-finish")},
                fact(6,"message/content-part","answer",Some("B")),
                Fact {tool_name:Some("robot_dog_action".into()),..tool(7,"tool_call_completed","motion")},
                tool(8,"tool_started","motion"),
                Fact {result_error:Some(1),..tool(9,"tool_completed","motion")},
                Fact {tool_name:Some("report_completion".into()),..tool(10,"tool_call_completed","report")},
                Fact {result_error:Some(0),..tool(11,"tool_completed","report")},
            ] {timeline.accept(&event).unwrap();}
            let updates=timeline.finish(&turn(),11).unwrap();
            assert!(!updates.iter().any(|update|update.call_id.as_deref()==Some("quiet-finish")));
            let answer=source(&updates,"answer");assert_eq!(answer.segments.len(),1);assert_eq!(answer.segments[0].end_utf16,Some(2));
            assert_eq!(updates.iter().find(|update|update.call_id.as_deref()==Some("motion")).unwrap().timeline.segments[0].state,State::Failed);
            assert!(updates.iter().any(|update|update.call_id.as_deref()==Some("report")),"other controls remain inspectable");
        }
    }

    #[test]
    fn narration_identity_survives_interleaved_runtime_and_atomic_reasoning() {
        for (prefix, suffix) in [("我将", "按照飞书官方文档的步骤安装。先安装 CLI："),
            ("**CL", "I** 和 [SKILL](https://example.test) 都已安装成功。😀")] {
            let mut timeline = Timeline::new(&turn());
            for event in [runtime(2,"output_text_delta",1),fact(3,"message/content-part","answer",Some(prefix)),
                runtime(4,"reasoning_delta",1),fact(5,"thinking/content-part","thought",Some("inspect")),
                runtime(6,"output_text_delta",1),fact(7,"message/content-part","answer",Some(suffix)),
                fact(8,"thinking/content-part","late-thought",Some("atomic late reasoning"))] {
                timeline.accept(&event).unwrap();
            }
            let updates = timeline.finish(&turn(),8).unwrap();
            let segments = &source(&updates,"answer").segments;
            assert_eq!(segments.len(),1,"reasoning delivery never creates a text boundary");
            assert_eq!(segments[0].segment_id,"event-3");
            assert_eq!((segments[0].start_utf16,segments[0].end_utf16),
                (Some(0),Some(format!("{prefix}{suffix}").encode_utf16().count() as u64)));
            assert_eq!(segments[0].state,State::Running);
            assert_eq!(source(&updates,"late-thought").segments[0].state,State::Completed,
                "late reasoning bytes do not reopen a runtime activity");
        }
    }

    #[test]
    fn open_reasoning_and_narration_have_independent_lifecycles() {
        let mut timeline = Timeline::new(&turn());
        for event in [runtime(2,"output_text_delta",1),fact(3,"message/content-part","answer",Some("CL")),
            runtime(4,"reasoning_delta",1),fact(5,"thinking/content-part","thought",Some("inspect"))] {
            timeline.accept(&event).unwrap();
        }
        let updates = timeline.finish(&turn(),5).unwrap();
        assert_eq!(source(&updates,"answer").segments[0].state,State::Running);
        assert_eq!(source(&updates,"thought").segments[0].state,State::Running);
        assert_eq!(source(&updates,"answer").segments[0].last_seq,3,
            "reasoning activity does not change a body range or its source cursor");
    }

    #[test]
    fn terminal_activity_settles_both_open_content_channels() {
        for (event,state) in [("turn_failed",State::Failed),("turn_cancelled",State::Cancelled),
            ("turn_paused",State::Interrupted),("turn_completed",State::Completed),
            ("silent_completion",State::Completed)] {
            let mut timeline = Timeline::new(&turn());
            for event in [runtime(2,"output_text_delta",1),fact(3,"message/content-part","answer",Some("CL")),
                runtime(4,"reasoning_delta",1),fact(5,"thinking/content-part","thought",Some("inspect")),runtime(6,event,1)] {
                timeline.accept(&event).unwrap();
            }
            let updates = timeline.finish(&turn(),6).unwrap();
            for id in ["answer","thought"] {
                assert_eq!(source(&updates,id).segments[0].state,state);
                assert_eq!(source(&updates,id).segments[0].last_seq,6);
            }
        }
    }

    #[test]
    fn typed_text_blocks_and_final_delivery_remain_real_boundaries() {
        let block = |index| serde_json::json!({"item_id":"message-one","output_index":0,"content_index":index}).to_string();
        let mut timeline = Timeline::new(&turn());
        for event in [Fact {text_source_json:Some(block(0)),..fact(2,"message/content-part","answer",Some("one"))},
            runtime(3,"reasoning_delta",1),fact(4,"thinking/content-part","thought",Some("inspect")),
            Fact {text_source_json:Some(block(0)),..fact(5,"message/content-part","answer",Some(" continued"))},
            Fact {text_source_json:Some(block(1)),..fact(6,"message/content-part","answer",Some("two"))},
            runtime(7,"completion_delivered",1),fact(8,"message/content-part","answer",Some("final"))] {
            timeline.accept(&event).unwrap();
        }
        let updates = timeline.finish(&turn(),8).unwrap();
        let segments = &source(&updates,"answer").segments;
        assert_eq!(segments.len(),3);
        assert_eq!(segments.iter().map(|segment| (segment.segment_id.as_str(),segment.start_utf16,segment.end_utf16)).collect::<Vec<_>>(),
            [("event-2",Some(0),Some(13)),("event-6",Some(13),Some(16)),("event-8",Some(16),Some(21))]);
    }

    #[test]
    fn malformed_or_unknown_text_source_metadata_fails_closed() {
        for value in [
            serde_json::json!({"item_id":"","content_index":0}),
            serde_json::json!({"item_id":"body","content_index":-1}),
            serde_json::json!({"item_id":"body","content_index":0,"unknown_identity":"future"}),
            serde_json::json!({"item_id":"body"}),
            serde_json::json!("body"),
        ] {
            let mut timeline = Timeline::new(&turn());
            assert!(timeline.accept(&Fact {text_source_json:Some(value.to_string()),
                ..fact(2,"message/content-part","answer",Some("body"))}).is_err());
        }
    }

    #[test]
    fn distinct_messages_steps_and_steering_cannot_merge_across_reasoning() {
        let mut timeline = Timeline::new(&turn());
        for event in [runtime(2,"output_text_delta",1),fact(3,"message/content-part","answer",Some("first")),
            runtime(4,"reasoning_delta",1),fact(5,"message/content-part","other",Some("other")),
            fact(6,"message/content-part","answer",Some("again")),runtime(7,"reasoning_delta",2),
            fact(8,"message/content-part","answer",Some("step two")),fact(9,"turn/steer-accepted","turn-operation",None),
            fact(10,"message/content-part","answer",Some("after steering"))] {
            timeline.accept(&event).unwrap();
        }
        let updates = timeline.finish(&turn(),10).unwrap();
        assert_eq!(source(&updates,"answer").segments.iter().map(|segment| segment.segment_id.as_str()).collect::<Vec<_>>(),
            ["event-3","event-6","event-8","event-10"]);
        assert_eq!(source(&updates,"other").segments.len(),1);
    }

    #[test]
    fn delayed_parallel_results_and_repeated_start_facts_do_not_move_or_split_later_narration() {
        let mut timeline = Timeline::new(&turn());
        for event in [tool(2,"tool_call_completed","one"),tool(3,"tool_started","one"),
            runtime(4,"output_text_delta",1),fact(5,"message/content-part","answer",Some("First ")),
            Fact { result_error:Some(0), ..tool(6,"tool_completed","one") },
            tool(7,"tool_started","one"),runtime(8,"output_text_delta",1),
            fact(9,"message/content-part","answer",Some("second.")),tool(10,"tool_call_completed","two"),tool(11,"tool_started","two")] {
            timeline.accept(&event).unwrap();
        }
        let updates = timeline.finish(&turn(),11).unwrap();
        let answer = source(&updates,"answer");
        assert_eq!(answer.segments.len(),1,"results and duplicate starts cannot cut a newer body run");
        assert_eq!(answer.segments[0].end_utf16,Some(13));
        let first = updates.iter().find(|update|update.call_id.as_deref()==Some("one")).unwrap();
        let second = updates.iter().find(|update|update.call_id.as_deref()==Some("two")).unwrap();
        assert_eq!(first.timeline.segments[0].group_id,format!("process:{ROOT}"));
        assert_eq!(second.timeline.segments[0].group_id,"process:event-5");
        assert_eq!(first.timeline.segments[0].state,State::Completed);
    }

    #[test]
    fn ranges_are_utf16_and_narration_runs_are_independent_boundaries() {
        let mut timeline = Timeline::new(&turn());
        for event in [fact(2,"message/content-part","answer",Some("A😀e\u{301}")),
            tool(3,"tool_call_completed","one"),tool(4,"tool_started","one"),
            fact(5,"message/content-part","answer",Some("🧑‍💻终")),tool(6,"tool_call_completed","two"),tool(7,"tool_started","two")] {
            timeline.accept(&event).unwrap();
        }
        let updates=timeline.finish(&turn(),7).unwrap();
        let segments=&source(&updates,"answer").segments;
        assert_eq!(segments.len(),2);
        assert_eq!((segments[0].start_utf16,segments[0].end_utf16),(Some(0),Some(5)));
        assert_eq!((segments[1].start_utf16,segments[1].end_utf16),(Some(5),Some(11)));
        assert_eq!(updates.iter().find(|update|update.call_id.as_deref()==Some("two")).unwrap().timeline.segments[0].group_id,"process:event-5");
    }

    #[test]
    fn hidden_prefix_narration_keeps_the_same_group_anchor_without_loading_its_body() {
        let events=[fact(2,"message/content-part","prefix",Some("intro")),tool(3,"tool_call_completed","one"),tool(4,"tool_started","one")];
        let mut complete=Timeline::new(&turn());let mut partial=Timeline::new(&turn());
        for event in events {complete.accept(&event).unwrap();let hidden=Fact {content:None,..event};partial.accept(&hidden).unwrap();}
        let complete=complete.finish(&turn(),4).unwrap();let partial=partial.finish(&turn(),4).unwrap();
        assert_eq!(partial.iter().find(|update|update.call_id.is_some()).unwrap(),complete.iter().find(|update|update.call_id.is_some()).unwrap());
        assert!(!partial.iter().any(|update|update.source_message_id=="prefix"));
    }

    #[test]
    fn future_turn_terminal_does_not_close_an_older_snapshot_and_missing_results_are_never_success() {
        let mut closed=turn();closed.last_seq=Some(30);closed.state="completed".into();
        let build=|| {let mut timeline=Timeline::new(&closed);for event in [tool(2,"tool_call_completed","pending"),tool(3,"tool_started","pending"),
            runtime(4,"reasoning_delta",1),fact(5,"thinking/content-part","thought",Some("wait"))] {timeline.accept(&event).unwrap();}timeline};
        let early=build().finish(&closed,10).unwrap();
        assert_eq!(source(&early,"thought").segments[0].state,State::Running);
        let settled=build().finish(&closed,30).unwrap();
        assert_eq!(source(&settled,"thought").segments[0].state,State::Completed);
        assert_eq!(settled.iter().find(|update|update.call_id.is_some()).unwrap().timeline.segments[0].state,State::Interrupted);
    }

    #[test]
    fn typed_activity_handoffs_empty_reopening_and_pause_preserve_source_lifecycle() {
        let mut timeline=Timeline::new(&turn());
        for event in [runtime(2,"reasoning_delta",1),fact(3,"thinking/content-part","thought",Some("inspect")),
            runtime(4,"output_text_delta",1),runtime(5,"reasoning_delta",1)] {timeline.accept(&event).unwrap();}
        let updates=timeline.finish(&turn(),5).unwrap();let segments=&source(&updates,"thought").segments;
        assert_eq!(segments.len(),2);assert_eq!(segments[0].state,State::Completed);
        assert_eq!((segments[1].start_utf16,segments[1].end_utf16),(Some(7),Some(7)));
        assert_eq!(segments[1].state,State::Running);
        let mut paused=Timeline::new(&turn());for event in [runtime(2,"reasoning_delta",1),
            fact(3,"thinking/content-part","thought",Some("inspect")),runtime(4,"turn_paused",1)] {paused.accept(&event).unwrap();}
        assert_eq!(source(&paused.finish(&turn(),4).unwrap(),"thought").segments[0].state,State::Interrupted);
    }

    #[test]
    fn voice_withdrawal_keeps_the_boundary_but_returns_no_obsolete_narration() {
        let id=nomifun_agent_contracts::canonical_assistant_step_message_id(ROOT,1).unwrap();
        let mut timeline=Timeline::new(&turn());
        for event in [fact(2,"message/content-part",&id,Some("withdrawn")),runtime(3,"voice_model_step_superseded",1),
            tool(4,"tool_call_completed","later"),tool(5,"tool_started","later")] {timeline.accept(&event).unwrap();}
        let updates=timeline.finish(&turn(),5).unwrap();
        assert!(source(&updates,&id).segments.is_empty());
        assert_eq!(updates.iter().find(|update|update.call_id.is_some()).unwrap().timeline.segments[0].group_id,"process:event-2");
    }

    #[test]
    fn human_steering_separates_process_groups_without_a_narration_body() {
        let mut timeline=Timeline::new(&turn());
        for event in [tool(2,"tool_call_completed","before"),tool(3,"tool_started","before"),
            fact(4,"turn/steer-accepted","turn-operation",None),tool(5,"tool_call_completed","after"),tool(6,"tool_started","after")] {
            timeline.accept(&event).unwrap();
        }
        let updates=timeline.finish(&turn(),6).unwrap();
        assert_eq!(updates.iter().find(|update|update.call_id.as_deref()==Some("before")).unwrap().timeline.segments[0].group_id,format!("process:{ROOT}"));
        assert_eq!(updates.iter().find(|update|update.call_id.as_deref()==Some("after")).unwrap().timeline.segments[0].group_id,"process:event-4");
    }

    #[test]
    fn a_late_foreign_owner_result_cannot_settle_a_reused_call_id_in_this_turn() {
        let mut timeline=Timeline::new(&turn());
        for event in [tool(2,"tool_call_completed","same"),
            Fact {call_id:Some("same".into()),..fact(3,"tool/call-started","current-owner",None)},
            Fact {call_id:Some("same".into()),result_error:Some(1),..fact(4,"tool/result-recorded","previous-turn-owner",None)},
            Fact {call_id:Some("foreign".into()),causation_event_id:Some("another-turn-start".into()),
                ..fact(5,"tool/call-started","foreign-owner",None)}] {timeline.accept(&event).unwrap();}
        let updates=timeline.finish(&turn(),5).unwrap();
        assert_eq!(updates.len(),1);assert_eq!(updates[0].source_message_id,"current-owner");
        assert_eq!(updates[0].timeline.segments[0].state,State::Running);
        assert_eq!(updates[0].timeline.segments[0].last_seq,3);
    }

    #[test]
    fn reasoning_body_alone_is_not_proof_of_an_active_runtime_phase() {
        let mut timeline=Timeline::new(&turn());
        timeline.accept(&fact(2,"thinking/content-part","thought",Some("recorded evidence"))).unwrap();
        let updates=timeline.finish(&turn(),2).unwrap();
        assert_eq!(source(&updates,"thought").segments[0].state,State::Completed);
    }

    #[test]
    fn a_result_from_a_different_model_step_does_not_reveal_an_unpaired_proposal() {
        let mut timeline=Timeline::new(&turn());
        timeline.accept(&tool(2,"tool_call_completed","same")).unwrap();
        timeline.accept(&Fact {runtime_step:Some(2),result_error:Some(1),..tool(3,"tool_completed","same")}).unwrap();
        assert!(timeline.finish(&turn(),3).unwrap().is_empty());
    }

    #[test]
    fn unknown_versions_fail_closed_and_foreign_runtime_producers_cannot_change_lifecycle() {
        let mut timeline=Timeline::new(&turn());
        timeline.accept(&runtime(2,"reasoning_delta",1)).unwrap();
        timeline.accept(&fact(3,"thinking/content-part","thought",Some("inspect"))).unwrap();
        timeline.accept(&Fact {producer_id:"owning_plugin".into(),..runtime(4,"output_text_delta",1)}).unwrap();
        let updates=timeline.finish(&turn(),4).unwrap();
        assert_eq!(source(&updates,"thought").segments[0].state,State::Running);
        let mut future=Timeline::new(&turn());
        assert!(future.accept(&Fact {kind_version:2,..runtime(2,"reasoning_delta",1)}).is_err());
        assert!(future.accept(&Fact {kind_version:2,..fact(3,"thinking/content-part","thought",Some("future"))}).is_err());
    }
}
