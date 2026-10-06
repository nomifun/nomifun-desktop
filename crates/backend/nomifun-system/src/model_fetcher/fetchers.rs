use std::time::Duration;

use axum::http::StatusCode;
use nomifun_api_types::{ModelCatalogSource, ModelContextLimitKind, ModelInfo, ModelTask, ModelTaskSource, ModelTokenLimitSources};
use nomifun_common::AppError;
use nomifun_model_invoke::{AuthMaterial, AuthScheme};
use serde::Deserialize;
use tracing::warn;

use super::{FetchConfig, apply_catalog_auth};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct FetchedCatalog {
    pub models: Vec<ModelInfo>,
    pub source: ModelCatalogSource,
}

impl FetchedCatalog {
    fn remote(models: Vec<ModelInfo>) -> Self {
        Self { models, source: ModelCatalogSource::Remote }
    }

    fn documentation(models: Vec<ModelInfo>) -> Self {
        Self { models, source: ModelCatalogSource::OfficialDocumentation }
    }
}

/// Dispatch to the appropriate platform-specific fetcher.
pub(crate) async fn fetch_for_platform(
    client: &reqwest::Client,
    config: &FetchConfig,
) -> Result<FetchedCatalog, AppError> {
    if super::credentials_are_empty(&config.auth.credentials)
        && super::is_public_catalog(&config.platform, &config.base_url)
    {
        let models = if config.platform == "deepgram" {
            fetch_deepgram_catalog(client, &config.base_url, "").await?.models
        } else {
            fetch_openai_public_catalog(client, &config.base_url).await?
        };
        return Ok(FetchedCatalog::remote(models));
    }
    match catalog_platform(&config.platform, &config.base_url) {
        "anthropic" | "claude" => {
            require_auth_scheme(config, AuthScheme::HeaderKey("x-api-key".into()))?;
            let secret = config.primary_secret()?;
            fetch_anthropic(client, &config.base_url, &secret).await.map(FetchedCatalog::remote)
        }
        "gemini" => {
            require_auth_scheme(config, AuthScheme::HeaderKey("x-goog-api-key".into()))?;
            let secret = config.primary_secret()?;
            fetch_gemini(client, &config.base_url, &secret).await.map(FetchedCatalog::remote)
        }
        "deepgram" => {
            require_auth_scheme(config, AuthScheme::TokenHeader)?;
            let secret = config.primary_secret()?;
            fetch_deepgram_catalog(client, &config.base_url, &secret)
                .await
                .map(|catalog| FetchedCatalog::remote(catalog.models))
        }
        "xai" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_xai(client, &config.base_url, &secret).await.map(FetchedCatalog::remote)
        }
        // DeepSeek's live `/models` catalog is authoritative. Do not substitute
        // retired aliases when discovery is unavailable.
        "deepseek" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_openai_compatible(client, &config.base_url, &secret).await.map(FetchedCatalog::remote)
        }
        "bedrock" => {
            require_auth_scheme(config, AuthScheme::Bedrock)?;
            fetch_bedrock(config).await.map(FetchedCatalog::remote)
        }
        "gemini-vertex-ai" | "vertex-ai" => Err(AppError::BadRequest(
            "The legacy Vertex preset mixed Gemini model IDs with the Anthropic publisher protocol; create a provider-specific Vertex connection instead"
                .into(),
        )),
        "new-api" => {
            let secret = config.primary_secret()?;
            fetch_new_api(client, &config.base_url, &secret, &config.auth.scheme).await.map(FetchedCatalog::remote)
        }
        "mimo" => fetch_openai_compatible_with_auth(client, &config.base_url, &config.auth)
            .await
            .map(FetchedCatalog::remote),
        "mimo-token-plan-cn" | "mimo-token-plan-sgp" | "mimo-token-plan-ams" => {
            Ok(FetchedCatalog::documentation(mimo_token_plan_models()))
        }
        "stepfun" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_stepfun(client, &config.base_url, &secret).await
        }
        "minimax" | "minimax-code" => fetch_openai_compatible_with_auth(client, &config.base_url, &config.auth)
            .await
            .map(FetchedCatalog::remote),
        "minimax-coding-plan" => Ok(FetchedCatalog::documentation(minimax_code_models())),
        "dashscope" => fetch_dashscope(client, &config.base_url, &config.auth)
            .await
            .map(FetchedCatalog::remote),
        // Zhipu OpenAPI does not expose an OpenAI-compatible `GET /models`.
        "zhipu" => Ok(FetchedCatalog::documentation(zhipu_models())),
        "ark-coding-plan" => Ok(FetchedCatalog::documentation(ark_coding_plan_models())),
        "ark-agent-plan" if is_official_ark_agent_plan_base_url(&config.base_url) => {
            Ok(FetchedCatalog::documentation(ark_agent_plan_models()))
        }
        "stepfun-plan" => Ok(FetchedCatalog::documentation(stepfun_plan_models())),
        "dashscope-coding" => Ok(FetchedCatalog::documentation(fallback_models(DASHSCOPE_MODELS))),
        "glm-coding-plan" => Ok(FetchedCatalog::documentation(glm_coding_plan_models())),
        "qianfan-coding-plan" => Ok(FetchedCatalog::documentation(qianfan_coding_plan_models())),
        _ => fetch_openai_compatible_with_auth(client, &config.base_url, &config.auth).await.map(FetchedCatalog::remote),
    }
}

/// Discovery follows the configured commercial channel. Older providers can
/// still carry the general StepFun family with a Step Plan root; requesting a
/// standard catalog or probing a different billing root in that case would
/// use the wrong channel. Only exact official roots get this interpretation;
/// custom gateways retain the discovery behavior their platform declares.
pub(crate) fn catalog_platform<'a>(platform: &'a str, base_url: &str) -> &'a str {
    if platform == "stepfun" && is_official_stepfun_plan_base_url(base_url) {
        "stepfun-plan"
    } else if platform == "ark" && is_official_ark_agent_plan_base_url(base_url) {
        "ark-agent-plan"
    } else if platform == "ark" && exact_official_catalog_root(base_url, &["ark.cn-beijing.volces.com"], &["/api/coding/v3"]) {
        "ark-coding-plan"
    } else if platform == "dashscope" && exact_official_catalog_root(base_url, &["coding.dashscope.aliyuncs.com"], &["/v1"]) {
        "dashscope-coding"
    } else if platform == "zhipu" && exact_official_catalog_root(base_url, &["open.bigmodel.cn"], &["/api/coding/paas/v4"]) {
        "glm-coding-plan"
    } else if platform == "qianfan" && exact_official_catalog_root(base_url, &["qianfan.baidubce.com"], &["/v2/coding"]) {
        "qianfan-coding-plan"
    } else if platform == "mimo" && exact_official_catalog_root(base_url, &["token-plan-cn.xiaomimimo.com"], &["/v1"]) {
        "mimo-token-plan-cn"
    } else if platform == "mimo" && exact_official_catalog_root(base_url, &["token-plan-sgp.xiaomimimo.com"], &["/v1"]) {
        "mimo-token-plan-sgp"
    } else if platform == "mimo" && exact_official_catalog_root(base_url, &["token-plan-ams.xiaomimimo.com"], &["/v1"]) {
        "mimo-token-plan-ams"
    } else {
        platform
    }
}

fn exact_official_catalog_root(base_url: &str, hosts: &[&str], paths: &[&str]) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| hosts.contains(&host))
        && url.port_or_known_default() == Some(443)
        && paths.contains(&url.path().trim_end_matches('/'))
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn is_official_stepfun_plan_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && matches!(url.host_str(), Some("api.stepfun.com" | "api.stepfun.ai"))
        && url.port_or_known_default() == Some(443)
        && matches!(url.path().trim_end_matches('/'), "/step_plan" | "/step_plan/v1")
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}


fn require_auth_scheme(config: &FetchConfig, expected: AuthScheme) -> Result<(), AppError> {
    let compatible = match (&config.auth.scheme, &expected) {
        (AuthScheme::HeaderKey(actual), AuthScheme::HeaderKey(expected)) => {
            actual.eq_ignore_ascii_case(expected)
        }
        (actual, expected) => actual == expected,
    };
    if compatible {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Provider '{}' model discovery does not support auth scheme {:?}; expected {:?}",
            config.platform, config.auth.scheme, expected
        )))
    }
}

// ---------------------------------------------------------------------------
// Deepgram native model catalog
// ---------------------------------------------------------------------------

/// Deepgram returns separate STT and TTS arrays rather than an OpenAI-style
/// `{data: [...]}` list. Keep those source sections as exact task metadata: a
/// canonical model name is not a reliable way to infer whether a future model
/// belongs to speech recognition or synthesis.
pub(crate) struct DeepgramCatalog {
    pub models: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct DeepgramModelsResponse {
    #[serde(default)]
    stt: Vec<DeepgramModel>,
    #[serde(default)]
    tts: Vec<DeepgramModel>,
}

#[derive(Deserialize)]
struct DeepgramModel {
    #[serde(default)]
    canonical_name: String,
    #[serde(default)]
    name: Option<String>,
}

pub(crate) async fn fetch_deepgram_catalog(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<DeepgramCatalog, AppError> {
    let base = ensure_v1_path(base_url);
    let url = nomifun_model_invoke::join_endpoint(&base, "/models");
    let request = client.get(&url);
    let request = if api_key.is_empty() { request } else {
        request.header("Authorization", format!("Token {api_key}"))
    };
    let resp = request
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| remote_error(&error))?;
    check_response_status(&resp)?;

    let body: DeepgramModelsResponse = resp
        .json()
        .await
        .map_err(|_| AppError::BadGateway("Deepgram models response was not valid JSON".into()))?;

    let mut models: Vec<ModelInfo> = Vec::new();
    for (items, task) in [
        (body.stt, ModelTask::SpeechRecognition),
        (body.tts, ModelTask::SpeechSynthesis),
    ] {
        for item in items {
            let id = item.canonical_name.trim();
            if id.is_empty() {
                continue;
            }
            if let Some(model) = models.iter_mut().find(|model| model.id == id) {
                if !model.tasks.contains(&task) {
                    model.tasks.push(task);
                }
            } else {
                models.push(ModelInfo {
                    id: id.to_owned(),
                    name: item.name.filter(|name| !name.trim().is_empty()),
                    tasks: vec![task],
                    tasks_source: Some(ModelTaskSource::ProviderDeclared),
                    traits: Vec::new(),
                    context_limit: None,
                    output_limit: None,
                    token_limit_sources: None,
                });
            }
        }
    }

    Ok(DeepgramCatalog { models })
}

// ---------------------------------------------------------------------------
// xAI modality-specific catalogs
// ---------------------------------------------------------------------------

async fn fetch_xai(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let base = ensure_v1_path(base_url);
    let mut models: Vec<ModelInfo> = Vec::new();
    for (path, task) in [
        ("language-models", ModelTask::Chat),
        ("image-generation-models", ModelTask::ImageGeneration),
        ("video-generation-models", ModelTask::VideoGeneration),
    ] {
        let url = format!("{}/{path}", base.trim_end_matches('/'));
        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| remote_error(&error))?;
        check_response_status(&resp)?;
        let body: XaiModelsResponse = resp
            .json()
            .await
            .map_err(|_| AppError::BadGateway(format!("xAI {path} response was not valid JSON")))?;
        for item in body.models {
            let mut candidate = item.into_info();
            candidate.tasks_source = Some(ModelTaskSource::ProviderDeclared);
            if let Some(model) = models.iter_mut().find(|known| known.id == candidate.id) {
                if !model.tasks.contains(&task) {
                    model.tasks.push(task);
                }
                merge_declared_limits(model, &candidate);
            } else {
                candidate.tasks.push(task);
                models.push(candidate);
            }
        }
    }

