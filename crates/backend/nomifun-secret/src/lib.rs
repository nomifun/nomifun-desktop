//! `nomifun-secret` — credential vault for the native browser-use engine.
//!
//! Provides a [`SecretStore`] that:
//!
//! * **encrypts** each registered value at rest with AES-256-GCM, reusing the
//!   shared implementation in [`nomifun_common`] (`encrypt_string` /
//!   `decrypt_string`) per P2 design ruling ⑦ — no second crypto stack;
//! * **binds** each secret to a set of allowed origins by their **eTLD+1**
//!   (registrable domain), computed offline from the compile-time Public Suffix
//!   List via the [`psl`] crate (DESIGN §4 / §16);
//! * **fail-closed resolves**: [`SecretStore::resolve`] decrypts and returns the
//!   value *only* when the current origin's eTLD+1 is among the secret's allowed
//!   eTLD+1s. Any failure to prove authorization (unknown name, unbound origin,
//!   unparseable host) yields [`None`]. This holds regardless of session mode:
//!   the gate is a property of the store, not of an tool-execution approval that
//!   yolo/companion could bypass.
//!
//! The returned [`SecretValue`] redacts itself in `Debug`/`Display`; plaintext
//! is reachable only via [`SecretValue::expose`] (for `Input.insertText`
//! injection) — the value must never reach the LLM, logs, or the ref table.
//!
//! ## Key provisioning
//!
//! The AES-256-GCM key is a 32-byte secret supplied by the caller via
//! [`SecretStore::new`], matching the codebase convention of threading
//! `encryption_key: [u8; 32]` (the machine-bound `encryption_key` file
//! provisioned at the app data-dir layer). This crate does **not** invent its
//! own machine-binding scheme. [`SecretStore::ephemeral`] generates a random
//! per-process key for tests and headless throwaway use.

mod domain;
mod error;
mod value;
mod vault;

// X2 `web` feature: per-pet secret CRUD service + axum routes (mounted by
// nomifun-app). Gated so pure-logic consumers don't pull axum / auth / api-types.
#[cfg(feature = "web")]
mod routes;
#[cfg(feature = "web")]
pub mod service;
#[cfg(feature = "web")]
mod state;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use domain::{etld_plus_one, host_of, same_etld_plus_one};
pub use error::SecretError;
pub use value::SecretValue;
pub use vault::{
    SHARED_SECRET_DIR, SecretVaultFile, load_secret_store, pet_vault_path, save_secret_store,
    secret_vault_path, shared_vault_path,
};

#[cfg(feature = "web")]
pub use routes::secret_routes;
#[cfg(feature = "web")]
pub use service::SecretService;
#[cfg(feature = "web")]
pub use state::SecretRouterState;

/// Size of the AES-256-GCM key, in bytes.
pub const KEY_SIZE: usize = 32;

/// An encrypted secret bound to a set of registrable domains (eTLD+1).
///
/// **The `ciphertext` is already AES-256-GCM-encrypted** (the value never lives in
/// the clear in a `SecretRecord`). The record is `Serialize`/`Deserialize` so the
/// store can be **persisted as-is** (X2 vault: a JSON file of records, where every
/// `value` is already ciphertext — the on-disk file therefore never contains
/// plaintext, and we do **not** add a second crypto layer over the per-record AES).
/// `allowed_etld1` + `name` are not secret (they are policy, not credentials).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretRecord {
    /// base64(nonce || ciphertext || tag), produced by `encrypt_string`.
    ciphertext: String,
    /// Allowed origins reduced to their eTLD+1 (already normalized/lowercased).
    allowed_etld1: Vec<String>,
}

/// A secret's **non-sensitive metadata** for listing — its name and the set of
/// registrable domains (eTLD+1) it is bound to. **Never carries the value**
/// (X2 红线：列表绝不回 value / 不过 LLM；value 仅经 [`SecretStore::resolve`] →
/// `Input.insertText` 注入）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretListing {
    /// The secret's name (the `secret:NAME` reference key).
    pub name: String,
    /// The registrable domains (eTLD+1) this secret is bound to.
    pub allowed_etld1: Vec<String>,
}

