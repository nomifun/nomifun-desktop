//! Pure mapping from a turn's terminal relay outcome to the structured
//! `(result_error_code, result_error_retryable)` receipt columns (spec D4).
//!
//! Codes are stable snake_case tokens shared with the wire contract and the
//! channel retry logic (batch 2): `AgentErrorCode` variants map to their
//! serde name lowercased (the enum serializes SCREAMING_SNAKE_CASE; the
//! interface contract for these columns is snake_case), plus fixed lifecycle
//! codes for outcomes that never reach an `AgentErrorCode`.

use nomifun_api_types::AgentErrorCode;

use crate::stream_relay::RelayTerminal;

/// Fixed lifecycle codes (not derived from `AgentErrorCode`).
pub const EMPTY_FINAL_TEXT: &str = "empty_final_text";
pub const CHANNEL_CLOSED: &str = "channel_closed";
pub const TURN_CANCELLED: &str = "turn_cancelled";
pub const OWNER_TASK_EXITED: &str = "owner_task_exited";
pub const ADMISSION_REJECTED: &str = "admission_rejected";
pub const PREPARATION_FAILED: &str = "preparation_failed";
const UNKNOWN_UPSTREAM_ERROR: &str = "unknown_upstream_error";

/// Retryability of the fixed lifecycle codes, per the D4 contract.
pub const fn fixed_code_retryable(code: &str) -> bool {
    matches!(
        code.as_bytes(),
        b"channel_closed" | b"owner_task_exited" | b"preparation_failed"
    )
}

/// One fixed lifecycle failure as a completion pair.
pub fn fixed_failure(code: &'static str) -> Option<(String, bool)> {
    Some((code.to_owned(), fixed_code_retryable(code)))
}

/// Map a terminal relay outcome (+ the final assistant text) to the
/// structured receipt error columns.
///
/// `None` means success. `Some((code, retryable))` is a failure with a stable
/// machine-readable code. `Finish` with an empty/whitespace final text is the
/// previously silent `result_ok = false` asymmetry and now yields
/// `empty_final_text` (not retryable).
pub fn map_turn_failure(
    terminal: &RelayTerminal,
    final_text: Option<&str>,
) -> Option<(String, bool)> {
    match terminal {
        RelayTerminal::Finish => {
            if final_text.is_some_and(|text| !text.trim().is_empty()) {
                None
            } else {
                Some((EMPTY_FINAL_TEXT.to_owned(), false))
            }
        }
        RelayTerminal::ChannelClosed => Some((CHANNEL_CLOSED.to_owned(), true)),
        RelayTerminal::Error { code, retryable } => Some((
            code.map(agent_error_code_token)
                .unwrap_or_else(|| UNKNOWN_UPSTREAM_ERROR.to_owned()),
            retryable.unwrap_or(false),
        )),
    }
}

/// The snake_case wire token for one `AgentErrorCode` variant: its serde name
/// (SCREAMING_SNAKE_CASE in `agent_error.rs`) lowercased.
fn agent_error_code_token(code: AgentErrorCode) -> String {
    match serde_json::to_value(code) {
        Ok(serde_json::Value::String(name)) => name.to_ascii_lowercase(),
        _ => UNKNOWN_UPSTREAM_ERROR.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_with_text_is_success() {
        assert_eq!(map_turn_failure(&RelayTerminal::Finish, Some("ok")), None);
    }

    #[test]
    fn finish_with_empty_text_is_empty_final_text_not_retryable() {
        assert_eq!(
            map_turn_failure(&RelayTerminal::Finish, Some("  ")),
            Some(("empty_final_text".into(), false))
        );
    }

    #[test]
    fn finish_without_text_is_empty_final_text_not_retryable() {
        assert_eq!(
            map_turn_failure(&RelayTerminal::Finish, None),
            Some(("empty_final_text".into(), false))
        );
    }

    #[test]
    fn channel_closed_is_retryable() {
        assert_eq!(
            map_turn_failure(&RelayTerminal::ChannelClosed, None),
            Some(("channel_closed".into(), true))
        );
    }

    #[test]
    fn error_uses_agent_error_code_serde_name_and_retryable() {
        let t = RelayTerminal::Error {
            code: Some(AgentErrorCode::UserLlmProviderRateLimited),
            retryable: Some(true),
        };
        assert_eq!(
            map_turn_failure(&t, None),
            Some(("user_llm_provider_rate_limited".into(), true))
        );
    }

    #[test]
    fn error_without_code_defaults_unknown_upstream_and_uses_retryable_flag() {
        let t = RelayTerminal::Error {
            code: None,
            retryable: None,
        };
        assert_eq!(
            map_turn_failure(&t, None),
            Some(("unknown_upstream_error".into(), false))
        );
    }

    #[test]
    fn error_final_text_never_masks_the_failure() {
        let t = RelayTerminal::Error {
            code: Some(AgentErrorCode::NomifunStreamBroken),
            retryable: Some(false),
        };
        assert_eq!(
            map_turn_failure(&t, Some("partial output")),
            Some(("nomifun_stream_broken".into(), false))
        );
    }

    #[test]
    fn fixed_lifecycle_codes_have_contracted_retryability() {
        for (code, retryable) in [
            (EMPTY_FINAL_TEXT, false),
            (CHANNEL_CLOSED, true),
            (TURN_CANCELLED, false),
            (OWNER_TASK_EXITED, true),
            (ADMISSION_REJECTED, false),
            (PREPARATION_FAILED, true),
        ] {
            assert_eq!(
                fixed_code_retryable(code),
                retryable,
                "retryability contract for {code}"
            );
        }
    }
}
