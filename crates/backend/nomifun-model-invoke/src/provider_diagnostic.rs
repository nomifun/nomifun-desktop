//! Bounded native evidence, not provider prose. These refinements are display
//! data and never choose key rotation, retries or canonical settlement.
use crate::error::{InvokeErrorKind, ModelFailureDiagnostic, ModelFailureReason};
use nomifun_net::secret_redaction::{SecretRedactor, sanitized_endpoint};
use serde_json::{Map, Value};

pub(crate) fn kind_reason(kind: InvokeErrorKind) -> ModelFailureReason {
    use InvokeErrorKind as Kind;
    use ModelFailureReason as Reason;
    match kind {
        Kind::Auth => Reason::AuthFailed,
        Kind::RateLimited => Reason::RateLimited,
        Kind::QuotaExhausted => Reason::InsufficientQuota,
        Kind::Network => Reason::NetworkFailure,
        Kind::Timeout => Reason::RequestTimeout,
        Kind::InvalidParams => Reason::InvalidRequest,
        Kind::ContentPolicy => Reason::ContentPolicy,
        Kind::NonApiResponse => Reason::NonApiResponse,
        Kind::ParseError => Reason::InvalidResponse,
        Kind::Config | Kind::NoAdapter | Kind::MissingConnection => Reason::ConfigurationError,
        Kind::UnsupportedTask | Kind::NotPollable => Reason::UnsupportedFeature,
        Kind::ProviderError | Kind::JobFailed => Reason::ProviderUnavailable,
    }
}

pub(crate) fn http_reason(status: u16) -> ModelFailureReason {
    use ModelFailureReason as Reason;
    match status {
        401 => Reason::AuthFailed,
        402 => Reason::BillingRequired,
        403 => Reason::PermissionDenied,
        404 => Reason::EndpointMissing,
        407 => Reason::ProxyFailure,
        408 | 504 => Reason::RequestTimeout,
        429 => Reason::RateLimited,
        400 | 413 | 422 => Reason::InvalidRequest,
        500..=599 => Reason::UpstreamServerError,
        _ => Reason::ProviderUnavailable,
    }
}

