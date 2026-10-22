use super::*;

pub(super) fn delegated_params(session: &str, turn: &str, call: &str) -> CreateAgentExecutionParams {
    CreateAgentExecutionParams {
        lead_conversation_id: Some(session.into()),
        initial_plan_input: serde_json::json!({"mode":"explicit","source":{
            "agent_session_id":session,"turn_operation_id":turn,
            "tool_operation_id":format!("tool-{call}"),"effect_id":format!("effect-{call}"),
        },"plan":{"steps":[{"title":"Implement feature","spec":"implement the delegated feature",
            "tool_policy":"full","depends_on":[]}]}}).to_string(),
        ..execution_params()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn top_level_delegation_replays_concurrent_and_completed_commands_without_new_work() {
    let directory = tempfile::tempdir().unwrap();
    let (db, owner) = file_database(&directory.path().join("delegation.sqlite")).await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let session = nomifun_common::generate_id();
    insert_canonical_session(&db, &session, &owner, "live").await;
    let first = delegated_params(&session, "original-turn", "one");
    let mut repeated = delegated_params(&session, "original-turn", "two");
    let parsed: serde_json::Value = serde_json::from_str(&repeated.initial_plan_input).unwrap();
    repeated.initial_plan_input = serde_json::to_string_pretty(&parsed).unwrap();
    let first_participants = [participant(nomifun_common::generate_id())];
    let repeated_participants = [participant(nomifun_common::generate_id())];
    let created = NewAgentExecutionEvent {
        payload: serde_json::json!({"status":"planning","delegation_command_digest":"caller-cannot-author"}).to_string(),
        ..event(AgentExecutionEventKind::Created)
    };
    let (one, two) = tokio::join!(
        repository.create_execution_with_participants(&owner, &first, &first_participants, &created),
        repository.create_execution_with_participants(&owner, &repeated, &repeated_participants, &created),
    );
    let one = one.unwrap();
    let two = two.unwrap();
    assert_eq!(one.execution_id, two.execution_id, "different tool call ids in one Turn identify the same semantic command");
    let events = repository.list_events(&owner, &one.execution_id, 0, 20).await.unwrap();
    assert_eq!(events.len(), 1);
    let baseline: serde_json::Value = serde_json::from_str(&events[0].payload).unwrap();
    assert_eq!(baseline["delegation_command_digest"].as_str().unwrap().len(), 64);
    assert_ne!(baseline["delegation_command_digest"], "caller-cannot-author");
    let participants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_participants WHERE execution_id=?")
        .bind(&one.execution_id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(participants, 1, "a replay does not mint another participant snapshot");

    for field in ["goal", "plan", "policy", "parallel", "workspace", "turn"] {
        let mut changed = delegated_params(&session, "original-turn", field);
        match field {
            "goal" => changed.goal.push_str(" distinct objective"),
            "plan" => {
                let mut plan: serde_json::Value = serde_json::from_str(&changed.initial_plan_input).unwrap();
                plan["plan"]["steps"][0]["spec"] = "a different delegated operation".into();
                changed.initial_plan_input = plan.to_string();
            }
            "policy" => changed.delegation_policy = DelegationPolicy::Disabled,
            "parallel" => changed.max_parallel = 3,
            "workspace" => changed.work_dir = Some("another-workspace".into()),
            "turn" => changed = delegated_params(&session, "successor-turn", field),
            _ => unreachable!(),
        }
        assert!(matches!(repository.create_execution_with_participants(&owner, &changed,
            &[participant(nomifun_common::generate_id())], &created).await, Err(nomifun_db::DbError::Conflict(_))),
            "{field} changes command identity and remains subject to the active Execution guard");
    }

    // Simulate the final domain row after workspace preparation and replanning.
    // Only the original Created digest can identify the earlier request now.
    sqlx::query("UPDATE agent_executions SET status='completed',goal='revised goal',work_dir='prepared/absolute/workspace',version=version+5 WHERE execution_id=?")
        .bind(&one.execution_id).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE execution_id=?")
        .bind(&one.execution_id).execute(db.pool()).await.unwrap();
    let finished = repository.create_execution_with_participants(&owner, &repeated,
        &repeated_participants, &created).await.unwrap();
    assert_eq!(finished.execution_id, one.execution_id);
    assert_eq!(finished.status, "completed");
    assert_eq!(finished.goal, "revised goal");
    assert_eq!(finished.version, one.version + 5);
    let mut distinct = repeated.clone();
    distinct.goal.push_str(" another independent objective");
    let successor = repository.create_execution_with_participants(&owner, &distinct,
        &[participant(nomifun_common::generate_id())], &created).await.unwrap();
    assert_ne!(successor.execution_id, one.execution_id);
    assert_eq!(repository.create_execution_with_participants(&owner, &first,
        &first_participants, &created).await.unwrap().execution_id, one.execution_id,
        "acknowledging completed work cannot disturb a different active Execution");
    assert_eq!(repository.list_events(&owner, &one.execution_id, 0, 20).await.unwrap().len(), 1);
    db.close().await;
}

#[tokio::test]
async fn top_level_delegation_does_not_reconstruct_a_missing_original_command_digest() {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let session = nomifun_common::generate_id();
    insert_canonical_session(&db, &session, OWNER_ID, "live").await;
    let params = delegated_params(&session, "original-turn", "first");
    let participants = [participant(nomifun_common::generate_id())];
    let created = event(AgentExecutionEventKind::Created);
    let original = repository.create_execution_with_participants(OWNER_ID, &params, &participants, &created).await.unwrap();
    sqlx::query("UPDATE agent_execution_events SET payload=json_remove(payload,'$.delegation_command_digest') WHERE execution_id=?")
        .bind(&original.execution_id).execute(db.pool()).await.unwrap();
    assert!(matches!(repository.create_execution_with_participants(OWNER_ID, &params, &participants, &created).await,
        Err(nomifun_db::DbError::Conflict(_))), "old history cannot gain replay authority from mutable rows");
    let baseline = repository.list_events(OWNER_ID, &original.execution_id, 0, 20).await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&baseline[0].payload).unwrap().get("delegation_command_digest").is_none());
    db.close().await;
}
