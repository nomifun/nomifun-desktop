use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::error::AppError;

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;

/// Encrypt a string value using AES-256-GCM.
///
/// The key must be exactly 32 bytes. Output is base64-encoded (nonce + ciphertext + tag).
pub fn encrypt_string(plaintext: &str, key: &[u8]) -> Result<String, AppError> {
    validate_key_size(key)?;

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| AppError::Internal(format!("Failed to create cipher: {e}")))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| AppError::Internal(format!("RNG failure: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| AppError::Internal(format!("Encryption failed: {e}")))?;

    let mut combined = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    Ok(BASE64.encode(combined))
}

/// Decrypt an AES-256-GCM encrypted string.
///
/// The key must be exactly 32 bytes. Input is base64-encoded (nonce + ciphertext + tag).
pub fn decrypt_string(ciphertext: &str, key: &[u8]) -> Result<String, AppError> {
    validate_key_size(key)?;

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| AppError::Internal(format!("Failed to create cipher: {e}")))?;

    let combined = BASE64
        .decode(ciphertext)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64: {e}")))?;

    if combined.len() < NONCE_SIZE {
        return Err(AppError::BadRequest("Ciphertext too short".into()));
    }

    let (nonce_bytes, encrypted) = combined.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, encrypted)
        .map_err(|_| AppError::BadRequest("Decryption failed: invalid key or corrupted data".into()))?;

    String::from_utf8(plaintext).map_err(|e| AppError::Internal(format!("Invalid UTF-8 in decrypted data: {e}")))
}

fn validate_key_size(key: &[u8]) -> Result<(), AppError> {
    if key.len() != KEY_SIZE {
        return Err(AppError::BadRequest(format!(
            "AES-256 key must be exactly {KEY_SIZE} bytes, got {}",
            key.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x42; 32]
    }

    #[test]
    fn test_roundtrip() {
        let key = test_key();
        let encrypted = encrypt_string("hello", &key).unwrap();
        let decrypted = decrypt_string(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "hello");
    }

    #[test]
    fn test_empty_string() {
        let key = test_key();
        let encrypted = encrypt_string("", &key).unwrap();
        let decrypted = decrypt_string(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_unicode() {
        let key = test_key();
        let encrypted = encrypt_string("你好世界", &key).unwrap();
        let decrypted = decrypt_string(&encrypted, &key).unwrap();
        assert_eq!(decrypted, "你好世界");
    }

    #[test]
    fn test_wrong_key_fails() {
        let key = test_key();
        let encrypted = encrypt_string("hello", &key).unwrap();
        let wrong_key = [0x99; 32];
        assert!(decrypt_string(&encrypted, &wrong_key).is_err());
    }

    #[test]
    fn test_nonce_randomness() {
        let key = test_key();
        let enc1 = encrypt_string("hello", &key).unwrap();
        let enc2 = encrypt_string("hello", &key).unwrap();
        assert_ne!(enc1, enc2);
    }

    #[test]
    fn test_invalid_key_size() {
        let short_key = [0u8; 16];
        assert!(encrypt_string("hello", &short_key).is_err());
        assert!(decrypt_string("dGVzdA==", &short_key).is_err());
    }

    #[test]
    fn test_invalid_base64() {
        let key = test_key();
        assert!(decrypt_string("not-valid-base64!!!", &key).is_err());
    }

    #[test]
    fn test_ciphertext_too_short() {
        let key = test_key();
        // Base64 of less than 12 bytes
        let short = BASE64.encode([0u8; 5]);
        assert!(decrypt_string(&short, &key).is_err());
    }
}
