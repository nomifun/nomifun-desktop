use super::*;
use nomifun_agent_contracts::AgentInputSource;

async fn accepted_fixture(
    store: &AgentSessionStore, key: &str, stored: bool,
) -> (AgentSessionLiveRecord, OperationId, SessionEventRecord, SessionEventRecord, serde_json::Value) {
    let (session, ready) = create_ready(store, key).await;
    let operation = OperationId::from(format!("accepted-{key}"));
    let source = AgentInputSource::AuthenticatedRobot {
        companion_id: Uuid::now_v7().to_string(), robot_id: "robot-1".into(), connection_id: "connection-1".into(),
        presented_question_id: None,
    };
    let input = json!({"content":"work on the desktop", "input_source":source});
    if !stored {
        let (root, start) = store.start_turn(&session.agent_session_id, "session_api".into(), key.into(),
            operation.clone(), StrictJsonValue(input.clone())).await.unwrap();
        return (session, operation, root.record.unwrap(), start.record.unwrap(), input);
    }
    let root_id = Uuid::now_v7().to_string();
    let root = append(&session.agent_session_id, &root_id, "session_api", &format!("{key}:message"),
        "message/user-accepted", &root_id, Some(ready), input.clone());
    let root = append_stored(store, root, input.clone(), &format!("{key}:root-payload")).await;
    let start_value = json!({"operation_id":operation, "source_message_id":root.event_id});
    let start = append(&session.agent_session_id, &Uuid::now_v7().to_string(), "session_api", &format!("{key}:turn"),
        "turn/started", operation.as_ref(), Some(root.event_id.clone()), start_value.clone());
    let start = append_stored(store, start, start_value, &format!("{key}:start-payload")).await;
    (session, operation, root, start, input)
}

async fn append_stored(
    store: &AgentSessionStore, mut event: SessionEventAppend, value: serde_json::Value, payload_id: &str,
) -> SessionEventRecord {
    let bytes = canonical_json_bytes(&value).unwrap();
    let payload = SessionPayloadRecord {
        payload_id: payload_id.into(), agent_session_id: event.agent_session_id.clone(),
        media_type: "application/json".into(), byte_len: bytes.len() as u64, digest: digest_bytes(&bytes),
        body: SessionPayloadBody::Json(StrictJsonValue(value)),
    };
    event.semantic_event.payload = SessionEventPayloadRef::Stored(payload.payload_id.clone());
    store.append_event_with_payload(&event, Some(&payload)).await.unwrap().record.unwrap()
}

#[tokio::test]
async fn turn_accepted_input_preserves_closed_robot_source_without_reading_successor_or_projection() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, operation, root, start, input) = accepted_fixture(&store, "closed-source", false).await;
    let sid = &session.agent_session_id;
    assert_eq!(store.turn_accepted_input(sid, &operation).await.unwrap(), (root.clone(), StrictJsonValue(input.clone())));
    let terminal = append(sid, "accepted-parent-terminal", "runtime_supervisor", "accepted-parent-terminal",
        "turn/completed", operation.as_ref(), Some(start.event_id), json!({}));
    store.append_turn_terminal(&terminal, &operation).await.unwrap();
    let next = OperationId::from("accepted-successor");
    let next_input = json!({"content":"another request", "input_source":{
        "kind":"authenticated_robot", "companion_id":input["input_source"]["companion_id"],
        "robot_id":"robot-1", "connection_id":"connection-2",
    }});
    let (next_root, _) = store.start_turn(sid, "session_api".into(), "accepted-successor".into(),
        next.clone(), StrictJsonValue(next_input.clone())).await.unwrap();
    sqlx::query("DELETE FROM agent_messages WHERE session_id=?").bind(sid.as_ref())
        .execute(store.test_pool()).await.unwrap();
    let cursor = store.current_cursor(sid).await.unwrap();
    let (original, accepted) = store.turn_accepted_input(sid, &operation).await.unwrap();
    assert_eq!(original, root);
    assert_eq!(accepted.0, input);
    assert_eq!(serde_json::from_value::<AgentInputSource>(accepted.0["input_source"].clone()).unwrap(),
        serde_json::from_value::<AgentInputSource>(input["input_source"].clone()).unwrap());
    assert_eq!(store.turn_accepted_input(sid, &next).await.unwrap(),
        (next_root.record.unwrap(), StrictJsonValue(next_input)));
    assert_eq!(store.current_cursor(sid).await.unwrap(), cursor, "fact reads must not append events");
    assert!(matches!(store.native_recovery_facts(sid, &operation).await, Err(SessionStoreError::ExecutionFenced)),
        "closed-Turn source reads cannot grant native recovery authority");
    assert!(matches!(store.turn_accepted_input(sid, &"missing-operation".into()).await, Err(SessionStoreError::NotFound(_))));
    let (foreign, _) = create_ready(&store, "foreign-source").await;
    assert!(matches!(store.turn_accepted_input(&foreign.agent_session_id, &operation).await, Err(SessionStoreError::NotFound(_))));
}

