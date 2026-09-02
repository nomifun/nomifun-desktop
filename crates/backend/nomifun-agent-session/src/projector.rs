use nomifun_agent_contracts::{
    AgentSessionId, SessionEventPayloadRef, SessionEventRecord, digest_bytes, digest_payload,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::SessionStoreError;
use crate::types::{MessageProjection, SessionHeadProjection};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionDocument {
    projection_id: String,
    correlation_id: String,
    presentation_intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    part_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_summary: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_effect: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProjectionEvent {
    #[serde(rename = "seq")]
    _seq: u64,
    kind: String,
    #[serde(rename = "kind_version")]
    _kind_version: u32,
    payload: Value,
}

pub(crate) fn initial_head(session_id: &AgentSessionId) -> SessionHeadProjection {
    SessionHeadProjection {
        session_id: session_id.clone(),
        status: "opening".to_owned(),
        active_turn_id: None,
        active_set_generation: 0,
        runtime_checkpoint_locator: None,
        runtime_checkpoint_digest: None,
        runtime_bound_event_id: None,
        runtime_protocol_version: None,
        snapshot_digest: None,
        checkpoint_through_seq: None,
        last_seq: 0,
        unread_count: 0,
    }
}

pub(crate) fn reduce_head(
    head: &mut SessionHeadProjection,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    match event.kind.0.as_str() {
        "session/opening" => {
            head.status = "opening".to_owned();
            head.active_turn_id = None;
        }
        "session/ready" => {
            head.status = "ready".to_owned();
            head.active_turn_id = None;
        }
        "session/open-failed" => {
            head.status = "open_failed".to_owned();
            head.active_turn_id = None;
        }
        "turn/started" => {
            head.status = "running".to_owned();
            head.active_turn_id = Some(event.correlation_id.0.clone());
        }
        "turn/completed" | "turn/failed" | "turn/cancelled" => {
            head.status = "ready".to_owned();
            head.active_turn_id = None;
        }
        "effect/uncertain" => {
            head.status = "failed".to_owned();
            head.active_turn_id = None;
        }
        "message/completed" => {
            head.unread_count = head.unread_count.saturating_add(1);
        }
        "capability/active-set-committed" => {
            let generation = required_u64(payload, "generation")?;
            validate_active_set_payload(payload)?;
            if event.seq > 2 && generation != head.active_set_generation.saturating_add(1) {
                return Err(SessionStoreError::InvalidEvent(format!(
                    "active-set generation must advance from {} to {}, got {generation}",
                    head.active_set_generation,
                    head.active_set_generation.saturating_add(1)
                )));
            }
            if event.seq <= 2 && generation != 0 {
                return Err(SessionStoreError::InvalidEvent(
                    "opening active-set generation must be zero".to_owned(),
                ));
            }
            head.active_set_generation = generation;
        }
        "runtime/bound" => {
            head.runtime_bound_event_id = Some(event.event_id.0.clone());
            head.runtime_protocol_version = optional_string(payload, "protocol_version");
            head.snapshot_digest = snapshot_digest(payload);
            head.checkpoint_through_seq = Some(event.seq);
        }
        "runtime/checkpointed" => {
            head.runtime_checkpoint_locator = checkpoint_locator(payload);
            head.runtime_checkpoint_digest = checkpoint_digest(payload);
            head.runtime_bound_event_id = optional_string(payload, "runtime_bound_event_id")
                .or_else(|| head.runtime_bound_event_id.clone());
            head.runtime_protocol_version = optional_string(payload, "protocol_version")
                .or_else(|| head.runtime_protocol_version.clone());
            head.snapshot_digest =
                snapshot_digest(payload).or_else(|| head.snapshot_digest.clone());
            head.checkpoint_through_seq =
                optional_u64(payload, "through_seq").or(head.checkpoint_through_seq);
        }
        "runtime/binding-discarded" => {
            clear_checkpoint(head);
        }
        _ => {}
    }

    head.last_seq = event.seq;
    Ok(())
}

pub(crate) fn reduce_message_projection(
    existing: Option<MessageProjection>,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<MessageProjection, SessionStoreError> {
    let (projection_id, presentation_intent) = projection_identity(event);
    let mut document = match existing.as_ref() {
        Some(existing) => {
            let (document, legacy_events) =
                decode_projection_document(existing.projection.clone())?;
            normalize_legacy_projection(document, legacy_events)?
        }
        None => ProjectionDocument {
            projection_id: projection_id.clone(),
            correlation_id: event.correlation_id.0.clone(),
            presentation_intent: presentation_intent.clone(),
            state: None,
            content: None,
            content_digest: None,
            part_count: None,
            tool_summary: None,
            reference: None,
            terminal_effect: None,
        },
    };

    apply_projection_semantics(&mut document, event, payload)?;
    let semantic_digest = digest_payload(&document)?.0;
    let first_seq = existing.as_ref().map_or(event.seq, |row| row.first_seq);

    Ok(MessageProjection {
        session_id: event.agent_session_id.clone(),
        projection_id,
        first_seq,
        last_seq: event.seq,
        presentation_intent,
        projection: serde_json::to_value(document)?,
        semantic_digest,
    })
}

pub(crate) fn payload_value(event: &SessionEventRecord, stored_body: Option<Value>) -> Value {
    match &event.payload {
        SessionEventPayloadRef::Empty => Value::Null,
        SessionEventPayloadRef::InlineJson(value) => value.0.clone(),
        SessionEventPayloadRef::Stored(_) => stored_body.unwrap_or(Value::Null),
    }
}

fn projection_identity(event: &SessionEventRecord) -> (String, String) {
    let prefix = event.kind.0.split('/').next().unwrap_or("event");
    let intent = match prefix {
        "session" => "session_status",
        "turn" => "turn_status",
        "message" => "message",
        "context" => "context",
        "capability" => "capability",
        "tool" => "tool",
        "effect" => "effect",
        "runtime" => "runtime",
        "compaction" => "compaction",
        _ => "event",
    };
    (
        format!("{prefix}:{}", event.correlation_id.0),
        intent.to_owned(),
    )
}

fn apply_projection_semantics(
    document: &mut ProjectionDocument,
    event: &SessionEventRecord,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    match event.kind.0.as_str() {
        "session/opening" => document.state = Some("opening".to_owned()),
        "session/ready" => document.state = Some("ready".to_owned()),
        "session/open-failed" => document.state = Some("open_failed".to_owned()),
        "turn/started" => document.state = Some("running".to_owned()),
        "turn/completed" => document.state = Some("completed".to_owned()),
        "turn/failed" => document.state = Some("failed".to_owned()),
        "turn/cancelled" => document.state = Some("cancelled".to_owned()),
        "message/user-accepted" => {
            document.state = Some("accepted".to_owned());
            if let Some(content) = payload.get("content").and_then(Value::as_str) {
                document.content = Some(content.to_owned());
            }
        }
        "message/content-part" => {
            let content = payload
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SessionStoreError::InvalidEvent(
                        "message/content-part requires bounded content".to_owned(),
                    )
                })?;
            document.state = Some("streaming".to_owned());
            document
                .content
                .get_or_insert_with(String::new)
                .push_str(content);
            document.part_count = Some(
                document
                    .part_count
                    .unwrap_or_default()
                    .saturating_add(1),
            );
        }
        "message/completed" => {
            let expected_part_count = document.part_count.unwrap_or_default();
            let part_count = optional_u64(payload, "part_count").ok_or_else(|| {
                SessionStoreError::InvalidEvent("message/completed requires part_count".to_owned())
            })?;
            if part_count != expected_part_count {
                return Err(SessionStoreError::InvalidEvent(format!(
                    "message/completed part_count {part_count} does not match {expected_part_count} committed parts"
                )));
            }
            let expected_digest =
                digest_bytes(document.content.as_deref().unwrap_or_default().as_bytes()).0;
            let content_digest = optional_string(payload, "content_digest").ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "message/completed requires content_digest".to_owned(),
                )
            })?;
            if content_digest != expected_digest {
                return Err(SessionStoreError::InvalidEvent(
                    "message/completed content_digest does not match committed content parts"
                        .to_owned(),
                ));
            }
            document.state = Some("completed".to_owned());
            document.content_digest = Some(content_digest);
            document.part_count = Some(part_count);
        }
        "tool/call-started" => document.state = Some("started".to_owned()),
        "tool/result-recorded" => document.state = Some("recorded".to_owned()),
        "effect/started" => document.state = Some("started".to_owned()),
        "effect/succeeded" => document.state = Some("succeeded".to_owned()),
        "effect/failed" => document.state = Some("failed".to_owned()),
        "effect/uncertain" => document.state = Some("uncertain".to_owned()),
        "effect/reconciled" => {
            document.state = payload
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("reconciled".to_owned()));
        }
        "capability/active-set-committed" => {
            document.state = optional_u64(payload, "generation")
                .map(|generation| format!("generation_{generation}"));
        }
        "runtime/bound" => document.state = Some("bound".to_owned()),
        "runtime/checkpointed" => document.state = Some("checkpointed".to_owned()),
        "runtime/binding-discarded" => document.state = Some("discarded".to_owned()),
        "compaction/completed" => document.state = Some("completed".to_owned()),
        "session/forked" => document.state = Some("forked".to_owned()),
        _ => {}
    }
    apply_projection_summaries(
        document,
        event.kind.0.as_str(),
        Some(event.event_id.0.as_str()),
        payload,
    )?;
    Ok(())
}

