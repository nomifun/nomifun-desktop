//! Shared Chat protocol verification over the exact Agent wire stack.
//!
//! Protocol discovery and the saved-capability health check both run
//! [`verify_chat_agent_round_trip`]: broker serializers plus
//! `merge_chat_provider_params`, one [`SingleAttemptHttpExecutor`] request per
//! round with SSE or AWS event-stream framing, and a fresh
//! `new_verification_frame_decoder` per round that applies the broker's
//! canonical event-sequence rules. There is no retry and no second wire stack.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use nomifun_agent_contracts::StrictJsonValue;
use nomifun_api_types::ProviderHealthCheckErrorKind;
use nomifun_chat_model_broker::{
    ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelErrorCode,
    ChatModelEvent, ChatModelInput, ChatProtocol, ChatResponseFormat, ChatRole,
    ChatToolCall, ChatToolChoice, ChatToolDefinition, ChatToolResultPart, PromptCachePolicy,
    ProviderWireFrame, ToolCallId, chat_protocol_for_id, encode_verification_chat_input,
    new_verification_frame_decoder,
};

use crate::auth::AuthMaterial;
use crate::call::ResolvedTaskConfig;
use crate::chat_executor::{
    OpaqueCredentialLease, OpaqueCredentialResolver, SingleAttemptFraming,
    SingleAttemptHttpExecutor, SingleAttemptRequest, SingleAttemptResponseBudget,
};
use crate::error::{GatewayBusinessError, InvokeError, InvokeErrorKind, ModelFailureReason};

/// The harmless echo tool every Chat verification round trip offers. The model
/// must call it once with `{"value":"ok"}` before the exchange sends the local
/// result back and requires a final visible answer.
pub const CHAT_VERIFICATION_TOOL: &str = "nomifun_protocol_echo";

const VERIFICATION_INSTRUCTIONS: &str = "This is an isolated protocol verification. First call nomifun_protocol_echo once with value ok. After the tool result, return a short visible answer. Do not call any other tools.";
const VERIFICATION_PROMPT: &str = "Verify the echo tool, then reply OK.";
const VERIFICATION_TOOL_RESULT: &str = "{\"value\":\"ok\"}";
/// Buffered semantic text+reasoning bound for one verification round.
const MAX_BUFFERED_BYTES: usize = 256 * 1024;
/// Raw provider response byte bound per request, matching standalone probing.
const RESPONSE_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// One fully resolved Chat verification target: the exact URL, credentials,
/// provider parameters and output ceiling the saved capability would use.
pub struct ChatVerificationTarget {
    pub protocol_id: String,
    /// The complete submit endpoint this exchange posts to.
    pub url: String,
    pub model: String,
    /// Decrypted credential material. It leaves the target only through the
    /// opaque executor lease and is never written to diagnostics.
    pub auth: AuthMaterial,
    pub provider_params: Value,
    /// The capability's own output ceiling; the probe request stays bounded by
    /// it through the same `merge_chat_provider_params` path Agent turns use.
    pub configured_output_limit: Option<i64>,
    /// Required for AWS SigV4-backed Bedrock requests.
    pub region: Option<String>,
}

impl ChatVerificationTarget {
    /// Build the target from one resolved task capability, deriving the
    /// Bedrock URL/region exactly like `knowledge_writeback_completion.rs`.
    pub fn from_resolved(config: &ResolvedTaskConfig) -> Result<Self, InvokeError> {
        let (url, region) =
            if chat_protocol_for_id(&config.protocol) == Some(ChatProtocol::Bedrock) {
                let region = config
                    .bedrock_config
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .and_then(|value| {
                        value.get("region").and_then(Value::as_str).map(str::to_owned)
                    })
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| InvokeError::config("Bedrock signing region is missing"))?;
                (
                    format!(
                        "https://bedrock-runtime.{region}.amazonaws.com/model/{}/invoke-with-response-stream",
                        config.model
                    ),
                    Some(region),
                )
            } else {
                (config.http_endpoint()?, None)
            };
        Ok(Self {
            protocol_id: config.protocol.clone(),
            url,
            model: config.model.clone(),
            auth: config.connection.auth.clone(),
            provider_params: config.provider_params.clone(),
            configured_output_limit: config.output_limit,
            region,
        })
    }
}

