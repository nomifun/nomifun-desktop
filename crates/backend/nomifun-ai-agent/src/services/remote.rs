use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::SigningKey;
use nomifun_api_types::{
    CreateRemoteAgentRequest, HandshakeResponse, RemoteAgentListItem, RemoteAgentResponse,
    TestRemoteAgentConnectionRequest, UpdateRemoteAgentRequest,
};
use nomifun_common::{
    AppError, RemoteAgentAuthType, RemoteAgentProtocol, RemoteAgentStatus, decrypt_string, encrypt_string,
};
use nomifun_db::models::RemoteAgentRow;
use nomifun_db::{IRemoteAgentRepository, UpdateRemoteAgentParams};
use tokio_tungstenite::tungstenite;
use tracing::warn;

/// Service layer for Remote Agent CRUD and connection management.
#[derive(Clone)]
pub struct RemoteAgentService {
    repo: Arc<dyn IRemoteAgentRepository>,
    encryption_key: [u8; 32],
}

impl RemoteAgentService {
    pub fn new(repo: Arc<dyn IRemoteAgentRepository>, encryption_key: [u8; 32]) -> Self {
        Self { repo, encryption_key }
    }

    /// List all remote agents (auth_token omitted).
    pub async fn list(&self) -> Result<Vec<RemoteAgentListItem>, AppError> {
        let rows = self.repo.list().await.map_err(db_err)?;
        rows.into_iter().map(|r| self.row_to_list_item(r)).collect()
    }

    /// Get a single remote agent by ID (auth_token masked).
    pub async fn get(&self, id: &str) -> Result<RemoteAgentResponse, AppError> {
        let id = parse_id(id)?;
        let row = self
            .repo
            .find_by_id(id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| AppError::NotFound(format!("Remote agent '{id}' not found")))?;
        self.row_to_response(row)
    }

    /// Create a new remote agent. OpenClaw protocol auto-generates Ed25519 keys.
    pub async fn create(&self, req: CreateRemoteAgentRequest) -> Result<RemoteAgentResponse, AppError> {
        validate_create_request(&req)?;

        let encrypted_token = req
            .auth_token
            .as_deref()
            .map(|t| encrypt_string(t, &self.encryption_key))
            .transpose()?;

        let (device_id, device_public_key, device_private_key) = if req.protocol == RemoteAgentProtocol::OpenClaw {
            let (id, pub_key, priv_key) = generate_device_keypair(&self.encryption_key)?;
            (Some(id), Some(pub_key), Some(priv_key))
        } else {
            (None, None, None)
        };

        let row = self
            .repo
            .create(nomifun_db::CreateRemoteAgentParams {
                name: &req.name,
                protocol: &enum_to_str(&req.protocol),
                url: &req.url,
                auth_type: &enum_to_str(&req.auth_type),
                auth_token: encrypted_token.as_deref(),
                allow_insecure: req.allow_insecure,
                avatar: req.avatar.as_deref(),
                description: req.description.as_deref(),
                device_id: device_id.as_deref(),
                device_public_key: device_public_key.as_deref(),
                device_private_key: device_private_key.as_deref(),
                device_token: None,
            })
            .await
            .map_err(db_err)?;

        self.row_to_response(row)
    }

    /// Update an existing remote agent.
    pub async fn update(&self, id: &str, req: UpdateRemoteAgentRequest) -> Result<RemoteAgentResponse, AppError> {
        let id = parse_id(id)?;
        let encrypted_token = match &req.auth_token {
            Some(Some(t)) => Some(Some(encrypt_string(t, &self.encryption_key)?)),
            Some(None) => Some(None),
            None => None,
        };

        let protocol_str = req.protocol.map(|p| enum_to_str(&p));
        let auth_type_str = req.auth_type.map(|a| enum_to_str(&a));

        let params = UpdateRemoteAgentParams {
            name: req.name.as_deref(),
            protocol: protocol_str.as_deref(),
            url: req.url.as_deref(),
            auth_type: auth_type_str.as_deref(),
            auth_token: encrypted_token.as_ref().map(|o| o.as_deref()),
            allow_insecure: req.allow_insecure,
            avatar: req.avatar.as_ref().map(|o| o.as_deref()),
            description: req.description.as_ref().map(|o| o.as_deref()),
        };

        let row = self.repo.update(id, params).await.map_err(|e| match e {
            nomifun_db::DbError::NotFound(msg) => AppError::NotFound(msg),
            other => AppError::Internal(other.to_string()),
        })?;

        self.row_to_response(row)
    }

