mod fetchers;
mod probe;
mod url_fixer;

use std::sync::Arc;

use nomifun_api_types::{
    BedrockConfig, FetchModelsAnonymousRequest, FetchModelsRequest, FetchModelsResponse,
    ModelCatalogSource, ModelInfo, ModelTaskSource, infer_catalog_tasks_and_traits,
    is_retired_provider_model, verified_catalog_tasks_and_traits,
};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::IProviderRepository;
use nomifun_model_invoke::{AuthMaterial, AuthScheme};

use crate::provider::{deserialize_opt, validate_provider_auth, validate_provider_base_url};
use crate::provider_connection::decrypt_credentials;

type HttpClientFactory = Arc<dyn Fn() -> reqwest::Client + Send + Sync>;

/// Internal configuration extracted from a provider row for model fetching.
#[derive(Clone)]
pub(crate) struct FetchConfig {
    pub platform: String,
    pub base_url: String,
    pub auth: AuthMaterial,
    pub bedrock_config: Option<BedrockConfig>,
}

impl std::fmt::Debug for FetchConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FetchConfig")
            .field("platform", &self.platform)
            .field("base_url", &self.base_url)
            .field("auth_scheme", &self.auth.scheme)
            .field("credentials", &"<redacted>")
            .field("bedrock_config", &self.bedrock_config)
            .finish()
    }
}

/// Service for fetching model lists from remote provider APIs.
#[derive(Clone)]
pub struct ModelFetchService {
    repo: Arc<dyn IProviderRepository>,
    encryption_key: [u8; 32],
    http_client: HttpClientFactory,
}

impl ModelFetchService {
    pub fn new(
        repo: Arc<dyn IProviderRepository>,
        encryption_key: [u8; 32],
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            repo,
            encryption_key,
            http_client: Arc::new(move || http_client.clone()),
        }
    }

    pub fn new_dynamic(repo: Arc<dyn IProviderRepository>, encryption_key: [u8; 32]) -> Self {
        Self {
            repo,
            encryption_key,
            http_client: Arc::new(nomifun_net::http_client),
        }
    }

    fn http_client(&self) -> reqwest::Client {
        (self.http_client)()
    }

    /// Fetch models for a provider by ID. If `try_fix` is true and the
    /// initial request fails on an OpenAI-compatible platform, attempt
    /// URL auto-correction with parallel probing.
    pub async fn fetch_models(
        &self,
        provider_id: &str,
        req: &FetchModelsRequest,
    ) -> Result<FetchModelsResponse, AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))?;
        let config = self.load_provider_config(provider_id).await?;
        self.fetch_with_config(&config, req.try_fix).await
    }

    /// Fetch models using credentials supplied in the request, without a
    /// persisted provider row. Powers the pre-create "Fetch Models" preview
    /// in the Add-Platform form.
    pub async fn fetch_models_anonymous(
        &self,
        req: &FetchModelsAnonymousRequest,
    ) -> Result<FetchModelsResponse, AppError> {
        if crate::provider::is_retired_provider_platform(req.platform.trim()) {
            return Err(AppError::Forbidden(
                "This retired built-in provider platform cannot be recreated".into(),
            ));
        }
        validate_anonymous_request(req)?;
        let config = FetchConfig {
            platform: req.platform.clone(),
            base_url: req.base_url.clone(),
            auth: AuthMaterial {
                scheme: parse_auth_scheme(&req.auth_scheme)?,
                credentials: req.credentials.clone(),
            },
            bedrock_config: req.bedrock_config.clone(),
        };
        self.fetch_with_config(&config, req.try_fix).await
    }

    /// Shared fetch+try_fix branch used by both the by-id and anonymous
    /// entry points.
    async fn fetch_with_config(
        &self,
        config: &FetchConfig,
        try_fix: bool,
    ) -> Result<FetchModelsResponse, AppError> {
        if config.platform == crate::model_gateway::PLATFORM {
            let catalog = crate::model_gateway::fetch_catalog(&config.base_url, &config.primary_secret()?).await?;
            return Ok(FetchModelsResponse {
                models: catalog.models.into_iter().map(|model| {
                    let traits = catalog.imports.iter().find(|m| m.model == model.id).and_then(|m| m.capabilities.iter().find(|c| c.task == nomifun_api_types::ModelTask::Chat)).map(|c| c.traits.clone()).unwrap_or_default();
                    ModelInfo { id: model.id, name: Some(model.display_name), tasks: model.tasks,
                    tasks_source: Some(ModelTaskSource::ProviderDeclared),
                    traits,
                    context_limit: model.context_window, output_limit: model.max_output_tokens,
                    token_limit_sources: None,
                }}).collect(),
                catalog_source: Some(ModelCatalogSource::Remote), fixed_base_url: None,
            });
        }
        let http_client = self.http_client();
        let catalog_platform = fetchers::catalog_platform(&config.platform, &config.base_url);
        match fetchers::fetch_for_platform(&http_client, &config).await {
            Ok(catalog) => Ok(fetch_models_response(catalog_platform, catalog.models, catalog.source, None)),
            Err(err)
                if try_fix
                    && supports_url_fix(catalog_platform)
                    && !(catalog_platform == "dashscope"
                        && fetchers::dashscope_native_models_url(&config.base_url).is_some())
                    && is_url_fix_candidate(&err) =>
            {
                url_fixer::try_fix_url(&http_client, &config)
                    .await
                    .map(|mut response| {
                        prepare_catalog_models(catalog_platform, &mut response.models);
                        response
                    })
                    .map_err(|_| err)
            }
            Err(err) => Err(err),
        }
    }

    /// Extract and decrypt provider configuration from DB.
    async fn load_provider_config(&self, provider_id: &str) -> Result<FetchConfig, AppError> {
        let row = self
            .repo
            .find_by_id(provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider {provider_id} not found")))?;
        if crate::provider::is_retired_provider_platform(&row.platform) {
            return Err(AppError::Forbidden(
                "This retired built-in provider no longer exposes a model catalog".into(),
            ));
        }

        let credentials =
            decrypt_credentials(&row.credentials_encrypted, &self.encryption_key)?;

        let bedrock_config: Option<BedrockConfig> =
            deserialize_opt(&row.bedrock_config, "bedrock_config")?;
        validate_provider_auth(
            &row.platform,
            &row.auth_scheme,
            &credentials,
            bedrock_config.as_ref(),
        )?;

        Ok(FetchConfig {
            platform: row.platform,
            base_url: row.base_url,
            auth: AuthMaterial {
                scheme: parse_auth_scheme(&row.auth_scheme)?,
                credentials,
            },
            bedrock_config,
        })
    }
}

