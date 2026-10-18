//! Store-owned factoring of tool bodies. Semantic events remain unchanged to
//! consumers; only their durable representation shares Session-scoped bytes.
//! References are never accepted from an append draft or followed recursively.
use super::*;
use serde::{Deserialize, Serialize};

#[path = "shared_input_content.rs"]
mod input_content;

const KEY: &str = "_canonical_shared_content";
const MIN_BYTES: usize = 1024;
const MAX_REFERENCES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContentKind { ToolOutput, ToolArguments }

impl ContentKind {
    fn media_type(self) -> &'static str {
        match self {
            Self::ToolOutput => "application/vnd.nomifun.tool-output+json",
            Self::ToolArguments => "application/vnd.nomifun.tool-arguments+json",
        }
    }
    fn id(self, session: &AgentSessionId, digest: &DigestHex) -> ArtifactId {
        let kind = match self { Self::ToolOutput => "output", Self::ToolArguments => "arguments" };
        format!("shared-tool:{}:{kind}:{}", session.as_ref(), digest.as_ref()).into()
    }
    fn accepts(self, value: &Value) -> bool {
        match self { Self::ToolOutput => value.is_array(), Self::ToolArguments => value.is_object() }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentReference {
    pointer: String,
    kind: ContentKind,
    payload_id: ArtifactId,
    digest: DigestHex,
    byte_len: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentReferences {
    version: u8,
    references: Vec<ContentReference>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    input_sources: Vec<input_content::InputReference>,
}

pub(super) struct Prepared {
    pub append: SessionEventAppend,
    pub envelope: Option<SessionPayloadRecord>,
    pub contents: Vec<SessionPayloadRecord>,
}

fn invalid(message: &str) -> SessionStoreError { SessionStoreError::InvalidPayload(message.into()) }

pub(super) fn supported(kind: &str) -> bool {
    matches!(kind, "runtime/progress-recorded" | "tool/result-recorded")
}

// Idempotency is about the same semantic fact, including when a current-
// generation event was committed before factoring was introduced. Do this
// before inserting leaves so retrying such a fact cannot create orphan bytes.
pub(super) async fn duplicate(tx: &mut Transaction<'_, Sqlite>, append: &SessionEventAppend,
    supplied: Option<&SessionPayloadRecord>) -> Result<Option<StoredEventRow>, SessionStoreError> {
    if !supported(&append.semantic_event.kind.0) { return Ok(None); }
    let existing = match event_by_event_id_tx(tx, append.event_id.as_ref()).await? {
        Some(row) => Some(row),
        None => event_by_producer_key_tx(tx, append.producer_id.as_ref(), append.idempotency_key.as_ref()).await?,
    };
    let Some(existing) = existing else { return Ok(None); };
    let mut recorded = event_from_row(existing.clone())?;
    let value = payload_value_for_event_tx(tx, &recorded).await?;
    recorded.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(value));
    let mut proposed = record_from_append(append, recorded.seq);
    let value = match (&append.semantic_event.payload, supplied) {
        (_, Some(payload)) => payload_body_to_value(&payload.body)?,
        (SessionEventPayloadRef::Stored(_), _) => payload_value_for_event_tx(tx, &proposed).await?,
        _ => payload_value(&proposed, None),
    };
    proposed.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(value));
    if proposed != recorded {
        return Err(SessionStoreError::IdempotencyConflict("event identity already committed with different semantic content".into()));
    }
    Ok(Some(existing))
}

// Discover only formally owned slots. Tool-produced JSON and strings are data,
// including objects that happen to contain a similarly named property.
fn slots(kind: &str, value: &Value) -> Vec<(String, ContentKind)> {
    let mut slots = Vec::new();
    if kind == "tool/result-recorded" {
        if value.pointer("/output/call_id").and_then(Value::as_str).is_some() {
            slots.push(("/output/output".into(), ContentKind::ToolOutput));
        }
        return slots;
    }
    if kind != "runtime/progress-recorded" { return slots; }
    match value.pointer("/event/event").and_then(Value::as_str) {
        Some("host_tool_settled" | "tool_completed" | "tool_outcome_reconciled") => {
            if value.pointer("/event/result/call_id").and_then(Value::as_str).is_some() {
                slots.push(("/event/result/output".into(), ContentKind::ToolOutput));
            }
        }
        Some("tool_call_completed") => slots.push(("/event/call/arguments".into(), ContentKind::ToolArguments)),
        Some("context_compacted") => {
            if let Some(items) = value.pointer("/event/retained_context").and_then(Value::as_array) {
                for (item_index, item) in items.iter().enumerate() {
                    if item.get("kind").and_then(Value::as_str) != Some("message") { continue; }
                    if let Some(parts) = item.pointer("/message/content").and_then(Value::as_array) {
                        for (part_index, part) in parts.iter().enumerate() {
                            let (field, kind) = match part.get("type").and_then(Value::as_str) {
                                Some("tool_call") => ("arguments", ContentKind::ToolArguments),
                                Some("tool_result") => ("output", ContentKind::ToolOutput),
                                _ => continue,
                            };
                            slots.push((format!("/event/retained_context/{item_index}/message/content/{part_index}/{field}"), kind));
                        }
                    }
                }
            }
        }
        _ => {},
    }
    slots
}

