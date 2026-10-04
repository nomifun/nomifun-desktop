//! Bounded data-only context snapshots decoded from canonical Session events.
//! These messages never contain executable tools, capabilities, effects,
//! completion authority, private reasoning or a Runtime checkpoint.
use std::collections::BTreeMap;
use nomifun_agent_contracts::{AgentHandoffBindingRefV1, AgentSessionId, DigestHex, SessionEventRecord, canonical_json_bytes, digest_payload};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::SessionStoreError;

pub const MAX_FORK_CONTEXT_BYTES: usize = crate::MAX_SINGLE_PAYLOAD_BYTES;
pub const MAX_FORK_CONTEXT_MESSAGES: usize = 4096;
pub const MAX_FORK_CONTEXT_DEPTH: u16 = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalContextRole { User, Assistant }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalContextMessage {
    pub seq: u64,
    pub role: CanonicalContextRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForkContextSnapshot {
    pub version: u8,
    pub parent_agent_session_id: AgentSessionId,
    pub parent_through_seq: u64,
    /// Original source identity only; the child never inherits its authority.
    pub source_binding: AgentHandoffBindingRefV1,
    pub fork_depth: u16,
    pub messages: Vec<CanonicalContextMessage>,
    pub content_digest: DigestHex,
}

fn failure(message: impl std::fmt::Display) -> SessionStoreError {
    SessionStoreError::InvalidPayload(format!("Canonical context: {message}"))
}

impl ForkContextSnapshot {
    pub fn new(parent_agent_session_id: AgentSessionId, parent_through_seq: u64,
        source_binding: AgentHandoffBindingRefV1, fork_depth: u16,
        messages: Vec<CanonicalContextMessage>) -> Result<Self, SessionStoreError> {
        let mut snapshot = Self { version: 1, parent_agent_session_id, parent_through_seq,
            source_binding, fork_depth, messages, content_digest: DigestHex::from(String::new()) };
        snapshot.content_digest = snapshot.expected_digest()?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn expected_digest(&self) -> Result<DigestHex, SessionStoreError> {
        Ok(digest_payload(&(self.version, &self.parent_agent_session_id, self.parent_through_seq,
            &self.source_binding, self.fork_depth, &self.messages))?)
    }

    pub fn validate(&self) -> Result<(), SessionStoreError> {
        if self.version != 1 || self.fork_depth == 0 || self.fork_depth > MAX_FORK_CONTEXT_DEPTH
            || self.messages.len() > MAX_FORK_CONTEXT_MESSAGES
            || self.messages.iter().any(|message| message.seq > self.parent_through_seq || message.content.is_empty())
            || self.messages.windows(2).any(|pair| pair[0].seq > pair[1].seq)
            || canonical_json_bytes(self)?.len() > MAX_FORK_CONTEXT_BYTES
            || self.content_digest != self.expected_digest()? {
            return Err(failure("fork context is invalid, out of budget or differs from its digest"));
        }
        Ok(())
    }

    /// A clear-context event in this child permanently removes the inherited
    /// base from inference. Reading it never opens or follows the parent.
    pub fn base_context(&self, context_floor: u64) -> Result<Vec<CanonicalContextMessage>, SessionStoreError> {
        self.validate()?;
        if context_floor > 0 { return Ok(Vec::new()); }
        let mut messages = self.messages.clone();
        for message in &mut messages { message.seq = 0; }
        Ok(messages)
    }
}

/// Domain messages and the transcript before a declared Agent transition are
/// data-only context. Decode their canonical events directly; UI projections
/// cannot supply missing Runtime facts or determine model message roles.
pub fn canonical_context_messages(
    events: &[SessionEventRecord],
    payloads: &BTreeMap<String, Value>,
    floor: u64,
    native_floor: u64,
    before_seq: u64,
) -> Result<Vec<CanonicalContextMessage>, SessionStoreError> {
    let mut native_roots = std::collections::BTreeSet::new();
    for turn in events.iter().filter(|event| {
        event.kind.0 == "turn/started" && event.seq > native_floor && event.seq < before_seq
    }) {
        let source = payloads.get(turn.event_id.as_ref())
            .and_then(|payload| payload.get("source_message_id"))
            .and_then(Value::as_str)
            .ok_or_else(|| failure("canonical Turn has no accepted source identity"))?;
        native_roots.insert(source);
    }
    let mut parts = BTreeMap::<String, (u64, String, u64)>::new();
    let mut messages = Vec::new();
    let mut ordered = events.iter().filter(|event| event.seq > floor && event.seq < before_seq)
        .collect::<Vec<_>>();
    ordered.sort_by_key(|event| event.seq);
    for event in ordered {
        let kind = event.kind.0.as_str();
        let accepted = kind == "message/user-accepted" && !native_roots.contains(event.event_id.as_ref());
        let projected = kind == "message/assistant-projected";
        let previous_agent_part = event.seq <= native_floor && kind == "message/content-part";
        let previous_agent_completed = event.seq <= native_floor && kind == "message/completed";
        let previous_agent_steer = event.seq <= native_floor && kind == "turn/steer-accepted";
        if !(accepted || projected || previous_agent_part || previous_agent_completed || previous_agent_steer) {
            continue;
        }
        if event.kind_version != 1 {
            return Err(failure("unsupported canonical context event version"));
        }
        let value = payloads.get(event.event_id.as_ref())
            .ok_or_else(|| failure("canonical context event has no resolved payload"))?;
        if previous_agent_completed {
            let Some((first_seq, content, count)) = parts.remove(event.correlation_id.as_ref()) else {
                // An empty response is represented by a completion with no parts.
                if value.get("part_count").and_then(Value::as_u64) == Some(0) { continue; }
                return Err(failure("canonical assistant completion has no content parts"));
            };
            if value.get("part_count").and_then(Value::as_u64) != Some(count)
                || value.get("content_digest").and_then(Value::as_str)
                    != Some(nomifun_agent_contracts::digest_bytes(content.as_bytes()).as_ref()) {
                return Err(failure("canonical assistant completion differs from its content parts"));
            }
            messages.push(CanonicalContextMessage { seq: first_seq, role: CanonicalContextRole::Assistant, content });
            continue;
        }
        let content = if previous_agent_steer { value.pointer("/input/content") } else { value.get("content") }
            .and_then(Value::as_str)
            .ok_or_else(|| failure("canonical context message has no text"))?;
        if previous_agent_part {
            let part = parts.entry(event.correlation_id.as_ref().to_owned())
                .or_insert_with(|| (event.seq, String::new(), 0));
            if part.1.len().saturating_add(content.len()) > 8 * 1024 * 1024 {
                return Err(failure("canonical assistant context exceeds its budget"));
            }
            part.1.push_str(content);
            part.2 += 1;
        } else if !content.is_empty() {
            messages.push(CanonicalContextMessage {
                seq: event.seq,
                role: if accepted || previous_agent_steer { CanonicalContextRole::User } else { CanonicalContextRole::Assistant },
                content: content.to_owned(),
            });
        }
    }
    messages.sort_by_key(|message| message.seq);
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{AgentPresetId, PresetRevisionRef, ResolvedSnapshotId, ResolvedSnapshotRef};

    fn snapshot(depth: u16, content: &str) -> ForkContextSnapshot {
        ForkContextSnapshot::new("0190f5fe-7c00-7a00-8000-000000000001".into(), 8,
            AgentHandoffBindingRefV1 {
                preset_revision_ref: PresetRevisionRef { preset_id: AgentPresetId::from("preset"),
                    revision: 1, revision_digest: "a".repeat(64).into() },
                resolved_snapshot_ref: ResolvedSnapshotRef { snapshot_id: ResolvedSnapshotId::from("snapshot"),
                    snapshot_digest: "b".repeat(64).into() },
                binding_version: 1,
            }, depth, vec![CanonicalContextMessage { seq: 7, role: CanonicalContextRole::User,
                content: content.into() }]).unwrap()
    }

    #[test]
    fn fork_base_is_flat_typed_data_and_its_digest_binds_source_and_text() {
        let base = snapshot(1, "accepted parent input");
        let raw = serde_json::to_value(&base).unwrap();
        for key in ["tools", "effects", "requirements", "checkpoint", "typed_resource_bindings"] {
            assert!(raw.get(key).is_none());
        }
        let mut forged = base.clone();
        forged.messages[0].content = "changed input".into();
        assert!(forged.validate().is_err());
        let mut changed_source = base.clone();
        changed_source.source_binding.binding_version += 1;
        assert!(changed_source.validate().is_err());
        let mut old_shape = serde_json::json!({"parent_agent_session_id":base.parent_agent_session_id,"parent_through_seq":8});
        assert!(serde_json::from_value::<ForkContextSnapshot>(old_shape.clone()).is_err());
        old_shape["checkpoint"] = serde_json::json!({});
        assert!(serde_json::from_value::<ForkContextSnapshot>(old_shape).is_err());
    }

    #[test]
    fn clear_context_removes_the_fork_base_without_following_its_parent() {
        let base = snapshot(1, "old parent input");
        assert_eq!(base.base_context(0).unwrap()[0].seq, 0);
        assert!(base.base_context(4).unwrap().is_empty());
    }

    #[test]
    fn fork_depth_and_content_budgets_are_enforced_without_old_format_fallback() {
        let mut base = snapshot(1, "bounded input");
        base.fork_depth = MAX_FORK_CONTEXT_DEPTH + 1;
        assert!(base.validate().is_err());
        let mut base = snapshot(1, "bounded input");
        base.messages[0].content = "a".repeat(MAX_FORK_CONTEXT_BYTES);
        assert!(base.validate().is_err());
    }

    #[test]
    fn accepted_steering_context_rejects_the_retired_text_field() {
        use nomifun_agent_contracts::{CorrelationId, EventId, EventProducerId, IdempotencyKey, SessionEventKind, SessionEventPayloadRef};
        let event = SessionEventRecord {
            agent_session_id: "0190f5fe-7c00-7a00-8000-000000000001".into(), seq: 3,
            event_id: EventId::from("steer"), producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from("steer"), kind: SessionEventKind("turn/steer-accepted".into()),
            kind_version: 1, correlation_id: CorrelationId::from("turn"),
            causation_event_id: None, payload: SessionEventPayloadRef::Empty,
        };
        let retired = BTreeMap::from([("steer".into(), serde_json::json!({"input":{"text":"old shape"}}))]);
        assert!(canonical_context_messages(std::slice::from_ref(&event), &retired, 0, 3, 4).is_err());
        let current = BTreeMap::from([("steer".into(), serde_json::json!({"input":{"content":"current input"}}))]);
        let accepted = canonical_context_messages(&[event], &current, 0, 3, 4).unwrap();
        assert_eq!(accepted[0].content, "current input");
        assert_eq!(accepted[0].role, CanonicalContextRole::User);
    }
}
