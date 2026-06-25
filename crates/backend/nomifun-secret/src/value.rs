//! Decrypted secret value with redacted formatting.

use std::fmt;

/// A decrypted secret value.
///
/// **Redacted by construction**: both [`Debug`] and [`Display`] render
/// `<redacted>` and never the plaintext, mirroring `TypeInput::Secret` in the
/// browser engine. The plaintext is reachable *only* via [`SecretValue::expose`]
/// — callers that expose it (e.g. `Input.insertText` injection) are responsible
/// for ensuring it never reaches the LLM, logs, or the ref table (DESIGN §16).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a plaintext value. Prefer obtaining values through
    /// [`crate::SecretStore::resolve`], which enforces the origin gate.
    pub fn new(plaintext: impl Into<String>) -> Self {
        SecretValue(plaintext.into())
    }

    /// Return the plaintext. The only path to the secret material.
    ///
    /// The caller assumes the obligation to keep it off the LLM / logs / refs.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Consume and return the owned plaintext.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: &str = "hunter2-super-secret-password";

    #[test]
    fn expose_returns_plaintext() {
        let v = SecretValue::new(PLAIN);
        assert_eq!(v.expose(), PLAIN);
        assert_eq!(v.into_inner(), PLAIN);
    }

    #[test]
    fn debug_is_redacted() {
        let v = SecretValue::new(PLAIN);
        let s = format!("{v:?}");
        assert!(!s.contains(PLAIN), "Debug leaked plaintext: {s}");
        assert!(s.contains("<redacted>"));
    }

    #[test]
    fn display_is_redacted() {
        let v = SecretValue::new(PLAIN);
        let s = format!("{v}");
        assert!(!s.contains(PLAIN), "Display leaked plaintext: {s}");
        assert_eq!(s, "<redacted>");
    }

    #[test]
    fn debug_redacted_even_when_nested() {
        // e.g. format!("{:?}", Some(secret)) or in a struct must not leak.
        let v = Some(SecretValue::new(PLAIN));
        let s = format!("{v:?}");
        assert!(!s.contains(PLAIN), "nested Debug leaked plaintext: {s}");
    }
}
