//! Row models and parameter structs for the `agent_metadata` table.
//!
//! JSON-encoded columns (`agent_source_info`, `args`, `env`,
//! `native_skills_dirs`, `behavior_policy`) stay as opaque strings at this layer. The ai-agent crate
//! owns the schema of these payloads and decodes them on read.

use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `agent_metadata` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentMetadataRow {
    pub id: i64,
    /// Bare UUIDv7 business ID for every agent row.
    pub agent_id: String,
    pub icon: Option<String>,
    pub name: String,
    pub name_i18n: Option<String>,
    pub description: Option<String>,
    pub description_i18n: Option<String>,

    pub backend: Option<String>,
    pub agent_type: String,
    pub agent_source: String,
    pub agent_source_info: Option<String>,
    /// Stable catalog/install lineage, such as `agent_builtin_claude`.
    ///
    /// This is deliberately separate from `agent_id`; the latter is always a
    /// bare UUIDv7, including for builtin rows.
    pub source_key: Option<String>,

    pub enabled: bool,

    pub command: Option<String>,
    pub args: Option<String>,
    pub env: Option<String>,
    pub native_skills_dirs: Option<String>,

    pub behavior_policy: Option<String>,

    /// Display ordering key — smaller values appear first.
    pub sort_order: i64,

    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Insert / upsert parameters for the full row.
///
/// JSON fields are pre-serialized strings; the caller is responsible for
/// encoding. `source_key` is catalog-owned: custom inserts leave it `NULL`,
/// while the repository preserves existing builtin lineage on conflict.
#[derive(Debug, Clone)]
pub struct UpsertAgentMetadataParams<'a> {
    pub agent_id: &'a str,
    pub icon: Option<&'a str>,
    pub name: &'a str,
    pub name_i18n: Option<&'a str>,
    pub description: Option<&'a str>,
    pub description_i18n: Option<&'a str>,
    pub backend: Option<&'a str>,
    pub agent_type: &'a str,
    pub agent_source: &'a str,
    pub agent_source_info: Option<&'a str>,
    pub enabled: bool,
    pub command: Option<&'a str>,
    pub args: Option<&'a str>,
    pub env: Option<&'a str>,
    pub native_skills_dirs: Option<&'a str>,
    pub behavior_policy: Option<&'a str>,
    pub sort_order: i64,
}
