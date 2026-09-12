use std::path::PathBuf;
use std::sync::Arc;

use nomifun_ai_agent::factory::{AgentFactoryDeps, build_agent_factory};
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_common::{AgentType, ConversationId, ProviderWithModel, encrypt_string};
use nomifun_db::{
    CreateProviderParams, IProviderConnectionRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, UpdateProviderParams,
    init_database_memory,
};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, default_adapters};

const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PROVIDER_ID_1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PROVIDER_ID_2: &str = "0190f5fe-7c00-7a00-8000-000000000002";
const MISSING_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000099";

fn test_encryption_key() -> [u8; 32] {
    [0xABu8; 32]
}

async fn setup() -> (
    Arc<dyn IProviderRepository>,
    Arc<dyn IProviderModelRepository>,
    Arc<ModelInvokeService>,
) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let provider_repo: Arc<dyn IProviderRepository> = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let provider_model_repo: Arc<dyn IProviderModelRepository> =
        Arc::new(SqliteProviderModelRepository::new(pool.clone()));
    let capability_repo: Arc<dyn IProviderModelCapabilityRepository> =
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
    let connection_repo: Arc<dyn IProviderConnectionRepository> =
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone()));
    let model_invoke = Arc::new(ModelInvokeService::new(
        provider_repo.clone(),
        provider_model_repo.clone(),
        capability_repo,
        connection_repo,
        test_encryption_key(),
        reqwest::Client::new(),
        AdapterRegistry::new(default_adapters()),
    ));
    (provider_repo, provider_model_repo, model_invoke)
}

