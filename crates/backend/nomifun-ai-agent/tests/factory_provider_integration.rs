use std::path::PathBuf;
use std::sync::Arc;

use nomifun_ai_agent::factory::{AgentFactoryDeps, build_agent_factory};
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_common::{AgentType, ConversationId, ProviderWithModel, encrypt_string};
use nomifun_db::{
    CreateProviderParams, IProviderConnectionRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
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
    make_factory_with_summon(model_invoke, None)
}

fn make_factory_with_summon(
    model_invoke: Arc<ModelInvokeService>,
    companion_summon: Option<Arc<dyn nomifun_ai_agent::CompanionSummonProvider>>,
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
        companion_summon,
        ssh_provider: None,
        companion_skill_sink: None,
        computer_history_sink: None,
        model_invoke,
        model_invoke_service: None,
        encryption_key: test_encryption_key(),
        data_dir: PathBuf::from("/tmp/nomi-test"),
        work_dir: PathBuf::from("/tmp/nomi-test"),
        mcp_server_repo: None,
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

// ── In-session companion summon (spec §设计 B) ─────────────────────────────

/// Records provider consultations so the test can assert the factory's summon
/// gating without reaching into the (private) engine tool registry — tool
/// registration itself is covered by the manager unit tests.
struct FakeSummonProvider {
    sync_calls: std::sync::Mutex<Vec<(String, String, Vec<String>)>>,
    clear_calls: std::sync::Mutex<Vec<String>>,
}

impl FakeSummonProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sync_calls: std::sync::Mutex::new(Vec::new()),
            clear_calls: std::sync::Mutex::new(Vec::new()),
        })
    }
}

struct FakeSummonMemorySink;

#[async_trait::async_trait]
impl nomifun_ai_agent::CompanionMemorySink for FakeSummonMemorySink {
    async fn recall(&self, _c: &str, _q: &[String], _k: Option<&str>, _a: bool, _l: usize) -> Result<String, String> {
        Ok(String::new())
    }
    async fn save(&self, _c: &str, _k: &str, _co: &str, _t: &[String]) -> Result<String, String> {
        Err("read-only".into())
    }
    async fn recent_events(&self, _l: usize) -> Result<String, String> {
        Err("unavailable".into())
    }
}

struct FakeSummonContextSink;

#[async_trait::async_trait]
impl nomifun_ai_agent::SummonContextSink for FakeSummonContextSink {
    async fn resolve_context(&self) -> Option<String> {
        None
    }
}

#[async_trait::async_trait]
impl nomifun_ai_agent::CompanionSummonProvider for FakeSummonProvider {
    async fn companion_name(&self, _companion_id: &str) -> Option<String> {
        Some("小助".into())
    }
    fn summon_memory_sink(
        &self,
        _companion_id: &str,
    ) -> Result<Arc<dyn nomifun_ai_agent::CompanionMemorySink>, nomifun_common::AppError> {
        Ok(Arc::new(FakeSummonMemorySink))
    }
    fn summon_context_sink(
        &self,
        _config: &nomifun_api_types::SummonConfig,
    ) -> Result<Arc<dyn nomifun_ai_agent::SummonContextSink>, nomifun_common::AppError> {
        Ok(Arc::new(FakeSummonContextSink))
    }
    async fn sync_summon_workspace_skills(
        &self,
        conversation_id: &str,
        _workspace: &std::path::Path,
        companion_id: &str,
        skill_exclusions: &[String],
    ) -> Result<Vec<String>, nomifun_common::AppError> {
        self.sync_calls.lock().unwrap().push((
            conversation_id.to_owned(),
            companion_id.to_owned(),
            skill_exclusions.to_vec(),
        ));
        Ok(vec!["skill-a".into()])
    }
    async fn clear_summon_workspace_skills(
        &self,
        conversation_id: &str,
        _workspace: &std::path::Path,
    ) -> Result<(), nomifun_common::AppError> {
        self.clear_calls.lock().unwrap().push(conversation_id.to_owned());
        Ok(())
    }
}

const SUMMON_COMPANION_ID: &str = "0190f5fe-7c00-7a00-8abc-000000000001";

fn summon_build_options(conversation_id: &str, extra: serde_json::Value) -> AgentRuntimeBuildOptions {
    AgentRuntimeBuildOptions {
        user_id: TEST_OWNER_ID.into(),
        agent_type: AgentType::Nomi,
        workspace: "/tmp/test-workspace".into(),
        model: Some(ProviderWithModel {
            provider_id: PROVIDER_ID_1.into(),
            model: "gpt-4o".into(),
            use_model: None,
        }),
        conversation_id: conversation_id.to_owned(),
        delegation_policy: Default::default(),
        conversation_created_at: Some(1),
        workspace_binding_lease: None,
        extra,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_summon_session_consults_provider_and_materializes_skills() {
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        model_invoke,
        Some(fake.clone()),
    );

    let conversation_id = ConversationId::new().into_string();
    let result = factory(summon_build_options(
        &conversation_id,
        serde_json::json!({
            "summon": {
                "companion_id": SUMMON_COMPANION_ID,
                "memory_ids": [],
                "skill_exclusions": ["heavy-refactor"],
                "summoned_at": 1,
            }
        }),
    ))
    .await;
    assert!(result.is_ok(), "summon session must build: {:?}", result.err());

    let sync_calls = fake.sync_calls.lock().unwrap().clone();
    assert_eq!(sync_calls.len(), 1, "skills materialized exactly once per build");
    assert_eq!(sync_calls[0].0, conversation_id);
    assert_eq!(sync_calls[0].1, SUMMON_COMPANION_ID);
    assert_eq!(sync_calls[0].2, vec!["heavy-refactor".to_owned()]);
    assert!(fake.clear_calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_plain_session_runs_summon_cleanup_only() {
    // A non-summoned session build triggers the manifest-owned cleanup path
    // (unloads skills after 解除召唤 on the next build) and never syncs.
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        model_invoke,
        Some(fake.clone()),
    );

    let conversation_id = ConversationId::new().into_string();
    let result = factory(summon_build_options(&conversation_id, serde_json::json!({}))).await;
    assert!(result.is_ok(), "{:?}", result.err());
    assert!(fake.sync_calls.lock().unwrap().is_empty());
    assert_eq!(fake.clear_calls.lock().unwrap().as_slice(), &[conversation_id]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_companion_session_ignores_summon() {
    // persona boundary: a companion conversation never consults the summon
    // provider — neither sync nor cleanup (its manifest belongs to the
    // companion-thread skill reconciler).
    let (provider_repo, provider_model_repo, model_invoke) = setup().await;
    insert_test_provider(
        provider_repo.as_ref(),
        provider_model_repo.as_ref(),
        PROVIDER_ID_1,
        "openai",
    )
    .await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        model_invoke,
        Some(fake.clone()),
    );

    let result = factory(summon_build_options(
        &ConversationId::new().into_string(),
        serde_json::json!({
            "companion_session": true,
            "summon": {
                "companion_id": SUMMON_COMPANION_ID,
                "summoned_at": 1,
            }
        }),
    ))
    .await;
    assert!(result.is_ok(), "{:?}", result.err());
    assert!(fake.sync_calls.lock().unwrap().is_empty(), "companion sessions must not summon");
    assert!(fake.clear_calls.lock().unwrap().is_empty(), "companion manifests are not summon-owned");
}