fn decode_projection_document(
    mut value: Value,
) -> Result<(ProjectionDocument, Option<Vec<LegacyProjectionEvent>>), SessionStoreError> {
    let legacy_events = value
        .as_object_mut()
        .and_then(|object| object.remove("events"))
        .map(serde_json::from_value)
        .transpose()?;
    let document = serde_json::from_value(value)?;
    Ok((document, legacy_events))
}

fn normalize_legacy_projection(
    mut document: ProjectionDocument,
    legacy_events: Option<Vec<LegacyProjectionEvent>>,
) -> Result<ProjectionDocument, SessionStoreError> {
    let Some(events) = legacy_events else {
        return Ok(document);
    };

    if document.part_count.is_none() {
        let part_count = events
            .iter()
            .filter(|event| event.kind == "message/content-part")
            .count() as u64;
        if part_count > 0 {
            document.part_count = Some(part_count);
        }
    }

    if document.content.is_none() {
        let mut content = String::new();
        for event in &events {
            match event.kind.as_str() {
                "message/user-accepted" => {
                    if let Some(value) = event.payload.get("content").and_then(Value::as_str) {
                        content = value.to_owned();
                    }
                }
                "message/content-part" => {
                    if let Some(value) = event.payload.get("content").and_then(Value::as_str) {
                        content.push_str(value);
                    }
                }
                _ => {}
            }
        }
        if !content.is_empty() {
            document.content = Some(content);
        }
    }

    for event in &events {
        apply_projection_summaries(&mut document, &event.kind, None, &event.payload)?;
    }
    Ok(document)
}

