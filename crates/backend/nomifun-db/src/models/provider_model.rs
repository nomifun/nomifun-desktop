use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Identity and display metadata for one provider model.
///
/// Invocation configuration never lives on this row. Each usable modality is
/// an independent [`ProviderModelCapabilityRow`].
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderModelRow {
    pub id: i64,
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub description: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// One task-scoped invocation configuration, keyed by
/// `(provider_id, model, task)`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderModelCapabilityRow {
    pub id: i64,
    pub provider_id: String,
    pub model: String,
    pub task: String,
    pub traits: String,
    pub protocol: String,
    pub connection_role: String,
    pub base_url_override: Option<String>,
    pub endpoint: Option<String>,
    pub poll_endpoint: Option<String>,
    pub content_endpoint: Option<String>,
    pub realtime_endpoint: Option<String>,
    pub allow_cross_origin_credentials: bool,
    pub provider_params: String,
    pub context_limit: Option<i64>,
    pub health: Option<String>,
    pub health_checked_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Complete task-scoped capability input. JSON values are pre-serialized by
/// the caller; protocol and connection role are validated again by the
/// repository before persistence.
#[derive(Debug, Clone, Copy, Default)]
pub struct NewProviderModelCapability<'a> {
    pub task: &'a str,
    pub traits: &'a str,
    pub protocol: &'a str,
    pub connection_role: &'a str,
    pub base_url_override: Option<&'a str>,
    pub endpoint: Option<&'a str>,
    pub poll_endpoint: Option<&'a str>,
    pub content_endpoint: Option<&'a str>,
    pub realtime_endpoint: Option<&'a str>,
    pub allow_cross_origin_credentials: bool,
    pub provider_params: &'a str,
    pub context_limit: Option<i64>,
}

/// Create a model and its complete non-empty capability set atomically.
#[derive(Debug, Clone, Default)]
pub struct NewProviderModel<'a> {
    pub model: &'a str,
    pub enabled: bool,
    pub sort_order: i64,
    pub description: Option<&'a str>,
    pub capabilities: &'a [NewProviderModelCapability<'a>],
}