/// Why a verification exchange did not complete.
#[derive(Debug)]
pub enum ChatVerificationFailure {
    /// The exchange could not be encoded or dispatched (local contract or
    /// configuration fault); no protocol conclusion exists.
    Request(ChatModelError),
    /// HTTP transport, credential, or non-2xx failure from the executor.
    Transport(InvokeError),
    /// The provider stream or its decoded event sequence violated the wire
    /// contract Agent turns would enforce.
    Stream(ChatModelError),
    /// Generation worked but the exchange did not prove Agent tool support.
    Inconclusive(&'static str),
    /// The exchange deadline elapsed.
    Timeout,
}

/// Typed classification of one verification failure. `incompatible` marks
/// evidence that this endpoint/protocol cannot serve the exchange at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatVerificationDiagnosis {
    pub error_kind: ProviderHealthCheckErrorKind,
    pub http_status: Option<u16>,
    pub incompatible: bool,
}

impl ChatVerificationFailure {
    /// Redacted presentation text for operator surfaces.
    pub fn message(&self) -> String {
        match self {
            Self::Request(error) | Self::Stream(error) => error.message.clone(),
            Self::Transport(error) => error.to_string(),
            Self::Inconclusive(message) => (*message).to_owned(),
            Self::Timeout => {
                "verification timed out; protocol support remains inconclusive".to_owned()
            }
        }
    }

    /// Typed diagnosis for both callers; `None` only for [`Self::Inconclusive`].
    pub fn diagnosis(&self) -> Option<ChatVerificationDiagnosis> {
        match self {
            Self::Inconclusive(_) => None,
            Self::Timeout => Some(ChatVerificationDiagnosis {
                error_kind: ProviderHealthCheckErrorKind::Timeout,
                http_status: None,
                incompatible: false,
            }),
            Self::Request(_) => Some(ChatVerificationDiagnosis {
                error_kind: ProviderHealthCheckErrorKind::InvalidRequest,
                http_status: None,
                incompatible: false,
            }),
            Self::Transport(error) => Some(diagnose_transport(error)),
            Self::Stream(error) => Some(diagnose_stream(error)),
        }
    }
}

impl std::fmt::Display for ChatVerificationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for ChatVerificationFailure {}

use ProviderHealthCheckErrorKind as HealthKind;

/// Shared reason→kind mapping: diagnostic reason first, then HTTP status,
/// then the error's own kind/code.
fn reason_kind(reason: ModelFailureReason) -> HealthKind {
    match reason {
        ModelFailureReason::AuthFailed
        | ModelFailureReason::InvalidKey
        | ModelFailureReason::ExpiredKey
        | ModelFailureReason::CredentialsMissing => HealthKind::Unauthorized,
        ModelFailureReason::AuthSchemeMismatch => HealthKind::InvalidAuthorizationHeader,
        ModelFailureReason::PermissionDenied => HealthKind::Forbidden,
        ModelFailureReason::ModelPermissionDenied
        | ModelFailureReason::ModelNotFound
        | ModelFailureReason::ModelNotInPlan => HealthKind::ModelUnavailable,
        ModelFailureReason::BillingRequired
        | ModelFailureReason::InsufficientQuota
        | ModelFailureReason::InsufficientBalance
        | ModelFailureReason::SpendLimitReached
        | ModelFailureReason::SubscriptionExpired => HealthKind::InsufficientQuota,
        ModelFailureReason::RateLimited => HealthKind::RateLimited,
        ModelFailureReason::NonApiResponse => HealthKind::NonApiResponse,
        ModelFailureReason::DnsFailure
        | ModelFailureReason::ConnectionFailed
        | ModelFailureReason::TlsFailure
        | ModelFailureReason::ProxyFailure
        | ModelFailureReason::NetworkFailure
        | ModelFailureReason::StreamInterrupted => HealthKind::ConnectionError,
        ModelFailureReason::RequestTimeout => HealthKind::Timeout,
        ModelFailureReason::EndpointMissing | ModelFailureReason::InvalidEndpoint => {
            HealthKind::NotFound
        }
        ModelFailureReason::UpstreamServerError
        | ModelFailureReason::ProviderOverloaded
        | ModelFailureReason::ProviderUnavailable
        | ModelFailureReason::InvalidResponse => HealthKind::ApiError,
        ModelFailureReason::InvalidRequest
        | ModelFailureReason::UnsupportedFeature
        | ModelFailureReason::PromptTooLong
        | ModelFailureReason::ContentPolicy
        | ModelFailureReason::ConfigurationError
        | ModelFailureReason::CredentialTargetMismatch => HealthKind::InvalidRequest,
    }
}

