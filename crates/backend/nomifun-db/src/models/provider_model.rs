use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `provider_models` table — the authoritative per-model
/// entity (migration 014 converged `providers.models` + the parallel JSON map
/// columns + `model_profiles` into this table). `tasks`/`traits`/`params` are
/// JSON text (serialized `ModelTask[]` / `ModelTrait[]` / object) and `health`
/// is a JSON `ModelHealthStatus`; the service layer (de)serializes them.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProviderModelRow {
    pub id: i64,
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub tasks: String,           // JSON Vec<ModelTask>
    pub traits: String,          // JSON Vec<ModelTrait>
    pub protocol: Option<String>,
    pub connection_role: Option<String>,
    pub params: String,          // JSON object
    pub context_limit: Option<i64>,
    pub description: Option<String>,
    pub source: String,          // "inferred" | "user"
    pub health: Option<String>,  // JSON ModelHealthStatus
    pub health_checked_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Insert params: JSON strings pre-serialized by the caller.
#[derive(Debug, Clone, Default)]
pub struct NewProviderModel<'a> {
    pub model: &'a str,
    pub enabled: bool,
    pub sort_order: i64,
    pub tasks: &'a str,
    pub traits: &'a str,
    pub protocol: Option<&'a str>,
    pub params: &'a str,
    pub context_limit: Option<i64>,
    pub description: Option<&'a str>,
    pub source: &'a str,
    pub health: Option<&'a str>,
}

/// Partial update; `None` = keep, `Some(None)` = clear (for nullable columns).
#[derive(Debug, Clone, Default)]
pub struct ProviderModelUpdate<'a> {
    pub enabled: Option<bool>,
    pub sort_order: Option<i64>,
    pub tasks: Option<&'a str>,
    pub traits: Option<&'a str>,
    pub protocol: Option<Option<&'a str>>,
    pub connection_role: Option<Option<&'a str>>,
    pub params: Option<&'a str>,
    pub context_limit: Option<Option<i64>>,
    pub description: Option<Option<&'a str>>,
    pub source: Option<&'a str>,
}
