//! Installation-scoped access token for the Remote capability front door.
//!
//! NomiFun Desktop has exactly one Remote token. It authenticates the
//! installation owner and never selects or impersonates a companion. The
//! plaintext is minted with [`crate::generate_random_hex_secret`], persisted
//! only as a SHA-256 hash, and may be rotated or revoked at any time.

use std::sync::RwLock;

use sha2::{Digest, Sha256};

/// SHA-256 of `token`, lowercase hex (64 chars).
pub fn token_sha256_hex(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time string compare (both inputs are fixed-length hex hashes here).
fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// In-memory validator for the single installation-scoped Remote token.
///
/// An empty value closes the Remote front door. Mint/rotate/revoke updates this
/// cache immediately, so request authentication does not need a database read.
#[derive(Debug, Default)]
pub struct InstanceTokenValidator {
    token_hash: RwLock<Option<String>>,
}

impl InstanceTokenValidator {
    /// Build a validator from the optional persisted token hash.
    pub fn new(initial: Option<String>) -> Self {
        Self { token_hash: RwLock::new(initial) }
    }

    /// Validate a presented bearer token against the installation token.
    pub fn validate(&self, presented_token: &str) -> bool {
        if presented_token.is_empty() {
            return false;
        }
        let presented_hash = token_sha256_hex(presented_token);
        self.token_hash
            .read()
            .expect("instance token lock poisoned")
            .as_deref()
            .is_some_and(|stored| ct_eq(&presented_hash, stored))
    }

    /// Mint or rotate the installation token.
    pub fn set_token(&self, token_hash: String) {
        *self.token_hash.write().expect("instance token lock poisoned") = Some(token_hash);
    }

    /// Revoke the installation token.
    pub fn clear_token(&self) {
        *self.token_hash.write().expect("instance token lock poisoned") = None;
    }

    /// Whether the installation currently has a Remote token configured.
    pub fn is_configured(&self) -> bool {
        self.token_hash.read().expect("instance token lock poisoned").is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_rotates_and_revokes_the_single_instance_token() {
        let old = crate::generate_random_hex_secret();
        let new = crate::generate_random_hex_secret();
        let validator = InstanceTokenValidator::new(Some(token_sha256_hex(&old)));

        assert!(validator.is_configured());
        assert!(validator.validate(&old));
        assert!(!validator.validate(&new));
        assert!(!validator.validate("wrong"));
        assert!(!validator.validate(""));

        validator.set_token(token_sha256_hex(&new));
        assert!(!validator.validate(&old));
        assert!(validator.validate(&new));

        validator.clear_token();
        assert!(!validator.is_configured());
        assert!(!validator.validate(&new));
    }
}