fn status_kind(status: u16) -> Option<HealthKind> {
    Some(match status {
        401 => HealthKind::Unauthorized,
        402 => HealthKind::InsufficientQuota,
        403 => HealthKind::Forbidden,
        404 => HealthKind::NotFound,
        429 => HealthKind::RateLimited,
        400 | 422 => HealthKind::InvalidRequest,
        status if status >= 500 => HealthKind::ApiError,
        _ => return None,
    })
}

fn invoke_error_kind(kind: InvokeErrorKind) -> HealthKind {
    match kind {
        InvokeErrorKind::Timeout => HealthKind::Timeout,
        InvokeErrorKind::Network => HealthKind::ConnectionError,
        InvokeErrorKind::Auth => HealthKind::Unauthorized,
        InvokeErrorKind::RateLimited => HealthKind::RateLimited,
        InvokeErrorKind::QuotaExhausted => HealthKind::InsufficientQuota,
        InvokeErrorKind::NonApiResponse => HealthKind::NonApiResponse,
        InvokeErrorKind::InvalidParams
        | InvokeErrorKind::Config
        | InvokeErrorKind::MissingConnection
        | InvokeErrorKind::NoAdapter
        | InvokeErrorKind::UnsupportedTask => HealthKind::InvalidRequest,
        InvokeErrorKind::ParseError
        | InvokeErrorKind::ProviderError
        | InvokeErrorKind::JobFailed
        | InvokeErrorKind::ContentPolicy => HealthKind::ApiError,
        _ => HealthKind::Unknown,
    }
}

fn chat_model_error_kind(code: ChatModelErrorCode) -> HealthKind {
    match code {
        ChatModelErrorCode::AuthenticationFailed => HealthKind::Unauthorized,
        ChatModelErrorCode::RateLimited => HealthKind::RateLimited,
        ChatModelErrorCode::ProviderUnavailable => HealthKind::ApiError,
        ChatModelErrorCode::InvalidRequest
        | ChatModelErrorCode::UnsupportedFeature
        | ChatModelErrorCode::PromptTooLong => HealthKind::InvalidRequest,
        ChatModelErrorCode::ProtocolViolation => HealthKind::ApiError,
        ChatModelErrorCode::StreamInterrupted => HealthKind::ConnectionError,
        _ => HealthKind::Unknown,
    }
}

/// Wire-contract incompatibility evidence shared by both failure channels.
fn is_incompatible_status(
    http_status: Option<u16>,
    reason: Option<ModelFailureReason>,
    provider_code: Option<&str>,
) -> bool {
    match http_status {
        Some(405 | 410 | 501) => true,
        Some(404) => {
            reason == Some(ModelFailureReason::EndpointMissing)
                || matches!(provider_code, Some("endpoint_not_found" | "route_not_found"))
        }
        Some(400 | 422) => {
            reason == Some(ModelFailureReason::UnsupportedFeature)
                || matches!(
                    provider_code,
                    Some(
                        "unsupported_protocol"
                            | "unsupported_endpoint"
                            | "unsupported_parameter"
                            | "unknown_parameter"
                            | "not_implemented"
                    )
                )
        }
        _ => false,
    }
}