/// AES-GCM-encrypted, origin-bound credential store.
///
/// Holds `name -> (ciphertext, allowed eTLD+1 set)`. The encryption key never
/// leaves the store; only [`resolve`](Self::resolve) decrypts, and only after
/// the origin gate passes.
pub struct SecretStore {
    key: [u8; KEY_SIZE],
    secrets: HashMap<String, SecretRecord>,
}

impl SecretStore {
    /// Create a store backed by a caller-supplied 32-byte AES-256-GCM key.
    ///
    /// The key should be the app's machine-bound `encryption_key` (the same one
    /// threaded as `[u8; 32]` elsewhere in the backend).
    pub fn new(key: [u8; KEY_SIZE]) -> Self {
        SecretStore {
            key,
            secrets: HashMap::new(),
        }
    }

    /// Create a store with a random, per-process ephemeral key.
    ///
    /// Suitable for tests and headless throwaway sessions where persistence
    /// across restarts is not required. Returns a [`SecretError::Crypto`] if the
    /// system RNG fails.
    pub fn ephemeral() -> Result<Self, SecretError> {
        let mut key = [0u8; KEY_SIZE];
        getrandom::getrandom(&mut key).map_err(|e| SecretError::Crypto(format!("RNG failure: {e}")))?;
        Ok(SecretStore::new(key))
    }

    /// Register (or overwrite) a secret bound to `allowed_origins`.
    ///
    /// Each `allowed_origins` entry may be a bare host (`x.com`) or a full
    /// origin (`https://x.com:443`); it is reduced to its eTLD+1. An entry with
    /// no derivable eTLD+1 (a bare public suffix, an IP, `localhost`, …) is
    /// rejected with [`SecretError::InvalidAllowedOrigin`] so a binding can
    /// never silently match nothing. An empty `allowed_origins` is likewise
    /// rejected — an unbound secret could never resolve and is almost certainly
    /// a caller error.
    pub fn register(&mut self, name: &str, value: &str, allowed_origins: Vec<String>) -> Result<(), SecretError> {
        if allowed_origins.is_empty() {
            return Err(SecretError::InvalidAllowedOrigin(String::new()));
        }

        let mut allowed_etld1 = Vec::with_capacity(allowed_origins.len());
        for origin in &allowed_origins {
            let e1 = etld_plus_one(origin).ok_or_else(|| SecretError::InvalidAllowedOrigin(origin.clone()))?;
            if !allowed_etld1.contains(&e1) {
                allowed_etld1.push(e1);
            }
        }

        let ciphertext =
            nomifun_common::encrypt_string(value, &self.key).map_err(|e| SecretError::Crypto(e.to_string()))?;

        self.secrets
            .insert(name.to_string(), SecretRecord { ciphertext, allowed_etld1 });
        Ok(())
    }

    /// Resolve a secret for the **current origin**, fail-closed.
    ///
    /// Returns `Some(SecretValue)` only when:
    /// 1. a secret is registered under `name`, **and**
    /// 2. `current_origin`'s eTLD+1 is among the secret's allowed eTLD+1s, **and**
    /// 3. decryption succeeds.
    ///
    /// Any other case (unknown name, unbound origin, unparseable host, crypto
    /// failure) returns `None`. The comparison ignores scheme/port/path and is
    /// case-insensitive on the host (`host_of` normalization).
    ///
    /// Returning `None` rather than a `Result` keeps the gate fail-closed by
    /// type: there is no error path that could be mishandled into exposing a
    /// value on an untrusted origin.
    pub fn resolve(&self, name: &str, current_origin: &str) -> Option<SecretValue> {
        let record = self.secrets.get(name)?;

        let current_e1 = etld_plus_one(current_origin)?;
        if !record.allowed_etld1.iter().any(|allowed| allowed == &current_e1) {
            return None;
        }

        match nomifun_common::decrypt_string(&record.ciphertext, &self.key) {
            Ok(plaintext) => Some(SecretValue::new(plaintext)),
            Err(_) => None,
        }
    }

