mod fetchers;
mod probe;
mod url_fixer;

use std::sync::Arc;

use nomifun_api_types::{
    BedrockConfig, FetchModelsAnonymousRequest, FetchModelsRequest, FetchModelsResponse,
    ModelInfo, infer_catalog_tasks_and_traits,
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
        if crate::managed_model::is_managed_provider_platform(req.platform.trim()) {
            return Err(AppError::Forbidden(
                "Reserved managed model platforms cannot be used for anonymous model fetching"
                    .into(),
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
        let http_client = self.http_client();
        match fetchers::fetch_for_platform(&http_client, &config).await {
            Ok(models) => Ok(fetch_models_response(&config.platform, models, None)),
            Err(err)
                if try_fix
                    && supports_url_fix(&config.platform)
                    && is_url_fix_candidate(&err) =>
            {
                url_fixer::try_fix_url(&http_client, &config)
                    .await
                    .map(|mut response| {
                        enrich_model_suggestions(&config.platform, &mut response.models);
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
        if crate::managed_model::is_managed_provider_platform(&row.platform) {
            return Err(AppError::Forbidden(
                "Managed model catalogs are available through the dedicated model-service API"
                    .into(),
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

fn enrich_model_suggestions(platform: &str, models: &mut [ModelInfo]) {
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
        }
        if model.traits.is_empty() {
            model.traits = traits;
        }
    }
}

fn fetch_models_response(
    platform: &str,
    mut models: Vec<ModelInfo>,
    fixed_base_url: Option<String>,
) -> FetchModelsResponse {
    enrich_model_suggestions(platform, &mut models);
    FetchModelsResponse { models, fixed_base_url }
}

impl FetchConfig {
    fn primary_secret(&self) -> Result<String, AppError> {
        self.auth
            .primary_secret()
            .map_err(|error| AppError::BadRequest(error.to_string()))
    }
}

/// Validate the full anonymous default-connection proposal before network I/O.
fn validate_anonymous_request(req: &FetchModelsAnonymousRequest) -> Result<(), AppError> {
    if req.platform.trim().is_empty() {
        return Err(AppError::BadRequest("platform is required".into()));
    }
    validate_provider_base_url(&req.platform, &req.base_url)?;
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
    async fn fetch_models_minimax_returns_hardcoded() {
        let (svc, db) = setup().await;
        let id = create_provider(&db, "minimax", "https://unused", "fake-key").await;
        let req = FetchModelsRequest { try_fix: false };
        let resp = svc.fetch_models(&id, &req).await.unwrap();
        assert!(resp.models.iter().any(|model| model.id == "MiniMax-M3"));
        assert!(!resp.models.iter().any(|model| model.id == "MiniMax-Text-01"));
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
            .fetch_models("nomifun-free-model", &FetchModelsRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn fetch_models_rejects_persisted_managed_platform_alias() {
        let (svc, db) = setup().await;
        let id = create_provider(
            &db,
            crate::managed_model::FREE_MODEL_PLATFORM,
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
    async fn anonymous_fetch_rejects_reserved_managed_platform() {
        let (svc, _db) = setup().await;
        let err = svc
            .fetch_models_anonymous(&FetchModelsAnonymousRequest {
                platform: crate::managed_model::FREE_MODEL_PLATFORM.into(),
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
    async fn fetch_models_anonymous_minimax_returns_hardcoded() {
        let (svc, _db) = setup().await;
        let req = FetchModelsAnonymousRequest {
            platform: "minimax".into(),
            base_url: "https://unused".into(),
            auth_scheme: "bearer".into(),
            credentials: serde_json::json!({"api_keys":["fake-key"]}),
            bedrock_config: None,
            try_fix: false,
        };
        let resp = svc.fetch_models_anonymous(&req).await.unwrap();
        assert!(resp.models.iter().any(|model| model.id == "MiniMax-M3"));
        assert!(!resp.models.iter().any(|model| model.id == "MiniMax-Text-01"));
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
                traits: Vec::new(),
                context_limit: None,
            },
            ModelInfo {
                id: "us.anthropic.claude-sonnet-4-v1:0".into(),
                name: Some("Claude Sonnet".into()),
                tasks: vec![nomifun_api_types::ModelTask::Chat],
                traits: Vec::new(),
                context_limit: None,
            },
        ];
        enrich_model_suggestions("bedrock", &mut models);
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
            traits: Vec::new(),
            context_limit: Some(1_048_576),
        }];
        enrich_model_suggestions("gemini", &mut models);
        assert_eq!(models[0].context_limit, Some(1_048_576));
        assert!(!models[0].tasks.is_empty());
    }
}
