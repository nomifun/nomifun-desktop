//! Small helpers shared by the channel plugins.

use std::time::Duration;

/// Exponential reconnect backoff: `2^attempt` seconds, capped at `cap`.
pub(crate) fn backoff_delay(attempt: u32, cap: Duration) -> Duration {
    let secs = 2u64.saturating_pow(attempt).min(cap.as_secs());
    Duration::from_secs(secs)
}

/// Truncate `text` to at most `limit` characters, appending `"..."` if cut.
///
/// Counts `char`s (not bytes) and cuts at a char boundary, so multibyte text
/// is never split mid-character and a message whose char count fits the limit
/// is returned unchanged.
pub(crate) fn truncate_message(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let truncated: String = text.chars().take(limit.saturating_sub(3)).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::RECONNECT_MAX_DELAY;

    #[test]
    fn backoff_exponential() {
        assert_eq!(backoff_delay(1, RECONNECT_MAX_DELAY), Duration::from_secs(2));
        assert_eq!(backoff_delay(2, RECONNECT_MAX_DELAY), Duration::from_secs(4));
        assert_eq!(backoff_delay(3, RECONNECT_MAX_DELAY), Duration::from_secs(8));
        assert_eq!(backoff_delay(4, RECONNECT_MAX_DELAY), Duration::from_secs(16));
    }

    #[test]
    fn backoff_capped() {
        // 2^5 = 32, capped to 30.
        assert_eq!(backoff_delay(5, RECONNECT_MAX_DELAY), Duration::from_secs(30));
        assert_eq!(backoff_delay(10, RECONNECT_MAX_DELAY), Duration::from_secs(30));
    }

    #[test]
    fn truncate_within_limit() {
        assert_eq!(truncate_message("Hello, world!", 100), "Hello, world!");
    }

    #[test]
    fn truncate_at_limit() {
        assert_eq!(truncate_message("abc", 3), "abc");
    }

    #[test]
    fn truncate_exceeds_limit() {
        let result = truncate_message("Hello, world!", 10);
        assert_eq!(result, "Hello, ...");
        assert!(result.len() <= 10);
    }

    #[test]
    fn truncate_unicode() {
        // chars().take(2) = "你好", then "..."
        assert_eq!(truncate_message("你好世界测试文本", 5), "你好...");
    }

    #[test]
    fn truncate_multibyte_within_char_limit() {
        // 2 chars / 6 bytes: fits the char limit, returned unchanged.
        assert_eq!(truncate_message("你好", 5), "你好");
    }

    #[test]
    fn truncate_tiny_limit_saturates() {
        // limit < 3 must not underflow.
        assert_eq!(truncate_message("abcdef", 2), "...");
    }

    #[test]
    fn truncate_empty() {
        assert_eq!(truncate_message("", 4000), "");
    }
}
