use std::path::PathBuf;
use std::sync::Arc;

use nomifun_ai_agent::AcpSessionSyncService;
use nomifun_ai_agent::AcpSkillManager;
use nomifun_ai_agent::factory::{AgentFactoryDeps, build_agent_factory};
use nomifun_ai_agent::registry::AgentRegistry;
use nomifun_ai_agent::types::AgentRuntimeBuildOptions;
use nomifun_common::{AgentType, ConversationId, ProviderWithModel, encrypt_string};
use nomifun_db::{
    CreateProviderParams, IAcpSessionRepository, IProviderRepository, SqliteAcpSessionRepository,
    SqliteAgentMetadataRepository, SqliteProviderRepository, SqliteRemoteAgentRepository, init_database_memory,
};

const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PROVIDER_ID_1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
const PROVIDER_ID_2: &str = "0190f5fe-7c00-7a00-8000-000000000002";
const MISSING_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000099";

fn test_encryption_key() -> [u8; 32] {
    [0xABu8; 32]
}

async fn setup() -> (
    Arc<dyn IProviderRepository>,
    Arc<dyn nomifun_db::IProviderModelRepository>,
    Arc<SqliteRemoteAgentRepository>,
    Arc<AgentRegistry>,
    Arc<AcpSessionSyncService>,
) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let provider_repo: Arc<dyn IProviderRepository> = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository> =
        Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone()));
    let remote_agent_repo = Arc::new(SqliteRemoteAgentRepository::new(pool.clone()));
    let metadata_repo = Arc::new(SqliteAgentMetadataRepository::new(pool.clone()));
    let registry = AgentRegistry::new(metadata_repo);
    registry.hydrate().await.unwrap();
    let session_repo: Arc<dyn IAcpSessionRepository> = Arc::new(SqliteAcpSessionRepository::new(pool));
    let acp_agent_service = AcpSessionSyncService::new(session_repo);
    (provider_repo, provider_model_repo, remote_agent_repo, registry, acp_agent_service)
}

async fn insert_test_provider(repo: &dyn IProviderRepository, id: &str, platform: &str) {
    let key = test_encryption_key();
    let encrypted_api_key = encrypt_string("sk-test-key-12345", &key).unwrap();
    repo.create(CreateProviderParams {
        provider_id: Some(id),
        platform,
        name: "Test Provider",
        base_url: "https://api.example.com/v1",
        api_key_encrypted: &encrypted_api_key,
        models: r#"["gpt-4o","gpt-5.4"]"#,
        enabled: true,
        model_context_limits: None,
        model_protocols: None,
        model_descriptions: None,
        model_enabled: None,
        bedrock_config: None,
        is_full_url: false,
        sort_order: None,
    })
    .await
    .unwrap();
}

fn make_factory(
    provider_repo: Arc<dyn IProviderRepository>,
    provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    remote_agent_repo: Arc<SqliteRemoteAgentRepository>,
    agent_registry: Arc<AgentRegistry>,
    acp_agent_service: Arc<AcpSessionSyncService>,
) -> nomifun_ai_agent::runtime_registry::AgentRuntimeFactory {
    make_factory_with_summon(
        provider_repo,
        provider_model_repo,
        remote_agent_repo,
        agent_registry,
        acp_agent_service,
        None,
    )
}

fn make_factory_with_summon(
    provider_repo: Arc<dyn IProviderRepository>,
    provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    remote_agent_repo: Arc<SqliteRemoteAgentRepository>,
    agent_registry: Arc<AgentRegistry>,
    acp_agent_service: Arc<AcpSessionSyncService>,
    companion_summon: Option<Arc<dyn nomifun_ai_agent::CompanionSummonProvider>>,
) -> nomifun_ai_agent::runtime_registry::AgentRuntimeFactory {
    let tmp = tempfile::TempDir::new().unwrap();
    let skill_paths = Arc::new(nomifun_extension::resolve_skill_paths(tmp.path(), tmp.path()));
    build_agent_factory(AgentFactoryDeps {
        authoritative_user_id: Arc::from(TEST_OWNER_ID),
        cron_sink_factory: None,
        gateway_mcp_config: None,
        open_mcp_config: None,
        computer_mcp_config: None,
        browser_mcp_config: None,
        #[cfg(feature = "browser-use")]
        browser_lane_provider: None,
        client_prefs: None,
        settings_repo: None,
        companion_prompt: None,
        companion_summon,
        ssh_provider: None,
        companion_skill_sink: None,
        skill_manager: AcpSkillManager::new(skill_paths),
        remote_agent_repo,
        provider_repo,
        provider_model_repo,
        model_invoke_service: None,
        encryption_key: test_encryption_key(),
        agent_registry,
        acp_agent_service,
        data_dir: PathBuf::from("/tmp/nomi-test"),
        work_dir: PathBuf::from("/tmp/nomi-test"),
        backend_binary_path: Arc::new(PathBuf::from("/tmp/nomi-test/nomicore")),
        requirement_mcp_config: None,
        knowledge_mcp_config: None,
        mcp_server_repo: None,
        requirement_sink: None,
        companion_sink: None,
        knowledge_retrieval: None,
        knowledge_writeback: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_returns_unavailable_when_no_providers_configured() {
    // With NO providers in the DB, a conversation bound to any provider id has
    // nothing to fall back to → ProviderUnavailable (the friendly terminal error,
    // surfaced to the user as "no usable model" rather than a raw provider id).
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    let factory = make_factory(provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service);

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
        Ok(_) => panic!("Expected ProviderUnavailable error when no providers configured, got Ok"),
        Err(e) => {
            let err_msg = e.to_string();
            assert!(
                err_msg.contains("No usable model provider"),
                "Expected ProviderUnavailable error, got: {err_msg}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_falls_back_to_first_enabled_when_bound_provider_missing() {
    // A conversation bound to a DELETED provider must NOT hard-fail while an
    // enabled provider still exists — it falls back to the first enabled model
    // instead of erroring with "Provider '<id>' not found".
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_1, "openai").await;
    let factory = make_factory(provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service);

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
    assert!(result.is_ok(), "Expected fallback Ok, got: {:?}", result.err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_resolves_provider_from_db() {
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_1, "openai").await;
    let factory = make_factory(provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service);

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
        extra: serde_json::json!({ "max_tokens": 2048 }),
    };

    let result = factory(options).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result.err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nomi_factory_respects_use_model_override() {
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_2, "openai").await;
    let factory = make_factory(provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service);

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
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_1, "openai").await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        provider_repo,
        provider_model_repo,
        remote_agent_repo,
        agent_registry,
        acp_agent_service,
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
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_1, "openai").await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        provider_repo,
        provider_model_repo,
        remote_agent_repo,
        agent_registry,
        acp_agent_service,
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
    let (provider_repo, provider_model_repo, remote_agent_repo, agent_registry, acp_agent_service) = setup().await;
    insert_test_provider(&*provider_repo, PROVIDER_ID_1, "openai").await;
    let fake = FakeSummonProvider::new();
    let factory = make_factory_with_summon(
        provider_repo,
        provider_model_repo,
        remote_agent_repo,
        agent_registry,
        acp_agent_service,
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
