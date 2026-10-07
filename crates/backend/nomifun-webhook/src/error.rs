use thiserror::Error;

/// Errors from outbound webhook delivery. These are surfaced to clients only via
/// the explicit `/test` endpoint (as a 502); the completion notifier logs and
/// swallows them so a failing webhook never affects requirement state.
#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("signing failed: {0}")]
    Sign(String),

    #[error("request failed: {0}")]
    Http(String),

    #[error("remote rejected the webhook: {0}")]
    Remote(String),

    #[error("webhook delivery outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

impl WebhookError {
    pub const fn outcome_unknown(&self) -> bool {
        matches!(self, Self::OutcomeUnknown(_))
    }
}
