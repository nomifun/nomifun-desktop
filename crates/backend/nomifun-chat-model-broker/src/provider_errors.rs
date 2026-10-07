//! Protocol error envelopes, not model/tool text. Keep provider prose out of
//! canonical errors and classify only explicit machine-readable identifiers.
use serde_json::{Map, Value};
use nomifun_net::provider_capability::{
    ProviderTechnicalCapability, classify_unsupported_technical_capability,
};
use nomifun_net::provider_gateway_error::{GatewayBusinessError, classify_gateway_business_error};

use crate::{ChatModelError, ChatModelErrorCode, ChatProtocol, ChatRetryDirective};
use crate::contracts::{ModelFailureDiagnostic, ModelFailureReason};
use crate::ChatModelFeature;

fn malformed() -> ChatModelError {
    let mut failure = ChatModelError::protocol_violation(
        "provider error envelope is malformed or contains undispatched output; automatic replay is not allowed",
    );
    failure.diagnostic = Some(ModelFailureDiagnostic::new(ModelFailureReason::InvalidResponse));
    failure
}

/// Transport context has already passed the credential-aware redactor. Native
/// parsing refines only the reason; it must not replace those safe identifiers
/// with opaque strings copied out of the unredacted provider envelope.
pub(crate) fn decode_with_context(
    protocol: ChatProtocol,
    event: &str,
    data: &Value,
    context: Option<&ModelFailureDiagnostic>,
) -> Option<ChatModelError> {
    let mut failure = decode(protocol, event, data)?;
    if let Some(context) = context {
        let mut diagnostic = context.clone();
        if let Some(native) = &failure.diagnostic {
            if native.reason != ModelFailureReason::ProviderUnavailable {
                diagnostic.reason = native.reason;
            }
        }
        failure.diagnostic = Some(diagnostic);
    }
    Some(failure)
}

fn fallback_reason(code: ChatModelErrorCode) -> ModelFailureReason {
    use ChatModelErrorCode as Code;
    use ModelFailureReason as Reason;
    match code {
        Code::AuthenticationFailed => Reason::AuthFailed,
        Code::RateLimited => Reason::RateLimited,
        Code::PromptTooLong => Reason::PromptTooLong,
        Code::InvalidRequest => Reason::InvalidRequest,
        Code::UnsupportedFeature => Reason::UnsupportedFeature,
        Code::ProtocolViolation => Reason::InvalidResponse,
        Code::StreamInterrupted => Reason::StreamInterrupted,
        _ => Reason::ProviderUnavailable,
    }
}

fn known_identifier(value: &str) -> bool {
    ModelFailureReason::from_machine_code(value).is_some()
        || classify(value).is_some()
        || matches!(value,
            "unsupported_parameter" | "unsupported_feature" | "not_supported"
            | "function_calling_not_supported" | "tool_calls_not_supported" | "tools_not_supported"
            | "unsupported_function_calling" | "unsupported_tool_calls" | "unsupported_tools"
            | "reasoning_not_supported" | "thinking_not_supported" | "unsupported_reasoning"
            | "unsupported_thinking" | "streaming_not_supported" | "stream_not_supported"
            | "unsupported_streaming" | "unsupported_stream")
}

fn google_error_info_reason(
    protocol: ChatProtocol,
    envelope: &Map<String, Value>,
    generic: ModelFailureReason,
) -> Option<ModelFailureReason> {
    if protocol != ChatProtocol::Gemini {
        return None;
    }
    ModelFailureReason::refine_google_error_info(envelope, generic)
}

fn diagnose(protocol: ChatProtocol, mut failure: ChatModelError, envelope: &Map<String, Value>) -> ChatModelError {
    let reason = ["code", "type", "status"].into_iter()
        .filter_map(|field| envelope.get(field).and_then(Value::as_str))
        .find_map(ModelFailureReason::from_machine_code)
        .unwrap_or_else(|| fallback_reason(failure.code));
    let reason = google_error_info_reason(protocol, envelope, reason).unwrap_or(reason);
    let mut diagnostic = ModelFailureDiagnostic::new(reason);
    diagnostic.provider_code = envelope.get("code").and_then(Value::as_str)
        .filter(|value| known_identifier(value)).map(str::to_owned);
    diagnostic.provider_type = envelope.get("type").and_then(Value::as_str)
        .filter(|value| known_identifier(value)).map(str::to_owned);
    // No credential redactor is available here. Even apparently well-formed
    // opaque request IDs may contain credentials and must not be copied.
    failure.diagnostic = Some(diagnostic);
    failure
}

