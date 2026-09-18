use nomifun_common::{
    AdaptationPolicy, AgentExecutionActor, AgentExecutionEventKind, AgentExecutionStatus,
    DecisionPolicy, DelegationPolicy,
};
use nomifun_db::{
    CreateAgentExecutionParams, CreateAgentExecutionTemplateParams,
    IAgentExecutionRepository, IAgentExecutionTemplateRepository,
    IClientPreferenceRepository, IProviderRepository,
    NewAgentExecutionEvent, NewAgentExecutionParticipant,
    NewAgentExecutionTemplateParticipant, SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SqliteClientPreferenceRepository, SqliteProviderRepository,
    UpdateAgentExecutionParams, init_database_memory,
};

const NOMI_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";

fn encrypted_bearer_credentials() -> String {
    nomifun_common::encrypt_string(
        r#"{"api_keys":["test-only"]}"#,
        &[0x42; 32],
    )
    .expect("typed test credentials should encrypt")
}

async fn insert_provider(database: &nomifun_db::Database, id: &str) {
    let credentials_encrypted = encrypted_bearer_credentials();
    nomifun_db::sqlx::query(
        "INSERT INTO providers (\
            provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
            created_at, updated_at\
         ) VALUES (?, 'openai', ?, 'https://example.invalid', 'bearer', ?, \
                    1, 1, 1)",
    )
    .bind(id)
    .bind(id)
    .bind(credentials_encrypted)
    .execute(database.pool())
    .await
    .unwrap();
}

fn template_participant(provider_id: &str) -> NewAgentExecutionTemplateParticipant {
    NewAgentExecutionTemplateParticipant {
        template_participant_id: uuid::Uuid::now_v7().to_string(),
        source_agent_id: NOMI_AGENT_ID.to_owned(),
        preset_id: None,
        preset_revision: None,
        agent_snapshot: None,
        provider_id: Some(provider_id.to_owned()),
        model: Some("model".to_owned()),
        role: None,
        capability: None,
        constraints: None,
        description: None,
        system_prompt: None,
        enabled_skills: "[]".to_owned(),
        disabled_builtin_skills: "[]".to_owned(),
        sort_order: 0,
    }
}

fn execution_participant(provider_id: &str) -> NewAgentExecutionParticipant {
    NewAgentExecutionParticipant {
        participant_id: uuid::Uuid::now_v7().to_string(),
        source_agent_id: NOMI_AGENT_ID.to_owned(),
        preset_id: None,
        preset_revision: None,
        agent_snapshot: None,
        provider_id: Some(provider_id.to_owned()),
        model: Some("model".to_owned()),
        role: None,
        capability: None,
        constraints: None,
        description: None,
        system_prompt: None,
        enabled_skills: "[]".to_owned(),
        disabled_builtin_skills: "[]".to_owned(),
        sort_order: 0,
    }
}

fn event(kind: AgentExecutionEventKind) -> NewAgentExecutionEvent {
    NewAgentExecutionEvent {
        event_type: kind,
        step_id: None,
        attempt_id: None,
        actor: AgentExecutionActor::system(),
        payload: "{}".to_owned(),
    }
}

