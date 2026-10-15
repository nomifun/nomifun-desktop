//! Setup-only protocol selection through the production streaming providers.
//! No catalog writes, Session, fallback policy or runtime route substitution.
use std::time::{Duration, Instant};
use nomi_config::compat::ProviderCompat;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_providers::openai::OpenAIProvider;
use nomi_providers::openai_responses::OpenAIResponsesProvider;
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomifun_api_types::{ChatProtocolDetectionAttempt, ChatProtocolDetectionStatus as Status,
    DetectChatProtocolAnonymousRequest, DetectChatProtocolRequest, DetectChatProtocolResponse,
    ModelTask, ProviderHealthCheckErrorKind as ErrorKind, ProviderModelCapabilityInput};
use nomifun_common::{AppError, ProviderId};
use nomifun_model_invoke::{AuthMaterial, AuthScheme, ResolvedConnection, protocol_task_descriptor, resolve_submit_url};
use serde_json::Value;
use super::ModelFetchService;

const RESPONSES: &str = "openai.responses";
const CHAT: &str = "openai.chat_text";
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_OUTPUT_LIMIT: u32 = 4096;

impl ModelFetchService {
    pub async fn detect_chat_protocol(
        &self, provider_id: &str, req: &DetectChatProtocolRequest,
        connections: &crate::provider_connection::ProviderConnectionService,
    ) -> Result<DetectChatProtocolResponse, AppError> {
        ProviderId::parse(provider_id).map_err(|error| AppError::BadRequest(error.to_string()))?;
        let row = self.repo.find_by_id(provider_id).await?.ok_or_else(|| AppError::NotFound("provider not found".into()))?;
        let revision = row.config_revision;
        let config = self.load_provider_config(provider_id).await?;
        let connection = if req.capability.connection_role == "default" {
            ResolvedConnection { role:"default".into(), base_url:config.base_url, auth:config.auth, extra:Value::Null }
        } else {
            connections.resolve_detection_connection(provider_id, &req.capability.connection_role).await?
        };
        let mut result = detect(&row.platform, &req.model, &req.capability, connection).await?;
        let current = self.repo.find_by_id(provider_id).await?;
        if current.is_none_or(|provider| provider.config_revision != revision) {
            return Err(AppError::Conflict("provider configuration changed during protocol detection; retry with the current draft".into()));
        }
        result.config_revision = Some(revision);
        Ok(result)
    }

    pub async fn detect_chat_protocol_anonymous(
        &self, req: &DetectChatProtocolAnonymousRequest,
    ) -> Result<DetectChatProtocolResponse, AppError> {
        if crate::provider::is_retired_provider_platform(req.platform.trim()) {
            return Err(AppError::Forbidden("retired provider cannot be tested".into()));
        }
        let (base_url, auth_scheme, credentials) = if req.capability.connection_role == "default" {
            (&req.base_url, &req.auth_scheme, &req.credentials)
        } else {
            let mut matching = req.connections.iter().filter(|connection| connection.role == req.capability.connection_role);
            let connection = matching.next().ok_or_else(|| AppError::BadRequest("selected named connection must be supplied before protocol detection".into()))?;
            if matching.next().is_some() { return Err(AppError::BadRequest("duplicate named connection role".into())); }
            (&connection.base_url, &connection.auth_scheme, &connection.credentials)
        };
        crate::provider::validate_provider_base_url(&req.platform, base_url)?;
        let auth_scheme = crate::provider::validate_provider_auth(&req.platform, auth_scheme, credentials, None)?;
        let connection = ResolvedConnection {
            role:req.capability.connection_role.clone(), base_url:base_url.clone(),
            auth:AuthMaterial { scheme:AuthScheme::parse(&auth_scheme).map_err(|error| AppError::BadRequest(error.message))?, credentials:credentials.clone() },
            extra:Value::Null,
        };
        detect(&req.platform, &req.model, &req.capability, connection).await
    }
}