fn diagnose_transport(error: &InvokeError) -> ChatVerificationDiagnosis {
    let diagnostic = error.diagnostic.as_ref();
    let http_status = error
        .http_status
        .or_else(|| diagnostic.and_then(|d| d.http_status));
    let reason = diagnostic.map(|d| d.reason);
    let provider_code = diagnostic.and_then(|d| d.provider_code.as_deref());
    let error_kind = if let Some(business) = error.gateway_business_error {
        match business {
            GatewayBusinessError::InsufficientBalance
            | GatewayBusinessError::SubscriptionExpired => HealthKind::InsufficientQuota,
            GatewayBusinessError::ModelNotInPlan => HealthKind::ModelUnavailable,
            GatewayBusinessError::KeyExpired => HealthKind::Unauthorized,
            GatewayBusinessError::RateLimited => HealthKind::RateLimited,
        }
    } else if let Some(reason) = reason {
        // A document body is a wrong-address symptom, but an auth/quota/rate
        // status still carries its stop authority even for HTML responses.
        if reason == ModelFailureReason::NonApiResponse
            && let Some(status) = http_status
            && matches!(status, 401 | 402 | 403 | 429)
        {
            status_kind(status).unwrap()
        } else {
            reason_kind(reason)
        }
    } else if let Some(kind) = http_status.and_then(status_kind) {
        kind
    } else {
        invoke_error_kind(error.kind)
    };
    ChatVerificationDiagnosis {
        error_kind,
        http_status,
        incompatible: error.kind == InvokeErrorKind::ParseError
            || is_incompatible_status(http_status, reason, provider_code),
    }
}

fn diagnose_stream(error: &ChatModelError) -> ChatVerificationDiagnosis {
    let diagnostic = error.diagnostic.as_ref();
    let http_status = error
        .provider_status
        .or_else(|| diagnostic.and_then(|d| d.http_status));
    let reason = diagnostic.map(|d| d.reason);
    let provider_code = diagnostic.and_then(|d| d.provider_code.as_deref());
    let error_kind = if let Some(reason) = reason {
        reason_kind(reason)
    } else if let Some(kind) = http_status.and_then(status_kind) {
        kind
    } else {
        chat_model_error_kind(error.code)
    };
    ChatVerificationDiagnosis {
        error_kind,
        http_status,
        incompatible: matches!(
            error.code,
            ChatModelErrorCode::ProtocolViolation | ChatModelErrorCode::UnsupportedFeature
        ) || is_incompatible_status(http_status, reason, provider_code),
    }
}

/// The credential material for exactly one request: resolvers never see the
/// response and the lease handle carries no secret material.
struct OneRequestCredential {
    material: AuthMaterial,
    used: AtomicBool,
}

#[async_trait]
impl OpaqueCredentialResolver for OneRequestCredential {
    async fn resolve(&self, lease: &OpaqueCredentialLease) -> Result<AuthMaterial, InvokeError> {
        if lease.handle() != "chat-verification" {
            return Err(InvokeError::config("credential lease is not bound to verification"));
        }
        if self.used.swap(true, Ordering::SeqCst) {
            return Err(InvokeError::config(
                "verification credential lease was resolved more than once",
            ));
        }
        Ok(self.material.clone())
    }
}

fn verification_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        name: CHAT_VERIFICATION_TOOL.to_owned(),
        description:
            "A harmless local protocol test that returns the constant ok; no external side effects."
                .to_owned(),
        input_schema: StrictJsonValue(json!({
            "type": "object",
            "properties": {"value": {"type": "string", "enum": ["ok"]}},
            "required": ["value"],
            "additionalProperties": false
        })),
        deferred: false,
    }
}

