//! Small shared helpers.

/// Return the last `max_chars` bytes of `s`, snapped to a char boundary so the
/// result is always valid UTF-8 (never panics on multibyte content). When `s`
/// fits, returns it unchanged.
pub fn tail_chars(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let mut start = s.len() - max_chars;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_shorter_than_limit_unchanged() {
        assert_eq!(tail_chars("abc", 10), "abc");
    }

    #[test]
    fn tail_truncates_to_limit() {
        assert_eq!(tail_chars("abcdefgh", 3), "fgh");
    }

    #[test]
    fn tail_never_splits_multibyte() {
        // "你好世界" is 12 bytes (3 each). Asking for 7 bytes must snap forward
        // to a char boundary, never panic.
        let s = "你好世界";
        let out = tail_chars(s, 7);
        assert!(s.ends_with(&out));
        // valid UTF-8 by construction (String); length is a whole number of chars
        assert_eq!(out.chars().count(), 2);
    }
}
