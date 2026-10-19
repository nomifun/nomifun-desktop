//! Stateless text conversion: one resolved model, one HTTP request and no tools.
//!
//! The shared Chat serializers/decoders are pure codecs here. This path creates
//! no Agent session, retained conversation, retry loop or project configuration.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use nomifun_api_types::{BedrockAuthMethod, BedrockConfig, ModelTask};
use nomifun_chat_model_broker::{
    ChatContentPart, ChatFinishReason, ChatMessage, ChatModelEvent, ChatModelInput,
    ChatProtocol, ChatReasoningRequest, ChatResponseFormat, ChatRole, ChatToolChoice,
    PromptCachePolicy, ProviderWireFrame, ReasoningEffort, ReasoningSummary,
    chat_protocol_for_id, encode_standalone_chat_input, new_standalone_frame_decoder,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::{
    AuthMaterial, InvokeError, InvokeErrorKind, ModelInvokeService, ModelRef,
    OpaqueCredentialLease, OpaqueCredentialResolver, ResolvedTaskConfig,
    SingleAttemptFraming, SingleAttemptHttpExecutor, SingleAttemptRequest,
    SingleAttemptResponseBudget, protocol_supports_reasoning_effort_value,
};

/// Only the current source and transformation instructions enter the request.
/// Limits are caller-owned; the saved model ceiling can only lower them.
pub struct TextTransformRequest<'a> {
    pub system: &'a str,
    pub body: &'a str,
    pub max_output_tokens: u32,
    pub max_output_utf8_bytes: usize,
    /// One absolute deadline including local resolution, connection and output.
    pub timeout: Duration,
}

/// Durations are measured from entry, rather than summing overlapping stages.
/// Unknown provider usage remains absent. No text or provider payload is logged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct TextTransformMetrics {
    pub resolve_ms: u64,
    pub headers_ms: Option<u64>,
    pub first_frame_ms: Option<u64>,
    pub first_text_ms: Option<u64>,
    pub total_ms: u64,
    pub output_bytes: u64,
    pub wire_bytes: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

pub struct TextTransformResult {
    pub text: String,
    pub metrics: TextTransformMetrics,
}

impl ModelInvokeService {
    /// Execute exactly one tool-free transformation. Cancellation drops the
    /// active request/stream, so no detached task can emit a late candidate.
    pub async fn transform_text(
        &self,
        selection: &ModelRef,
        request: TextTransformRequest<'_>,
        cancel: CancellationToken,
        mut on_delta: impl FnMut(&str) + Send,
    ) -> Result<TextTransformResult, InvokeError> {
        validate_request(&request)?;
        let started = Instant::now();
        let deadline = started + request.timeout;
        let mut metrics = TextTransformMetrics::default();
        let operation = async {
            let config = self.resolve_task_config(selection, ModelTask::Chat).await?;
            metrics.resolve_ms = elapsed_ms(started);
            execute(&self.http, &config, &request, started, deadline, &mut metrics, |delta| {
                if !cancel.is_cancelled() { on_delta(delta); }
            }).await
        };
        let result = within_deadline(&cancel, deadline, operation).await;
        metrics.total_ms = elapsed_ms(started);
        log_metrics(&metrics, if result.is_ok() { 0 } else if cancel.is_cancelled() { 1 }
            else if result.as_ref().is_err_and(|e| e.kind == InvokeErrorKind::Timeout) { 2 } else { 3 });
        result.map(|text| TextTransformResult { text, metrics })
    }
}

fn validate_request(request: &TextTransformRequest<'_>) -> Result<(), InvokeError> {
    if request.body.trim().is_empty() || request.system.trim().is_empty()
        || request.max_output_tokens == 0 || request.max_output_utf8_bytes == 0
        || request.max_output_utf8_bytes > 1024 * 1024
        || request.body.len() > 1024 * 1024 || request.system.len() > 16 * 1024
        || request.timeout.is_zero() || request.timeout > Duration::from_secs(120)
    {
        return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "文字转换输入或限制无效"));
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn cancelled() -> InvokeError {
    // The document owner treats its cancellation token as authoritative and
    // discards this result; it must never commit an interrupted candidate.
    InvokeError::new(InvokeErrorKind::InvalidParams, "文字转换已取消")
}