    // Current xAI STT/TTS APIs are services rather than model-ID endpoints.
    // The model picker still requires a third-level value, so expose explicit
    // service profiles instead of inventing an upstream model field.
    models.push(ModelInfo {
        id: "xai-tts".into(),
        name: Some("xAI Text-to-Speech service".into()),
        tasks: vec![ModelTask::SpeechSynthesis],
        tasks_source: Some(ModelTaskSource::OfficialDocumentation),
        traits: Vec::new(),
        context_limit: None,
        output_limit: None,
        token_limit_sources: None,
    });
    models.push(ModelInfo {
        id: "xai-stt".into(),
        name: Some("xAI Speech-to-Text service".into()),
        tasks: vec![ModelTask::SpeechRecognition],
        tasks_source: Some(ModelTaskSource::OfficialDocumentation),
        traits: Vec::new(),
        context_limit: None,
        output_limit: None,
        token_limit_sources: None,
    });
    Ok(models)
}

#[derive(Deserialize)]
struct XaiModelsResponse {
    models: Vec<OpenAiModel>,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible (default)
// ---------------------------------------------------------------------------

/// Response shape for OpenAI `/models` endpoint.
#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default, deserialize_with = "deserialize_model_display_name")]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_model_display_name")]
    title: Option<String>,
    /// OpenRouter and several China-based OpenAI-compatible gateways declare the
    /// model's input window here. Plain OpenAI does not send it, so it stays
    /// `None` rather than being guessed from the model id.
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    context_length: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    context_size: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    context_window: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_output_tokens: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_output_length: Option<i64>,
    #[serde(default)]
    top_provider: Option<OpenAiTopProvider>,
}

#[derive(Deserialize)]
struct OpenAiTopProvider {
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    context_length: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_completion_tokens: Option<i64>,
}

impl OpenAiModel {
    fn into_info(self) -> ModelInfo {
        let context = [
            (self.context_length, "context_length"),
            (self.context_window, "context_window"),
            (self.context_size, "context_size"),
            (self.top_provider.as_ref().and_then(|provider| provider.context_length), "top_provider.context_length"),
        ].into_iter().find(|(value, _)| value.is_some());
        let output = [
            (self.max_output_tokens, "max_output_tokens"),
            (self.max_output_length, "max_output_length"),
            (self.top_provider.as_ref().and_then(|provider| provider.max_completion_tokens), "top_provider.max_completion_tokens"),
        ].into_iter().find(|(value, _)| value.is_some());
        let context_limit = context.and_then(|(limit, _)| limit);
        let output_limit = output.and_then(|(limit, _)| limit);
        let sources = token_limit_sources(
            context_limit,
            context.map(|(_, field)| field).unwrap_or_default(),
            output_limit,
            output.map(|(_, field)| field).unwrap_or_default(),
        );
        ModelInfo { id: self.id, name: self.name.or(self.title), tasks: Vec::new(), tasks_source: None, traits: Vec::new(),
            context_limit, output_limit, token_limit_sources: sources }
    }
}

/// Fetch models from an OpenAI-compatible `/models` endpoint.
pub(super) async fn fetch_openai_compatible(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let auth = AuthMaterial {
        scheme: AuthScheme::Bearer,
        credentials: serde_json::json!({"api_keys":[api_key]}),
    };
    fetch_openai_compatible_with_auth(client, base_url, &auth).await
}

pub(super) async fn fetch_openai_compatible_with_auth(
    client: &reqwest::Client,
    base_url: &str,
    auth: &AuthMaterial,
) -> Result<Vec<ModelInfo>, AppError> {
    fetch_openai_catalog(client, base_url, Some(auth)).await
}

/// Called only after exact public-catalog source validation. Discovery does
/// not add an unauthenticated scheme to saved provider invocation contracts.
async fn fetch_openai_public_catalog(client: &reqwest::Client, base_url: &str) -> Result<Vec<ModelInfo>, AppError> {
    fetch_openai_catalog(client, base_url, None).await
}

async fn fetch_openai_catalog(client: &reqwest::Client, base_url: &str, auth: Option<&AuthMaterial>) -> Result<Vec<ModelInfo>, AppError> {
    let url = nomifun_model_invoke::join_endpoint(base_url, "/models");
    let request = client.get(&url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");
    let request = match auth {
        Some(auth) => apply_catalog_auth(request, auth)?,
        None => request,
    };
    let resp = request
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| remote_error(&e))?;

    check_response_status(&resp)?;

    let body: OpenAiModelsResponse = resp
        .json()
        .await
        .map_err(|_| AppError::BadGateway("Remote models response was not valid JSON".into()))?;

    Ok(body
        .data
        .into_iter()
        .map(OpenAiModel::into_info)
        .collect())
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

const MAX_MODEL_CATALOG_PAGES: usize = 100;

/// Response shape for Anthropic `/v1/models`.
#[derive(Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_input_tokens: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_tokens: Option<i64>,
}

async fn fetch_anthropic(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let url = nomifun_model_invoke::join_endpoint(base_url, "/v1/models");
    let mut models = Vec::new();
    let mut after_id: Option<String> = None;
    let mut seen_cursors = std::collections::HashSet::new();
    let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
    for _ in 0..MAX_MODEL_CATALOG_PAGES {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Timeout(
                "Anthropic model catalog request timed out while loading all pages".into(),
            ));
        }
        let mut request = client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .query(&[("limit", "1000")]);
        if let Some(cursor) = after_id.as_deref() {
            // Cursors stay opaque query values on the same catalog endpoint.
            request = request.query(&[("after_id", cursor)]);
        }
        let resp = request
            .timeout(remaining)
            .send()
            .await
            .map_err(|error| {
                warn_remote_request_failure_without_fallback("anthropic", &error);
                remote_error(&error)
            })?;

        // A failed later page must not turn a partial catalog into success.
        check_response_status(&resp)?;
        let body: AnthropicModelsResponse = resp.json().await.map_err(|_| {
            AppError::BadGateway("Anthropic models response was not valid JSON".into())
        })?;
        // Native max_tokens is the model ceiling, not a request default.
        models.extend(body.data.into_iter().map(|m| ModelInfo {
            id: m.id,
            name: m.display_name,
            tasks: Vec::new(),
            tasks_source: None,
            traits: Vec::new(),
            context_limit: m.max_input_tokens,
            output_limit: m.max_tokens,
            token_limit_sources: token_limit_sources(m.max_input_tokens, "max_input_tokens", m.max_tokens, "max_tokens"),
        }));
        if !body.has_more {
            return Ok(models);
        }
        let Some(cursor) = body.last_id.filter(|id| !id.trim().is_empty()) else {
            return Err(AppError::BadGateway(
                "Anthropic models response was missing its next-page cursor".into(),
            ));
        };
        if !seen_cursors.insert(cursor.clone()) {
            return Err(AppError::BadGateway(
                "Anthropic model catalog repeated a pagination cursor".into(),
            ));
        }
        after_id = Some(cursor);
    }
    Err(AppError::BadGateway(
        "Anthropic model catalog exceeded the pagination limit".into(),
    ))
}

// ---------------------------------------------------------------------------
// Gemini
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GeminiModelsResponse {
    models: Vec<GeminiModel>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(default, rename = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
    /// `/v1beta/models` already reports the input window Nomi otherwise makes
    /// the user retype. Optional because non-generative entries (embedding,
    /// retrieval) omit it.
    #[serde(
        default,
        rename = "inputTokenLimit",
        deserialize_with = "deserialize_declared_token_limit"
    )]
    input_token_limit: Option<i64>,
    #[serde(default, rename = "outputTokenLimit", deserialize_with = "deserialize_declared_token_limit")]
    output_token_limit: Option<i64>,
}

async fn fetch_gemini(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let url = nomifun_model_invoke::join_endpoint(base_url, "/v1beta/models");
    let mut models = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_cursors = std::collections::HashSet::new();
    let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
    for _ in 0..MAX_MODEL_CATALOG_PAGES {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Timeout(
                "Gemini model catalog request timed out while loading all pages".into(),
            ));
        }
        let mut request = client
            .get(&url)
            .header("x-goog-api-key", api_key)
            .query(&[("pageSize", "1000")]);
        if let Some(cursor) = page_token.as_deref() {
            request = request.query(&[("pageToken", cursor)]);
        }
        let resp = request
            .timeout(remaining)
            .send()
            .await
            .map_err(|error| {
                warn_remote_request_failure_without_fallback("gemini", &error);
                remote_error(&error)
            })?;
        check_response_status(&resp)?;
        let body: GeminiModelsResponse = resp.json().await.map_err(|_| {
            AppError::BadGateway("Gemini models response was not valid JSON".into())
        })?;
        models.extend(body.models.into_iter().map(|m| {
            let id = m.name.strip_prefix("models/").unwrap_or(&m.name).to_owned();
            let tasks = gemini_declared_tasks(&m.supported_generation_methods);
            let tasks_source = (!tasks.is_empty()).then_some(ModelTaskSource::ProviderDeclared);
            ModelInfo {
                id,
                name: None,
                tasks,
                tasks_source,
                traits: Vec::new(),
                context_limit: m.input_token_limit,
                output_limit: m.output_token_limit,
                token_limit_sources: token_limit_sources(m.input_token_limit, "inputTokenLimit", m.output_token_limit, "outputTokenLimit"),
            }
        }));
        let Some(cursor) = body.next_page_token.filter(|token| !token.is_empty()) else {
            return Ok(models);
        };
        if !seen_cursors.insert(cursor.clone()) {
            return Err(AppError::BadGateway(
                "Gemini model catalog repeated a pagination cursor".into(),
            ));
        }
        page_token = Some(cursor);
    }
    Err(AppError::BadGateway(
        "Gemini model catalog exceeded the pagination limit".into(),
    ))
}

fn gemini_declared_tasks(methods: &[String]) -> Vec<ModelTask> {
    let mut tasks = Vec::new();
    for method in methods {
        // `generateContent` can produce text, image or speech output. Its
        // presence alone does not prove which task should initialize a model.
        let task = match method.as_str() {
            "embedContent" | "batchEmbedContents" => ModelTask::Embedding,
            "bidiGenerateContent" => ModelTask::RealtimeConversation,
            _ => continue,
        };
        if !tasks.contains(&task) {
            tasks.push(task);
        }
    }
    tasks
}

