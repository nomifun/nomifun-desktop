//! DB capability -> Agent provider config contract for every Chat serializer.

use std::sync::Arc;

use nomi_config::config::ProviderType;
use nomifun_ai_agent::resolve_provider_config;
use nomifun_common::encrypt_string;
use nomifun_db::{
    CreateProviderParams, IProviderConnectionRepository, IProviderModelCapabilityRepository,
    IProviderModelRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, default_adapters};

const TEST_KEY: [u8; 32] = [0x4C; 32];
const OPENAI_ID: &str = "0190f5fe-7c00-7a00-8000-000000000011";
const ANTHROPIC_ID: &str = "0190f5fe-7c00-7a00-8000-000000000012";
const GEMINI_ID: &str = "0190f5fe-7c00-7a00-8000-000000000013";
const BEDROCK_ID: &str = "0190f5fe-7c00-7a00-8000-000000000014";

struct Harness {
    provider_repo: Arc<SqliteProviderRepository>,
    invoke: ModelInvokeService,
}

async fn setup() -> Harness {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let model_repo: Arc<dyn IProviderModelRepository> =
        Arc::new(SqliteProviderModelRepository::new(pool.clone()));
    let capability_repo: Arc<dyn IProviderModelCapabilityRepository> =
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
    let connection_repo: Arc<dyn IProviderConnectionRepository> =
        Arc::new(SqliteProviderConnectionRepository::new(pool));
    let invoke = ModelInvokeService::new(
        provider_repo.clone(),
        model_repo,
        capability_repo,
        connection_repo,
        TEST_KEY,
        reqwest::Client::new(),
        AdapterRegistry::new(default_adapters()),
    );
    Harness {
        provider_repo,
        invoke,
    }
}

struct ProviderFixture<'a> {
    id: &'a str,
    platform: &'a str,
    base_url: &'a str,
    auth_scheme: &'a str,
    credentials_json: &'a str,
    bedrock_config: Option<&'a str>,
    model: &'a str,
    protocol: &'a str,
    endpoint: Option<&'a str>,
    provider_params: &'a str,
}

