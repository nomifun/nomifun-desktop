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
/// A durable Running turn settled by restart-orphan recovery: the owning
/// process exited before any terminal outcome was written, and boot-time
/// terminal proof later closed the generation. Retryable — the request never
/// produced a result, so resending it is safe once the row is finished.
pub const INTERRUPTED_BY_RESTART: &str = "interrupted_by_restart";
const UNKNOWN_UPSTREAM_ERROR: &str = "unknown_upstream_error";

/// Retryability of the fixed lifecycle codes, per the D4 contract.
pub const fn fixed_code_retryable(code: &str) -> bool {
    matches!(
        code.as_bytes(),
        b"channel_closed" | b"owner_task_exited" | b"preparation_failed" | b"interrupted_by_restart"
    )
}

/// One fixed lifecycle failure as a completion pair.
pub fn fixed_failure(code: &'static str) -> Option<(String, bool)> {
    Some((code.to_owned(), fixed_code_retryable(code)))
}

/// Classify a runtime-BUILD failure (factory/spawn/handshake) into the same
/// structured code the chat error card carries, instead of the flat
/// `preparation_failed`. Channel retry logic and Mac-side receipt debugging
/// can then distinguish `user_agent_handshake_timeout` from
/// `user_agent_not_installed` etc. without parsing `result_error` text.
///
/// Runs the error through `AgentSendError`'s classifier — the exact mapping
/// used for the persisted `tips` card — so the receipt and the card always
/// agree. Unclassifiable errors (the catch-all `UNKNOWN_UPSTREAM_ERROR`
/// bucket) keep the legacy `preparation_failed` code: for a build-phase
/// failure that generic bucket carries no signal, while `preparation_failed`
/// at least names the phase.
pub fn classified_preparation_failure(
    err: &nomifun_common::AppError,
) -> Option<(String, bool)> {
    let stream_error =
        nomifun_ai_agent::AgentSendError::from_app_error_ref(err).into_stream_error();
    match stream_error.code {
        Some(code) if code != AgentErrorCode::UnknownUpstreamError => Some((
            agent_error_code_token(code),
            stream_error
                .retryable
                .unwrap_or(fixed_code_retryable(PREPARATION_FAILED)),
        )),
        _ => fixed_failure(PREPARATION_FAILED),
    }
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
            (INTERRUPTED_BY_RESTART, true),
        ] {
            assert_eq!(
                fixed_code_retryable(code),
                retryable,
                "retryability contract for {code}"
            );
        }
    }

    /// Build-phase failures carry the classified code on the receipt: the
    /// exact classifier the chat error card uses, so both surfaces agree.
    #[test]
    fn classified_preparation_failure_carries_handshake_timeout_code() {
        let err = nomifun_common::AppError::BadGateway(
            "Initialize handshake timed out after 120s".into(),
        );
        assert_eq!(
            classified_preparation_failure(&err),
            Some(("user_agent_handshake_timeout".into(), true))
        );

        let err = nomifun_common::AppError::BadGateway(
            "Agent process exited before initialize handshake completed (exit code 1)".into(),
        );
        assert_eq!(
            classified_preparation_failure(&err),
            Some(("user_agent_handshake_failed".into(), true))
        );
    }

    /// Unclassifiable build failures keep the legacy phase code so receipt
    /// consumers keying on `preparation_failed` still see build failures.
    #[test]
    fn classified_preparation_failure_falls_back_to_preparation_failed() {
        let err = nomifun_common::AppError::BadGateway("agent exploded mysteriously".into());
        assert_eq!(
            classified_preparation_failure(&err),
            Some((PREPARATION_FAILED.to_owned(), true))
        );
    }
}
