use std::time::{SystemTime, UNIX_EPOCH};

/// Unix timestamp in milliseconds.
pub type TimestampMs = i64;

/// Get current timestamp in milliseconds.
pub fn now_ms() -> TimestampMs {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_millis() as TimestampMs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_ms_positive() {
        assert!(now_ms() > 0);
    }

    #[test]
    fn test_now_ms_reasonable_range() {
        let ts = now_ms();
        // After 2020-01-01 and before 2100-01-01
        assert!(ts > 1_577_836_800_000);
        assert!(ts < 4_102_444_800_000);
    }

    #[test]
    fn test_monotonic() {
        let ts1 = now_ms();
        let ts2 = now_ms();
        assert!(ts2 >= ts1);
    }
}
