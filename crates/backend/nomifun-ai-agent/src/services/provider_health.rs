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
    CapabilityHealth, HealthStatus, ModelTask, ProviderHealthCheckErrorKind,
    ProviderHealthCheckRequest, ProviderHealthCheckResponse,
};
use nomifun_common::AppError;
use nomifun_model_invoke::{ModelInvokeService, ModelRef};
use regex::Regex;
use tracing::{info, warn};

use crate::factory::provider_config::resolve_provider_fields;
use crate::types::NomiResolvedConfig;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_CHECK_PROMPT: &str = "Reply with exactly OK.";
const HEALTH_CHECK_MSG_ID: &str = "provider-health-check";

pub struct ProviderHealthCheckService {
    data_dir: PathBuf,
    /// Unified invoke layer: non-chat modality probes ride
    /// [`ModelInvokeService::probe`] (chat stays on the agent engine).
    invoke: Arc<ModelInvokeService>,
}

impl ProviderHealthCheckService {
    pub fn new(
        data_dir: PathBuf,
        invoke: Arc<ModelInvokeService>,
    ) -> Self {
        Self {
            data_dir,
            invoke,
        }
    }

    pub async fn health_check(
        &self,
        req: ProviderHealthCheckRequest,
    ) -> Result<ProviderHealthCheckResponse, AppError> {
        let task = req.task;
        let model_ref = ModelRef {
            provider_id: req.provider_id,
            model: req.model,
        };
        let resolved = self
            .invoke
            .resolve_task_config(&model_ref, task)
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        let provider_id = resolved.provider_id.clone();
        let expected_config_revision = resolved.config_revision;
        let platform = resolved.platform.clone();
        let model = resolved.model.clone();

        let response = if task == ModelTask::Chat {
            let config = self.resolve_probe_config(&provider_id, &model).await?;
            run_probe(provider_id, platform, model, task, config).await?
        } else {
            info!(
                provider_id = %provider_id,
                platform = %platform,
                model = %model,
                task = ?task,
                "Modality health check started"
            );
            let probe = if task == ModelTask::RealtimeConversation {
                self.invoke.probe_realtime(&model_ref).await
            } else {
                self.invoke.probe(&model_ref, task).await
            };
            match probe {
                Ok(report) if report.healthy => healthy_response(
                    provider_id,
                    platform,
                    model,
                    task,
                    Duration::from_millis(report.latency_ms),
                ),
                Ok(report) => {
                    let message = report
                        .message
                        .unwrap_or_else(|| "modality probe failed".to_owned());
                    let lower_message = message.to_ascii_lowercase();
                    let timeout_stage = if lower_message.contains("realtime probe timeout")
                        || lower_message.contains("realtime websocket handshake timed out")
                    {
                        Some("realtime_session_created".to_owned())
                    } else if lower_message.contains("modality probe timeout") {
                        Some("modality_probe".to_owned())
                    } else {
                        None
                    };
                    unhealthy_response(
                        provider_id,
                        platform,
                        model,
                        task,
                        Duration::from_millis(report.latency_ms),
                        message,
                        timeout_stage,
                    )
                }
                Err(error) => unhealthy_response(
                    provider_id,
                    platform,
                    model,
                    task,
                    Duration::ZERO,
                    error.to_string(),
                    None,
                ),
            }
        };
        log_health_check_result(&response);
        persist_probe_outcome(
            self.invoke.provider_model_capability_repo().as_ref(),
            expected_config_revision,
            &response,
        )
        .await;
        Ok(response)
    }

