use super::*;

#[tokio::test]
async fn unclassified_terminal_history_preserves_detail_without_inventing_source_or_retry_permission() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "unclassified-terminal").await;
    let sid = &session.agent_session_id;
    let operation = OperationId::from("unclassified-terminal-turn");
    let (input, start) = store.start_turn(sid, "session_api".into(), "unclassified-terminal-input".into(), operation.clone(),
        StrictJsonValue(json!({"content":"Inspect the cause"}))).await.unwrap();
    let source = input.ack.unwrap().event_id;
    store.append_event(&append(sid, "unclassified-terminal-failed", "runtime_supervisor", "unclassified-terminal-failed",
        "turn/failed", operation.as_ref(), Some(start.ack.unwrap().event_id),
        json!({"message":"Preserved original failure detail"}))).await.unwrap();
    let facts = store.read_events(sid, None, 500).await.unwrap();
    let summaries = store.turn_history_for_sources(sid, &[source.as_ref().to_owned()]).await.unwrap();
    assert_eq!(summaries[0].projection["error"], json!({
        "message":"Preserved original failure detail", "code":"AGENT_FAILURE_SOURCE_UNRECORDED", "retryable":false,
    }));
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.read_events(sid, None, 500).await.unwrap(), facts);
}

#[tokio::test]
async fn terminal_mirrors_are_absent_and_turn_summary_resolves_the_exact_canonical_payload() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    for (table, removed) in [("agent_turns", vec!["result_json", "error_json"]), ("agent_effects", vec!["bounded_observation_json"])] {
        let columns: Vec<String> = sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .fetch_all(store.test_pool()).await.unwrap();
        for name in removed { assert!(!columns.iter().any(|column| column == name)); }
    }
    let (session, _) = create_ready(&store, "terminal-content").await;
    let sid = &session.agent_session_id;
    let operation = OperationId::from("terminal-content-turn");
    let (input, start) = store.start_turn(sid, "session_api".into(), "terminal-content-input".into(), operation.clone(),
        StrictJsonValue(json!({"content":"Inspect the source"}))).await.unwrap();
    let source = input.ack.unwrap().event_id;
    let start = start.ack.unwrap().event_id;
    let error = json!({"message":"The exact canonical failure", "code":"NOMIFUN_STATE_INCONSISTENT", "retryable":false});
    let terminal = append(sid, "terminal-content-failed", "runtime_supervisor", "terminal-content-failed",
        "turn/failed", operation.as_ref(), Some(start), json!({"error":error,"finished_at_ms":1_990_000_000_000_i64}));
    store.append_event(&terminal).await.unwrap();
    let summaries = store.turn_history_for_sources(sid, &[source.as_ref().to_owned()]).await.unwrap();
    assert_eq!(summaries[0].projection["error"], error);
    assert_eq!(summaries[0].projection["finished_at_ms"], 1_990_000_000_000_i64);
    let same = store.turn_history_for_operation(&owner(), sid, &operation).await.unwrap().unwrap();
    assert_eq!(same.projection["error"], error);
    let (foreign, _) = create_turn(&store, "terminal-content-foreign", "terminal-content-foreign-turn").await;
    let foreign_start: String = sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=?")
        .bind(foreign.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    sqlx::query("UPDATE agent_turns SET terminal_event_id=? WHERE session_id=? AND operation_id=?")
        .bind(foreign_start).bind(sid.as_ref()).bind(operation.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.turn_history_for_sources(sid, &[source.as_ref().to_owned()]).await.is_err(),
        "a foreign lifecycle reference must fail instead of falling back to a cached result");
    assert!(store.turn_history_for_operation(&owner(), sid, &operation).await.is_err());
}