fn prepare_catalog_models(platform: &str, models: &mut Vec<ModelInfo>) {
    // Catalogs can lag an official shutdown. Apply the same lifecycle rule as
    // saved provider/model projections; taskless retired rows are not choices.
    models.retain(|model| !is_retired_provider_model(platform, &model.id));
    for model in models {
        // Bedrock catalog rows are authoritative at the protocol-family
        // boundary: only Anthropic/Claude entries can use the implemented
        // `bedrock.anthropic_messages` adapter. Leaving every other family
        // taskless prevents the generic name fallback from claiming Chat.
        if platform.eq_ignore_ascii_case("bedrock") && model.tasks.is_empty() {
            continue;
        }
        let (tasks, traits) = infer_catalog_tasks_and_traits(platform, &model.id);
        if model.tasks.is_empty() {
            model.tasks = tasks;
            model.tasks_source = Some(if verified_catalog_tasks_and_traits(platform, &model.id).is_some() {
                ModelTaskSource::OfficialDocumentation
            } else {
                ModelTaskSource::Inferred
            });
        }
        if model.traits.is_empty() {
            model.traits = traits;
        }
    }
}

fn fetch_models_response(
    platform: &str,
    mut models: Vec<ModelInfo>,
    source: ModelCatalogSource,
    fixed_base_url: Option<String>,
) -> FetchModelsResponse {
    prepare_catalog_models(platform, &mut models);
    FetchModelsResponse { models, catalog_source: Some(source), fixed_base_url }
}

impl FetchConfig {
    fn primary_secret(&self) -> Result<String, AppError> {
        self.auth
            .primary_secret()
            .map_err(|error| AppError::BadRequest(error.to_string()))
    }
}

fn credentials_are_empty(credentials: &serde_json::Value) -> bool {
    credentials.as_object().is_some_and(|value| value.is_empty())
}

