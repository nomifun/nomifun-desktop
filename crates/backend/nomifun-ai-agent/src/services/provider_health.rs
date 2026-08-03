use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nomi_agent::bootstrap::AgentBootstrap;
use nomi_agent::engine::AgentEngine;
use nomi_agent::output::OutputSink;
use nomi_agent::output::null_sink::NullSink;
use nomi_config::config::{CliArgs, Config};
use nomifun_api_types::{
    HealthStatus, ModelHealthStatus, ModelTask, ProviderHealthCheckErrorKind,
    ProviderHealthCheckRequest, ProviderHealthCheckResponse,
};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::{IProviderModelRepository, IProviderRepository, models::Provider};
use nomifun_model_invoke::{ModelInvokeService, ModelRef};
use regex::Regex;
use tracing::{info, warn};

use crate::factory::nomi::{
    map_nomi_provider, resolve_bedrock_config, resolve_nomi_url_and_compat,
};
use crate::types::NomiResolvedConfig;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const OPENAI_MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com";
const HEALTH_CHECK_PROMPT: &str = "Reply with exactly OK.";
const HEALTH_CHECK_MSG_ID: &str = "provider-health-check";

pub struct ProviderHealthCheckService {
    provider_repo: Arc<dyn IProviderRepository>,
    provider_model_repo: Arc<dyn IProviderModelRepository>,
    encryption_key: [u8; 32],
    data_dir: PathBuf,
    /// Unified invoke layer: non-chat modality probes ride
    /// [`ModelInvokeService::probe`] (chat stays on the agent engine).
    invoke: Arc<ModelInvokeService>,
}

impl ProviderHealthCheckService {
    pub fn new(
        provider_repo: Arc<dyn IProviderRepository>,
        provider_model_repo: Arc<dyn IProviderModelRepository>,
        encryption_key: [u8; 32],
        data_dir: PathBuf,
        invoke: Arc<ModelInvokeService>,
    ) -> Self {
        Self {
            provider_repo,
            provider_model_repo,
            encryption_key,
            data_dir,
            invoke,
        }
    }

    pub async fn health_check(
        &self,
        req: ProviderHealthCheckRequest,
    ) -> Result<ProviderHealthCheckResponse, AppError> {
        let provider_id = ProviderId::parse(req.provider_id.clone())
            .map_err(|error| AppError::BadRequest(format!("invalid provider_id: {error}")))?;
        if req.model.trim().is_empty() {
            return Err(AppError::BadRequest("model is required".into()));
        }

        let provider_id = provider_id.as_str();
        let model = req.model.trim();
        let row = self
            .provider_repo
            .find_by_id(provider_id)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to load provider config: {e}")))?
            .ok_or_else(|| AppError::BadRequest(format!("Provider '{provider_id}' not found")))?;
        ProviderId::parse(&row.provider_id).map_err(|error| {
            AppError::Internal(format!(
                "stored providers.provider_id '{}' is not canonical: {error}",
                row.provider_id
            ))
        })?;
        let persisted_provider_id = row.provider_id.clone();

        // Determine which task to probe. Authority order: explicit request >
        // stored profile primary task > name/platform heuristic > Chat. This is
        // what makes image/tts/asr models probe their correct endpoint instead
        // of always hitting /chat/completions (the StepFun 404 root cause).
        let profile = self.provider_model_repo.get(provider_id, model).await.ok().flatten();
        let task = req
            .task
            .or_else(|| {
                profile
                    .as_ref()
                    .and_then(|p| serde_json::from_str::<Vec<ModelTask>>(&p.tasks).ok())
                    .and_then(|tasks| tasks.first().copied())
            })
            .unwrap_or_else(|| {
                nomifun_api_types::derive_tasks_and_traits(&row.platform, model)
                    .0
                    .first()
                    .copied()
                    .unwrap_or(ModelTask::Chat)
            });

        if task == ModelTask::Chat {
            let protocol = profile.as_ref().and_then(|p| p.protocol.as_deref());
            let config = self.resolve_probe_config(&row, model, protocol)?;
            let response = if should_use_openai_model_probe(&row.platform, &config) {
                run_openai_model_probe(
                    persisted_provider_id,
                    row.platform,
                    model.to_owned(),
                    config.api_key,
                    config.base_url,
                )
                .await?
            } else {
                run_probe(persisted_provider_id, row.platform, config).await?
            };
            persist_probe_outcome(self.provider_model_repo.as_ref(), &response).await;
            return Ok(response);
        }

        // Non-chat task: probe the task's real endpoint through the unified
        // invoke layer (resolution, minimal request and the 60 s cap live in
        // `ModelInvokeService::probe`).
        info!(
            provider_id = %persisted_provider_id,
            platform = %row.platform,
            model = %model,
            task = ?task,
            "Modality health check started"
        );
        let model_ref =
            ModelRef { provider_id: persisted_provider_id.clone(), model: model.to_owned() };
        let response = match self.invoke.probe(&model_ref, task).await {
            Ok(report) if report.healthy => healthy_response(
                persisted_provider_id,
                row.platform,
                model.to_owned(),
                Duration::from_millis(report.latency_ms),
            ),
            Ok(report) => {
                let message =
                    report.message.unwrap_or_else(|| "modality probe failed".to_owned());
                // The invoke probe folds its own timeout into this message;
                // surface it as the legacy timeout_stage so classify_error
                // keeps reporting Timeout.
                let timeout_stage =
                    message.contains("modality probe timeout").then(|| "modality_probe".to_owned());
                unhealthy_response(
                    persisted_provider_id,
                    row.platform,
                    model.to_owned(),
                    Duration::from_millis(report.latency_ms),
                    message,
                    timeout_stage,
                )
            }
            // probe() only errs before touching the catalog or the wire (its
            // chat guard — unreachable here since task != Chat). The endpoint's
            // wire contract stays "200 + unhealthy", never an HTTP error, and
            // that also keeps disabled provider/model rows probeable from the
            // UI (they fold into an unhealthy report inside probe()).
            Err(error) => unhealthy_response(
                persisted_provider_id,
                row.platform,
                model.to_owned(),
                Duration::ZERO,
                error.to_string(),
                None,
            ),
        };
        log_health_check_result(&response);
        persist_probe_outcome(self.provider_model_repo.as_ref(), &response).await;
        Ok(response)
    }