    async fn resolve_probe_config(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<NomiResolvedConfig, AppError> {
        let fields = resolve_provider_fields(self.invoke.as_ref(), provider_id, model_id).await?;

        Ok(NomiResolvedConfig {
            provider: fields.provider,
            api_key: fields.api_key,
            model: fields.model,
            base_url: fields.base_url,
            system_prompt: Some(
                "You are a provider health probe. Reply with exactly OK and do not use tools."
                    .into(),
            ),
            max_tokens: 16,
            max_turns: Some(1),
            context_limit: fields.context_limit.map(|value| value as u64),
            compat_overrides: fields.compat_overrides,
            session_directory: self.data_dir.join("nomi-health-check-sessions"),
            session_mode: None,
            extra_mcp_servers: HashMap::new(),
            loopback_capability_leases: Default::default(),
            bedrock_config: fields.bedrock_config,
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
        })
    }
}
/// Persist one probe outcome onto the model's authoritative catalog row.
///
/// Persist the observation on the exact task capability. Task and observation
/// timestamp are already columns in the capability row and are not duplicated
/// inside the health JSON.
pub(crate) async fn persist_probe_outcome(
    repo: &dyn nomifun_db::IProviderModelCapabilityRepository,
    expected_config_revision: i64,
    response: &ProviderHealthCheckResponse,
) {
    let health = CapabilityHealth {
        status: response.status,
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
    let task = serde_json::to_value(response.task)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("ModelTask serializes as a string");
    match repo
        .set_health(
            &response.provider_id,
            expected_config_revision,
            &response.model,
            &task,
            Some(&json),
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!(
            provider_id = %response.provider_id,
            model = %response.model,
            task = ?response.task,
            expected_config_revision,
            "probe outcome discarded: task capability is missing or its invocation graph changed"
        ),
        Err(error) => warn!(
            provider_id = %response.provider_id,
            model = %response.model,
            %error,
            "probe outcome health write-back failed"
        ),
    }
}

async fn run_probe(
    provider_id: String,
    platform: String,
    model: String,
    task: ModelTask,
    config_extra: NomiResolvedConfig,
) -> Result<ProviderHealthCheckResponse, AppError> {
    let started = Instant::now();

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
                task,
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
                task,
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
                task,
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
                task,
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
    task: ModelTask,
    elapsed: Duration,
) -> ProviderHealthCheckResponse {
    ProviderHealthCheckResponse {
        provider_id,
        platform,
        model,
        task,
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
    config.compat.extra_body = config_extra.compat_overrides.extra_body;

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
    task: ModelTask,
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
        task,
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
    // Upstream statuses arrive in two shapes: the agent engine's "api error
    // NNN" and the invoke layer's "provider returned NNN <reason>".
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
        || lower.starts_with("network:")
        || lower.contains("websocket connection failed")
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
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderModelCapabilityRepository, IProviderRepository,
        NewProviderModel, NewProviderModelCapability, SqliteProviderConnectionRepository,
        SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, init_database_memory,
    };
    use nomifun_model_invoke::{AdapterRegistry, default_adapters};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn classifies_invoke_errors_without_transport_fallbacks() {
        assert_eq!(
            classify_error("provider returned 401 Unauthorized", false),
            ProviderHealthCheckErrorKind::Unauthorized
        );
        assert_eq!(
            classify_error("Network: request failed: connection reset", false),
            ProviderHealthCheckErrorKind::ConnectionError
        );
        assert_eq!(
            classify_error("provider returned 429 Too Many Requests", false),
            ProviderHealthCheckErrorKind::RateLimited
        );
        assert_eq!(
            classify_error("anything", true),
            ProviderHealthCheckErrorKind::Timeout
        );
    }

    #[test]
    fn extracts_http_status_from_supported_error_shapes() {
        assert_eq!(extract_http_status("API error 403 forbidden"), Some(403));
        assert_eq!(
            extract_http_status("provider returned 422 Unprocessable Entity"),
            Some(422)
        );
        assert_eq!(extract_http_status("connection refused"), None);
    }

    #[tokio::test]
    async fn modality_health_redacts_response_and_persisted_capability_error() {
        const ENCRYPTION_KEY: [u8; 32] = [0xA7; 32];
        const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000071";
        const MODEL: &str = "embedding-secret-regression";
        let secret = "health-key/+?=value";
        let encoded = "health-key%2F%2B%3F%3Dvalue";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string(format!(
                "Authorization: Bearer {secret}; x-api-key={secret}; api_key={encoded}"
            )))
            .expect(1)
            .mount(&server)
            .await;

        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let model_repo = Arc::new(SqliteProviderModelRepository::new(pool.clone()));
        let capability_repo =
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
        let connection_repo = Arc::new(SqliteProviderConnectionRepository::new(pool));
        let encrypted_credentials = encrypt_string(
            &serde_json::json!({ "api_keys": [secret] }).to_string(),
            &ENCRYPTION_KEY,
        )
        .unwrap();
        let endpoint = format!("{}/v1/embeddings", server.uri());
        let capabilities = [NewProviderModelCapability {
            task: "embedding",
            traits: "[]",
            protocol: "openai.embeddings",
            connection_role: "default",
            endpoint: Some(&endpoint),
            provider_params: "{}",
            context_limit: Some(8_192),
            ..Default::default()
        }];
        let initial_model = NewProviderModel {
            model: MODEL,
            enabled: true,
            sort_order: 0,
            description: None,
            capabilities: &capabilities,
        };
        provider_repo
            .create(
                CreateProviderParams {
                    provider_id: Some(PROVIDER_ID),
                    platform: "openai",
                    name: "Secret regression",
                    base_url: &server.uri(),
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

        let invoke = Arc::new(ModelInvokeService::new(
            provider_repo,
            model_repo,
            capability_repo.clone(),
            connection_repo,
            ENCRYPTION_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(default_adapters()),
        ));
        let data_dir = tempfile::tempdir().unwrap();
        let service = ProviderHealthCheckService::new(data_dir.path().to_owned(), invoke);
        let response = service
            .health_check(ProviderHealthCheckRequest {
                provider_id: PROVIDER_ID.to_owned(),
                model: MODEL.to_owned(),
                task: ModelTask::Embedding,
            })
            .await
            .unwrap();

        assert_eq!(response.status, HealthStatus::Unhealthy);
        assert_eq!(response.http_status, Some(401));
        let response_json = serde_json::to_string(&response).unwrap();
        for leaked in [secret, encoded] {
            assert!(
                !response_json.contains(leaked),
                "health response leaked credential: {response_json}"
            );
        }
        assert!(response_json.contains("[REDACTED]"));

        let stored = capability_repo
            .get(PROVIDER_ID, MODEL, "embedding")
            .await
            .unwrap()
            .unwrap()
            .health
            .expect("failed probe persists task-scoped health");
        for leaked in [secret, encoded] {
            assert!(
                !stored.contains(leaked),
                "persisted capability health leaked credential: {stored}"
            );
        }
        assert!(stored.contains("[REDACTED]"));
        server.verify().await;
    }
}
