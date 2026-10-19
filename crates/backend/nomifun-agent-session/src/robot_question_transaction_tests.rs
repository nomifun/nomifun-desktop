//! The question-playback writer joins its caller's existing SQLite transaction.

use super::*;
use std::time::Duration;

async fn question_proof_fixture(key: &str) -> (AgentSessionStore, AgentSessionId, SessionEventAppend) {
    // This pool has one connection. Acquiring another connection from inside
    // the writer would block until the bounded append timeout below.
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready) = create_ready(&store, key).await;
    let session_id = session.agent_session_id;
    let message_id = Uuid::now_v7().to_string();
    let question = append(&session_id, &message_id, "session_api", &format!("question-{key}"),
        "message/assistant-projected", &message_id, Some(ready),
        json!({"content":"Which document should I use?"}));
    let notice = store.append_event(&question).await.unwrap().ack.unwrap();
    let proof = append(&session_id, &format!("playback-{key}"), "session_api",
        &format!("playback-{key}"), "robot/question-playback-accepted", &message_id,
        Some(notice.event_id), json!({"notice_message_id":message_id,"notice_seq":notice.seq,
            "connection_id":"original-connection","presentation_id":"original-presentation"}));
    (store, session_id, proof)
}

#[tokio::test]
async fn question_playback_append_uses_the_owned_transaction_and_only_commit_persists_it() {
    let (store, session, proof) = question_proof_fixture("owned-question-transaction").await;
    let before_head = store.head(&session).await.unwrap();
    let before_events = store.read_events(&session, None, 100).await.unwrap();

    let mut transaction = store.test_pool().begin().await.unwrap();
    let appended = tokio::time::timeout(Duration::from_secs(2),
        store.append_robot_question_playback_in_transaction(&mut transaction, &proof))
        .await.expect("the playback writer must not reacquire the only connection").unwrap();
    assert!(appended.persisted);
    assert!(!appended.duplicate);
    assert_eq!(appended.cursor.seq, before_head.last_seq + 1);
    let replay = tokio::time::timeout(Duration::from_secs(2),
        store.append_robot_question_playback_in_transaction(&mut transaction, &proof))
        .await.expect("in-transaction replay must not reacquire the pool").unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.ack, appended.ack);
    assert_eq!(replay.cursor, appended.cursor);
    let in_transaction_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='robot/question-playback-accepted'")
        .bind(session.as_ref()).fetch_one(&mut *transaction).await.unwrap();
    assert_eq!(in_transaction_count, 1);
    transaction.rollback().await.unwrap();

    assert_eq!(store.head(&session).await.unwrap(), before_head);
    assert_eq!(store.read_events(&session, None, 100).await.unwrap(), before_events,
        "the writer cannot commit facts or projections owned by its caller");

    let mut transaction = store.test_pool().begin().await.unwrap();
    let committed = tokio::time::timeout(Duration::from_secs(2),
        store.append_robot_question_playback_in_transaction(&mut transaction, &proof))
        .await.expect("the commit attempt must use the same owned connection").unwrap();
    assert!(!committed.duplicate);
    transaction.commit().await.unwrap();
    let after = store.read_events(&session, None, 100).await.unwrap();
    assert_eq!(after.events.len(), before_events.events.len() + 1);
    let recorded = after.events.iter().filter(|event|
        event.kind.0 == "robot/question-playback-accepted").collect::<Vec<_>>();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].event_id, proof.event_id);
    assert_eq!(recorded[0].payload, proof.semantic_event.payload);
    assert_eq!(store.head(&session).await.unwrap().last_seq, before_head.last_seq + 1);

    let mut transaction = store.test_pool().begin().await.unwrap();
    let replay = store.append_robot_question_playback_in_transaction(&mut transaction, &proof)
        .await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.ack, committed.ack);
    transaction.commit().await.unwrap();
    assert_eq!(store.read_events(&session, None, 100).await.unwrap(), after);
}

#[tokio::test]
async fn question_playback_transaction_api_rejects_other_writers_and_payload_storage_without_changes() {
    let (store, session, proof) = question_proof_fixture("restricted-question-transaction").await;
    let before_head = store.head(&session).await.unwrap();
    let before_events = store.read_events(&session, None, 100).await.unwrap();
    let mut rejected = Vec::new();
    for kind in ["runtime/progress-recorded", "turn/completed", "context/cleared"] {
        let mut candidate = proof.clone();
        candidate.semantic_event.kind = SessionEventKind(kind.into());
        rejected.push(candidate);
    }
    let mut wrong_producer = proof.clone();
    wrong_producer.producer_id = "runtime-supervisor".into();
    rejected.push(wrong_producer);
    let mut wrong_version = proof.clone();
    wrong_version.semantic_event.kind_version = 2;
    rejected.push(wrong_version);
    let mut stored_payload = proof.clone();
    stored_payload.semantic_event.payload = SessionEventPayloadRef::Stored("unavailable-payload".into());
    rejected.push(stored_payload);
    let mut empty_payload = proof;
    empty_payload.semantic_event.payload = SessionEventPayloadRef::Empty;
    rejected.push(empty_payload);

    let mut transaction = store.test_pool().begin().await.unwrap();
    for candidate in &rejected {
        let result = tokio::time::timeout(Duration::from_secs(2),
            store.append_robot_question_playback_in_transaction(&mut transaction, candidate))
            .await.expect("rejection cannot acquire another connection");
        assert!(matches!(result, Err(SessionStoreError::ExecutionFenced)),
            "the narrow transaction writer accepted an unrelated event: {candidate:?}");
    }
    // Commit instead of rollback so accidental partial writes remain visible.
    transaction.commit().await.unwrap();
    assert_eq!(store.head(&session).await.unwrap(), before_head);
    assert_eq!(store.read_events(&session, None, 100).await.unwrap(), before_events);
}