fn has_output(value: &Value) -> bool {
    value.as_object().is_some_and(has_output_object)
}

fn has_output_object(value: &Map<String, Value>) -> bool {
    [
        "output",
        "choices",
        "candidates",
        "delta",
        "text",
        "output_text",
        "content",
        "tool_calls",
        "usage",
        "usageMetadata",
    ]
    .iter()
    .any(|key| {
        value.get(*key).is_some_and(|part| match part {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => !items.is_empty(),
            // A non-null usage object (even all zeroes) is semantic metadata,
            // and objects in output fields must not be silently ignored.
            _ => true,
        })
    })
}

fn classify(code: &str) -> Option<(ChatModelErrorCode, ChatRetryDirective, &'static str)> {
    use ChatModelErrorCode as Code;
    use ChatRetryDirective as Retry;
    Some(match code {
        "context_length_exceeded" | "prompt_too_long" => (
            Code::PromptTooLong,
            Retry::Never,
            "provider rejected the input context length",
        ),
        "rate_limit_exceeded" | "rate_limit_error" => (
            Code::RateLimited,
            Retry::RetrySameRoute,
            "provider rate limit rejected the attempt",
        ),
        "overloaded_error" | "server_error" | "internal_server_error" => (
            Code::ProviderUnavailable,
            Retry::RetrySameRoute,
            "provider is temporarily unavailable",
        ),
        "invalid_api_key" | "authentication_error" | "permission_error" => (
            Code::AuthenticationFailed,
            Retry::Never,
            "provider rejected request authentication or permission",
        ),
        "invalid_request_error" | "invalid_argument" | "INVALID_ARGUMENT" | "invalid_prompt"
        | "unsupported_value" | "invalid_value" => (
            Code::InvalidRequest,
            Retry::Never,
            "provider rejected the request parameters",
        ),
        "insufficient_quota" | "quota_exceeded" | "usage_not_included" => (
            Code::ProviderUnavailable,
            Retry::Never,
            "provider account quota or usage entitlement rejected the attempt",
        ),
        "content_policy_violation"
        | "cyber_policy"
        | "bio_policy"
        | "misalignment_policy_violation" => (
            Code::UnsupportedFeature,
            Retry::Never,
            "provider policy rejected the request",
        ),
        _ => return None,
    })
}

pub(crate) fn gateway_error(business: GatewayBusinessError) -> ChatModelError {
    let (code, retry) = match business {
        GatewayBusinessError::InsufficientBalance
        | GatewayBusinessError::SubscriptionExpired
        | GatewayBusinessError::ModelNotInPlan => (ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::Never),
        GatewayBusinessError::KeyExpired => (ChatModelErrorCode::AuthenticationFailed, ChatRetryDirective::Never),
        GatewayBusinessError::RateLimited => (ChatModelErrorCode::RateLimited, ChatRetryDirective::RetrySameRoute),
    };
    let reason = match business {
        GatewayBusinessError::InsufficientBalance => ModelFailureReason::InsufficientBalance,
        GatewayBusinessError::SubscriptionExpired => ModelFailureReason::SubscriptionExpired,
        GatewayBusinessError::ModelNotInPlan => ModelFailureReason::ModelNotInPlan,
        GatewayBusinessError::KeyExpired => ModelFailureReason::ExpiredKey,
        GatewayBusinessError::RateLimited => ModelFailureReason::RateLimited,
    };
    let mut failure = ChatModelError::new(code, business.action_message(), retry);
    let mut diagnostic = ModelFailureDiagnostic::new(reason);
    diagnostic.provider_code = Some(business.code().to_owned());
    failure.diagnostic = Some(diagnostic);
    failure
}

