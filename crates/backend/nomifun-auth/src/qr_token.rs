use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;

use nomifun_common::AppError;

/// QR token time-to-live: 5 minutes.
const QR_TOKEN_TTL_MS: i64 = 5 * 60 * 1000;

/// Random token length in bytes (produces 64-char hex string).
const QR_TOKEN_BYTES: usize = 32;

/// Internal data for a QR login token.
struct QrTokenData {
    created_at_ms: i64,
    used: bool,
}

/// In-memory QR login token store with automatic expiration.
///
/// Tokens are one-time-use and expire after 5 minutes.
/// Thread-safe via `DashMap`.
pub struct QrTokenStore {
    tokens: DashMap<String, QrTokenData>,
}

impl Default for QrTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl QrTokenStore {
    pub fn new() -> Self {
        Self { tokens: DashMap::new() }
    }

    /// Generate a new QR login token and store it.
    ///
    /// Returns the 64-character hex token string.
    pub fn generate(&self) -> String {
        self.generate_with_expiry().0
    }

    /// Generate a new QR login token and return it along with its expiry timestamp (ms).
    ///
    /// Returns `(token, expires_at_ms)` where `expires_at_ms` is the absolute
    /// Unix time in milliseconds when the token becomes invalid.
    pub fn generate_with_expiry(&self) -> (String, i64) {
        let mut buf = [0u8; QR_TOKEN_BYTES];
        getrandom::getrandom(&mut buf).expect("OS entropy source unavailable");

        let mut token = String::with_capacity(QR_TOKEN_BYTES * 2);
        for byte in buf {
            let _ = write!(token, "{byte:02x}");
        }

        let created_at_ms = nomifun_common::now_ms();
        self.tokens.insert(
            token.clone(),
            QrTokenData {
                created_at_ms,
                used: false,
            },
        );

        (token, created_at_ms + QR_TOKEN_TTL_MS)
    }

    /// Validate and consume a QR token (one-time use).
    ///
    /// Checks existence, expiry (5 min), and used status atomically.
    pub fn validate_and_consume(&self, token: &str) -> Result<(), AppError> {
        let mut entry = self
            .tokens
            .get_mut(token)
            .ok_or_else(|| AppError::Unauthorized("Invalid QR token".into()))?;

        if entry.used {
            return Err(AppError::Unauthorized("QR token already used".into()));
        }

        let now = nomifun_common::now_ms();
        let elapsed_ms = now.saturating_sub(entry.created_at_ms);
        if elapsed_ms > QR_TOKEN_TTL_MS {
            return Err(AppError::Unauthorized("QR token expired".into()));
        }

        entry.used = true;
        Ok(())
    }

    /// Remove expired tokens to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = nomifun_common::now_ms();
        self.tokens
            .retain(|_, data| now.saturating_sub(data.created_at_ms) <= QR_TOKEN_TTL_MS);
    }

    /// Start a background task that cleans up expired tokens periodically.
    pub fn start_cleanup_task(self: &Arc<Self>, interval: Duration) {
        let store = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                store.cleanup();
            }
        });
    }

    /// Number of stored tokens (for monitoring/testing).
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_64_hex_chars() {
        let store = QrTokenStore::new();
        let token = store.generate();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_produces_unique_tokens() {
        let store = QrTokenStore::new();
        let t1 = store.generate();
        let t2 = store.generate();
        assert_ne!(t1, t2);
    }

    #[test]
    fn generate_increments_count() {
        let store = QrTokenStore::new();
        assert_eq!(store.token_count(), 0);
        store.generate();
        assert_eq!(store.token_count(), 1);
        store.generate();
        assert_eq!(store.token_count(), 2);
    }

    #[test]
    fn validate_and_consume_valid_token() {
        let store = QrTokenStore::new();
        let token = store.generate();
        assert!(store.validate_and_consume(&token).is_ok());
    }

    #[test]
    fn validate_nonexistent_token_fails() {
        let store = QrTokenStore::new();
        let err = store.validate_and_consume("nonexistent").unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn validate_already_used_token_fails() {
        let store = QrTokenStore::new();
        let token = store.generate();
        store.validate_and_consume(&token).unwrap();

        let err = store.validate_and_consume(&token).unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn validate_expired_token_fails() {
        let store = QrTokenStore::new();
        // Manually insert an expired token
        store.tokens.insert(
            "expired_token".to_owned(),
            QrTokenData {
                created_at_ms: 1000, // very old
                used: false,
            },
        );

        let err = store.validate_and_consume("expired_token").unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[test]
    fn cleanup_removes_expired_tokens() {
        let store = QrTokenStore::new();
        // Insert an expired token
        store.tokens.insert(
            "old".to_owned(),
            QrTokenData {
                created_at_ms: 1000,
                used: false,
            },
        );
        // Insert a fresh token
        store.generate();
        assert_eq!(store.token_count(), 2);

        store.cleanup();
        assert_eq!(store.token_count(), 1);
    }

    #[test]
    fn cleanup_keeps_fresh_tokens() {
        let store = QrTokenStore::new();
        store.generate();
        store.cleanup();
        assert_eq!(store.token_count(), 1);
    }
}
