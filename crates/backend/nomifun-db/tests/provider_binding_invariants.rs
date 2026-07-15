use nomifun_common::{
    AdaptationPolicy, AgentExecutionActor, AgentExecutionEventKind, AgentExecutionStatus,
    ConversationId, DecisionPolicy, DelegationPolicy, PlanGate,
};
use nomifun_db::models::ConversationRow;
use nomifun_db::{
    CreateAgentExecutionParams, CreateAgentExecutionTemplateParams,
    DbError, IAgentExecutionRepository, IAgentExecutionTemplateRepository,
    IConversationRepository, IProviderRepository,
    NewAgentExecutionEvent, NewAgentExecutionParticipant,
    NewAgentExecutionTemplateParticipant, SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository, SqliteConversationRepository,
    SqliteProviderRepository,
    UpdateAgentExecutionParams, init_database_memory,
};

async fn insert_provider(database: &nomifun_db::Database, id: &str) {
    nomifun_db::sqlx::query(
        "INSERT INTO providers (\
            id, platform, name, base_url, api_key_encrypted, models, enabled, \
            capabilities, created_at, updated_at\
         ) VALUES (?, 'openai', ?, 'https://example.invalid', 'encrypted', \
                   '[]', 1, '[]', 1, 1)",
    )
    .bind(id)
    .bind(id)
    .execute(database.pool())
    .await
    .unwrap();
}

fn conversation(
    installation_owner: &str,
    name: &str,
    model: Option<serde_json::Value>,
    execution_model_pool: Option<serde_json::Value>,
) -> ConversationRow {
    ConversationRow {
        id: ConversationId::new().into_string(),
        user_id: installation_owner.to_owned(),
        name: name.to_owned(),
        r#type: "nomi".to_owned(),
        extra: "{}".to_owned(),
        delegation_policy: "automatic".to_owned(),
        execution_model_pool: execution_model_pool.map(|value| value.to_string()),
        decision_policy: "automatic".to_owned(),
        execution_template_id: None,
        model: model.map(|value| value.to_string()),
        status: Some("pending".to_owned()),
        source: Some("nomifun".to_owned()),
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        created_at: 1,
        updated_at: 1,
    }
}