// ---------------------------------------------------------------------------
// Bedrock (AWS SDK)
// ---------------------------------------------------------------------------

async fn fetch_bedrock(config: &FetchConfig) -> Result<Vec<ModelInfo>, AppError> {
    let bedrock_cfg = config
        .bedrock_config
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("Bedrock requires bedrockConfig".into()))?;
    let sdk_config = crate::bedrock_probe::service::build_bedrock_aws_config(
        bedrock_cfg,
        &config.auth.credentials,
    )
    .await?;
    let client = aws_sdk_bedrock::Client::new(&sdk_config);
    let foundation = client
        .list_foundation_models()
        .send()
        .await
        .map_err(|error| AppError::BadGateway(format!("Bedrock API error: {error}")))?;
    let mut models = foundation
        .model_summaries()
        .iter()
        .map(|model| {
            let tasks = bedrock_tasks(
                model.model_id(),
                Some(model.model_arn()),
                model.provider_name(),
            );
            ModelInfo {
                id: model.model_id().to_owned(),
                name: model.model_name().map(str::to_owned),
                // Provider family/model identifiers select the implemented
                // adapter, but the AWS catalog has not declared a task.
                tasks_source: (!tasks.is_empty()).then_some(ModelTaskSource::Inferred),
                tasks,
                traits: Vec::new(),
                context_limit: None,
                output_limit: None,
                token_limit_sources: None,
            }
        })
        .collect::<Vec<_>>();

    let mut pages = client
        .list_inference_profiles()
        .into_paginator()
        .send();
    while let Some(page) = pages
        .try_next()
        .await
        .map_err(|error| AppError::BadGateway(format!("Bedrock API error: {error}")))?
    {
        for profile in page.inference_profile_summaries() {
            let tasks = if is_anthropic_bedrock_identifier(profile.inference_profile_id())
                || profile.models().iter().any(|model| {
                    model
                        .model_arn()
                        .is_some_and(is_anthropic_bedrock_identifier)
                }) {
                vec![ModelTask::Chat]
            } else {
                Vec::new()
            };
            upsert_bedrock_model(
                &mut models,
                ModelInfo {
                    id: profile.inference_profile_id().to_owned(),
                    name: Some(profile.inference_profile_name().to_owned()),
                    tasks_source: (!tasks.is_empty()).then_some(ModelTaskSource::Inferred),
                    tasks,
                    traits: Vec::new(),
                    context_limit: None,
                    output_limit: None,
                    token_limit_sources: None,
                },
            );
        }
    }

    Ok(models)
}

fn bedrock_tasks(model_id: &str, model_arn: Option<&str>, provider_name: Option<&str>) -> Vec<ModelTask> {
    if provider_name.is_some_and(|provider| provider.eq_ignore_ascii_case("anthropic"))
        || is_anthropic_bedrock_identifier(model_id)
        || model_arn.is_some_and(is_anthropic_bedrock_identifier)
    {
        vec![ModelTask::Chat]
    } else {
        Vec::new()
    }
}

fn is_anthropic_bedrock_identifier(identifier: &str) -> bool {
    let model_id = identifier
        .rsplit_once("foundation-model/")
        .map(|(_, model)| model)
        .unwrap_or(identifier);
    let model_id = ["us.", "eu.", "apac.", "global."]
        .iter()
        .find_map(|prefix| model_id.strip_prefix(prefix))
        .unwrap_or(model_id);
    model_id.starts_with("anthropic.claude")
}

fn upsert_bedrock_model(models: &mut Vec<ModelInfo>, candidate: ModelInfo) {
    if let Some(existing) = models.iter_mut().find(|model| model.id == candidate.id) {
        merge_declared_limits(existing, &candidate);
        if existing.tasks.is_empty() && !candidate.tasks.is_empty() {
            existing.tasks = candidate.tasks;
            existing.tasks_source = candidate.tasks_source;
        }
        if existing.name.is_none() {
            existing.name = candidate.name;
        }
    } else {
        models.push(candidate);
    }
}

// ---------------------------------------------------------------------------
// Maintained catalogs for products without a reliable account catalog
// ---------------------------------------------------------------------------

fn mimo_token_plan_models() -> Vec<ModelInfo> {
    // The regular API has a live catalog. Regional subscription gateways have
    // a separately documented model set; never fetch it with a plan key from
    // the pay-as-you-go host. Ultraspeed is not included in Token Plan.
    fallback_models(&[
        "mimo-v2.6-pro",
        "mimo-v2.6-flash",
        "mimo-v2.5-pro",
        "mimo-v2.5",
        "mimo-v2.5-asr",
        "mimo-v2.5-tts",
        "mimo-v2.5-tts-voicedesign",
        "mimo-v2.5-tts-voiceclone",
    ])
}

fn minimax_code_models() -> Vec<ModelInfo> {
    fallback_models(&[
        "MiniMax-M3",
        "MiniMax-M2.7",
        "MiniMax-M2.7-highspeed",
    ])
}

const ZHIPU_MODELS: &[&str] = &[
    // Text / reasoning.
    "glm-5.3",
    "glm-5.2",
    "glm-5.1",
    "glm-5-turbo",
    "glm-5",
    "glm-4.7",
    "glm-4.7-flash",
    "glm-4.7-flashx",
    "glm-4.6",
    "glm-4.5-air",
    "glm-4.5-airx",
    "glm-4-flash-250414",
    "glm-4-flashx-250414",
    // Vision-language.
    "glm-5.3-flash",
    "glm-5.3-flashx",
    "glm-5v-turbo",
    "glm-4.6v",
    "autoglm-phone",
    "glm-4.6v-flash",
    "glm-4.6v-flashx",
    "glm-4v-flash",
    "glm-4.1v-thinking-flashx",
    "glm-4.1v-thinking-flash",
    // Image generation.
    "glm-image",
    "cogview-4-250304",
    "cogview-4",
    "cogview-3-flash",
    // Video generation. The Vidu family is omitted until every callable API
    // model ID is explicitly documented; do not invent an alias from a family
    // name shown in the product overview.
    "cogvideox-3",
    "cogvideox-2",
    "cogvideox-flash",
    // Audio.
    "glm-asr-2512",
    "glm-tts",
    // Vector and rerank.
    "embedding-3",
    "embedding-2",
    "rerank",
];

/// Current Zhipu OpenAPI suggestions, verified 2026-10-06 against the official
/// model overview: https://docs.bigmodel.cn/cn/guide/start/model-overview
///
/// This is intentionally static because `https://open.bigmodel.cn/api/paas/v4`
/// has no public `GET /models` operation. Keep model IDs in their callable API
/// form rather than the title casing used in parts of the documentation.
fn zhipu_models() -> Vec<ModelInfo> {
    fallback_models(ZHIPU_MODELS)
}

fn ark_coding_plan_models() -> Vec<ModelInfo> {
    fallback_models(ARK_PLAN_MODELS)
}

// ---------------------------------------------------------------------------
// Ark Agent Plan (official documentation suggestions)
// ---------------------------------------------------------------------------

/// Account catalog APIs use Access Key HMAC authentication, not the inference
/// API Key configured here. These documented suggestions are not an account
/// entitlement list. Users can still type any model or router ID.
/// Independently documented by the current Coding Plan and Agent Plan ZCode
/// guides (2026-10-06). These are suggested model names, not guaranteed account
/// entitlements or an exhaustive list of every modality offered by a plan.
const ARK_PLAN_MODELS: &[&str] = &[
    "ark-code-latest",
    "doubao-seed-evolving",
    "doubao-seed-2.1-pro",
    "doubao-seed-2.1-lite",
    "doubao-seed-2.0-mini",
    "deepseek-v4.1-flash",
    "deepseek-v4-flash",
    "deepseek-v4-pro",
    "glm-5.3",
    "glm-5.3-flash",
    "glm-latest",
    "minimax-m3",
    "kimi-k2.7-code",
    "kimi-k3",
    "kimi-k2.8-preview",
];

fn ark_agent_plan_models() -> Vec<ModelInfo> {
    fallback_models(ARK_PLAN_MODELS)
}

fn is_official_ark_agent_plan_base_url(base_url: &str) -> bool {
    exact_official_catalog_root(base_url, &["ark.cn-beijing.volces.com"], &["/api/plan/v3"])
}

/// Suggestions verified against the official Chinese Step Plan overview and
/// reasoning/audio integration guides on 2026-10-06. These are documentation
/// suggestions, not the account's live catalog: StepFun only documents the
/// standard `/v1/models` endpoint, while plan calls use `/step_plan/v1`.
/// https://platform.stepfun.com/docs/zh/step-plan/overview
const STEPFUN_PLAN_MODELS: &[&str] = &[
    "step-5-preview",
    "step-3.7-flash",
    "step-3.5-flash",
    "step-3.5-flash-2603",
    "stepaudio-2.5-realtime",
    "stepaudio-2.5-chat",
    "stepaudio-2.5-tts",
    "stepaudio-2.5-asr",
    "step-router-v1",
    "step-image-edit-2",
];

fn stepfun_plan_models() -> Vec<ModelInfo> {
    fallback_models(STEPFUN_PLAN_MODELS)
}

// ---------------------------------------------------------------------------
// StepFun (remote catalog with an official-host fallback)
// ---------------------------------------------------------------------------

/// Public StepFun baseline refreshed from official docs on 2026-10-06. It spans chat,
/// realtime speech, audio chat, dedicated TTS/ASR, and image generation/edit.
/// The live `/v1/models` catalog remains authoritative and every model it
/// returns (including unknown future IDs) is preserved. This list is only used
/// when the official host is temporarily unavailable or returns an empty list.
///
/// Keep plan-only `step-router-v1` out of this list: it is not callable through
/// the regular `https://api.stepfun.com/v1` billing endpoint.
const STEPFUN_FALLBACK_MODELS: &[&str] = &[
    // Chat / reasoning. `step-3.7-flash` accepts vision input.
    "step-5-preview",
    "step-3.7-flash",
    "step-3.5-flash",
    "step-3.5-flash-2603",
    // Realtime and audio chat use chat/realtime protocols rather than the
    // one-shot TTS or transcription tasks.
    "stepaudio-2.5-realtime",
    "stepaudio-2.5-chat",
    // Dedicated one-shot speech models.
    "stepaudio-2.5-tts",
    "stepaudio-2.5-asr",
    // The lighter dedicated TTS surface. Omitting it meant a user whose
    // catalog fetch failed could not select it at all, even though it serves
    // the same `stepfun.audio_speech` protocol.
    "step-tts-mini",
    // Image generation plus editing.
    "step-image-edit-2",
];