fn apply_projection_summaries(
    document: &mut ProjectionDocument,
    kind: &str,
    event_id: Option<&str>,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    match kind {
        "tool/call-started" | "tool/result-recorded" => {
            let summary = object_slot(&mut document.tool_summary);
            for key in [
                "operation_id",
                "capability_id",
                "action_id",
                "name",
                "tool",
                "coding_surface",
            ] {
                copy_scalar(payload, summary, key);
            }
            if kind == "tool/result-recorded" {
                summary.insert("result_state".to_owned(), json!("recorded"));
                copy_digest(payload, summary, "output", "result_digest")?;
                copy_bounded_error(payload, summary);
            }
        }
        "effect/started"
        | "effect/succeeded"
        | "effect/failed"
        | "effect/uncertain"
        | "effect/reconciled" => {
            let effect = object_slot(&mut document.terminal_effect);
            for key in [
                "effect_id",
                "operation_id",
                "capability_id",
                "action_id",
                "effect",
                "coding_surface",
            ] {
                copy_scalar(payload, effect, key);
            }
            let state = match kind {
                "effect/started" => "started",
                "effect/succeeded" => "succeeded",
                "effect/failed" => "failed",
                "effect/uncertain" => "uncertain",
                "effect/reconciled" => payload
                    .get("outcome")
                    .and_then(Value::as_str)
                    .unwrap_or("reconciled"),
                _ => unreachable!("effect event was matched above"),
            };
            effect.insert("state".to_owned(), json!(state));
            copy_digest(payload, effect, "output", "result_digest")?;
            copy_digest(payload, effect, "receipt", "result_digest")?;
            copy_bounded_error(payload, effect);
        }
        _ => {}
    }

    copy_references(document, event_id, payload)?;
    Ok(())
}

fn object_slot(slot: &mut Option<Value>) -> &mut Map<String, Value> {
    if !matches!(slot, Some(Value::Object(_))) {
        *slot = Some(Value::Object(Map::new()));
    }
    slot.as_mut()
        .and_then(Value::as_object_mut)
        .expect("object slot was initialized above")
}