async fn within_deadline<T>(
    cancel: &CancellationToken,
    deadline: Instant,
    operation: impl std::future::Future<Output = Result<T, InvokeError>>,
) -> Result<T, InvokeError> {
    let result = tokio::select! { biased;
        _ = cancel.cancelled() => Err(cancelled()),
        result = tokio::time::timeout_at(deadline, operation) =>
            result.unwrap_or_else(|_| Err(InvokeError::new(InvokeErrorKind::Timeout, "文字转换超时"))),
    };
    if cancel.is_cancelled() {
        Err(cancelled())
    } else if Instant::now() >= deadline {
        // Tokio's timeout cannot interrupt one synchronous poll. Even a ready
        // result must not commit after crossing the absolute deadline.
        Err(InvokeError::new(InvokeErrorKind::Timeout, "文字转换超时"))
    } else {
        result
    }
}

fn log_metrics(metrics: &TextTransformMetrics, outcome: u8) {
    tracing::info!(target: "nomifun_text_transform",
        outcome, resolve_ms = metrics.resolve_ms, headers_ms = metrics.headers_ms.unwrap_or_default(),
        first_frame_ms = metrics.first_frame_ms.unwrap_or_default(),
        first_text_ms = metrics.first_text_ms.unwrap_or_default(), total_ms = metrics.total_ms,
        output_bytes = metrics.output_bytes, wire_bytes = metrics.wire_bytes,
        input_tokens = metrics.input_tokens.unwrap_or_default(),
        output_tokens = metrics.output_tokens.unwrap_or_default(),
        reasoning_tokens = metrics.reasoning_tokens.unwrap_or_default(),
        usage_reported = u8::from(metrics.output_tokens.is_some()),
        reasoning_usage_reported = u8::from(metrics.reasoning_tokens.is_some()), "text conversion timing");
}

fn text_input(config: &ResolvedTaskConfig, request: &TextTransformRequest<'_>) -> ChatModelInput {
    let documented = crate::model_profiles::documented_reasoning_efforts(
        &config.platform, &config.model, &config.protocol);
    let effort = [ReasoningEffort::None, ReasoningEffort::Minimal, ReasoningEffort::Low]
        .into_iter().find(|effort| documented.is_some_and(|values| values.iter().any(|value| value == effort.as_str()))
            && protocol_supports_reasoning_effort_value(&config.protocol, effort.as_str()));
    ChatModelInput {
        instructions: vec![request.system.to_owned()],
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: request.body.to_owned() }],
            provider_round_id: None,
        }],
        tools: Vec::new(),
        tool_choice: ChatToolChoice::None,
        parallel_tool_calls: None,
        max_output_tokens: Some(request.max_output_tokens),
        reasoning: effort.map(|effort| ChatReasoningRequest {
            effort: Some(effort), summary: ReasoningSummary::None, max_reasoning_tokens: None,
        }),
        prompt_cache: PromptCachePolicy::Disabled,
        response_format: ChatResponseFormat::Text,
        requested_output_modalities: Default::default(),
        provider_round_parent: None,
        preserve_native_responses_items: false,
        metadata: Default::default(),
    }
}

fn request_target(config: &ResolvedTaskConfig, protocol: ChatProtocol) -> Result<(String, Option<String>), InvokeError> {
    if protocol != ChatProtocol::Bedrock {
        return Ok((config.http_endpoint()?, None));
    }
    let region = bedrock_metadata(config)?.region;
    let model = url::form_urlencoded::byte_serialize(config.model.as_bytes()).collect::<String>();
    Ok((format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model}/invoke-with-response-stream"), Some(region)))
}

fn bedrock_metadata(config: &ResolvedTaskConfig) -> Result<BedrockConfig, InvokeError> {
    let mut metadata: BedrockConfig = config.bedrock_config.as_deref()
        .and_then(|value| serde_json::from_str(value).ok())
        .ok_or_else(|| InvokeError::config("Bedrock authentication metadata is missing or invalid"))?;
    metadata.region = metadata.region.trim().to_owned();
    metadata.profile = metadata.profile.map(|profile| profile.trim().to_owned());
    if metadata.region.is_empty() || !metadata.region.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        || match metadata.auth_method {
            BedrockAuthMethod::Profile => metadata.profile.as_deref().is_none_or(str::is_empty),
            BedrockAuthMethod::AccessKey | BedrockAuthMethod::DefaultChain => metadata.profile.is_some(),
        }
    {
        return Err(InvokeError::config("Bedrock signing region or authentication profile is invalid"));
    }
    Ok(metadata)
}

/// An absent plan means the resolved material is already ready for signing.
/// Only an explicitly selected Bedrock profile/chain may invoke the AWS SDK.
fn sdk_credential_plan(config: &ResolvedTaskConfig) -> Result<Option<BedrockConfig>, InvokeError> {
    if config.protocol != "bedrock.anthropic_messages" { return Ok(None); }
    if config.connection.auth.scheme != crate::AuthScheme::Bedrock {
        return Err(InvokeError::config("Bedrock requires its declared authentication scheme"));
    }
    config.connection.auth.validate_credentials()?;
    let metadata = bedrock_metadata(config)?;
    let empty = config.connection.auth.credentials.as_object().is_some_and(serde_json::Map::is_empty);
    match metadata.auth_method {
        BedrockAuthMethod::AccessKey if !empty => Ok(None),
        BedrockAuthMethod::Profile | BedrockAuthMethod::DefaultChain if empty => Ok(Some(metadata)),
        _ => Err(InvokeError::config("Bedrock credentials do not match the selected authentication method")),
    }
}

async fn sdk_credentials(metadata: BedrockConfig) -> Result<aws_credential_types::Credentials, InvokeError> {
    use aws_credential_types::provider::ProvideCredentials;
    let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(metadata.region));
    if let Some(profile) = metadata.profile { loader = loader.profile_name(profile); }
    let sdk = loader.load().await;
    let provider = sdk.credentials_provider()
        .ok_or_else(|| InvokeError::config("AWS credential provider is not configured"))?;
    provider.provide_credentials().await
        .map_err(|_| InvokeError::new(InvokeErrorKind::Auth, "AWS credentials could not be resolved"))
}