async fn detect(platform: &str, model: &str, original: &ProviderModelCapabilityInput,
    connection: ResolvedConnection) -> Result<DetectChatProtocolResponse, AppError> {
    if model.trim().is_empty() || original.task != ModelTask::Chat || !matches!(original.protocol.trim(), CHAT | RESPONSES) {
        return Err(AppError::BadRequest("protocol detection requires an exact model and a Chat capability".into()));
    }
    if connection.auth.scheme != AuthScheme::Bearer {
        return Err(AppError::BadRequest("OpenAI protocol detection requires the selected connection's bearer authentication".into()));
    }
    connection.auth.validate_credentials().map_err(|error| AppError::BadRequest(error.message))?;
    let started = Instant::now();
    let first = match original.protocol.trim() { CHAT => CHAT, RESPONSES => RESPONSES, _ => {
        nomifun_model_invoke::model_profiles::recommended_chat_protocol(platform, Some(model))
            .filter(|protocol| matches!(*protocol, CHAT | RESPONSES)).unwrap_or(RESPONSES)
    }};
    let protocols = [first, if first == RESPONSES { CHAT } else { RESPONSES }];
    let mut result = DetectChatProtocolResponse { selected_protocol:None, selected_capability:None,
        attempts:Vec::new(), elapsed_ms:0, config_revision:None };
    for protocol in protocols {
        let attempt_started = Instant::now();
        let (candidate, output_limit) = match prepare_candidate(platform, model, original, protocol, &connection) {
            Ok(candidate) => candidate,
            Err(error) if result.attempts.is_empty() => return Err(error),
            Err(error) => {
                let mut attempt = failed_attempt(protocol, PROBE_OUTPUT_LIMIT, ErrorKind::InvalidRequest,
                    &error.to_string());
                attempt.elapsed_ms = millis(attempt_started);
                result.attempts.push(attempt);
                break;
            }
        };
        let outcome = tokio::time::timeout(ATTEMPT_TIMEOUT,
            check_candidate(model, &candidate, &connection, output_limit)).await;
        let mut attempt = match outcome {
            Ok(Ok(())) => ChatProtocolDetectionAttempt {protocol:protocol.into(),status:Status::Verified,
                probe_output_limit:output_limit,message:None,error_kind:None,http_status:None,elapsed_ms:0},
            Ok(Err(error)) => classify_error(protocol, output_limit, error, &connection.auth),
            Err(_) => failed_attempt(protocol, output_limit, ErrorKind::Timeout,
                "model verification timed out; this does not prove the protocol is unsupported"),
        };
        attempt.elapsed_ms = millis(attempt_started);
        let status = attempt.status;
        result.attempts.push(attempt);
        if status == Status::Verified {
            result.selected_protocol = Some(protocol.into());
            result.selected_capability = Some(candidate);
            break;
        }
        if status != Status::Incompatible { break; }
    }
    result.elapsed_ms = millis(started);
    Ok(result)
}

fn prepare_candidate(platform: &str, model: &str, original: &ProviderModelCapabilityInput,
    protocol: &str, connection: &ResolvedConnection) -> Result<(ProviderModelCapabilityInput, u32), AppError> {
    let mut candidate = candidate_capability(original, protocol)?;
    if original.protocol.trim() != protocol
        && original.base_url_override.as_deref().is_some_and(|root| {
            Some(root) == nomifun_model_invoke::model_profiles::documented_chat_base_url(platform, Some(model))
        }) {
        candidate.base_url_override = None;
    }
    if original.protocol.trim() != protocol && original.base_url_override.is_none() {
        let manifest = nomifun_model_invoke::protocol_manifest_for_model_connection(
            platform, Some(&connection.base_url), Some(model), ModelTask::Chat);
        if let Some(recommendation) = manifest.recommendation.filter(|recommendation|
            recommendation.protocol_id == protocol && recommendation.base_url_override_required) {
            candidate.base_url_override = recommendation.default_base_url;
        }
    }
    crate::provider_model::validate_protocol(platform, &candidate)?;
    crate::provider_model::validate_provider_params(protocol, ModelTask::Chat, &candidate.provider_params)?;
    crate::provider_model::validate_capability_urls(&candidate, &connection.base_url)?;
    crate::provider_model::validate_positive_token_limit("context_limit", candidate.context_limit)?;
    crate::provider_model::validate_positive_token_limit("output_limit", candidate.output_limit)?;
    crate::provider_model::validate_compaction_threshold(&candidate)?;
    let output_limit = candidate.output_limit.map(|limit| u32::try_from(limit).ok().filter(|limit| *limit > 0)
        .ok_or_else(|| AppError::BadRequest("output_limit must be positive and fit in u32".into())))
        .transpose()?.unwrap_or(PROBE_OUTPUT_LIMIT).min(PROBE_OUTPUT_LIMIT);
    Ok((candidate, output_limit))
}

