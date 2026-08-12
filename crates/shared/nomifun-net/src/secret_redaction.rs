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

    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// Replace every exact raw or encoded secret representation.
    pub fn redact(&self, input: &str) -> String {
        let mut output = input.to_owned();
        for variant in self.variants.iter() {
            if output.contains(variant) {
                output = output.replace(variant, REDACTED);
            }
        }
        output
    }
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
    fn redacts_all_keys_and_longest_variant_first() {
        let redactor = SecretRedactor::new(["token", "token-long"]);
        assert_eq!(
            redactor.redact("token-long then token"),
            "[REDACTED] then [REDACTED]"
        );
    }

    #[test]
    fn empty_values_do_not_redact_unrelated_text() {
        let redactor = SecretRedactor::new(["", "   "]);
        assert_eq!(redactor.redact("upstream unavailable"), "upstream unavailable");
    }
}