/// Public discovery is tied to an exact official URL. A custom relay never
/// inherits permission to skip authentication merely by its provider label.
pub(crate) fn is_public_catalog(platform: &str, base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else { return false; };
    if url.scheme() != "https" || url.port_or_known_default() != Some(443)
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    { return false; }
    let source = (url.host_str().unwrap_or_default(), url.path().trim_end_matches('/'));
    matches!((platform, source),
        ("openrouter", ("openrouter.ai", "/api/v1"))
        | ("poe", ("api.poe.com", "/v1"))
        | ("modelscope", ("api-inference.modelscope.cn", "/v1"))
        | ("deepgram", ("api.deepgram.com", "" | "/v1"))
    )
}

fn catalog_needs_credentials(platform: &str, base_url: &str) -> bool {
    !is_public_catalog(platform, base_url) && !matches!(
        fetchers::catalog_platform(platform, base_url),
        "mimo-token-plan-cn" | "mimo-token-plan-sgp" | "mimo-token-plan-ams"
        | "minimax-coding-plan" | "zhipu" | "ark-coding-plan" | "ark-agent-plan"
        | "stepfun-plan" | "dashscope-coding" | "glm-coding-plan" | "qianfan-coding-plan"
    )
}

/// Validate discovery independently of save/invocation. Public lists and
/// documentation suggestions do not need a model invocation credential.
fn validate_anonymous_request(req: &FetchModelsAnonymousRequest) -> Result<(), AppError> {
    if req.platform.trim().is_empty() {
        return Err(AppError::BadRequest("platform is required".into()));
    }
    validate_provider_base_url(&req.platform, &req.base_url)?;
    parse_auth_scheme(&req.auth_scheme)?;
    if credentials_are_empty(&req.credentials)
        && !catalog_needs_credentials(&req.platform, &req.base_url)
    {
        return Ok(());
    }
    validate_provider_auth(
        &req.platform,
        &req.auth_scheme,
        &req.credentials,
        req.bedrock_config.as_ref(),
    )?;
    Ok(())
}

fn parse_auth_scheme(raw: &str) -> Result<AuthScheme, AppError> {
    AuthScheme::parse(raw).map_err(|error| AppError::BadRequest(error.to_string()))
}

pub(crate) fn apply_catalog_auth(
    request: reqwest::RequestBuilder,
    auth: &AuthMaterial,
) -> Result<reqwest::RequestBuilder, AppError> {
    auth
        .validate_credentials()
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    auth
        .apply(request)
        .map_err(|error| AppError::BadRequest(error.to_string()))
}

/// Platforms that support URL auto-fix (OpenAI-compatible).
fn supports_url_fix(platform: &str) -> bool {
    !matches!(
        platform,
        "anthropic"
            | "claude"
            | "gemini"
            | "deepgram"
            | "bedrock"
            | "vertex-ai"
            | "mimo"
            | "mimo-token-plan-cn"
            | "mimo-token-plan-sgp"
            | "mimo-token-plan-ams"
            | "minimax"
            | "minimax-code"
            | "minimax-coding-plan"
            | "ark-coding-plan"
            | "ark-agent-plan"
            | "stepfun-plan"
            | "dashscope-coding"
            | "glm-coding-plan"
            | "qianfan-coding-plan"
    )
}

