use nomifun_agent_contracts::{
    AgentSessionId, SessionEventPayloadRef, SessionEventRecord, digest_bytes, digest_payload,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SessionStoreError;
use crate::types::{MessageProjection, SessionHeadProjection};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionDocument {
    projection_id: String,
    correlation_id: String,
    presentation_intent: String,
    events: Vec<ProjectionEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    part_count: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionEvent {
    seq: u64,
    kind: String,
    kind_version: u32,
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
            serde_json::from_value::<ProjectionDocument>(existing.projection.clone())?
        }
        None => ProjectionDocument {
            projection_id: projection_id.clone(),
            correlation_id: event.correlation_id.0.clone(),
            presentation_intent: presentation_intent.clone(),
            events: Vec::new(),
            state: None,
            content: None,
            content_digest: None,
            part_count: None,
        },
    };

    document.events.push(ProjectionEvent {
        seq: event.seq,
        kind: event.kind.0.clone(),
        kind_version: event.kind_version,
        payload: payload.clone(),
    });
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
        "message/user-accepted" => document.state = Some("accepted".to_owned()),
        "message/content-part" => {
            document.state = Some("streaming".to_owned());
            let content = payload
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SessionStoreError::InvalidEvent(
                        "message/content-part requires bounded content".to_owned(),
                    )
                })?;
            document
                .content
                .get_or_insert_with(String::new)
                .push_str(content);
        }
        "message/completed" => {
            let expected_part_count = document
                .events
                .iter()
                .filter(|event| event.kind == "message/content-part")
                .count() as u64;
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
    Ok(())
}

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
