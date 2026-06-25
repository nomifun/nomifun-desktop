use serde::{Deserialize, Serialize};

/// Non-blocking warning emitted when a `PreSendHook` fails to fully
/// transform a prompt. Hook name and message travel as plain strings so
/// the frontend can render a toast without owning the hook enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcpPromptHookWarningPayload {
    /// Stable hook identifier, e.g. "session_new_prelude".
    pub hook: String,
    /// Human-readable failure description.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_snake_case_hook_and_message() {
        let p = AcpPromptHookWarningPayload {
            hook: "session_new_prelude".into(),
            message: "discover_by_names failed".into(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["hook"], "session_new_prelude");
        assert_eq!(v["message"], "discover_by_names failed");
    }

    #[test]
    fn round_trip_preserves_fields() {
        let p = AcpPromptHookWarningPayload {
            hook: "model_identity_reminder".into(),
            message: "".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: AcpPromptHookWarningPayload = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }
}