async fn insert_test_provider(
    repo: &dyn IProviderRepository,
    model_repo: &dyn IProviderModelRepository,
    id: &str,
    platform: &str,
) {
    let key = test_encryption_key();
    let encrypted_credentials =
        encrypt_string(r#"{"api_keys":["sk-test-key-12345"]}"#, &key).unwrap();
    let capabilities = [NewProviderModelCapability {
        task: "chat",
        traits: "[]",
        protocol: "openai.chat_text",
        connection_role: "default",
        provider_params: "{}",
        context_limit: Some(128_000),
        ..Default::default()
    }];
    let initial_model = NewProviderModel {
        model: "gpt-4o",
        enabled: true,
        sort_order: 0,
        description: None,
        capabilities: &capabilities,
    };
    let (provider, _) = repo.create(
        CreateProviderParams {
            provider_id: Some(id),
            platform,
            name: "Test Provider",
            base_url: "https://api.example.com/v1",
            auth_scheme: "bearer",
            credentials_encrypted: &encrypted_credentials,
            enabled: true,
            bedrock_config: None,
            sort_order: None,
        },
        &initial_model,
        &[],
    )
    .await
    .unwrap();
    model_repo
        .save(
            id,
            provider.config_revision,
            &NewProviderModel {
                model: "gpt-5.4",
                enabled: true,
                sort_order: 1,
                description: None,
                capabilities: &capabilities,
            },
        )
        .await
        .unwrap();
}

fn make_factory(
    model_invoke: Arc<ModelInvokeService>,
) -> nomifun_ai_agent::runtime_registry::AgentRuntimeFactory {
    build_agent_factory(AgentFactoryDeps {
        authoritative_user_id: Arc::from(TEST_OWNER_ID),
        cron_sink_factory: None,
        gateway_mcp_config: None,
        #[cfg(feature = "browser-use")]
        browser_lane_provider: None,
        client_prefs: None,
        settings_repo: None,
        companion_prompt: None,
        ssh_provider: None,
        companion_skill_sink: None,
        model_invoke,
        model_invoke_service: None,
        provider_config_digest_resolver: None,
        encryption_key: test_encryption_key(),
        data_dir: PathBuf::from("/tmp/nomi-test"),
        work_dir: PathBuf::from("/tmp/nomi-test"),
        mcp_server_repo: None,
        mcp_oauth_service: None,
        requirement_sink: None,
        companion_sink: None,
        knowledge_retrieval: None,
        knowledge_writeback: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_returns_unavailable_when_no_providers_configured() {
    // With no provider row, exact capability resolution fails at the selected
    // provider. No catalog-wide fallback is allowed.
    let (_provider_repo, _provider_model_repo, model_invoke) = setup().await;
    let factory = make_factory(model_invoke);

    let options = AgentRuntimeBuildOptions {
        user_id: TEST_OWNER_ID.into(),
        agent_type: AgentType::Nomi,
        workspace: "/tmp/test-workspace".into(),
        model: Some(ProviderWithModel {
            provider_id: MISSING_PROVIDER_ID.into(),
            model: "gpt-4o".into(),
            use_model: None,
        }),
        conversation_id: ConversationId::new().into_string(),
        delegation_policy: Default::default(),
        conversation_created_at: Some(1),
        workspace_binding_lease: None,
        extra: serde_json::json!({}),
    };

    let result = factory(options).await;
    match result {
        Ok(_) => panic!("expected exact missing-provider failure, got Ok"),
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("provider not found"),
                "expected exact provider-not-found error, got: {err_msg}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_rejects_missing_bound_provider_without_fallback() {
    // A missing bound provider fails exactly. Another enabled provider must
    // never be substituted because that would bypass the selected capability.
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let factory = make_factory(model_invoke);

    let options = AgentRuntimeBuildOptions {
        user_id: TEST_OWNER_ID.into(),
        agent_type: AgentType::Nomi,
        workspace: "/tmp/test-workspace".into(),
        model: Some(ProviderWithModel {
            provider_id: MISSING_PROVIDER_ID.into(),
            model: "gpt-4o".into(),
            use_model: None,
        }),
        conversation_id: ConversationId::new().into_string(),
        delegation_policy: Default::default(),
        conversation_created_at: Some(1),
        workspace_binding_lease: None,
        extra: serde_json::json!({}),
    };

    let result = factory(options).await;
    let error = result.err().expect("missing provider must fail exactly");
    assert!(error.to_string().contains("provider not found"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_resolves_provider_from_db() {
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let factory = make_factory(model_invoke);

    let options = AgentRuntimeBuildOptions {
        user_id: TEST_OWNER_ID.into(),
        agent_type: AgentType::Nomi,
        workspace: "/tmp/test-workspace".into(),
        model: Some(ProviderWithModel {
            provider_id: PROVIDER_ID_1.into(),
            model: "gpt-4o".into(),
            use_model: None,
        }),
        conversation_id: ConversationId::new().into_string(),
        delegation_policy: Default::default(),
        conversation_created_at: Some(1),
        workspace_binding_lease: None,
        extra: serde_json::json!({}),
    };

    let result = factory(options).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_respects_use_model_override() {
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_2,
        "openai",
    )
    .await;
    let factory = make_factory(model_invoke);

    let options = AgentRuntimeBuildOptions {
        user_id: TEST_OWNER_ID.into(),
        agent_type: AgentType::Nomi,
        workspace: "/tmp/test-workspace".into(),
        model: Some(ProviderWithModel {
            provider_id: PROVIDER_ID_2.into(),
            model: "gpt-4o".into(),
            use_model: Some("gpt-5.4".into()),
        }),
        conversation_id: ConversationId::new().into_string(),
        delegation_policy: Default::default(),
        conversation_created_at: Some(1),
        workspace_binding_lease: None,
        extra: serde_json::json!({}),
    };

    let result = factory(options).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

#[tokio::test]
async fn exact_provider_revision_rejects_a_stale_agent_snapshot() {
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let frozen_revision = provider_repo
        .find_by_id(PROVIDER_ID_1)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    let workspace = PathBuf::from("/tmp/test-workspace");
    nomifun_ai_agent::resolve_provider_config_at_revision(
        model_invoke.as_ref(),
        PROVIDER_ID_1,
        "gpt-4o",
        frozen_revision,
        workspace.as_path(),
    )
    .await
    .expect("the frozen provider graph should resolve before it changes");

    provider_repo
        .update(
            PROVIDER_ID_1,
            frozen_revision,
            UpdateProviderParams {
                base_url: Some("https://changed.example.invalid/v1"),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let error = nomifun_ai_agent::resolve_provider_config_at_revision(
        model_invoke.as_ref(),
        PROVIDER_ID_1,
        "gpt-4o",
        frozen_revision,
        workspace.as_path(),
    )
    .await
    .expect_err("a stale Agent Snapshot must not adopt the updated provider graph");
    assert!(matches!(error, nomifun_common::AppError::Conflict(_)));
}
