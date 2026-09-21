//! Protocol error envelopes, not model/tool text. Keep provider diagnostics
//! out of canonical errors and classify only explicit machine-readable codes.
use serde_json::{Map, Value};
use nomifun_net::provider_capability::{
    ProviderTechnicalCapability, classify_unsupported_technical_capability,
};

use crate::{ChatModelError, ChatModelErrorCode, ChatProtocol, ChatRetryDirective};
use crate::ChatModelFeature;

fn malformed() -> ChatModelError {
    ChatModelError::protocol_violation(
        "provider error envelope is malformed or contains undispatched output; automatic replay is not allowed",
    )
}

fn has_output(value: &Value) -> bool {
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
            Retry::Failover,
            "provider rate limit rejected the attempt",
        ),
        "overloaded_error" | "server_error" | "internal_server_error" => (
            Code::ProviderUnavailable,
            Retry::Failover,
            "provider is temporarily unavailable",
        ),
        "invalid_api_key" | "authentication_error" | "permission_error" => (
            Code::AuthenticationFailed,
            Retry::Never,
            "provider rejected request authentication or permission",
        ),
        "invalid_request_error" | "invalid_argument" | "INVALID_ARGUMENT" | "invalid_prompt" => (
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
                Retry::Failover,
                "Bedrock rate limit rejected the attempt",
            ),
            "internalServerException"
            | "InternalServerException"
            | "serviceUnavailableException"
            | "ServiceUnavailableException" => (
                Code::ProviderUnavailable,
                Retry::Failover,
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
        return Some(ChatModelError::new(code, message, retry));
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
    for field in ["code", "type", "status"] {
        if error.get(field).is_some_and(|value| {
            !value.is_null() && !value.is_string() && !(field == "code" && value.is_number())
        }) {
            return Some(malformed());
        }
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
        return Some(failure);
    }
    // Specific code wins over a generic type (e.g. context_length_exceeded
    // plus invalid_request_error). Do not interpret natural-language messages
    // or parse retry delays from diagnostics that may contain private data.
    for field in ["code", "type", "status"] {
        if let Some((code, retry, message)) =
            error.get(field).and_then(Value::as_str).and_then(classify)
        {
            return Some(ChatModelError::new(code, message, retry));
        }
    }
    // Keep the old explicitly named generic error behavior; newly recognized
    // failed-response/JSON envelopes do not gain automatic retry by default.
    Some(ChatModelError::new(
        ChatModelErrorCode::ProviderUnavailable,
        "provider returned an unclassified stream error",
        if named_error {
            ChatRetryDirective::Failover
        } else {
            ChatRetryDirective::Never
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
