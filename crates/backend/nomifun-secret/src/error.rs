//! Error types for the secret vault.

/// Errors that can arise from secret registration and resolution.
///
/// Resolution is **fail-closed**: any failure to prove the current origin is
/// authorized yields [`None`] from [`crate::SecretStore::resolve`] rather than a
/// soft error, so a secret value is never returned on an untrusted origin.
/// `SecretError` is reserved for *registration*-time and *crypto*-time faults.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// No secret is registered under the given name.
    #[error("secret not found: no credential registered under that name")]
    NotFound,

    /// The current origin's eTLD+1 is not in the secret's allowed-origins set.
    /// Resolution is denied (fail-closed); the value is never decrypted.
    #[error("origin not allowed: current origin is not bound to this secret (fail-closed)")]
    OriginNotAllowed,

    /// A registered `allowed_origins` entry could not be parsed to an eTLD+1.
    /// Registration is rejected so the binding can never silently match nothing.
    #[error("invalid allowed origin '{0}': cannot derive a registrable domain (eTLD+1)")]
    InvalidAllowedOrigin(String),

    /// AES-GCM encryption/decryption (or key) failure. The message never
    /// contains plaintext or key material.
    #[error("crypto error: {0}")]
    Crypto(String),
}
