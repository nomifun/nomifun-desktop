//! Pure mapping from a turn's terminal relay outcome to the structured
//! `(result_error_code, result_error_retryable)` receipt columns (spec D4).
//!
//! Codes are stable snake_case tokens shared with the wire contract and the
//! channel retry logic (batch 2): `AgentErrorCode` variants map to their
//! serde name lowercased (the enum serializes SCREAMING_SNAKE_CASE; the
//! interface contract for these columns is snake_case), plus fixed lifecycle
//! codes for outcomes that never reach an `AgentErrorCode`.

use nomifun_ai_agent::protocol::events::TurnStopReason;
use nomifun_api_types::AgentErrorCode;

use crate::stream_relay::{RelayOutcome, RelayTerminal};

/// Fixed lifecycle codes (not derived from `AgentErrorCode`).
pub const EMPTY_FINAL_TEXT: &str = "empty_final_text";
pub const CHANNEL_CLOSED: &str = "channel_closed";
pub const TURN_CANCELLED: &str = "turn_cancelled";
pub const OWNER_TASK_EXITED: &str = "owner_task_exited";
pub const ADMISSION_REJECTED: &str = "admission_rejected";
pub const PREPARATION_FAILED: &str = "preparation_failed";
/// The model stopped because it hit the per-request output ceiling before the
/// request was delivered. Retryable, and specifically *resumable*: the work
/// already done is real, so the correct recovery is another round against the
/// original requirement rather than a fresh generation from zero.
pub const OUTPUT_TRUNCATED: &str = "output_truncated";
/// The turn consumed its whole per-turn provider-request budget. Retryable for
/// the same reason as `output_truncated` — partial progress is genuine.
pub const TURN_REQUESTS_EXHAUSTED: &str = "turn_requests_exhausted";
/// The model refused the request. Never retryable: an identical resend produces
/// an identical refusal.
pub const MODEL_REFUSED: &str = "model_refused";
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
        b"channel_closed"
            | b"owner_task_exited"
            | b"preparation_failed"
            | b"interrupted_by_restart"
            | b"output_truncated"
            | b"turn_requests_exhausted"
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

/// The fixed code for a `Finish` whose stop reason proves the turn did not
/// accomplish what it was asked to do.
///
/// `EndTurn` (and a legacy `None`) are the only clean completions. Every other
/// reason means the provider stopped the model before the request was
/// delivered, which is a turn FAILURE no matter how much prose was streamed
/// first — the contract on [`TurnStopReason`] says so explicitly, and the IM
/// channel path already enforces it
/// (`nomifun-channel/src/message_service.rs:767-796`). The conversation path
/// used to ignore this, which is how a truncated turn that wrote nothing to
/// disk was recorded as `result_ok = 1`.
pub const fn incomplete_stop_code(stop_reason: Option<TurnStopReason>) -> Option<&'static str> {
    match stop_reason {
        None | Some(TurnStopReason::EndTurn) => None,
        Some(TurnStopReason::MaxTokens) => Some(OUTPUT_TRUNCATED),
        Some(TurnStopReason::MaxTurnRequests) => Some(TURN_REQUESTS_EXHAUSTED),
        Some(TurnStopReason::Refusal) => Some(MODEL_REFUSED),
        Some(TurnStopReason::Cancelled) => Some(TURN_CANCELLED),
    }
}

/// Whether a terminal relay outcome has a durable result that can satisfy the
/// accepted turn.
///
/// Takes the whole [`RelayOutcome`] rather than a few extracted fields: the stop
/// reason lives on the same struct as the terminal and the output evidence, and
/// passing them separately is what let three consumers silently adjudicate on a
/// subset.
///
/// `committed_artifact_count` is passed separately and deliberately: it is the
/// TURN-scoped total accumulated across every continuation resend in the send
/// loop, which is strictly greater than any single `outcome`'s per-pass count.
/// A committed artifact batch is first-class output — native image turns
/// intentionally suppress provider prose until the host has verified and
/// atomically committed the image receipts.
pub fn turn_succeeded(outcome: &RelayOutcome, committed_artifact_count: usize) -> bool {
    matches!(outcome.terminal, RelayTerminal::Finish)
        && incomplete_stop_code(outcome.stop_reason).is_none()
        && (outcome
            .final_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
            || committed_artifact_count > 0)
}

