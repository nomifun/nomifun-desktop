//! Credential-aware redaction for untrusted upstream diagnostics.
//!
//! Providers sometimes echo authentication material in error bodies. Callers
//! build one redactor from the exact runtime credentials and apply it before
//! classifying, logging, returning, or persisting the diagnostic. Raw and URL
//! form/percent-encoded representations are covered without relying on secret
//! prefixes such as `sk-` or `AKIA`.

use std::sync::Arc;

const REDACTED: &str = "[REDACTED]";

/// An immutable set of exact secret representations.
///
/// Deliberately does not implement `Debug`: its replacement table contains
/// live credential material.
#[derive(Clone, Default)]
pub struct SecretRedactor {
    variants: Arc<Vec<String>>,
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
            push_variant(&mut variants, secret.to_owned());
            push_variant(&mut variants, trimmed.to_owned());

            // Query encoders use two common space representations: HTML-form
            // encoding uses `+`, while RFC 3986 component encoding uses
            // `%20`. `form_urlencoded::byte_serialize` only produces the
            // former, so relying on it alone misses the URLs most gateways
            // echo in diagnostics.
            let form_encoded = encode_component(trimmed, true, true);
            let percent_encoded = encode_component(trimmed, false, false);
            for encoded in [&form_encoded, &percent_encoded] {
                push_encoded_variants(&mut variants, encoded);

                // A gateway may quote an already encoded query value in JSON
                // and encode it once more. Cover both form and RFC spellings.
                let double_form = encode_component(encoded, true, true);
                let double_percent = encode_component(encoded, false, false);
                push_encoded_variants(&mut variants, &double_form);
                push_encoded_variants(&mut variants, &double_percent);
            }
        }
        variants.sort_unstable();
        variants.dedup();
        variants.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        Self {
            variants: Arc::new(variants),
        }
    }

    /// Whether no exact credential representations were configured. Even when
    /// this is true, [`Self::redact`] still removes query strings from embedded
    /// HTTP(S) URLs because an upstream may mint credentials unknown to us.
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
        let unsafe_suffix = self
            .variants
            .iter()
            .map(|variant| longest_proper_prefix_suffix(input, variant.as_bytes()))
            .max()
            .unwrap_or(0);
        input.len().saturating_sub(unsafe_suffix)
    }

    /// Replace every exact raw or encoded secret representation.
    pub fn redact(&self, input: &str) -> String {
        let mut output = input.to_owned();
        for variant in self.variants.iter() {
            if output.contains(variant) {
                output = output.replace(variant, REDACTED);
            }
        }
        redact_url_queries(&output)
    }
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
    let mut failure = vec![0usize; relevant.len()];
    for index in 1..relevant.len() {
        let mut matched = failure[index - 1];
        while matched > 0 && relevant[index] != relevant[matched] {
            matched = failure[matched - 1];
        }
        if relevant[index] == relevant[matched] {
            matched += 1;
        }
        failure[index] = matched;
    }

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

/// Remove query strings from every HTTP(S) URL embedded in an untrusted
/// diagnostic. Upstream gateways frequently quote their own outbound URL in
/// an error body (for example `Post "https://host/path?token=...": EOF`).
/// Exact credential redaction cannot know credentials minted by that gateway,
/// so query values are always treated as sensitive before the body is logged,
/// persisted, or returned to a caller.
pub fn redact_url_queries(input: &str) -> String {
    fn next_url_start(input: &str, from: usize) -> Option<usize> {
        let bytes = input.as_bytes();
        (from..bytes.len()).find(|&index| {
            bytes[index..]
                .get(..7)
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(b"http://"))
                || bytes[index..]
                    .get(..8)
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(b"https://"))
        })
    }

    fn url_end(input: &str, start: usize) -> usize {
        input[start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                (ch.is_whitespace()
                    || matches!(ch, '"' | '\'' | '<' | '>' | ')' | '}'))
                .then_some(start + offset)
            })
            .unwrap_or(input.len())
    }

    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(start) = next_url_start(input, cursor) {
        let end = url_end(input, start);
        let Some(query_offset) = input[start..end].find('?') else {
            output.push_str(&input[cursor..end]);
            cursor = end;
            continue;
        };
        let query = start + query_offset;
        output.push_str(&input[cursor..=query]);
        output.push_str("<redacted>");
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

fn push_encoded_variants(target: &mut Vec<String>, value: &str) {
    if value.is_empty() {
        return;
    }
    push_variant(target, value.to_owned());
    push_variant(target, percent_hex_case(value, true));
    push_variant(target, percent_hex_case(value, false));
}

fn push_variant(target: &mut Vec<String>, value: String) {
    if !value.is_empty() && !target.iter().any(|existing| existing == &value) {
        target.push(value);
    }
}

fn percent_hex_case(value: &str, uppercase: bool) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            output.push('%');
            for byte in &bytes[index + 1..=index + 2] {
                output.push(if uppercase {
                    (*byte as char).to_ascii_uppercase()
                } else {
                    (*byte as char).to_ascii_lowercase()
                });
            }
            index += 3;
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
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
    fn redacts_all_keys_and_longest_variant_first() {
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
    fn empty_values_do_not_redact_unrelated_text() {
        let redactor = SecretRedactor::new(["", "   "]);
        assert_eq!(redactor.redact("upstream unavailable"), "upstream unavailable");
    }
}
