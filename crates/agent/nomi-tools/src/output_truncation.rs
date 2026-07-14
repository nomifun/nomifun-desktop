//! Head/tail output truncation for tool results.
//!
//! Preserves a prefix and a suffix on UTF-8 boundaries, dropping the middle
//! and inserting a marker that records how much was removed. Ported (and
//! de-dependency-ed) from codex `utils/string/src/truncate.rs`.
//!
//! Unlike the engine-level fallback in `nomi-agent::tool_execution` (private,
//! char-counted, multi-pass), this is a reusable, single-pass, tested pure
//! function so any tool (Bash today; Grep/Read later) can bound its output.

const APPROX_BYTES_PER_TOKEN: usize = 4;

/// How much output to retain before the middle is elided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationBudget {
    /// Retain at most this many bytes (split across head/tail).
    Bytes(usize),
    /// Retain at most ~this many tokens, estimated at 4 bytes/token.
    Tokens(usize),
}

impl TruncationBudget {
    fn byte_budget(self) -> usize {
        match self {
            TruncationBudget::Bytes(b) => b,
            TruncationBudget::Tokens(t) => t.saturating_mul(APPROX_BYTES_PER_TOKEN),
        }
    }
    fn use_tokens(self) -> bool {
        matches!(self, TruncationBudget::Tokens(_))
    }
}

/// Truncate `s` to `budget`, keeping the head and tail and eliding the middle.
///
/// Returns the original string untouched when it already fits. Otherwise the
/// result is `<head><marker><tail>` where the marker reports the elided amount,
/// e.g. `…12345 chars truncated…`. UTF-8 char boundaries are always respected.
pub fn truncate_middle(s: &str, budget: TruncationBudget) -> String {
    let max_bytes = budget.byte_budget();
    let use_tokens = budget.use_tokens();

    if s.is_empty() {
        return String::new();
    }
    if max_bytes == 0 {
        let total_chars = s.chars().count();
        return marker(use_tokens, removed_units(use_tokens, s.len(), total_chars));
    }
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let total_bytes = s.len();
    let (left_budget, right_budget) = split_budget(max_bytes);
    let (removed_chars, left, right) = split_string(s, left_budget, right_budget);
    let marker = marker(
        use_tokens,
        removed_units(use_tokens, total_bytes.saturating_sub(max_bytes), removed_chars),
    );

    let mut out = String::with_capacity(left.len() + marker.len() + right.len());
    out.push_str(left);
    out.push_str(&marker);
    out.push_str(right);
    out
}

/// Approximate token count for a string (~4 bytes/token), saturating (ceil).
pub fn approx_token_count(text: &str) -> usize {
    text.len()
        .saturating_add(APPROX_BYTES_PER_TOKEN.saturating_sub(1))
        / APPROX_BYTES_PER_TOKEN
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

fn marker(use_tokens: bool, removed: u64) -> String {
    if use_tokens {
        format!("\n…{removed} tokens truncated…\n")
    } else {
        format!("\n…{removed} chars truncated…\n")
    }
}

fn removed_units(use_tokens: bool, removed_bytes: usize, removed_chars: usize) -> u64 {
    if use_tokens {
        (removed_bytes as u64).saturating_add(APPROX_BYTES_PER_TOKEN as u64 - 1)
            / APPROX_BYTES_PER_TOKEN as u64
    } else {
        u64::try_from(removed_chars).unwrap_or(u64::MAX)
    }
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
        // total_bytes - max_bytes = 100 - 20 = 80
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

    #[test]
    fn token_budget_path() {
        let input = "a".repeat(100);
        // Tokens(5) => 20 bytes budget, input is 100 bytes => truncated
        let result = truncate_middle(&input, TruncationBudget::Tokens(5));
        assert!(result.contains("tokens truncated"), "got: {result}");
        // small input under token budget is unchanged
        assert_eq!(truncate_middle("abcd", TruncationBudget::Tokens(5)), "abcd");
    }

    #[test]
    fn approx_token_count_basic() {
        assert_eq!(approx_token_count(""), 0);
        assert_eq!(approx_token_count("abcd"), 1);
        assert_eq!(approx_token_count("abcde"), 2); // ceil(5/4)
    }
}