fn verification_input(messages: Vec<ChatMessage>, probe_output_tokens: u32) -> ChatModelInput {
    ChatModelInput {
        instructions: vec![VERIFICATION_INSTRUCTIONS.to_owned()],
        messages,
        tools: vec![verification_tool()],
        tool_choice: ChatToolChoice::Auto,
        parallel_tool_calls: None,
        max_output_tokens: Some(probe_output_tokens),
        reasoning: None,
        prompt_cache: PromptCachePolicy::Disabled,
        response_format: ChatResponseFormat::Text,
        requested_output_modalities: Default::default(),
        provider_round_parent: None,
        preserve_native_responses_items: false,
        metadata: Default::default(),
    }
}

fn user_message() -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: vec![ChatContentPart::Text {
            text: VERIFICATION_PROMPT.to_owned(),
        }],
        provider_round_id: None,
    }
}

struct PendingVerificationCall {
    name: String,
    arguments: String,
}

/// One decoded round: the assistant parts to replay, the completed tool calls
/// in emission order, the finish reason and the accumulated visible text.
struct VerificationRound {
    parts: Vec<ChatContentPart>,
    calls: Vec<ChatToolCall>,
    finish_reason: ChatFinishReason,
    text: String,
}

/// Replays decoder output with the same assembly rules as the Agent runtime's
/// `StepState`: reasoning signatures attach to their block, tool deltas must
/// equal the completed arguments, and every proposed call must complete.
#[derive(Default)]
struct RoundState {
    parts: Vec<ChatContentPart>,
    pending: BTreeMap<ToolCallId, PendingVerificationCall>,
    calls: Vec<ChatToolCall>,
    pending_reasoning_signature: Option<String>,
    buffered_bytes: usize,
    text: String,
    rejected: bool,
    finish_reason: Option<ChatFinishReason>,
}

impl RoundState {
    fn append_text(&mut self, text: &str) {
        self.text.push_str(text);
        if let Some(ChatContentPart::Text { text: existing }) = self.parts.last_mut() {
            existing.push_str(text);
        } else {
            self.parts.push(ChatContentPart::Text { text: text.to_owned() });
        }
    }

    fn append_reasoning(&mut self, text: &str) {
        if let Some(ChatContentPart::Reasoning {
            text: existing,
            signature: None,
            encrypted_content: None,
        }) = self.parts.last_mut()
        {
            existing.push_str(text);
        } else {
            self.parts.push(ChatContentPart::Reasoning {
                text: text.to_owned(),
                signature: self.pending_reasoning_signature.take(),
                encrypted_content: None,
            });
        }
    }

    fn set_reasoning_signature(&mut self, signature: String) -> Result<(), ChatVerificationFailure> {
        let multiple = || {
            ChatVerificationFailure::Stream(ChatModelError::protocol_violation(
                "provider emitted more than one reasoning signature",
            ))
        };
        if let Some(ChatContentPart::Reasoning {
            signature: existing,
            text,
            ..
        }) = self.parts.last_mut()
        {
            if existing.is_some() {
                return Err(multiple());
            }
            if text.is_empty() {
                self.pending_reasoning_signature = Some(signature);
            } else {
                *existing = Some(signature);
            }
        } else {
            if self.pending_reasoning_signature.is_some() {
                return Err(multiple());
            }
            self.pending_reasoning_signature = Some(signature);
        }
        Ok(())
    }