fn template_participant(provider_id: &str) -> NewAgentExecutionTemplateParticipant {
    NewAgentExecutionTemplateParticipant {
        source_agent_id: "nomi".to_owned(),
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
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
        id: format!("participant_{provider_id}"),
        source_agent_id: "nomi".to_owned(),
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
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
    insert_provider(&database, "prov_0190f5fe-7c00-7a00-8000-000000000002").await;
    insert_provider(&database, "prov_0190f5fe-7c00-7a00-8000-000000000001").await;
    let conversations = SqliteConversationRepository::new(database.pool().clone());
    let templates = SqliteAgentExecutionTemplateRepository::new(database.pool().clone());
    let executions = SqliteAgentExecutionRepository::new(database.pool().clone());
    let providers = SqliteProviderRepository::new(database.pool().clone());

    assert!(
        nomifun_db::sqlx::query(
            "INSERT INTO client_preferences (key, value, updated_at) \
             VALUES ('idmm_backup_provider_id', 'prov_0190f5fe-7c00-7a00-8000-000000000003', 1)",
        )
        .execute(database.pool())
        .await
        .is_err(),
        "new IDMM backup bindings require an existing provider"
    );

    assert!(
        conversations
            .create(&conversation(
                &owner,
                "missing lead",
                Some(serde_json::json!({
                    "provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000003",
                    "model": "model"
                })),
                None,
            ))
            .await
            .is_err(),
        "new Conversation lead bindings require an existing provider"
    );
    assert!(
        conversations
            .create(&conversation(
                &owner,
                "missing collaborator",
                Some(serde_json::json!({
                    "provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001",
                    "model": "model"
                })),
                Some(serde_json::json!({
                    "mode": "range",
                    "models": [
                        {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model"},
                        {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000003", "model": "model"}
                    ]
                })),
            ))
            .await
            .is_err(),
        "new Conversation model pools require every provider to exist"
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
                    participants: vec![template_participant("prov_0190f5fe-7c00-7a00-8000-000000000003")],
                },
            )
            .await
            .is_err(),
        "new Template bindings require an existing provider"
    );

    let soft_ref_conversation = conversations
        .create(&conversation(
            &owner,
            "soft references",
            Some(serde_json::json!({
                "provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001",
                "model": "model"
            })),
            Some(serde_json::json!({
                "mode": "range",
                "models": [
                    {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model"},
                    {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000002", "model": "model"}
                ]
            })),
        ))
        .await
        .unwrap();
    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) VALUES (\
            'agent.model_failover', \
            '{\"enabled\":true,\"queue\":[{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8000-000000000002\",\"model\":\"model\"},{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8000-000000000001\",\"model\":\"model\"}],\"max_switches\":4,\"stamp_unhealthy\":true}', \
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
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model_first"},
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000002", "model": "model"},
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000003", "model": "model"},
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model_second"}
        ])
        .to_string(),
    )
    .execute(database.pool())
    .await
    .unwrap();

    // This is the race-equivalent path: an application usage scan can observe
    // no hard binding, then a soft reference exists before the raw DELETE.
    nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
        .execute(database.pool())
        .await
        .unwrap();
    let pool: serde_json::Value = serde_json::from_str(
        &conversations
            .get(&soft_ref_conversation)
            .await
            .unwrap()
            .unwrap()
            .execution_model_pool
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        pool,
        serde_json::json!({
            "mode": "range",
            "models": [{"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model"}]
        }),
        "provider deletion prunes persisted collaboration candidates in the same transaction"
    );
    let failover: String = nomifun_db::sqlx::query_scalar(
        "SELECT value FROM client_preferences WHERE key = 'agent.model_failover'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&failover).unwrap()["queue"],
        serde_json::json!([{"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model"}])
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
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model_first"},
            {"provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000001", "model": "model_second"}
        ]),
        "provider deletion preserves candidate order while pruning the deleted and already-missing providers"
    );

    insert_provider(&database, "prov_0190f5fe-7c00-7a00-8000-000000000002").await;
    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) \
         VALUES ('idmm_backup_provider_id', 'prov_0190f5fe-7c00-7a00-8000-000000000002', 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    assert!(
        nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
            .execute(database.pool())
            .await
            .is_err(),
        "IDMM backup is a hard binding protected inside provider DELETE"
    );
    assert!(
        nomifun_db::sqlx::query(
            "UPDATE client_preferences SET value = 'prov_0190f5fe-7c00-7a00-8000-000000000003' \
             WHERE key = 'idmm_backup_provider_id'",
        )
        .execute(database.pool())
        .await
        .is_err(),
        "IDMM backup updates cannot introduce a missing provider"
    );
    nomifun_db::sqlx::query(
        "DELETE FROM client_preferences WHERE key = 'idmm_backup_provider_id'",
    )
    .execute(database.pool())
    .await
    .unwrap();
    let hard_conversation = conversations
        .create(&conversation(
            &owner,
            "hard lead",
            Some(serde_json::json!({
                "provider_id": "prov_0190f5fe-7c00-7a00-8000-000000000002",
                "model": "model"
            })),
            None,
        ))
        .await
        .unwrap();
    let conflict = providers.delete("prov_0190f5fe-7c00-7a00-8000-000000000002").await.unwrap_err();
    assert!(
        matches!(
            conflict,
            DbError::Conflict(ref message)
                if message == "provider is still referenced by an executable Agent binding"
        ),
        "the repository must preserve the DB's race-authority conflict as a 409-class error; got {conflict:?}"
    );
    nomifun_db::sqlx::query("UPDATE conversations SET model = NULL WHERE id = ?")
        .bind(&hard_conversation)
        .execute(database.pool())
        .await
        .unwrap();

    let template = templates
        .create_template(
            &owner,
            &CreateAgentExecutionTemplateParams {
                name: "hard template".to_owned(),
                description: None,
                max_parallel: Some(1),
                work_dir: None,
                context: None,
                participants: vec![template_participant("prov_0190f5fe-7c00-7a00-8000-000000000002")],
            },
        )
        .await
        .unwrap();
    assert!(
        nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
            .execute(database.pool())
            .await
            .is_err(),
        "the DB closes a Template usage-scan/delete race"
    );
    assert!(
        templates
            .delete_template(&owner, &template.template.id, template.template.version)
            .await
            .unwrap()
    );

    let execution = executions
        .create_execution_with_participants(
            &owner,
            &CreateAgentExecutionParams {
                goal: "hard execution".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: Some(hard_conversation.clone()),
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[execution_participant("prov_0190f5fe-7c00-7a00-8000-000000000002")],
            &event(AgentExecutionEventKind::Created),
        )
        .await
        .unwrap();
    assert!(
        nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
            .execute(database.pool())
            .await
            .is_err(),
        "the DB closes an Agent Execution usage-scan/delete race"
    );
    executions
        .update_execution(
            &owner,
            &execution.id,
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
    nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
        .execute(database.pool())
        .await
        .unwrap();

    let missing_execution = executions
        .create_execution_with_participants(
            &owner,
            &CreateAgentExecutionParams {
                goal: "missing provider".to_owned(),
                status: AgentExecutionStatus::Planning,
                plan_gate: PlanGate::Automatic,
                adaptation_policy: AdaptationPolicy::Fixed,
                decision_policy: DecisionPolicy::Automatic,
                delegation_policy: DelegationPolicy::Automatic,
                max_parallel: 1,
                work_dir: None,
                lead_conversation_id: None,
                initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
            },
            &[execution_participant("prov_0190f5fe-7c00-7a00-8000-000000000003")],
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
    insert_provider(&database, "prov_0190f5fe-7c00-7a00-8000-000000000002").await;
    nomifun_db::sqlx::query(
        "INSERT INTO client_preferences (key, value, updated_at) \
         VALUES ('nomi.collaborationModels', \
                 '[{\"provider_id\":\"prov_0190f5fe-7c00-7a00-8000-000000000002\",\"model\":\"model\"}]', 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();

    nomifun_db::sqlx::query("DELETE FROM providers WHERE id = 'prov_0190f5fe-7c00-7a00-8000-000000000002'")
        .execute(database.pool())
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
