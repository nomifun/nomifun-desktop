//! Credential-aware redaction for untrusted upstream diagnostics.
//!
//! Providers sometimes echo authentication material in error bodies. Callers
//! build one redactor from the exact runtime credentials and apply it before
//! classifying, logging, returning, or persisting the diagnostic. Raw and URL
//! form/percent-encoded representations are covered without relying on secret
//! prefixes such as `sk-` or `AKIA`.

use std::{borrow::Cow, ops::Range, sync::Arc};

const REDACTED: &str = "[REDACTED]";

/// An immutable set of exact secret representations.
///
/// Deliberately does not implement `Debug`: its replacement table contains
/// live credential material.
#[derive(Clone, Default)]
pub struct SecretRedactor {
    variants: Arc<Vec<Vec<u8>>>,
}

impl SecretRedactor {
    /// Build a redactor from every credential that can be used by a runtime.
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut variants = Vec::new();
        for secret in secrets {
            let secret = secret.as_ref();
            let trimmed = secret.trim();
            if trimmed.is_empty() {
                continue;
            }
            variants.push(secret.to_owned());
            variants.push(trimmed.to_owned());

            // Query encoders use two common space representations: HTML-form
            // encoding uses `+`, while RFC 3986 component encoding uses
            // `%20`. `form_urlencoded::byte_serialize` only produces the
            // former, so relying on it alone misses the URLs most gateways
            // echo in diagnostics.
            let form_encoded = encode_component(trimmed, true, true);
            let percent_encoded = encode_component(trimmed, false, false);
            for encoded in [&form_encoded, &percent_encoded] {
                variants.push(encoded.to_owned());

                // A gateway may quote an already encoded query value in JSON
                // and encode it once more. Cover both form and RFC spellings.
                variants.push(encode_component(encoded, true, true));
                variants.push(encode_component(encoded, false, false));
            }
        }
        let mut variants: Vec<Vec<u8>> = variants.into_iter()
            .map(|value| canonical_percent_hex(value.as_bytes()).into_owned())
            .collect();
        variants.sort_unstable();
        variants.dedup();
        Self {
            variants: Arc::new(variants),
        }
    }

    /// Whether no exact credential representations were configured. Even when
    /// this is true, [`Self::redact`] still removes embedded URL credentials
    /// because an upstream may mint credentials unknown to us.
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// Return the largest byte boundary that is safe to keep when `input` is
    /// known to have been truncated before redaction.
    ///
    /// A retained tail can be a proper prefix of a raw, form-encoded,
    /// percent-encoded, or double-encoded credential whose remaining bytes
    /// fell beyond the read cap. Exact replacement cannot recognize that
    /// fragment. This method conservatively removes the longest such suffix;
    /// complete variants remain intact for [`Self::redact`] to replace.
    pub fn redaction_safe_truncation_boundary(&self, input: &[u8]) -> usize {
        let canonical = canonical_percent_hex(input);
        let unsafe_suffix = self
            .variants
            .iter()
            .map(|variant| longest_proper_prefix_suffix(&canonical, variant))
            .max()
            .unwrap_or(0);
        let boundary = input.len().saturating_sub(unsafe_suffix);
        if unsafe_suffix == 0 {
            return boundary;
        }
        // Removing a partial credential may cut through a complete overlapping
        // credential. Remove that whole connected match too, not its suffix only.
        self.matching_ranges(&canonical)
            .into_iter()
            .find(|range| range.start < boundary && range.end > boundary)
            .map_or(boundary, |range| range.start)
    }

    /// Replace every exact raw or encoded secret representation.
    pub fn redact(&self, input: &str) -> String {
        let canonical = canonical_percent_hex(input.as_bytes());
        let mut output = String::with_capacity(input.len());
        let mut cursor = 0;
        for range in self.matching_ranges(&canonical) {
            output.push_str(&input[cursor..range.start]);
            output.push_str(REDACTED);
            cursor = range.end;
        }
        output.push_str(&input[cursor..]);
        redact_url_queries(&output)
    }

    /// Match original bytes only, including overlaps. Coalesce per pattern
    /// before sorting so repeated/overlapping secrets do not build a list of
    /// every occurrence. UTF-8 patterns always match on valid string boundaries.
    fn matching_ranges(&self, input: &[u8]) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for variant in self.variants.iter().filter(|value| value.len() <= input.len()) {
            let pattern = variant.as_slice();
            let failure = prefix_failure_table(pattern);
            let mut matched = 0;
            let first_range = ranges.len();
            for (index, byte) in input.iter().enumerate() {
                while matched > 0 && *byte != pattern[matched] {
                    matched = failure[matched - 1];
                }
                if *byte == pattern[matched] {
                    matched += 1;
                }
                if matched == pattern.len() {
                    let range = index + 1 - matched..index + 1;
                    if ranges.len() > first_range && ranges.last().unwrap().end > range.start {
                        ranges.last_mut().unwrap().end = range.end;
                    } else {
                        ranges.push(range);
                    }
                    matched = failure[matched - 1];
                }
            }
        }
        ranges.sort_unstable_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut().filter(|last| last.end > range.start) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }
}

