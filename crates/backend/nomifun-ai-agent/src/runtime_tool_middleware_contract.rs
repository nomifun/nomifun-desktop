//! Host-owned gates for one outer tool invocation. Middleware only returns a decision.
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_INPUT_BYTES: usize = 256 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_REASON_BYTES: usize = 2048;

#[derive(Clone, Debug, Serialize)]
pub struct BeforeToolInput {
    pub phase: &'static str,
    pub invocation_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub redacted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum BeforeToolDecision {
    Allow {},
    Deny { reason: String },
}

impl BeforeToolDecision {
    pub fn validate(&self) -> Result<(), String> {
        if serde_json::to_vec(self)
            .map_err(|_| "before_tool output encoding failed")?
            .len()
            > MAX_OUTPUT_BYTES
        {
            return Err("before_tool output exceeds 64 KiB".into());
        }
        if let Self::Deny { reason } = self {
            if reason.trim().is_empty() || reason.len() > MAX_REASON_BYTES {
                return Err("before_tool deny reason must contain 1..2048 UTF-8 bytes".into());
            }
        }
        Ok(())
    }
}

/// Validate the wire size before decoding so ignored whitespace cannot bypass it.
pub fn decode_decision(bytes: &[u8]) -> Result<BeforeToolDecision, String> {
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err("before_tool output exceeds 64 KiB".into());
    }
    let decision: BeforeToolDecision = serde_json::from_slice(bytes)
        .map_err(|_| "before_tool returned an invalid decision".to_owned())?;
    decision.validate()?;
    Ok(decision)
}

#[async_trait]
pub trait ToolCallMiddleware: Send + Sync {
    async fn before_tool(&self, input: BeforeToolInput) -> Result<BeforeToolDecision, String>;
    fn label(&self) -> &str;
}

/// Check the complete input before redaction as well; masking must not turn an
/// oversized argument into a silently accepted gate input.
pub(crate) fn input(
    invocation_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> Result<BeforeToolInput, String> {
    let mut input = BeforeToolInput {
        phase: "before_tool",
        invocation_id: invocation_id.to_owned(),
        tool_call_id: tool_call_id.to_owned(),
        tool_name: tool_name.to_owned(),
        arguments: arguments.clone(),
        redacted: false,
    };
    check_input_size(&input)?;
    redact_arguments(&mut input.arguments, &mut input.redacted);
    check_input_size(&input)?;
    Ok(input)
}

fn check_input_size(input: &BeforeToolInput) -> Result<(), String> {
    if serde_json::to_vec(input)
        .map_err(|_| "before_tool input encoding failed")?
        .len()
        > MAX_INPUT_BYTES
    {
        return Err("before_tool input exceeds 256 KiB; target tool was not executed".into());
    }
    Ok(())
}

fn sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "password"
            | "passwd"
            | "secret"
            | "clientsecret"
            | "apikey"
            | "accesskey"
            | "accesskeyid"
            | "secretaccesskey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "privatekey"
            | "credentials"
    ) || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("token")
        || normalized.ends_with("apikey")
}

fn redact_arguments(value: &mut Value, redacted: &mut bool) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive_key(key) {
                    *value = Value::String("[REDACTED]".into());
                    *redacted = true;
                } else {
                    redact_arguments(value, redacted);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_arguments(value, redacted);
            }
        }
        Value::String(text) => {
            let safe = nomi_redact::redact_secrets_owned(text.clone());
            *redacted |= safe != *text;
            *text = safe;
        }
        _ => {}
    }
}

pub(crate) async fn apply(
    middleware: &[Arc<dyn ToolCallMiddleware>],
    input: BeforeToolInput,
    stopped: impl Fn() -> bool,
) -> Result<BeforeToolDecision, String> {
    for entry in middleware {
        if stopped() {
            return Err("before_tool chain stopped after an earlier gate failure".into());
        }
        let decision = entry
            .before_tool(input.clone())
            .await
            .map_err(|_| "before_tool service failed; target tool was not executed".to_owned())?;
        decision.validate()?;
        if matches!(decision, BeforeToolDecision::Deny { .. }) {
            return Ok(decision);
        }
    }
    Ok(BeforeToolDecision::Allow {})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_decisions_and_bounded_reasons() {
        assert_eq!(
            decode_decision(br#"{"decision":"allow"}"#).unwrap(),
            BeforeToolDecision::Allow {}
        );
        for bytes in [
            br#"{"decision":"allow","reason":"ignored"}"#.as_slice(),
            br#"{"decision":"deny","reason":""}"#,
            br#"{"decision":"deny","reason":"no","arguments":{}}"#,
        ] {
            assert!(decode_decision(bytes).is_err());
        }
        assert!(
            BeforeToolDecision::Deny {
                reason: "界".repeat(683)
            }
            .validate()
            .is_err()
        );
        assert!(decode_decision(&vec![b' '; MAX_OUTPUT_BYTES + 1]).is_err());
    }
    #[test]
    fn recursive_redaction_preserves_business_arguments_without_truncation() {
        let raw = serde_json::json!({"nested": [{"api_key":"sensitive", "quantity":7}], "password":"hidden", "destination":"office"});
        let safe = input("invocation", "call", "business", &raw).unwrap();
        assert!(safe.redacted);
        assert_eq!(safe.arguments["nested"][0]["api_key"], "[REDACTED]");
        assert_eq!(safe.arguments["nested"][0]["quantity"], 7);
        assert_eq!(safe.arguments["destination"], "office");
        assert_eq!(raw["password"], "hidden");
        assert!(
            input(
                "i",
                "c",
                "t",
                &serde_json::json!({"password":"x".repeat(MAX_INPUT_BYTES)})
            )
            .is_err()
        );
    }
}