async fn insert_provider(repo: &SqliteProviderRepository, fixture: ProviderFixture<'_>) {
    let encrypted_credentials = encrypt_string(fixture.credentials_json, &TEST_KEY).unwrap();
    let capabilities = [NewProviderModelCapability {
        task: "chat",
        traits: "[\"streaming\"]",
        protocol: fixture.protocol,
        connection_role: "default",
        endpoint: fixture.endpoint,
        provider_params: fixture.provider_params,
        context_limit: Some(32_000),
        ..Default::default()
    }];
    let model = NewProviderModel {
        model: fixture.model,
        enabled: true,
        sort_order: 0,
        description: None,
        capabilities: &capabilities,
    };
    repo.create(
        CreateProviderParams {
            provider_id: Some(fixture.id),
            platform: fixture.platform,
            name: fixture.platform,
            base_url: fixture.base_url,
            auth_scheme: fixture.auth_scheme,
            credentials_encrypted: &encrypted_credentials,
            enabled: true,
            bedrock_config: fixture.bedrock_config,
            sort_order: None,
        },
        &model,
        &[],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn capability_endpoints_and_sdk_config_reach_the_matching_nomi_serializer() {
    let harness = setup().await;
    insert_provider(
        harness.provider_repo.as_ref(),
        ProviderFixture {
            id: OPENAI_ID,
            platform: "openai",
            base_url: "https://gateway.openai.example/root",
            auth_scheme: "bearer",
            credentials_json: r#"{"api_keys":["sk-openai-1","sk-openai-2"]}"#,
            bedrock_config: None,
            model: "gpt-contract",
            protocol: "openai.chat_text",
            endpoint: Some(
                "https://gateway.openai.example/tenant/chat?api-version=2026-08-11",
            ),
            provider_params: r#"{"temperature":0.3,"future":{"mode":"fast"},"max_tokens_field":"max_completion_tokens"}"#,
        },
    )
    .await;
    insert_provider(
        harness.provider_repo.as_ref(),
        ProviderFixture {
            id: ANTHROPIC_ID,
            platform: "anthropic",
            base_url: "https://gateway.anthropic.example/root",
            auth_scheme: "header_key:x-api-key",
            credentials_json: r#"{"api_keys":["sk-anthropic-1","sk-anthropic-2"]}"#,
            bedrock_config: None,
            model: "claude-contract",
            protocol: "anthropic.messages",
            endpoint: Some(
                "https://gateway.anthropic.example/tenant/messages?api-version=2026-08-11",
            ),
            provider_params: r#"{"top_k":7}"#,
        },
    )
    .await;
    insert_provider(
        harness.provider_repo.as_ref(),
        ProviderFixture {
            id: GEMINI_ID,
            platform: "gemini",
            base_url: "https://generativelanguage.googleapis.com",
            auth_scheme: "header_key:x-goog-api-key",
            credentials_json: r#"{"api_keys":["sk-gemini-1","sk-gemini-2"]}"#,
            bedrock_config: None,
            model: "gemini-contract",
            protocol: "gemini.generate_text",
            endpoint: None,
            provider_params: r#"{"generationConfig":{"temperature":0.7}}"#,
        },
    )
    .await;
    insert_provider(
        harness.provider_repo.as_ref(),
        ProviderFixture {
            id: BEDROCK_ID,
            platform: "bedrock",
            base_url: "",
            auth_scheme: "bedrock",
            credentials_json: "{}",
            bedrock_config: Some(
                r#"{"auth_method":"profile","region":"us-east-1","profile":"contract"}"#,
            ),
            model: "anthropic.claude-contract-v1:0",
            protocol: "bedrock.anthropic_messages",
            endpoint: None,
            provider_params: r#"{"top_k":11}"#,
        },
    )
    .await;

    let workspace = tempfile::tempdir().unwrap();

    let openai = resolve_provider_config(
        &harness.invoke,
        OPENAI_ID,
        "gpt-contract",
        workspace.path(),
    )
    .await
    .unwrap();
    assert_eq!(openai.provider, ProviderType::OpenAI);
    assert_eq!(
        openai.base_url,
        "https://gateway.openai.example/tenant/chat?api-version=2026-08-11"
    );
    assert_eq!(openai.compat.api_path.as_deref(), Some(""));
    assert_eq!(openai.api_key, "sk-openai-1\nsk-openai-2");
    assert_eq!(openai.compat.max_tokens_field.as_deref(), Some("max_completion_tokens"));
    assert_eq!(openai.compat.extra_body.as_ref().unwrap()["temperature"], 0.3);
    assert_eq!(openai.compat.extra_body.as_ref().unwrap()["future"]["mode"], "fast");
    assert!(openai.compat.extra_body.as_ref().unwrap().get("max_tokens_field").is_none());

    let anthropic = resolve_provider_config(
        &harness.invoke,
        ANTHROPIC_ID,
        "claude-contract",
        workspace.path(),
    )
    .await
    .unwrap();
    assert_eq!(anthropic.provider, ProviderType::Anthropic);
    assert_eq!(
        anthropic.base_url,
        "https://gateway.anthropic.example/tenant/messages?api-version=2026-08-11"
    );
    assert_eq!(anthropic.compat.api_path.as_deref(), Some(""));
    assert_eq!(anthropic.api_key, "sk-anthropic-1\nsk-anthropic-2");
    assert_eq!(anthropic.compat.extra_body.as_ref().unwrap()["top_k"], 7);

    let gemini = resolve_provider_config(
        &harness.invoke,
        GEMINI_ID,
        "gemini-contract",
        workspace.path(),
    )
    .await
    .unwrap();
    assert_eq!(gemini.provider, ProviderType::Gemini);
    assert_eq!(
        gemini.base_url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-contract:streamGenerateContent?alt=sse"
    );
    assert_eq!(gemini.compat.api_path.as_deref(), Some(""));
    assert_eq!(gemini.api_key, "sk-gemini-1\nsk-gemini-2");
    assert_eq!(gemini.compat.extra_body.as_ref().unwrap()["generationConfig"]["temperature"], 0.7);

    let bedrock = resolve_provider_config(
        &harness.invoke,
        BEDROCK_ID,
        "anthropic.claude-contract-v1:0",
        workspace.path(),
    )
    .await
    .unwrap();
    assert_eq!(bedrock.provider, ProviderType::Bedrock);
    assert!(bedrock.base_url.is_empty());
    assert!(bedrock.compat.api_path.is_none());
    assert_eq!(bedrock.compat.extra_body.as_ref().unwrap()["top_k"], 11);
    let bedrock_config = bedrock.bedrock.expect("Bedrock SDK config");
    assert_eq!(bedrock_config.region.as_deref(), Some("us-east-1"));
    assert_eq!(bedrock_config.profile.as_deref(), Some("contract"));
}
