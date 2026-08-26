use nomifun_db::models::ConversationRow;
use nomifun_db::{
    CoordinatedProviderModelDelete, CreateProviderParams, CreateTerminalParams,
    CreativeStudioTemplateRow, DbError, IConversationRepository,
    IProviderConnectionRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository, ITerminalRepository, NewProviderModel,
    NewProviderModelCapability, ProviderModelCleanupPlan, ProviderModelProjectCleanup,
    ProviderModelTemplateCleanup, SqliteConversationRepository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, SqliteTerminalRepository,
    UpdateProviderParams, UpsertProviderConnectionParams, init_database_memory,
};

const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
const CALLER_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000010";

static CHAT_CAPABILITIES: [NewProviderModelCapability<'static>; 1] = [NewProviderModelCapability {
    task: "chat",
    traits: "[\"vision_input\"]",
    protocol: "openai.chat_text",
    connection_role: "default",
    base_url_override: None,
    endpoint: Some("/chat/completions"),
    poll_endpoint: None,
    content_endpoint: None,
    realtime_endpoint: None,
    allow_cross_origin_credentials: false,
    provider_params: "{}",
    context_limit: Some(128_000),
    output_limit: None,
}];

static IMAGE_CAPABILITIES: [NewProviderModelCapability<'static>; 1] =
    [NewProviderModelCapability {
        task: "image_generation",
        traits: "[]",
        protocol: "ark.images",
        connection_role: "default",
        base_url_override: None,
        endpoint: None,
        poll_endpoint: None,
        content_endpoint: None,
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{\"seed\":7}",
        context_limit: None,
        output_limit: None,
    }];

static VIDEO_CAPABILITIES: [NewProviderModelCapability<'static>; 1] =
    [NewProviderModelCapability {
        task: "video_generation",
        traits: "[]",
        protocol: "openai.videos",
        connection_role: "default",
        base_url_override: None,
        endpoint: Some("/videos"),
        poll_endpoint: Some("/videos/{id}"),
        content_endpoint: Some("/videos/{id}/content"),
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{}",
        context_limit: None,
        output_limit: None,
    }];

static VOICE_CAPABILITIES: [NewProviderModelCapability<'static>; 1] =
    [NewProviderModelCapability {
        task: "speech_synthesis",
        traits: "[]",
        protocol: "volc.tts_v3",
        connection_role: "voice",
        base_url_override: None,
        endpoint: Some("/api/v3/tts/unidirectional"),
        poll_endpoint: None,
        content_endpoint: None,
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{}",
        context_limit: None,
        output_limit: None,
    }];

fn provider_params(provider_id: Option<&str>) -> CreateProviderParams<'_> {
    CreateProviderParams {
        provider_id,
        platform: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        auth_scheme: "bearer",
        credentials_encrypted: "cipher",
        enabled: true,
        bedrock_config: None,
        sort_order: None,
    }
}

fn voice_connection<'a>(
    label: Option<&'a str>,
    base_url: &'a str,
) -> UpsertProviderConnectionParams<'a> {
    UpsertProviderConnectionParams {
        role: "voice",
        label,
        base_url,
        auth_scheme: "bearer",
        credentials_encrypted: "voice-cipher",
        extra: "{}",
    }
}

fn model<'a>(
    name: &'a str,
    capabilities: &'a [NewProviderModelCapability<'a>],
) -> NewProviderModel<'a> {
    NewProviderModel {
        model: name,
        enabled: true,
        sort_order: 0,
        description: Some("configured model"),
        capabilities,
    }
}

async fn insert_provider(
    repository: &SqliteProviderRepository,
    provider_id: &'static str,
    name: &'static str,
) {
    let mut params = provider_params(Some(provider_id));
    params.name = name;
    repository
        .create(params, &model("chat", &CHAT_CAPABILITIES), &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn aggregate_create_persists_provider_model_capability_and_named_connection() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let connection = voice_connection(Some("Voice API"), "https://voice.example.com/v1");
    let (provider, stored_model) = repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("voice-model", &VOICE_CAPABILITIES),
            &[connection],
        )
        .await
        .unwrap();

    assert_eq!(provider.provider_id, PROVIDER_ID);
    assert_eq!(provider.auth_scheme, "bearer");
    assert_eq!(stored_model.model, "voice-model");
    let capability = SqliteProviderModelCapabilityRepository::new(db.pool().clone())
        .get(PROVIDER_ID, "voice-model", "speech_synthesis")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capability.protocol, "volc.tts_v3");
    assert_eq!(capability.connection_role, "voice");
    let connection_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_connections WHERE provider_id = ? AND role = 'voice'",
    )
    .bind(PROVIDER_ID)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(connection_count, 1);
}

#[tokio::test]
async fn model_display_name_is_separate_from_the_runtime_model_id() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("ep-test-endpoint", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();

    let models = SqliteProviderModelRepository::new(db.pool().clone());
    models
        .set_display_name(PROVIDER_ID, "ep-test-endpoint", Some("Seedance 1.5 Pro"))
        .await
        .unwrap();

    let row = models
        .get(PROVIDER_ID, "ep-test-endpoint")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.model, "ep-test-endpoint");
    assert_eq!(row.display_name.as_deref(), Some("Seedance 1.5 Pro"));
}

#[tokio::test]
async fn aggregate_create_rejects_a_capability_with_an_unconfigured_named_role() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let error = repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("voice-model", &VOICE_CAPABILITIES),
            &[],
        )
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(_)));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "the invalid aggregate must roll back its provider"
    );
}