async fn resolve_transform_auth<F, Fut>(config: &ResolvedTaskConfig, provide: F) -> Result<AuthMaterial, InvokeError>
where
    F: FnOnce(BedrockConfig) -> Fut,
    Fut: std::future::Future<Output = Result<aws_credential_types::Credentials, InvokeError>>,
{
    let Some(metadata) = sdk_credential_plan(config)? else { return Ok(config.connection.auth.clone()); };
    let credentials = provide(metadata).await?;
    let mut material = serde_json::json!({
        "access_key_id": credentials.access_key_id(), "secret_access_key": credentials.secret_access_key(),
    });
    if let Some(token) = credentials.session_token() { material["session_token"] = token.into(); }
    let auth = AuthMaterial { scheme: crate::AuthScheme::Bedrock, credentials: material };
    auth.validate_credentials()?;
    Ok(auth)
}

struct OneRequestCredential {
    material: AuthMaterial,
    used: AtomicBool,
}

#[async_trait]
impl OpaqueCredentialResolver for OneRequestCredential {
    async fn resolve(&self, lease: &OpaqueCredentialLease) -> Result<AuthMaterial, InvokeError> {
        if lease.handle() != "text-transform" || self.used.swap(true, Ordering::AcqRel) {
            return Err(InvokeError::config("text transformation credential lease is single use"));
        }
        Ok(self.material.clone())
    }
}

async fn execute(
    http: &reqwest::Client,
    config: &ResolvedTaskConfig,
    request: &TextTransformRequest<'_>,
    started: Instant,
    deadline: Instant,
    metrics: &mut TextTransformMetrics,
    on_delta: impl FnMut(&str) + Send,
) -> Result<String, InvokeError> {
    let protocol = chat_protocol_for_id(&config.protocol)
        .ok_or_else(|| InvokeError::config("selected model has no standalone Chat codec"))?;
    let redactor = config.connection.auth.secret_redactor();
    let input = text_input(config, request);
    let mut body = encode_standalone_chat_input(protocol, &config.model, &input,
        &config.provider_params, config.output_limit)
        .map_err(|error| InvokeError::parse(error.message).redacted(&redactor))?;
    if protocol == ChatProtocol::Gemini && input.prompt_cache == PromptCachePolicy::Disabled {
        // A remote cache reference can carry prior content even when the
        // explicit messages contain only this source. This API disables it;
        // other Chat consumers retain their own cache policy and merge rules.
        if let Some(fields) = body.as_object_mut() {
            fields.remove("cachedContent");
            fields.remove("cached_content");
        }
    }
    let (url, region) = request_target(config, protocol)?;
    // Profile/default-chain refresh is inside the owning operation's absolute
    // deadline and cancellation boundary, without a nested runtime or fallback.
    let auth = resolve_transform_auth(config, sdk_credentials).await?;
    let redactor = auth.secret_redactor();
    let wire_limit = request.max_output_utf8_bytes.saturating_mul(8).saturating_add(256 * 1024);
    let budget = SingleAttemptResponseBudget::new(wire_limit)?;
    let executor = SingleAttemptHttpExecutor::new(http.clone(), Arc::new(OneRequestCredential {
        material: auth, used: AtomicBool::new(false),
    })).with_response_budget(budget.clone());
    let remaining = deadline.saturating_duration_since(Instant::now());
    let stream = executor.open_stream(SingleAttemptRequest {
        protocol: config.protocol.clone(), url, model: config.model.clone(), body,
        credential: OpaqueCredentialLease::new("text-transform")?,
        timeout: remaining, idle_timeout: remaining,
        framing: if protocol == ChatProtocol::Bedrock { SingleAttemptFraming::AwsEventStream } else { SingleAttemptFraming::Sse },
        region,
    }).await?;
    metrics.headers_ms = Some(elapsed_ms(started));
    drain(stream, protocol, request.max_output_utf8_bytes, started, metrics, &budget, &redactor, on_delta).await
}

