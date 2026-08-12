use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `provider_connections` table: non-default per-role
/// connection profiles for a provider (e.g. a separate voice domain +
/// credential set). The providers row itself is the explicit `default`
/// connection. `extra` is a JSON object.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderConnectionRow {
    pub id: i64,
    pub connection_id: String,
    pub provider_id: String,
    pub role: String,
    pub label: Option<String>,
    pub base_url: String,
    pub auth_scheme: String,
    pub credentials_encrypted: String,
    pub extra: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Upsert params, keyed by `(provider_id, role)`; `extra` is a JSON object
/// pre-serialized by the caller.
#[derive(Debug, Clone)]
pub struct UpsertProviderConnectionParams<'a> {
    pub role: &'a str,
    pub label: Option<&'a str>,
    pub base_url: &'a str,
    pub auth_scheme: &'a str,
    pub credentials_encrypted: &'a str,
    pub extra: &'a str,
}