#[tokio::test]
async fn aggregate_create_rolls_back_everything_on_invalid_child() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let invalid = NewProviderModel {
        capabilities: &[],
        ..model("broken", &CHAT_CAPABILITIES)
    };
    assert!(
        repository
            .create(provider_params(Some(PROVIDER_ID)), &invalid, &[])
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn model_save_preserves_health_only_when_invocation_config_is_unchanged() {
    let db = init_database_memory().await.unwrap();
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("multi", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE provider_model_capabilities SET updated_at = 123 \
         WHERE provider_id = ? AND model = 'multi' AND task = 'chat'",
    )
    .bind(PROVIDER_ID)
    .execute(db.pool())
    .await
    .unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    capabilities
        .set_health(
            PROVIDER_ID,
            0,
            "multi",
            "chat",
            Some(r#"{"status":"healthy"}"#),
        )
        .await
        .unwrap();

    models
        .save(PROVIDER_ID, 0, &model("multi", &CHAT_CAPABILITIES))
        .await
        .unwrap();
    assert!(
        capabilities
            .get(PROVIDER_ID, "multi", "chat")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some(),
        "an identical full save must preserve the observation"
    );
    let unchanged = capabilities
        .get(PROVIDER_ID, "multi", "chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.updated_at, 123);
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        0
    );

    static TWO: [NewProviderModelCapability<'static>; 2] = [
        NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            base_url_override: None,
            endpoint: Some("/v2/chat"),
            poll_endpoint: None,
            content_endpoint: None,
            realtime_endpoint: None,
            allow_cross_origin_credentials: false,
            provider_params: "{}",
            context_limit: None,
            output_limit: None,
        },
        NewProviderModelCapability {
            task: "image_generation",
            ..IMAGE_CAPABILITIES[0]
        },
    ];
    models
        .save(PROVIDER_ID, 0, &model("multi", &TWO))
        .await
        .unwrap();
    let changed = capabilities
        .get(PROVIDER_ID, "multi", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(changed.health.is_none());
    assert!(changed.health_checked_at.is_none());
    assert_ne!(changed.updated_at, 123);
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        1
    );
    assert!(
        capabilities
            .get(PROVIDER_ID, "multi", "image_generation")
            .await
            .unwrap()
            .is_some()
    );

    models
        .save(PROVIDER_ID, 1, &model("multi", &IMAGE_CAPABILITIES))
        .await
        .unwrap();
    assert!(
        capabilities
            .get(PROVIDER_ID, "multi", "chat")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        capabilities
            .get(PROVIDER_ID, "multi", "image_generation")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        2
    );

    let disabled = NewProviderModel {
        enabled: false,
        ..model("multi", &IMAGE_CAPABILITIES)
    };
    models.save(PROVIDER_ID, 2, &disabled).await.unwrap();
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        3
    );
    assert!(
        models
            .delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: PROVIDER_ID.to_owned(),
                model: "multi".to_owned(),
                expected_config_revision: 3,
                cleanup: ProviderModelCleanupPlan::default(),
            })
            .await
            .unwrap()
    );
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        4
    );
}

#[tokio::test]
async fn concurrent_model_saves_use_provider_config_revision_as_a_cas_guard() {
    let db = init_database_memory().await.unwrap();
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("chat", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    let first = SqliteProviderModelRepository::new(db.pool().clone());
    let second = first.clone();
    let video = model("video", &VIDEO_CAPABILITIES);
    let image = model("image", &IMAGE_CAPABILITIES);

    let (first_result, second_result) = tokio::join!(
        first.save(PROVIDER_ID, 0, &video),
        second.save(PROVIDER_ID, 0, &image)
    );
    assert_eq!(
        usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
        1
    );
    let conflict = first_result.err().or_else(|| second_result.err()).unwrap();
    assert!(matches!(conflict, DbError::Conflict(_)));
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        1
    );
}