#[tokio::test]
async fn provider_bindings_are_validated_and_delete_is_atomic_after_a_stale_scan() {
    let database = init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
    insert_provider(&database, "0190f5fe-7c00-7a00-8000-000000000002").await;
    insert_provider(&database, "0190f5fe-7c00-7a00-8000-000000000001").await;
    let templates = SqliteAgentExecutionTemplateRepository::new(database.pool().clone());
    let executions = SqliteAgentExecutionRepository::new(database.pool().clone());
    let providers = SqliteProviderRepository::new(database.pool().clone());
    let preferences = SqliteClientPreferenceRepository::new(database.pool().clone());

    assert!(
        preferences
            .upsert_batch(&[(
                "nomi.defaultModel",
                r#"{"provider_id":"0190f5fe-7c00-7a00-8000-000000000003","model":"model"}"#,
            )])
            .await
            .is_err(),
        "the authoritative preference repository requires an existing provider"
    );

    assert!(
        templates
            .create_template(
                &owner,
                &CreateAgentExecutionTemplateParams {
                    name: "missing provider".to_owned(),
                    description: None,
                    max_parallel: Some(1),
                    work_dir: None,
                    context: None,
                    participants: vec![template_participant("0190f5fe-7c00-7a00-8000-000000000003")],
                },
            )
            .await
            .is_err(),
        "new Template bindings require an existing provider"
    );

    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES (\
            'agent.model_failover', \
            '{\"enabled\":true,\"queue\":[{\"provider_id\":\"0190f5fe-7c00-7a00-8000-000000000002\",\"model\":\"model\"},{\"provider_id\":\"0190f5fe-7c00-7a00-8000-000000000001\",\"model\":\"model\"}],\"max_switches\":4}', \
            1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) \
         VALUES ('nomi.collaborationModels', ?, 1)",
    )
    .bind(
        serde_json::json!([
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000001", "model": "model_first"},
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000002", "model": "model"},
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000003", "model": "model"},
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000001", "model": "model_second"}
        ])
        .to_string(),
    )
    .execute(database.pool())
    .await
    .unwrap();

    // This is the race-equivalent path: an application usage scan can observe
    // no hard binding, then a soft reference exists before the raw DELETE.
    providers
        .delete("0190f5fe-7c00-7a00-8000-000000000002")
        .await
        .unwrap();
    let failover: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'agent.model_failover'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&failover).unwrap()["queue"],
        serde_json::json!([{"provider_id": "0190f5fe-7c00-7a00-8000-000000000001", "model": "model"}])
    );
    let collaboration_models: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'nomi.collaborationModels'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&collaboration_models).unwrap(),
        serde_json::json!([
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000001", "model": "model_first"},
            {"provider_id": "0190f5fe-7c00-7a00-8000-000000000001", "model": "model_second"}
        ]),
        "provider deletion preserves candidate order while pruning the deleted and already-missing providers"
    );

    insert_provider(&database, "0190f5fe-7c00-7a00-8000-000000000002").await;
    let template = templates
        .create_template(
            &owner,
            &CreateAgentExecutionTemplateParams {
                name: "hard template".to_owned(),
                description: None,
                max_parallel: Some(1),
                work_dir: None,
                context: None,
                participants: vec![template_participant("0190f5fe-7c00-7a00-8000-000000000002")],
            },
        )
        .await
        .unwrap();
    assert!(
        providers
            .delete("0190f5fe-7c00-7a00-8000-000000000002")
            .await
            .is_err(),
        "the DB closes a Template usage-scan/delete race"
    );
    assert!(
        templates
            .delete_template(
                &owner,
                &template.template.execution_template_id,
                template.template.version,
            )
            .await
            .unwrap()
    );

    let execution = executions
        .create_execution_with_participants(
            &owner,
            &CreateAgentExecutionParams {
                goal: "hard execution".to_owned(),
                status: AgentExecutionStatus::Planning,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[execution_participant("0190f5fe-7c00-7a00-8000-000000000002")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap();
    assert!(
        providers
            .delete("0190f5fe-7c00-7a00-8000-000000000002")
            .await
            .is_err(),
        "the DB closes an Agent Execution usage-scan/delete race"
    );
    executions
        .update_execution(
            &owner,
            &execution.execution_id,
            execution.version,
            None,
            &UpdateAgentExecutionParams {
                status: Some(AgentExecutionStatus::Cancelled),
                ..Default::default()
            },
            &event(AgentExecutionEventKind::StatusChanged),
        )
        .await
        .unwrap();
    providers
        .delete("0190f5fe-7c00-7a00-8000-000000000002")
        .await
        .unwrap();
    let historical_provider_id: Option<String> = nomifun_db::sqlx::query_scalar(
        "SELECT provider_id FROM agent_execution_participants WHERE execution_id = ?",
    )
    .bind(&execution.execution_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        historical_provider_id.as_deref(),
        Some("0190f5fe-7c00-7a00-8000-000000000002"),
        "cancelled execution participants keep their historical provider snapshot"
    );
    assert!(
        providers
            .find_by_id("0190f5fe-7c00-7a00-8000-000000000002")
            .await
            .unwrap()
            .is_none(),
        "KEEP_HISTORY does not require retaining the live provider catalog row"
    );

    let missing_execution = executions
        .create_execution_with_participants(
            &owner,
            &CreateAgentExecutionParams {
                goal: "missing provider".to_owned(),
                status: AgentExecutionStatus::Planning,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[execution_participant("0190f5fe-7c00-7a00-8000-000000000003")],
            &event(AgentExecutionEventKind::Created),
        )
        .await;
    assert!(
        missing_execution.is_err(),
        "new reopenable Execution bindings require an existing provider"
    );
}

#[tokio::test]
async fn provider_delete_keeps_empty_collaboration_models_preference_as_an_array() {
    let database = init_database_memory().await.unwrap();
    insert_provider(&database, "0190f5fe-7c00-7a00-8000-000000000002").await;
    let providers = SqliteProviderRepository::new(database.pool().clone());
    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) \
         VALUES ('nomi.collaborationModels', \
                 '[{\"provider_id\":\"0190f5fe-7c00-7a00-8000-000000000002\",\"model\":\"model\"}]', 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();

    providers
        .delete("0190f5fe-7c00-7a00-8000-000000000002")
        .await
        .unwrap();

    let value: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'nomi.collaborationModels'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(value, "[]");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&value).unwrap(),
        serde_json::json!([])
    );
}

#[tokio::test]
async fn knowledge_retrieval_preference_requires_both_explicit_stages() {
    const RETRIEVAL_KEY: &str = "knowledge.retrieval";

    let database = init_database_memory().await.unwrap();
    let preferences = SqliteClientPreferenceRepository::new(database.pool().clone());

    for incomplete in [
        serde_json::json!({}),
        serde_json::json!({"embedding": {"mode": "local"}}),
        serde_json::json!({"rerank": {"mode": "local"}}),
    ] {
        let value = incomplete.to_string();
        assert!(
            preferences
                .upsert_batch(&[(RETRIEVAL_KEY, &value)])
                .await
                .is_err(),
            "the repository write boundary accepted an incomplete retrieval preference: {incomplete}"
        );
    }

    assert!(
        preferences
            .get_by_keys(&[RETRIEVAL_KEY])
            .await
            .unwrap()
            .is_empty(),
        "rejected configurations must not leave a partial preference row"
    );
}

#[tokio::test]
async fn provider_delete_downgrades_only_the_affected_knowledge_retrieval_stage() {
    const PROVIDER_A: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PROVIDER_B: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const RETRIEVAL_KEY: &str = "knowledge.retrieval";

    let database = init_database_memory().await.unwrap();
    insert_provider(&database, PROVIDER_A).await;
    insert_provider(&database, PROVIDER_B).await;
    let providers = SqliteProviderRepository::new(database.pool().clone());
    let preferences = SqliteClientPreferenceRepository::new(database.pool().clone());
    let retrieval_preference = serde_json::json!({
        "embedding": {
            "mode": "remote",
            "provider_id": PROVIDER_A,
            "model": "embedding-model"
        },
        "rerank": {
            "mode": "remote",
            "provider_id": PROVIDER_B,
            "model": "rerank-model"
        }
    })
    .to_string();

    preferences
        .upsert_batch(&[(RETRIEVAL_KEY, &retrieval_preference)])
        .await
        .unwrap();

    providers.delete(PROVIDER_A).await.unwrap();

    let after_provider_a: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'knowledge.retrieval'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&after_provider_a).unwrap(),
        serde_json::json!({
            "embedding": {"mode": "local"},
            "rerank": {
                "mode": "remote",
                "provider_id": PROVIDER_B,
                "model": "rerank-model"
            }
        }),
        "deleting the embedding Provider must preserve the independent rerank stage"
    );

    providers.delete(PROVIDER_B).await.unwrap();

    let after_provider_b: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'knowledge.retrieval'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&after_provider_b).unwrap(),
        serde_json::json!({
            "embedding": {"mode": "local"},
            "rerank": {"mode": "local"}
        }),
        "deleting the remaining Provider must leave an explicit all-local configuration"
    );
}