fn prefix_failure_table(pattern: &[u8]) -> Vec<usize> {
    let mut failure = vec![0usize; pattern.len()];
    for index in 1..pattern.len() {
        let mut matched = failure[index - 1];
        while matched > 0 && pattern[index] != pattern[matched] {
            matched = failure[matched - 1];
        }
        if pattern[index] == pattern[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }
    failure
}

/// Find the longest suffix of `input` that is a proper prefix of `pattern`.
///
/// The KMP failure table keeps this linear in the bounded diagnostic and
/// relevant pattern prefix. Only `input.len() + 1` pattern bytes can affect
/// the answer, so an accidentally huge configured credential cannot create an
/// equally huge temporary allocation here.
fn longest_proper_prefix_suffix(input: &[u8], pattern: &[u8]) -> usize {
    if input.is_empty() || pattern.len() < 2 {
        return 0;
    }
    let relevant_len = pattern.len().min(input.len().saturating_add(1));
    let relevant = &pattern[..relevant_len];
    let failure = prefix_failure_table(relevant);

    let mut matched = 0usize;
    for (index, byte) in input.iter().copied().enumerate() {
        while matched > 0 && byte != relevant[matched] {
            matched = failure[matched - 1];
        }
        if byte == relevant[matched] {
            matched += 1;
        }
        if matched == relevant.len() {
            if index + 1 == input.len() && relevant.len() == pattern.len() {
                // The complete variant is safe for the normal exact redactor.
                return 0;
            }
            matched = failure[matched - 1];
        }
    }
    matched.min(pattern.len() - 1)
}

/// Remove query strings and other URL credentials from untrusted diagnostics.
///
/// All explicit `scheme://` URLs are covered, including proxy/WebSocket URLs.
/// Userinfo, query values and fragments may carry credentials unknown to the
/// caller. Keep hosts/routes and surrounding reasons, but not sensitive tails.
/// The historical function name is retained for existing callers.
pub fn redact_url_queries(input: &str) -> String {
    const MARKER: &str = "<redacted>";

    fn next_authority(input: &str, mut from: usize) -> Option<usize> {
        while let Some(offset) = input[from..].find("://") {
            let colon = from + offset;
            let start = input[..colon].as_bytes().iter().rposition(|byte| {
                !byte.is_ascii_alphanumeric() && !matches!(byte, b'+' | b'-' | b'.')
            }).map_or(0, |index| index + 1);
            if start < colon && input.as_bytes()[start].is_ascii_alphabetic() {
                return Some(colon + 3);
            }
            from = colon + 3;
        }
        None
    }

    fn url_end(input: &str, start: usize) -> usize {
        let mut skip_until = start;
        for (offset, ch) in input[start..].char_indices() {
            let index = start + offset;
            if index < skip_until {
                continue;
            }
            // Our own marker may appear in userinfo or a query on a second
            // pass. Treat it as one token, not an HTML delimiter.
            if input[index..].starts_with(MARKER) {
                skip_until = index + MARKER.len();
                continue;
            }
            if ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '>') {
                return index;
            }
        }
        input.len()
    }

    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(authority) = next_authority(input, cursor) {
        let end = url_end(input, authority);
        let authority_end = input[authority..end].find(['/', '?', '#'])
            .map_or(end, |offset| authority + offset);
        output.push_str(&input[cursor..authority]);
        let rest = if let Some(at) = input[authority..authority_end].rfind('@') {
            output.push_str(MARKER);
            output.push('@');
            authority + at + 1
        } else {
            authority
        };
        if let Some(offset) = input[authority_end..end].find(['?', '#']) {
            let sensitive = authority_end + offset;
            output.push_str(&input[rest..=sensitive]);
            output.push_str(MARKER);
            // Parentheses inside a query are legal: never stop at the first
            // one and expose its suffix. Preserve only surrounding end punctuation.
            let tail = input[sensitive + 1..end].trim_end_matches([')', '}']);
            output.push_str(&input[sensitive + 1 + tail.len()..end]);
        } else {
            output.push_str(&input[rest..end]);
        }
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn encode_component(input: &str, space_as_plus: bool, form_safe_star: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        let unescaped = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_')
            || (form_safe_star && byte == b'*')
            || (!form_safe_star && byte == b'~');
        if unescaped {
            output.push(byte as char);
        } else if byte == b' ' && space_as_plus {
            output.push('+');
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    output
}

/// A same-length view with only percent-escape hex digits uppercased. Keeping
/// offsets unchanged lets matches and truncation boundaries index the original
/// UTF-8/byte input. Follow escaped '%' (%25) into a second encoding layer too;
/// never lowercase ordinary credential characters or decode untrusted bytes.
fn canonical_percent_hex(input: &[u8]) -> Cow<'_, [u8]> {
    if !input.contains(&b'%') {
        return Cow::Borrowed(input);
    }
    let mut output = input.to_vec();
    let mut index = 0;
    while index < output.len() {
        if output[index] != b'%' {
            index += 1;
            continue;
        }
        index += 1;
        loop {
            let end = (index + 2).min(output.len());
            let digits = &mut output[index..end];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_hexdigit) {
                break;
            }
            // A single final hex digit is a truncated escape prefix.
            digits.make_ascii_uppercase();
            let escaped_percent = digits == b"25";
            index = end;
            if !escaped_percent {
                break;
            }
        }
    }
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_raw_form_encoded_percent_encoded_and_double_encoded_secrets() {
        let secret = "sk live/+?=token";
        let redactor = SecretRedactor::new([secret]);
        for rendered in [
            format!("Authorization: Bearer {secret}"),
            "query=sk+live%2F%2B%3F%3Dtoken".to_owned(),
            "query=sk%20live%2f%2b%3f%3dtoken".to_owned(),
            "query=sk%2520live%252F%252B%253F%253Dtoken".to_owned(),
        ] {
            let sanitized = redactor.redact(&rendered);
            assert!(!sanitized.contains("sk live"), "raw secret leaked: {sanitized}");
            assert!(!sanitized.contains("sk+live"), "form secret leaked: {sanitized}");
            assert!(!sanitized.contains("sk%20live"), "encoded secret leaked: {sanitized}");
            assert!(!sanitized.contains("sk%2520live"), "double encoded secret leaked: {sanitized}");
            assert!(sanitized.contains(REDACTED));
        }
    }

    #[test]
    fn redacts_queries_from_quoted_gateway_urls_without_hiding_the_route() {
        let redactor = SecretRedactor::default();
        let sanitized = redactor.redact(
            r#"Post "https://chatgpt.com/backend-api/codex/responses?access_token=secret&x=1": EOF"#,
        );

        assert_eq!(
            sanitized,
            r#"Post "https://chatgpt.com/backend-api/codex/responses?<redacted>": EOF"#
        );
        assert!(!sanitized.contains("access_token"));
        assert!(!sanitized.contains("secret"));

        assert_eq!(
            redact_url_queries("HTTPS://gateway.test/path?token=secret"),
            "HTTPS://gateway.test/path?<redacted>"
        );
        assert_eq!(
            redact_url_queries("http://[::1]/path?token=secret"),
            "http://[::1]/path?<redacted>"
        );
    }

    #[test]
    fn redacts_all_keys_including_nested_variants() {
        let redactor = SecretRedactor::new(["token", "token-long"]);
        assert_eq!(
            redactor.redact("token-long then token"),
            "[REDACTED] then [REDACTED]"
        );
    }

    #[test]
    fn truncation_boundary_removes_only_an_incomplete_secret_suffix() {
        let redactor = SecretRedactor::new(["abcabx"]);
        let partial = b"ordinary diagnostic: abcab";
        assert_eq!(
            redactor.redaction_safe_truncation_boundary(partial),
            partial.len() - 5
        );

        let complete = b"ordinary diagnostic: abcabx";
        assert_eq!(
            redactor.redaction_safe_truncation_boundary(complete),
            complete.len(),
            "a complete secret must remain available to the exact redactor"
        );

        let ordinary = b"ordinary diagnostic tail";
        assert_eq!(
            redactor.redaction_safe_truncation_boundary(ordinary),
            ordinary.len(),
            "an unrelated diagnostic tail must not be removed"
        );
    }

    #[test]
    fn overlapping_credentials_redact_the_union_of_original_matches() {
        let redactor = SecretRedactor::new(["abcd", "bcdef"]);
        assert_eq!(redactor.redact("before abcdef after"), "before [REDACTED] after");
    }

    #[test]
    fn self_overlapping_credentials_do_not_leave_a_suffix() {
        let redactor = SecretRedactor::new(["ababa", "密钥密"]);
        assert_eq!(redactor.redact("abababa 密钥密钥密"), "[REDACTED] [REDACTED]");
    }

    #[test]
    fn replacement_markers_are_not_input_to_later_secret_matches() {
        let redactor = SecretRedactor::new(["long-secret-value", "REDACTED"]);
        assert_eq!(redactor.redact("long-secret-value then REDACTED"), "[REDACTED] then [REDACTED]");
        assert_eq!(SecretRedactor::new(["x"]).redact("xx"), "[REDACTED][REDACTED]");
    }

    #[test]
    fn truncating_an_overlapping_secret_does_not_expose_another_secret_prefix() {
        let redactor = SecretRedactor::new(["abcdef", "defghi"]);
        let input = b"error: abcdef";
        let boundary = redactor.redaction_safe_truncation_boundary(input);
        assert_eq!(&input[..boundary], b"error: ");
    }

    #[test]
    fn overlapping_match_ranges_agree_with_a_small_exhaustive_byte_oracle() {
        fn word(bits: usize, len: usize) -> String {
            (0..len).map(|index| if bits & (1 << index) == 0 { 'a' } else { 'b' }).collect()
        }
        for len in 1..=4 {
            for bits in 0..1 << len {
                let pattern = word(bits, len);
                let second: String = pattern.chars().rev().collect();
                let redactor = SecretRedactor::new([&pattern, &second]);
                for input_len in 0..=7 {
                    for input_bits in 0..1 << input_len {
                        let input = word(input_bits, input_len);
                        let mut expected = vec![false; input.len()];
                        for variant in [&pattern, &second] {
                            for start in 0..input.len() {
                                if input[start..].starts_with(variant.as_str()) {
                                    expected[start..start + variant.len()].fill(true);
                                }
                            }
                        }
                        let mut actual = vec![false; input.len()];
                        for range in redactor.matching_ranges(input.as_bytes()) {
                            actual[range].fill(true);
                        }
                        assert_eq!(actual, expected, "pattern={pattern}, input={input}");
                    }
                }
            }
        }
    }

    #[test]
    fn url_query_parentheses_do_not_expose_gateway_credentials() {
        assert_eq!(redact_url_queries("error (https://host/path?cb=f(x)&token=SECRET) failed"),
            "error (https://host/path?<redacted>) failed");
    }

    #[test]
    fn url_diagnostics_remove_userinfo_fragments_and_non_http_credentials() {
        for (input, expected) in [
            ("https://user:SECRET@host/path", "https://<redacted>@host/path"),
            ("wss://host/path?token=SECRET", "wss://host/path?<redacted>"),
            ("https://host/path#token=SECRET", "https://host/path#<redacted>"),
            ("socks5h://user:SECRET@[::1]:7890", "socks5h://<redacted>@[::1]:7890"),
        ] {
            assert_eq!(redact_url_queries(input), expected);
        }
    }

    #[test]
    fn repeated_url_redaction_is_stable_and_keeps_surrounding_diagnostics() {
        let input = "原因 (HTTPS://user:pass@host/路径?token=secret) and 'wss://[::1]/rt#secret': EOF";
        let expected = "原因 (HTTPS://<redacted>@host/路径?<redacted>) and 'wss://[::1]/rt#<redacted>': EOF";
        assert_eq!(redact_url_queries(input), expected);
        assert_eq!(redact_url_queries(expected), expected);
        assert_eq!(redact_url_queries("https://host/?<redacted>"), "https://host/?<redacted>");
    }

    #[test]
    fn mixed_percent_hex_case_is_redacted_outside_urls() {
        let redactor = SecretRedactor::new(["é/+?"]);
        assert_eq!(redactor.redact("echo=%c3%A9%2f%2B%3f"), "echo=[REDACTED]");
    }

    #[test]
    fn double_encoding_preserves_case_insensitive_inner_escapes() {
        let redactor = SecretRedactor::new(["é/+?"]);
        assert_eq!(redactor.redact("echo=%25c3%25A9%252f%252B%253f"), "echo=[REDACTED]");
    }

    #[test]
    fn truncated_mixed_encoding_does_not_leave_a_credential_prefix() {
        let redactor = SecretRedactor::new(["é/+?"]);
        for encoded in ["é/+?", "%c3%A9%2f%2B%3f", "%25c3%25A9%252f%252B%253f"] {
            let input = format!("echo={encoded}");
            for length in 1..encoded.len() {
                let partial = &input.as_bytes()[..5 + length];
                let boundary = redactor.redaction_safe_truncation_boundary(partial);
                assert_eq!(&partial[..boundary], b"echo=", "encoding={encoded}, length={length}");
            }
        }
    }

    #[test]
    fn percent_canonicalization_does_not_change_ordinary_case_or_unicode() {
        let text = "中文 AbCd %zz";
        assert_eq!(canonical_percent_hex(text.as_bytes()).as_ref(), text.as_bytes());
        assert!(matches!(canonical_percent_hex(b"plain text"), Cow::Borrowed(_)));
        let redactor = SecretRedactor::new(["AbCd"]);
        assert_eq!(redactor.redact("abcd ABCD AbCd 中文"), "abcd ABCD [REDACTED] 中文");
    }

    #[test]
    fn empty_values_do_not_redact_unrelated_text() {
        let redactor = SecretRedactor::new(["", "   "]);
        assert_eq!(redactor.redact("upstream unavailable"), "upstream unavailable");
    }
}
