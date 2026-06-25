use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row in `connector_credentials` — encrypted credentials for a source connector
/// (feishu / notion / …). `payload_encrypted` is an opaque AES-256-GCM ciphertext;
/// the service layer holds the key and (de)serializes the JSON payload (e.g.
/// `{ "app_id": ..., "app_secret": ... }`). Secrets never appear on the wire —
/// API responses expose only `id` / `kind` / `name`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConnectorCredentialRow {
    pub id: String,
    /// Connector discriminator: "feishu", "notion", …
    pub kind: String,
    /// User-facing label.
    pub name: String,
    /// AES-256-GCM ciphertext of the JSON credential payload.
    pub payload_encrypted: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