#[tokio::test]
async fn turn_accepted_input_resolves_stored_root_and_started_payloads() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, operation, root, start, input) = accepted_fixture(&store, "stored-source", true).await;
    assert!(matches!(&root.payload, SessionEventPayloadRef::Stored(_)));
    assert!(matches!(&start.payload, SessionEventPayloadRef::Stored(_)));
    assert_eq!(store.turn_accepted_input(&session.agent_session_id, &operation).await.unwrap(),
        (root, StrictJsonValue(input)));
}

#[tokio::test]
async fn turn_accepted_input_rejects_inconsistent_exact_turn_identity() {
    for defect in ["missing-start", "missing-root", "turn-id", "turn-source", "start-kind", "start-version",
        "start-producer", "start-correlation", "start-operation", "start-source", "root-kind", "root-version",
        "root-producer", "root-correlation", "root-sequence", "root-session", "start-session", "both-producers"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, operation, root, start, _) = accepted_fixture(&store, "identity-source", false).await;
        if matches!(defect, "root-session" | "start-session") {
            let (foreign, _) = create_ready(&store, "identity-foreign").await;
            sqlx::query("UPDATE agent_events SET session_id=? WHERE event_id=?")
                .bind(foreign.agent_session_id.as_ref())
                .bind(if defect == "root-session" {root.event_id.as_ref()} else {start.event_id.as_ref()})
                .execute(store.test_pool()).await.unwrap();
        } else if defect == "both-producers" {
            sqlx::query("UPDATE agent_events SET producer_id='runtime_supervisor' WHERE event_id IN (?,?)")
                .bind(root.event_id.as_ref()).bind(start.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else {
            let (sql, target) = match defect {
                "missing-start" => ("UPDATE agent_turns SET started_event_id=NULL WHERE started_event_id=?", &start.event_id),
                "missing-root" => ("UPDATE agent_events SET causation_event_id=NULL WHERE event_id=?", &start.event_id),
                "turn-id" => ("UPDATE agent_turns SET turn_id='foreign-operation' WHERE started_event_id=?", &start.event_id),
                "turn-source" => ("UPDATE agent_turns SET source_message_id='foreign-root' WHERE started_event_id=?", &start.event_id),
                "start-kind" => ("UPDATE agent_events SET kind='session/ready' WHERE event_id=?", &start.event_id),
                "start-version" => ("UPDATE agent_events SET kind_version=2 WHERE event_id=?", &start.event_id),
                "start-producer" => ("UPDATE agent_events SET producer_id='runtime_supervisor' WHERE event_id=?", &start.event_id),
                "start-correlation" => ("UPDATE agent_events SET correlation_id='foreign-operation' WHERE event_id=?", &start.event_id),
                "start-operation" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.operation_id','foreign-operation') WHERE event_id=?", &start.event_id),
                "start-source" => ("UPDATE agent_events SET inline_json=json_set(inline_json,'$.source_message_id','foreign-root') WHERE event_id=?", &start.event_id),
                "root-kind" => ("UPDATE agent_events SET kind='session/ready' WHERE event_id=?", &root.event_id),
                "root-version" => ("UPDATE agent_events SET kind_version=2 WHERE event_id=?", &root.event_id),
                "root-producer" => ("UPDATE agent_events SET producer_id='runtime_supervisor' WHERE event_id=?", &root.event_id),
                "root-correlation" => ("UPDATE agent_events SET correlation_id='foreign-root' WHERE event_id=?", &root.event_id),
                "root-sequence" => ("UPDATE agent_events SET seq=seq+100 WHERE event_id=?", &root.event_id),
                _ => unreachable!(),
            };
            sqlx::query(sql).bind(target.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        assert!(matches!(store.turn_accepted_input(&session.agent_session_id, &operation).await,
            Err(SessionStoreError::InvalidEvent(_))), "accepted inconsistent {defect}");
    }
}

#[tokio::test]
async fn turn_accepted_input_rejects_stored_payload_integrity_and_ownership_damage() {
    for defect in ["root-digest", "root-length", "root-owner", "started-digest"] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, operation, root, start, _) = accepted_fixture(&store, "damaged-source", true).await;
        let target = if defect == "started-digest" {&start} else {&root};
        let SessionEventPayloadRef::Stored(payload_id) = &target.payload else {panic!("expected stored payload")};
        if defect == "root-owner" {
            let (foreign, _) = create_ready(&store, "payload-foreign").await;
            sqlx::query("UPDATE agent_payloads SET session_id=? WHERE payload_id=?")
                .bind(foreign.agent_session_id.as_ref()).bind(payload_id.as_ref()).execute(store.test_pool()).await.unwrap();
        } else {
            let sql = if defect == "root-length" {
                "UPDATE agent_payloads SET byte_len=byte_len+1 WHERE payload_id=?"
            } else {
                "UPDATE agent_payloads SET digest=(CASE substr(digest,1,1) WHEN '0' THEN '1' ELSE '0' END)||substr(digest,2) WHERE payload_id=?"
            };
            sqlx::query(sql).bind(payload_id.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        assert!(matches!(store.turn_accepted_input(&session.agent_session_id, &operation).await,
            Err(SessionStoreError::InvalidPayload(_))), "accepted damaged {defect}");
    }
}