    fn resolve_probe_config(
        &self,
        row: &Provider,
        model_id: &str,
        protocol: Option<&str>,
    ) -> Result<NomiResolvedConfig, AppError> {
        let api_key = nomifun_common::decrypt_string(&row.api_key_encrypted, &self.encryption_key)?;
        let provider = map_nomi_provider(&row.platform, protocol);
        let (base_url, compat_overrides) =
            resolve_nomi_url_and_compat(&row.platform, &row.base_url, &provider, row.is_full_url);
        let bedrock_config = if row.platform == "bedrock" {
            resolve_bedrock_config(row.bedrock_config.as_deref())
        } else {
            None
        };

        Ok(NomiResolvedConfig {
            provider,
            api_key,
            model: model_id.to_owned(),
            base_url,
            system_prompt: Some(
                "You are a provider health probe. Reply with exactly OK and do not use tools."
                    .into(),
            ),
            max_tokens: 16,
            max_turns: Some(1),
            context_limit: None,
            compat_overrides,
            session_directory: self.data_dir.join("nomi-health-check-sessions"),
            session_mode: None,
            extra_mcp_servers: HashMap::new(),
            loopback_capability_leases: Default::default(),
            bedrock_config,
            computer_use: false,
            browser_use: false,
            browser_source: "managed".to_owned(),
            browser_full_power: false,
            browser_persistent_login: false,
            browser_site_memory: false,
            browser_takeover: false,
            browser_unrestricted_approval: false,
            browser_visual_fallback: false,
            goal: None,
            persistent_login_key: None,
            owner_token: None,
            // 健康探针一回合、不用工具：不安装 embedded AgentExecution。
            install_embedded_agent_execution: false,
            allowed_tools: Vec::new(),
            write_root: None,
})
    }
}

