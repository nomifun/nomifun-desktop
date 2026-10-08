//! Model gateway business failures carried by native error envelopes.
//! Provider diagnostics and response-supplied URLs are never action authority.

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayBusinessError {
    InsufficientBalance,
    SubscriptionExpired,
    ModelNotInPlan,
    KeyExpired,
    RateLimited,
}

impl GatewayBusinessError {
    pub fn from_code(code: &str) -> Option<Self> {
        Some(match code {
            "insufficient_balance" => Self::InsufficientBalance,
            "subscription_expired" => Self::SubscriptionExpired,
            "model_not_in_plan" => Self::ModelNotInPlan,
            "key_expired" => Self::KeyExpired,
            "rate_limited" => Self::RateLimited,
            _ => return None,
        })
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::InsufficientBalance => "insufficient_balance",
            Self::SubscriptionExpired => "subscription_expired",
            Self::ModelNotInPlan => "model_not_in_plan",
            Self::KeyExpired => "key_expired",
            Self::RateLimited => "rate_limited",
        }
    }

    pub fn http_status(self) -> u16 {
        match self {
            Self::InsufficientBalance => 402,
            Self::SubscriptionExpired | Self::ModelNotInPlan => 403,
            Self::KeyExpired => 401,
            Self::RateLimited => 429,
        }
    }

    /// Only these locally authored messages may reach the current error card.
    /// Purchase and console actions resolve independently from provider meta.
    pub fn action_message(self) -> &'static str {
        match self {
            Self::InsufficientBalance => "The model gateway balance is insufficient. Top up the account to continue.",
            Self::SubscriptionExpired => "The model gateway subscription has expired. Renew the subscription to continue.",
            Self::ModelNotInPlan => "The selected model is not included in your gateway plan. Choose an included model or change the plan.",
            Self::KeyExpired => "The model gateway key has expired. Create a new key and update provider credentials.",
            Self::RateLimited => "The model gateway rate limited the request. Wait and retry the same model.",
        }
    }

    pub fn is_billing(self) -> bool {
        matches!(self, Self::InsufficientBalance | Self::SubscriptionExpired | Self::ModelNotInPlan)
    }
}

fn has_output(value: &Value) -> bool {
    ["output", "choices", "candidates", "delta", "text", "output_text", "content",
        "tool_calls", "usage", "usageMetadata"]
        .iter().any(|field| value.get(*field).is_some_and(|part| match part {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => !items.is_empty(),
            _ => true,
        }))
}

/// Classify only a complete, bounded native HTTP error envelope. No recursive
/// search can promote a tool result, a model answer, or diagnostic prose.
pub fn classify_gateway_business_error_body(body: &[u8]) -> Option<GatewayBusinessError> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    if !value.is_object() || has_output(&value) {
        return None;
    }
    let error = if let Some(response) = value.get("response") {
        if value.get("type").and_then(Value::as_str) != Some("response.failed")
            || response.get("status").and_then(Value::as_str) != Some("failed")
            || has_output(response)
        {
            return None;
        }
        response.get("error")?
    } else {
        value.get("error")?
    };
    if has_output(error) { return None; }
    classify_gateway_business_error(error.as_object()?)
}

/// The caller has already established a native error envelope. OpenAI and
/// Anthropic carry an exact string code. Gemini preserves its native integer
/// code and requires the gateway's exact ErrorInfo identity and reason.
pub fn classify_gateway_business_error(error: &Map<String, Value>) -> Option<GatewayBusinessError> {
    match error.get("code")? {
        Value::String(code) => GatewayBusinessError::from_code(code),
        Value::Number(status) => {
            let status = status.as_u64()?;
            let details = error.get("details")?.as_array()?;
            let mut classified = None;
            for detail in details {
                if detail.get("domain").and_then(Value::as_str) != Some("nomifun-model-gateway") {
                    continue;
                }
                if classified.is_some()
                    || detail.get("@type").and_then(Value::as_str) != Some("type.googleapis.com/google.rpc.ErrorInfo")
                {
                    return None;
                }
                let metadata = detail.get("metadata")?.as_object()?;
                if metadata.values().any(|value| !value.is_string()) { return None; }
                let code = metadata.get("nomifun_code")?.as_str()?;
                let business = GatewayBusinessError::from_code(code)?;
                if detail.get("reason").and_then(Value::as_str) != Some(code.to_ascii_uppercase().as_str())
                    || status != u64::from(business.http_status())
                {
                    return None;
                }
                classified = Some(business);
            }
            classified
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gemini(code: &str) -> Value {
        let business = GatewayBusinessError::from_code(code).unwrap();
        json!({"error": {"code": business.http_status(), "details": [{
            "@type": "type.googleapis.com/google.rpc.ErrorInfo", "domain": "nomifun-model-gateway",
            "reason": code.to_ascii_uppercase(), "metadata": {"nomifun_code": code}
        }]}})
    }

    #[test]
    fn all_five_business_codes_use_exact_native_fields() {
        for code in ["insufficient_balance", "subscription_expired", "model_not_in_plan", "key_expired", "rate_limited"] {
            let expected = GatewayBusinessError::from_code(code);
            for value in [json!({"error": {"code": code, "message": "private URL and key must be ignored"}}),
                json!({"type": "error", "error": {"code": code, "type": "permission_error"}}), gemini(code)] {
                assert_eq!(classify_gateway_business_error_body(&serde_json::to_vec(&value).unwrap()), expected);
            }
        }
    }

    #[test]
    fn foreign_errorinfo_and_diagnostic_impersonation_are_not_gateway_evidence() {
        for value in [
            json!({"error":{"message":"insufficient_balance", "type":"permission_error"}}),
            json!({"error":{"message":"{\"code\":\"insufficient_balance\"}"}}),
            json!({"choices":[{"message":{"content":"insufficient_balance"}}]}),
            json!({"error":{"code":"subscription_expired"},"choices":[{"delta":{"tool_calls":[]}}]}),
            json!({"tool_result":{"error":{"code":"model_not_in_plan"}}}),
            json!({"error":{"code":"INSUFFICIENT_BALANCE"}}),
        ] {
            assert_eq!(classify_gateway_business_error_body(&serde_json::to_vec(&value).unwrap()), None);
        }
        for (field, replacement) in [("domain", json!("googleapis.com")),
            ("@type", json!("type.googleapis.com/google.rpc.Other")), ("reason", json!("PERMISSION_DENIED"))] {
            let mut value = gemini("insufficient_balance");
            value["error"]["details"][0][field] = replacement;
            assert_eq!(classify_gateway_business_error_body(&serde_json::to_vec(&value).unwrap()), None, "field {field}");
        }
        let mut value = gemini("insufficient_balance");
        value["error"]["code"] = json!(403);
        assert_eq!(classify_gateway_business_error_body(&serde_json::to_vec(&value).unwrap()), None);
        let mut value = gemini("insufficient_balance");
        value["error"]["details"][0]["metadata"]["nomifun_code"] = json!("model_not_in_plan");
        assert_eq!(classify_gateway_business_error_body(&serde_json::to_vec(&value).unwrap()), None);
    }
}
