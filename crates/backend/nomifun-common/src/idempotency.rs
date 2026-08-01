//! Shared Idempotency-Key validation.
//!
//! One home for the "1..=N visible ASCII bytes" character rule and the
//! "exactly one `Idempotency-Key` header" extraction that the gateway MCP
//! endpoint, the public companion surface, and the stdio bridges all enforce.
//! Callers that need different client-visible error wording (e.g. the cron
//! REST route) call [`is_visible_ascii_key`] and keep their own errors.

use axum::http::HeaderMap;

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Max accepted `Idempotency-Key` length (bytes).
pub const MAX_IDEMPOTENCY_KEY_LEN: usize = 128;

/// The shared character rule: `1..=max_len` bytes, all visible ASCII
/// (`0x21..=0x7e` — no spaces, no controls).
pub fn is_visible_ascii_key(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// Extract the required `Idempotency-Key` header: exactly one occurrence,
/// [`MAX_IDEMPOTENCY_KEY_LEN`]-capped visible ASCII.
pub fn required_idempotency_key(headers: &HeaderMap) -> Result<&str, &'static str> {
    let mut values = headers.get_all(IDEMPOTENCY_KEY_HEADER).iter();
    let Some(value) = values.next() else {
        return Err("missing Idempotency-Key header");
    };
    if values.next().is_some() {
        return Err("expected exactly one Idempotency-Key header");
    }
    let value = value
        .to_str()
        .map_err(|_| "Idempotency-Key must be visible ASCII")?;
    if !is_visible_ascii_key(value, MAX_IDEMPOTENCY_KEY_LEN) {
        return Err("Idempotency-Key must contain 1..=128 visible ASCII bytes");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn header_requires_exactly_one_visible_ascii_value() {
        let missing = HeaderMap::new();
        assert!(required_idempotency_key(&missing).is_err());

        let mut duplicate = HeaderMap::new();
        duplicate.append(IDEMPOTENCY_KEY_HEADER, HeaderValue::from_static("first"));
        duplicate.append(IDEMPOTENCY_KEY_HEADER, HeaderValue::from_static("second"));
        assert!(required_idempotency_key(&duplicate).is_err());

        for illegal in ["", "contains space"] {
            let mut headers = HeaderMap::new();
            headers.insert(IDEMPOTENCY_KEY_HEADER, HeaderValue::from_str(illegal).unwrap());
            assert!(required_idempotency_key(&headers).is_err());
        }

        let mut oversized = HeaderMap::new();
        oversized.insert(
            IDEMPOTENCY_KEY_HEADER,
            HeaderValue::from_str(&"x".repeat(129)).unwrap(),
        );
        assert!(required_idempotency_key(&oversized).is_err());

        let mut valid = HeaderMap::new();
        valid.insert(
            IDEMPOTENCY_KEY_HEADER,
            HeaderValue::from_static("gateway-tool-v1-abc_123"),
        );
        assert_eq!(
            required_idempotency_key(&valid).unwrap(),
            "gateway-tool-v1-abc_123"
        );
    }

    #[test]
    fn visible_ascii_rule_bounds() {
        assert!(is_visible_ascii_key("a", 128));
        assert!(is_visible_ascii_key(&"x".repeat(128), 128));
        assert!(!is_visible_ascii_key("", 128));
        assert!(!is_visible_ascii_key(&"x".repeat(129), 128));
        assert!(!is_visible_ascii_key("has space", 128));
        assert!(!is_visible_ascii_key("tab\there", 128));
        assert!(!is_visible_ascii_key("ünïcode", 128));
    }
}
