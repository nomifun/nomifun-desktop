use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::error::AuthError;

/// bcrypt cost factor (higher = slower but more secure).
const BCRYPT_COST: u32 = 12;

/// Minimum time for password verification to prevent timing attacks.
const MIN_VERIFY_DURATION: Duration = Duration::from_millis(50);

// Character sets for credential generation
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SPECIAL: &[u8] = b"!@#$%^&*";
const ALPHANUMERIC_LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const ALL_PASSWORD_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*";

/// Pre-computed dummy hash for timing attack prevention.
static DUMMY_HASH: OnceLock<String> = OnceLock::new();

/// Hash a password using bcrypt with cost factor 12.
///
/// **Note**: This is a CPU-intensive blocking operation. In async contexts,
/// wrap in `tokio::task::spawn_blocking`.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    bcrypt::hash(password, BCRYPT_COST).map_err(|e| AuthError::HashError(e.to_string()))
}

/// Verify a password against a bcrypt hash.
///
/// Returns `true` if the password matches, `false` otherwise.
/// bcrypt internally uses constant-time comparison.
///
/// **Note**: This is a CPU-intensive blocking operation.
pub fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
    bcrypt::verify(password, hash).map_err(|e| AuthError::HashError(e.to_string()))
}

/// Verify a password with a guaranteed minimum execution time of 50ms.
///
/// Runs bcrypt verification on a blocking thread pool and pads the response
/// time to at least 50ms. This prevents timing attacks that could distinguish
/// "user exists + wrong password" from "user doesn't exist".
pub async fn verify_password_timed(password: &str, hash: &str) -> Result<bool, AuthError> {
    let start = Instant::now();
    let password = password.to_owned();
    let hash = hash.to_owned();

    let result = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(|e| AuthError::HashError(format!("Task join error: {e}")))?;

    let elapsed = start.elapsed();
    if elapsed < MIN_VERIFY_DURATION {
        tokio::time::sleep(MIN_VERIFY_DURATION - elapsed).await;
    }

    result
}

/// Get a pre-computed bcrypt hash for timing attack prevention.
///
/// When a login attempt references a non-existent user, verify the supplied
/// password against this dummy hash to consume the same amount of time as
/// a real verification.
pub fn dummy_password_hash() -> &'static str {
    DUMMY_HASH.get_or_init(|| {
        // bcrypt hash of a fixed dummy input. This cannot fail for valid input;
        // if it does, the bcrypt implementation is fundamentally broken.
        bcrypt::hash("__nomifun_dummy_password__", BCRYPT_COST).expect("bcrypt hash of constant input must succeed")
    })
}

/// Generate random user credentials for auto-bootstrap scenarios.
///
/// Returns `(username, password)` where:
/// - username: 6-8 lowercase alphanumeric characters
/// - password: 12-17 mixed characters (upper, lower, digits, special)
pub fn generate_user_credentials() -> (String, String) {
    let username_len = random_range(6, 9);
    let password_len = random_range(12, 18);

    let username = random_string(username_len, ALPHANUMERIC_LOWER);
    let password = generate_strong_password(password_len);

    (username, password)
}

/// Generate a strong random password suitable for WebUI admin reset.
///
/// Guarantees ≥1 character from each category (upper, lower, digit, special)
/// and fills remaining slots from a mixed charset.
pub fn generate_password(len: usize) -> String {
    // Enforce minimum length of 4 to satisfy the four-category guarantee.
    generate_strong_password(len.max(4))
}

// --- Internal helpers ---

/// Fill a buffer with cryptographically random bytes.
///
/// Panics if the OS entropy source is unavailable. This mirrors the behavior
/// of `uuid::Uuid::now_v7()` used in `nomifun-common`.
fn fill_random(buf: &mut [u8]) {
    getrandom::getrandom(buf).expect("OS entropy source unavailable");
}

/// Generate a random integer in `[min, max_exclusive)`.
fn random_range(min: usize, max_exclusive: usize) -> usize {
    let range = max_exclusive - min;
    let mut buf = [0u8; 4];
    fill_random(&mut buf);
    min + (u32::from_le_bytes(buf) as usize) % range
}

/// Pick a random byte from the given charset.
fn random_from(charset: &[u8]) -> u8 {
    let mut buf = [0u8; 1];
    fill_random(&mut buf);
    charset[buf[0] as usize % charset.len()]
}

