//! Host-owned gates for one outer tool invocation. Middleware only returns a decision.
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
}