    fn record_tool_delta(
        &mut self,
        call_id: &ToolCallId,
        name: &str,
        arguments_delta: &str,
    ) -> Result<(), ChatVerificationFailure> {
        let violation = |message: &str| {
            ChatVerificationFailure::Stream(ChatModelError::protocol_violation(message))
        };
        if self.calls.iter().any(|call| call.call_id == *call_id) {
            return Err(violation("verification tool call emitted a delta after completion"));
        }
        if !self.pending.contains_key(call_id) && self.pending.len() + self.calls.len() >= 2 {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification exceeded its bounded response budget; Agent support remains inconclusive",
            ));
        }
        let entry = self.pending.entry(call_id.clone()).or_insert_with(|| {
            PendingVerificationCall { name: String::new(), arguments: String::new() }
        });
        if !name.is_empty() {
            if !entry.name.is_empty() && entry.name != name {
                return Err(violation("verification tool call changed its name"));
            }
            entry.name = name.to_owned();
        }
        entry.arguments.push_str(arguments_delta);
        Ok(())
    }

    fn record_tool_completed(&mut self, call: &ChatToolCall) -> Result<(), ChatVerificationFailure> {
        if let Some(pending) = self.pending.remove(&call.call_id)
            && !pending.arguments.is_empty()
        {
            let assembled: Value = serde_json::from_str(&pending.arguments).map_err(|_| {
                ChatVerificationFailure::Stream(ChatModelError::protocol_violation(
                    "verification tool call assembled invalid arguments",
                ))
            })?;
            if assembled != call.arguments.0 {
                return Err(ChatVerificationFailure::Stream(
                    ChatModelError::protocol_violation(
                        "verification tool call arguments differ from their deltas",
                    ),
                ));
            }
        }
        if self.calls.iter().any(|completed| completed.call_id == call.call_id) {
            return Err(ChatVerificationFailure::Stream(ChatModelError::protocol_violation(
                "verification tool call completed more than once",
            )));
        }
        if !self.calls.is_empty() {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification exceeded its bounded response budget; Agent support remains inconclusive",
            ));
        }
        self.parts.push(ChatContentPart::ToolCall {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            provider_metadata: call.provider_metadata.clone(),
        });
        self.calls.push(call.clone());
        Ok(())
    }

    fn observe(&mut self, event: &ChatModelEvent) -> Result<(), ChatVerificationFailure> {
        match event {
            ChatModelEvent::ResponseStarted { .. } | ChatModelEvent::Usage { .. } => {}
            ChatModelEvent::OutputTextDelta { text, .. } => {
                self.buffered_bytes += text.len();
                self.append_text(text);
            }
            ChatModelEvent::OutputAudioDelta { data_base64, .. } => {
                self.buffered_bytes += data_base64.len();
            }
            ChatModelEvent::ReasoningDelta { text } => {
                self.buffered_bytes += text.len();
                self.append_reasoning(text);
            }
            ChatModelEvent::ReasoningSignature { signature } => {
                self.buffered_bytes += signature.len();
                self.set_reasoning_signature(signature.clone())?;
            }
            ChatModelEvent::ReasoningBlock { text, encrypted_content } => {
                self.buffered_bytes +=
                    text.len() + encrypted_content.as_deref().map_or(0, str::len);
                self.parts.push(ChatContentPart::Reasoning {
                    text: text.clone(),
                    signature: None,
                    encrypted_content: encrypted_content.clone(),
                });
            }
            ChatModelEvent::ProviderReasoningBlock { block } => {
                self.buffered_bytes += block.visible_text().as_deref().map_or(0, str::len);
                self.parts.push(ChatContentPart::ProviderReasoning { block: block.clone() });
            }
            ChatModelEvent::ToolCallDelta {
                call_id,
                name,
                arguments_delta,
            } => {
                self.buffered_bytes += arguments_delta.len();
                self.record_tool_delta(call_id, name, arguments_delta)?;
            }
            ChatModelEvent::ToolCallCompleted { call } => self.record_tool_completed(call)?,
            ChatModelEvent::ToolCallRejected { .. } => self.rejected = true,
            // Retained-round markers are never replayed; verification sends no
            // provider_round_parent, so these carry no authority here.
            ChatModelEvent::ProviderRoundId { .. }
            | ChatModelEvent::NativeResponsesItem { .. } => {}
            ChatModelEvent::Completed { finish_reason } => self.finish_reason = Some(*finish_reason),
        }
        if self.buffered_bytes > MAX_BUFFERED_BYTES {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification exceeded its bounded response budget; Agent support remains inconclusive",
            ));
        }
        Ok(())
    }

    /// The same completion rules as `StepState::finalize`: every proposed call
    /// completed and no dangling signature or empty reasoning part remains.
    fn finalize(self) -> Result<VerificationRound, ChatVerificationFailure> {
        if !self.pending.is_empty() || self.pending_reasoning_signature.is_some()
            || self.parts.iter().any(|part| matches!(part,
                ChatContentPart::Reasoning { text, encrypted_content, .. }
                    if text.is_empty() && encrypted_content.is_none()))
        {
            return Err(ChatVerificationFailure::Stream(ChatModelError::protocol_violation(
                "provider stream ended with uncommitted reasoning or tool state",
            )));
        }
        Ok(VerificationRound {
            parts: self.parts,
            calls: self.calls,
            finish_reason: self.finish_reason.expect("finalize runs after Completed"),
            text: self.text,
        })
    }
}

