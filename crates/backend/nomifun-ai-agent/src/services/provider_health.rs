use std::sync::Arc;
use std::time::{Duration, Instant};

use nomifun_api_types::{
    CapabilityHealth, HealthStatus, ModelTask, ProviderHealthCheckErrorKind,
    ProviderHealthCheckRequest, ProviderHealthCheckResponse,
};
use nomifun_common::AppError;
use nomifun_model_invoke::{
    ChatVerificationFailure, ChatVerificationTarget, ModelInvokeService, ModelRef,
    verify_chat_agent_round_trip,
};
use regex::Regex;
use tracing::{info, warn};

/// Whole-exchange deadline for the Chat health round trip.
const CHAT_PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// Default probe output ceiling; the capability's own `output_limit` wins when
/// it is lower, matching the saved configuration exactly.
const CHAT_PROBE_OUTPUT_TOKENS: u32 = 4096;

pub struct ProviderHealthCheckService {
    /// Unified invoke layer: non-chat modality probes ride
    /// [`ModelInvokeService::probe`]; Chat health runs the shared Agent wire
    /// stack through [`verify_chat_agent_round_trip`].
    invoke: Arc<ModelInvokeService>,
}

impl ProviderHealthCheckService {
    pub fn new(
        invoke: Arc<ModelInvokeService>,
    ) -> Self {
        Self {
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
            info!(
                provider_id = %provider_id,
                platform = %platform,
                model = %model,
                "Provider health check started"
            );
            let started = Instant::now();
            let probe_tokens = resolved
                .output_limit
                .and_then(|limit| u32::try_from(limit).ok())
                .filter(|limit| *limit > 0)
                .unwrap_or(CHAT_PROBE_OUTPUT_TOKENS)
                .min(CHAT_PROBE_OUTPUT_TOKENS);
            match ChatVerificationTarget::from_resolved(&resolved)
                .map_err(ChatVerificationFailure::Transport)
            {
                Err(failure) => chat_probe_outcome(
                    provider_id, platform, model, task, started.elapsed(), &failure,
                ),
                Ok(target) => match verify_chat_agent_round_trip(
                    &target,
                    probe_tokens,
                    CHAT_PROBE_TIMEOUT,
                )
                .await
                {
                    Ok(()) => healthy_response(
                        provider_id, platform, model, task, started.elapsed(),
                    ),
                    Err(failure) => chat_probe_outcome(
                        provider_id, platform, model, task, started.elapsed(), &failure,
                    ),
                },
            }
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
                Ok(report) if report.status == HealthStatus::Healthy => healthy_response(
                    provider_id,
                    platform,
                    model,
                    task,
                    Duration::from_millis(report.latency_ms),
                ),
                Ok(report) if report.status == HealthStatus::Unknown => {
                    let mut response = healthy_response(provider_id, platform, model, task,
                        Duration::from_millis(report.latency_ms));
                    response.status = HealthStatus::Unknown;
                    response.message = report.message;
                    response
                }
                Ok(report) => {
                    let message = report
                        .message
                        .unwrap_or_else(|| "modality probe failed".to_owned());
                    let lower_message = message.to_ascii_lowercase();
                    let timeout_stage = if lower_message.contains("realtime probe timeout")
                        || lower_message.contains("realtime websocket handshake timed out")
                    {
                        Some("realtime_session_configuration".to_owned())
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
        // Stamp the address once, here, rather than threading it through every
        // response builder. Both branches resolve their URL from this same
        // `resolved` value, so this is the URL the probe actually requested —
        // and it is the fact that separates "wrong base URL" from "bad key".
        // Query material is redacted: Gemini and `query_key:` schemes carry
        // credentials there.
        let mut response = response;
        response.attempted_url = resolved
            .http_endpoint()
            .ok()
            .map(|url| nomifun_net::secret_redaction::redact_url_queries(&url));
        log_health_check_result(&response);
        persist_probe_outcome(
            self.invoke.provider_model_capability_repo().as_ref(),
            expected_config_revision,
            &response,
        )
        .await;
        Ok(response)
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
        // Carry the discriminators through. Narrowing to `error` here is what
        // made a stored 404 and a stored 401 indistinguishable after the fact.
        error_kind: response.error_kind,
        http_status: response.http_status,
        attempted_url: response.attempted_url.clone(),
        unsupported_technical_capabilities: Vec::new(),
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

/// Map one typed verification result onto the health response: success is
/// `Healthy`, a well-formed exchange that never used the probe tool stays
/// `Unknown`, and every failure carries the runner's typed diagnosis rather
/// than a parsed string.
fn chat_probe_outcome(
    provider_id: String,
    platform: String,
    model: String,
    task: ModelTask,
    elapsed: Duration,
    failure: &ChatVerificationFailure,
) -> ProviderHealthCheckResponse {
    match failure {
        ChatVerificationFailure::Inconclusive(message) => {
            let mut response = healthy_response(provider_id, platform, model, task, elapsed);
            response.status = HealthStatus::Unknown;
            response.message = Some((*message).to_owned());
            response
        }
        ChatVerificationFailure::Timeout => unhealthy_response(
            provider_id,
            platform,
            model,
            task,
            elapsed,
            format!("Health check timeout ({}s)", CHAT_PROBE_TIMEOUT.as_secs()),
            Some("chat_probe".into()),
        ),
        _ => {
            let diagnosis = failure.diagnosis();
            let mut response = unhealthy_response(
                provider_id,
                platform,
                model,
                task,
                elapsed,
                failure.message(),
                None,
            );
            if let Some(diagnosis) = diagnosis {
                response.error_kind = Some(diagnosis.error_kind);
                response.http_status = diagnosis.http_status;
            }
            response
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
        attempted_url: None,
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
        HealthStatus::Unknown => info!(
            provider_id = %response.provider_id,
            model = %response.model,
            task = ?response.task,
            "Provider accepted the probe; generated output remains unverified"
        ),
        HealthStatus::Unhealthy => warn!(
            provider_id = %response.provider_id,
            platform = %response.platform,
            model = %response.model,
            elapsed_ms = response.elapsed_ms,
            error_kind = ?response.error_kind,
            http_status = ?response.http_status,
            timeout_stage = ?response.timeout_stage,
            attempted_url = ?response.attempted_url,
            "Provider health check failed"
        ),
    }
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
        attempted_url: None,
    }
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

/// Map an `InvokeError` Display prefix (`"{kind:?}: …"`) onto a health-check
/// kind.
///
/// This is a LAST RESORT, consulted only after the prose arms have failed to
/// find anything more specific. The prefix is deliberately coarse: a 400 whose
/// body reports an exhausted balance arrives as `InvalidParams` (see
/// `transport::error_from_response`), and reporting that as "fix your request"
/// instead of "top up the account" loses the actionable half of the diagnosis.
///
/// `Network` and `Timeout` are the exception and are matched EARLY by the
/// caller: their prose arms overlap ("request timed out" reads as both), so the
/// typed kind is strictly better information there.
fn invoke_error_kind_prefix(message: &str) -> Option<ProviderHealthCheckErrorKind> {
    use ProviderHealthCheckErrorKind as K;

    match invoke_error_prefix(message)? {
        // Local/config faults the operator can fix in model management.
        "InvalidParams" | "Config" | "UnsupportedTask" | "NoAdapter" | "MissingConnection"
        | "NotPollable" => Some(K::InvalidRequest),
        // The provider answered, but not with something usable.
        "ParseError" | "JobFailed" | "ContentPolicy" | "ProviderError" => Some(K::ApiError),
        "NonApiResponse" => Some(K::NonApiResponse),
        "QuotaExhausted" => Some(K::InsufficientQuota),
        // Without a recognizable status this is still a credential rejection.
        "Auth" => Some(K::Unauthorized),
        "RateLimited" => Some(K::RateLimited),
        _ => None,
    }
}

/// The transport-level kinds whose prose arms are ambiguous, so the typed kind
/// wins outright.
fn invoke_error_transport_prefix(message: &str) -> Option<ProviderHealthCheckErrorKind> {
    match invoke_error_prefix(message)? {
        "Network" => Some(ProviderHealthCheckErrorKind::ConnectionError),
        "Timeout" => Some(ProviderHealthCheckErrorKind::Timeout),
        _ => None,
    }
}

/// Extract a bare CamelCase `Kind:` prefix. Prose that merely contains a colon
/// (`"provider returned 400: ..."`) has whitespace in the candidate and is
/// rejected. This cannot prove the string came from `InvokeError`, which is why
/// every mapping above is either a last resort or strictly more informative
/// than the prose it replaces.
fn invoke_error_prefix(message: &str) -> Option<&str> {
    let (prefix, _) = message.split_once(':')?;
    (!prefix.is_empty() && !prefix.contains(char::is_whitespace)).then_some(prefix)
}

pub(crate) fn classify_error(message: &str, is_timeout: bool) -> ProviderHealthCheckErrorKind {
    if is_timeout {
        return ProviderHealthCheckErrorKind::Timeout;
    }

    // Transport kinds first: their prose arms genuinely overlap, so the invoke
    // layer's own classification is better than re-deriving it.
    if let Some(kind) = invoke_error_transport_prefix(message) {
        return kind;
    }

    let lower = message.to_lowercase();
    // A document body is a wrong-address symptom and must be recognized before
    // the status/prose arms: both lanes phrase it with "provider returned", so
    // it would otherwise be filed as a generic upstream API error and the one
    // actionable fact — that this URL serves a web page — would be lost.
    if lower.contains("web page, not an api response") {
        return ProviderHealthCheckErrorKind::NonApiResponse;
    }
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
    if lower.contains("invalidendpointormodel.notfound")
        || (lower.contains("model or endpoint")
            && (lower.contains("does not exist") || lower.contains("do not have access")))
        || (lower.contains("ark video generation cannot use doubao seed")
            && lower.contains("seedance"))
        || (lower.contains("console display name") && lower.contains("api model id"))
    {
        return ProviderHealthCheckErrorKind::ModelUnavailable;
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

    // Last resort before Unknown: the invoke layer's own typed kind. Coarser
    // than every arm above, which is why it runs last.
    invoke_error_kind_prefix(message).unwrap_or(ProviderHealthCheckErrorKind::Unknown)
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
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const CHAT_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000072";
    const CHAT_MODEL: &str = "strict-anthropic-health";
    const CHAT_KEY: &str = "local-health-fixture-key";

    fn anthropic_text_stream(text: &str, stop_reason: &str, complete: bool) -> String {
        let mut events = vec![
            serde_json::json!({"type":"message_start","message":{"id":"msg_health","type":"message","role":"assistant",
                "content":[],"model":CHAT_MODEL,"usage":{"input_tokens":2,"output_tokens":0}}}),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":stop_reason},"usage":{"output_tokens":1}}),
        ];
        if complete { events.push(serde_json::json!({"type":"message_stop"})); }
        anthropic_frames(events)
    }

    fn anthropic_frames(events: Vec<serde_json::Value>) -> String {
        events.into_iter().map(|event| format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect()
    }

    fn anthropic_tool_stream(name: &str) -> String {
        anthropic_frames(vec![
            serde_json::json!({"type":"message_start","message":{"id":"msg_health","type":"message","role":"assistant",
                "content":[],"model":CHAT_MODEL,"usage":{"input_tokens":2,"output_tokens":0}}}),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call_health","name":name,"input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"value\":\"ok\"}"}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":1}}),
            serde_json::json!({"type":"message_stop"}),
        ])
    }

    /// Two-round endpoint: the request containing the local tool result gets
    /// `second`; every other request gets `first`. Anthropic requests still
    /// require the pinned Messages version header.
    #[derive(Clone)]
    struct StrictAnthropicEndpoint { first: String, second: String }
    impl Respond for StrictAnthropicEndpoint {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            if request.headers.get("anthropic-version").and_then(|value| value.to_str().ok()) != Some("2023-06-01") {
                return ResponseTemplate::new(400).set_body_json(serde_json::json!({
                    "error":{"type":"invalid_request_error","message":"anthropic-version header is required"}
                }));
            }
            let body = String::from_utf8_lossy(&request.body);
            let stream = if body.contains("tool_result") { &self.second } else { &self.first };
            ResponseTemplate::new(200).set_body_raw(stream.clone(),"text/event-stream")
        }
    }

    async fn anthropic_health_fixture(server: &MockServer) -> (
        ProviderHealthCheckService, Arc<SqliteProviderModelCapabilityRepository>,
    ) {
        managed_chat_health_fixture(server,"anthropic","anthropic.messages","header_key:x-api-key","/v1/messages").await
    }

    async fn managed_chat_health_fixture(
        server: &MockServer, platform: &str, protocol: &str, auth_scheme: &str, endpoint_path: &str,
    ) -> (ProviderHealthCheckService, Arc<SqliteProviderModelCapabilityRepository>) {
        managed_task_health_fixture(server, platform, protocol, auth_scheme, endpoint_path, ModelTask::Chat).await
    }

    async fn managed_task_health_fixture(
        server: &MockServer, platform: &str, protocol: &str, auth_scheme: &str, endpoint_path: &str, task: ModelTask,
    ) -> (ProviderHealthCheckService, Arc<SqliteProviderModelCapabilityRepository>) {
        const KEY: [u8;32] = [0xA8;32];
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let capability_repo = Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
        let credentials = encrypt_string(&serde_json::json!({"api_keys":[CHAT_KEY]}).to_string(),&KEY).unwrap();
        let endpoint = format!("{}{endpoint_path}",server.uri());
        let task_name = serde_json::to_value(task).unwrap();
        let capabilities = [NewProviderModelCapability {
            task:task_name.as_str().unwrap(),traits:"[]",protocol,connection_role:"default",
            endpoint:Some(&endpoint),provider_params:"{}",context_limit:Some(64000),output_limit:Some(4096),
            ..Default::default()
        }];
        provider_repo.create(CreateProviderParams {
            provider_id:Some(CHAT_PROVIDER_ID),platform,name:"Strict local health fixture",
            base_url:&server.uri(),auth_scheme,credentials_encrypted:&credentials,
            enabled:true,bedrock_config:None,sort_order:None,
        },&NewProviderModel {model:CHAT_MODEL,enabled:true,capabilities:&capabilities,..Default::default()},&[]).await.unwrap();
        let invoke = Arc::new(ModelInvokeService::new(provider_repo,
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),capability_repo.clone(),
            Arc::new(SqliteProviderConnectionRepository::new(pool)),KEY,reqwest::Client::new(),
            AdapterRegistry::new(default_adapters())));
        (ProviderHealthCheckService::new(invoke),capability_repo)
    }

    #[tokio::test]
    async fn anthropic_health_uses_required_version_header_and_complete_visible_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/messages")).and(header("x-api-key",CHAT_KEY))
            .respond_with(StrictAnthropicEndpoint {
                first:anthropic_tool_stream("nomifun_protocol_echo"),
                second:anthropic_text_stream("OK","end_turn",true)})
            .expect(3).mount(&server).await;
        let missing_version = reqwest::Client::new().post(format!("{}/v1/messages",server.uri()))
            .header("x-api-key",CHAT_KEY).json(&serde_json::json!({"model":CHAT_MODEL,"stream":true,
                "max_tokens":4096,"messages":[{"role":"user","content":"Reply OK"}]}))
            .send().await.unwrap();
        assert_eq!(missing_version.status(),reqwest::StatusCode::BAD_REQUEST);
        let (service,capabilities) = anthropic_health_fixture(&server).await;
        let response = service.health_check(ProviderHealthCheckRequest {
            provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::Chat,
        }).await.unwrap();
        assert_eq!(response.status,HealthStatus::Healthy,"{response:?}");
        assert_eq!(response.attempted_url.as_deref(),Some(format!("{}/v1/messages",server.uri()).as_str()));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(),3,"one manual sanity request plus the two verification rounds");
        assert!(!requests[0].headers.contains_key("anthropic-version"));
        let first: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(requests[1].headers["anthropic-version"],"2023-06-01");
        assert_eq!(first["model"],CHAT_MODEL);
        assert_eq!(first["stream"],true);
        assert_eq!(first["tools"][0]["name"],"nomifun_protocol_echo");
        let second: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
        let tool_result = second["messages"].as_array().unwrap().iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .find(|part| part["type"] == "tool_result")
            .expect("round two replays the local tool result");
        assert_eq!(tool_result["tool_use_id"],"call_health");
        let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"chat").await.unwrap().unwrap().health.unwrap();
        let stored: CapabilityHealth = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored.status,HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn anthropic_health_distinguishes_decoder_failures_from_inconclusive_rounds() {
        for (name,stream,expected) in [
            ("partial",anthropic_text_stream("OK","end_turn",false),HealthStatus::Unhealthy),
            ("usage_only",anthropic_text_stream("","end_turn",true),HealthStatus::Unknown),
            ("max_tokens",anthropic_text_stream("OK","max_tokens",true),HealthStatus::Unknown),
            ("unexpected_tool",anthropic_tool_stream("unexpected_tool"),HealthStatus::Unknown),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).and(path("/v1/messages")).and(header("x-api-key",CHAT_KEY))
                .respond_with(StrictAnthropicEndpoint {first:stream,second:anthropic_text_stream("OK","end_turn",true)})
                .expect(1).mount(&server).await;
            let (service,capabilities) = anthropic_health_fixture(&server).await;
            let response = service.health_check(ProviderHealthCheckRequest {
                provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::Chat,
            }).await.unwrap();
            assert_eq!(response.status,expected,"{name}: {response:?}");
            assert!(response.message.as_deref().is_some_and(|message| !message.is_empty()));
            assert!(!serde_json::to_string(&response).unwrap().contains(CHAT_KEY));
            let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"chat").await.unwrap().unwrap().health.unwrap();
            let stored: CapabilityHealth = serde_json::from_str(&stored).unwrap();
            assert_eq!(stored.status,expected,"{name}");
        }
    }

    #[tokio::test]
    async fn anthropic_health_rejects_version_errors_and_redacts_diagnostics() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/messages")).and(header("anthropic-version","2023-06-01"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "type":"error","error":{"type":"invalid_request_error",
                    "message":format!("anthropic-version rejected for key {CHAT_KEY}")}
            }))).expect(1).mount(&server).await;
        let (service,capabilities) = anthropic_health_fixture(&server).await;
        let response = service.health_check(ProviderHealthCheckRequest {
            provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::Chat,
        }).await.unwrap();
        assert_eq!(response.status,HealthStatus::Unhealthy,"{response:?}");
        assert_eq!(response.http_status,Some(400));
        let response_json = serde_json::to_string(&response).unwrap();
        assert!(response_json.contains("anthropic-version"));
        assert!(!response_json.contains(CHAT_KEY));
        let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"chat").await.unwrap().unwrap().health.unwrap();
        assert!(!stored.contains(CHAT_KEY));
        assert_eq!(serde_json::from_str::<CapabilityHealth>(&stored).unwrap().status,HealthStatus::Unhealthy);
    }

    /// Two-round endpoint for the non-Anthropic protocols: the request carrying
    /// the local tool result (round two) is routed by a wire marker.
    #[derive(Clone)]
    struct TwoRoundSse { first: String, second: String, marker: &'static str }
    impl Respond for TwoRoundSse {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body = String::from_utf8_lossy(&request.body);
            let stream = if body.contains(self.marker) { &self.second } else { &self.first };
            ResponseTemplate::new(200).set_body_raw(stream.clone(), "text/event-stream")
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ChatHealthScenario { Complete, Empty, Partial, Truncated, Error }

    fn openai_chat_tool_stream() -> String {
        let events = [
            serde_json::json!({"id":"chatcmpl_health","choices":[{"index":0,
                "delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_health","type":"function",
                    "function":{"name":"nomifun_protocol_echo","arguments":""}}]},"finish_reason":null}]}),
            serde_json::json!({"id":"chatcmpl_health","choices":[{"index":0,
                "delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"value\":\"ok\"}"}}]},
                "finish_reason":null}]}),
            serde_json::json!({"id":"chatcmpl_health","choices":[{"index":0,"delta":{},
                "finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":1}}),
        ];
        format!("{}data: [DONE]\n\n", data_sse_frames(events.into()))
    }

    fn responses_tool_stream() -> String {
        anthropic_frames(vec![
            serde_json::json!({"type":"response.created","response":{"id":"resp_health","status":"in_progress"}}),
            serde_json::json!({"type":"response.output_item.added","output_index":0,
                "item":{"id":"fc_health","type":"function_call","status":"in_progress",
                    "arguments":"","call_id":"call_health","name":"nomifun_protocol_echo"}}),
            serde_json::json!({"type":"response.function_call_arguments.delta","output_index":0,
                "item_id":"fc_health","delta":"{\"value\":\"ok\"}"}),
            serde_json::json!({"type":"response.function_call_arguments.done","output_index":0,
                "item_id":"fc_health","arguments":"{\"value\":\"ok\"}"}),
            serde_json::json!({"type":"response.output_item.done","output_index":0,
                "item":{"id":"fc_health","type":"function_call","status":"completed",
                    "arguments":"{\"value\":\"ok\"}","call_id":"call_health","name":"nomifun_protocol_echo"}}),
            serde_json::json!({"type":"response.completed","response":{"id":"resp_health","status":"completed",
                "output":[{"id":"fc_health","type":"function_call","status":"completed",
                    "arguments":"{\"value\":\"ok\"}","call_id":"call_health","name":"nomifun_protocol_echo"}],
                "usage":{"input_tokens":2,"output_tokens":1}}}),
        ])
    }

    fn gemini_tool_stream() -> String {
        data_sse_frames(vec![
            serde_json::json!({"candidates":[{"index":0,"content":{"role":"model",
                "parts":[{"functionCall":{"name":"nomifun_protocol_echo","args":{"value":"ok"}}}]}}],
                "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}}),
            serde_json::json!({"candidates":[{"index":0,"finishReason":"STOP"}],
                "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}}),
        ])
    }

    fn data_sse_frames(events: Vec<serde_json::Value>) -> String {
        events.into_iter().map(|event| format!("data: {event}\n\n")).collect()
    }

    fn openai_chat_health_stream(scenario: ChatHealthScenario) -> String {
        let mut events = vec![serde_json::json!({"choices":[{"index":0,
            "delta":{"role":"assistant","content":if scenario==ChatHealthScenario::Empty {""} else {"OK"}},
            "finish_reason":null}]})];
        if scenario == ChatHealthScenario::Partial {return data_sse_frames(events);}
        if scenario == ChatHealthScenario::Error {
            events.push(serde_json::json!({"error":{"code":"invalid_request_error","message":format!("fixture stream failure {CHAT_KEY}")}}));
        } else {
            events.push(serde_json::json!({"choices":[{"index":0,"delta":{},
                "finish_reason":if scenario==ChatHealthScenario::Truncated {"length"} else {"stop"}}],
                "usage":{"prompt_tokens":2,"completion_tokens":1}}));
        }
        format!("{}data: [DONE]\n\n",data_sse_frames(events))
    }

    fn responses_health_stream(scenario: ChatHealthScenario) -> String {
        let mut events = vec![serde_json::json!({"type":"response.created","response":{
            "id":"resp_health","status":"in_progress","store":false,"output":[]}})];
        if scenario != ChatHealthScenario::Empty {
            events.extend([
                serde_json::json!({"type":"response.output_item.added","output_index":0,
                    "item":{"id":"msg_health","type":"message","role":"assistant","status":"in_progress","content":[]}}),
                serde_json::json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_health","content_index":0,
                    "part":{"type":"output_text","text":"","annotations":[]}}),
                serde_json::json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_health","content_index":0,"delta":"OK"}),
            ]);
        }
        if scenario == ChatHealthScenario::Partial {return anthropic_frames(events);}
        if scenario == ChatHealthScenario::Error {
            events.push(serde_json::json!({"type":"error","code":"invalid_request_error","message":format!("fixture stream failure {CHAT_KEY}")}));
        } else {
            let status = if scenario==ChatHealthScenario::Truncated {"incomplete"} else {"completed"};
            // The production decoder corroborates the terminal output against
            // each item's output_item.done snapshot, so emit the done event.
            let item = serde_json::json!({"id":"msg_health","type":"message","role":"assistant",
                "status":status,"content":[{"type":"output_text","text":"OK","annotations":[]}]});
            if scenario != ChatHealthScenario::Empty {
                events.push(serde_json::json!({"type":"response.output_item.done","output_index":0,
                    "item":item}));
            }
            let output = if scenario==ChatHealthScenario::Empty {vec![]} else {vec![item]};
            let mut response = serde_json::json!({"id":"resp_health","status":status,"store":false,
                "output":output,"usage":{"input_tokens":2,"output_tokens":1}});
            if scenario==ChatHealthScenario::Truncated {
                response["incomplete_details"] = serde_json::json!({"reason":"max_output_tokens"});
            }
            events.push(serde_json::json!({"type":format!("response.{status}"),"response":response}));
        }
        anthropic_frames(events)
    }

    fn gemini_health_stream(scenario: ChatHealthScenario) -> String {
        let mut events = vec![serde_json::json!({"candidates":[{"index":0,"content":{"role":"model",
            "parts":[{"text":if scenario==ChatHealthScenario::Empty {""} else {"OK"}}]}}],
            "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}})];
        if scenario == ChatHealthScenario::Partial {return data_sse_frames(events);}
        if scenario == ChatHealthScenario::Error {
            events.push(serde_json::json!({"error":{"code":400,"status":"INVALID_ARGUMENT","message":format!("fixture stream failure {CHAT_KEY}")}}));
        } else {
            events.push(serde_json::json!({"candidates":[{"index":0,
                "finishReason":if scenario==ChatHealthScenario::Truncated {"MAX_TOKENS"} else {"STOP"}}],
                "usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1}}));
        }
        data_sse_frames(events)
    }

    #[allow(clippy::too_many_arguments)]
    async fn assert_chat_health_scenarios(
        platform: &str, protocol: &str, auth_scheme: &str, endpoint_path: &str, auth_header: &str,
        result_marker: &'static str, tool_stream: fn() -> String, stream: fn(ChatHealthScenario) -> String,
    ) {
        for scenario in [ChatHealthScenario::Complete,ChatHealthScenario::Empty,ChatHealthScenario::Partial,
            ChatHealthScenario::Truncated,ChatHealthScenario::Error] {
            let server = MockServer::start().await;
            let auth_value = if auth_header=="authorization" {format!("Bearer {CHAT_KEY}")} else {CHAT_KEY.to_owned()};
            let request_path = endpoint_path.split('?').next().unwrap();
            let expected_requests = if scenario==ChatHealthScenario::Complete {2u64} else {1u64};
            Mock::given(method("POST")).and(path(request_path)).and(header(auth_header,auth_value.as_str()))
                .respond_with(TwoRoundSse {
                    first:if scenario==ChatHealthScenario::Complete {tool_stream()} else {stream(scenario)},
                    second:stream(ChatHealthScenario::Complete),marker:result_marker})
                .expect(expected_requests).mount(&server).await;
            let (service,capabilities) = managed_chat_health_fixture(&server,platform,protocol,auth_scheme,endpoint_path).await;
            let response = service.health_check(ProviderHealthCheckRequest {
                provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::Chat,
            }).await.unwrap();
            // Only the complete two-round tool exchange proves Agent support.
            // A well-formed answer that never uses the probe tool stays Unknown;
            // decoder/sequence violations are Unhealthy.
            let expected = match scenario {
                ChatHealthScenario::Complete => HealthStatus::Healthy,
                ChatHealthScenario::Partial | ChatHealthScenario::Error => HealthStatus::Unhealthy,
                ChatHealthScenario::Empty | ChatHealthScenario::Truncated => HealthStatus::Unknown,
            };
            assert_eq!(response.status,expected,"{protocol} {scenario:?}: {response:?}");
            if expected!=HealthStatus::Healthy {
                assert!(response.message.as_deref().is_some_and(|message| !message.is_empty()));
            }
            assert!(!serde_json::to_string(&response).unwrap().contains(CHAT_KEY));
            let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"chat").await.unwrap().unwrap().health.unwrap();
            assert!(!stored.contains(CHAT_KEY));
            assert_eq!(serde_json::from_str::<CapabilityHealth>(&stored).unwrap().status,expected,"{protocol} {scenario:?}");
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len() as u64,expected_requests,"{protocol} {scenario:?}");
            assert!(!requests[0].headers.contains_key("anthropic-version"));
            let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            match protocol {
                "openai.chat_text" => {
                    assert!(body["messages"].is_array());assert_eq!(body["model"],CHAT_MODEL);assert_eq!(body["stream"],true);
                    assert_eq!(body["tools"][0]["function"]["name"],"nomifun_protocol_echo");
                    assert_eq!(body["tool_choice"],"auto");
                }
                "openai.responses" => {
                    assert!(body["input"].is_array());assert_eq!(body["model"],CHAT_MODEL);assert_eq!(body["stream"],true);assert_eq!(body["store"],false);
                    assert_eq!(body["tools"][0]["name"],"nomifun_protocol_echo");
                }
                "gemini.generate_text" => {
                    assert!(body["contents"].is_array());
                    assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"],"nomifun_protocol_echo");
                    assert_eq!(requests[0].url.query_pairs().find(|(key,_)| key=="alt").map(|(_,value)| value.into_owned()).as_deref(),Some("sse"));
                    assert!(!requests[0].headers.contains_key("authorization"));
                }
                _ => unreachable!(),
            }
        }
    }

    #[tokio::test]
    async fn openai_chat_health_requires_a_complete_visible_non_truncated_response() {
        assert_chat_health_scenarios("openai","openai.chat_text","bearer","/v1/chat/completions","authorization",
            "\"tool_call_id\"",openai_chat_tool_stream,openai_chat_health_stream).await;
    }

    #[tokio::test]
    async fn responses_health_requires_a_complete_visible_non_truncated_response() {
        assert_chat_health_scenarios("openai","openai.responses","bearer","/v1/responses","authorization",
            "function_call_output",responses_tool_stream,responses_health_stream).await;
    }

    #[tokio::test]
    async fn gemini_health_requires_a_complete_visible_non_truncated_response() {
        assert_chat_health_scenarios("gemini","gemini.generate_text","header_key:x-goog-api-key",
            &format!("/v1beta/models/{CHAT_MODEL}:streamGenerateContent?alt=sse"),"x-goog-api-key",
            "functionResponse",gemini_tool_stream,gemini_health_stream).await;
    }

    #[tokio::test]
    async fn accepted_media_jobs_remain_unknown_without_failure_or_followup_requests() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"accepted-job","status":"queued"})))
            .expect(1).mount(&server).await;
        let (service,capabilities) = managed_task_health_fixture(&server,"openai","openai.videos","bearer","/v1/videos",ModelTask::VideoGeneration).await;
        let response = service.health_check(ProviderHealthCheckRequest {
            provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::VideoGeneration,
        }).await.unwrap();
        assert_eq!(response.status,HealthStatus::Unknown,"{response:?}");
        assert!(response.message.is_some());
        assert!(response.error_kind.is_none());
        assert!(response.http_status.is_none());
        let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"video_generation").await.unwrap().unwrap().health.unwrap();
        let stored: CapabilityHealth = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored.status,HealthStatus::Unknown);
        assert!(stored.error_kind.is_none());
        assert_eq!(server.received_requests().await.unwrap().len(),1,"health must not poll or download accepted media");
    }

    #[tokio::test]
    async fn empty_successful_audio_is_persisted_as_unhealthy() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(Vec::<u8>::new(),"audio/mpeg"))
            .expect(1).mount(&server).await;
        let (service,capabilities) = managed_task_health_fixture(&server,"openai","openai.audio_speech","bearer","/v1/audio/speech",ModelTask::SpeechSynthesis).await;
        let response = service.health_check(ProviderHealthCheckRequest {
            provider_id:CHAT_PROVIDER_ID.into(),model:CHAT_MODEL.into(),task:ModelTask::SpeechSynthesis,
        }).await.unwrap();
        assert_eq!(response.status,HealthStatus::Unhealthy,"{response:?}");
        assert!(response.error_kind.is_some());
        let stored = capabilities.get(CHAT_PROVIDER_ID,CHAT_MODEL,"speech_synthesis").await.unwrap().unwrap().health.unwrap();
        assert_eq!(serde_json::from_str::<CapabilityHealth>(&stored).unwrap().status,HealthStatus::Unhealthy);
    }

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

    /// `InvokeError`'s Display is `"{kind:?}: {message}"`, so the invoke layer's
    /// own typed kinds arrive as a `Kind:` prefix. These were falling through to
    /// `Unknown`, which is what made a local StepFun TTS parameter rejection and
    /// a real upstream outage look identical in model management.
    #[test]
    fn classifies_typed_invoke_error_kind_prefixes() {
        for (message, expected) in [
            (
                "InvalidParams: StepFun TTS requires a non-empty provider voice id",
                ProviderHealthCheckErrorKind::InvalidRequest,
            ),
            (
                "ParseError: StepFun ASR SSE produced no transcript (39 bytes)",
                ProviderHealthCheckErrorKind::ApiError,
            ),
            (
                "Config: resolved protocol has no injected submit endpoint",
                ProviderHealthCheckErrorKind::InvalidRequest,
            ),
            (
                "UnsupportedTask: adapter cannot serve task",
                ProviderHealthCheckErrorKind::InvalidRequest,
            ),
            (
                "Network: could not connect to upstream provider (dns error)",
                ProviderHealthCheckErrorKind::ConnectionError,
            ),
            (
                "Auth: provider rejected the api key",
                ProviderHealthCheckErrorKind::Unauthorized,
            ),
            (
                "QuotaExhausted: account balance is exhausted",
                ProviderHealthCheckErrorKind::InsufficientQuota,
            ),
        ] {
            assert_eq!(classify_error(message, false), expected, "message: {message}");
        }
    }

    /// A `Timeout` kind must not be reported as a generic connection error: the
    /// prose arm matches "request timed out" for both.
    #[test]
    fn timeout_kind_prefix_is_a_timeout_not_a_connection_error() {
        assert_eq!(
            classify_error("Timeout: upstream request timed out", false),
            ProviderHealthCheckErrorKind::Timeout
        );
    }

    /// A typed prefix must never outrank a MORE specific signal carried in the
    /// message. `error_from_response` classifies 400/422 as `InvalidParams`, so
    /// a 400 whose body says the account is out of credit still has to read as
    /// a quota problem ("top up") rather than a request problem ("fix your
    /// request"). Same for a `Config` error naming a missing provider.
    #[test]
    fn specific_message_signals_outrank_the_coarse_typed_prefix() {
        for (message, expected) in [
            (
                "InvalidParams: provider returned 400: {\"error\":{\"code\":\"insufficient_quota\"}}",
                ProviderHealthCheckErrorKind::InsufficientQuota,
            ),
            (
                "InvalidParams: provider returned 429: slow down",
                ProviderHealthCheckErrorKind::RateLimited,
            ),
            (
                "ParseError: provider returned 429 Too Many Requests",
                ProviderHealthCheckErrorKind::RateLimited,
            ),
            (
                "Config: provider not found: 019ff453",
                ProviderHealthCheckErrorKind::NotFound,
            ),
            (
                "InvalidParams: provider returned 401: invalid api key",
                ProviderHealthCheckErrorKind::Unauthorized,
            ),
            (
                "ProviderError: provider returned 404 Not Found: \
                 {\"error\":{\"code\":\"InvalidEndpointOrModel.NotFound\",\
                 \"message\":\"The model or endpoint Doubao-Seed-2.0-mini does not exist or you do not have access to it.\"}}",
                ProviderHealthCheckErrorKind::ModelUnavailable,
            ),
            (
                "InvalidParams: Ark video generation cannot use Doubao Seed chat model \
                 \"Doubao-Seed-2.0-mini\"; Seed and Seedance are different model families",
                ProviderHealthCheckErrorKind::ModelUnavailable,
            ),
            (
                "InvalidParams: Ark video model \"Doubao-Seedance-1.5-pro\" is a console \
                 display name, not an API Model ID; use \
                 \"doubao-seedance-1-5-pro-251215\" exactly",
                ProviderHealthCheckErrorKind::ModelUnavailable,
            ),
        ] {
            assert_eq!(classify_error(message, false), expected, "message: {message}");
        }
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
        let service = ProviderHealthCheckService::new(invoke);
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
