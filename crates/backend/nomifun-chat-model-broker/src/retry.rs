//! One bounded, cancellation-aware retry owner lives in the broker. Delays
//! apply only before semantic output; they never authorize replay of tools.
use std::time::Duration;

use crate::ChatModelError;

const INITIAL_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 8_000;
const MAX_RETRY_WAIT_MS: u64 = 120_000;

pub(crate) fn delay(error: &ChatModelError, failed_attempt: u8) -> Option<Duration> {
    // UUID's random tail supplies per-request jitter without a shared RNG lock
    // or a seed derived from credentials, provider data or prompt contents.
    let random = uuid::Uuid::now_v7().as_u128() as u64;
    delay_with_jitter(error, failed_attempt, random)
}

fn delay_with_jitter(
    error: &ChatModelError,
    failed_attempt: u8,
    random: u64,
) -> Option<Duration> {
    if error.semantic_output_committed {
        return None;
    }
    let exponent = u32::from(failed_attempt.saturating_sub(1)).min(5);
    let ceiling = (INITIAL_BACKOFF_MS << exponent).min(MAX_BACKOFF_MS);
    // Equal jitter keeps a nonzero backoff while spreading concurrent clients.
    let floor = ceiling / 2;
    let jittered = floor + random % (ceiling - floor + 1);
    let wait = jittered.max(error.retry_after_ms.unwrap_or(0));
    // Never shorten a server's Retry-After to fit a local request budget.
    // An excessive delay is surfaced to the caller instead of busy retrying.
    (wait <= MAX_RETRY_WAIT_MS).then(|| Duration::from_millis(wait))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatModelErrorCode, ChatRetryDirective};

    fn transient() -> ChatModelError {
        ChatModelError::new(
            ChatModelErrorCode::RateLimited,
            "provider rate limited the attempt",
            ChatRetryDirective::RetrySameRoute,
        )
    }

    #[test]
    fn exponential_equal_jitter_is_nonzero_and_capped() {
        for attempt in 1..=u8::MAX {
            let expected = (500_u64 << u32::from(attempt.saturating_sub(1)).min(5)).min(8_000);
            for random in [0, 1, 250, 500, u64::MAX] {
                let wait = delay_with_jitter(&transient(), attempt, random).unwrap().as_millis();
                assert!((u128::from(expected / 2)..=u128::from(expected)).contains(&wait));
            }
        }
    }

    #[test]
    fn server_delay_is_a_minimum_not_a_jitter_ceiling() {
        let mut error = transient();
        error.retry_after_ms = Some(7_000);
        assert_eq!(delay_with_jitter(&error, 1, 0), Some(Duration::from_secs(7)));
        error.retry_after_ms = Some(MAX_RETRY_WAIT_MS);
        assert_eq!(delay_with_jitter(&error, 1, 0), Some(Duration::from_secs(120)));
        error.retry_after_ms = Some(MAX_RETRY_WAIT_MS + 1);
        assert_eq!(delay_with_jitter(&error, 1, 0), None);
        error.retry_after_ms = Some(u64::MAX);
        assert_eq!(delay_with_jitter(&error, 1, u64::MAX), None);
    }

    #[test]
    fn committed_output_cannot_be_retried_even_with_an_inconsistent_directive() {
        let mut error = transient();
        error.semantic_output_committed = true;
        assert_eq!(delay_with_jitter(&error, 1, 0), None);
    }
}