fn candidate_capability(original: &ProviderModelCapabilityInput, protocol: &str) -> Result<ProviderModelCapabilityInput, AppError> {
    let mut candidate = original.clone();
    candidate.protocol = protocol.into();
    if original.protocol.trim() == protocol { return Ok(candidate); }
    if let Some(endpoint) = original.endpoint.as_deref() {
        let suffix_start = endpoint.find(['?', '#']).unwrap_or(endpoint.len());
        let (path, tail) = endpoint.split_at(suffix_start);
        let (old, new) = if protocol == CHAT { ("/responses", "/chat/completions") } else { ("/chat/completions", "/responses") };
        let path = path.trim_end_matches('/');
        let converted = if path == old.trim_start_matches('/') {
            new.trim_start_matches('/').to_owned()
        } else if let Some(root) = path.strip_suffix(old) {
            format!("{root}{new}")
        } else {
            return Err(AppError::BadRequest("custom endpoint cannot be safely converted to another protocol; configure its endpoint explicitly".into()));
        };
        candidate.endpoint = Some(format!("{converted}{tail}"));
    }
    let params = candidate.provider_params.as_object_mut().ok_or_else(|| AppError::BadRequest("provider_params must be an object".into()))?;
    for key in ["text", "response_format", "reasoning", "thinking"] {
        let has_policy = params.get(key).is_some_and(|value| match value {
            Value::Null => false,
            Value::Object(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            Value::String(value) => !value.trim().is_empty(),
            Value::Bool(_) | Value::Number(_) => true,
        });
        if has_policy {
            return Err(AppError::BadRequest(format!(
                "provider_params.{key} cannot be safely converted between Chat Completions and Responses; keep the current protocol or explicitly configure this policy for the other protocol"
            )));
        }
    }
    if protocol == CHAT {
        for key in ["chain_rounds", "include"] { params.remove(key); }
    } else {
        for key in ["stream_options", "require_reasoning_content"] { params.remove(key); }
    }
    if params.get("max_tokens_field").and_then(Value::as_str)
        .is_some_and(|field| matches!(field.trim(), "max_tokens" | "max_completion_tokens" | "max_output_tokens")) {
        params.remove("max_tokens_field");
    }
    Ok(candidate)
}

async fn check_candidate(model: &str, capability: &ProviderModelCapabilityInput,
    connection: &ResolvedConnection, output_limit: u32) -> Result<(), ProviderError> {
    let mut connection = connection.clone();
    if let Some(root) = &capability.base_url_override { connection.base_url = root.clone(); }
    let descriptor = protocol_task_descriptor(&capability.protocol, ModelTask::Chat).expect("validated Chat protocol");
    let endpoint = capability.endpoint.as_deref().unwrap_or_else(|| descriptor.endpoints.iter()
        .find(|endpoint| endpoint.field == "endpoint").expect("HTTP Chat endpoint").default_value.as_str());
    let url = resolve_submit_url(&connection, &capability.protocol, ModelTask::Chat, endpoint, model,
        capability.allow_cross_origin_credentials).map_err(|error| ProviderError::Config(error.message))?;
    let mut compat = if capability.protocol == RESPONSES { ProviderCompat::openai_responses_defaults() }
        else { ProviderCompat::openai_defaults() };
    let mut params = capability.provider_params.as_object().expect("validated object").clone();
    compat.max_tokens_field = params.remove("max_tokens_field").and_then(|value| value.as_str().map(str::trim).map(str::to_owned)).or(compat.max_tokens_field);
    compat.require_reasoning_content = params.remove("require_reasoning_content").and_then(|value| value.as_bool());
    compat.chain_rounds = params.remove("chain_rounds").and_then(|value| value.as_bool());
    compat.reasoning_effort = params.remove("reasoning_effort").and_then(|value| value.as_str().map(str::to_owned));
    params.remove("_nomifun_context_limit_kind");
    params.remove(nomifun_api_types::MODEL_GATEWAY_CATALOG_BASELINE_PARAM);
    compat.api_path = Some(String::new());
    compat.extra_body = Some(params);
    let keys = connection.auth.secrets().join("\n");
    let provider: Box<dyn LlmProvider> = if capability.protocol == RESPONSES {
        Box::new(OpenAIResponsesProvider::new(&keys, &url, compat))
    } else { Box::new(OpenAIProvider::new(&keys, &url, compat)) };
    let request = LlmRequest {model:model.to_owned(),system:"Reply with exactly OK. Do not use tools.".into(),
        messages:vec![Message::new(Role::User,vec![ContentBlock::Text{text:"Reply OK.".into()}])],
        tools:Vec::new(),max_tokens:Some(output_limit),thinking:None,reasoning_effort:None,retain_provider_round:false};
    let mut stream = provider.stream(&request).await?;
    let mut visible = false;
    while let Some(event) = stream.recv().await {
        match event {
            LlmEvent::TextDelta(text) => visible |= !text.trim().is_empty(),
            LlmEvent::Done {stop_reason:StopReason::EndTurn,..} if visible => return Ok(()),
            LlmEvent::Done {stop_reason:StopReason::MaxTokens,..} => return Err(ProviderError::Config(
                "verification output budget exhausted; protocol support remains inconclusive".into())),
            LlmEvent::Done {..} => return Err(ProviderError::Config("verification completed without a visible answer".into())),
            LlmEvent::Error(message) => return Err(stream_error(message)),
            _ => {}
        }
    }
    Err(ProviderError::Connection("verification stream closed without a terminal result".into()))
}

fn stream_error(message: String) -> ProviderError {
    if let Some(message) = message.strip_prefix("SSE parse error: ") { return ProviderError::Parse(message.to_owned()); }
    if let Some(rest) = message.strip_prefix("API error ") {
        if let Some((status, message)) = rest.split_once(": ") {
            if let Ok(status) = status.parse() { return ProviderError::Api {status,message:message.to_owned()}; }
        }
    }
    ProviderError::Connection(message)
}

fn classify_error(protocol: &str, output_limit: u32, error: ProviderError, auth: &AuthMaterial) -> ChatProtocolDetectionAttempt {
    let mut attempt = failed_attempt(protocol, output_limit, ErrorKind::ApiError, "model verification failed");
    match &error {
        ProviderError::Parse(_) => { attempt.status = Status::Incompatible; attempt.error_kind = Some(ErrorKind::InvalidRequest); }
        ProviderError::Api {status,message} => {
            attempt.http_status = Some(*status);
            attempt.error_kind = Some(match status {401 => ErrorKind::Unauthorized,403 => ErrorKind::Forbidden,
                404 => ErrorKind::NotFound,429 => ErrorKind::RateLimited,
                status if *status >= 500 => ErrorKind::ApiError,_ => ErrorKind::InvalidRequest});
            // The production Responses provider appends its own endpoint hint
            // to a 404 body. Decode the complete leading JSON value so that a
            // model/auth error keeps its machine evidence despite that hint.
            let body = serde_json::Deserializer::from_str(message).into_iter::<Value>()
                .next().and_then(Result::ok);
            let code = body.as_ref().and_then(|body| body.pointer("/error/code").or_else(|| body.get("code"))
                .or_else(|| body.pointer("/error/type")))
                .and_then(Value::as_str).unwrap_or_default();
            if matches!(code,"invalid_api_key"|"invalid_access_token"|"authentication_error"|"unauthorized") { attempt.error_kind = Some(ErrorKind::Unauthorized); }
            else if matches!(code,"access_denied"|"forbidden"|"account_suspended") { attempt.error_kind = Some(ErrorKind::Forbidden); }
            else if matches!(code,"insufficient_quota"|"quota_exceeded"|"insufficient_balance") { attempt.error_kind = Some(ErrorKind::InsufficientQuota); }
            else if matches!(code,"model_not_found"|"model_not_available"|"model_not_supported")
                || body.as_ref().and_then(|body| body.pointer("/error/param")).and_then(Value::as_str) == Some("model") {
                attempt.error_kind = Some(ErrorKind::ModelUnavailable);
            }
            else if matches!(*status,405|410|501)
                || (*status == 404 && (matches!(code,"endpoint_not_found"|"route_not_found")
                    || body.as_ref().and_then(|body| body.get("detail")).and_then(Value::as_str) == Some("Not Found")))
                || (matches!(*status,400|422) && matches!(code,"unsupported_protocol"|"unsupported_endpoint"|"unsupported_parameter"|"unknown_parameter"|"not_implemented")) {
                attempt.status = Status::Incompatible;
            }
        }
        ProviderError::RateLimited {..} => attempt.error_kind = Some(ErrorKind::RateLimited),
        ProviderError::Http(error) if error.is_timeout() => attempt.error_kind = Some(ErrorKind::Timeout),
        ProviderError::Connection(_) | ProviderError::Http(_) | ProviderError::StreamTruncated(_) => attempt.error_kind = Some(ErrorKind::ConnectionError),
        ProviderError::NonApiResponse {..} => attempt.error_kind = Some(ErrorKind::NonApiResponse),
        ProviderError::Config(_) | ProviderError::PromptTooLong(_) => attempt.error_kind = Some(ErrorKind::InvalidRequest),
    }
    let message = if matches!(error,ProviderError::Http(_)) { "model verification transport failed".to_owned() }
        else { nomifun_net::secret_redaction::SecretRedactor::new(auth.secrets())
            .redact(&error.to_string()).chars().take(350).collect() };
    attempt.message = Some(message);
    attempt
}

fn failed_attempt(protocol: &str, output_limit: u32, error_kind: ErrorKind, message: &str) -> ChatProtocolDetectionAttempt {
    ChatProtocolDetectionAttempt {protocol:protocol.into(),status:Status::Failed,probe_output_limit:output_limit,
        message:Some(message.into()),error_kind:Some(error_kind),http_status:None,elapsed_ms:0}
}
fn millis(started: Instant) -> u64 { started.elapsed().as_millis().try_into().unwrap_or(u64::MAX) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reported_stepfun_parse_failure_is_incompatible_only_during_setup() {
        let error = stream_error("SSE parse error: Responses content part does not match its message item".into());
        assert!(matches!(error, ProviderError::Parse(_)));
        let auth = AuthMaterial { scheme:AuthScheme::Bearer, credentials:serde_json::json!({"api_keys":["fixture"]}) };
        let attempt = classify_error(RESPONSES,4096,error,&auth);
        assert_eq!(attempt.status,Status::Incompatible);
    }

    fn zhipu_chat_draft() -> ProviderModelCapabilityInput {
        serde_json::from_value(serde_json::json!({"task":"chat","protocol":CHAT,
            "connection_role":"default","provider_params":{}})).unwrap()
    }

    fn bearer_connection(root: &str) -> ResolvedConnection {
        ResolvedConnection { role:"default".into(), base_url:root.into(),
            auth:AuthMaterial {scheme:AuthScheme::Bearer,credentials:serde_json::json!({"api_keys":["fixture"]})},
            extra:Value::Null }
    }

    #[test]
    fn official_zhipu_connection_switch_uses_the_documented_responses_root() {
        let original = zhipu_chat_draft();
        let connection = bearer_connection("https://open.bigmodel.cn/api/paas/v4");
        let (candidate, _) = prepare_candidate("zhipu", "glm-5.3", &original, RESPONSES, &connection).unwrap();
        assert_eq!(candidate.base_url_override.as_deref(),Some("https://open.bigmodel.cn/api/v1"));
        assert!(original.base_url_override.is_none());
    }

    #[test]
    fn switching_protocol_preserves_custom_relays_and_explicit_root_overrides() {
        let original = zhipu_chat_draft();
        let relay = bearer_connection("https://relay.example/v1");
        let (candidate, _) = prepare_candidate("zhipu", "glm-5.3", &original, RESPONSES, &relay).unwrap();
        assert!(candidate.base_url_override.is_none());
        assert_eq!(relay.base_url,"https://relay.example/v1");
        let mut explicit = original;
        explicit.base_url_override = Some("https://relay.example/custom".into());
        let (candidate, _) = prepare_candidate("zhipu", "glm-5.3", &explicit, RESPONSES, &relay).unwrap();
        assert_eq!(candidate.base_url_override,explicit.base_url_override);
    }
}