async fn drain(
    mut stream: crate::SingleAttemptStream,
    protocol: ChatProtocol,
    max_output_utf8_bytes: usize,
    started: Instant,
    metrics: &mut TextTransformMetrics,
    budget: &SingleAttemptResponseBudget,
    redactor: &nomifun_net::secret_redaction::SecretRedactor,
    mut on_delta: impl FnMut(&str) + Send,
) -> Result<String, InvokeError> {
    let mut decoder = new_standalone_frame_decoder(protocol);
    let mut output = String::new();
    let mut response_started = false;
    while let Some(item) = stream.next().await {
        let frame = item?;
        metrics.wire_bytes = budget.received_bytes();
        metrics.first_frame_ms.get_or_insert_with(|| elapsed_ms(started));
        let events = decoder.decode_frame(ProviderWireFrame {
            event: frame.event, data: frame.data, diagnostic: frame.diagnostic,
        }).map_err(|error| InvokeError::parse(error.message).redacted(redactor))?;
        let mut completed = false;
        for event in events {
            if completed {
                return Err(InvokeError::parse("文字转换结束帧包含迟到事件"));
            }
            match event {
                ChatModelEvent::ResponseStarted { .. } => {
                    if response_started { return Err(InvokeError::parse("文字转换返回了重复的响应")); }
                    response_started = true;
                }
                ChatModelEvent::OutputTextDelta { text, .. } => {
                    if !output.len().checked_add(text.len()).is_some_and(|size| size <= max_output_utf8_bytes) {
                        return Err(InvokeError::parse("文字转换结果超过长度限制"));
                    }
                    if !text.is_empty() { metrics.first_text_ms.get_or_insert_with(|| elapsed_ms(started)); }
                    output.push_str(&text);
                    metrics.output_bytes = output.len() as u64;
                    on_delta(&text);
                }
                ChatModelEvent::Usage { usage } => {
                    metrics.input_tokens = Some(usage.input_tokens);
                    metrics.output_tokens = Some(usage.output_tokens);
                    // Canonical usage represents an omitted reasoning count
                    // as zero. Without native presence evidence, zero cannot
                    // prove that this response did no private reasoning.
                    metrics.reasoning_tokens = (usage.reasoning_tokens > 0).then_some(usage.reasoning_tokens);
                }
                ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed } => completed = true,
                ChatModelEvent::Completed { .. } => return Err(InvokeError::parse("文字转换未完整完成，已保留原文")),
                ChatModelEvent::ReasoningDelta { .. } | ChatModelEvent::ReasoningSignature { .. }
                | ChatModelEvent::ReasoningBlock { .. } | ChatModelEvent::ProviderReasoningBlock { .. } => {}
                ChatModelEvent::ToolCallDelta { .. } | ChatModelEvent::ToolCallCompleted { .. }
                | ChatModelEvent::ToolCallRejected { .. } | ChatModelEvent::OutputAudioDelta { .. }
                | ChatModelEvent::ProviderRoundId { .. } | ChatModelEvent::NativeResponsesItem { .. } =>
                    return Err(InvokeError::parse("文字转换返回了不允许的工具、媒体或会话状态")),
            }
        }
        if completed {
            if output.trim().is_empty() { return Err(InvokeError::parse("文字转换没有返回正文")); }
            // A canonical protocol terminal is the completion boundary. The
            // stream is dropped now, even if the HTTP connection stays open.
            return Ok(output);
        }
    }
    Err(InvokeError::parse("文字转换连接结束时没有完整完成标记"))
}

#[cfg(test)]
#[path = "text_transform_tests.rs"]
mod tests;