async fn fetch_stepfun(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<FetchedCatalog, AppError> {
    match fetch_openai_compatible(client, base_url, api_key).await {
        Ok(models) if !models.is_empty() => Ok(FetchedCatalog::remote(models)),
        Ok(_) if is_official_stepfun_base_url(base_url) => {
            warn!("StepFun models API returned an empty catalog, using fallback list");
            Ok(FetchedCatalog::documentation(fallback_models(STEPFUN_FALLBACK_MODELS)))
        }
        Ok(models) => Ok(FetchedCatalog::remote(models)),
        Err(error)
            if is_official_stepfun_base_url(base_url)
                && is_catalog_availability_error(&error) =>
        {
            warn!(
                error_code = error.error_code(),
                "StepFun models API unavailable, using fallback list"
            );
            Ok(FetchedCatalog::documentation(fallback_models(STEPFUN_FALLBACK_MODELS)))
        }
        Err(error) => Err(error),
    }
}

fn is_official_stepfun_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && matches!(url.host_str(), Some("api.stepfun.com" | "api.stepfun.ai"))
        && url.port_or_known_default() == Some(443)
        && url.path().trim_end_matches('/') == "/v1"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn is_catalog_availability_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::BadGateway(_) | AppError::Timeout(_) | AppError::RateLimited
    )
}

fn glm_coding_plan_models() -> Vec<ModelInfo> {
    fallback_models(&["glm-5.3", "glm-5.3-flash"])
}

fn qianfan_coding_plan_models() -> Vec<ModelInfo> {
    fallback_models(&[
        "qianfan-code-latest",
        "kimi-k2.5",
        "deepseek-v3.2",
        "glm-5",
        "minimax-m2.5",
        "ernie-4.5-turbo-20260402",
        "deepseek-v4-flash",
        "glm-5.1",
    ])
}

// ---------------------------------------------------------------------------
// new-api (OpenAI-compatible with /v1 enforcement)
// ---------------------------------------------------------------------------

async fn fetch_new_api(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    auth_scheme: &AuthScheme,
) -> Result<Vec<ModelInfo>, AppError> {
    let normalized = ensure_v1_path(base_url);
    let auth = AuthMaterial {
        scheme: auth_scheme.clone(),
        credentials: serde_json::json!({"api_keys":[api_key]}),
    };
    fetch_openai_compatible_with_auth(client, &normalized, &auth).await
}

/// Ensure the URL path ends with `/v1`.
///
/// Delegates to the shared URL algebra so this crate has exactly one `/v1`
/// policy. The join is idempotent: a root that already ends in `/v1` is
/// returned unchanged rather than doubled.
fn ensure_v1_path(base_url: &str) -> String {
    nomifun_model_invoke::join_endpoint(base_url, "/v1")
}

// ---------------------------------------------------------------------------
// DashScope native paginated catalog
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DashscopeModelsResponse {
    output: DashscopeModelsPage,
}

#[derive(Deserialize)]
struct DashscopeModelsPage {
    total: u64,
    models: Vec<DashscopeModel>,
}

#[derive(Deserialize)]
struct DashscopeModel {
    model: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model_info: Option<DashscopeModelLimits>,
}

#[derive(Deserialize)]
struct DashscopeModelLimits {
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    context_window: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_input_tokens: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_declared_token_limit")]
    max_output_tokens: Option<i64>,
}

impl DashscopeModel {
    fn into_info(self) -> ModelInfo {
        let limits = self.model_info;
        let combined = limits.as_ref().and_then(|limits| limits.context_window);
        let context_limit = combined.or_else(|| limits.as_ref().and_then(|limits| limits.max_input_tokens));
        let output_limit = limits.as_ref().and_then(|limits| limits.max_output_tokens);
        let token_limit_sources = (context_limit.is_some() || output_limit.is_some()).then(|| ModelTokenLimitSources {
            context_limit: context_limit.map(|_| if combined.is_some() { "model_info.context_window" } else { "model_info.max_input_tokens" }.into()),
            output_limit: output_limit.map(|_| "model_info.max_output_tokens".into()),
            context_limit_kind: context_limit.map(|_| if combined.is_some() { ModelContextLimitKind::Combined } else { ModelContextLimitKind::InputOnly }),
        });
        ModelInfo {
            id: self.model,
            name: self.name,
            tasks: Vec::new(),
            tasks_source: None,
            traits: Vec::new(),
            context_limit,
            output_limit,
            token_limit_sources,
        }
    }
}

/// The native catalog is on the configured official host at `/api/v1/models`.
/// Keep custom gateways on their supplied compatible root instead of sending
/// their credentials to a different host or assuming they expose native APIs.
pub(super) fn dashscope_native_models_url(base_url: &str) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(base_url.trim()).ok()?;
    let host = url.host_str()?;
    let workspace_host = [
        "cn-beijing", "ap-southeast-1", "ap-northeast-1", "eu-central-1", "us-east-1",
    ].iter().any(|region| {
        let suffix = format!(".{region}.maas.aliyuncs.com");
        host.strip_suffix(&suffix).is_some_and(|workspace| !workspace.is_empty() && !workspace.contains('.'))
    });
    if url.scheme() != "https"
        || !(matches!(host, "dashscope.aliyuncs.com" | "dashscope-intl.aliyuncs.com" | "cn-hongkong.dashscope.aliyuncs.com") || workspace_host)
        || url.port_or_known_default() != Some(443)
        || !matches!(url.path().trim_end_matches('/'), "" | "/compatible-mode/v1" | "/api/v1")
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_path("/api/v1/models");
    Some(url)
}

async fn fetch_dashscope(
    client: &reqwest::Client,
    base_url: &str,
    auth: &AuthMaterial,
) -> Result<Vec<ModelInfo>, AppError> {
    match dashscope_native_models_url(base_url) {
        Some(url) => fetch_dashscope_native(client, url, auth).await,
        None => fetch_openai_compatible_with_auth(client, base_url, auth).await,
    }
}

async fn fetch_dashscope_native(
    client: &reqwest::Client,
    mut url: reqwest::Url,
    auth: &AuthMaterial,
) -> Result<Vec<ModelInfo>, AppError> {
    let mut models: Vec<ModelInfo> = Vec::new();
    let mut received = 0_u64;
    let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
    for page_no in 1..=1000 {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Timeout("DashScope model catalog timed out before all pages were received".into()));
        }
        url.query_pairs_mut().clear()
            .append_pair("page_no", &page_no.to_string())
            .append_pair("page_size", "100");
        let response = apply_catalog_auth(client.get(url.clone()), auth)?
            .timeout(remaining)
            .send()
            .await
            .map_err(|error| remote_error(&error))?;
        check_response_status(&response)?;
        let body: DashscopeModelsResponse = response.json().await.map_err(|_| {
            AppError::BadGateway("DashScope native models response was not valid JSON".into())
        })?;
        let count = body.output.models.len() as u64;
        received += count;
        for model in body.output.models {
            let candidate = model.into_info();
            if !candidate.id.trim().is_empty() {
                if let Some(existing) = models.iter_mut().find(|model| model.id == candidate.id) {
                    merge_declared_limits(existing, &candidate);
                } else {
                    models.push(candidate);
                }
            }
        }
        if received >= body.output.total {
            return Ok(models);
        }
        if count == 0 {
            return Err(AppError::BadGateway("DashScope native models catalog ended before its declared total".into()));
        }
    }
    Err(AppError::BadGateway("DashScope native models catalog exceeded the supported page count".into()))
}

// ---------------------------------------------------------------------------
// dashscope-coding (official documentation suggestions)
// ---------------------------------------------------------------------------

const DASHSCOPE_MODELS: &[&str] = &[
    "qwen3.7-plus",
    "qwen3.6-plus",
    "kimi-k2.5",
    "glm-5",
    "MiniMax-M2.5",
    "qwen3.5-plus",
    "qwen3-max-2026-01-23",
    "qwen3-coder-next",
    "qwen3-coder-plus",
    "glm-4.7",
];

// ---------------------------------------------------------------------------
// Provider-declared context windows
// ---------------------------------------------------------------------------

/// Read an advisory token limit out of a catalog entry without letting it break
/// the listing.
///
/// Providers disagree on how the number is typed: Gemini and OpenRouter send a
/// JSON integer, some OpenAI-compatible gateways send a float or a decimal
/// string, and a few send `null` or `0` for models they do not describe. Model
/// discovery must not start failing with "response was not valid JSON" because
/// of a field that is only a convenience, so anything unusable degrades to
/// `None`. Non-positive values are dropped too: `resolve_context_window` already
/// treats `0` as unset, and offering it to the UI would prefill a window that
/// cannot be honored.
fn deserialize_model_display_name<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where D: serde::Deserializer<'de> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| value.as_str().map(str::trim)
        .filter(|text| !text.is_empty()).map(str::to_owned)))
}

fn deserialize_declared_token_limit<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(declared_token_limit))
}

fn declared_token_limit(value: &serde_json::Value) -> Option<i64> {
    let limit = match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().filter(|value| value.is_finite()
                && value.fract() == 0.0 && *value > 0.0 && *value < i64::MAX as f64)
                .map(|value| value as i64))?,
        serde_json::Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    (limit > 0).then_some(limit)
}

fn token_limit_sources(context_limit: Option<i64>, context_field: &str,
    output_limit: Option<i64>, output_field: &str) -> Option<ModelTokenLimitSources> {
    (context_limit.is_some() || output_limit.is_some()).then(|| ModelTokenLimitSources {
        context_limit: context_limit.map(|_| context_field.to_owned()),
        output_limit: output_limit.map(|_| output_field.to_owned()),
        context_limit_kind: context_limit.and_then(|_| match context_field {
            "inputTokenLimit" | "max_input_tokens" => Some(ModelContextLimitKind::InputOnly),
            "context_length" | "context_window" | "context_size" | "top_provider.context_length" => Some(ModelContextLimitKind::Combined),
            _ => None,
        }),
    })
}