/// Persist one probe outcome onto the model's authoritative catalog row.
///
/// Serializes the wire [`ModelHealthStatus`] shape (the same struct Task 4's
/// row→response projection parses back out of `provider_models.health`) and
/// writes it via `set_health`, which also stamps `health_checked_at = now`.
/// Best-effort by design: a persistence failure (or a probe for a model that
/// has no catalog row) logs a warning and never fails the health request.
pub(crate) async fn persist_probe_outcome(
    repo: &dyn IProviderModelRepository,
    response: &ProviderHealthCheckResponse,
) {
    let health = ModelHealthStatus {
        status: response.status,
        // `health_checked_at` is authoritative for observation time; keep the
        // wire struct's own `last_check` mirror populated for UI parity.
        last_check: Some(nomifun_common::now_ms()),
        latency: Some(i64::try_from(response.elapsed_ms).unwrap_or(i64::MAX)),
        error: response.message.clone(),
    };
    let json = match serde_json::to_string(&health) {
        Ok(json) => json,
        Err(error) => {
            warn!(
                provider_id = %response.provider_id,
                model = %response.model,
                %error,
                "could not serialize probe outcome; skipping health write-back"
            );
            return;
        }
    };
    match repo
        .set_health(&response.provider_id, &response.model, Some(&json))
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!(
            provider_id = %response.provider_id,
            model = %response.model,
            "probe outcome not persisted: model has no provider_models row"
        ),
        Err(error) => warn!(
            provider_id = %response.provider_id,
            model = %response.model,
            %error,
            "probe outcome health write-back failed"
        ),
    }
}

fn should_use_openai_model_probe(_platform: &str, config: &NomiResolvedConfig) -> bool {
    config.provider == "openai"
        && config
            .base_url
            .as_deref()
            .map(is_official_openai_base_url)
            .unwrap_or(true)
}

fn is_official_openai_base_url(base_url: &str) -> bool {
    let lower = base_url.trim().to_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    without_scheme == "api.openai.com" || without_scheme.starts_with("api.openai.com/")
}

async fn run_openai_model_probe(
    provider_id: String,
    platform: String,
    model: String,
    api_key: String,
    base_url: Option<String>,
) -> Result<ProviderHealthCheckResponse, AppError> {
    let started = Instant::now();
    let url = openai_model_probe_url(base_url.as_deref(), &model);
    let client = nomifun_net::http_client();

    info!(
        provider_id = %provider_id,
        platform = %platform,
        model = %model,
        "OpenAI model health check started"
    );

    match tokio::time::timeout(
        OPENAI_MODEL_PROBE_TIMEOUT,
        client.get(&url).bearer_auth(api_key).send(),
    )
    .await
    {
        Ok(Ok(response)) if response.status().is_success() => {
            let response = ProviderHealthCheckResponse {
                provider_id,
                platform,
                model,
                status: HealthStatus::Healthy,
                elapsed_ms: elapsed_ms(started.elapsed()),
                message: None,
                error_kind: None,
                http_status: None,
                timeout_stage: None,
            };
            log_health_check_result(&response);
            Ok(response)
        }
        Ok(Ok(response)) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let message = format!("OpenAI model probe API error {}: {body}", status.as_u16());
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                message,
                None,
            );
            log_health_check_result(&response);
            Ok(response)
        }
        Ok(Err(error)) => {
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                format!("OpenAI model probe HTTP error: {error}"),
                None,
            );
            log_health_check_result(&response);
            Ok(response)
        }
        Err(_) => {
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                format!(
                    "OpenAI model probe timeout ({}s)",
                    OPENAI_MODEL_PROBE_TIMEOUT.as_secs()
                ),
                Some("openai_models".into()),
            );
            log_health_check_result(&response);
            Ok(response)
        }
    }
}

fn openai_model_probe_url(base_url: Option<&str>, model: &str) -> String {
    let base = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPENAI_BASE_URL)
        .trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/v1/models/{model}")
}

async fn run_probe(
    provider_id: String,
    platform: String,
    config_extra: NomiResolvedConfig,
) -> Result<ProviderHealthCheckResponse, AppError> {
    let started = Instant::now();
    let model = config_extra.model.clone();

    info!(
        provider_id = %provider_id,
        platform = %platform,
        model = %model,
        "Provider health check started"
    );

    let mut engine = match build_probe_engine(config_extra).await {
        Ok(engine) => engine,
        Err(error) => {
            let message = format!("Nomi probe bootstrap failed: {error}");
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                message,
                None,
            );
            log_health_check_result(&response);
            return Ok(response);
        }
    };

    match tokio::time::timeout(
        HEALTH_CHECK_TIMEOUT,
        engine.execute_turn(HEALTH_CHECK_PROMPT, HEALTH_CHECK_MSG_ID),
    )
    .await
    {
        Ok(Ok(_)) => {
            let response = ProviderHealthCheckResponse {
                provider_id,
                platform,
                model,
                status: HealthStatus::Healthy,
                elapsed_ms: elapsed_ms(started.elapsed()),
                message: None,
                error_kind: None,
                http_status: None,
                timeout_stage: None,
            };
            log_health_check_result(&response);
            Ok(response)
        }
        Ok(Err(error)) => {
            let message = error.to_string();
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                message,
                None,
            );
            log_health_check_result(&response);
            Ok(response)
        }
        Err(_) => {
            let response = unhealthy_response(
                provider_id,
                platform,
                model,
                started.elapsed(),
                format!("Health check timeout ({}s)", HEALTH_CHECK_TIMEOUT.as_secs()),
                Some("engine_run".into()),
            );
            log_health_check_result(&response);
            Ok(response)
        }
    }
}

