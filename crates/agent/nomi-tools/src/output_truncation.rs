//! Head/tail output truncation for tool results.
//!
//! Preserves a prefix and a suffix on UTF-8 boundaries, dropping the middle
//! and inserting a marker that records how much was removed. Ported (and
//! de-dependency-ed) from codex `utils/string/src/truncate.rs`.
//!
//! Unlike the engine-level fallback in `nomi-agent::tool_execution` (private,
//! char-counted, multi-pass), this is a reusable, single-pass, tested pure
//! function (used by Read today) so any tool can bound its output.

/// How much output to retain before the middle is elided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationBudget {
    /// Retain at most this many bytes (split across head/tail).
    Bytes(usize),
}

impl TruncationBudget {
    fn byte_budget(self) -> usize {
        match self {
            TruncationBudget::Bytes(b) => b,
        }
    }
}

/// Truncate `s` to `budget`, keeping the head and tail and eliding the middle.
///
/// Returns the original string untouched when it already fits. Otherwise the
/// result is `<head><marker><tail>` where the marker reports the elided amount,
/// e.g. `…12345 chars truncated…`. UTF-8 char boundaries are always respected.
pub fn truncate_middle(s: &str, budget: TruncationBudget) -> String {
    let max_bytes = budget.byte_budget();

    if s.is_empty() {
        return String::new();
    }
    if max_bytes == 0 {
        let total_chars = s.chars().count();
        return marker(u64::try_from(total_chars).unwrap_or(u64::MAX));
    }
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let (left_budget, right_budget) = split_budget(max_bytes);
    let (removed_chars, left, right) = split_string(s, left_budget, right_budget);
    let marker = marker(u64::try_from(removed_chars).unwrap_or(u64::MAX));

    let mut out = String::with_capacity(left.len() + marker.len() + right.len());
    out.push_str(left);
    out.push_str(&marker);
    out.push_str(right);
    out
}

fn split_budget(budget: usize) -> (usize, usize) {
    let left = budget / 2;
    (left, budget - left)
}

/// Walk char boundaries: fill `beginning_bytes` into the prefix, find the first
/// char whose start lands in the trailing `end_bytes` window for the suffix,
/// and count the chars dropped in between. All slice boundaries are guaranteed
/// to land on char boundaries, so the returned `&str`s are always valid.
fn split_string(s: &str, beginning_bytes: usize, end_bytes: usize) -> (usize, &str, &str) {
    let len = s.len();
    let tail_start_target = len.saturating_sub(end_bytes);
    let mut prefix_end = 0usize;
    let mut suffix_start = len;
    let mut removed_chars = 0usize;
    let mut suffix_started = false;

    for (idx, ch) in s.char_indices() {
        let char_end = idx + ch.len_utf8();
        if char_end <= beginning_bytes {
            prefix_end = char_end;
            continue;
        }
        if idx >= tail_start_target {
            if !suffix_started {
                suffix_start = idx;
                suffix_started = true;
            }
            continue;
        }
        removed_chars = removed_chars.saturating_add(1);
    }

    if suffix_start < prefix_end {
        suffix_start = prefix_end;
    }
    (removed_chars, &s[..prefix_end], &s[suffix_start..])
}

fn marker(removed: u64) -> String {
    format!("\n…{removed} chars truncated…\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_input_unchanged() {
        assert_eq!(truncate_middle("hello", TruncationBudget::Bytes(50_000)), "hello");
        // exactly at budget is also unchanged
        assert_eq!(truncate_middle("hello", TruncationBudget::Bytes(5)), "hello");
    }

    #[test]
    fn empty_input() {
        assert_eq!(truncate_middle("", TruncationBudget::Bytes(10)), "");
        assert_eq!(truncate_middle("", TruncationBudget::Bytes(0)), "");
    }

    #[test]
    fn large_input_keeps_head_and_tail() {
        let input = format!("{}{}", "0".repeat(100), "1".repeat(100));
        let result = truncate_middle(&input, TruncationBudget::Bytes(20));
        assert!(result.starts_with('0'), "should keep head: {result}");
        assert!(result.ends_with('1'), "should keep tail: {result}");
        assert!(result.contains("chars truncated"), "should mark elision: {result}");
        assert!(result.len() < input.len());
    }

    #[test]
    fn marker_reports_removed_count() {
        let input = "a".repeat(100);
        let result = truncate_middle(&input, TruncationBudget::Bytes(20));
        // total_chars - retained_chars = 100 - 20 = 80
        assert!(result.contains("80 chars truncated"), "got: {result}");
    }

    #[test]
    fn utf8_boundary_safe_multibyte() {
        let input = "é".repeat(100); // 2 bytes each => 200 bytes
        let result = truncate_middle(&input, TruncationBudget::Bytes(21)); // odd budget
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(!result.contains('\u{FFFD}'), "no replacement chars");
        // every byte index that starts a slice must be a char boundary (no panic implies it)
    }

    #[test]
    fn utf8_boundary_safe_emoji() {
        let input = "🦀".repeat(50); // 4 bytes each => 200 bytes
        let result = truncate_middle(&input, TruncationBudget::Bytes(10));
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.starts_with('🦀'), "head crab intact: {result}");
        assert!(result.ends_with('🦀'), "tail crab intact: {result}");
    }

    #[test]
    fn budget_zero_returns_only_marker() {
        let result = truncate_middle("hello world", TruncationBudget::Bytes(0));
        assert!(result.contains("chars truncated"));
        assert!(!result.contains("hello"));
    }

    #[test]
    fn budget_one_no_overlap_no_panic() {
        let input = "abcdefghij";
        let result = truncate_middle(input, TruncationBudget::Bytes(1));
        // head gets 0 bytes (1/2), tail gets 1 byte; no overlap, valid utf8
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.contains("chars truncated"));
    }
}
