//! Workshop fixtures authored through the canonical Store and its projector.

use nomifun_agent_contracts::{
    AgentBindingValue, AgentSessionLiveRecord, AgentSessionMetadata, EventId, PresetRevisionRef,
    PrincipalRef, ResolvedSnapshotRef, SemanticSessionEventDraft, SessionEventAppend,
    SessionEventKind, SessionEventPayloadRef, StrictJsonValue, digest_bytes,
};
use nomifun_agent_session::{AgentSessionStore, CreateSessionRequest};
use serde_json::{Value, json};

pub(crate) async fn completed_agent_history(
    database: &nomifun_db::Database,
    assistant_texts: &[String],
) -> (String, Vec<String>) {
    let owner_id = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let session_id = nomifun_common::generate_id();
    let store = AgentSessionStore::from_pool(database.pool().clone())
        .await
        .unwrap();
    let binding = AgentBindingValue {
        preset_revision_ref: PresetRevisionRef {
            preset_id: nomifun_common::generate_id().into(),
            revision: 1,
            revision_digest: "a".repeat(64).into(),
        },
        resolved_snapshot_ref: ResolvedSnapshotRef {
            snapshot_id: nomifun_common::generate_id().into(),
            snapshot_digest: "b".repeat(64).into(),
        },
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let created = store
        .create_session(CreateSessionRequest::new(
            AgentSessionLiveRecord {
                agent_session_id: session_id.clone().into(),
                owner_ref: PrincipalRef {
                    principal_kind: "user".into(),
                    principal_id: owner_id,
                },
                metadata: AgentSessionMetadata {
                    purpose: Default::default(),
                    app_project_id: None,
                    companion: None,
                    title: Some("Creative Studio Agent".into()),
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: binding,
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            1,
            format!("open:{session_id}"),
            "session_api".into(),
            format!("open:{session_id}").into(),
            session_id.clone().into(),
        ))
        .await
        .unwrap();
    store
        .append_event(&event(
            &session_id,
            format!("ready:{session_id}"),
            "session/ready",
            &session_id,
            created.opening_ack.event_id,
            json!({}),
        ))
        .await
        .unwrap();

    let mut message_ids = Vec::new();
    for (index, text) in assistant_texts.iter().enumerate() {
        let operation = format!("workshop:{session_id}:{index}");
        let (input, started) = store
            .start_turn(
                &session_id.clone().into(),
                "session_api".into(),
                operation.clone().into(),
                operation.clone().into(),
                StrictJsonValue(json!({"content": "request"})),
            )
            .await
            .unwrap();
        let input_id = input.record.unwrap().event_id;
        let assistant_id =
            nomifun_agent_contracts::canonical_assistant_step_message_id(input_id.as_ref(), 1)
                .unwrap();
        let part_id = format!("assistant-part:{session_id}:{operation}:1:1");
        store
            .append_event(&event(
                &session_id,
                part_id.clone(),
                "message/content-part",
                &assistant_id,
                started.ack.unwrap().event_id,
                json!({"content": text, "turn_id": input_id}),
            ))
            .await
            .unwrap();
        let completed_id = format!("assistant-complete:{session_id}:{operation}:1");
        store
            .append_event(&event(
                &session_id,
                completed_id.clone(),
                "message/completed",
                &assistant_id,
                part_id.into(),
                json!({"part_count": 1, "content_digest": digest_bytes(text.as_bytes())}),
            ))
            .await
            .unwrap();
        store
            .append_event(&event(
                &session_id,
                format!("turn-terminal:{session_id}:{operation}"),
                "turn/completed",
                &operation,
                completed_id.into(),
                json!({"model_steps": 1, "finished_at_ms": nomifun_common::now_ms()}),
            ))
            .await
            .unwrap();
        message_ids.push(input_id.as_ref().to_owned());
        message_ids.push(assistant_id.clone());
    }
    (session_id, message_ids)
}

fn event(
    session: &str,
    identity: String,
    kind: &str,
    correlation: &str,
    cause: EventId,
    payload: Value,
) -> SessionEventAppend {
    SessionEventAppend {
        agent_session_id: session.to_owned().into(),
        event_id: identity.clone().into(),
        producer_id: "runtime_supervisor".into(),
        idempotency_key: identity.into(),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind(kind.to_owned()),
            kind_version: 1,
            correlation_id: correlation.to_owned().into(),
            causation_event_id: Some(cause),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
        },
    }
}