pub(super) async fn prepare(tx:&mut Transaction<'_,Sqlite>,append: &SessionEventAppend, supplied: Option<&SessionPayloadRecord>) -> Result<Prepared, SessionStoreError> {
    let mut prepared = Prepared { append: append.clone(), envelope: supplied.cloned(), contents: Vec::new() };
    if !supported(&append.semantic_event.kind.0) {
        return Ok(prepared);
    }
    let mut value = match (&append.semantic_event.payload, supplied) {
        (SessionEventPayloadRef::InlineJson(value), _) => value.0.clone(),
        (SessionEventPayloadRef::Stored(_), Some(record)) => payload_body_to_value(&record.body)?,
        // Existing direct payload references retain their normal validation.
        _ => return Ok(prepared),
    };
    if value.get(KEY).is_some() { return Err(invalid("shared content references are authored only by the Store")); }
    let input_sources=input_content::prepare(tx,append,&mut value).await?;
    let mut references = Vec::new();
    for (pointer, kind) in slots(&append.semantic_event.kind.0, &value) {
        let Some(body) = value.pointer_mut(&pointer) else { continue; };
        if !kind.accepts(body) { continue; }
        let bytes = canonical_json_bytes(body)?;
        if bytes.len() < MIN_BYTES { continue; }
        if references.len() >= MAX_REFERENCES { return Err(invalid("shared tool content reference bound exceeded")); }
        let digest = digest_bytes(&bytes);
        let payload_id = kind.id(&append.agent_session_id, &digest);
        references.push(ContentReference { pointer, kind, payload_id: payload_id.clone(), digest: digest.clone(), byte_len: bytes.len() as u64 });
        prepared.contents.push(SessionPayloadRecord {
            payload_id, agent_session_id: append.agent_session_id.clone(), media_type: kind.media_type().into(),
            byte_len: bytes.len() as u64, digest, body: SessionPayloadBody::Json(StrictJsonValue(body.take())),
        });
    }
    if references.is_empty() && input_sources.is_empty() { return Ok(prepared); }
    value.as_object_mut().ok_or_else(|| invalid("shared content envelope must be an object"))?
        .insert(KEY.into(), serde_json::to_value(ContentReferences { version: 1, references, input_sources })?);
    let bytes = canonical_json_bytes(&value)?;
    if bytes.len() <= MAX_INLINE_JSON_BYTES {
        prepared.append.semantic_event.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(value));
        prepared.envelope = None;
    } else {
        let digest = digest_bytes(&bytes);
        let payload_id: ArtifactId = format!("shared-envelope:{}:{}", append.agent_session_id.as_ref(), digest.as_ref()).into();
        prepared.append.semantic_event.payload = SessionEventPayloadRef::Stored(payload_id.clone());
        prepared.envelope = Some(SessionPayloadRecord { payload_id, agent_session_id: append.agent_session_id.clone(),
            media_type: "application/json".into(), byte_len: bytes.len() as u64, digest,
            body: SessionPayloadBody::Json(StrictJsonValue(value)) });
    }
    Ok(prepared)
}

pub(super) async fn hydrate(tx: &mut Transaction<'_, Sqlite>, event: &SessionEventRecord, mut value: Value) -> Result<Value, SessionStoreError> {
    let Some(metadata) = value.get(KEY) else { return Ok(value); };
    if event.kind_version != 1 || !supported(&event.kind.0) {
        return Err(invalid("shared content references require an exact supported event contract"));
    }
    let metadata: ContentReferences = serde_json::from_value(metadata.clone())?;
    if metadata.version != 1 || (metadata.references.is_empty() && metadata.input_sources.is_empty()) || metadata.references.len() > MAX_REFERENCES {
        return Err(invalid("invalid shared content codec or reference count"));
    }
    let allowed = slots(&event.kind.0, &value);
    value.as_object_mut().ok_or_else(|| invalid("shared content envelope must be an object"))?.remove(KEY);
    let mut expanded_bytes=canonical_json_bytes(&value)?.len();
    let mut seen = BTreeSet::new();
    for reference in metadata.references {
        if !seen.insert(reference.pointer.clone()) || !allowed.contains(&(reference.pointer.clone(), reference.kind))
            || reference.payload_id != reference.kind.id(&event.agent_session_id, &reference.digest)
            || reference.byte_len < MIN_BYTES as u64 || reference.byte_len > MAX_SINGLE_PAYLOAD_BYTES as u64 {
            return Err(invalid("shared content reference is outside its exact Session and typed slot"));
        }
        expanded_bytes=expanded_bytes.saturating_sub(4).saturating_add(reference.byte_len as usize);
        if expanded_bytes>MAX_SINGLE_PAYLOAD_BYTES {
            return Err(invalid("expanded shared content event exceeds its original payload bound"));
        }
        let slot = value.pointer_mut(&reference.pointer).ok_or_else(|| invalid("shared content slot missing"))?;
        if !slot.is_null() { return Err(invalid("shared content slot must be an empty placeholder")); }
        let row = payload_by_id_tx(tx, reference.payload_id.as_ref()).await?
            .ok_or_else(|| invalid("shared tool content is missing"))?;
        let payload = payload_from_row(row)?;
        if payload.agent_session_id != event.agent_session_id || payload.media_type != reference.kind.media_type()
            || payload.digest != reference.digest || payload.byte_len != reference.byte_len {
            return Err(invalid("shared tool content identity or media type differs"));
        }
        let SessionPayloadBody::Json(body) = payload.body else { return Err(invalid("shared tool content must be typed JSON")); };
        let bytes = canonical_json_bytes(&body.0)?;
        if bytes.len() as u64 != reference.byte_len || digest_bytes(&bytes) != reference.digest || !reference.kind.accepts(&body.0) {
            return Err(invalid("shared tool content differs from its exact bytes or type"));
        }
        *slot = body.0;
    }
    input_content::hydrate(tx,event,&mut value,metadata.input_sources).await?;
    if canonical_json_bytes(&value)?.len()>MAX_SINGLE_PAYLOAD_BYTES {
        return Err(invalid("expanded input event exceeds its original payload bound"));
    }
    Ok(value)
}
