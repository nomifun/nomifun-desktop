//! `order_key` (display form like "1.2") → `sort_seq` (lexically sortable).
//!
//! Each numeric segment is zero-padded to 8 digits and joined with '.'. Because
//! '.' (0x2E) sorts before digits (0x30+), a parent ("1" → "00000001") sorts
//! before its children ("1.1" → "00000001.00000001"). Empty/invalid keys map to
//! a high sentinel so they sort last.

const SEGMENT_WIDTH: usize = 8;
const SENTINEL: &str = "99999999";

/// Normalize a dotted-decimal `order_key` into a lexically-sortable `sort_seq`.
pub fn to_sort_seq(order_key: &str) -> String {
    let trimmed = order_key.trim();
    if trimmed.is_empty() {
        return SENTINEL.to_string();
    }
    let mut segments = Vec::new();
    for raw in trimmed.split('.') {
        let raw = raw.trim();
        match raw.parse::<u64>() {
            Ok(n) => segments.push(format!("{n:0>width$}", width = SEGMENT_WIDTH)),
            // Any malformed segment ⇒ whole key sorts last.
            Err(_) => return SENTINEL.to_string(),
        }
    }
    segments.join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_examples() {
        assert_eq!(to_sort_seq("1"), "00000001");
        assert_eq!(to_sort_seq("2"), "00000002");
        assert_eq!(to_sort_seq("1.1"), "00000001.00000001");
        assert_eq!(to_sort_seq("1.2"), "00000001.00000002");
        assert_eq!(to_sort_seq("1.10"), "00000001.00000010");
        assert_eq!(to_sort_seq("2.3.1"), "00000002.00000003.00000001");
    }

    #[test]
    fn ordering_is_correct() {
        let mut keys = vec!["2", "1.10", "1.2", "1.1", "1", "2.3.1"];
        keys.sort_by_key(|k| to_sort_seq(k));
        assert_eq!(keys, vec!["1", "1.1", "1.2", "1.10", "2", "2.3.1"]);
    }

    #[test]
    fn empty_and_malformed_sort_last() {
        assert_eq!(to_sort_seq(""), SENTINEL);
        assert_eq!(to_sort_seq("   "), SENTINEL);
        assert_eq!(to_sort_seq("abc"), SENTINEL);
        assert_eq!(to_sort_seq("1.x"), SENTINEL);
        // A real key sorts before the sentinel.
        assert!(to_sort_seq("9999") < to_sort_seq(""));
    }
}
