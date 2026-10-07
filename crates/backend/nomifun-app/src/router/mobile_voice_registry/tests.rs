use super::*;
use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, UpsertProviderConnectionParams,
    init_database_memory,
};

const KEY: [u8; 32] = [0x63; 32];
async fn fixture() -> (Arc<AppVoiceRegistry>, sqlx::SqlitePool) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let providers = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let encrypted =
        nomifun_common::encrypt_string(r#"{"api_keys":["private-registry-fixture-key"]}"#, &KEY)
            .unwrap();
    for (id, model) in [
        (
            "0190f5fe-7c00-7a00-8000-000000000291",
            "stepaudio-3-realtime-preview",
        ),
        ("0190f5fe-7c00-7a00-8000-000000000292", "gpt-live-1"),
        ("0190f5fe-7c00-7a00-8000-000000000293", "unknown-live-model"),
    ] {
        let caps = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            provider_params: "{}",
            ..Default::default()
        }];
        let named = [UpsertProviderConnectionParams {
            role: "voice",
            label: Some("Existing named connection"),
            base_url: "https://voice.example/v1",
            auth_scheme: "bearer",
            credentials_encrypted: &encrypted,
            extra: "{}",
        }];
        providers
            .create(
                CreateProviderParams {
                    provider_id: Some(id),
                    platform: "custom",
                    name: "configured Main provider",
                    base_url: "https://api.example/v1",
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model,
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &caps,
                },
                &named,
            )
            .await
            .unwrap();
    }
    let invoke = Arc::new(ModelInvokeService::new(
        providers,
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
        KEY,
        reqwest::Client::new(),
        nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
    ));
    (
        AppVoiceRegistry::new(invoke, pool.clone(), Arc::from("owner")).unwrap(),
        pool,
    )
}
fn selection() -> VoiceProfileUpdate {
    VoiceProfileUpdate {
        work_steering_policy:Default::default(),
        expected_revision: 0,
        agent_session_id: "agent-session".into(),
        binding_version: 1,
        enabled: true,
        provider_id: "0190f5fe-7c00-7a00-8000-000000000292".into(),
        model: "gpt-live-1".into(),
        adapter_id: None,
        connection_role: "voice".into(),
        transport: VoiceTransportPreference::Relay,
        adapter_config: json!({}),
    }
}

#[tokio::test]
async fn catalog_and_materialization_are_read_only_and_unknown_is_not_promoted() {
    let (registry, pool) = fixture().await;
    let before:Vec<(String,String,String,String)>=sqlx::query_as("SELECT provider_id,model,task,protocol FROM provider_model_capabilities ORDER BY provider_id,model,task").fetch_all(&pool).await.unwrap();
    let revisions_before: Vec<(String, i64)> =
        sqlx::query_as("SELECT provider_id,config_revision FROM providers ORDER BY provider_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let catalog = registry.catalog().await.unwrap();
    let models = catalog["models"].as_array().unwrap();
    assert_eq!(models.len(), 3);
    let live = models
        .iter()
        .find(|model| model["model"] == "gpt-live-1")
        .unwrap();
    assert_eq!(
        live["connection_roles"],
        json!(["default", "voice"]),
        "the real Mobile profile consumer uses string role candidates and includes(role)"
    );
    assert_eq!(live["connection_role_details"][1]["role"], "voice");
    assert!(models.iter().all(|model| {
        model["label"].is_string()
            && model["connection_roles"]
                .as_array()
                .unwrap()
                .iter()
                .all(Value::is_string)
    }));
    assert_eq!(
        live["adapters"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|adapter| adapter["support"] == "supported")
            .count(),
        1
    );
    assert!(
        models
            .iter()
            .find(|model| model["model"] == "unknown-live-model")
            .unwrap()["adapters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|adapter| adapter["support"] == "unknown")
    );
    let record = registry.resolve_record(&selection(), 1).await.unwrap();
    assert_eq!(record.adapter_id, voice::OPENAI_LIVE_ADAPTER_ID);
    assert_eq!(
        record.connection_config_ref,
        format!("provider:{}:voice", record.provider_id)
    );
    let serialized = serde_json::to_string(&record).unwrap();
    assert!(
        !serialized.contains("private-registry-fixture-key") && !serialized.contains("api_keys")
    );
    assert!(!catalog.to_string().contains("private-registry-fixture-key"));
    let mut unknown = selection();
    unknown.provider_id = "0190f5fe-7c00-7a00-8000-000000000293".into();
    unknown.model = "unknown-live-model".into();
    assert!(registry.resolve_record(&unknown, 1).await.is_err());
    let mut extra = selection();
    extra.adapter_config = json!({"transport":"native_webrtc"});
    assert!(registry.resolve_record(&extra, 1).await.is_err());
    let after:Vec<(String,String,String,String)>=sqlx::query_as("SELECT provider_id,model,task,protocol FROM provider_model_capabilities ORDER BY provider_id,model,task").fetch_all(&pool).await.unwrap();
    assert_eq!(before, after);
    let revisions_after: Vec<(String, i64)> =
        sqlx::query_as("SELECT provider_id,config_revision FROM providers ORDER BY provider_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(revisions_before, revisions_after);
    assert!(
        after.iter().all(|(_, _, task, _)| task == "chat"),
        "VoiceProfile never creates or rewrites Main capability tasks"
    );
}

#[tokio::test]
async fn selected_connection_secret_rotates_without_rewriting_route_but_config_and_enable_fences_fail()
 {
    let (registry, pool) = fixture().await;
    let record = registry.resolve_record(&selection(), 1).await.unwrap();
    let identity = record.identity().unwrap();
    let old = registry.lease_revision(&record).await.unwrap();
    let rotated =
        nomifun_common::encrypt_string(r#"{"api_keys":["rotated-private-fixture-key"]}"#, &KEY)
            .unwrap();
    sqlx::query("UPDATE provider_connections SET credentials_encrypted=? WHERE provider_id=? AND role='voice'").bind(rotated).bind(&record.provider_id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE providers SET config_revision=config_revision+1 WHERE provider_id=?")
        .bind(&record.provider_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_ne!(old, registry.lease_revision(&record).await.unwrap());
    assert_eq!(identity, record.identity().unwrap());
    registry.registry.create(&record).await.unwrap(); // constructor only, never a socket/model call
    sqlx::query("UPDATE provider_connections SET base_url='https://changed.example/v1' WHERE provider_id=? AND role='voice'").bind(&record.provider_id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE providers SET config_revision=config_revision+1 WHERE provider_id=?")
        .bind(&record.provider_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        registry.lease_revision(&record).await.unwrap_err().kind,
        VoiceErrorKind::StaleBinding
    );
    assert!(registry.registry.create(&record).await.is_err());
    sqlx::query("UPDATE provider_models SET enabled=0 WHERE provider_id=? AND model=?")
        .bind(&record.provider_id)
        .bind(&record.model)
        .execute(&pool)
        .await
        .unwrap();
    assert!(registry.resolve_record(&selection(), 2).await.is_err());
}

#[tokio::test]
async fn registry_constructor_does_not_need_open_database() {
    let (registry, pool) = fixture().await;
    pool.close().await;
    assert!(AppVoiceRegistry::new(registry.invoke.clone(), pool, Arc::from("owner")).is_ok());
}