/// Generate a random string of the given length from the charset.
fn random_string(len: usize, charset: &[u8]) -> String {
    let mut buf = vec![0u8; len];
    fill_random(&mut buf);
    buf.iter()
        .map(|b| charset[*b as usize % charset.len()] as char)
        .collect()
}

/// Generate a strong password with guaranteed character variety.
fn generate_strong_password(len: usize) -> String {
    let mut chars = Vec::with_capacity(len);

    // Guarantee at least one character from each category
    chars.push(random_from(UPPER));
    chars.push(random_from(LOWER));
    chars.push(random_from(DIGITS));
    chars.push(random_from(SPECIAL));

    // Fill remaining positions from the full charset
    for _ in 4..len {
        chars.push(random_from(ALL_PASSWORD_CHARS));
    }

    // Fisher-Yates shuffle
    let mut shuffle_bytes = vec![0u8; chars.len()];
    fill_random(&mut shuffle_bytes);
    for i in (1..chars.len()).rev() {
        let j = shuffle_bytes[i] as usize % (i + 1);
        chars.swap(i, j);
    }

    chars.iter().map(|&b| b as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::{validate_password, validate_username};

    #[test]
    fn hash_and_verify_correct_password() {
        let hash = hash_password("my_secure_password").unwrap();
        assert!(verify_password("my_secure_password", &hash).unwrap());
    }

    #[test]
    fn verify_wrong_password() {
        let hash = hash_password("correct_password").unwrap();
        assert!(!verify_password("wrong_password", &hash).unwrap());
    }

    #[test]
    fn hash_produces_bcrypt_format() {
        let hash = hash_password("test_password").unwrap();
        assert!(hash.starts_with("$2b$12$"));
    }

    #[test]
    fn dummy_hash_is_valid_bcrypt() {
        let hash = dummy_password_hash();
        assert!(hash.starts_with("$2b$12$"));
        assert!(!verify_password("random", hash).unwrap());
    }

    #[test]
    fn dummy_hash_matches_dummy_input() {
        let hash = dummy_password_hash();
        assert!(verify_password("__nomifun_dummy_password__", hash).unwrap());
    }

    #[test]
    fn generate_credentials_produces_valid_username() {
        for _ in 0..10 {
            let (username, _) = generate_user_credentials();
            assert!(
                username.len() >= 6 && username.len() <= 8,
                "username length out of range: {}",
                username.len()
            );
            assert!(
                validate_username(&username).is_ok(),
                "generated username failed validation: {username}"
            );
        }
    }

    #[test]
    fn generate_credentials_produces_valid_password() {
        for _ in 0..10 {
            let (_, password) = generate_user_credentials();
            assert!(
                password.len() >= 12 && password.len() <= 17,
                "password length out of range: {}",
                password.len()
            );
            assert!(
                validate_password(&password).is_ok(),
                "generated password failed validation: {password}"
            );
        }
    }

    #[test]
    fn generate_credentials_has_character_variety() {
        for _ in 0..10 {
            let (_, password) = generate_user_credentials();
            let has_upper = password.bytes().any(|b| b.is_ascii_uppercase());
            let has_lower = password.bytes().any(|b| b.is_ascii_lowercase());
            let has_digit = password.bytes().any(|b| b.is_ascii_digit());
            let has_special = password.bytes().any(|b| SPECIAL.contains(&b));
            assert!(has_upper, "password missing uppercase: {password}");
            assert!(has_lower, "password missing lowercase: {password}");
            assert!(has_digit, "password missing digit: {password}");
            assert!(has_special, "password missing special char: {password}");
        }
    }

    #[tokio::test]
    async fn verify_timed_correct_password() {
        let hash = hash_password("test_password").unwrap();
        let start = Instant::now();
        let result = verify_password_timed("test_password", &hash).await;
        let elapsed = start.elapsed();
        assert!(result.unwrap());
        assert!(
            elapsed >= MIN_VERIFY_DURATION,
            "verification took {elapsed:?}, expected >= {MIN_VERIFY_DURATION:?}"
        );
    }

    #[tokio::test]
    async fn verify_timed_wrong_password() {
        let hash = hash_password("correct").unwrap();
        let start = Instant::now();
        let result = verify_password_timed("wrong", &hash).await;
        let elapsed = start.elapsed();
        assert!(!result.unwrap());
        assert!(elapsed >= MIN_VERIFY_DURATION);
    }
}