/// None means this is not a recognized error envelope: normal protocol
/// decoding must still process it. Nested tool arguments/results are never
/// searched. The transport has already bounded and parsed the complete frame.
pub(crate) fn decode(protocol: ChatProtocol, event: &str, data: &Value) -> Option<ChatModelError> {
    if event == "bedrock.exception" {
        if protocol != ChatProtocol::Bedrock
            || !data.as_object().is_some_and(|object| object.len() == 1)
            || !data
                .get("error")
                .and_then(Value::as_object)
                .is_some_and(|object| object.len() == 1)
        {
            return Some(malformed());
        }
        let Some(code) = data
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
        else {
            return Some(malformed());
        };
        use ChatModelErrorCode as Code;
        use ChatRetryDirective as Retry;
        let (code, retry, message) = match code {
            "throttlingException" | "ThrottlingException" => (
                Code::RateLimited,
                Retry::RetrySameRoute,
                "Bedrock rate limit rejected the attempt",
            ),
            "internalServerException"
            | "InternalServerException"
            | "serviceUnavailableException"
            | "ServiceUnavailableException" => (
                Code::ProviderUnavailable,
                Retry::RetrySameRoute,
                "Bedrock is temporarily unavailable",
            ),
            "accessDeniedException"
            | "AccessDeniedException"
            | "UnrecognizedClientException"
            | "InvalidSignatureException" => (
                Code::AuthenticationFailed,
                Retry::Never,
                "Bedrock rejected request authentication or permission",
            ),
            "validationException" | "ValidationException" => (
                Code::InvalidRequest,
                Retry::Never,
                "Bedrock rejected request parameters",
            ),
            "resourceNotFoundException" | "ResourceNotFoundException" => (
                Code::UnsupportedFeature,
                Retry::Never,
                "Bedrock model resource is unavailable",
            ),
            "modelTimeoutException"
            | "ModelTimeoutException"
            | "modelStreamErrorException"
            | "ModelStreamErrorException" => (
                Code::ProviderUnavailable,
                Retry::Never,
                "Bedrock model execution did not complete",
            ),
            _ => (
                Code::ProviderUnavailable,
                Retry::Never,
                "Bedrock returned an unclassified stream exception",
            ),
        };
        // The Broker still prohibits replay after committed semantic output.
        // ValidationException alone does not prove a context-length overflow.
        return Some(diagnose(protocol, ChatModelError::new(code, message, retry),
            data.get("error").and_then(Value::as_object).expect("validated exception")));
    }
    let named_error = event == "error" || event.ends_with(".error");
    let generic_frame = matches!(event, "message" | "json");
    let declared = data.get("type").and_then(Value::as_str);
    let response_failed = protocol == ChatProtocol::OpenaiResponses
        && (event == "response.failed" || (generic_frame && declared == Some("response.failed")));
    let body_error = generic_frame
        && (declared == Some("error") || data.get("error").is_some_and(|value| !value.is_null()));
    if !named_error && !response_failed && !body_error {
        return None;
    }
    let Some(_) = data.as_object() else {
        return Some(malformed());
    };
    if has_output(data) {
        return Some(malformed());
    }
    let error: &Map<String, Value> = if response_failed {
        if declared.is_some_and(|value| value != "response.failed") {
            return Some(malformed());
        }
        let Some(response) = data.get("response").filter(|value| value.is_object()) else {
            return Some(malformed());
        };
        if response
            .get("status")
            .is_some_and(|value| value.as_str() != Some("failed"))
            || has_output(response)
        {
            return Some(malformed());
        }
        let Some(error) = response.get("error").and_then(Value::as_object) else {
            return Some(malformed());
        };
        error
    } else if let Some(error) = data.get("error") {
        let Some(error) = error.as_object() else {
            return Some(malformed());
        };
        error
    } else {
        // Responses error events may carry code/message at the frame root.
        data.as_object().expect("object checked above")
    };
    if has_output_object(error) {
        return Some(malformed());
    }
    for field in ["code", "type", "status"] {
        if error.get(field).is_some_and(|value| {
            !value.is_null() && !value.is_string() && !(field == "code" && value.is_number())
        }) {
            return Some(malformed());
        }
    }
    if protocol != ChatProtocol::Bedrock
        && (protocol != ChatProtocol::Gemini || error.get("code").is_some_and(Value::is_number))
        && let Some(business) = classify_gateway_business_error(error)
    {
        return Some(gateway_error(business));
    }
    if let Some(feature) = classify_unsupported_technical_capability(error).map(|capability| {
        match capability {
            ProviderTechnicalCapability::FunctionCalling => ChatModelFeature::ToolCalls,
            ProviderTechnicalCapability::Reasoning => ChatModelFeature::Reasoning,
            ProviderTechnicalCapability::Streaming => ChatModelFeature::Streaming,
        }
    }) {
        let mut failure = ChatModelError::new(
            ChatModelErrorCode::UnsupportedFeature,
            "provider does not support the requested chat feature",
            ChatRetryDirective::Failover,
        );
        failure.unsupported_feature = Some(feature);
        let mut failure = diagnose(protocol, failure, error);
        failure.diagnostic.as_mut().expect("diagnosed").reason = ModelFailureReason::UnsupportedFeature;
        return Some(failure);
    }
    // Specific code wins over a generic type (e.g. context_length_exceeded
    // plus invalid_request_error). Do not interpret natural-language messages
    // or parse retry delays from diagnostics that may contain private data.
    for field in ["code", "type", "status"] {
        if let Some((code, retry, message)) =
            error.get(field).and_then(Value::as_str).and_then(classify)
        {
            return Some(diagnose(protocol, ChatModelError::new(code, message, retry), error));
        }
    }
    // Keep the old explicitly named generic error behavior; newly recognized
    // failed-response/JSON envelopes do not gain automatic retry by default.
    Some(diagnose(protocol, ChatModelError::new(
        ChatModelErrorCode::ProviderUnavailable,
        "provider returned an unclassified stream error",
        if named_error {
            ChatRetryDirective::Failover
        } else {
            ChatRetryDirective::Never
        },
    ), error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_reason_is_specific_without_changing_broker_retry_semantics() {
        for (machine, kind, reason, code, retry) in [
            ("invalid_api_key", "invalid_request_error", ModelFailureReason::InvalidKey,
                ChatModelErrorCode::AuthenticationFailed, ChatRetryDirective::Never),
            ("insufficient_quota", "rate_limit_error", ModelFailureReason::InsufficientQuota,
                ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::Never),
            ("model_not_found", "invalid_request_error", ModelFailureReason::ModelNotFound,
                ChatModelErrorCode::InvalidRequest, ChatRetryDirective::Never),
            ("content_policy_violation", "invalid_request_error", ModelFailureReason::ContentPolicy,
                ChatModelErrorCode::UnsupportedFeature, ChatRetryDirective::Never),
            ("context_length_exceeded", "invalid_request_error", ModelFailureReason::PromptTooLong,
                ChatModelErrorCode::PromptTooLong, ChatRetryDirective::Never),
            ("overloaded_error", "server_error", ModelFailureReason::ProviderOverloaded,
                ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::RetrySameRoute),
            ("server_error", "overloaded_error", ModelFailureReason::UpstreamServerError,
                ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::RetrySameRoute),
        ] {
            let error = decode(ChatProtocol::OpenaiChat, "error", &serde_json::json!({
                "error": {"code": machine, "type": kind, "message": "private native diagnostic"}
            })).unwrap();
            assert_eq!(error.code, code, "{machine}");
            assert_eq!(error.retry, retry, "{machine}");
            assert_eq!(error.diagnostic.as_ref().unwrap().reason, reason, "{machine}");
            assert!(!serde_json::to_string(&error).unwrap().contains("private native"));
        }
        let permission = decode(ChatProtocol::Anthropic, "error", &serde_json::json!({
            "error": {"type": "permission_error", "message": "invalid_api_key"}
        })).unwrap();
        assert_eq!(permission.code, ChatModelErrorCode::AuthenticationFailed);
        assert_eq!(permission.diagnostic.unwrap().reason, ModelFailureReason::PermissionDenied);
    }

    fn google_error_info(machine: &str, domain: &str) -> Value {
        serde_json::json!({"@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "domain": domain, "reason": machine,
            "metadata": {"service": "generativelanguage.googleapis.com", "secret": "private-api-key"},
            "message": "private-api-key"})
    }

    #[test]
    fn google_error_info_refines_generic_status_without_changing_coarse_error() {
        for domain in ["googleapis.com", "generativelanguage.googleapis.com"] {
            for (machine, expected) in [("API_KEY_INVALID", ModelFailureReason::InvalidKey),
                ("API_KEY_EXPIRED", ModelFailureReason::ExpiredKey),
                ("ACCESS_TOKEN_TYPE_UNSUPPORTED", ModelFailureReason::AuthSchemeMismatch)] {
                for (status, code, coarse) in [("INVALID_ARGUMENT", 400, ChatModelErrorCode::InvalidRequest),
                    ("UNAUTHENTICATED", 401, ChatModelErrorCode::ProviderUnavailable),
                    ("PERMISSION_DENIED", 403, ChatModelErrorCode::ProviderUnavailable)] {
                    let data = serde_json::json!({"error": {"code": code, "status": status,
                        "message": "private-api-key", "details": [google_error_info(machine, domain)]}});
                    let error = decode(ChatProtocol::Gemini, "json", &data).unwrap();
                    assert_eq!(error.code, coarse, "{machine} {status}");
                    assert_eq!(error.retry, ChatRetryDirective::Never);
                    assert_eq!(error.unsupported_feature, None);
                    assert_eq!(error.diagnostic.as_ref().unwrap().reason, expected, "{machine} {status}");
                    let json = serde_json::to_string(&error).unwrap();
                    assert!(!json.contains("private-api-key"));
                    assert!(!json.contains("metadata"));
                }
            }
        }
    }

    #[test]
    fn google_error_info_requires_unambiguous_native_identity_and_status() {
        let trusted = google_error_info("API_KEY_INVALID", "googleapis.com");
        let baseline = serde_json::json!({"error": {"code": 400, "status": "INVALID_ARGUMENT", "details": [trusted]}});
        let mut cases = Vec::new();
        for (field, value) in [("domain", serde_json::json!("untrusted.example")),
            ("@type", serde_json::json!("type.googleapis.com/google.rpc.Other")),
            ("reason", serde_json::json!("opaque-reason")),
            ("reason", serde_json::json!({"code": "API_KEY_INVALID"}))] {
            let mut data = baseline.clone();
            data["error"]["details"][0][field] = value;
            cases.push(data);
        }
        for details in [serde_json::json!([trusted, google_error_info("API_KEY_EXPIRED", "googleapis.com")]),
            serde_json::json!([trusted, google_error_info("API_KEY_INVALID", "googleapis.com")]),
            serde_json::json!([trusted, null]),
            serde_json::json!({"reason": "API_KEY_INVALID"})] {
            let mut data = baseline.clone();
            data["error"]["details"] = details;
            cases.push(data);
        }
        let mut prose = baseline.clone();
        prose["error"]["details"] = serde_json::json!([{"@type":"type.googleapis.com/google.rpc.Other",
            "metadata":{"nested":trusted}, "message":"API_KEY_INVALID"}]);
        cases.push(prose);
        for data in cases {
            let error = decode(ChatProtocol::Gemini, "json", &data).unwrap();
            assert_eq!(error.code, ChatModelErrorCode::InvalidRequest);
            assert_eq!(error.retry, ChatRetryDirective::Never);
            assert_eq!(error.diagnostic.unwrap().reason, ModelFailureReason::InvalidRequest, "{data}");
        }
        let foreign_protocol = decode(ChatProtocol::OpenaiChat, "json", &baseline).unwrap();
        assert_eq!(foreign_protocol.diagnostic.unwrap().reason, ModelFailureReason::InvalidRequest);
        let mut contradiction = baseline.clone();
        contradiction["error"]["code"] = serde_json::json!(401);
        let error = decode(ChatProtocol::Gemini, "json", &contradiction).unwrap();
        assert_eq!(error.diagnostic.unwrap().reason, ModelFailureReason::InvalidRequest);
        let mut specific = baseline.clone();
        specific["error"]["type"] = serde_json::json!("model_access_denied");
        let error = decode(ChatProtocol::Gemini, "json", &specific).unwrap();
        assert_eq!(error.diagnostic.unwrap().reason, ModelFailureReason::ModelPermissionDenied);
        let mut malformed = baseline;
        malformed["error"]["content"] = serde_json::json!(["undispatched output"]);
        let error = decode(ChatProtocol::Gemini, "json", &malformed).unwrap();
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
        assert_eq!(error.diagnostic.unwrap().reason, ModelFailureReason::InvalidResponse);
    }

    #[test]
    fn body_metadata_without_redaction_context_keeps_only_known_identifiers() {
        let error = decode(ChatProtocol::OpenaiChat, "error", &serde_json::json!({
            "error": {"code": "private-api-key", "type": "unknown-secret-type",
                "request_id": "private-api-key", "message": "insufficient_quota subscription_expired",
                "param": "private-api-key",
                "detail": {"code": "invalid_api_key"}}
        })).unwrap();
        let diagnostic = error.diagnostic.unwrap();
        assert_eq!(diagnostic.reason, ModelFailureReason::ProviderUnavailable);
        assert_eq!(diagnostic.provider_code, None);
        assert_eq!(diagnostic.provider_type, None);
        assert_eq!(diagnostic.provider_param, None);
        assert_eq!(diagnostic.request_id, None);
        let json = serde_json::to_string(&diagnostic).unwrap();
        assert!(!json.contains("private-api-key"));
        assert!(!json.contains("unknown-secret-type"));
    }

    #[test]
    fn redacted_http_context_survives_native_specificity_and_malformed_input() {
        let mut context = ModelFailureDiagnostic::new(ModelFailureReason::PermissionDenied);
        context.http_status = Some(403);
        context.provider_code = Some("safe-opaque-code".to_owned());
        context.provider_type = Some("safe-opaque-type".to_owned());
        context.provider_param = Some("/messages/0/content".to_owned());
        context.provider_id = Some("actual-provider".to_owned());
        context.model_name = Some("actual-attempt-model".to_owned());
        context.endpoint = Some("https://api.example.test/v1/messages".to_owned());
        context.request_id = Some("req_safe".to_owned());
        context.retry_after_ms = Some(1000);
        context.protocol = Some("anthropic.messages".to_owned());
        context.auth_scheme = Some("header_key".to_owned());
        context.content_type = Some("application/json".to_owned());
        let data = serde_json::json!({"error": {"code": "model_access_denied",
            "type": "permission_error", "param": "private-api-key",
            "request_id": "private-api-key", "message": "private-api-key"}});
        let error = decode_with_context(ChatProtocol::Anthropic, "error", &data, Some(&context)).unwrap();
        let mut expected = context.clone();
        expected.reason = ModelFailureReason::ModelPermissionDenied;
        assert_eq!(error.diagnostic, Some(expected));
        assert!(serde_json::to_string(&error).unwrap().contains("\"providerParam\":\"/messages/0/content\""));
        assert!(!serde_json::to_string(&error).unwrap().contains("private-api-key"));

        let unknown = decode_with_context(ChatProtocol::Anthropic, "error",
            &serde_json::json!({"error": {"code": "opaque-code"}}), Some(&context)).unwrap();
        assert_eq!(unknown.diagnostic, Some(context.clone()));
        let malformed = decode_with_context(ChatProtocol::Anthropic, "error",
            &serde_json::json!({"error": {"code": "subscription_expired"}, "content": ["output"]}),
            Some(&context)).unwrap();
        assert_eq!(malformed.code, ChatModelErrorCode::ProtocolViolation);
        assert_eq!(malformed.retry, ChatRetryDirective::Never);
        let diagnostic = malformed.diagnostic.unwrap();
        assert_eq!(diagnostic.reason, ModelFailureReason::InvalidResponse);
        assert_eq!(diagnostic.request_id, context.request_id);
    }

    #[test]
    fn technical_downgrade_uses_codes_and_exact_parameters_not_message_text() {
        let error = decode(
            ChatProtocol::OpenaiChat,
            "error",
            &serde_json::json!({
                "error": {
                    "code": "unsupported_parameter",
                    "type": "invalid_request_error",
                    "param": "tools",
                    "message": "diagnostic prose is not inspected"
                }
            }),
        )
        .unwrap();
        assert_eq!(error.code, ChatModelErrorCode::UnsupportedFeature);
        assert_eq!(error.unsupported_feature, Some(ChatModelFeature::ToolCalls));

        let generic = decode(
            ChatProtocol::OpenaiChat,
            "error",
            &serde_json::json!({
                "error": {
                    "code": "invalid_request_error",
                    "param": "tools",
                    "message": "tools are unsupported"
                }
            }),
        )
        .unwrap();
        assert_eq!(generic.code, ChatModelErrorCode::InvalidRequest);
        assert_eq!(generic.unsupported_feature, None);
    }

    #[test]
    fn transient_and_account_errors_never_become_capability_evidence() {
        for code in [
            "rate_limit_error",
            "authentication_error",
            "server_error",
            "insufficient_quota",
        ] {
            let error = decode(
                ChatProtocol::OpenaiChat,
                "error",
                &serde_json::json!({"error": {"code": code, "param": "tools"}}),
            )
            .unwrap();
            assert_eq!(error.unsupported_feature, None, "code {code}");
        }
    }

    #[test]
    fn gateway_business_errors_keep_safe_actions_and_never_downgrade_features() {
        for (business, expected_code, expected_retry) in [
            (GatewayBusinessError::InsufficientBalance, ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::Never),
            (GatewayBusinessError::SubscriptionExpired, ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::Never),
            (GatewayBusinessError::ModelNotInPlan, ChatModelErrorCode::ProviderUnavailable, ChatRetryDirective::Never),
            (GatewayBusinessError::KeyExpired, ChatModelErrorCode::AuthenticationFailed, ChatRetryDirective::Never),
            (GatewayBusinessError::RateLimited, ChatModelErrorCode::RateLimited, ChatRetryDirective::RetrySameRoute),
        ] {
            for protocol in [ChatProtocol::OpenaiChat, ChatProtocol::OpenaiResponses, ChatProtocol::Anthropic] {
                let data = serde_json::json!({"error": {"code": business.code(),
                    "type": "unsupported_parameter", "param": "tools",
                    "message": "private https://upstream.invalid/?key=secret",
                    "purchase_url": "https://untrusted.invalid/buy"}});
                let error = decode(protocol, "error", &data).unwrap();
                assert_eq!(error.code, expected_code);
                assert_eq!(error.retry, expected_retry);
                assert_eq!(error.message, business.action_message());
                assert_eq!(error.unsupported_feature, None);
                assert!(!error.message.contains("secret"));
                assert!(!error.message.contains("untrusted"));
            }
            let data = serde_json::json!({"error": {"code": business.http_status(), "details": [{
                "@type": "type.googleapis.com/google.rpc.ErrorInfo", "domain": "nomifun-model-gateway",
                "reason": business.code().to_ascii_uppercase(), "metadata": {"nomifun_code": business.code()}
            }]}});
            let error = decode(ChatProtocol::Gemini, "json", &data).unwrap();
            assert_eq!(error.code, expected_code);
            assert_eq!(error.retry, expected_retry);
            assert_eq!(error.message, business.action_message());
        }
    }

    #[test]
    fn gateway_error_diagnostics_and_foreign_errorinfo_are_not_billing_evidence() {
        for data in [
            serde_json::json!({"error":{"type":"permission_error", "message":"subscription_expired insufficient_balance"}}),
            serde_json::json!({"error":{"code":402, "message":"insufficient_balance", "details":[{
                "@type":"type.googleapis.com/google.rpc.ErrorInfo", "domain":"googleapis.com",
                "reason":"INSUFFICIENT_BALANCE", "metadata":{"nomifun_code":"insufficient_balance"}
            }]}}),
        ] {
            let error = decode(ChatProtocol::Gemini, "json", &data).unwrap();
            assert!(!error.message.contains("model gateway"));
        }
        assert!(decode(ChatProtocol::OpenaiChat, "message", &serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{"function": {"arguments": "{\"error\":{\"code\":\"insufficient_balance\"}}"}}]}}]
        })).is_none());
        let error = decode(ChatProtocol::OpenaiChat, "json", &serde_json::json!({
            "error":{"code":"insufficient_balance"}, "choices":[{"delta":{"content":"output"}}]
        })).unwrap();
        assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
        for field in ["content", "tool_calls", "usage"] {
            let mut data = serde_json::json!({"error":{"code":"insufficient_balance"}});
            data["error"][field] = serde_json::json!({"undispatched":"output"});
            let error = decode(ChatProtocol::OpenaiChat, "error", &data).unwrap();
            assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation, "inner output {field}");
        }
    }
}
