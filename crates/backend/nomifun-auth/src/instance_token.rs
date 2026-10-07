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

#[derive(Debug, Default)]
struct TokenState {
    generation: u64,
    token_hash: Option<String>,
}

/// In-memory validator for the single installation-scoped Remote token.
///
/// An empty value closes the Remote front door. Mint/rotate/revoke updates this
/// cache immediately, so request authentication does not need a database read.
/// Guards are held only for one synchronous validation or state mutation.
#[derive(Debug, Default)]
pub struct InstanceTokenValidator {
    state: RwLock<TokenState>,
}

impl InstanceTokenValidator {
    /// Build a validator from the optional persisted token hash.
    pub fn new(initial: Option<String>) -> Self {
        Self {
            state: RwLock::new(TokenState {
                generation: 0,
                token_hash: initial,
            }),
        }
    }

    /// Validate a presented bearer token against the installation token.
    pub fn validate(&self, presented_token: &str) -> bool {
        if presented_token.is_empty() {
            return false;
        }
        let presented_hash = token_sha256_hex(presented_token);
        self.state
            .read()
            .expect("instance token lock poisoned")
            .token_hash
            .as_deref()
            .is_some_and(|stored| ct_eq(&presented_hash, stored))
    }

    /// Mint or rotate the installation token.
    pub fn set_token(&self, token_hash: String) {
        let mut state = self.state.write().expect("instance token lock poisoned");
        state.generation = state.generation.saturating_add(1);
        state.token_hash = Some(token_hash);
    }

    /// Revoke the installation token.
    pub fn clear_token(&self) {
        let mut state = self.state.write().expect("instance token lock poisoned");
        state.generation = state.generation.saturating_add(1);
        state.token_hash = None;
    }

    /// Whether the installation currently has a Remote token configured.
    pub fn is_configured(&self) -> bool {
        self.state
            .read()
            .expect("instance token lock poisoned")
            .token_hash
            .is_some()
    }

    /// Monotonic generation of the currently published token state.
    pub fn generation(&self) -> u64 {
        self.state
            .read()
            .expect("instance token lock poisoned")
            .generation
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
        assert_eq!(validator.generation(), 0);
        assert!(validator.validate(&old));
        assert!(!validator.validate(&new));
        assert!(!validator.validate("wrong"));
        assert!(!validator.validate(""));

        validator.set_token(token_sha256_hex(&new));
        assert_eq!(validator.generation(), 1);
        assert!(!validator.validate(&old));
        assert!(validator.validate(&new));

        validator.clear_token();
        assert_eq!(validator.generation(), 2);
        assert!(!validator.is_configured());
        assert!(!validator.validate(&new));
    }
}