/// URL suffix probing can repair an incorrect API path. It cannot repair
/// credentials, DNS, TLS, firewall, proxy, rate-limit, or upstream 5xx
/// failures; probing every suffix in those cases only multiplies traffic and
/// delays the real error.
fn is_url_fix_candidate(error: &AppError) -> bool {
    match error {
        AppError::BadRequest(_) => true,
        AppError::BadGateway(message) => {
            message == "Remote models response was not valid JSON"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_connection::encrypt_credentials;
    use nomifun_db::{
        CreateProviderParams, NewProviderModel, NewProviderModelCapability,
        SqliteProviderRepository, init_database_memory,
    };
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_KEY: [u8; 32] = [0x42; 32];

    #[test]
    fn public_catalog_preview_is_exact_and_independent_of_save_credentials() {
        for (platform, base_url, scheme) in [
            ("openrouter", "https://openrouter.ai/api/v1", "bearer"),
            ("poe", "https://api.poe.com/v1", "bearer"),
            ("modelscope", "https://api-inference.modelscope.cn/v1", "bearer"),
            ("deepgram", "https://api.deepgram.com", "token"),
            ("deepgram", "https://api.deepgram.com/v1", "token"),
        ] {
            assert!(is_public_catalog(platform, base_url));
            assert!(validate_anonymous_request(&FetchModelsAnonymousRequest {
                platform: platform.into(), base_url: base_url.into(), auth_scheme: scheme.into(),
                credentials: serde_json::json!({}), bedrock_config: None, try_fix: false,
            }).is_ok());
            // Saving/invoking still validates the provider credential separately.
            assert!(validate_provider_auth(platform, scheme, &serde_json::json!({}), None).is_err());
        }
        for base in [
            "http://openrouter.ai/api/v1", "https://openrouter.ai.evil.example/api/v1",
            "https://openrouter.ai:444/api/v1", "https://openrouter.ai/api/v1?key=x",
            "https://user@openrouter.ai/api/v1", "https://openrouter.ai/custom",
        ] {
            assert!(!is_public_catalog("openrouter", base), "{base}");
            assert!(validate_anonymous_request(&FetchModelsAnonymousRequest {
                platform: "openrouter".into(), base_url: base.into(), auth_scheme: "bearer".into(),
                credentials: serde_json::json!({}), bedrock_config: None, try_fix: false,
            }).is_err());
        }
        assert!(!is_public_catalog("custom", "https://openrouter.ai/api/v1"));
    }

    #[tokio::test]
    async fn documentation_catalog_preview_does_not_require_an_invocation_key() {
        let (service, _) = setup().await;
        let response = service.fetch_models_anonymous(&FetchModelsAnonymousRequest {
            platform: "stepfun".into(), base_url: "https://api.stepfun.com/step_plan/v1".into(),
            auth_scheme: "bearer".into(), credentials: serde_json::json!({}),
            bedrock_config: None, try_fix: true,
        }).await.unwrap();
        assert_eq!(response.catalog_source, Some(ModelCatalogSource::OfficialDocumentation));
        assert!(response.models.iter().any(|model| model.id == "step-5-preview"));
        assert!(response.fixed_base_url.is_none());
    }

    async fn setup() -> (ModelFetchService, nomifun_db::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let svc = ModelFetchService::new(repo, TEST_KEY, reqwest::Client::new());
        (svc, db)
    }

    async fn create_provider(
        db: &nomifun_db::Database,
        platform: &str,
        base_url: &str,
        api_key: &str,
    ) -> String {
        let repo = SqliteProviderRepository::new(db.pool().clone());
        let encrypted =
            encrypt_credentials(&serde_json::json!({"api_keys":[api_key]}), &TEST_KEY).unwrap();
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            provider_params: "{}",
            ..Default::default()
        }];
        let initial_model = NewProviderModel {
            model: "test-model",
            enabled: true,
            capabilities: &capabilities,
            ..Default::default()
        };
        let (row, _) = repo
            .create(CreateProviderParams {
                provider_id: None,
                platform,
                name: "Test",
                base_url,
                auth_scheme: if platform == "deepgram" { "token" } else { "bearer" },
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
            }, &initial_model, &[])
            .await
            .unwrap();
        row.provider_id
    }

    #[test]
    fn supports_url_fix_openai_compatible() {
        assert!(supports_url_fix("openai"));
        assert!(supports_url_fix("new-api"));
        assert!(supports_url_fix("some-custom-provider"));
    }

    #[test]
    fn supports_url_fix_non_openai() {
        assert!(!supports_url_fix("anthropic"));
        assert!(!supports_url_fix("claude"));
        assert!(!supports_url_fix("gemini"));
        assert!(!supports_url_fix("deepgram"));
        assert!(!supports_url_fix("bedrock"));
        assert!(!supports_url_fix("vertex-ai"));
        assert!(!supports_url_fix("mimo"));
        assert!(!supports_url_fix("mimo-token-plan-cn"));
        assert!(!supports_url_fix("mimo-token-plan-sgp"));
        assert!(!supports_url_fix("mimo-token-plan-ams"));
        assert!(!supports_url_fix("minimax"));
        assert!(!supports_url_fix("minimax-code"));
        assert!(!supports_url_fix("minimax-coding-plan"));
        assert!(!supports_url_fix("ark-coding-plan"));
        assert!(!supports_url_fix("ark-agent-plan"));
        assert!(!supports_url_fix("stepfun-plan"));
        assert!(!supports_url_fix("dashscope-coding"));
        assert!(!supports_url_fix("glm-coding-plan"));
        assert!(!supports_url_fix("qianfan-coding-plan"));
    }

    #[test]
    fn url_fix_only_runs_for_path_shape_failures() {
        assert!(is_url_fix_candidate(&AppError::BadRequest(
            "Remote API rejected the model-list request (404 Not Found)".into()
        )));
        assert!(is_url_fix_candidate(&AppError::BadGateway(
            "Remote models response was not valid JSON".into()
        )));

        assert!(!is_url_fix_candidate(&AppError::Unauthorized(
            "bad key".into()
        )));
        assert!(!is_url_fix_candidate(&AppError::Timeout("slow".into())));
        assert!(!is_url_fix_candidate(&AppError::BadGateway(
            "Could not connect to the remote API".into()
        )));
        assert!(!is_url_fix_candidate(&AppError::BadGateway(
            "Remote API returned 503 Service Unavailable".into()
        )));
    }

    #[tokio::test]
    async fn load_config_nonexistent_provider_returns_not_found() {
        let (svc, _db) = setup().await;
        let err = svc.load_provider_config("no_such_id").await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn load_config_empty_api_key_returns_bad_request() {
        let (svc, db) = setup().await;
        let id = create_provider(&db, "openai", "https://api.openai.com", "   ").await;
        let err = svc.load_provider_config(&id).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn load_config_decrypts_api_key() {
        let (svc, db) = setup().await;
        let id = create_provider(&db, "openai", "https://api.openai.com", "sk-test-key").await;
        let config = svc.load_provider_config(&id).await.unwrap();
        assert_eq!(config.auth.primary_secret().unwrap(), "sk-test-key");
        assert_eq!(config.platform, "openai");
        assert_eq!(config.base_url, "https://api.openai.com");
        assert!(config.bedrock_config.is_none());
    }

    #[tokio::test]
    async fn fetch_models_vertex_ai_rejects_the_legacy_mixed_protocol_preset() {
        let (svc, db) = setup().await;
        let id = create_provider(&db, "vertex-ai", "https://unused", "fake-key").await;
        let req = FetchModelsRequest { try_fix: false };
        let err = svc.fetch_models(&id, &req).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn fetch_models_minimax_uses_saved_credentials_for_the_live_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .and(header("authorization", "Bearer fake-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data":[{"id":"MiniMax-future-account-model"}]
            }))).expect(1).mount(&server).await;
        let (svc, db) = setup().await;
        let id = create_provider(&db, "minimax", &format!("{}/v1", server.uri()), "fake-key").await;
        let req = FetchModelsRequest { try_fix: false };
        let resp = svc.fetch_models(&id, &req).await.unwrap();
        assert_eq!(resp.models.len(), 1);
        assert_eq!(resp.models[0].id, "MiniMax-future-account-model");
        assert_eq!(resp.catalog_source, Some(ModelCatalogSource::Remote));
    }

    #[tokio::test]
    async fn fetch_models_nonexistent_provider() {
        let (svc, _db) = setup().await;
        let req = FetchModelsRequest { try_fix: false };
        let missing = ProviderId::new().into_string();
        let err = svc.fetch_models(&missing, &req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fetch_models_rejects_noncanonical_provider_id_before_lookup() {
        let (svc, _db) = setup().await;
        let err = svc
            .fetch_models("not-a-provider-id", &FetchModelsRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn fetch_models_rejects_retired_builtin_platform() {
        let (svc, db) = setup().await;
        let id = create_provider(
            &db,
            "nomifun-free-model",
            "http://127.0.0.1:12345/v1",
            "internal-token",
        )
        .await;
        let err = svc
            .fetch_models(&id, &FetchModelsRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn anonymous_fetch_rejects_retired_builtin_platform() {
        let (svc, _db) = setup().await;
        let err = svc
            .fetch_models_anonymous(&FetchModelsAnonymousRequest {
                platform: "nomifun-free-model".into(),
                base_url: "https://example.com".into(),
                auth_scheme: "bearer".into(),
                credentials: serde_json::json!({"api_keys":["secret"]}),
                bedrock_config: None,
                try_fix: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn fetch_models_anonymous_minimax_reads_the_live_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .and(header("authorization", "Bearer fake-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data":[{"id":"MiniMax-future-preview-model"}]
            }))).expect(1).mount(&server).await;
        let (svc, _db) = setup().await;
        let req = FetchModelsAnonymousRequest {
            platform: "minimax".into(),
            base_url: format!("{}/v1", server.uri()),
            auth_scheme: "bearer".into(),
            credentials: serde_json::json!({"api_keys":["fake-key"]}),
            bedrock_config: None,
            try_fix: false,
        };
        let resp = svc.fetch_models_anonymous(&req).await.unwrap();
        assert_eq!(resp.models.len(), 1);
        assert_eq!(resp.models[0].id, "MiniMax-future-preview-model");
        assert_eq!(resp.catalog_source, Some(ModelCatalogSource::Remote));
        assert!(resp.fixed_base_url.is_none());
    }

    #[tokio::test]
    async fn deepgram_anonymous_fetch_returns_native_source_profiles() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Token first-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stt": [{"canonical_name": "opaque-one"}],
                "tts": [{"canonical_name": "opaque-two"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let db = init_database_memory().await.unwrap();
        let svc = ModelFetchService::new(
            Arc::new(SqliteProviderRepository::new(db.pool().clone())),
            TEST_KEY,
            reqwest::Client::builder().no_proxy().build().unwrap(),
        );
        let response = svc
            .fetch_models_anonymous(&FetchModelsAnonymousRequest {
                platform: "deepgram".into(),
                base_url: server.uri(),
                auth_scheme: "token".into(),
                // Model fetching deliberately uses the first configured key.
                credentials: serde_json::json!({"api_keys":["first-key","second-key"]}),
                bedrock_config: None,
                try_fix: true,
            })
            .await
            .unwrap();

        assert_eq!(
            response.models.iter().find(|model| model.id == "opaque-one").unwrap().tasks,
            vec![nomifun_api_types::ModelTask::SpeechRecognition]
        );
        assert_eq!(
            response.models.iter().find(|model| model.id == "opaque-two").unwrap().tasks,
            vec![nomifun_api_types::ModelTask::SpeechSynthesis]
        );
        assert!(response.fixed_base_url.is_none());
    }

    #[tokio::test]
    async fn fetch_models_anonymous_rejects_empty_api_key() {
        let (svc, _db) = setup().await;
        let req = FetchModelsAnonymousRequest {
            platform: "openai".into(),
            base_url: "https://api.openai.com".into(),
            auth_scheme: "bearer".into(),
            credentials: serde_json::json!({"api_keys":["   "]}),
            bedrock_config: None,
            try_fix: false,
        };
        let err = svc.fetch_models_anonymous(&req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn native_catalog_rejects_an_incompatible_auth_scheme_before_network() {
        let (svc, _db) = setup().await;
        let error = svc
            .fetch_models_anonymous(&FetchModelsAnonymousRequest {
                platform: "deepgram".into(),
                base_url: "https://api.deepgram.com".into(),
                auth_scheme: "bearer".into(),
                credentials: serde_json::json!({"api_keys":["secret"]}),
                bedrock_config: None,
                try_fix: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(message) if message.contains("expected")));
    }

    #[tokio::test]
    async fn fetch_models_anonymous_rejects_empty_platform() {
        let (svc, _db) = setup().await;
        let req = FetchModelsAnonymousRequest {
            platform: "".into(),
            base_url: "https://api.openai.com".into(),
            auth_scheme: "bearer".into(),
            credentials: serde_json::json!({"api_keys":["sk-test"]}),
            bedrock_config: None,
            try_fix: false,
        };
        let err = svc.fetch_models_anonymous(&req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn fetch_models_anonymous_bedrock_profile_accepts_empty_credentials() {
        let (_svc, _db) = setup().await;
        let req = FetchModelsAnonymousRequest {
            platform: "bedrock".into(),
            base_url: "".into(),
            auth_scheme: "bedrock".into(),
            credentials: serde_json::json!({}),
            bedrock_config: Some(BedrockConfig {
                auth_method: nomifun_api_types::BedrockAuthMethod::Profile,
                region: "us-east-1".into(),
                profile: Some("work".into()),
            }),
            try_fix: false,
        };
        assert!(validate_anonymous_request(&req).is_ok());
    }

    #[test]
    fn bedrock_non_anthropic_catalog_rows_do_not_gain_generic_chat() {
        let mut models = vec![
            ModelInfo {
                id: "amazon.nova-pro-v1:0".into(),
                name: Some("Nova Pro".into()),
                tasks: Vec::new(),
                tasks_source: None,
                traits: Vec::new(),
                context_limit: None,
                output_limit: None,
                token_limit_sources: None,
            },
            ModelInfo {
                id: "us.anthropic.claude-sonnet-4-v1:0".into(),
                name: Some("Claude Sonnet".into()),
                tasks: vec![nomifun_api_types::ModelTask::Chat],
                tasks_source: Some(ModelTaskSource::ProviderDeclared),
                traits: Vec::new(),
                context_limit: None,
                output_limit: None,
                token_limit_sources: None,
            },
        ];
        prepare_catalog_models("bedrock", &mut models);
        assert!(models[0].tasks.is_empty());
        assert_eq!(models[1].tasks, vec![nomifun_api_types::ModelTask::Chat]);
    }

    #[test]
    fn task_trait_enrichment_preserves_a_provider_declared_context_window() {
        // Inline suggestions must not overwrite the only automatic source for
        // the capability's context limit.
        let mut models = vec![ModelInfo {
            id: "gemini-3.1-pro".into(),
            name: None,
            tasks: Vec::new(),
            tasks_source: None,
            traits: Vec::new(),
            context_limit: Some(1_048_576),
            output_limit: Some(65_536),
            token_limit_sources: None,
        }];
        prepare_catalog_models("gemini", &mut models);
        assert_eq!(models[0].context_limit, Some(1_048_576));
        assert_eq!(models[0].output_limit, Some(65_536));
        assert!(!models[0].tasks.is_empty());
    }

    #[test]
    fn live_ids_do_not_confirm_tasks_inferred_from_names_or_missing_metadata() {
        let mut models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"opaque-future-id"},
            {"id":"future-asr-model"},
            {"id":"my-whisper-model"},
            {"id":"gpt-image-1"}
        ])).unwrap();
        prepare_catalog_models("openai", &mut models);
        assert_eq!(models[0].tasks, vec![nomifun_api_types::ModelTask::Chat]);
        assert_eq!(models[1].tasks, vec![nomifun_api_types::ModelTask::SpeechRecognition]);
        for model in &models[..3] {
            assert_eq!(model.tasks_source, Some(ModelTaskSource::Inferred), "{}", model.id);
        }
        assert_eq!(models[3].tasks_source, Some(ModelTaskSource::OfficialDocumentation));
    }

    #[test]
    fn declared_tasks_keep_their_evidence_and_are_not_replaced_by_name_guesses() {
        let mut models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"opaque-speech-id", "tasks":["speech_synthesis"], "tasks_source":"provider_declared"},
            {"id":"opaque-old-id", "tasks":["chat"]}
        ])).unwrap();
        prepare_catalog_models("openai", &mut models);
        assert_eq!(models[0].tasks, vec![nomifun_api_types::ModelTask::SpeechSynthesis]);
        assert_eq!(models[0].tasks_source, Some(ModelTaskSource::ProviderDeclared));
        assert_eq!(models[1].tasks_source, None, "older task metadata remains unconfirmed");
    }

    #[test]
    fn documentation_catalog_ids_are_not_all_assumed_to_have_confirmed_tasks() {
        let models: Vec<ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"mimo-v2.5-asr"}, {"id":"mimo-v2.6-future"}
        ])).unwrap();
        let response = fetch_models_response("mimo", models, ModelCatalogSource::OfficialDocumentation, None);
        assert_eq!(response.models[0].tasks_source, Some(ModelTaskSource::OfficialDocumentation));
        assert_eq!(response.models[1].tasks_source, Some(ModelTaskSource::Inferred));
    }
}