/// Probe a non-chat model at its correct endpoint: since the P1 invoke
/// redesign this rides [`ModelInvokeService::probe`] (see the non-chat branch
/// of [`ProviderHealthCheckService::health_check`]); the minimal request
/// bodies, the multipart missing-file tolerance and the 60 s cap live there.
fn healthy_response(
    provider_id: String,
    platform: String,
    model: String,
    elapsed: Duration,
) -> ProviderHealthCheckResponse {
    ProviderHealthCheckResponse {
        provider_id,
        platform,
        model,
        status: HealthStatus::Healthy,
        elapsed_ms: elapsed_ms(elapsed),
        message: None,
        error_kind: None,
        http_status: None,
        timeout_stage: None,
    }
}

fn log_health_check_result(response: &ProviderHealthCheckResponse) {
    match response.status {
        HealthStatus::Healthy => info!(
            provider_id = %response.provider_id,
            platform = %response.platform,
            model = %response.model,
            elapsed_ms = response.elapsed_ms,
            "Provider health check succeeded"
        ),
        HealthStatus::Unhealthy | HealthStatus::Unknown => warn!(
            provider_id = %response.provider_id,
            platform = %response.platform,
            model = %response.model,
            elapsed_ms = response.elapsed_ms,
            error_kind = ?response.error_kind,
            http_status = ?response.http_status,
            timeout_stage = ?response.timeout_stage,
            "Provider health check failed"
        ),
    }
}

async fn build_probe_engine(config_extra: NomiResolvedConfig) -> Result<AgentEngine, AppError> {
    let workspace = config_extra
        .session_directory
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sink: Arc<dyn OutputSink> = Arc::new(NullSink);
    let cli_args = CliArgs {
        provider: Some(config_extra.provider),
        api_key: Some(config_extra.api_key),
        base_url: config_extra.base_url,
        model: Some(config_extra.model),
        max_tokens: Some(config_extra.max_tokens),
        max_turns: config_extra.max_turns,
        system_prompt: config_extra.system_prompt,
        profile: None,
        auto_approve: false,
        project_dir: Some(PathBuf::from(&workspace)),
    };
    let mut config = Config::resolve(&cli_args)
        .map_err(|error| AppError::Internal(format!("Config resolve failed: {error}")))?;

    config.bedrock = config_extra.bedrock_config;
    config.session.enabled = false;
    config.mcp.servers.clear();
    config.file_cache.enabled = false;
    if let Some(field) = config_extra.compat_overrides.max_tokens_field {
        config.compat.max_tokens_field = Some(field);
    }
    if let Some(path) = config_extra.compat_overrides.api_path {
        config.compat.api_path = Some(path);
    }
    if let Some(required) = config_extra.compat_overrides.require_reasoning_content {
        config.compat.require_reasoning_content = Some(required);
    }

    let mut result = AgentBootstrap::new(config, workspace, sink)
        .install_embedded_agent_execution(config_extra.install_embedded_agent_execution)
        .build()
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    result.engine.registry_mut().clear();
    Ok(result.engine)
}

