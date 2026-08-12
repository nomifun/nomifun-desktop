use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `providers` table.
///
/// JSON fields (`bedrock_config`) are stored as TEXT in SQLite and deserialized
/// by the service layer. Model identity/display metadata lives in
/// `provider_models`; every invocation route and health observation lives in
/// `provider_model_capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Provider {
    pub id: i64,
    pub provider_id: String,
    pub platform: String,
    pub name: String,
    pub base_url: String,
    pub auth_scheme: String,
    pub credentials_encrypted: String,
    pub enabled: bool,
    /// JSON object: Bedrock-specific configuration.
    pub bedrock_config: Option<String>,
    /// Lower values have higher priority in provider selection.
    pub sort_order: i64,
    /// Monotonic revision of the effective invocation graph. Health and
    /// display-only writes never change it.
    pub config_revision: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
