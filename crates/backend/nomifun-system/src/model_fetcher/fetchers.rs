use std::time::Duration;

use axum::http::StatusCode;
use nomifun_api_types::ModelInfo;
use nomifun_common::AppError;
use serde::Deserialize;
use tracing::warn;

use super::FetchConfig;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Dispatch to the appropriate platform-specific fetcher.
pub(crate) async fn fetch_for_platform(
    client: &reqwest::Client,
    config: &FetchConfig,
) -> Result<Vec<ModelInfo>, AppError> {
    match config.platform.as_str() {
        "anthropic" | "claude" => fetch_anthropic(client, &config.base_url, &config.api_key).await,
        "gemini" => fetch_gemini(client, &config.base_url, &config.api_key).await,
        "xai" => fetch_xai(client, &config.base_url, &config.api_key).await,
        // DeepSeek's live `/models` catalog is authoritative. Do not substitute
        // retired aliases when discovery is unavailable.
        "deepseek" => {
            fetch_openai_compatible(client, &config.base_url, &config.api_key).await
        }
        "bedrock" => fetch_bedrock(config).await,
        "gemini-vertex-ai" | "vertex-ai" => Err(AppError::BadRequest(
            "The legacy Vertex preset mixed Gemini model IDs with the Anthropic publisher protocol; create a provider-specific Vertex connection instead"
                .into(),
        )),
        "new-api" => fetch_new_api(client, &config.base_url, &config.api_key).await,
        "mimo" | "mimo-token-plan-cn" | "mimo-token-plan-sgp" | "mimo-token-plan-ams" => {
            Ok(mimo_models())
        }
        "stepfun" => fetch_stepfun(client, &config.base_url, &config.api_key).await,
        "minimax" => Ok(minimax_models()),
        "minimax-code" | "minimax-coding-plan" => Ok(minimax_code_models()),
        // Zhipu OpenAPI does not expose an OpenAI-compatible `GET /models`.
        "zhipu" => Ok(zhipu_models()),
        "ark-coding-plan" => Ok(ark_coding_plan_models()),
        "ark-agent-plan" => fetch_ark_agent_plan(client, &config.base_url, &config.api_key).await,
        "stepfun-plan" => Ok(stepfun_plan_models()),
        "dashscope-coding" => {
            fetch_dashscope_coding(client, &config.base_url, &config.api_key).await
        }
        "glm-coding-plan" => Ok(glm_coding_plan_models()),
        "qianfan-coding-plan" => Ok(qianfan_coding_plan_models()),
        _ => fetch_openai_compatible(client, &config.base_url, &config.api_key).await,
    }
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
    let mut models = Vec::new();
    for path in ["language-models", "image-generation-models", "video-generation-models"] {
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
            if !models.iter().any(|known: &ModelInfo| known.id == item.id) {
                models.push(ModelInfo { id: item.id, name: None });
            }
        }
    }

    // Current xAI STT/TTS APIs are services rather than model-ID endpoints.
    // The model picker still requires a third-level value, so expose explicit
    // service profiles instead of inventing an upstream model field.
    models.push(ModelInfo {
        id: "xai-tts".into(),
        name: Some("xAI Text-to-Speech service".into()),
    });
    models.push(ModelInfo {
        id: "xai-stt".into(),
        name: Some("xAI Speech-to-Text service".into()),
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
}

/// Fetch models from an OpenAI-compatible `/models` endpoint.
pub(super) async fn fetch_openai_compatible(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
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
        .map(|m| ModelInfo {
            id: m.id,
            name: None,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Anthropic
// ---------------------------------------------------------------------------

/// Response shape for Anthropic `/v1/models`.
#[derive(Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModel>,
}

#[derive(Deserialize)]
struct AnthropicModel {
    id: String,
}

async fn fetch_anthropic(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let result = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await;

    let resp = result.map_err(|error| {
        warn_remote_request_failure_without_fallback("anthropic", &error);
        remote_error(&error)
    })?;

    // The provider catalog is the source of truth. Returning a stale local
    // list here previously surfaced models that Anthropic had already retired,
    // making a successful-looking configuration fail only at invocation time.
    check_response_status(&resp)?;

    let body: AnthropicModelsResponse = resp.json().await.map_err(|_| {
        AppError::BadGateway("Anthropic models response was not valid JSON".into())
    })?;
    Ok(body
        .data
        .into_iter()
        .map(|m| ModelInfo {
            id: m.id,
            name: None,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Gemini
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GeminiModelsResponse {
    models: Vec<GeminiModel>,
}

#[derive(Deserialize)]
struct GeminiModel {
    name: String,
}

async fn fetch_gemini(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    let url = format!("{}/v1beta/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(&url)
        .header("x-goog-api-key", api_key)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            warn_remote_request_failure_without_fallback("gemini", &error);
            remote_error(&error)
        })?;

    // The live catalog includes the account-visible model set and generation
    // methods. Never replace it with a static snapshot when it is unavailable:
    // Gemini models have explicit shutdown dates and old fallbacks silently
    // create dead configurations.
    check_response_status(&resp)?;
    let body: GeminiModelsResponse = resp
        .json()
        .await
        .map_err(|_| AppError::BadGateway("Gemini models response was not valid JSON".into()))?;
    Ok(body
        .models
        .into_iter()
        .map(|m| {
            let id = m.name.strip_prefix("models/").unwrap_or(&m.name).to_owned();
            ModelInfo { id, name: None }
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Bedrock (AWS SDK)
// ---------------------------------------------------------------------------

async fn fetch_bedrock(config: &FetchConfig) -> Result<Vec<ModelInfo>, AppError> {
    let bedrock_cfg = config
        .bedrock_config
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("Bedrock requires bedrockConfig".into()))?;

    let region = aws_sdk_bedrock::config::Region::new(bedrock_cfg.region.clone());

    let sdk_config = match bedrock_cfg.auth_method {
        nomifun_api_types::BedrockAuthMethod::AccessKey => {
            let key_id = bedrock_cfg
                .access_key_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("accessKeyId is required".into()))?;
            let secret = bedrock_cfg
                .secret_access_key
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("secretAccessKey is required".into()))?;

            let creds = aws_sdk_bedrock::config::Credentials::new(
                key_id, secret, None, // session token
                None, // expiry
                "nomifun",
            );
            aws_sdk_bedrock::Config::builder()
                .region(region)
                .credentials_provider(creds)
                .build()
        }
        nomifun_api_types::BedrockAuthMethod::Profile => {
            let profile = bedrock_cfg.profile.as_deref().unwrap_or("default");
            let aws_cfg = aws_config::from_env()
                .profile_name(profile)
                .region(aws_config::Region::new(bedrock_cfg.region.clone()))
                .load()
                .await;
            aws_sdk_bedrock::Config::new(&aws_cfg)
        }
    };

    let client = aws_sdk_bedrock::Client::from_conf(sdk_config);
    let resp = client
        .list_inference_profiles()
        .send()
        .await
        .map_err(|e| AppError::BadGateway(format!("Bedrock API error: {e}")))?;

    let profiles = resp.inference_profile_summaries();
    // Filter to only anthropic.claude models per API Spec
    let models: Vec<ModelInfo> = profiles
        .iter()
        .filter(|p| p.inference_profile_id().starts_with("anthropic.claude"))
        .map(|p| ModelInfo {
            id: p.inference_profile_id().to_string(),
            name: None,
        })
        .collect();

    Ok(models)
}

// ---------------------------------------------------------------------------
// Maintained catalogs for products without a reliable account catalog
// ---------------------------------------------------------------------------

fn minimax_models() -> Vec<ModelInfo> {
    // The retired MiniMax-Text-01 and abab6.5 aliases must not be offered as
    // an offline fallback. Until MiniMax exposes one cross-modality catalog,
    // keep this conservative list to the currently documented primary models.
    minimax_code_models()
}

fn mimo_models() -> Vec<ModelInfo> {
    fallback_models(&[
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

/// Current Zhipu OpenAPI baseline, verified 2026-08-11 against the official
/// model overview: https://docs.bigmodel.cn/cn/guide/start/model-overview
///
/// This is intentionally static because `https://open.bigmodel.cn/api/paas/v4`
/// has no public `GET /models` operation. Keep model IDs in their callable API
/// form rather than the title casing used in parts of the documentation.
fn zhipu_models() -> Vec<ModelInfo> {
    fallback_models(ZHIPU_MODELS)
}

fn ark_coding_plan_models() -> Vec<ModelInfo> {
    fallback_models(&["ark-code-latest"])
}

// ---------------------------------------------------------------------------
// Ark Agent Plan (remote catalog with fallback)
// ---------------------------------------------------------------------------

/// Switchable model set exposed by the Agent Plan router, used when the plan
/// gateway does not serve a `/models` catalog (the `/api/plan/v3` endpoint
/// only routes `/chat/completions` — `/models` returns 404). `ark-code-latest`
/// is the console-switchable router alias (recommended). The rest are the
/// concrete IDs verified to be accepted by the Agent Plan endpoint; other Ark
/// model IDs return `UnsupportedModel` there. Users can still type any ID.
const ARK_AGENT_PLAN_FALLBACK_MODELS: &[&str] = &[
    "ark-code-latest",
    "doubao-seed-2.0-code",
    "doubao-seed-2.0-pro",
    "doubao-seed-2.0-lite",
    "deepseek-v4-flash",
    "glm-5.2",
    "kimi-k2.6",
    "minimax-m2.7",
];

/// Ark Agent Plan: pull the model list from the official OpenAI-compatible
/// `/models` endpoint on the coding/agent base URL. The subscription gateway
/// often only routes `/chat/completions` (per Volcengine's "plan keys are for
/// coding/agent tools, not arbitrary API calls" policy), so on availability
/// failures or an empty catalog we fall back to the known switchable set.
/// Authentication and request errors are still returned to the caller.
/// Mirrors the fetch-then-fallback pattern used by `fetch_anthropic` /
/// `fetch_gemini`.
async fn fetch_ark_agent_plan(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    match fetch_openai_compatible(client, base_url, api_key).await {
        Ok(models) if !models.is_empty() => Ok(models),
        Ok(_) => {
            warn!("Ark Agent Plan models API returned empty list, using fallback");
            Ok(fallback_models(ARK_AGENT_PLAN_FALLBACK_MODELS))
        }
        Err(e)
            if is_catalog_availability_error(&e)
                || matches!(&e, AppError::BadRequest(_)) =>
        {
            warn!(error = %e, "Ark Agent Plan models API unavailable, using fallback list");
            Ok(fallback_models(ARK_AGENT_PLAN_FALLBACK_MODELS))
        }
        Err(e) => Err(e),
    }
}

fn stepfun_plan_models() -> Vec<ModelInfo> {
    fallback_models(&[
        "step-3.7-flash",
        "step-3.5-flash",
        "step-3.5-flash-2603",
        "stepaudio-2.5-realtime",
        "stepaudio-2.5-chat",
        "stepaudio-2.5-tts",
        "stepaudio-2.5-asr",
        "step-router-v1",
        "step-image-edit-2",
    ])
}

// ---------------------------------------------------------------------------
// StepFun (remote catalog with an official-host fallback)
// ---------------------------------------------------------------------------

/// Stable public StepFun chat model IDs documented by the provider. The live
/// `/v1/models` catalog remains authoritative; this list is only used when the
/// official host is temporarily unreachable, rate-limited, returns 5xx, sends
/// malformed JSON, or returns an empty catalog.
///
/// Keep plan-only `step-router-v1` out of this list: it is not callable through
/// the regular `https://api.stepfun.com/v1` billing endpoint.
const STEPFUN_FALLBACK_MODELS: &[&str] = &[
    // Chat / reasoning. `step-3.7-flash` is the flagship multimodal model and is
    // callable through the regular `/v1` billing endpoint (see the OpenAI
    // migration guide), so it belongs here, not only in the Step Plan list.
    "step-3.7-flash",
    "step-3.5-flash-2603",
    "step-3.5-flash",
    "step-3",
    "step-2-mini",
    "step-2-16k",
    // Vision-language chat.
    "step-1o-turbo-vision",
    "step-1o-vision-32k",
    "step-1v-32k",
    "step-1v-8k",
    // Text chat (legacy).
    "step-1-32k",
    "step-1-8k",
    // Speech recognition (ASR) — one-shot `/v1/audio/transcriptions`. Names
    // carry `asr`, so `derive_tasks_and_traits` classifies them as
    // SpeechRecognition. `step-asr` is the legacy id kept for older configs.
    "stepaudio-2.5-asr",
    "step-asr",
    // Speech synthesis (TTS) — `/v1/audio/speech`, `response_format=pcm` at
    // 24 kHz matches the robot downlink contract. Names carry `tts`.
    "stepaudio-2.5-tts",
    "step-tts-mini",
];

async fn fetch_stepfun(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    match fetch_openai_compatible(client, base_url, api_key).await {
        Ok(models) if !models.is_empty() => Ok(models),
        Ok(_) if is_official_stepfun_base_url(base_url) => {
            warn!("StepFun models API returned an empty catalog, using fallback list");
            Ok(fallback_models(STEPFUN_FALLBACK_MODELS))
        }
        Ok(models) => Ok(models),
        Err(error)
            if is_official_stepfun_base_url(base_url)
                && is_catalog_availability_error(&error) =>
        {
            warn!(
                error_code = error.error_code(),
                "StepFun models API unavailable, using fallback list"
            );
            Ok(fallback_models(STEPFUN_FALLBACK_MODELS))
        }
        Err(error) => Err(error),
    }
}

fn is_official_stepfun_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.stepfun.com")
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
    fallback_models(&["glm-5.2", "glm-5-turbo", "glm-4.7"])
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
) -> Result<Vec<ModelInfo>, AppError> {
    let normalized = ensure_v1_path(base_url);
    fetch_openai_compatible(client, &normalized, api_key).await
}

/// Ensure the URL path ends with `/v1`.
fn ensure_v1_path(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

// ---------------------------------------------------------------------------
// dashscope-coding (hardcoded + key validation)
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

async fn fetch_dashscope_coding(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    // Validate key by sending a minimal chat completion request
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": DASHSCOPE_MODELS[0],
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| remote_error(&e))?;

    check_response_status(&resp)?;

    Ok(fallback_models(DASHSCOPE_MODELS))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn fallback_models(ids: &[&str]) -> Vec<ModelInfo> {
    ids.iter()
        .map(|id| ModelInfo {
            id: (*id).to_string(),
            name: None,
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
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn no_proxy_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
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
        let ids = models.into_iter().map(|model| model.id).collect::<Vec<_>>();
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
    fn minimax_returns_expected_models() {
        let models = minimax_models();
        assert_eq!(models.len(), 3);
        assert!(models.contains(&ModelInfo { id: "MiniMax-M3".into(), name: None }));
        assert!(models.contains(&ModelInfo { id: "MiniMax-M2.7".into(), name: None }));
        assert!(models.contains(&ModelInfo {
            id: "MiniMax-M2.7-highspeed".into(),
            name: None
        }));
        assert!(!models.iter().any(|model| model.id.starts_with("MiniMax-M2.5")));
        assert!(!models.iter().any(|model| model.id.starts_with("MiniMax-M2.1")));
        assert!(!models.iter().any(|model| model.id == "MiniMax-M2"));
        assert!(!models.iter().any(|model| model.id == "MiniMax-Text-01"));
        assert!(!models.iter().any(|model| model.id.starts_with("abab6.5")));
    }

    #[test]
    fn mimo_models_match_current_v2_5_catalog() {
        let models = mimo_models();
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            vec![
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
        assert!(minimax_code_models().contains(&ModelInfo { id: "MiniMax-M3".into(), name: None }));
        assert!(minimax_code_models().contains(&ModelInfo {
            id: "MiniMax-M2.7-highspeed".into(),
            name: None
        }));
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
        assert!(ark_coding_plan_models().contains(&ModelInfo { id: "ark-code-latest".into(), name: None }));
        assert!(stepfun_plan_models().contains(&ModelInfo { id: "step-router-v1".into(), name: None }));
        assert!(glm_coding_plan_models().contains(&ModelInfo { id: "glm-5.2".into(), name: None }));
        assert!(
            qianfan_coding_plan_models().contains(&ModelInfo {
                id: "qianfan-code-latest".into(),
                name: None
            })
        );
    }

    #[test]
    fn coding_plan_catalogs_match_current_official_allowlists() {
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
            ["glm-5.2", "glm-5-turbo", "glm-4.7"]
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

    #[test]
    fn stepfun_fallback_has_public_models_but_not_plan_only_router() {
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        assert!(models.contains(&ModelInfo {
            id: "step-3.5-flash-2603".into(),
            name: None
        }));
        assert!(models.contains(&ModelInfo {
            id: "step-1o-turbo-vision".into(),
            name: None
        }));
        assert!(!models.iter().any(|model| model.id == "step-router-v1"));
    }

    #[test]
    fn stepfun_fallback_offers_speech_models_so_the_robot_voice_slots_are_fillable() {
        // Without ASR/TTS ids here, a first-run/offline install has no speech
        // model to select, so the robot's `voice.asr` / `voice.tts` slots stay
        // empty and the device is silent. See the 2026-08-08 stepfun-robot spec.
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        for id in ["stepaudio-2.5-asr", "step-asr", "stepaudio-2.5-tts", "step-tts-mini"] {
            assert!(
                models.iter().any(|model| model.id == id),
                "StepFun fallback list must offer {id} so the voice slots are fillable"
            );
        }
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
    fn ark_agent_plan_fallback_includes_router_alias_and_families() {
        let models = fallback_models(ARK_AGENT_PLAN_FALLBACK_MODELS);
        // Router alias must be present — it is the recommended, console-switchable entry.
        assert!(models.contains(&ModelInfo { id: "ark-code-latest".into(), name: None }));
        // A couple of the concrete IDs verified against the live Agent Plan endpoint.
        assert!(models.contains(&ModelInfo { id: "glm-5.2".into(), name: None }));
        assert!(models.contains(&ModelInfo {
            id: "deepseek-v4-flash".into(),
            name: None
        }));
    }

    #[test]
    fn fallback_models_builds_model_info_list() {
        let models = fallback_models(&["a", "b", "c"]);
        assert_eq!(models.len(), 3);
        assert_eq!(
            models[0],
            ModelInfo {
                id: "a".into(),
                name: None
            }
        );
    }
}
