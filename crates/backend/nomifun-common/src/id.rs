use crate::timestamp::now_ms;
use uuid::Uuid;

/// Lowercase Crockford-style base32 alphabet (drops `i`/`l`/`o`/`u` so the id
/// stays unambiguous when read aloud or copied by hand), kept in ascending
/// ASCII order. Because the alphabet itself is sorted, a fixed-width
/// big-endian encoding sorts lexicographically by the integer it encodes —
/// which is what lets the short id stand in for the retired `seq` ordering.
const SHORT_ID_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// Bits of millisecond timestamp retained (45 bits stays monotonic well past
/// year 3000) and the base32 char count that encodes them (45 / 5).
const TIME_BITS: u32 = 45;
const TIME_CHARS: usize = 9;
/// Random bits appended after the timestamp, and their base32 char count
/// (35 / 5). 35 bits per millisecond keeps cross-device collisions negligible.
const RAND_BITS: u32 = 35;
const RAND_CHARS: usize = 7;

/// Encode the low `chars * 5` bits of `value` as `chars` base32 characters,
/// most-significant character first (big-endian), so the resulting text sorts
/// in the same order as `value`.
fn encode_base32(mut value: u64, chars: usize) -> String {
    let mut buf = vec![0u8; chars];
    for slot in buf.iter_mut().rev() {
        *slot = SHORT_ID_ALPHABET[(value & 0b1_1111) as usize];
        value >>= 5;
    }
    // Every byte came from SHORT_ID_ALPHABET, which is ASCII.
    String::from_utf8(buf).expect("base32 alphabet is valid ASCII")
}

/// Generate a full UUID v7 string (36 chars).
///
/// For non-entity randomness only: auth tokens / credentials and the random
/// suffix of composite idempotency keys. Entity IDs must use
/// [`generate_prefixed_id`] instead.
pub fn generate_id() -> String {
    Uuid::now_v7().to_string()
}

/// Generate a prefixed entity ID: `{prefix}_{short}` (e.g. `conv_0fh3k…`,
/// `msg_0fh3k…`).
///
/// The 16-char body is a sortable short id — a 45-bit millisecond timestamp
/// (9 base32 chars) followed by 35 random bits (7 base32 chars). It is
/// lexicographically time-ordered (so it carries the ordering the retired
/// `seq` track used to provide) and globally unique for safe cross-device
/// exit, but roughly half the length of the former UUIDv7 tail so that
/// "display == primary key" stays human-readable wherever an id is shown.
///
/// The single minting convention for every entity ID across the backend —
/// see the prefix table in
/// `docs/superpowers/specs/2026-06-11-entity-seq-design.md`. Its frontend
/// mirror is `prefixedId` in `ui/src/common/utils/prefixedId.ts`; the two
/// implementations MUST stay bit-for-bit aligned so ids minted on either side
/// interleave and sort identically.
pub fn generate_prefixed_id(prefix: &str) -> String {
    let ms = (now_ms() as u64) & ((1u64 << TIME_BITS) - 1);
    let mut rand_bytes = [0u8; 8];
    getrandom::getrandom(&mut rand_bytes).expect("OS entropy source unavailable");
    let rand = u64::from_le_bytes(rand_bytes) & ((1u64 << RAND_BITS) - 1);
    format!("{prefix}_{}{}", encode_base32(ms, TIME_CHARS), encode_base32(rand, RAND_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_generate_id_is_valid_uuid() {
        let id = generate_id();
        assert!(Uuid::parse_str(&id).is_ok());
    }

    #[test]
    fn test_generate_id_is_v7() {
        let id = generate_id();
        let uuid = Uuid::parse_str(&id).unwrap();
        assert_eq!(uuid.get_version_num(), 7);
    }

    #[test]
    fn test_generate_prefixed_id_format() {
        let id = generate_prefixed_id("msg");
        assert!(id.starts_with("msg_"));
        let body = &id["msg_".len()..];
        assert_eq!(body.len(), TIME_CHARS + RAND_CHARS);
        assert!(
            body.bytes().all(|b| SHORT_ID_ALPHABET.contains(&b)),
            "body {body} must use only the short-id alphabet"
        );
    }

    #[test]
    fn test_prefixed_id_uniqueness() {
        let ids: HashSet<String> = (0..10_000).map(|_| generate_prefixed_id("x")).collect();
        assert_eq!(ids.len(), 10_000);
    }

    #[test]
    fn test_generate_id_uniqueness() {
        let ids: HashSet<String> = (0..1000).map(|_| generate_id()).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn test_prefixed_id_time_ordering() {
        // The timestamp is the high-order, big-endian prefix of the body, so a
        // later mint sorts after an earlier one once the millisecond differs.
        let earlier = generate_prefixed_id("c");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let later = generate_prefixed_id("c");
        assert!(later > earlier, "{later} should sort after {earlier}");
    }

    #[test]
    fn test_generate_id_time_ordering() {
        let id1 = generate_id();
        let id2 = generate_id();
        assert!(id2 >= id1);
    }

    #[test]
    fn test_long_prefix() {
        let prefix = "a".repeat(1000);
        let id = generate_prefixed_id(&prefix);
        assert!(id.starts_with(&prefix));
    }
}