#[tokio::test]
async fn effects_resolve_terminal_observations_and_fail_closed_on_mismatched_indexes() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, request, started) = create_pending_effect(&store, "effect-terminal-content", EffectStrategy::ManagedEffect).await;
    let sid = &session.agent_session_id;
    assert!(store.read_effect(sid, &request.effect_id).await.unwrap().unwrap().bounded_observation.is_none());
    let body = "Canonical file-write result ".repeat(2000);
    let result = json!({"output":{"text":body},"written":true});
    let terminal = EffectEventRequest { event_id: "effect-terminal-content-result".into(), producer_id: "capability-owner".into(),
        recorded_at: request.recorded_at + 1, causation_event_id: Some(started.clone()),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(result)), ..request.clone() };
    store.record_effect_terminal(terminal, EffectTerminalState::Succeeded).await.unwrap();
    let effect = store.read_effect(sid, &request.effect_id).await.unwrap().unwrap();
    assert_eq!(effect.state, AgentEffectState::Returned);
    assert_eq!(effect.bounded_observation.as_ref().unwrap()["output"]["text"], body);
    assert_eq!(store.list_effects(sid).await.unwrap(), vec![effect]);
    sqlx::query("UPDATE agent_effects SET terminal_event_id=? WHERE session_id=? AND effect_id=?")
        .bind(started.as_ref()).bind(sid.as_ref()).bind(&request.effect_id).execute(store.test_pool()).await.unwrap();
    assert!(store.read_effect(sid, &request.effect_id).await.is_err(), "a start event is not a terminal result");
    assert!(store.list_effects(sid).await.is_err());
}

#[tokio::test]
async fn terminal_mirror_migration_preserves_facts_and_rolls_back_for_a_damaged_reference() {
    for damaged in [false, true] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let key = if damaged { "mirror-migration-damaged" } else { "mirror-migration-valid" };
        let (session, _) = create_ready(&store, key).await;
        let sid = &session.agent_session_id;
        let operation = OperationId::from(key);
        let (_, started) = store.start_turn(sid, "session_api".into(), format!("{key}-input").into(), operation.clone(),
            StrictJsonValue(json!({"content":"Keep the canonical record"}))).await.unwrap();
        let start = started.ack.unwrap().event_id;
        let payload = json!({"model_steps":0,"finished_at_ms":1_990_000_000_000_i64});
        store.append_event(&append(sid, &format!("{key}-done"), "runtime_supervisor", &format!("{key}-done"),
            "turn/completed", operation.as_ref(), Some(start.clone()), payload.clone())).await.unwrap();
        let facts = store.read_events(sid, None, 500).await.unwrap();
        // Recreate only the published mirror columns in this isolated fixture;
        // the migration must consume canonical references, never their copies.
        for statement in ["ALTER TABLE agent_turns ADD COLUMN result_json TEXT", "ALTER TABLE agent_turns ADD COLUMN error_json TEXT",
            "ALTER TABLE agent_effects ADD COLUMN bounded_observation_json TEXT"] {
            sqlx::query(statement).execute(store.test_pool()).await.unwrap();
        }
        let mirrored = serde_json::to_string(&payload).unwrap();
        sqlx::query("UPDATE agent_turns SET result_json=? WHERE session_id=? AND operation_id=?")
            .bind(&mirrored).bind(sid.as_ref()).bind(operation.as_ref()).execute(store.test_pool()).await.unwrap();
        if damaged {
            sqlx::query("UPDATE agent_turns SET terminal_event_id=? WHERE session_id=? AND operation_id=?")
                .bind(start.as_ref()).bind(sid.as_ref()).bind(operation.as_ref()).execute(store.test_pool()).await.unwrap();
        }
        let mut tx = store.test_pool().begin().await.unwrap();
        let migration = sqlx::raw_sql(include_str!("../../nomifun-db/migrations/007_retire_terminal_content_mirrors.sql"))
            .execute(&mut *tx).await;
        if damaged {
            assert!(migration.is_err());
            tx.rollback().await.unwrap();
            let preserved: String = sqlx::query_scalar("SELECT result_json FROM agent_turns WHERE session_id=? AND operation_id=?")
                .bind(sid.as_ref()).bind(operation.as_ref()).fetch_one(store.test_pool()).await.unwrap();
            assert_eq!(preserved, mirrored, "failed upgrades preserve diagnostic bytes");
        } else {
            migration.unwrap();
            tx.commit().await.unwrap();
            let removed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('agent_turns') WHERE name IN ('result_json','error_json')")
                .fetch_one(store.test_pool()).await.unwrap();
            assert_eq!(removed, 0);
        }
        assert_eq!(store.read_events(sid, None, 500).await.unwrap(), facts);
    }
}
