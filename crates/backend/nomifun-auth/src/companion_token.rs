//! Per-companion access tokens for the Remote capability front door (`/mcp`).
//!
//! Each external connection binds to exactly one companion. Tokens are minted
//! with [`crate::generate_random_hex_secret`], persisted only as a SHA-256 hash,
//! and revocable. This module holds the hashing primitive and an in-memory
//! validator mapping `token → companion_id`, hot-swapped on mint/revoke.

use std::collections::HashMap;
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

/// Resolves a presented Remote token to the companion it is bound to.
///
/// Holds `companion_id → token_hash` in memory, hot-swapped via
/// [`insert_token`](Self::insert_token) / [`remove_token`](Self::remove_token)
/// on mint/revoke, so no DB round-trip is needed per request. An empty map means
/// no companion has a token → every `resolve` returns `None` (the front door is
/// closed until a token is minted).
#[derive(Debug, Default)]
pub struct CompanionTokenValidator {
    /// companion_id -> token_hash
    tokens: RwLock<HashMap<String, String>>,
}

impl CompanionTokenValidator {
    /// Build a validator seeded with persisted `(companion_id, token_hash)` pairs.
    pub fn new(initial: Vec<(String, String)>) -> Self {
        Self { tokens: RwLock::new(initial.into_iter().collect()) }
    }

    /// Resolve a presented token to its bound `companion_id`, or `None` if it
    /// matches no companion. Constant-time per-entry compare.
    pub fn resolve(&self, presented_token: &str) -> Option<String> {
        if presented_token.is_empty() {
            return None;
        }
        let presented_hash = token_sha256_hex(presented_token);
        let map = self.tokens.read().expect("companion token lock poisoned");
        for (companion_id, stored) in map.iter() {
            if ct_eq(&presented_hash, stored) {
                return Some(companion_id.clone());
            }
        }
        None
    }

    /// Mint/rotate the token for a companion (replaces any prior token).
    pub fn insert_token(&self, companion_id: String, token_hash: String) {
        self.tokens.write().expect("companion token lock poisoned").insert(companion_id, token_hash);
    }

    /// Revoke a companion's token.
    pub fn remove_token(&self, companion_id: &str) {
        self.tokens.write().expect("companion token lock poisoned").remove(companion_id);
    }

    /// Whether a companion currently has a token configured (status endpoint).
    pub fn is_configured_for(&self, companion_id: &str) -> bool {
        self.tokens.read().expect("companion token lock poisoned").contains_key(companion_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_minted_token_to_its_companion() {
        let token_a = crate::generate_random_hex_secret();
        let token_b = crate::generate_random_hex_secret();
        let v = CompanionTokenValidator::new(vec![("comp-a".into(), token_sha256_hex(&token_a))]);
        assert!(v.is_configured_for("comp-a"));
        assert_eq!(v.resolve(&token_a).as_deref(), Some("comp-a"));
        assert_eq!(v.resolve(&token_b), None);
        assert_eq!(v.resolve("wrong"), None);
        assert_eq!(v.resolve(""), None);

        // Mint for a second companion.
        v.insert_token("comp-b".into(), token_sha256_hex(&token_b));
        assert_eq!(v.resolve(&token_b).as_deref(), Some("comp-b"));

        // Revocation closes that companion's door only.
        v.remove_token("comp-a");
        assert!(!v.is_configured_for("comp-a"));
        assert_eq!(v.resolve(&token_a), None);
        assert_eq!(v.resolve(&token_b).as_deref(), Some("comp-b"));
    }

    #[test]
    fn rotation_replaces_prior_token_for_same_companion() {
        let old = crate::generate_random_hex_secret();
        let new = crate::generate_random_hex_secret();
        let v = CompanionTokenValidator::default();
        v.insert_token("comp".into(), token_sha256_hex(&old));
        v.insert_token("comp".into(), token_sha256_hex(&new));
        assert_eq!(v.resolve(&old), None);
        assert_eq!(v.resolve(&new).as_deref(), Some("comp"));
    }
}