async fn run_round(
    target: &ChatVerificationTarget,
    protocol: ChatProtocol,
    messages: Vec<ChatMessage>,
    probe_output_tokens: u32,
    deadline: Duration,
    started: tokio::time::Instant,
) -> Result<VerificationRound, ChatVerificationFailure> {
    let body = encode_verification_chat_input(
        protocol,
        &target.model,
        &verification_input(messages, probe_output_tokens),
        &target.provider_params,
        target.configured_output_limit,
    )
    .map_err(ChatVerificationFailure::Request)?;
    let http = nomifun_net::http_client_no_redirect()
        .map_err(|error| ChatVerificationFailure::Transport(InvokeError::network(&error)))?;
    let executor = SingleAttemptHttpExecutor::new(
        http,
        Arc::new(OneRequestCredential {
            material: target.auth.clone(),
            used: AtomicBool::new(false),
        }),
    )
    .with_response_budget(
        SingleAttemptResponseBudget::new(RESPONSE_BUDGET_BYTES)
            .map_err(ChatVerificationFailure::Transport)?,
    );
    let remaining = deadline.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(ChatVerificationFailure::Timeout);
    }
    let request = SingleAttemptRequest {
        protocol: target.protocol_id.clone(),
        url: target.url.clone(),
        model: target.model.clone(),
        body,
        credential: OpaqueCredentialLease::new("chat-verification")
            .map_err(ChatVerificationFailure::Transport)?,
        timeout: remaining,
        idle_timeout: remaining,
        framing: if protocol == ChatProtocol::Bedrock {
            SingleAttemptFraming::AwsEventStream
        } else {
            SingleAttemptFraming::Sse
        },
        region: target.region.clone(),
    };
    let mut stream = executor
        .open_stream(request)
        .await
        .map_err(ChatVerificationFailure::Transport)?;
    let mut decoder = new_verification_frame_decoder(protocol);
    let mut state = RoundState::default();
    while let Some(item) = stream.next().await {
        let frame = item.map_err(ChatVerificationFailure::Transport)?;
        let events = decoder
            .decode_frame(ProviderWireFrame {
                event: frame.event,
                data: frame.data,
                diagnostic: frame.diagnostic,
            })
            .map_err(ChatVerificationFailure::Stream)?;
        for event in &events {
            state.observe(event)?;
        }
        // Stop at the canonical terminal like the broker; never drain to EOF.
        if state.finish_reason.is_some() {
            if state.rejected {
                return Err(ChatVerificationFailure::Inconclusive(
                    "text generation succeeded without the verification tool; Agent support remains inconclusive",
                ));
            }
            return state.finalize();
        }
    }
    Err(ChatVerificationFailure::Stream(ChatModelError::stream_interrupted(
        "provider stream ended before a canonical terminal event",
    )))
}