fn unhealthy_response(
    provider_id: String,
    platform: String,
    model: String,
    elapsed: Duration,
    message: String,
    timeout_stage: Option<String>,
) -> ProviderHealthCheckResponse {
    let error_kind = classify_error(&message, timeout_stage.is_some());
    let http_status = extract_http_status(&message);
    ProviderHealthCheckResponse {
        provider_id,
        platform,
        model,
        status: HealthStatus::Unhealthy,
        elapsed_ms: elapsed_ms(elapsed),
        message: Some(message),
        error_kind: Some(error_kind),
        http_status,
        timeout_stage,
    }
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn classify_error(message: &str, is_timeout: bool) -> ProviderHealthCheckErrorKind {
    if is_timeout {
        return ProviderHealthCheckErrorKind::Timeout;
    }

    let lower = message.to_lowercase();
    // Upstream statuses arrive in two shapes: the engine/legacy "api error NNN"
    // and the invoke layer's "provider returned NNN <reason>".
    let mentions_status = |code: u16| {
        lower.contains(&format!("api error {code}")) || lower.contains(&format!("provider returned {code}"))
    };
    if lower.contains("invalid authorization header") || lower.contains("invalid x-api-key header")
    {
        return ProviderHealthCheckErrorKind::InvalidAuthorizationHeader;
    }
    if lower.contains("rate limited") || lower.contains(" 429") || mentions_status(429) {
        return ProviderHealthCheckErrorKind::RateLimited;
    }
    if lower.contains("insufficient_quota")
        || lower.contains("insufficient quota")
        || lower.contains("credit balance is too low")
        || lower.contains("billing")
    {
        return ProviderHealthCheckErrorKind::InsufficientQuota;
    }
    if lower.contains("aws credential")
        || lower.contains("loading credentials")
        || lower.contains("invalid refresh token")
        || lower.contains("session token not found")
    {
        return ProviderHealthCheckErrorKind::AwsCredentials;
    }
    if mentions_status(401)
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
    {
        return ProviderHealthCheckErrorKind::Unauthorized;
    }
    if mentions_status(403) || lower.contains("forbidden") {
        return ProviderHealthCheckErrorKind::Forbidden;
    }
    if mentions_status(404) || lower.contains("not found") {
        return ProviderHealthCheckErrorKind::NotFound;
    }
    if mentions_status(400)
        || mentions_status(422)
        || lower.contains("invalid_request")
        || lower.contains("invalid request")
    {
        return ProviderHealthCheckErrorKind::InvalidRequest;
    }
    if lower.contains("connection error")
        || lower.contains("http error")
        // InvokeError transport shapes: "Network: request failed: …" /
        // "Timeout: request timed out: …" (the 60 s probe cap rides
        // timeout_stage instead and never reaches this arm).
        || lower.contains("request failed")
        || lower.contains("request timed out")
    {
        return ProviderHealthCheckErrorKind::ConnectionError;
    }
    if lower.contains("api error") || lower.contains("provider error") || lower.contains("provider returned") {
        return ProviderHealthCheckErrorKind::ApiError;
    }

    ProviderHealthCheckErrorKind::Unknown
}

pub(crate) fn extract_http_status(message: &str) -> Option<u16> {
    let re = Regex::new(r"(?i)(?:api error|provider returned)\s+(\d{3})").ok()?;
    re.captures(message)
        .and_then(|captures| captures.get(1))
        .and_then(|matched| matched.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chat_probe_config(session_directory: PathBuf) -> NomiResolvedConfig {
        NomiResolvedConfig {
            provider: "openai".to_owned(),
            api_key: "sk-test".to_owned(),
            model: "gpt-test".to_owned(),
            base_url: Some("https://api.openai.com".to_owned()),
            system_prompt: Some("Reply with exactly OK.".to_owned()),
            max_tokens: 16,
            max_turns: Some(1),
            context_limit: None,
            compat_overrides: crate::types::NomiCompatOverrides::default(),
            session_directory,
            session_mode: None,
            extra_mcp_servers: HashMap::new(),
            loopback_capability_leases: Default::default(),
            bedrock_config: None,
            computer_use: false,
            browser_use: false,
            browser_source: "managed".to_owned(),
            browser_full_power: false,
            browser_persistent_login: false,
            browser_site_memory: false,
            browser_takeover: false,
            browser_unrestricted_approval: false,
            browser_visual_fallback: false,
            goal: None,
            persistent_login_key: None,
            owner_token: None,
            install_embedded_agent_execution: false,
            allowed_tools: Vec::new(),
            write_root: None,
        }
    }

    #[tokio::test]
    async fn chat_health_probe_engine_has_no_tools() {
        let temp = tempfile::tempdir().unwrap();
        let engine = build_probe_engine(test_chat_probe_config(temp.path().join("sessions")))
            .await
            .unwrap();
        assert!(engine.tool_names().is_empty());
    }

    #[test]
    fn classify_error_detects_quota_message() {
        let message = r#"Provider error: API error 400: {"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low"}}"#;
        assert_eq!(
            classify_error(message, false),
            ProviderHealthCheckErrorKind::InsufficientQuota
        );
        assert_eq!(extract_http_status(message), Some(400));
    }

    #[test]
    fn classify_error_detects_invalid_header() {
        assert_eq!(
            classify_error(
                "Connection error: Invalid authorization header: invalid header value",
                false
            ),
            ProviderHealthCheckErrorKind::InvalidAuthorizationHeader
        );
    }

    #[test]
    fn classify_error_detects_aws_credentials() {
        assert_eq!(
            classify_error(
                "Provider error: Connection error: AWS credential error: an error occurred while loading credentials",
                false
            ),
            ProviderHealthCheckErrorKind::AwsCredentials
        );
        assert_eq!(
            classify_error(
                "service error: UnauthorizedException: Session token not found or invalid",
                false
            ),
            ProviderHealthCheckErrorKind::AwsCredentials
        );
    }

    #[test]
    fn classify_error_detects_timeout() {
        assert_eq!(
            classify_error("Health check timeout (30s)", true),
            ProviderHealthCheckErrorKind::Timeout
        );
    }

    #[test]
    fn openai_model_probe_is_used_for_custom_openai_compatible_configs() {
        let config = NomiResolvedConfig {
            provider: "openai".to_owned(),
            api_key: "sk-test".to_owned(),
            model: "gpt-test".to_owned(),
            base_url: Some("https://api.openai.com".to_owned()),
            system_prompt: None,
            max_tokens: 16,
            max_turns: Some(1),
            context_limit: None,
            compat_overrides: crate::types::NomiCompatOverrides::default(),
            session_directory: PathBuf::from("/tmp/nomi-health"),
            session_mode: None,
            extra_mcp_servers: HashMap::new(),
            loopback_capability_leases: Default::default(),
            bedrock_config: None,
            computer_use: false,
            browser_use: false,
            browser_source: "managed".to_owned(),
            browser_full_power: false,
            browser_persistent_login: false,
            browser_site_memory: false,
            browser_takeover: false,
            browser_unrestricted_approval: false,
            browser_visual_fallback: false,
            goal: None,
            persistent_login_key: None,
            owner_token: None,
            install_embedded_agent_execution: false,
            allowed_tools: Vec::new(),
            write_root: None,
};

        assert!(should_use_openai_model_probe("custom", &config));
    }

    #[tokio::test]
    async fn openai_model_probe_uses_models_endpoint_for_success() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models/gpt-test"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "gpt-test",
                "object": "model"
            })))
            .mount(&server)
            .await;

        let response = run_openai_model_probe(
            "provider-1".to_owned(),
            "openai".to_owned(),
            "gpt-test".to_owned(),
            "sk-test".to_owned(),
            Some(server.uri()),
        )
        .await
        .unwrap();

        assert_eq!(response.status, HealthStatus::Healthy);
        assert_eq!(response.error_kind, None);
    }

    #[tokio::test]
    async fn openai_model_probe_preserves_rate_limit_classification() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models/gpt-test"))
            .respond_with(ResponseTemplate::new(429).set_body_string("Too Many Requests"))
            .mount(&server)
            .await;

        let response = run_openai_model_probe(
            "provider-1".to_owned(),
            "openai".to_owned(),
            "gpt-test".to_owned(),
            "sk-test".to_owned(),
            Some(server.uri()),
        )
        .await
        .unwrap();

        assert_eq!(response.status, HealthStatus::Unhealthy);
        assert_eq!(
            response.error_kind,
            Some(ProviderHealthCheckErrorKind::RateLimited)
        );
        assert_eq!(response.http_status, Some(429));
    }

    // -- modality probes through the invoke layer (health_check non-chat path) --

    const TEST_KEY: [u8; 32] = [0x42; 32];

    /// Real in-memory catalog + production invoke adapters behind the health
    /// service — the ported equivalent of the old `run_modality_probe` tests,
    /// now exercising the full `health_check` → `invoke.probe` path.
    async fn setup_health_service() -> (
        ProviderHealthCheckService,
        nomifun_db::SqlitePool,
        tempfile::TempDir,
    ) {
        let db = nomifun_db::init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        let temp = tempfile::tempdir().unwrap();
        let invoke = Arc::new(ModelInvokeService::new(
            Arc::new(nomifun_db::SqliteProviderRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            nomifun_model_invoke::AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
        ));
        let service = ProviderHealthCheckService::new(
            Arc::new(nomifun_db::SqliteProviderRepository::new(pool.clone())),
            Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone())),
            TEST_KEY,
            temp.path().to_path_buf(),
            invoke,
        );
        (service, pool, temp)
    }

    /// Seed an enabled provider (key decrypts to `sk-test`) + one model row.
    async fn seed_catalog(
        pool: &nomifun_db::SqlitePool,
        platform: &str,
        base_url: &str,
        model: &str,
        tasks: &str,
        model_enabled: bool,
    ) -> String {
        use nomifun_db::{
            CreateProviderParams, IProviderModelRepository, IProviderRepository, NewProviderModel,
        };
        let encrypted = nomifun_common::encrypt_string("sk-test", &TEST_KEY).unwrap();
        let pid = nomifun_db::SqliteProviderRepository::new(pool.clone())
            .create(CreateProviderParams {
                provider_id: None,
                platform,
                name: "Wiremock Provider",
                base_url,
                api_key_encrypted: &encrypted,
                models: "[]",
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
            .unwrap()
            .provider_id;
        nomifun_db::SqliteProviderModelRepository::new(pool.clone())
            .create(
                &pid,
                &NewProviderModel {
                    model,
                    enabled: model_enabled,
                    sort_order: 0,
                    tasks,
                    traits: "[]",
                    params: "{}",
                    source: "user",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        pid
    }

    #[tokio::test]
    async fn modality_probe_image_generation_success_is_healthy() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{ "b64_json": "AAAA" }]
            })))
            .mount(&server)
            .await;

        let (service, pool, _temp) = setup_health_service().await;
        let pid = seed_catalog(
            &pool,
            "stepfun-plan",
            &server.uri(),
            "step-image-edit-2",
            r#"["image_generation"]"#,
            true,
        )
        .await;

        let response = service
            .health_check(ProviderHealthCheckRequest {
                provider_id: pid.clone(),
                model: "step-image-edit-2".to_owned(),
                task: Some(ModelTask::ImageGeneration),
            })
            .await
            .unwrap();

        assert_eq!(response.status, HealthStatus::Healthy);
        assert_eq!(response.error_kind, None);
        assert_eq!(response.platform, "stepfun-plan");

        // The outcome lands on the model's catalog row (persist_probe_outcome).
        use nomifun_db::IProviderModelRepository;
        let row = nomifun_db::SqliteProviderModelRepository::new(pool.clone())
            .get(&pid, "step-image-edit-2")
            .await
            .unwrap()
            .unwrap();
        assert!(row.health_checked_at.is_some(), "probe outcome must be persisted");
    }

    #[tokio::test]
    async fn modality_probe_model_invalid_is_unhealthy_not_found() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Reproduces the StepFun 404 shape — at the CORRECT image endpoint.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "error": { "message": "The model \"x\" does not exist", "type": "model_invalid" }
            })))
            .mount(&server)
            .await;

        let (service, pool, _temp) = setup_health_service().await;
        let pid = seed_catalog(
            &pool,
            "stepfun-plan",
            &server.uri(),
            "x",
            r#"["image_generation"]"#,
            true,
        )
        .await;

        let response = service
            .health_check(ProviderHealthCheckRequest {
                provider_id: pid,
                model: "x".to_owned(),
                task: Some(ModelTask::ImageGeneration),
            })
            .await
            .unwrap();

        assert_eq!(response.status, HealthStatus::Unhealthy);
        assert_eq!(response.error_kind, Some(ProviderHealthCheckErrorKind::NotFound));
        assert_eq!(response.http_status, Some(404));
    }

    #[tokio::test]
    async fn modality_probe_disabled_model_is_unhealthy_response_not_http_error() {
        // Ledger note honored: invoke.probe refuses disabled rows, but the
        // health endpoint's wire contract stays "200 + unhealthy" so the UI's
        // health button keeps working on disabled rows.
        let (service, pool, _temp) = setup_health_service().await;
        let pid = seed_catalog(
            &pool,
            "openai",
            "https://unused.example",
            "gpt-image-1",
            r#"["image_generation"]"#,
            false,
        )
        .await;

        let response = service
            .health_check(ProviderHealthCheckRequest {
                provider_id: pid,
                model: "gpt-image-1".to_owned(),
                task: Some(ModelTask::ImageGeneration),
            })
            .await
            .expect("disabled model must yield an unhealthy response, not an HTTP error");

        assert_eq!(response.status, HealthStatus::Unhealthy);
        assert!(
            response.message.as_deref().unwrap_or_default().contains("model disabled"),
            "message: {:?}",
            response.message
        );
    }

    #[test]
    fn classify_error_understands_invoke_message_shapes() {
        // The invoke layer reports upstream failures as "provider returned NNN …".
        assert_eq!(
            classify_error("ProviderError: provider returned 404 Not Found: nope", false),
            ProviderHealthCheckErrorKind::NotFound
        );
        assert_eq!(
            classify_error("InvalidParams: provider returned 400 Bad Request: nope", false),
            ProviderHealthCheckErrorKind::InvalidRequest
        );
        assert_eq!(
            classify_error("ProviderError: provider returned 500 Internal Server Error: boom", false),
            ProviderHealthCheckErrorKind::ApiError
        );
        assert_eq!(
            classify_error("Network: request failed: error sending request", false),
            ProviderHealthCheckErrorKind::ConnectionError
        );
        assert_eq!(
            extract_http_status("provider returned 404 Not Found: nope"),
            Some(404)
        );
    }

    // -- persist_probe_outcome --

    const PROBE_PROVIDER: &str = "0190f5fe-7c00-7a00-8abc-012345678901";

    async fn seed_provider_with_model(
        db: &nomifun_db::Database,
    ) -> nomifun_db::SqliteProviderModelRepository {
        use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel};

        nomifun_db::SqliteProviderRepository::new(db.pool().clone())
            .create(CreateProviderParams {
                provider_id: Some(PROBE_PROVIDER),
                platform: "openai",
                name: "P",
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: "[]",
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
        let repo = nomifun_db::SqliteProviderModelRepository::new(db.pool().clone());
        repo.create(
            PROBE_PROVIDER,
            &NewProviderModel {
                model: "gpt-test",
                enabled: true,
                sort_order: 0,
                tasks: r#"["chat"]"#,
                traits: "[]",
                params: "{}",
                source: "inferred",
                ..Default::default()
            },
        )
        .await
        .unwrap();
        repo
    }

    #[tokio::test]
    async fn persist_probe_outcome_stores_healthy_result() {
        use nomifun_db::IProviderModelRepository;

        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo = seed_provider_with_model(&db).await;
        let response = ProviderHealthCheckResponse {
            provider_id: PROBE_PROVIDER.to_owned(),
            platform: "openai".to_owned(),
            model: "gpt-test".to_owned(),
            status: HealthStatus::Healthy,
            elapsed_ms: 321,
            message: None,
            error_kind: None,
            http_status: None,
            timeout_stage: None,
        };

        persist_probe_outcome(&repo, &response).await;

        let row = repo.get(PROBE_PROVIDER, "gpt-test").await.unwrap().unwrap();
        assert!(row.health_checked_at.is_some(), "set_health stamps checked_at");
        let health: ModelHealthStatus =
            serde_json::from_str(row.health.as_deref().unwrap()).unwrap();
        assert_eq!(health.status, HealthStatus::Healthy);
        assert_eq!(health.latency, Some(321));
        assert_eq!(health.error, None);
        assert!(health.last_check.is_some());
    }

    #[tokio::test]
    async fn persist_probe_outcome_stores_unhealthy_result_and_tolerates_missing_row() {
        use nomifun_db::IProviderModelRepository;

        let db = nomifun_db::init_database_memory().await.unwrap();
        let repo = seed_provider_with_model(&db).await;
        let response = unhealthy_response(
            PROBE_PROVIDER.to_owned(),
            "openai".to_owned(),
            "gpt-test".to_owned(),
            Duration::from_millis(45),
            "Provider error: API error 429: Too Many Requests".to_owned(),
            None,
        );

        persist_probe_outcome(&repo, &response).await;

        let row = repo.get(PROBE_PROVIDER, "gpt-test").await.unwrap().unwrap();
        let health: ModelHealthStatus =
            serde_json::from_str(row.health.as_deref().unwrap()).unwrap();
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert_eq!(
            health.error.as_deref(),
            Some("Provider error: API error 429: Too Many Requests")
        );

        // A probe for an uncatalogued model must be a silent no-op, never an error.
        let ghost = ProviderHealthCheckResponse {
            model: "ghost-model".to_owned(),
            ..response
        };
        persist_probe_outcome(&repo, &ghost).await;
        assert!(repo.get(PROBE_PROVIDER, "ghost-model").await.unwrap().is_none());
    }
}
