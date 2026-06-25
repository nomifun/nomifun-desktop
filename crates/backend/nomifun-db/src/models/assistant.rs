//! Row models and repository parameter structs for the assistants domain.

use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `assistants` table (user-authored assistants only).
///
/// JSON-encoded columns (`enabled_skills`, `custom_skill_names`,
/// `disabled_builtin_skills`, `prompts`, `models`, `*_i18n`) stay as opaque
/// strings at this layer; the service deserializes them.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssistantRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub avatar: Option<String>,
    pub preset_agent_type: String,
    pub enabled_skills: Option<String>,
    pub custom_skill_names: Option<String>,
    pub disabled_builtin_skills: Option<String>,
    pub prompts: Option<String>,
    pub models: Option<String>,
    pub name_i18n: Option<String>,
    pub description_i18n: Option<String>,
    pub prompts_i18n: Option<String>,
    pub audience_tags: Option<String>,
    pub scenario_tags: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

/// Row mapping for the `assistant_overrides` table (per-assistant user state).
///
/// `preset_agent_type` is `Some(_)` when the user has switched the main agent
/// on a built-in assistant (which cannot be mutated at its source). `None`
/// means "inherit from the built-in / user row".
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssistantOverrideRow {
    pub assistant_id: String,
    pub enabled: bool,
    pub sort_order: i32,
    pub last_used_at: Option<TimestampMs>,
    pub preset_agent_type: Option<String>,
    pub updated_at: TimestampMs,
}

/// Insert parameters for `IAssistantRepository::create` / `::upsert`.
///
/// JSON fields are pre-serialized strings so the repository layer stays
/// agnostic to how the service encodes them.
#[derive(Debug, Clone)]
pub struct CreateAssistantParams<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub avatar: Option<&'a str>,
    pub preset_agent_type: &'a str,
    pub enabled_skills: Option<&'a str>,
    pub custom_skill_names: Option<&'a str>,
    pub disabled_builtin_skills: Option<&'a str>,
    pub prompts: Option<&'a str>,
    pub models: Option<&'a str>,
    pub name_i18n: Option<&'a str>,
    pub description_i18n: Option<&'a str>,
    pub prompts_i18n: Option<&'a str>,
    pub audience_tags: Option<&'a str>,
    pub scenario_tags: Option<&'a str>,
}

/// Partial update parameters for `IAssistantRepository::update`.
///
/// Every field is `Option` — `None` keeps the current value.
#[derive(Debug, Clone, Default)]
pub struct UpdateAssistantParams<'a> {
    pub name: Option<&'a str>,
    pub description: Option<Option<&'a str>>,
    pub avatar: Option<Option<&'a str>>,
    pub preset_agent_type: Option<&'a str>,
    pub enabled_skills: Option<Option<&'a str>>,
    pub custom_skill_names: Option<Option<&'a str>>,
    pub disabled_builtin_skills: Option<Option<&'a str>>,
    pub prompts: Option<Option<&'a str>>,
    pub models: Option<Option<&'a str>>,
    pub name_i18n: Option<Option<&'a str>>,
    pub description_i18n: Option<Option<&'a str>>,
    pub prompts_i18n: Option<Option<&'a str>>,
    pub audience_tags: Option<Option<&'a str>>,
    pub scenario_tags: Option<Option<&'a str>>,
}

/// Upsert parameters for `IAssistantOverrideRepository::upsert`.
///
/// `preset_agent_type` uses `Option<Option<&str>>`: outer `None` keeps the
/// current value, outer `Some(inner)` writes `inner` (which itself may be
/// `None` to clear the override).
#[derive(Debug, Clone, Default)]
pub struct UpsertOverrideParams<'a> {
    pub assistant_id: &'a str,
    pub enabled: bool,
    pub sort_order: i32,
    pub last_used_at: Option<TimestampMs>,
    pub preset_agent_type: Option<Option<&'a str>>,
}

/// Row mapping for the `assistant_tags` table (user-created tags only).
/// Built-in seed tags are served from the embedded `tags.json` manifest.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AssistantTagRow {
    pub key: String,
    pub dimension: String,
    pub label: String,
    pub sort_order: i32,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone)]
pub struct CreateAssistantTagParams<'a> {
    pub key: &'a str,
    pub dimension: &'a str,
    pub label: &'a str,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateAssistantTagParams<'a> {
    pub label: Option<&'a str>,
    pub sort_order: Option<i32>,
}