/// Run the bounded two-round verification exchange against one Chat target:
/// the model must call [`CHAT_VERIFICATION_TOOL`] once with `{"value":"ok"}`,
/// receive the local tool result, and produce a final visible answer. Success
/// proves the exact Agent wire path (serializer, parameter merge, transport,
/// decoder, sequence rules) works against this endpoint and model.
pub async fn verify_chat_agent_round_trip(
    target: &ChatVerificationTarget,
    probe_output_tokens: u32,
    deadline: Duration,
) -> Result<(), ChatVerificationFailure> {
    let started = tokio::time::Instant::now();
    match tokio::time::timeout(deadline, verify_exchange(target, probe_output_tokens, deadline, started)).await {
        // The executor's per-attempt timeout is the remaining exchange
        // deadline, so whichever layer observes it first reports `Timeout`.
        Ok(Err(ChatVerificationFailure::Transport(error)))
            if error.kind == InvokeErrorKind::Timeout =>
        {
            Err(ChatVerificationFailure::Timeout)
        }
        Ok(result) => result,
        Err(_) => Err(ChatVerificationFailure::Timeout),
    }
}

async fn verify_exchange(
    target: &ChatVerificationTarget,
    probe_output_tokens: u32,
    deadline: Duration,
    started: tokio::time::Instant,
) -> Result<(), ChatVerificationFailure> {
    let protocol = chat_protocol_for_id(&target.protocol_id).ok_or_else(|| {
        ChatVerificationFailure::Request(ChatModelError::new(
            ChatModelErrorCode::AdapterUnavailable,
            "configured protocol has no Chat verification adapter",
            nomifun_chat_model_broker::ChatRetryDirective::Never,
        ))
    })?;
    let first = run_round(
        target,
        protocol,
        vec![user_message()],
        probe_output_tokens,
        deadline,
        started,
    )
    .await?;
    match first.finish_reason {
        ChatFinishReason::Completed | ChatFinishReason::ToolCalls => {}
        ChatFinishReason::MaxOutputTokens => {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification output budget exhausted; protocol support remains inconclusive",
            ));
        }
        _ => {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification ended without a normal terminal result",
            ));
        }
    }
    let Some(call) = first.calls.first() else {
        return Err(ChatVerificationFailure::Inconclusive(
            "text generation succeeded without the verification tool; Agent support remains inconclusive",
        ));
    };
    if call.name != CHAT_VERIFICATION_TOOL || call.arguments.0 != json!({"value": "ok"}) {
        return Err(ChatVerificationFailure::Inconclusive(
            "model did not produce the bounded verification tool call; Agent support remains inconclusive",
        ));
    }
    let second = run_round(
        target,
        protocol,
        vec![
            user_message(),
            ChatMessage {
                role: ChatRole::Assistant,
                content: first.parts,
                provider_round_id: None,
            },
            ChatMessage {
                role: ChatRole::Tool,
                content: vec![ChatContentPart::ToolResult {
                    call_id: call.call_id.clone(),
                    output: vec![ChatToolResultPart::Text {
                        text: VERIFICATION_TOOL_RESULT.to_owned(),
                    }],
                    is_error: false,
                }],
                provider_round_id: None,
            },
        ],
        probe_output_tokens,
        deadline,
        started,
    )
    .await?;
    match second.finish_reason {
        ChatFinishReason::Completed => {}
        ChatFinishReason::MaxOutputTokens => {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification output budget exhausted; protocol support remains inconclusive",
            ));
        }
        _ => {
            return Err(ChatVerificationFailure::Inconclusive(
                "verification ended without a normal terminal result",
            ));
        }
    }
    if !second.calls.is_empty() || second.text.trim().is_empty() {
        return Err(ChatVerificationFailure::Inconclusive(
            "tool verification completed without a final visible answer; Agent support remains inconclusive",
        ));
    }
    Ok(())
}