/// Map a terminal relay outcome plus its durable output evidence to the
/// structured receipt error columns.
///
/// `None` means success. `Some((code, retryable))` is a failure with a stable
/// machine-readable code. Order matters: an incomplete stop reason is reported
/// as ITSELF, before the empty-text check. A truncated turn typically streamed
/// a great deal of text, so stamping it `empty_final_text` would be both a lie
/// and useless to the UI, which needs a distinct code to offer a resume.
/// Provisional tool output and uncommitted artifact receipts must never be
/// counted here.
pub fn map_turn_failure(
    outcome: &RelayOutcome,
    committed_artifact_count: usize,
) -> Option<(String, bool)> {
    match &outcome.terminal {
        RelayTerminal::Finish => {
            if let Some(code) = incomplete_stop_code(outcome.stop_reason) {
                return Some((code.to_owned(), fixed_code_retryable(code)));
            }
            if turn_succeeded(outcome, committed_artifact_count) {
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

    /// A `Finish` outcome with an explicit stop reason and output evidence.
    fn finish(stop_reason: Option<TurnStopReason>, text: Option<&str>, artifacts: usize) -> RelayOutcome {
        RelayOutcome {
            terminal: RelayTerminal::Finish,
            stop_reason,
            final_text: text.map(str::to_owned),
            committed_artifact_count: artifacts,
            ..RelayOutcome::default()
        }
    }

    fn terminal(terminal: RelayTerminal, text: Option<&str>, artifacts: usize) -> RelayOutcome {
        RelayOutcome {
            terminal,
            stop_reason: None,
            final_text: text.map(str::to_owned),
            committed_artifact_count: artifacts,
            ..RelayOutcome::default()
        }
    }

    #[test]
    fn finish_with_text_is_success() {
        let outcome = finish(Some(TurnStopReason::EndTurn), Some("ok"), 0);
        assert!(turn_succeeded(&outcome, 0));
        assert_eq!(map_turn_failure(&outcome, 0), None);
    }

    /// A legacy `None` stop reason stays a clean completion: older producers did
    /// not populate it and must not retroactively become failures.
    #[test]
    fn finish_without_a_stop_reason_is_success() {
        assert_eq!(map_turn_failure(&finish(None, Some("ok"), 0), 0), None);
    }

    #[test]
    fn finish_with_committed_artifact_is_success_without_text() {
        let outcome = finish(Some(TurnStopReason::EndTurn), None, 1);
        assert!(turn_succeeded(&outcome, 1));
        assert_eq!(map_turn_failure(&outcome, 1), None);
    }

    #[test]
    fn finish_with_empty_text_is_empty_final_text_not_retryable() {
        assert_eq!(
            map_turn_failure(&finish(Some(TurnStopReason::EndTurn), Some("  "), 0), 0),
            Some(("empty_final_text".into(), false))
        );
    }

    #[test]
    fn finish_without_text_is_empty_final_text_not_retryable() {
        assert_eq!(
            map_turn_failure(&finish(Some(TurnStopReason::EndTurn), None, 0), 0),
            Some(("empty_final_text".into(), false))
        );
    }

    /// THE regression this module exists to prevent. A turn that streamed 82 KB
    /// of prose and then stopped on the output ceiling produced NOTHING the user
    /// asked for. Non-empty text is not proof of delivery.
    #[test]
    fn a_truncated_finish_is_never_a_success_however_much_text_it_streamed() {
        let outcome = finish(Some(TurnStopReason::MaxTokens), Some("a".repeat(82_586).as_str()), 0);
        assert!(!turn_succeeded(&outcome, 0));
        assert_eq!(
            map_turn_failure(&outcome, 0),
            Some(("output_truncated".into(), true)),
            "truncation must report ITSELF, not empty_final_text, and must be resumable"
        );
    }

    /// A committed artifact does not launder a truncated turn either: the relay
    /// already rolls those receipts back for any non-EndTurn Finish
    /// (`invalidates_completed_artifacts`), so agreeing here keeps one policy.
    #[test]
    fn a_truncated_finish_is_not_rescued_by_a_committed_artifact() {
        assert!(!turn_succeeded(&finish(Some(TurnStopReason::MaxTokens), None, 3), 3));
    }

    #[test]
    fn an_exhausted_turn_request_budget_is_a_retryable_failure() {
        assert_eq!(
            map_turn_failure(&finish(Some(TurnStopReason::MaxTurnRequests), Some("partial"), 0), 0),
            Some(("turn_requests_exhausted".into(), true))
        );
    }

    /// A refusal is a real answer about the request, so resending it unchanged
    /// cannot help.
    #[test]
    fn a_refusal_is_a_failure_that_is_not_retryable() {
        assert_eq!(
            map_turn_failure(&finish(Some(TurnStopReason::Refusal), Some("I can't help"), 0), 0),
            Some(("model_refused".into(), false))
        );
    }

    #[test]
    fn a_cancelled_finish_reports_the_cancellation() {
        assert_eq!(
            map_turn_failure(&finish(Some(TurnStopReason::Cancelled), Some("partial"), 0), 0),
            Some(("turn_cancelled".into(), false))
        );
    }

    #[test]
    fn channel_closed_is_retryable() {
        assert_eq!(
            map_turn_failure(&terminal(RelayTerminal::ChannelClosed, None, 0), 0),
            Some(("channel_closed".into(), true))
        );
    }

    #[test]
    fn error_uses_agent_error_code_serde_name_and_retryable() {
        let outcome = terminal(
            RelayTerminal::Error {
                code: Some(AgentErrorCode::UserLlmProviderRateLimited),
                retryable: Some(true),
            },
            None,
            0,
        );
        assert_eq!(
            map_turn_failure(&outcome, 0),
            Some(("user_llm_provider_rate_limited".into(), true))
        );
    }

    #[test]
    fn error_without_code_defaults_unknown_upstream_and_uses_retryable_flag() {
        let outcome = terminal(
            RelayTerminal::Error {
                code: None,
                retryable: None,
            },
            None,
            0,
        );
        assert_eq!(
            map_turn_failure(&outcome, 0),
            Some(("unknown_upstream_error".into(), false))
        );
    }

    #[test]
    fn error_final_text_never_masks_the_failure() {
        let outcome = terminal(
            RelayTerminal::Error {
                code: Some(AgentErrorCode::NomifunStreamBroken),
                retryable: Some(false),
            },
            Some("partial output"),
            1,
        );
        assert_eq!(
            map_turn_failure(&outcome, 0),
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
            (OUTPUT_TRUNCATED, true),
            (TURN_REQUESTS_EXHAUSTED, true),
            (MODEL_REFUSED, false),
        ] {
            assert_eq!(
                fixed_code_retryable(code),
                retryable,
                "retryability contract for {code}"
            );
        }
    }

    /// Every incomplete stop reason must map to a code whose retryability the
    /// fixed table actually knows, or the receipt would silently claim a
    /// truncated turn is not worth resuming.
    #[test]
    fn every_incomplete_stop_reason_has_a_contracted_code() {
        for reason in [
            TurnStopReason::MaxTokens,
            TurnStopReason::MaxTurnRequests,
            TurnStopReason::Refusal,
            TurnStopReason::Cancelled,
        ] {
            let code = incomplete_stop_code(Some(reason))
                .unwrap_or_else(|| panic!("{reason:?} must be an incomplete stop reason"));
            let (mapped, retryable) = map_turn_failure(&finish(Some(reason), Some("text"), 0), 0)
                .expect("must be a failure");
            assert_eq!(mapped, code);
            assert_eq!(retryable, fixed_code_retryable(code));
        }
        assert!(incomplete_stop_code(Some(TurnStopReason::EndTurn)).is_none());
        assert!(incomplete_stop_code(None).is_none());
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