#[tokio::test]
async fn stale_provider_and_connection_mutations_fail_after_a_model_revision_wins() {
    let db = init_database_memory().await.unwrap();
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("chat", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    SqliteProviderModelRepository::new(db.pool().clone())
        .save(PROVIDER_ID, 0, &model("video", &VIDEO_CAPABILITIES))
        .await
        .unwrap();

    let provider_error = providers
        .update(
            PROVIDER_ID,
            0,
            UpdateProviderParams {
                name: Some("stale rename"),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(provider_error, DbError::Conflict(_)));
    let connection_error = SqliteProviderConnectionRepository::new(db.pool().clone())
        .upsert(
            PROVIDER_ID,
            0,
            &voice_connection(Some("stale voice"), "https://voice.example.com/v1"),
        )
        .await
        .unwrap_err();
    assert!(matches!(connection_error, DbError::Conflict(_)));
    let provider = providers.find_by_id(PROVIDER_ID).await.unwrap().unwrap();
    assert_eq!(provider.name, "OpenAI");
    assert_eq!(provider.config_revision, 1);
}

#[tokio::test]
async fn stale_health_probe_cannot_overwrite_a_newer_invocation_graph() {
    let db = init_database_memory().await.unwrap();
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("chat", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    let changed_capabilities = [NewProviderModelCapability {
        endpoint: Some("/v2/chat/completions"),
        ..CHAT_CAPABILITIES[0]
    }];
    SqliteProviderModelRepository::new(db.pool().clone())
        .save(PROVIDER_ID, 0, &model("chat", &changed_capabilities))
        .await
        .unwrap();

    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    let before = capabilities
        .get(PROVIDER_ID, "chat", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !capabilities
            .set_health(
                PROVIDER_ID,
                0,
                "chat",
                "chat",
                Some(r#"{"status":"healthy"}"#),
            )
            .await
            .unwrap()
    );
    let after_stale = capabilities
        .get(PROVIDER_ID, "chat", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(after_stale.health.is_none());
    assert!(after_stale.health_checked_at.is_none());
    assert_eq!(after_stale.updated_at, before.updated_at);

    assert!(
        capabilities
            .set_health(
                PROVIDER_ID,
                1,
                "chat",
                "chat",
                Some(r#"{"status":"healthy"}"#),
            )
            .await
            .unwrap()
    );
    let after_fresh = capabilities
        .get(PROVIDER_ID, "chat", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(after_fresh.health.is_some());
    assert!(after_fresh.health_checked_at.is_some());
    assert_eq!(after_fresh.updated_at, before.updated_at);
}

#[tokio::test]
async fn model_save_persists_distinct_async_route_endpoints() {
    let db = init_database_memory().await.unwrap();
    SqliteProviderRepository::new(db.pool().clone())
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("chat", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    SqliteProviderModelRepository::new(db.pool().clone())
        .save(PROVIDER_ID, 0, &model("video", &VIDEO_CAPABILITIES))
        .await
        .unwrap();

    let stored = SqliteProviderModelCapabilityRepository::new(db.pool().clone())
        .get(PROVIDER_ID, "video", "video_generation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.endpoint.as_deref(), Some("/videos"));
    assert_eq!(stored.poll_endpoint.as_deref(), Some("/videos/{id}"));
    assert_eq!(
        stored.content_endpoint.as_deref(),
        Some("/videos/{id}/content")
    );
}

#[tokio::test]
async fn clone_graph_copies_configuration_but_not_health() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("gpt-5", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    SqliteProviderModelCapabilityRepository::new(db.pool().clone())
        .set_health(
            PROVIDER_ID,
            0,
            "gpt-5",
            "chat",
            Some(r#"{"status":"healthy"}"#),
        )
        .await
        .unwrap();
    let cloned = repository
        .clone_graph(PROVIDER_ID, "OpenAI Copy")
        .await
        .unwrap();
    assert_ne!(cloned.provider_id, PROVIDER_ID);
    let copied = SqliteProviderModelCapabilityRepository::new(db.pool().clone())
        .get(&cloned.provider_id, "gpt-5", "chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(copied.protocol, "openai.chat_text");
    assert!(copied.health.is_none());
    assert!(copied.health_checked_at.is_none());
}

#[tokio::test]
async fn managed_graph_replaces_membership_and_keeps_matching_capability_health() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    repository
        .save_managed_graph(
            provider_params(Some(PROVIDER_ID)),
            &[
                model("old", &CHAT_CAPABILITIES),
                model("kept", &CHAT_CAPABILITIES),
            ],
        )
        .await
        .unwrap();
    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    capabilities
        .set_health(
            PROVIDER_ID,
            0,
            "kept",
            "chat",
            Some(r#"{"status":"healthy"}"#),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE provider_model_capabilities SET updated_at = 321 \
         WHERE provider_id = ? AND model = 'kept' AND task = 'chat'",
    )
    .bind(PROVIDER_ID)
    .execute(db.pool())
    .await
    .unwrap();

    repository
        .save_managed_graph(
            provider_params(Some(PROVIDER_ID)),
            &[
                model("kept", &CHAT_CAPABILITIES),
                model("new", &CHAT_CAPABILITIES),
            ],
        )
        .await
        .unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone())
        .list_for_provider(PROVIDER_ID)
        .await
        .unwrap();
    assert_eq!(
        models
            .iter()
            .map(|row| row.model.as_str())
            .collect::<Vec<_>>(),
        ["kept", "new"]
    );
    assert!(
        capabilities
            .get(PROVIDER_ID, "kept", "chat")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some()
    );
    assert!(
        capabilities
            .get(PROVIDER_ID, "old", "chat")
            .await
            .unwrap()
            .is_none()
    );

    let mut changed_provider = provider_params(Some(PROVIDER_ID));
    changed_provider.base_url = "https://managed.example.test/v2";
    repository
        .save_managed_graph(
            changed_provider,
            &[
                model("kept", &CHAT_CAPABILITIES),
                model("new", &CHAT_CAPABILITIES),
            ],
        )
        .await
        .unwrap();
    let invalidated = capabilities
        .get(PROVIDER_ID, "kept", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(invalidated.health.is_none());
    assert!(invalidated.health_checked_at.is_none());
    assert_eq!(invalidated.updated_at, 321);
    assert_eq!(
        repository
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        2
    );
}

#[tokio::test]
async fn provider_delete_explicitly_removes_owned_graph() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("gpt-5", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    repository.delete(PROVIDER_ID).await.unwrap();
    for table in [
        "providers",
        "provider_models",
        "provider_model_capabilities",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE provider_id = ?");
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(PROVIDER_ID)
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[tokio::test]
async fn provider_auth_scheme_is_explicit_and_nonblank() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    repository
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("gpt-5", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    let updated = repository
        .update(
            PROVIDER_ID,
            0,
            UpdateProviderParams {
                auth_scheme: Some("token"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.auth_scheme, "token");
    let error = repository
        .update(
            PROVIDER_ID,
            1,
            UpdateProviderParams {
                auth_scheme: Some(" "),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(_)));
}

#[tokio::test]
async fn default_connection_changes_invalidate_only_effective_health() {
    let db = init_database_memory().await.unwrap();
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("gpt-5", &CHAT_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    capabilities
        .set_health(
            PROVIDER_ID,
            0,
            "gpt-5",
            "chat",
            Some(r#"{"status":"healthy"}"#),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE provider_model_capabilities SET updated_at = 322 \
         WHERE provider_id = ? AND model = 'gpt-5' AND task = 'chat'",
    )
    .bind(PROVIDER_ID)
    .execute(db.pool())
    .await
    .unwrap();

    providers
        .update(
            PROVIDER_ID,
            0,
            UpdateProviderParams {
                name: Some("Renamed only"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        capabilities
            .get(PROVIDER_ID, "gpt-5", "chat")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some()
    );

    providers
        .update(
            PROVIDER_ID,
            0,
            UpdateProviderParams {
                base_url: Some("https://api.openai.com/v2"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let invalidated = capabilities
        .get(PROVIDER_ID, "gpt-5", "chat")
        .await
        .unwrap()
        .unwrap();
    assert!(invalidated.health.is_none());
    assert!(invalidated.health_checked_at.is_none());
    assert_eq!(invalidated.updated_at, 322);
    assert_eq!(
        providers
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        1
    );
}

#[tokio::test]
async fn named_connection_changes_invalidate_only_its_capability_health() {
    let db = init_database_memory().await.unwrap();
    SqliteProviderRepository::new(db.pool().clone())
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("voice-model", &VOICE_CAPABILITIES),
            &[voice_connection(
                Some("Voice API"),
                "https://voice.example.com/v1",
            )],
        )
        .await
        .unwrap();
    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    capabilities
        .set_health(
            PROVIDER_ID,
            0,
            "voice-model",
            "speech_synthesis",
            Some(r#"{"status":"healthy"}"#),
        )
        .await
        .unwrap();
    sqlx::query(
        "UPDATE provider_model_capabilities SET updated_at = 323 \
         WHERE provider_id = ? AND model = 'voice-model' AND task = 'speech_synthesis'",
    )
    .bind(PROVIDER_ID)
    .execute(db.pool())
    .await
    .unwrap();
    let connections = SqliteProviderConnectionRepository::new(db.pool().clone());

    connections
        .upsert(
            PROVIDER_ID,
            0,
            &voice_connection(Some("Label-only change"), "https://voice.example.com/v1"),
        )
        .await
        .unwrap();
    assert!(
        capabilities
            .get(PROVIDER_ID, "voice-model", "speech_synthesis")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some()
    );

    connections
        .upsert(
            PROVIDER_ID,
            0,
            &voice_connection(Some("Label-only change"), "https://voice.example.com/v2"),
        )
        .await
        .unwrap();
    let invalidated = capabilities
        .get(PROVIDER_ID, "voice-model", "speech_synthesis")
        .await
        .unwrap()
        .unwrap();
    assert!(invalidated.health.is_none());
    assert!(invalidated.health_checked_at.is_none());
    assert_eq!(invalidated.updated_at, 323);
    assert_eq!(
        SqliteProviderRepository::new(db.pool().clone())
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        1
    );
}

#[tokio::test]
async fn bedrock_provider_allows_the_manifest_defined_empty_base_url() {
    static BEDROCK: [NewProviderModelCapability<'static>; 1] = [NewProviderModelCapability {
        task: "chat",
        traits: "[]",
        protocol: "bedrock.anthropic_messages",
        connection_role: "default",
        base_url_override: None,
        endpoint: None,
        poll_endpoint: None,
        content_endpoint: None,
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{}",
        context_limit: None,
        output_limit: Some(8192),
    }];
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let (provider, _) = repository
        .create(
            CreateProviderParams {
                provider_id: Some(PROVIDER_ID),
                platform: "bedrock",
                name: "Amazon Bedrock",
                base_url: "",
                auth_scheme: "bedrock",
                credentials_encrypted: "cipher",
                enabled: true,
                bedrock_config: Some("{}"),
                sort_order: None,
            },
            &model("anthropic.claude", &BEDROCK),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(provider.base_url, "");
    assert_eq!(provider.auth_scheme, "bedrock");
    let capability = SqliteProviderModelCapabilityRepository::new(db.pool().clone())
        .get(PROVIDER_ID, "anthropic.claude", "chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capability.output_limit, Some(8192));
}

#[tokio::test]
async fn delete_clears_all_idmm_session_bypass_references_but_preserves_watch_config() {
    const DELETED_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000020";
    const RETAINED_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000021";

    let db = init_database_memory().await.unwrap();
    let provider_repo = SqliteProviderRepository::new(db.pool().clone());
    insert_provider(&provider_repo, DELETED_PROVIDER, "deleted").await;
    insert_provider(&provider_repo, RETAINED_PROVIDER, "retained").await;

    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let conversation_repo = SqliteConversationRepository::new(db.pool().clone());
    let conversation_id = nomifun_common::ConversationId::new().into_string();
    conversation_repo
        .create(&ConversationRow {
            id: 0,
            conversation_id: conversation_id.clone(),
            user_id: owner.clone(),
            name: "IDMM cleanup".to_owned(),
            r#type: "nomi".to_owned(),
            extra: r#"{"workspace":"/tmp/idmm"}"#.to_owned(),
            delegation_policy: "automatic".to_owned(),
            execution_model_pool: None,
            decision_policy: "automatic".to_owned(),
            execution_template_id: None,
            model: None,
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
        })
        .await
        .unwrap();
    let conversation_idmm = serde_json::json!({
        "fault_watch": {
            "enabled": true,
            "scan_interval_secs": 23,
            "bypass_model": {
                "provider_id": DELETED_PROVIDER,
                "model": "fault-deleted"
            }
        },
        "decision_watch": {
            "enabled": true,
            "scan_interval_secs": 41,
            "bypass_model": {
                "provider_id": RETAINED_PROVIDER,
                "model": "decision-retained"
            }
        }
    })
    .to_string();
    conversation_repo
        .update_idmm(&conversation_id, Some(&conversation_idmm))
        .await
        .unwrap();

    let terminal_repo = SqliteTerminalRepository::new(db.pool().clone());
    let terminal = terminal_repo
        .create(&CreateTerminalParams {
            id: nomifun_common::TerminalId::new(),
            name: "IDMM cleanup".to_owned(),
            cwd: "/tmp".to_owned(),
            command: "$SHELL".to_owned(),
            args: "[]".to_owned(),
            env: None,
            backend: None,
            mode: None,
            cols: 80,
            rows: 24,
            user_id: nomifun_common::UserId::parse(owner).unwrap(),
        })
        .await
        .unwrap();
    let terminal_idmm = serde_json::json!({
        "fault_watch": {
            "enabled": true,
            "max_retries": 8,
            "bypass_model": {
                "provider_id": RETAINED_PROVIDER,
                "model": "fault-retained"
            }
        },
        "decision_watch": {
            "enabled": true,
            "max_retries": 5,
            "bypass_model": {
                "provider_id": DELETED_PROVIDER,
                "model": "decision-deleted"
            }
        }
    })
    .to_string();
    terminal_repo
        .update_idmm(terminal.terminal_id.as_str(), Some(&terminal_idmm))
        .await
        .unwrap();

    provider_repo.delete(DELETED_PROVIDER).await.unwrap();

    let conversation = conversation_repo
        .get(&conversation_id)
        .await
        .unwrap()
        .unwrap();
    let extra: serde_json::Value = serde_json::from_str(&conversation.extra).unwrap();
    assert!(extra["idmm"]["fault_watch"].get("bypass_model").is_none());
    assert_eq!(extra["idmm"]["fault_watch"]["enabled"], true);
    assert_eq!(extra["idmm"]["fault_watch"]["scan_interval_secs"], 23);
    assert_eq!(
        extra["idmm"]["decision_watch"]["bypass_model"]["provider_id"],
        RETAINED_PROVIDER
    );
    assert_eq!(extra["workspace"], "/tmp/idmm");

    let terminal_idmm: serde_json::Value = serde_json::from_str(
        &terminal_repo
            .get_idmm(terminal.terminal_id.as_str())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        terminal_idmm["fault_watch"]["bypass_model"]["provider_id"],
        RETAINED_PROVIDER
    );
    assert!(
        terminal_idmm["decision_watch"]
            .get("bypass_model")
            .is_none()
    );
    assert_eq!(terminal_idmm["decision_watch"]["enabled"], true);
    assert_eq!(terminal_idmm["decision_watch"]["max_retries"], 5);
}

#[tokio::test]
async fn delete_fails_closed_on_malformed_cron_provider_json() {
    let db = init_database_memory().await.unwrap();
    let repository = SqliteProviderRepository::new(db.pool().clone());
    insert_provider(&repository, CALLER_PROVIDER_ID, "malformed guard").await;
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO cron_jobs (\
            cron_job_id, user_id, name, schedule_kind, schedule_value, payload_message, \
            agent_type, agent_config, created_by, created_at, updated_at\
         ) VALUES (?, ?, 'malformed provider binding', 'every', '60000', '', \
                   'nomi', '{', 'user', 1, 1)",
    )
    .bind(nomifun_common::CronJobId::new().as_str())
    .bind(owner)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(db.pool())
        .await
        .unwrap();

    let error = repository.delete(CALLER_PROVIDER_ID).await.unwrap_err();
    assert!(matches!(error, DbError::Conflict(_)));
    assert!(
        repository
            .find_by_id(CALLER_PROVIDER_ID)
            .await
            .unwrap()
            .is_some()
    );
    let cron_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_jobs WHERE agent_config = '{'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(
        cron_count, 1,
        "failed deletion must not mutate the cron row"
    );
}

async fn seed_model_delete_provider(
    db: &nomifun_db::Database,
    with_surviving_model: bool,
) -> i64 {
    let providers = SqliteProviderRepository::new(db.pool().clone());
    providers
        .create(
            provider_params(Some(PROVIDER_ID)),
            &model("delete-me", &IMAGE_CAPABILITIES),
            &[],
        )
        .await
        .unwrap();
    if with_surviving_model {
        SqliteProviderModelRepository::new(db.pool().clone())
            .save(PROVIDER_ID, 0, &model("keep-me", &CHAT_CAPABILITIES))
            .await
            .unwrap();
        1
    } else {
        0
    }
}

async fn model_delete_provider_revision(db: &nomifun_db::Database) -> i64 {
    sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
        .bind(PROVIDER_ID)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn add_chat_capability_to_model(db: &nomifun_db::Database, model: &str) {
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = ? AND task = 'chat'",
    )
    .bind(PROVIDER_ID)
    .bind(model)
    .fetch_one(db.pool())
    .await
    .unwrap();
    if exists == 0 {
        sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, endpoint, \
                 allow_cross_origin_credentials, provider_params, context_limit, created_at, updated_at) \
             VALUES (?, ?, 'chat', '[\"vision_input\"]', 'openai.chat_text', 'default', \
                     '/chat/completions', 0, '{}', 128000, 1, 1)",
        )
        .bind(PROVIDER_ID)
        .bind(model)
        .execute(db.pool())
        .await
        .unwrap();
    }
}

fn conversation_with_model(
    user_id: &str,
    model: Option<serde_json::Value>,
    execution_model_pool: Option<serde_json::Value>,
    status: &str,
) -> ConversationRow {
    let now = nomifun_common::now_ms();
    ConversationRow {
        id: 0,
        conversation_id: nomifun_common::ConversationId::new().into_string(),
        user_id: user_id.to_owned(),
        name: "model deletion fixture".to_owned(),
        r#type: "nomi".to_owned(),
        extra: "{}".to_owned(),
        delegation_policy: "automatic".to_owned(),
        execution_model_pool: execution_model_pool.map(|value| value.to_string()),
        decision_policy: "automatic".to_owned(),
        execution_template_id: None,
        model: model.map(|value| value.to_string()),
        status: Some(if status == "running" {
            "finished".to_owned()
        } else {
            status.to_owned()
        }),
        source: Some("nomifun".to_owned()),
        channel_chat_id: None,
        pinned: false,
        pinned_at: None,
        cron_job_id: None,
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        created_at: now,
        updated_at: now,
    }
}

fn model_json(model: &str) -> serde_json::Value {
    serde_json::json!({
        "provider_id": PROVIDER_ID,
        "model": model,
        "use_model": model,
    })
}

fn model_pool(models: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "mode": "range",
        "models": models.iter().map(|model| serde_json::json!({
            "provider_id": PROVIDER_ID,
            "model": model,
        })).collect::<Vec<_>>(),
    })
}

async fn seed_model_cleanup_project(
    db: &nomifun_db::Database,
    marker: &str,
) -> (String, String) {
    let project_id = nomifun_common::CreativeStudioProjectId::new().into_string();
    let document_json = serde_json::json!({
        "schema": "nomifun.creative-studio/v1",
        "projectId": project_id,
        "nodes": [{ "marker": marker }],
        "connections": []
    })
    .to_string();
    sqlx::query(
        "INSERT INTO creative_studio_projects \
            (project_id, title, revision, node_count, connection_count, document_json, \
             created_at, updated_at) \
         VALUES (?, 'Model cleanup', 1, 1, 0, ?, 100, 100)",
    )
    .bind(&project_id)
    .bind(&document_json)
    .execute(db.pool())
    .await
    .unwrap();
    (project_id, document_json)
}

fn model_cleanup_project_patch(
    project_id: &str,
    expected_revision: i64,
    marker: &str,
) -> ProviderModelProjectCleanup {
    ProviderModelProjectCleanup {
        project_id: project_id.to_owned(),
        expected_revision,
        document_json: serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": project_id,
            "nodes": [{ "marker": marker }],
            "connections": []
        })
        .to_string(),
        node_count: 1,
        connection_count: 0,
        updated_at: 200,
    }
}

async fn seed_model_cleanup_template(
    db: &nomifun_db::Database,
    marker: &str,
) -> CreativeStudioTemplateRow {
    let template_id = nomifun_common::CreativeStudioTemplateId::new().into_string();
    let definition_json = serde_json::json!({
        "id": template_id,
        "revision": 1,
        "marker": marker
    })
    .to_string();
    sqlx::query_as::<_, CreativeStudioTemplateRow>(
        "INSERT INTO creative_studio_templates \
            (template_id, revision, name, description, category, visibility, definition_json, \
             created_at, updated_at) \
         VALUES (?, 1, 'Model cleanup', '', '', 'private', ?, 100, 100) RETURNING *",
    )
    .bind(&template_id)
    .bind(&definition_json)
    .fetch_one(db.pool())
    .await
    .unwrap()
}

fn model_cleanup_template_patch(
    row: &CreativeStudioTemplateRow,
    expected_revision: i64,
    marker: &str,
) -> ProviderModelTemplateCleanup {
    let mut replacement = row.clone();
    replacement.revision = expected_revision + 1;
    replacement.updated_at = 200;
    replacement.definition_json = serde_json::json!({
        "id": row.template_id,
        "revision": replacement.revision,
        "marker": marker
    })
    .to_string();
    ProviderModelTemplateCleanup {
        template_id: row.template_id.clone(),
        expected_revision,
        replacement,
    }
}

async fn seed_live_model_creation_task(
    db: &nomifun_db::Database,
    project_id: &str,
) {
    let task_id = nomifun_common::CreationTaskId::new().into_string();
    let node_id = nomifun_common::CreativeStudioNodeId::new().into_string();
    sqlx::query(
        "INSERT INTO creation_tasks \
            (creation_task_id, project_id, node_id, provider_id, model, capability, params, \
             input_bindings, status, error, result_asset_ids, remote_task_id, attempt, \
             submitted_at, started_at, finished_at, request_fingerprint) \
         VALUES (?, ?, ?, ?, 'delete-me', 't2i', '{}', '[]', 'queued', NULL, '[]', \
                 NULL, 0, 100, NULL, NULL, '{\"fixture\":true}')",
    )
    .bind(task_id)
    .bind(project_id)
    .bind(node_id)
    .bind(PROVIDER_ID)
    .execute(db.pool())
    .await
    .unwrap();
}

async fn seed_model_template_run(
    db: &nomifun_db::Database,
    template: &CreativeStudioTemplateRow,
    step_kind: &str,
    status: &str,
) {
    let template_run_id = nomifun_common::CreativeStudioTemplateRunId::new().into_string();
    let template_step_id = nomifun_common::CreativeStudioTemplateStepId::new().into_string();
    let step = match step_kind {
        "generate-images" => serde_json::json!({
            "kind": "generate-images",
            "generation": {
                "model": { "providerId": PROVIDER_ID, "model": "delete-me" }
            }
        }),
        "draft-prompts" => serde_json::json!({
            "kind": "draft-prompts",
            "planning": {
                "model": { "providerId": PROVIDER_ID, "model": "delete-me" }
            }
        }),
        other => panic!("unsupported template fixture step {other}"),
    };
    let aggregate_json = serde_json::json!({
        "kind": "nomifun.creative-studio.template-run",
        "version": 1,
        "revision": 1,
        "templateSnapshot": {
            "id": template.template_id,
            "revision": template.revision,
            "steps": [step]
        },
        "request": {
            "id": template_run_id,
            "templateId": template.template_id,
            "templateRevision": template.revision
        },
        "record": {
            "requestId": template_run_id,
            "templateId": template.template_id,
            "status": status
        }
    })
    .to_string();
    sqlx::query(
        "INSERT INTO creative_studio_template_runs \
            (template_run_id, template_id, template_revision, revision, status, step_ids_json, \
             aggregate_json, created_at, updated_at) \
         VALUES (?, ?, ?, 1, ?, ?, ?, 100, 100)",
    )
    .bind(&template_run_id)
    .bind(&template.template_id)
    .bind(template.revision)
    .bind(status)
    .bind(serde_json::to_string(&[template_step_id]).unwrap())
    .bind(aggregate_json)
    .execute(db.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn coordinated_model_delete_applies_all_cleanup_and_preserves_sibling_model() {
    let db = init_database_memory().await.unwrap();
    let expected_config_revision = seed_model_delete_provider(&db, true).await;
    let (project_id, _) = seed_model_cleanup_project(&db, "delete-me").await;
    let template = seed_model_cleanup_template(&db, "delete-me").await;
    seed_model_template_run(&db, &template, "generate-images", "succeeded").await;
    let models = SqliteProviderModelRepository::new(db.pool().clone());

    let deleted = models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision,
            cleanup: ProviderModelCleanupPlan {
                projects: vec![model_cleanup_project_patch(&project_id, 1, "cleared")],
                templates: vec![model_cleanup_template_patch(&template, 1, "cleared")],
            },
        })
        .await
        .unwrap();

    assert!(deleted);
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_none());
    assert!(models.get(PROVIDER_ID, "keep-me").await.unwrap().is_some());
    assert!(
        SqliteProviderModelCapabilityRepository::new(db.pool().clone())
            .get(PROVIDER_ID, "delete-me", "image_generation")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        SqliteProviderModelCapabilityRepository::new(db.pool().clone())
            .get(PROVIDER_ID, "keep-me", "chat")
            .await
            .unwrap()
            .is_some()
    );
    let project: (i64, String) = sqlx::query_as(
        "SELECT revision, document_json FROM creative_studio_projects WHERE project_id = ?",
    )
    .bind(&project_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(project.0, 2);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&project.1).unwrap()["nodes"][0]["marker"], "cleared");
    let template_after: (i64, String) = sqlx::query_as(
        "SELECT revision, definition_json FROM creative_studio_templates WHERE template_id = ?",
    )
    .bind(&template.template_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(template_after.0, 2);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&template_after.1).unwrap()["marker"], "cleared");
    assert_eq!(
        SqliteProviderRepository::new(db.pool().clone())
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        expected_config_revision + 1
    );
}

#[tokio::test]
async fn coordinated_model_delete_rejects_live_creation_task_without_writes() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, false).await;
    let (project_id, original_document) = seed_model_cleanup_project(&db, "original").await;
    seed_live_model_creation_task(&db, &project_id).await;
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    let error = models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision: 0,
            cleanup: ProviderModelCleanupPlan {
                projects: vec![model_cleanup_project_patch(&project_id, 1, "must-not-write")],
                templates: vec![],
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("live creation task")));
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
    let project: (i64, String) = sqlx::query_as(
        "SELECT revision, document_json FROM creative_studio_projects WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(project, (1, original_document));
    assert_eq!(model_delete_provider_revision(&db).await, 0);
}

#[tokio::test]
async fn coordinated_model_delete_rejects_each_live_template_snapshot_binding() {
    for step_kind in ["generate-images", "draft-prompts"] {
        let db = init_database_memory().await.unwrap();
        seed_model_delete_provider(&db, false).await;
        let template = seed_model_cleanup_template(&db, "original").await;
        seed_model_template_run(&db, &template, step_kind, "queued").await;
        let models = SqliteProviderModelRepository::new(db.pool().clone());
        let error = models
            .delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: PROVIDER_ID.to_owned(),
                model: "delete-me".to_owned(),
                expected_config_revision: 0,
                cleanup: ProviderModelCleanupPlan {
                    projects: vec![],
                    templates: vec![model_cleanup_template_patch(
                        &template,
                        1,
                        "must-not-write",
                    )],
                },
            })
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("nonterminal template run")));
        assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
        let revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM creative_studio_templates WHERE template_id = ?",
        )
        .bind(&template.template_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(revision, 1, "{step_kind}");
        assert_eq!(model_delete_provider_revision(&db).await, 0, "{step_kind}");
    }
}

#[tokio::test]
async fn stale_project_cleanup_rolls_back_earlier_project_patch_and_model_delete() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, false).await;
    let (first_id, first_original) = seed_model_cleanup_project(&db, "first-original").await;
    let (stale_id, _) = seed_model_cleanup_project(&db, "stale-original").await;
    let stale_document = model_cleanup_project_patch(&stale_id, 1, "newer-writer").document_json;
    sqlx::query(
        "UPDATE creative_studio_projects SET revision = 2, document_json = ? WHERE project_id = ?",
    )
    .bind(&stale_document)
    .bind(&stale_id)
    .execute(db.pool())
    .await
    .unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    let error = models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision: 0,
            cleanup: ProviderModelCleanupPlan {
                projects: vec![
                    model_cleanup_project_patch(&first_id, 1, "first-cleaned"),
                    model_cleanup_project_patch(&stale_id, 1, "stale-cleaned"),
                ],
                templates: vec![],
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("project")));
    let first_after: (i64, String) = sqlx::query_as(
        "SELECT revision, document_json FROM creative_studio_projects WHERE project_id = ?",
    )
    .bind(&first_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(first_after, (1, first_original));
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
    assert_eq!(model_delete_provider_revision(&db).await, 0);
}

#[tokio::test]
async fn stale_template_cleanup_rolls_back_project_patch_and_model_delete() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, false).await;
    let (project_id, project_original) = seed_model_cleanup_project(&db, "original").await;
    let template = seed_model_cleanup_template(&db, "original").await;
    let stale_definition = serde_json::json!({
        "id": template.template_id,
        "revision": 2,
        "marker": "newer-writer"
    })
    .to_string();
    sqlx::query(
        "UPDATE creative_studio_templates SET revision = 2, definition_json = ?, updated_at = 150 \
         WHERE template_id = ?",
    )
    .bind(stale_definition)
    .bind(&template.template_id)
    .execute(db.pool())
    .await
    .unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    let error = models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision: 0,
            cleanup: ProviderModelCleanupPlan {
                projects: vec![model_cleanup_project_patch(&project_id, 1, "cleaned")],
                templates: vec![model_cleanup_template_patch(&template, 1, "cleaned")],
            },
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("template")));
    let project_after: (i64, String) = sqlx::query_as(
        "SELECT revision, document_json FROM creative_studio_projects WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(project_after, (1, project_original));
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
    assert_eq!(model_delete_provider_revision(&db).await, 0);
}

#[tokio::test]
async fn missing_model_returns_false_without_applying_cleanup() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, false).await;
    let (project_id, original_document) = seed_model_cleanup_project(&db, "original").await;
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    assert!(
        !models
            .delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: PROVIDER_ID.to_owned(),
                model: "missing".to_owned(),
                expected_config_revision: 0,
                cleanup: ProviderModelCleanupPlan {
                    projects: vec![model_cleanup_project_patch(
                        &project_id,
                        1,
                        "must-not-write",
                    )],
                    templates: vec![],
                },
            })
            .await
            .unwrap()
    );
    let project: (i64, String) = sqlx::query_as(
        "SELECT revision, document_json FROM creative_studio_projects WHERE project_id = ?",
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(project, (1, original_document));
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
    assert_eq!(
        SqliteProviderRepository::new(db.pool().clone())
            .find_by_id(PROVIDER_ID)
            .await
            .unwrap()
            .unwrap()
            .config_revision,
        0
    );
}

