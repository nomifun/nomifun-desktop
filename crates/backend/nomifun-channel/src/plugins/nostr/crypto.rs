//! Nostr cryptographic helpers — thin wrappers over the `nostr` crate for
//! key parsing (nsec/hex), pubkey derivation, NIP-04 encrypt/decrypt, event
//! building and signing.

use nostr::nips::nip04;
use nostr::{EventBuilder, FromBech32, Keys, Kind, PublicKey, SecretKey, Tag, ToBech32};

use crate::error::ChannelError;

// ---------------------------------------------------------------------------
// Key parsing
// ---------------------------------------------------------------------------

/// Parse a private key from either nsec bech32 or 64-char hex.
pub fn parse_secret_key(input: &str) -> Result<SecretKey, ChannelError> {
    let trimmed = input.trim();
    if trimmed.starts_with("nsec1") {
        SecretKey::from_bech32(trimmed)
            .map_err(|e| ChannelError::InvalidConfig(format!("invalid nsec key: {e}")))
    } else {
        SecretKey::parse(trimmed)
            .map_err(|e| ChannelError::InvalidConfig(format!("invalid hex private key: {e}")))
    }
}

/// Encode a public key as npub bech32.
pub fn pubkey_to_npub(pk: &PublicKey) -> String {
    pk.to_bech32().unwrap_or_else(|_| pk.to_hex())
}

/// Parse a public key from hex (64 chars).
pub fn parse_pubkey_hex(hex: &str) -> Result<PublicKey, ChannelError> {
    PublicKey::parse(hex.trim())
        .map_err(|e| ChannelError::InvalidConfig(format!("invalid pubkey hex: {e}")))
}

// ---------------------------------------------------------------------------
// NIP-04 encrypt / decrypt
// ---------------------------------------------------------------------------

/// NIP-04 encrypt plaintext for a recipient.
pub fn nip04_encrypt(
    sender_sk: &SecretKey,
    recipient_pk: &PublicKey,
    plaintext: &str,
) -> Result<String, ChannelError> {
    nip04::encrypt(sender_sk, recipient_pk, plaintext)
        .map_err(|e| ChannelError::MessageSendFailed(format!("NIP-04 encrypt failed: {e}")))
}

/// NIP-04 decrypt ciphertext from a sender.
pub fn nip04_decrypt(
    receiver_sk: &SecretKey,
    sender_pk: &PublicKey,
    ciphertext: &str,
) -> Result<String, ChannelError> {
    nip04::decrypt(receiver_sk, sender_pk, ciphertext)
        .map_err(|e| ChannelError::MessageSendFailed(format!("NIP-04 decrypt failed: {e}")))
}

// ---------------------------------------------------------------------------
// Event construction & signing
// ---------------------------------------------------------------------------

/// Build a signed kind-4 (NIP-04 DM) event.
///
/// Returns `(event_id_hex, event_json)` ready to publish via `["EVENT", ...]`.
pub fn build_dm_event(
    keys: &Keys,
    recipient_pk: &PublicKey,
    plaintext: &str,
) -> Result<(String, String), ChannelError> {
    let encrypted = nip04_encrypt(keys.secret_key(), recipient_pk, plaintext)?;

    let event = EventBuilder::new(Kind::EncryptedDirectMessage, &encrypted)
        .tag(Tag::public_key(*recipient_pk))
        .sign_with_keys(keys)
        .map_err(|e| ChannelError::MessageSendFailed(format!("event signing failed: {e}")))?;

    let id = event.id.to_hex();
    let json = serde_json::to_string(&event)
        .map_err(|e| ChannelError::MessageSendFailed(format!("event serialization failed: {e}")))?;

    Ok((id, json))
}

// ---------------------------------------------------------------------------
// Relay list parsing
// ---------------------------------------------------------------------------

/// Parse a comma-separated relay URL list. Returns at least one URL.
pub fn parse_relay_urls(input: Option<&str>) -> Vec<String> {
    let default_relays = "wss://relay.damus.io,wss://nos.lol";
    let raw = input.unwrap_or(default_relays);
    let urls: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if urls.is_empty() {
        default_relays
            .split(',')
            .map(|s| s.to_owned())
            .collect()
    } else {
        urls
    }
}

// ---------------------------------------------------------------------------
// Helpers for inbound event processing
// ---------------------------------------------------------------------------