    /// Number of registered secrets.
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    /// True when no secrets are registered.
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Remove a secret. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.secrets.remove(name).is_some()
    }

    /// List every secret's **non-sensitive metadata** (name + bound eTLD+1s),
    /// sorted by name for a stable UI order. **Never exposes the value** (the
    /// listing carries only policy, not credentials) — this is the type that
    /// backs the `list_secrets` endpoint.
    pub fn list(&self) -> Vec<SecretListing> {
        let mut out: Vec<SecretListing> = self
            .secrets
            .iter()
            .map(|(name, rec)| SecretListing {
                name: name.clone(),
                allowed_etld1: rec.allowed_etld1.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// The union of every registered secret's allowed eTLD+1s (deduped, sorted).
    ///
    /// **裁决⑤ 共用真值**：this is the data source for `FirewallConfig.allow_etld1`
    /// — the same per-pet `allowed_origins` that gate `secret:NAME` resolution also
    /// gate the browser's egress domain allowlist (one config, two uses). An empty
    /// store yields an empty vec → the firewall's domain allowlist stays empty
    /// (= unrestricted egress, current behavior) until the user registers a secret.
    pub fn allowed_etld1_union(&self) -> Vec<String> {
        let mut set: Vec<String> = Vec::new();
        for rec in self.secrets.values() {
            for e1 in &rec.allowed_etld1 {
                if !set.contains(e1) {
                    set.push(e1.clone());
                }
            }
        }
        set.sort();
        set
    }

    /// Export the store's records (name → already-encrypted record) for
    /// persistence. The values stay ciphertext throughout — this never decrypts.
    pub(crate) fn to_records(&self) -> HashMap<String, SecretRecord> {
        self.secrets.clone()
    }

    /// Rebuild a store from persisted records under `key`. The records carry
    /// ciphertext encrypted under the **same machine-bound key**; we do not
    /// re-encrypt. A `resolve` later will GCM-authenticate against `key` (a
    /// wrong key → `None`, fail-closed).
    pub(crate) fn from_records(key: [u8; KEY_SIZE], secrets: HashMap<String, SecretRecord>) -> Self {
        SecretStore { key, secrets }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SecretStore {
        SecretStore::new([0x42; KEY_SIZE])
    }

    // ---- registration ----

    #[test]
    fn register_rejects_empty_allowed_origins() {
        let mut s = store();
        assert!(matches!(
            s.register("pw", "v", vec![]),
            Err(SecretError::InvalidAllowedOrigin(_))
        ));
    }

    #[test]
    fn register_rejects_bare_public_suffix() {
        let mut s = store();
        assert!(matches!(
            s.register("pw", "v", vec!["co.uk".into()]),
            Err(SecretError::InvalidAllowedOrigin(_))
        ));
    }

    #[test]
    fn register_accepts_origin_with_scheme_and_port() {
        let mut s = store();
        assert!(s.register("pw", "v", vec!["https://x.com:443".into()]).is_ok());
    }

    // ---- resolve origin gate (fail-closed) ----

    #[test]
    fn resolve_allows_subdomain_of_allowed_origin() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["x.com".into()]).unwrap();

        let got = s.resolve("pw", "https://login.x.com");
        assert_eq!(got.map(|v| v.into_inner()).as_deref(), Some("secret-val"));
    }

    #[test]
    fn resolve_allows_deep_subdomain_with_port_and_path() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["x.com".into()]).unwrap();
        assert!(s.resolve("pw", "https://sub.login.x.com:8443/account?next=1").is_some());
    }

    #[test]
    fn resolve_denies_cross_domain_fail_closed() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["x.com".into()]).unwrap();
        assert!(s.resolve("pw", "https://evil.com").is_none());
        assert!(s.resolve("pw", "https://x.com.evil.com").is_none());
    }

    #[test]
    fn resolve_denies_distinct_co_uk_registrable() {
        // a.co.uk and b.co.uk are different registrable domains (co.uk is a
        // public suffix). Binding to a.co.uk must NOT leak to b.co.uk.
        let mut s = store();
        s.register("pw", "secret-val", vec!["a.co.uk".into()]).unwrap();
        assert!(s.resolve("pw", "https://www.a.co.uk").is_some());
        assert!(s.resolve("pw", "https://b.co.uk").is_none());
    }

    #[test]
    fn resolve_unknown_name_is_none() {
        let s = store();
        assert!(s.resolve("missing", "https://x.com").is_none());
    }

    #[test]
    fn resolve_unparseable_origin_is_none() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["x.com".into()]).unwrap();
        assert!(s.resolve("pw", "").is_none());
        assert!(s.resolve("pw", "co.uk").is_none()); // bare suffix → None
    }

    #[test]
    fn resolve_ignores_scheme_and_case() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["X.COM".into()]).unwrap();
        assert!(s.resolve("pw", "HTTP://LOGIN.X.COM").is_some());
    }

    #[test]
    fn resolve_multiple_allowed_origins() {
        let mut s = store();
        s.register("pw", "secret-val", vec!["x.com".into(), "y.org".into()])
            .unwrap();
        assert!(s.resolve("pw", "https://a.x.com").is_some());
        assert!(s.resolve("pw", "https://b.y.org").is_some());
        assert!(s.resolve("pw", "https://z.com").is_none());
    }

    // ---- AES round-trip & ciphertext != plaintext ----

    #[test]
    fn aes_round_trip_recovers_value() {
        let mut s = store();
        s.register("pw", "the-real-password", vec!["x.com".into()]).unwrap();
        let v = s.resolve("pw", "https://x.com").unwrap();
        assert_eq!(v.expose(), "the-real-password");
    }

    #[test]
    fn ciphertext_differs_from_plaintext() {
        let mut s = store();
        let plain = "the-real-password";
        s.register("pw", plain, vec!["x.com".into()]).unwrap();
        let ct = &s.secrets.get("pw").unwrap().ciphertext;
        assert_ne!(ct, plain);
        assert!(!ct.contains(plain), "ciphertext must not contain plaintext");
    }

    #[test]
    fn wrong_key_decryption_fails_closed_to_none() {
        // Encrypt under one key, attempt resolve under a store with another key:
        // GCM auth fails → resolve returns None (never a corrupt value).
        let mut s1 = SecretStore::new([0x01; KEY_SIZE]);
        s1.register("pw", "v", vec!["x.com".into()]).unwrap();
        let record = s1.secrets.remove("pw").unwrap();

        let mut s2 = SecretStore::new([0x02; KEY_SIZE]);
        s2.secrets.insert("pw".into(), record);
        assert!(s2.resolve("pw", "https://x.com").is_none());
    }

    // ---- bookkeeping ----

    #[test]
    fn len_and_remove() {
        let mut s = store();
        assert!(s.is_empty());
        s.register("a", "1", vec!["x.com".into()]).unwrap();
        s.register("b", "2", vec!["y.com".into()]).unwrap();
        assert_eq!(s.len(), 2);
        assert!(s.remove("a"));
        assert!(!s.remove("a"));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn register_overwrites_same_name() {
        let mut s = store();
        s.register("pw", "old", vec!["x.com".into()]).unwrap();
        s.register("pw", "new", vec!["x.com".into()]).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.resolve("pw", "https://x.com").unwrap().expose(), "new");
    }

    #[test]
    fn ephemeral_store_works() {
        let mut s = SecretStore::ephemeral().unwrap();
        s.register("pw", "v", vec!["x.com".into()]).unwrap();
        assert_eq!(s.resolve("pw", "https://x.com").unwrap().expose(), "v");
    }

    // ---- list (metadata only, NEVER the value) ----

    #[test]
    fn list_returns_name_and_origins_never_value() {
        let mut s = store();
        s.register("github", "ghp_supersecret", vec!["github.com".into()]).unwrap();
        s.register("bank", "hunter2", vec!["chase.com".into(), "https://www.chase.com".into()])
            .unwrap();

        let listed = s.list();
        // Sorted by name → bank, github.
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "bank");
        assert_eq!(listed[0].allowed_etld1, vec!["chase.com".to_string()]); // both inputs → one eTLD+1
        assert_eq!(listed[1].name, "github");
        assert_eq!(listed[1].allowed_etld1, vec!["github.com".to_string()]);

        // **安全断言**：the listing type carries NO value field and its serialized
        // form must never contain any plaintext value.
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains("ghp_supersecret"), "list must NOT leak value: {json}");
        assert!(!json.contains("hunter2"), "list must NOT leak value: {json}");
    }

    #[test]
    fn list_empty_store_is_empty() {
        assert!(store().list().is_empty());
    }

    // ---- allowed_etld1_union (裁决⑤ 共用真值: secret allowed_origins → firewall allow_etld1) ----

    #[test]
    fn allowed_etld1_union_dedups_and_sorts() {
        let mut s = store();
        s.register("a", "1", vec!["x.com".into(), "https://sub.y.org".into()]).unwrap();
        s.register("b", "2", vec!["y.org".into(), "z.net".into()]).unwrap(); // y.org overlaps a's
        let union = s.allowed_etld1_union();
        // Deduped (y.org once) + sorted.
        assert_eq!(union, vec!["x.com".to_string(), "y.org".to_string(), "z.net".to_string()]);
    }

    #[test]
    fn allowed_etld1_union_empty_store_is_empty() {
        // Empty store → empty allowlist → firewall stays unrestricted (zero regression).
        assert!(store().allowed_etld1_union().is_empty());
    }

    // ---- to_records / from_records round-trip (vault persistence reuses ciphertext, no double crypto) ----

    #[test]
    fn records_round_trip_preserves_resolve_and_keeps_ciphertext() {
        let mut s = SecretStore::new([0x42; KEY_SIZE]);
        s.register("pw", "the-real-password", vec!["x.com".into()]).unwrap();
        let records = s.to_records();
        // Records carry ciphertext, NOT plaintext.
        let recs_json = serde_json::to_string(&records).unwrap();
        assert!(!recs_json.contains("the-real-password"), "records must hold ciphertext only");

        // Rebuild under the SAME key → resolve still works (no re-encryption).
        let rebuilt = SecretStore::from_records([0x42; KEY_SIZE], records.clone());
        assert_eq!(rebuilt.resolve("pw", "https://login.x.com").unwrap().expose(), "the-real-password");

        // Rebuild under a DIFFERENT key → GCM auth fails → resolve None (fail-closed).
        let wrong = SecretStore::from_records([0x99; KEY_SIZE], records);
        assert!(wrong.resolve("pw", "https://x.com").is_none(), "wrong key must fail-closed");
    }

    // ---- nonce randomness (inherited from nomifun-common, asserted here) ----

    #[test]
    fn same_value_yields_different_ciphertext() {
        let mut s = store();
        s.register("a", "same", vec!["x.com".into()]).unwrap();
        s.register("b", "same", vec!["x.com".into()]).unwrap();
        let ca = &s.secrets.get("a").unwrap().ciphertext;
        let cb = &s.secrets.get("b").unwrap().ciphertext;
        assert_ne!(ca, cb, "random nonce must produce distinct ciphertexts");
    }
}
