use axum::http::HeaderMap;
use nomifun_common::required_idempotency_key;
use sha2::{Digest, Sha256};

pub(crate) const CONVERSATION_SEND_TOOL: &str =
    "nomi_send_to_conversation";

/// Bind a caller-selected replay token to the authenticated principal and
/// capability. The external key is not trusted or globally unique by itself;
/// the resulting bounded token is safe to pass into the conversation receipt
/// boundary, where target owner/conversation and payload are checked again.
pub(crate) fn remote_operation_id(
    headers: &HeaderMap,
    principal_id: &str,
    tool_name: &str,
) -> Result<String, &'static str> {
    fn hash_field(hasher: &mut Sha256, value: &str) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }

    let client_key = required_idempotency_key(headers)?;
    let mut hasher = Sha256::new();
    hasher.update(b"nomifun-remote-tool:v1\0");
    hash_field(&mut hasher, principal_id);
    hash_field(&mut hasher, tool_name);
    hash_field(&mut hasher, client_key);
    Ok(format!("remote-tool-v1-{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn remote_key_is_stable_and_authenticated_principal_scoped() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_static("caller-retry-7"),
        );
        let first = remote_operation_id(
            &headers,
            "companion-a",
            CONVERSATION_SEND_TOOL,
        )
        .unwrap();
        let retry = remote_operation_id(
            &headers,
            "companion-a",
            CONVERSATION_SEND_TOOL,
        )
        .unwrap();
        let other_principal = remote_operation_id(
            &headers,
            "companion-b",
            CONVERSATION_SEND_TOOL,
        )
        .unwrap();

        assert_eq!(first, retry);
        assert_ne!(first, other_principal);
        assert!(first.len() <= nomifun_common::MAX_IDEMPOTENCY_KEY_LEN);
        assert!(
            first
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte))
        );
    }
}