fn copy_scalar(payload: &Value, target: &mut Map<String, Value>, key: &str) {
    let Some(value) = payload.get(key) else {
        return;
    };
    match value {
        Value::String(value) => {
            if value.len() <= MAX_SUMMARY_STRING_BYTES {
                target.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
        Value::Bool(_) | Value::Number(_) => {
            target.insert(key.to_owned(), value.clone());
        }
        Value::Null | Value::Array(_) | Value::Object(_) => {}
    }
}

fn copy_digest(
    payload: &Value,
    target: &mut Map<String, Value>,
    source_key: &str,
    target_key: &str,
) -> Result<(), SessionStoreError> {
    if let Some(value) = payload.get(source_key) {
        target.insert(
            target_key.to_owned(),
            Value::String(digest_payload(value)?.0),
        );
    }
    Ok(())
}

fn copy_bounded_error(payload: &Value, target: &mut Map<String, Value>) {
    let Some(error) = payload.get("error").and_then(Value::as_str) else {
        return;
    };
    if error.len() <= MAX_SUMMARY_STRING_BYTES {
        target.insert("error".to_owned(), Value::String(error.to_owned()));
    }
}

fn copy_references(
    document: &mut ProjectionDocument,
    event_id: Option<&str>,
    payload: &Value,
) -> Result<(), SessionStoreError> {
    let reference = object_slot(&mut document.reference);
    if let Some(event_id) = event_id {
        reference.insert("last_event_id".to_owned(), json!(event_id));
    }
    for key in [
        "reference",
        "ref",
        "result_ref",
        "output_ref",
        "receipt_ref",
        "artifact_ref",
        "locator",
        "response_id",
        "runtime_binding_id",
        "snapshot_digest",
        "effect_id",
        "resource_binding_ids",
    ] {
        let Some(value) = payload.get(key) else {
            continue;
        };
        let serialized_len = serde_json::to_vec(value)?.len();
        if serialized_len <= MAX_REFERENCE_BYTES {
            reference.insert(key.to_owned(), value.clone());
        } else {
            reference.insert(
                format!("{key}_digest"),
                Value::String(digest_payload(value)?.0),
            );
        }
    }
    Ok(())
}

const MAX_SUMMARY_STRING_BYTES: usize = 1024;
const MAX_REFERENCE_BYTES: usize = 4096;

fn clear_checkpoint(head: &mut SessionHeadProjection) {
    head.runtime_checkpoint_locator = None;
    head.runtime_checkpoint_digest = None;
    head.runtime_bound_event_id = None;
    head.runtime_protocol_version = None;
    head.snapshot_digest = None;
    head.checkpoint_through_seq = None;
}

fn required_u64(value: &Value, field: &str) -> Result<u64, SessionStoreError> {
    optional_u64(value, field).ok_or_else(|| {
        SessionStoreError::InvalidEvent(format!("event payload requires integer {field}"))
    })
}

fn validate_active_set_payload(payload: &Value) -> Result<(), SessionStoreError> {
    let active = payload
        .get("active_capability_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "active-set event requires active_capability_ids".to_owned(),
            )
        })?;
    let active = active
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                SessionStoreError::InvalidEvent(
                    "active_capability_ids must contain strings".to_owned(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if active.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(SessionStoreError::InvalidEvent(
            "active_capability_ids must be sorted and unique".to_owned(),
        ));
    }
    let expected = digest_payload(&active)?.0;
    let actual = payload
        .get("active_set_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SessionStoreError::InvalidEvent(
                "active-set event requires active_set_digest".to_owned(),
            )
        })?;
    if actual != expected {
        return Err(SessionStoreError::InvalidEvent(
            "active_set_digest does not match active_capability_ids".to_owned(),
        ));
    }
    Ok(())
}

fn optional_u64(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn snapshot_digest(value: &Value) -> Option<String> {
    optional_string(value, "snapshot_digest").or_else(|| {
        value
            .get("resolved_snapshot_ref")
            .and_then(|snapshot| snapshot.get("snapshot_digest"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn checkpoint_locator(value: &Value) -> Option<String> {
    optional_string(value, "locator").or_else(|| {
        value
            .get("locator")
            .and_then(|locator| locator.get("normalized_relative_path"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn checkpoint_digest(value: &Value) -> Option<String> {
    optional_string(value, "digest").or_else(|| {
        value
            .get("locator")
            .and_then(|locator| locator.get("digest"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}
