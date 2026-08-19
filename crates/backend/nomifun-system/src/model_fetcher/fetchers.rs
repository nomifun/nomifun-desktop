use std::time::Duration;

use axum::http::StatusCode;
use nomifun_api_types::{ModelInfo, ModelTask};
use nomifun_common::AppError;
use nomifun_model_invoke::{AuthMaterial, AuthScheme};
use serde::Deserialize;
use tracing::warn;

use super::{FetchConfig, apply_catalog_auth};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Dispatch to the appropriate platform-specific fetcher.
pub(crate) async fn fetch_for_platform(
    client: &reqwest::Client,
    config: &FetchConfig,
) -> Result<Vec<ModelInfo>, AppError> {
    match config.platform.as_str() {
        "anthropic" | "claude" => {
            require_auth_scheme(config, AuthScheme::HeaderKey("x-api-key".into()))?;
            let secret = config.primary_secret()?;
            fetch_anthropic(client, &config.base_url, &secret).await
        }
        "gemini" => {
            require_auth_scheme(config, AuthScheme::HeaderKey("x-goog-api-key".into()))?;
            let secret = config.primary_secret()?;
            fetch_gemini(client, &config.base_url, &secret).await
        }
        "deepgram" => {
            require_auth_scheme(config, AuthScheme::TokenHeader)?;
            let secret = config.primary_secret()?;
            fetch_deepgram_catalog(client, &config.base_url, &secret)
                .await
                .map(|catalog| catalog.models)
        }
        "xai" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_xai(client, &config.base_url, &secret).await
        }
        // DeepSeek's live `/models` catalog is authoritative. Do not substitute
        // retired aliases when discovery is unavailable.
        "deepseek" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_openai_compatible(client, &config.base_url, &secret).await
        }
        "bedrock" => {
            require_auth_scheme(config, AuthScheme::Bedrock)?;
            fetch_bedrock(config).await
        }
        "gemini-vertex-ai" | "vertex-ai" => Err(AppError::BadRequest(
            "The legacy Vertex preset mixed Gemini model IDs with the Anthropic publisher protocol; create a provider-specific Vertex connection instead"
                .into(),
        )),
        "new-api" => {
            let secret = config.primary_secret()?;
            fetch_new_api(client, &config.base_url, &secret, &config.auth.scheme).await
        }
        "mimo" | "mimo-token-plan-cn" | "mimo-token-plan-sgp" | "mimo-token-plan-ams" => {
            Ok(mimo_models())
        }
        "stepfun" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_stepfun(client, &config.base_url, &secret).await
        }
        "minimax" => Ok(minimax_models()),
        "minimax-code" | "minimax-coding-plan" => Ok(minimax_code_models()),
        // Zhipu OpenAPI does not expose an OpenAI-compatible `GET /models`.
        "zhipu" => Ok(zhipu_models()),
        "ark-coding-plan" => Ok(ark_coding_plan_models()),
        "ark-agent-plan" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_ark_agent_plan(client, &config.base_url, &secret).await
        }
        "stepfun-plan" => Ok(stepfun_plan_models()),
        "dashscope-coding" => {
            require_auth_scheme(config, AuthScheme::Bearer)?;
            let secret = config.primary_secret()?;
            fetch_dashscope_coding(client, &config.base_url, &secret).await
        }
        "glm-coding-plan" => Ok(glm_coding_plan_models()),
        "qianfan-coding-plan" => Ok(qianfan_coding_plan_models()),
        _ => fetch_openai_compatible_with_auth(client, &config.base_url, &config.auth).await,
    }
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
    let resp = client
        .get(&url)
        .header("Authorization", format!("Token {api_key}"))
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
                    traits: Vec::new(),
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
            if let Some(model) = models.iter_mut().find(|known| known.id == item.id) {
                if !model.tasks.contains(&task) {
                    model.tasks.push(task);
                }
            } else {
                models.push(ModelInfo {
                    id: item.id,
                    name: None,
                    tasks: vec![task],
                    traits: Vec::new(),
                });
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
        traits: Vec::new(),
    });
    models.push(ModelInfo {
        id: "xai-stt".into(),
        name: Some("xAI Speech-to-Text service".into()),
        tasks: vec![ModelTask::SpeechRecognition],
        traits: Vec::new(),
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
    let url = nomifun_model_invoke::join_endpoint(base_url, "/models");
    let request = apply_catalog_auth(client.get(&url), auth)?;
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
        .map(|m| ModelInfo {
            id: m.id,
            name: None,
            tasks: Vec::new(),
            traits: Vec::new(),
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
    let url = nomifun_model_invoke::join_endpoint(base_url, "/v1/models");
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
            tasks: Vec::new(),
            traits: Vec::new(),
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
    let url = nomifun_model_invoke::join_endpoint(base_url, "/v1beta/models");
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
            ModelInfo {
                id,
                name: None,
                tasks: Vec::new(),
                traits: Vec::new(),
            }
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
        .map(|model| ModelInfo {
            id: model.model_id().to_owned(),
            name: model.model_name().map(str::to_owned),
            tasks: bedrock_tasks(
                model.model_id(),
                Some(model.model_arn()),
                model.provider_name(),
            ),
            traits: Vec::new(),
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
                    tasks,
                    traits: Vec::new(),
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
        if existing.tasks.is_empty() && !candidate.tasks.is_empty() {
            existing.tasks = candidate.tasks;
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

/// Step Plan catalog verified against the official plan documentation on
/// 2026-08-11. The router alias is plan-only; all remaining entries are also
/// part of the regular-API fallback below.
const STEPFUN_PLAN_MODELS: &[&str] = &[
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

/// Current public StepFun model baseline verified 2026-08-11. It spans chat,
/// realtime speech, audio chat, dedicated TTS/ASR, and image generation/edit.
/// The live `/v1/models` catalog remains authoritative and every model it
/// returns (including unknown future IDs) is preserved. This list is only used
/// when the official host is temporarily unavailable or returns an empty list.
///
/// Keep plan-only `step-router-v1` out of this list: it is not callable through
/// the regular `https://api.stepfun.com/v1` billing endpoint.
const STEPFUN_FALLBACK_MODELS: &[&str] = &[
    // Chat / reasoning. `step-3.7-flash` accepts vision input.
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
// dashscope-coding (official static catalog)
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
    _client: &reqwest::Client,
    _base_url: &str,
    _api_key: &str,
) -> Result<Vec<ModelInfo>, AppError> {
    // Coding Plan does not expose a reliable `/models` catalog. Listing must
    // therefore be side-effect-free: neither a 405 from `/models` nor a
    // billable synthetic chat request is an acceptable prerequisite for
    // entering a documented model ID. The task-aware health check validates
    // credentials and the selected model after it has been saved.
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
            tasks: Vec::new(),
            traits: Vec::new(),
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
        assert!(models.iter().any(|model| model.id == "MiniMax-M3"));
        assert!(models.iter().any(|model| model.id == "MiniMax-M2.7"));
        assert!(models.iter().any(|model| model.id == "MiniMax-M2.7-highspeed"));
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
        assert!(glm_coding_plan_models().iter().any(|model| model.id == "glm-5.2"));
        assert!(qianfan_coding_plan_models().iter().any(|model| model.id == "qianfan-code-latest"));
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

    #[tokio::test]
    async fn dashscope_coding_catalog_does_not_require_models_or_billable_chat_probe() {
        let models = fetch_dashscope_coding(
            &reqwest::Client::new(),
            "http://127.0.0.1:1/v1",
            "not-used-for-listing",
        )
        .await
        .unwrap();
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            DASHSCOPE_MODELS
        );
    }

    #[test]
    fn stepfun_fallback_has_public_models_but_not_plan_only_router() {
        let models = fallback_models(STEPFUN_FALLBACK_MODELS);
        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            [
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
            assert_eq!(
                traits,
                vec![
                    ModelTrait::AudioInput,
                    ModelTrait::AudioOutput,
                    ModelTrait::Realtime,
                    ModelTrait::Streaming,
                ]
            );
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

        assert_eq!(
            models.into_iter().map(|model| model.id).collect::<Vec<_>>(),
            ["step-3.7-flash", "step-future-modality-1"]
        );
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
        assert!(models.iter().any(|model| model.id == "ark-code-latest"));
        // A couple of the concrete IDs verified against the live Agent Plan endpoint.
        assert!(models.iter().any(|model| model.id == "glm-5.2"));
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
                traits: Vec::new(),
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