fn merge_declared_limits(existing: &mut ModelInfo, candidate: &ModelInfo) {
    let mut sources = existing.token_limit_sources.clone().unwrap_or_default();
    if existing.context_limit.is_none() {
        existing.context_limit = candidate.context_limit;
        sources.context_limit = candidate.token_limit_sources.as_ref()
            .and_then(|source| source.context_limit.clone());
        sources.context_limit_kind = candidate.token_limit_sources.as_ref()
            .and_then(|source| source.context_limit_kind);
    }
    if existing.output_limit.is_none() {
        existing.output_limit = candidate.output_limit;
        sources.output_limit = candidate.token_limit_sources.as_ref()
            .and_then(|source| source.output_limit.clone());
    }
    if sources.context_limit.is_some() || sources.output_limit.is_some() {
        existing.token_limit_sources = Some(sources);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn fallback_models(ids: &[&str]) -> Vec<ModelInfo> {
    ids.iter()
        .map(|id| ModelInfo {
            id: (*id).to_string(),
            name: None,
            tasks: Vec::new(),
            tasks_source: None,
            traits: Vec::new(),
            context_limit: None,
            output_limit: None,
            token_limit_sources: None,
        })
        .collect()
}

fn check_response_status(resp: &reqwest::Response) -> Result<(), AppError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    match status {
        StatusCode::UNAUTHORIZED => {
            Err(AppError::Unauthorized("Remote API rejected the API key".into()))
        }
        StatusCode::FORBIDDEN => Err(AppError::Forbidden(
            "Remote API denied access for this API key".into(),
        )),
        StatusCode::TOO_MANY_REQUESTS => Err(AppError::RateLimited),
        status if status.is_client_error() => Err(AppError::BadRequest(format!(
            "Remote API rejected the model-list request ({status})"
        ))),
        status => Err(AppError::BadGateway(format!(
            "Remote API returned {status}"
        ))),
    }
}

fn remote_error(e: &reqwest::Error) -> AppError {
    if e.is_timeout() {
        AppError::Timeout(
            "Remote API request timed out; check the network and system proxy".into(),
        )
    } else if e.is_connect() {
        AppError::BadGateway(
            "Could not connect to the remote API; check DNS, TLS, firewall, and system proxy settings"
                .into(),
        )
    } else {
        // Never expose reqwest's Display text here. It includes the request URL,
        // which can carry credentials (notably Gemini's `?key=...`).
        AppError::BadGateway("Remote API request failed before a response was received".into())
    }
}

fn warn_remote_request_failure_without_fallback(provider: &str, error: &reqwest::Error) {
    warn!(
        provider,
        timeout = error.is_timeout(),
        connect = error.is_connect(),
        request = error.is_request(),
        body = error.is_body(),
        decode = error.is_decode(),
        "Provider models API unreachable; refusing to return a stale fallback list"
    );
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::{ModelTask, ModelTrait, infer_catalog_tasks_and_traits};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn compatible_catalog_keeps_declared_provider_names_and_token_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/models"))
            .and(header("accept", "application/json"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[
                {"id":"deepseek-future","name":"Future model","context_window":100000,"max_output_tokens":20000},
                {"id":"novita-future","title":"New model","context_size":"64000"},
                {"id":"infini-future","context_length":32000,"max_output_length":8000},
                {"id":"unknown-metadata","name":{},"title":[],"context_size":0,"max_output_length":"bad"}
            ]}))).expect(1).mount(&server).await;
        let models = fetch_openai_public_catalog(&no_proxy_client(), &server.uri()).await.unwrap();
        assert_eq!(models[0].name.as_deref(), Some("Future model"));
        assert_eq!(models[0].context_limit, Some(100000));
        assert_eq!(models[0].output_limit, Some(20000));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().context_limit.as_deref(), Some("context_window"));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().context_limit_kind, Some(ModelContextLimitKind::Combined));
        assert_eq!(models[1].name.as_deref(), Some("New model"));
        assert_eq!(models[1].context_limit, Some(64000));
        assert_eq!(models[2].output_limit, Some(8000));
        assert_eq!(models[2].token_limit_sources.as_ref().unwrap().output_limit.as_deref(), Some("max_output_length"));
        assert_eq!(models[3].id, "unknown-metadata");
        assert!(models[3].name.is_none());
        assert!(models[3].context_limit.is_none());
        assert!(models[3].output_limit.is_none());
        assert!(!server.received_requests().await.unwrap()[0].headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn public_deepgram_catalog_does_not_send_an_empty_auth_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stt":[{"canonical_name":"future-stt"}],"tts":[{"canonical_name":"future-tts"}]
            }))).expect(1).mount(&server).await;
        let catalog = fetch_deepgram_catalog(&no_proxy_client(), &server.uri(), "").await.unwrap();
        assert_eq!(catalog.models.len(), 2);
        assert!(!server.received_requests().await.unwrap()[0].headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn gemini_uses_the_live_v1beta_catalog_and_header_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("x-goog-api-key", "gemini-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [{"name": "models/gemini-3.1-pro"}, {"name": "models/gemini-3.1-flash"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let models = fetch_gemini(&no_proxy_client(), &server.uri(), "gemini-key")
            .await
            .unwrap();
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            ["gemini-3.1-pro", "gemini-3.1-flash"]
        );
    }

    #[tokio::test]
    async fn gemini_carries_the_declared_input_token_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {
                        "name": "models/gemini-3.1-pro",
                        "inputTokenLimit": 1_048_576,
                        "outputTokenLimit": 65_536,
                        "supportedGenerationMethods": ["generateContent"]
                    },
                    // Embedding-style entries omit the window entirely.
                    {"name": "models/text-embedding-005"}
                ]
            })))
            .mount(&server)
            .await;

        let models = fetch_gemini(&no_proxy_client(), &server.uri(), "gemini-key")
            .await
            .unwrap();
        assert_eq!(models[0].id, "gemini-3.1-pro");
        assert_eq!(models[0].context_limit, Some(1_048_576));
        assert_eq!(models[0].output_limit, Some(65_536));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().context_limit_kind, Some(ModelContextLimitKind::InputOnly));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().output_limit.as_deref(), Some("outputTokenLimit"));
        assert_eq!(models[1].context_limit, None);
        assert_eq!(models[1].output_limit, None);
    }

    #[tokio::test]
    async fn gemini_only_confirms_unambiguous_native_task_methods() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name":"models/opaque-vector", "supportedGenerationMethods":["embedContent", "batchEmbedContents"]},
                    {"name":"models/opaque-live", "supportedGenerationMethods":["bidiGenerateContent"]},
                    {"name":"models/opaque-output", "supportedGenerationMethods":["generateContent"]},
                    {"name":"models/opaque-future", "supportedGenerationMethods":["futureGenerationMethod"]}
                ]
            })))
            .expect(1).mount(&server).await;
        let models = fetch_gemini(&no_proxy_client(), &server.uri(), "gemini-key").await.unwrap();
        assert_eq!(models[0].tasks, vec![ModelTask::Embedding]);
        assert_eq!(models[1].tasks, vec![ModelTask::RealtimeConversation]);
        for model in &models[..2] {
            assert_eq!(model.tasks_source, Some(ModelTaskSource::ProviderDeclared));
        }
        for model in &models[2..] {
            assert!(model.tasks.is_empty());
            assert_eq!(model.tasks_source, None);
        }
    }

    #[tokio::test]
    async fn openai_compatible_catalog_carries_context_length_when_the_gateway_sends_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "openrouter/gateway-model", "context_length": 131_072},
                    // A gateway that types the same field loosely must not fail
                    // the whole listing.
                    {"id": "decimal-string-gateway", "context_length": "32768"},
                    {"id": "float-gateway", "context_length": 65_536.0},
                    // Unusable or absent values stay absent; nothing is guessed.
                    {"id": "zero-gateway", "context_length": 0},
                    {"id": "null-gateway", "context_length": serde_json::Value::Null},
                    {"id": "prose-gateway", "context_length": "unknown"},
                    {"id": "plain-openai-model"}
                ]
            })))
            .mount(&server)
            .await;

        let models = fetch_openai_compatible(&no_proxy_client(), &server.uri(), "key")
            .await
            .unwrap();
        let limits = models
            .iter()
            .map(|model| (model.id.as_str(), model.context_limit))
            .collect::<Vec<_>>();
        assert_eq!(
            limits,
            [
                ("openrouter/gateway-model", Some(131_072)),
                ("decimal-string-gateway", Some(32_768)),
                ("float-gateway", Some(65_536)),
                ("zero-gateway", None),
                ("null-gateway", None),
                ("prose-gateway", None),
                ("plain-openai-model", None),
            ]
        );
    }

    #[test]
    fn catalog_output_limits_have_explicit_field_provenance_not_request_defaults() {
        use serde_json::json;
        let models: OpenAiModelsResponse = serde_json::from_value(json!({"data":[
            {"id":"openrouter/model","context_length":131072,"top_provider":{"context_length":65536,"max_completion_tokens":32768}},
            {"id":"direct-declared","max_output_tokens":"65536"},
            {"id":"request-default-not-ceiling","max_tokens":4096,"default_parameters":{"max_tokens":4096}},
            {"id":"invalid","max_output_tokens":200.5},
            {"id":"plain-openai"}
        ]})).unwrap();
        let models = models.data.into_iter().map(OpenAiModel::into_info).collect::<Vec<_>>();
        assert_eq!(models[0].context_limit, Some(131072)); // Never silently min with another field.
        assert_eq!(models[0].output_limit, Some(32768));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().context_limit_kind, Some(ModelContextLimitKind::Combined));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().output_limit.as_deref(), Some("top_provider.max_completion_tokens"));
        assert_eq!(models[1].output_limit, Some(65536));
        assert_eq!(models[1].token_limit_sources.as_ref().unwrap().output_limit.as_deref(), Some("max_output_tokens"));
        for model in &models[2..] {
            assert_eq!(model.output_limit, None);
            assert_eq!(model.token_limit_sources, None);
        }
    }

    #[test]
    fn anthropic_native_catalog_carries_new_declared_limits_and_preserves_older_unknowns() {
        let response: AnthropicModelsResponse = serde_json::from_value(serde_json::json!({"data":[
            {"id":"claude-live","display_name":"Claude live","max_input_tokens":1000000,"max_tokens":64000},
            {"id":"older-identity-only"}
        ]})).unwrap();
        assert_eq!(response.data[0].max_input_tokens, Some(1000000));
        assert_eq!(response.data[0].max_tokens, Some(64000));
        assert_eq!(token_limit_sources(response.data[0].max_input_tokens, "max_input_tokens", response.data[0].max_tokens, "max_tokens").unwrap().context_limit_kind, Some(ModelContextLimitKind::InputOnly));
        assert_eq!(response.data[1].max_input_tokens, None);
        assert_eq!(response.data[1].max_tokens, None);
    }

    #[test]
    fn duplicate_catalog_merge_retains_the_first_declared_limit_without_clamping() {
        let mut existing: ModelInfo = serde_json::from_value(serde_json::json!({
            "id":"model","context_limit":1000000,"output_limit":64000,
            "token_limit_sources":{"context_limit":"context_length","output_limit":"max_output_tokens"}
        })).unwrap();
        let smaller: ModelInfo = serde_json::from_value(serde_json::json!({"id":"model","context_limit":32000,"output_limit":4096})).unwrap();
        merge_declared_limits(&mut existing, &smaller);
        assert_eq!(existing.context_limit, Some(1000000));
        assert_eq!(existing.output_limit, Some(64000));
        assert_eq!(existing.token_limit_sources.as_ref().unwrap().output_limit.as_deref(), Some("max_output_tokens"));
    }

    #[test]
    fn declared_token_limit_accepts_only_usable_positive_numbers() {
        use serde_json::json;

        assert_eq!(declared_token_limit(&json!(200_000)), Some(200_000));
        assert_eq!(declared_token_limit(&json!(200_000.7)), None);
        assert_eq!(declared_token_limit(&json!(65_536.0)), Some(65_536));
        assert_eq!(declared_token_limit(&json!(9_223_372_036_854_775_808_u64)), None);
        assert_eq!(declared_token_limit(&json!(" 32768 ")), Some(32_768));
        for unusable in [
            json!(0),
            json!(-1),
            json!("0"),
            json!("128k"),
            json!(""),
            json!(serde_json::Value::Null),
            json!(true),
            json!({"tokens": 4096}),
            json!([4096]),
        ] {
            assert_eq!(declared_token_limit(&unusable), None, "{unusable}");
        }
    }

    #[tokio::test]
    async fn deepgram_uses_native_catalog_token_auth_and_preserves_source_tasks() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Token dg-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stt": [
                    {"name": "Opaque input model", "canonical_name": "future-alpha"},
                    {"name": "Shared model", "canonical_name": "shared-canonical"}
                ],
                "tts": [
                    {"name": "Opaque output model", "canonical_name": "future-beta"},
                    {"name": "Shared model", "canonical_name": "shared-canonical"}
                ]
            })))
            .expect(2)
            .mount(&server)
            .await;

        // Both the preset host root and a user-entered `/v1` root must resolve
        // to the one native endpoint, never `/models` or `/v1/v1/models`.
        for base_url in [server.uri(), format!("{}/v1", server.uri())] {
            let catalog = fetch_deepgram_catalog(&no_proxy_client(), &base_url, "dg-key")
                .await
                .unwrap();
            assert_eq!(
                catalog.models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
                ["future-alpha", "shared-canonical", "future-beta"]
            );
            assert_eq!(
                catalog.models.iter().find(|model| model.id == "future-alpha").unwrap().tasks,
                vec![ModelTask::SpeechRecognition]
            );
            assert_eq!(
                catalog.models.iter().find(|model| model.id == "future-beta").unwrap().tasks,
                vec![ModelTask::SpeechSynthesis]
            );
            assert_eq!(
                catalog.models.iter().find(|model| model.id == "shared-canonical").unwrap().tasks,
                vec![ModelTask::SpeechRecognition, ModelTask::SpeechSynthesis]
            );
            assert!(catalog.models.iter().all(|model|
                model.tasks_source == Some(ModelTaskSource::ProviderDeclared)));
        }
    }

    #[tokio::test]
    async fn xai_merges_modality_catalogs_and_explicit_audio_service_profiles() {
        let server = MockServer::start().await;
        for (endpoint, ids) in [
            ("language-models", vec!["grok-4", "shared-model"]),
            ("image-generation-models", vec!["grok-imagine-image", "shared-model"]),
            ("video-generation-models", vec!["grok-imagine-video"]),
        ] {
            Mock::given(method("GET"))
                .and(path(format!("/v1/{endpoint}")))
                .and(header("authorization", "Bearer xai-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "models": ids.into_iter().map(|id| serde_json::json!({"id": id})).collect::<Vec<_>>()
                })))
                .expect(1)
                .mount(&server)
                .await;
        }

        let models = fetch_xai(&no_proxy_client(), &server.uri(), "xai-key")
            .await
            .unwrap();
        assert_eq!(
            models.iter().find(|model| model.id == "shared-model").unwrap().tasks,
            vec![ModelTask::Chat, ModelTask::ImageGeneration]
        );
        let ids = models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>();
        assert!(models.iter().filter(|model| !model.id.starts_with("xai-"))
            .all(|model| model.tasks_source == Some(ModelTaskSource::ProviderDeclared)));
        assert!(models.iter().filter(|model| model.id.starts_with("xai-"))
            .all(|model| model.tasks_source == Some(ModelTaskSource::OfficialDocumentation)));
        assert_eq!(
            ids,
            [
                "grok-4",
                "shared-model",
                "grok-imagine-image",
                "grok-imagine-video",
                "xai-tts",
                "xai-stt",
            ]
        );
    }

    #[tokio::test]
    async fn regional_live_catalogs_preserve_future_models_and_supplied_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer catalog-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "future-official-model"}, {"id": "account-custom-model"}]
            })))
            .expect(3)
            .mount(&server)
            .await;
        for platform in ["mimo", "minimax", "minimax-code"] {
            let catalog = fetch_for_platform(&no_proxy_client(), &FetchConfig {
                platform: platform.into(),
                base_url: format!("{}/v1", server.uri()),
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({"api_keys": ["catalog-key"]}) },
                bedrock_config: None,
            }).await.unwrap();
            assert_eq!(catalog.source, ModelCatalogSource::Remote);
            assert_eq!(catalog.models.into_iter().map(|model| model.id).collect::<Vec<_>>(), ["future-official-model", "account-custom-model"]);
        }
    }

    #[tokio::test]
    async fn regional_mimo_discovery_supports_official_api_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("api-key", "catalog-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "mimo-future"}]})))
            .expect(1)
            .mount(&server)
            .await;
        let catalog = fetch_for_platform(&no_proxy_client(), &FetchConfig {
            platform: "mimo".into(),
            base_url: format!("{}/v1", server.uri()),
            auth: AuthMaterial { scheme: AuthScheme::HeaderKey("api-key".into()), credentials: serde_json::json!({"api_keys": ["catalog-key"]}) },
            bedrock_config: None,
        }).await.unwrap();
        assert_eq!(catalog.source, ModelCatalogSource::Remote);
        assert_eq!(catalog.models[0].id, "mimo-future");
    }

    #[tokio::test]
    async fn regional_documentation_catalogs_do_not_require_credentials_or_network() {
        for (platform, base_url) in [
            ("mimo-token-plan-cn", "https://token-plan-cn.xiaomimimo.com/v1"),
            ("mimo-token-plan-sgp", "https://token-plan-sgp.xiaomimimo.com/v1"),
            ("mimo-token-plan-ams", "https://token-plan-ams.xiaomimimo.com/v1"),
            ("minimax-coding-plan", "https://api.minimaxi.com/v1"),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4"),
            ("glm-coding-plan", "https://open.bigmodel.cn/api/coding/paas/v4"),
            ("ark-coding-plan", "https://ark.cn-beijing.volces.com/api/coding/v3"),
            ("ark-agent-plan", "https://ark.cn-beijing.volces.com/api/plan/v3"),
            ("qianfan-coding-plan", "https://qianfan.baidubce.com/v2/coding"),
        ] {
            let catalog = fetch_for_platform(&no_proxy_client(), &FetchConfig {
                platform: platform.into(), base_url: base_url.into(),
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({}) },
                bedrock_config: None,
            }).await.unwrap();
            assert_eq!(catalog.source, ModelCatalogSource::OfficialDocumentation, "{platform}");
            assert!(!catalog.models.is_empty(), "{platform}");
        }
    }

    #[tokio::test]
    async fn regional_ark_agent_custom_gateway_retains_compatible_catalog() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/plan/v3/models"))
            .and(header("authorization", "Bearer catalog-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "custom-plan-model"}]})))
            .expect(1)
            .mount(&server)
            .await;
        let catalog = fetch_for_platform(&no_proxy_client(), &FetchConfig {
            platform: "ark-agent-plan".into(), base_url: format!("{}/plan/v3", server.uri()),
            auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({"api_keys": ["catalog-key"]}) },
            bedrock_config: None,
        }).await.unwrap();
        assert_eq!(catalog.source, ModelCatalogSource::Remote);
        assert_eq!(catalog.models[0].id, "custom-plan-model");
    }

    #[test]
    fn regional_dashscope_native_catalog_uses_only_recognized_official_roots() {
        for base in [
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/",
            "https://cn-hongkong.dashscope.aliyuncs.com/compatible-mode/v1",
            "https://workspace.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        ] {
            let original = reqwest::Url::parse(base).unwrap();
            let native = dashscope_native_models_url(base).unwrap();
            assert_eq!(native.host_str(), original.host_str());
            assert_eq!(native.path(), "/api/v1/models");
        }
        for base in [
            "http://dashscope.aliyuncs.com/compatible-mode/v1",
            "https://dashscope.aliyuncs.com.evil.example/compatible-mode/v1",
            "https://proxy.example/compatible-mode/v1",
            "https://coding.dashscope.aliyuncs.com/v1",
            "https://dashscope.aliyuncs.com/other-api",
            "https://dashscope.aliyuncs.com/compatible-mode/v1?routing=custom",
        ] {
            assert!(dashscope_native_models_url(base).is_none(), "{base}");
        }
    }

    #[tokio::test]
    async fn regional_dashscope_native_catalog_reads_every_page_and_declared_limits() {
        use wiremock::matchers::query_param;
        let server = MockServer::start().await;
        for (page, data) in [
            ("1", serde_json::json!([{"model": "qwen-current", "name": "Current", "model_info": {"context_window": 1000000, "max_output_tokens": 128000}}])),
            ("2", serde_json::json!([{"model": "future-modality-model", "model_info": {"max_input_tokens": "64000", "max_output_tokens": null}}])),
        ] {
            Mock::given(method("GET"))
                .and(path("/api/v1/models"))
                .and(query_param("page_no", page))
                .and(query_param("page_size", "100"))
                .and(header("authorization", "Bearer catalog-key"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"output": {"total": 2, "models": data}})))
                .expect(1)
                .mount(&server)
                .await;
        }
        let models = fetch_dashscope_native(&no_proxy_client(), reqwest::Url::parse(&format!("{}/api/v1/models", server.uri())).unwrap(),
            &AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({"api_keys": ["catalog-key"]}) }).await.unwrap();
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["qwen-current", "future-modality-model"]);
        assert_eq!(models[0].name.as_deref(), Some("Current"));
        assert_eq!(models[0].context_limit, Some(1000000));
        assert_eq!(models[0].output_limit, Some(128000));
        assert_eq!(models[0].token_limit_sources.as_ref().unwrap().context_limit_kind, Some(ModelContextLimitKind::Combined));
        assert_eq!(models[1].context_limit, Some(64000));
        assert_eq!(models[1].token_limit_sources.as_ref().unwrap().context_limit_kind, Some(ModelContextLimitKind::InputOnly));
    }

    #[tokio::test]
    async fn regional_dashscope_custom_gateway_keeps_supplied_path_and_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/custom/compatible-mode/v1/models"))
            .and(header("x-catalog-key", "catalog-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "custom-qwen"}]})))
            .expect(1)
            .mount(&server)
            .await;
        let catalog = fetch_for_platform(&no_proxy_client(), &FetchConfig {
            platform: "dashscope".into(), base_url: format!("{}/custom/compatible-mode/v1", server.uri()),
            auth: AuthMaterial { scheme: AuthScheme::HeaderKey("x-catalog-key".into()), credentials: serde_json::json!({"api_keys": ["catalog-key"]}) },
            bedrock_config: None,
        }).await.unwrap();
        assert_eq!(catalog.source, ModelCatalogSource::Remote);
        assert_eq!(catalog.models[0].id, "custom-qwen");
    }

    #[tokio::test]
    async fn regional_dashscope_native_catalog_rejects_incomplete_pagination() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"output": {"total": 5, "models": []}})))
            .expect(1)
            .mount(&server)
            .await;
        let result = fetch_dashscope_native(&no_proxy_client(), reqwest::Url::parse(&format!("{}/api/v1/models", server.uri())).unwrap(),
            &AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({"api_keys": ["catalog-key"]}) }).await;
        assert!(matches!(result, Err(AppError::BadGateway(message)) if message.contains("declared total")));
    }

    #[test]
    fn regional_subscription_roots_override_only_their_matching_official_family() {
        for (family, root, effective) in [
            ("ark", "https://ark.cn-beijing.volces.com/api/plan/v3", "ark-agent-plan"),
            ("ark", "https://ark.cn-beijing.volces.com/api/coding/v3", "ark-coding-plan"),
            ("dashscope", "https://coding.dashscope.aliyuncs.com/v1", "dashscope-coding"),
            ("zhipu", "https://open.bigmodel.cn/api/coding/paas/v4", "glm-coding-plan"),
            ("qianfan", "https://qianfan.baidubce.com/v2/coding", "qianfan-coding-plan"),
            ("mimo", "https://token-plan-sgp.xiaomimimo.com/v1", "mimo-token-plan-sgp"),
        ] {
            assert_eq!(catalog_platform(family, root), effective);
        }
        assert_eq!(catalog_platform("ark", "https://proxy.example/api/plan/v3"), "ark");
        assert_eq!(catalog_platform("ark", "https://ark.cn-beijing.volces.com/api/plan/v3?route=custom"), "ark");
        assert_eq!(catalog_platform("mimo", "https://token-plan-sgp.xiaomimimo.com.evil.example/v1"), "mimo");
    }

    #[test]
    fn ensure_v1_path_already_present() {
        assert_eq!(
            ensure_v1_path("https://api.example.com/v1"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn ensure_v1_path_missing() {
        assert_eq!(
            ensure_v1_path("https://api.example.com"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn ensure_v1_path_trailing_slash() {
        assert_eq!(
            ensure_v1_path("https://api.example.com/"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn ensure_v1_path_with_v1_and_trailing_slash() {
        assert_eq!(
            ensure_v1_path("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn mimo_token_plan_suggestions_include_current_models_without_ultraspeed() {
        let models = mimo_token_plan_models();
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            vec![
                "mimo-v2.6-pro",
                "mimo-v2.6-flash",
                "mimo-v2.5-pro",
                "mimo-v2.5",
                "mimo-v2.5-asr",
                "mimo-v2.5-tts",
                "mimo-v2.5-tts-voicedesign",
                "mimo-v2.5-tts-voiceclone",
            ]
        );
    }

    #[test]
    fn minimax_code_plan_models_include_current_coding_models() {
        assert!(minimax_code_models().iter().any(|model| model.id == "MiniMax-M3"));
        assert!(minimax_code_models().iter().any(|model| model.id == "MiniMax-M2.7-highspeed"));
        assert_eq!(minimax_code_models().len(), 3);
    }

    #[test]
    fn zhipu_static_catalog_matches_verified_openapi_baseline() {
        assert_eq!(
            zhipu_models()
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>(),
            ZHIPU_MODELS
        );
    }

    #[test]
    fn coding_plan_fallbacks_include_default_router_models() {
        assert!(ark_coding_plan_models().iter().any(|model| model.id == "ark-code-latest"));
        assert!(stepfun_plan_models().iter().any(|model| model.id == "step-router-v1"));
        assert!(glm_coding_plan_models().iter().any(|model| model.id == "glm-5.3"));
        assert!(qianfan_coding_plan_models().iter().any(|model| model.id == "qianfan-code-latest"));
    }

    #[test]
    fn coding_plan_suggestions_match_current_official_documentation() {
        assert_eq!(
            DASHSCOPE_MODELS,
            [
                "qwen3.7-plus",
                "qwen3.6-plus",
                "kimi-k2.5",
                "glm-5",
                "MiniMax-M2.5",
                "qwen3.5-plus",
                "qwen3-max-2026-01-23",
                "qwen3-coder-next",
                "qwen3-coder-plus",
                "glm-4.7",
            ]
        );
        assert_eq!(
            glm_coding_plan_models()
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>(),
            ["glm-5.3", "glm-5.3-flash"]
        );
        assert_eq!(
            qianfan_coding_plan_models()
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>(),
            [
                "qianfan-code-latest",
                "kimi-k2.5",
                "deepseek-v3.2",
                "glm-5",
                "minimax-m2.5",
                "ernie-4.5-turbo-20260402",
                "deepseek-v4-flash",
                "glm-5.1",
            ]
        );
        assert_eq!(
            stepfun_plan_models()
                .into_iter()
                .map(|model| model.id)
                .collect::<Vec<_>>(),
            [
                "step-5-preview",
                "step-3.7-flash",
                "step-3.5-flash",
                "step-3.5-flash-2603",
                "stepaudio-2.5-realtime",
                "stepaudio-2.5-chat",
                "stepaudio-2.5-tts",
                "stepaudio-2.5-asr",
                "step-router-v1",
                "step-image-edit-2",
            ]
        );
    }

    #[tokio::test]
    async fn dashscope_coding_catalog_does_not_require_models_or_billable_chat_probe() {
        let catalog = fetch_for_platform(
            &no_proxy_client(),
            &FetchConfig {
                platform: "dashscope-coding".into(),
                base_url: "http://127.0.0.1:1/v1".into(),
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: serde_json::json!({}) },
                bedrock_config: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            catalog.models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            DASHSCOPE_MODELS
        );
        assert_eq!(catalog.source, ModelCatalogSource::OfficialDocumentation);
    }

    #[test]
    fn stepfun_fallback_has_public_models_but_not_plan_only_router() {
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            [
                "step-5-preview",
                "step-3.7-flash",
                "step-3.5-flash",
                "step-3.5-flash-2603",
                "stepaudio-2.5-realtime",
                "stepaudio-2.5-chat",
                "stepaudio-2.5-tts",
                "stepaudio-2.5-asr",
                "step-tts-mini",
                "step-image-edit-2",
            ]
        );
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        assert!(!models.iter().any(|model| model.id == "step-router-v1"));
    }

    #[test]
    fn stepfun_fallback_offers_speech_models_so_the_robot_voice_slots_are_fillable() {
        // Without ASR/TTS ids here, a first-run/offline install has no speech
        // model to select, so the robot's `voice.asr` / `voice.tts` slots stay
        // empty and the device is silent. See the 2026-08-08 stepfun-robot spec.
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        for id in ["stepaudio-2.5-asr", "stepaudio-2.5-tts"] {
            assert!(
                models.iter().any(|model| model.id == id),
                "StepFun fallback list must offer {id} so the voice slots are fillable"
            );
        }
    }

    #[test]
    fn stepfun_current_catalog_entries_derive_the_expected_capabilities() {
        // Exact catalog IDs seed inline task and trait suggestions.
        for platform in ["stepfun", "stepfun-plan"] {
            assert_eq!(
                infer_catalog_tasks_and_traits(platform, "stepaudio-2.5-tts").0,
                vec![ModelTask::SpeechSynthesis]
            );
            assert_eq!(
                infer_catalog_tasks_and_traits(platform, "stepaudio-2.5-asr").0,
                vec![ModelTask::SpeechRecognition]
            );
            let (tasks, traits) =
                infer_catalog_tasks_and_traits(platform, "stepaudio-2.5-realtime");
            assert_eq!(tasks, vec![ModelTask::RealtimeConversation]);
            assert!(!tasks.contains(&ModelTask::Chat));
            assert!(traits.is_empty());
            let (tasks, traits) =
                infer_catalog_tasks_and_traits(platform, "stepaudio-2.5-chat");
            assert_eq!(tasks, vec![ModelTask::Chat]);
            assert_eq!(traits, vec![ModelTrait::AudioInput]);
            assert_eq!(
                infer_catalog_tasks_and_traits(platform, "step-image-edit-2").0,
                vec![ModelTask::ImageGeneration, ModelTask::ImageEdit]
            );
        }

        let (tasks, traits) = infer_catalog_tasks_and_traits("stepfun", "step-3.7-flash");
        assert_eq!(tasks, vec![ModelTask::Chat]);
        assert_eq!(
            traits,
            vec![ModelTrait::VisionInput, ModelTrait::VideoInput]
        );
    }

    #[tokio::test]
    async fn stepfun_live_catalog_preserves_unknown_future_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "step-3.7-flash"},
                    {"id": "step-future-modality-1"}
                ]
            })))
            .mount(&server)
            .await;

        let models = fetch_stepfun(&no_proxy_client(), &server.uri(), "test-key")
            .await
            .unwrap();

        assert_eq!(models.source, ModelCatalogSource::Remote);
        assert_eq!(
            models.models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            ["step-3.7-flash", "step-future-modality-1"]
        );
    }

    #[test]
    fn stepfun_catalog_channel_follows_exact_official_plan_roots() {
        for host in ["api.stepfun.com", "api.stepfun.ai"] {
            for suffix in ["/step_plan", "/step_plan/v1", "/step_plan/v1/"] {
                assert_eq!(
                    catalog_platform("stepfun", &format!("https://{host}{suffix}")),
                    "stepfun-plan"
                );
            }
        }
        for root in [
            "https://api.stepfun.com/v1",
            "http://api.stepfun.com/step_plan/v1",
            "https://api.stepfun.com.evil.example/step_plan/v1",
            "https://api.example.com/step_plan/v1",
            "https://api.stepfun.com/step_plan/v1?key=ignored",
            "https://api.stepfun.com/step_plan/v1#fragment",
            "https://user:password@api.stepfun.com/step_plan/v1",
        ] {
            assert_eq!(catalog_platform("stepfun", root), "stepfun", "{root}");
        }
        assert_eq!(
            catalog_platform("custom", "https://api.stepfun.com/step_plan/v1"),
            "custom"
        );
    }

    #[tokio::test]
    async fn stepfun_general_family_with_official_plan_root_offers_plan_documentation() {
        let catalog = fetch_for_platform(
            &no_proxy_client(),
            &FetchConfig {
                platform: "stepfun".into(),
                base_url: "https://api.stepfun.com/step_plan/v1".into(),
                auth: AuthMaterial {
                    scheme: AuthScheme::Bearer,
                    credentials: serde_json::json!({"api_keys":["test-only-key"]}),
                },
                bedrock_config: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(catalog.source, ModelCatalogSource::OfficialDocumentation);
        assert!(catalog.models.iter().any(|model| model.id == "step-5-preview"));
        assert!(catalog.models.iter().any(|model| model.id == "step-router-v1"));
    }

    #[test]
    fn stepfun_fallback_is_restricted_to_exact_official_https_host() {
        assert!(is_official_stepfun_base_url(
            "https://api.stepfun.com/v1"
        ));
        assert!(is_official_stepfun_base_url(
            " https://api.stepfun.com/v1/ "
        ));
        assert!(!is_official_stepfun_base_url(
            "http://api.stepfun.com/v1"
        ));
        assert!(!is_official_stepfun_base_url(
            "https://api.stepfun.com.evil.example/v1"
        ));
        assert!(!is_official_stepfun_base_url(
            "https://proxy.example.com/v1"
        ));
        assert!(!is_official_stepfun_base_url(
            "https://api.stepfun.com/not-v1"
        ));
        assert!(!is_official_stepfun_base_url(
            "https://api.stepfun.com/v1?route=other"
        ));
    }

    #[test]
    fn catalog_fallback_never_masks_bad_credentials_or_requests() {
        assert!(is_catalog_availability_error(&AppError::BadGateway(
            "upstream 500".into()
        )));
        assert!(is_catalog_availability_error(&AppError::Timeout(
            "slow".into()
        )));
        assert!(is_catalog_availability_error(&AppError::RateLimited));
        assert!(!is_catalog_availability_error(&AppError::Unauthorized(
            "bad key".into()
        )));
        assert!(!is_catalog_availability_error(&AppError::Forbidden(
            "no access".into()
        )));
        assert!(!is_catalog_availability_error(&AppError::BadRequest(
            "bad endpoint".into()
        )));
    }

    #[tokio::test]
    async fn remote_transport_error_does_not_expose_url_credentials() {
        let secret = "must-not-appear";
        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:1/models?key={secret}"))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .unwrap_err();

        let public_error = remote_error(&error).to_string();
        assert!(!public_error.contains(secret));
        assert!(!public_error.contains("?key="));
        // Windows can classify a refused loopback connection as connect,
        // request, or timeout depending on the networking stack. The stable
        // contract of this test is that every public message is non-empty and
        // credential-safe, independent of that platform classification.
        assert!(!public_error.is_empty());
    }

    #[test]
    fn ark_agent_plan_documentation_includes_current_router_alias_and_families() {
        let models = ark_agent_plan_models();
        // Router alias must be present — it is the recommended, console-switchable entry.
        assert!(models.iter().any(|model| model.id == "ark-code-latest"));
        // Current documented names remain suggestions; account entitlements vary.
        assert!(models.iter().any(|model| model.id == "glm-5.3"));
        assert!(models.iter().any(|model| model.id == "deepseek-v4-flash"));
    }

    #[test]
    fn fallback_models_builds_model_info_list() {
        let models = fallback_models(&["a", "b", "c"]);
        assert_eq!(models.len(), 3);
        assert_eq!(
            models[0],
            ModelInfo {
                id: "a".into(),
                name: None,
                tasks: Vec::new(),
                tasks_source: None,
                traits: Vec::new(),
                context_limit: None,
                output_limit: None,
                token_limit_sources: None,
            }
        );
    }

    #[test]
    fn bedrock_anthropic_detection_covers_cross_region_profiles_and_backing_arns() {
        for identifier in [
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
            "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
            "eu.anthropic.claude-sonnet-4-20250514-v1:0",
            "arn:aws:bedrock:us-east-1::foundation-model/anthropic.claude-3-haiku-20240307-v1:0",
        ] {
            assert!(is_anthropic_bedrock_identifier(identifier), "{identifier}");
        }
        for identifier in [
            "amazon.nova-pro-v1:0",
            "us.meta.llama3-3-70b-instruct-v1:0",
            "arn:aws:bedrock:us-east-1::foundation-model/mistral.mistral-large-2407-v1:0",
        ] {
            assert!(!is_anthropic_bedrock_identifier(identifier), "{identifier}");
        }
    }

    #[tokio::test]
    async fn anthropic_catalog_paginates_with_opaque_cursors_and_keeps_every_model() {
        const CURSOR: &str = "https://other.invalid/models?key=value +&/";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "anthropic-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(|request: &wiremock::Request| {
                let cursor = request.url.query_pairs()
                    .find(|(key, _)| key == "after_id")
                    .map(|(_, value)| value.into_owned());
                match cursor.as_deref() {
                    None => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "data": [{"id": "claude-page-one", "display_name": "First page"}],
                        "has_more": true,
                        "last_id": CURSOR
                    })),
                    Some(CURSOR) => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "data": [{
                            "id": "claude-future-model",
                            "display_name": "Future model",
                            "max_input_tokens": 1_048_576,
                            "max_tokens": 65_536
                        }],
                        "has_more": false
                    })),
                    _ => ResponseTemplate::new(400),
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let models = fetch_anthropic(&no_proxy_client(), &server.uri(), "anthropic-key")
            .await.unwrap();
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            ["claude-page-one", "claude-future-model"]);
        assert_eq!(models[1].name.as_deref(), Some("Future model"));
        assert_eq!(models[1].context_limit, Some(1_048_576));
        assert_eq!(models[1].output_limit, Some(65_536));
        assert_eq!(models[1].token_limit_sources.as_ref().unwrap().context_limit.as_deref(),
            Some("max_input_tokens"));
    }

    #[tokio::test]
    async fn gemini_catalog_paginates_with_opaque_tokens_and_keeps_every_model() {
        const CURSOR: &str = "https://other.invalid/models?key=value +&/";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("x-goog-api-key", "gemini-key"))
            .respond_with(|request: &wiremock::Request| {
                let cursor = request.url.query_pairs()
                    .find(|(key, _)| key == "pageToken")
                    .map(|(_, value)| value.into_owned());
                match cursor.as_deref() {
                    None => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "models": [{"name": "models/gemini-page-one"}],
                        "nextPageToken": CURSOR
                    })),
                    Some(CURSOR) => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "models": [{
                            "name": "models/gemini-future-model",
                            "inputTokenLimit": 1_048_576,
                            "outputTokenLimit": 65_536,
                            "supportedGenerationMethods": ["futureGenerationMethod"]
                        }, {"name": "models/embedding-future-model"}]
                    })),
                    _ => ResponseTemplate::new(400),
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let models = fetch_gemini(&no_proxy_client(), &server.uri(), "gemini-key")
            .await.unwrap();
        assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            ["gemini-page-one", "gemini-future-model", "embedding-future-model"]);
        assert_eq!(models[1].context_limit, Some(1_048_576));
        assert_eq!(models[1].output_limit, Some(65_536));
        assert_eq!(models[1].token_limit_sources.as_ref().unwrap().output_limit.as_deref(),
            Some("outputTokenLimit"));
        assert_eq!(models[2].context_limit, None);
    }

    #[tokio::test]
    async fn native_catalog_pagination_rejects_a_failed_later_page() {
        for (platform, endpoint, auth_header, cursor_param) in [
            ("anthropic", "/v1/models", "x-api-key", "after_id"),
            ("gemini", "/v1beta/models", "x-goog-api-key", "pageToken"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(endpoint))
                .and(header(auth_header, "native-key"))
                .respond_with(move |request: &wiremock::Request| {
                    if request.url.query_pairs().any(|(key, _)| key == cursor_param) {
                        return ResponseTemplate::new(503);
                    }
                    let body = if platform == "anthropic" {
                        serde_json::json!({"data": [{"id": "partial-model"}],
                            "has_more": true, "last_id": "next-page"})
                    } else {
                        serde_json::json!({"models": [{"name": "models/partial-model"}],
                            "nextPageToken": "next-page"})
                    };
                    ResponseTemplate::new(200).set_body_json(body)
                })
                .expect(2)
                .mount(&server)
                .await;
            let result = if platform == "anthropic" {
                fetch_anthropic(&no_proxy_client(), &server.uri(), "native-key").await
            } else {
                fetch_gemini(&no_proxy_client(), &server.uri(), "native-key").await
            };
            assert!(matches!(result, Err(AppError::BadGateway(_))), "{platform}");
        }
    }

    #[tokio::test]
    async fn native_catalog_pagination_rejects_repeated_cursors() {
        for (platform, endpoint) in [("anthropic", "/v1/models"), ("gemini", "/v1beta/models")] {
            let server = MockServer::start().await;
            let body = if platform == "anthropic" {
                serde_json::json!({"data": [{"id": "partial-model"}],
                    "has_more": true, "last_id": "repeated-cursor"})
            } else {
                serde_json::json!({"models": [{"name": "models/partial-model"}],
                    "nextPageToken": "repeated-cursor"})
            };
            Mock::given(method("GET"))
                .and(path(endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(2)
                .mount(&server)
                .await;
            let error = if platform == "anthropic" {
                fetch_anthropic(&no_proxy_client(), &server.uri(), "native-key").await
            } else {
                fetch_gemini(&no_proxy_client(), &server.uri(), "native-key").await
            }.unwrap_err();
            assert!(error.to_string().contains("repeated a pagination cursor"), "{platform}");
        }
    }

    #[tokio::test]
    async fn anthropic_catalog_pagination_requires_a_cursor_when_more_pages_exist() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "partial-model"}], "has_more": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        let error = fetch_anthropic(&no_proxy_client(), &server.uri(), "native-key")
            .await.unwrap_err();
        assert!(error.to_string().contains("missing its next-page cursor"));
    }

    #[tokio::test]
    async fn native_catalog_pagination_bounds_non_repeating_cursors() {
        for (platform, endpoint, cursor_param) in [
            ("anthropic", "/v1/models", "after_id"),
            ("gemini", "/v1beta/models", "pageToken"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(endpoint))
                .respond_with(move |request: &wiremock::Request| {
                    let page = request.url.query_pairs()
                        .find(|(key, _)| key == cursor_param)
                        .and_then(|(_, value)| value.parse::<usize>().ok())
                        .unwrap_or(0);
                    let body = if platform == "anthropic" {
                        serde_json::json!({"data": [], "has_more": true,
                            "last_id": (page + 1).to_string()})
                    } else {
                        serde_json::json!({"models": [], "nextPageToken": (page + 1).to_string()})
                    };
                    ResponseTemplate::new(200).set_body_json(body)
                })
                .expect(MAX_MODEL_CATALOG_PAGES as u64)
                .mount(&server)
                .await;
            let error = if platform == "anthropic" {
                fetch_anthropic(&no_proxy_client(), &server.uri(), "native-key").await
            } else {
                fetch_gemini(&no_proxy_client(), &server.uri(), "native-key").await
            }.unwrap_err();
            assert!(error.to_string().contains("exceeded the pagination limit"), "{platform}");
        }
    }

    #[test]
    fn bedrock_non_anthropic_catalog_entries_remain_taskless() {
        assert_eq!(
            bedrock_tasks(
                "amazon.nova-pro-v1:0",
                Some("arn:aws:bedrock:us-east-1::foundation-model/amazon.nova-pro-v1:0"),
                Some("Amazon"),
            ),
            Vec::<ModelTask>::new()
        );
        assert_eq!(
            bedrock_tasks(
                "us.anthropic.claude-sonnet-4-20250514-v1:0",
                None,
                None,
            ),
            vec![ModelTask::Chat]
        );
    }
}