#[tokio::test]
async fn caller_transaction_resolves_inline_stored_and_shared_content_without_reacquiring_or_writing() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "owned-payload-reader").await;
    let session = session.agent_session_id;
    let (accepted, started) = store.start_turn(&session, "session_api".into(),
        "owned-reader-input".into(), "owned-reader-turn".into(),
        StrictJsonValue(json!({"content":"Read the exact saved sources"}))).await.unwrap();
    let started = started.ack.unwrap().event_id;
    let stored_value = json!({"content":"legitimate stored message body"});
    let logical = canonical_json_bytes(&stored_value).unwrap();
    let payload = SessionPayloadRecord {
        payload_id: "owned-reader-stored-body".into(), agent_session_id: session.clone(),
        media_type: "application/json".into(), byte_len: logical.len() as u64,
        digest: digest_bytes(&logical), body: SessionPayloadBody::Json(StrictJsonValue(stored_value.clone())),
    };
    let mut stored = append(&session, "owned-reader-stored-event", "runtime_supervisor",
        "owned-reader-stored-event", "message/content-part", "owned-reader-message",
        Some(started.clone()), stored_value.clone());
    stored.semantic_event.payload = SessionEventPayloadRef::Stored(payload.payload_id.clone());
    store.append_event_with_payload(&stored, Some(&payload)).await.unwrap();
    let shared_value = json!({"producer_seq":1,"event":{"event":"tool_completed","step":1,
        "result":{"call_id":"owned-reader-call","output":[{"type":"text",
            "text":"exact shared tool source".repeat(400)}],"is_error":false}}});
    let shared = append(&session, "owned-reader-shared-event", "runtime_supervisor",
        "owned-reader-shared-event", "runtime/progress-recorded", "owned-reader-turn",
        Some(started), shared_value.clone());
    store.append_event(&shared).await.unwrap();
    let shared_leaves: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_payloads WHERE session_id=? AND payload_id LIKE 'shared-tool:%'")
        .bind(session.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert!(shared_leaves > 0, "the fixture must exercise actual shared-content hydration");
    let ids = [accepted.ack.unwrap().event_id, stored.event_id.clone(), shared.event_id.clone()];
    let original_reader = store.resolved_event_payloads(&session, &ids).await.unwrap();
    assert_eq!(original_reader[ids[0].as_ref()]["content"], "Read the exact saved sources");
    assert_eq!(original_reader[stored.event_id.as_ref()], stored_value);
    assert_eq!(original_reader[shared.event_id.as_ref()], shared_value);
    let (foreign, foreign_ready) = create_ready(&store, "owned-reader-foreign").await;
    let before_head = store.head(&session).await.unwrap();
    let before_events = store.read_events(&session, None, 100).await.unwrap();
    let foreign_before = store.read_events(&foreign.agent_session_id, None, 100).await.unwrap();
    let payloads_before: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(byte_len),0) FROM agent_payloads")
        .fetch_one(store.test_pool()).await.unwrap();

    let mut transaction = store.test_pool().begin().await.unwrap();
    let resolved = tokio::time::timeout(Duration::from_secs(2),
        store.resolved_event_payloads_in_transaction(&mut transaction, &session, &ids))
        .await.expect("the reader must use the caller's only connection").unwrap();
    assert_eq!(resolved, original_reader);
    for invalid in [event_id("owned-reader-missing-event"), foreign_ready] {
        assert!(tokio::time::timeout(Duration::from_secs(2),
            store.resolved_event_payloads_in_transaction(&mut transaction, &session, &[invalid]))
            .await.expect("failed resolution must not reacquire the pool").is_err());
    }
    transaction.commit().await.unwrap();
    assert_eq!(store.head(&session).await.unwrap(), before_head);
    assert_eq!(store.read_events(&session, None, 100).await.unwrap(), before_events);
    assert_eq!(store.read_events(&foreign.agent_session_id, None, 100).await.unwrap(), foreign_before);
    let payloads_after: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(byte_len),0) FROM agent_payloads")
        .fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(payloads_after, payloads_before, "read success and rejection cannot add canonical facts");
}