    /// Delete a remote agent.
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        let id = parse_id(id)?;
        self.repo.delete(id).await.map_err(|e| match e {
            nomifun_db::DbError::NotFound(msg) => AppError::NotFound(msg),
            other => AppError::Internal(other.to_string()),
        })
    }

    /// Test a WebSocket connection to a remote agent URL (10s timeout, SSRF protected).
    pub async fn test_connection(&self, req: TestRemoteAgentConnectionRequest) -> Result<(), AppError> {
        validate_ws_url(&req.url)?;

        let url = req.url.clone();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::task::spawn_blocking(move || {
                tungstenite::connect(&url)
                    .map(|_| ())
                    .map_err(|e| AppError::BadGateway(format!("WebSocket connection failed: {e}")))
            })
            .await
            .map_err(|e| AppError::Internal(format!("Join error: {e}")))?
        })
        .await;

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AppError::Timeout("Connection timed out after 10 seconds".into())),
        }
    }

    /// OpenClaw device handshake (15s timeout).
    pub async fn handshake(&self, id: &str) -> Result<HandshakeResponse, AppError> {
        let id = parse_id(id)?;
        let row = self
            .repo
            .find_by_id(id)
            .await
            .map_err(db_err)?
            .ok_or_else(|| AppError::NotFound(format!("Remote agent '{id}' not found")))?;

        let protocol = parse_protocol(&row.protocol);
        if protocol != RemoteAgentProtocol::OpenClaw {
            return Err(AppError::BadRequest(
                "Handshake is only supported for OpenClaw protocol".into(),
            ));
        }

        validate_ws_url(&row.url)?;

        let url = row.url.clone();
        let connect_result = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::task::spawn_blocking(move || {
                tungstenite::connect(&url)
                    .map(|_| ())
                    .map_err(|e| AppError::BadGateway(format!("Handshake connection failed: {e}")))
            })
            .await
            .map_err(|e| AppError::Internal(format!("Join error: {e}")))?
        })
        .await;

        match connect_result {
            Ok(Ok(_)) => {
                let now = nomifun_common::now_ms();
                let _ = self.repo.update_status(id, "connected", Some(now)).await;
                Ok(HandshakeResponse {
                    status: "ok".to_string(),
                })
            }
            Ok(Err(e)) => {
                let _ = self.repo.update_status(id, "error", None).await;
                Err(e)
            }
            Err(_) => {
                let _ = self.repo.update_status(id, "error", None).await;
                Err(AppError::Timeout("Handshake timed out after 15 seconds".into()))
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────

    fn row_to_list_item(&self, row: RemoteAgentRow) -> Result<RemoteAgentListItem, AppError> {
        Ok(RemoteAgentListItem {
            id: row.id,
            name: row.name,
            protocol: parse_protocol(&row.protocol),
            url: row.url,
            auth_type: parse_auth_type(&row.auth_type),
            allow_insecure: row.allow_insecure,
            avatar: row.avatar,
            description: row.description,
            status: parse_status(&row.status),
            last_connected_at: row.last_connected_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    fn row_to_response(&self, row: RemoteAgentRow) -> Result<RemoteAgentResponse, AppError> {
        let masked_token =
            row.auth_token
                .as_deref()
                .map(|encrypted| match decrypt_string(encrypted, &self.encryption_key) {
                    Ok(plain) => mask_token(&plain),
                    Err(e) => {
                        warn!("Failed to decrypt auth_token for agent {}: {e}", row.id);
                        "***".to_string()
                    }
                });

        let device_public_key = row
            .device_public_key
            .as_deref()
            .map(|encrypted| decrypt_string(encrypted, &self.encryption_key).unwrap_or_else(|_| "***".to_string()));

        Ok(RemoteAgentResponse {
            id: row.id,
            name: row.name,
            protocol: parse_protocol(&row.protocol),
            url: row.url,
            auth_type: parse_auth_type(&row.auth_type),
            auth_token: masked_token,
            allow_insecure: row.allow_insecure,
            avatar: row.avatar,
            description: row.description,
            device_id: row.device_id,
            device_public_key,
            status: parse_status(&row.status),
            last_connected_at: row.last_connected_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

// ── Validation ──────────────────────────────────────────────────

fn validate_create_request(req: &CreateRemoteAgentRequest) -> Result<(), AppError> {
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }
    if req.url.trim().is_empty() {
        return Err(AppError::BadRequest("url must not be empty".into()));
    }
    validate_ws_url(&req.url)
}

fn validate_ws_url(url: &str) -> Result<(), AppError> {
    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        return Err(AppError::BadRequest("URL must use ws:// or wss:// protocol".into()));
    }
    Ok(())
}

// ── Token masking ───────────────────────────────────────────────

fn mask_token(token: &str) -> String {
    if token.len() <= 4 {
        "***".to_string()
    } else {
        format!("***{}", &token[token.len() - 4..])
    }
}

// ── Ed25519 key generation ──────────────────────────────────────

fn generate_device_keypair(encryption_key: &[u8; 32]) -> Result<(String, String, String), AppError> {
    let mut rng_bytes = [0u8; 32];
    getrandom::getrandom(&mut rng_bytes).map_err(|e| AppError::Internal(format!("RNG failure: {e}")))?;

    let signing_key = SigningKey::from_bytes(&rng_bytes);
    let verifying_key = signing_key.verifying_key();

    let device_id = nomifun_common::generate_prefixed_id("dev");

    // Encode keys as base64 before encrypting
    let pub_b64 = BASE64.encode(verifying_key.as_bytes());
    let priv_b64 = BASE64.encode(signing_key.to_bytes());

    let encrypted_pub = encrypt_string(&pub_b64, encryption_key)?;
    let encrypted_priv = encrypt_string(&priv_b64, encryption_key)?;

    Ok((device_id, encrypted_pub, encrypted_priv))
}

// ── Enum serialization helpers ──────────────────────────────────

fn enum_to_str<T: serde::Serialize>(val: &T) -> String {
    serde_json::to_string(val)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

fn enum_from_str<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_str(&format!("\"{s}\"")).ok()
}

fn parse_protocol(s: &str) -> RemoteAgentProtocol {
    enum_from_str(s).unwrap_or(RemoteAgentProtocol::Acp)
}

fn parse_auth_type(s: &str) -> RemoteAgentAuthType {
    enum_from_str(s).unwrap_or(RemoteAgentAuthType::None)
}

fn parse_status(s: &str) -> RemoteAgentStatus {
    enum_from_str(s).unwrap_or(RemoteAgentStatus::Unknown)
}

fn db_err(e: nomifun_db::DbError) -> AppError {
    AppError::Internal(e.to_string())
}

/// Parse a remote-agent path id (`Path<String>`) into the i64 primary key.
/// A non-numeric id can never match an i64-keyed row, so it surfaces as
/// `NotFound` rather than a 400 — matching the "missing agent" semantics
/// callers already handle.
fn parse_id(id: &str) -> Result<i64, AppError> {
    id.parse::<i64>()
        .map_err(|_| AppError::NotFound(format!("Remote agent '{id}' not found")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_token_long() {
        assert_eq!(mask_token("my-secret-token-1234"), "***1234");
    }

    #[test]
    fn mask_token_short() {
        assert_eq!(mask_token("ab"), "***");
    }

    #[test]
    fn mask_token_exactly_four() {
        assert_eq!(mask_token("abcd"), "***");
    }

    #[test]
    fn mask_token_five() {
        assert_eq!(mask_token("abcde"), "***bcde");
    }

    #[test]
    fn validate_ws_url_accepts_ws() {
        assert!(validate_ws_url("ws://localhost:8080").is_ok());
    }

    #[test]
    fn validate_ws_url_accepts_wss() {
        assert!(validate_ws_url("wss://remote.example.com").is_ok());
    }

    #[test]
    fn validate_ws_url_rejects_http() {
        assert!(validate_ws_url("http://example.com").is_err());
    }

    #[test]
    fn validate_ws_url_rejects_https() {
        assert!(validate_ws_url("https://example.com").is_err());
    }

    #[test]
    fn generate_device_keypair_produces_valid_output() {
        let key = [0x42u8; 32];
        let (id, pub_key, priv_key) = generate_device_keypair(&key).unwrap();

        assert!(id.starts_with("dev_"));
        assert!(!pub_key.is_empty());
        assert!(!priv_key.is_empty());

        // Decrypt and verify the keys decode correctly
        let pub_b64 = decrypt_string(&pub_key, &key).unwrap();
        let priv_b64 = decrypt_string(&priv_key, &key).unwrap();

        let pub_bytes = BASE64.decode(&pub_b64).unwrap();
        let priv_bytes = BASE64.decode(&priv_b64).unwrap();
        assert_eq!(pub_bytes.len(), 32);
        assert_eq!(priv_bytes.len(), 32);

        // Verify the keypair is consistent
        let signing = SigningKey::from_bytes(&priv_bytes.try_into().unwrap());
        let verifying = signing.verifying_key();
        assert_eq!(verifying.as_bytes(), pub_bytes.as_slice());
    }

    #[test]
    fn enum_to_str_protocol() {
        assert_eq!(enum_to_str(&RemoteAgentProtocol::OpenClaw), "openclaw");
        assert_eq!(enum_to_str(&RemoteAgentProtocol::ZeroClaw), "zeroclaw");
        assert_eq!(enum_to_str(&RemoteAgentProtocol::Acp), "acp");
    }

    #[test]
    fn enum_to_str_auth_type() {
        assert_eq!(enum_to_str(&RemoteAgentAuthType::Bearer), "bearer");
        assert_eq!(enum_to_str(&RemoteAgentAuthType::Password), "password");
        assert_eq!(enum_to_str(&RemoteAgentAuthType::None), "none");
    }

    #[test]
    fn parse_protocol_known_values() {
        assert_eq!(parse_protocol("openclaw"), RemoteAgentProtocol::OpenClaw);
        assert_eq!(parse_protocol("zeroclaw"), RemoteAgentProtocol::ZeroClaw);
        assert_eq!(parse_protocol("acp"), RemoteAgentProtocol::Acp);
    }

    #[test]
    fn parse_protocol_unknown_defaults() {
        assert_eq!(parse_protocol("unknown_proto"), RemoteAgentProtocol::Acp);
    }

    #[test]
    fn parse_auth_type_known_values() {
        assert_eq!(parse_auth_type("bearer"), RemoteAgentAuthType::Bearer);
        assert_eq!(parse_auth_type("password"), RemoteAgentAuthType::Password);
        assert_eq!(parse_auth_type("none"), RemoteAgentAuthType::None);
    }

    #[test]
    fn parse_status_known_values() {
        assert_eq!(parse_status("unknown"), RemoteAgentStatus::Unknown);
        assert_eq!(parse_status("connected"), RemoteAgentStatus::Connected);
        assert_eq!(parse_status("pending"), RemoteAgentStatus::Pending);
        assert_eq!(parse_status("error"), RemoteAgentStatus::Error);
    }
}