/// Check if an event has a `["p", <pubkey_hex>]` tag matching the given pubkey.
pub fn has_p_tag(tags_json: &[serde_json::Value], target_pk_hex: &str) -> bool {
    tags_json.iter().any(|tag| {
        if let Some(arr) = tag.as_array() {
            arr.len() >= 2
                && arr[0].as_str() == Some("p")
                && arr[1].as_str() == Some(target_pk_hex)
        } else {
            false
        }
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Known test vector: the nostr crate's own test key pair.
    const SENDER_SK_HEX: &str =
        "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";
    const RECEIVER_SK_HEX: &str =
        "7b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e";

    #[test]
    fn parse_hex_private_key() {
        let sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        // Derive pubkey — should not panic.
        let _pk = Keys::new(sk.clone()).public_key();
    }

    #[test]
    fn parse_nsec_private_key() {
        // Convert hex to nsec first, then parse back.
        let sk = SecretKey::parse(SENDER_SK_HEX).unwrap();
        let nsec = sk.to_bech32().unwrap();
        assert!(nsec.starts_with("nsec1"));
        let parsed = parse_secret_key(&nsec).unwrap();
        assert_eq!(parsed, sk);
    }

    #[test]
    fn parse_invalid_key_fails() {
        assert!(parse_secret_key("not_a_key").is_err());
        assert!(parse_secret_key("nsec1invalid").is_err());
        assert!(parse_secret_key("").is_err());
    }

    #[test]
    fn npub_encoding() {
        let sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        let pk = Keys::new(sk.clone()).public_key();
        let npub = pubkey_to_npub(&pk);
        assert!(npub.starts_with("npub1"));
    }

    #[test]
    fn nip04_roundtrip() {
        let sender_sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        let receiver_sk = parse_secret_key(RECEIVER_SK_HEX).unwrap();
        let sender_pk = Keys::new(sender_sk.clone()).public_key();
        let receiver_pk = Keys::new(receiver_sk.clone()).public_key();

        let plaintext = "Hello, Nostr!";
        let ciphertext = nip04_encrypt(&sender_sk, &receiver_pk, plaintext).unwrap();
        let decrypted = nip04_decrypt(&receiver_sk, &sender_pk, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn nip04_known_ciphertext() {
        // Known test vector from the nostr crate.
        let sender_sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        let receiver_sk = parse_secret_key(RECEIVER_SK_HEX).unwrap();
        let sender_pk = Keys::new(sender_sk.clone()).public_key();

        let ciphertext = "dJc+WbBgaFCD2/kfg1XCWJParplBDxnZIdJGZ6FCTOg=?iv=M6VxRPkMZu7aIdD+10xPuw==";
        let plaintext = nip04_decrypt(&receiver_sk, &sender_pk, ciphertext).unwrap();
        assert_eq!(plaintext, "Saturn, bringer of old age");
    }

    #[test]
    fn build_dm_event_produces_valid_json() {
        let sender_sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        let receiver_sk = parse_secret_key(RECEIVER_SK_HEX).unwrap();
        let sender_keys = Keys::new(sender_sk);
        let receiver_pk = Keys::new(receiver_sk.clone()).public_key();

        let (id, json) = build_dm_event(&sender_keys, &receiver_pk, "test message").unwrap();
        assert!(!id.is_empty());
        assert_eq!(id.len(), 64); // sha256 hex

        // Parse and verify.
        let event: nostr::Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event.kind, Kind::EncryptedDirectMessage);
        assert_eq!(event.id.to_hex(), id);

        // Decrypt the content.
        let decrypted = nip04_decrypt(
            &receiver_sk,
            &sender_keys.public_key(),
            event.content.as_str(),
        )
        .unwrap();
        assert_eq!(decrypted, "test message");
    }

    #[test]
    fn parse_relay_urls_defaults() {
        let urls = parse_relay_urls(None);
        assert_eq!(urls, vec!["wss://relay.damus.io", "wss://nos.lol"]);
    }

    #[test]
    fn parse_relay_urls_custom() {
        let urls = parse_relay_urls(Some("wss://relay1.com, wss://relay2.com"));
        assert_eq!(urls, vec!["wss://relay1.com", "wss://relay2.com"]);
    }

    #[test]
    fn parse_relay_urls_empty_falls_back_to_defaults() {
        let urls = parse_relay_urls(Some(""));
        assert_eq!(urls, vec!["wss://relay.damus.io", "wss://nos.lol"]);
    }

    #[test]
    fn has_p_tag_match() {
        let tags = vec![serde_json::json!(["p", "abc123"])];
        assert!(has_p_tag(&tags, "abc123"));
        assert!(!has_p_tag(&tags, "xyz789"));
    }

    #[test]
    fn has_p_tag_no_match_empty() {
        let tags: Vec<serde_json::Value> = vec![];
        assert!(!has_p_tag(&tags, "abc123"));
    }

    #[test]
    fn parse_pubkey_hex_valid() {
        let sk = parse_secret_key(SENDER_SK_HEX).unwrap();
        let pk = Keys::new(sk.clone()).public_key();
        let hex = pk.to_hex();
        let parsed = parse_pubkey_hex(&hex).unwrap();
        assert_eq!(parsed, pk);
    }

    #[test]
    fn parse_pubkey_hex_invalid() {
        assert!(parse_pubkey_hex("not_a_hex_pubkey").is_err());
    }
}