#[tokio::test]
async fn coordinated_model_delete_retargets_idle_conversation_model_and_pool() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, true).await;
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let repo = SqliteConversationRepository::new(db.pool().clone());
    let conversation = conversation_with_model(
        &owner,
        Some(model_json("delete-me")),
        Some(model_pool(&["delete-me", "keep-me"])),
        "finished",
    );
    let conversation_id = repo.create(&conversation).await.unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());

    assert!(
        models
            .delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: PROVIDER_ID.to_owned(),
                model: "delete-me".to_owned(),
                expected_config_revision: model_delete_provider_revision(&db).await,
                cleanup: ProviderModelCleanupPlan::default(),
            })
            .await
            .unwrap()
    );

    let updated = repo.get(&conversation_id).await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(updated.model.as_deref().unwrap()).unwrap(),
        model_json("keep-me")
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            updated.execution_model_pool.as_deref().unwrap()
        )
        .unwrap(),
        model_pool(&["keep-me"])
    );
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_none());
}

#[tokio::test]
async fn coordinated_model_delete_rejects_running_conversation_without_writes() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, true).await;
    add_chat_capability_to_model(&db, "keep-me").await;
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let repo = SqliteConversationRepository::new(db.pool().clone());
    let conversation = conversation_with_model(
        &owner,
        Some(model_json("delete-me")),
        Some(model_pool(&["delete-me", "keep-me"])),
        "running",
    );
    let conversation_id = repo.create(&conversation).await.unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    let receipt_id = format!("test-running-{conversation_id}");
    sqlx::query(
        "INSERT INTO conversation_delivery_receipts \
            (operation_id, message_id, conversation_id, projected_conversation_id, \
             projected_message_id, user_id, kind, request_payload, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, NULL, ?, 'turn', '{}', 'accepted', 1, 1)",
    )
    .bind(&receipt_id)
    .bind(nomifun_common::MessageId::new().into_string())
    .bind(&conversation_id)
    .bind(&conversation_id)
    .bind(&owner)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE conversations SET status = 'running', active_turn_operation_id = ?, \
             admission_epoch = admission_epoch + 1 WHERE conversation_id = ?",
    )
    .bind(&receipt_id)
    .bind(&conversation_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());
    let error = models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision: 1,
            cleanup: ProviderModelCleanupPlan::default(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, DbError::Conflict(message) if message.contains("running Conversation")));
    assert!(models.get(PROVIDER_ID, "delete-me").await.unwrap().is_some());
    let unchanged = repo.get(&conversation_id).await.unwrap().unwrap();
    let expected_model = model_json("delete-me").to_string();
    assert_eq!(unchanged.model.as_deref(), Some(expected_model.as_str()));
}

#[tokio::test]
async fn coordinated_model_delete_clears_idle_conversation_when_no_chat_fallback_exists() {
    let db = init_database_memory().await.unwrap();
    seed_model_delete_provider(&db, false).await;
    let owner = nomifun_db::installation_owner_id(db.pool()).await.unwrap();
    let repo = SqliteConversationRepository::new(db.pool().clone());
    let conversation = conversation_with_model(
        &owner,
        Some(model_json("delete-me")),
        Some(serde_json::json!({
            "mode": "single",
            "model": model_json("delete-me"),
        })),
        "finished",
    );
    let conversation_id = repo.create(&conversation).await.unwrap();
    let models = SqliteProviderModelRepository::new(db.pool().clone());

    models
        .delete_coordinated(&CoordinatedProviderModelDelete {
            provider_id: PROVIDER_ID.to_owned(),
            model: "delete-me".to_owned(),
            expected_config_revision: 0,
            cleanup: ProviderModelCleanupPlan::default(),
        })
        .await
        .unwrap();

    let updated = repo.get(&conversation_id).await.unwrap().unwrap();
    assert!(updated.model.is_none());
    assert!(updated.execution_model_pool.is_none());
}