pub(crate) fn identifier(value: &str, redactor: &SecretRedactor) -> Option<String> {
    let safe = redactor.redact(value);
    (!value.is_empty() && safe == value && value.len() <= 200 && value.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-#".contains(&byte))).then_some(safe)
}

fn parameter(value: &str, redactor: &SecretRedactor) -> Option<String> {
    let safe = redactor.redact(value);
    (!value.is_empty() && safe == value && value.len() <= 200 && value.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_.-/~[]".contains(&byte))).then_some(safe)
}

pub(crate) fn response_context(
    response: &reqwest::Response,
    protocol: Option<&str>,
    auth_scheme: Option<&str>,
    redactor: &SecretRedactor,
) -> ModelFailureDiagnostic {
    let mut diagnostic = ModelFailureDiagnostic::new(http_reason(response.status().as_u16()));
    if nomifun_net::api_response::is_non_api_content_type(response.headers()).is_some() {
        diagnostic.reason = ModelFailureReason::NonApiResponse;
    }
    diagnostic.http_status = Some(response.status().as_u16());
    diagnostic.endpoint = sanitized_endpoint(response.url().as_str())
        .map(|value| redactor.redact(&value)).and_then(|value| sanitized_endpoint(&value));
    diagnostic.protocol = protocol.and_then(|value| identifier(value, redactor));
    diagnostic.auth_scheme = auth_scheme.and_then(|value| identifier(value, redactor));
    diagnostic.request_id = ["x-request-id", "request-id", "x-amzn-requestid", "x-amz-request-id", "x-goog-request-id"]
        .into_iter().find_map(|name| response.headers().get(name).and_then(|value| value.to_str().ok())
            .and_then(|value| identifier(value, redactor)));
    diagnostic.content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok()).and_then(|value| {
            let mime = value.split(';').next()?.trim().to_ascii_lowercase();
            (!mime.is_empty() && mime.len() <= 100 && mime.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/-+._".contains(&byte)))
                .then_some(mime).filter(|value| redactor.redact(value) == *value)
        });
    diagnostic
}

fn has_output(value: &Map<String, Value>) -> bool {
    ["output", "choices", "candidates", "delta", "text", "output_text", "content", "tool_calls", "usage", "usageMetadata"]
        .iter().any(|key| value.get(*key).is_some_and(|part| match part {
            Value::Null => false,
            Value::String(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            _ => true,
        }))
}

pub(crate) fn error_object<'a>(value: &'a Value, protocol: Option<&str>) -> Option<&'a Map<String, Value>> {
    let root = value.as_object()?;
    if has_output(root) { return None; }
    let object = if root.get("type").and_then(Value::as_str) == Some("response.failed") {
        let response = root.get("response")?.as_object()?;
        if response.get("status").and_then(Value::as_str) != Some("failed") || has_output(response) { return None; }
        response.get("error")?.as_object()?
    } else if let Some(error) = root.get("error") {
        error.as_object()?
    } else if matches!(protocol, Some("openai.responses" | "openai.chat_text"))
        && root.get("type").and_then(Value::as_str) == Some("error") {
        root
    } else if matches!(protocol, Some("bedrock.anthropic_messages")) && root.contains_key("__type") {
        root
    } else { return None; };
    (!has_output(object)).then_some(object)
}

pub(crate) fn refine_from_body(
    mut diagnostic: ModelFailureDiagnostic,
    value: &Value,
    protocol: Option<&str>,
    redactor: &SecretRedactor,
) -> ModelFailureDiagnostic {
    let Some(object) = error_object(value, protocol) else { return diagnostic; };
    let machine = ["code", "type", "status", "__type"].into_iter()
        .filter_map(|field| object.get(field).and_then(Value::as_str))
        .find_map(ModelFailureReason::from_machine_code);
    if let Some(reason) = machine { diagnostic.reason = reason; }
    if protocol == Some("gemini.generate_text") {
        diagnostic.reason = ModelFailureReason::refine_google_error_info(object, diagnostic.reason)
            .unwrap_or(diagnostic.reason);
    }
    diagnostic.provider_code = object.get("code").or_else(|| object.get("__type"))
        .and_then(|value| match value {
            Value::String(value) => identifier(value, redactor),
            Value::Number(value) if value.is_u64() => Some(value.to_string()),
            _ => None,
        });
    diagnostic.provider_type = object.get("type").or_else(|| object.get("status"))
        .and_then(Value::as_str).and_then(|value| identifier(value, redactor));
    diagnostic.provider_param = object.get("param").and_then(Value::as_str)
        .and_then(|value| parameter(value, redactor));
    diagnostic
}

pub(crate) fn frame_is_error(event: &str, value: &Value) -> bool {
    matches!(event, "error" | "response.failed" | "bedrock.exception")
        || matches!(value.get("type").and_then(Value::as_str), Some("error" | "response.failed"))
        || value.get("error").is_some_and(|error| !error.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn diagnose(status: u16, body: Value) -> ModelFailureDiagnostic {
        refine_from_body(ModelFailureDiagnostic::new(http_reason(status)), &body,
            Some("openai.chat_text"), &SecretRedactor::new(["fixture-private-key"]))
    }

    #[test]
    fn specific_machine_code_wins_over_status_and_generic_type_without_prose_guesses() {
        for (status, code, reason) in [
            (401, "invalid_api_key", ModelFailureReason::InvalidKey),
            (403, "key_expired", ModelFailureReason::ExpiredKey),
            (429, "credit_balance_exhausted", ModelFailureReason::InsufficientBalance),
            (429, "project_spend_limit_exceeded", ModelFailureReason::SpendLimitReached),
            (404, "model_not_found", ModelFailureReason::ModelNotFound),
            (403, "model_permission_denied", ModelFailureReason::ModelPermissionDenied),
            (503, "overloaded_error", ModelFailureReason::ProviderOverloaded),
        ] {
            let result = diagnose(status, json!({"error":{"code":code,"type":"insufficient_quota",
                "message":"API key expired, balance exhausted, DNS error"}}));
            assert_eq!(result.reason, reason, "{code}");
            assert_eq!(result.provider_code.as_deref(), Some(code));
        }
        assert_eq!(diagnose(403, json!({"error":{"message":"API key expired"}})).reason, ModelFailureReason::PermissionDenied);
        assert_eq!(diagnose(404, json!({"error":{"code":"ResourceNotFoundException"}})).reason, ModelFailureReason::EndpointMissing);
        assert_eq!(kind_reason(InvokeErrorKind::QuotaExhausted), ModelFailureReason::InsufficientQuota);
    }

    #[test]
    fn responses_root_error_retains_only_redacted_native_identifiers_and_parameter_paths() {
        let result = refine_from_body(ModelFailureDiagnostic::new(ModelFailureReason::ProviderUnavailable),
            &json!({"type":"error","code":"invalid_api_key","param":"messages[1].content",
                "message":"fixture-private-key", "request_id":"fixture-private-key"}),
            Some("openai.responses"), &SecretRedactor::new(["fixture-private-key"]));
        assert_eq!(result.reason, ModelFailureReason::InvalidKey);
        assert_eq!(result.provider_code.as_deref(), Some("invalid_api_key"));
        assert_eq!(result.provider_param.as_deref(), Some("messages[1].content"));
        assert!(!serde_json::to_string(&result).unwrap().contains("fixture-private-key"));
        let result = refine_from_body(ModelFailureDiagnostic::new(ModelFailureReason::ProviderUnavailable),
            &json!({"type":"error","code":"fixture-private-key","param":"fixture-private-key"}),
            Some("openai.responses"), &SecretRedactor::new(["fixture-private-key"]));
        assert!(result.provider_code.is_none());
        assert!(result.provider_param.is_none());
    }

    #[test]
    fn quoted_tool_errors_malformed_bodies_and_parameter_values_are_not_diagnostics() {
        for body in [
            json!({"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"invalid_api_key"}}]}}]}),
            json!({"error":{"code":"key_expired"},"output":[{"text":"actual output"}]}),
            json!({"error":"invalid_api_key"}),
        ] {
            assert_eq!(diagnose(403, body).reason, ModelFailureReason::PermissionDenied);
        }
        for value in ["model=private value", "https://host/path?key=value", "/messages/0/\"private-value\""] {
            assert!(diagnose(400, json!({"error":{"param":value}})).provider_param.is_none());
        }
    }

    #[test]
    fn native_google_errorinfo_and_its_status_are_required_for_auth_refinement() {
        let value = json!({"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{
            "@type":"type.googleapis.com/google.rpc.ErrorInfo","domain":"googleapis.com",
            "reason":"API_KEY_INVALID","metadata":{"secret":"not copied"}}
        ]}});
        let result = refine_from_body(ModelFailureDiagnostic::new(http_reason(400)), &value,
            Some("gemini.generate_text"), &SecretRedactor::default());
        assert_eq!(result.reason, ModelFailureReason::InvalidKey);
        assert!(!serde_json::to_string(&result).unwrap().contains("not copied"));
    }

    #[tokio::test]
    async fn html_auth_responses_are_non_api_diagnostics_without_changing_http_failure_kind() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        use wiremock::matchers::{method, path};
        let server = MockServer::start().await;
        for status in [401, 403] {
            let route = format!("/html-{status}");
            Mock::given(method("GET")).and(path(route.clone()))
                .respond_with(ResponseTemplate::new(status).set_body_raw("<html>sign in</html>", "text/html")
                    .insert_header("x-request-id", "trace-public"))
                .mount(&server).await;
            let response = reqwest::Client::builder().no_proxy().build().unwrap()
                .get(format!("{}{}?key=private#fragment", server.uri(), route)).send().await.unwrap();
            let error = crate::transport::error_from_response(response).await;
            assert_eq!(error.kind, InvokeErrorKind::Auth);
            let diagnostic = error.diagnostic.unwrap();
            assert_eq!(diagnostic.reason, ModelFailureReason::NonApiResponse);
            assert_eq!(diagnostic.http_status, Some(status));
            assert_eq!(diagnostic.endpoint, Some(format!("{}{}", server.uri(), route)));
            assert_eq!(diagnostic.request_id.as_deref(), Some("trace-public"));
        }
    }

    #[tokio::test]
    async fn media_non_api_error_retains_the_final_redirected_endpoint_and_request_id() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        use wiremock::matchers::{method, path};
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/entry"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/login?key=fixture-private-key"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/login"))
            .respond_with(ResponseTemplate::new(401).set_body_raw("<html>sign in</html>", "text/html")
                .insert_header("x-request-id", "trace-final"))
            .mount(&server).await;
        let auth = crate::AuthMaterial { scheme: crate::AuthScheme::Bearer,
            credentials: json!({"api_keys":["fixture-private-key"]}) };
        let error = crate::transport::get_request(&reqwest::Client::builder().no_proxy().build().unwrap(),
            &format!("{}/entry", server.uri()), std::time::Duration::from_secs(2), &auth).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::NonApiResponse);
        let diagnostic = error.diagnostic.unwrap();
        assert_eq!(diagnostic.reason, ModelFailureReason::NonApiResponse);
        assert_eq!(diagnostic.endpoint, Some(format!("{}/login", server.uri())));
        assert_eq!(diagnostic.request_id.as_deref(), Some("trace-final"));
        assert!(!serde_json::to_string(&diagnostic).unwrap().contains("fixture-private-key"));
    }
}
