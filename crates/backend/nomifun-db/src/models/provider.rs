use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `providers` table.
///
/// JSON fields (bedrock_config) are stored as TEXT in SQLite and deserialized
/// by the service layer. The per-model surface (membership, enabled,
/// protocol, context limit, description, health) lives exclusively on
/// `provider_models` rows since migration 016 dropped the legacy `models`
/// array and the five per-model JSON map columns; migration 017 dropped the
/// provider-level `capabilities` column (the wire field is retired and
/// always `[]`).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Provider {
    pub id: i64,
    pub provider_id: String,
    pub platform: String,
    pub name: String,
    pub base_url: String,
    pub api_key_encrypted: String,
    pub enabled: bool,
    /// JSON object: Bedrock-specific configuration.
    pub bedrock_config: Option<String>,
    /// When true, base_url is treated as a complete endpoint URL.
    /// The system will NOT append paths like /v1/chat/completions.
    pub is_full_url: bool,
    /// Lower values have higher priority in provider selection.
    pub sort_order: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
