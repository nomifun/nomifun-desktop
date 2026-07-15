//! Contracts for reusable NomiFun presets and their execution snapshots.

use std::collections::HashMap;
use nomifun_common::KnowledgeBaseId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresetSource { Builtin, User, Extension }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PresetTarget { Conversation, ExecutionStep, Companion, PublicCompanion, Cron }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresetTagDimension { Audience, Scenario }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPreference {
    #[serde(deserialize_with = "crate::serde_util::deserialize_agent_reference")]
    pub agent_id: String,
    #[serde(default)] pub required: bool,
}

/// Provider-qualified model reference. Bundled and extension catalog entries
/// may omit `provider_id`; execution must resolve those unqualified natural
/// model names to a canonical provider entity before dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelPreference {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_provider_id"
    )]
    pub provider_id: Option<String>,
    pub model: String,
    #[serde(default)] pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillBinding {
    pub skill_name: String,
    #[serde(default)] pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeBaseBinding {
    pub knowledge_base_id: KnowledgeBaseId,
    #[serde(default)] pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresetKnowledgePolicy {
    #[serde(default)] pub enabled: bool,
    #[serde(default = "default_knowledge_mode")] pub mode: String,
    #[serde(default)] pub writeback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub eagerness: Option<String>,
    #[serde(default)] pub grounded: bool,
}

fn default_knowledge_mode() -> String { "inherit".to_string() }

impl Default for PresetKnowledgePolicy {
    fn default() -> Self {
        Self { enabled: false, mode: default_knowledge_mode(), writeback: false, eagerness: None, grounded: false }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_reference")]
    pub id: String,
    pub revision: i64,
    pub source: PresetSource,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub source_key: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")] pub name_i18n: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub description: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")] pub description_i18n: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub routing_description: Option<String>,
    #[serde(default)] pub instructions: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")] pub instructions_i18n: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub avatar: Option<String>,
    #[serde(default)] pub fallback_allowed: bool,
    #[serde(default)] pub targets: Vec<PresetTarget>,
    #[serde(default)] pub agent_preferences: Vec<AgentPreference>,
    #[serde(default)] pub model_preferences: Vec<ModelPreference>,
    #[serde(default)] pub included_skills: Vec<SkillBinding>,
    #[serde(default)] pub excluded_auto_skills: Vec<String>,
    #[serde(default)] pub knowledge_policy: PresetKnowledgePolicy,
    #[serde(default)] pub knowledge_bases: Vec<KnowledgeBaseBinding>,
    #[serde(default)] pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")] pub examples_i18n: HashMap<String, Vec<String>>,
    #[serde(default)] pub audience_tags: Vec<String>,
    #[serde(default)] pub scenario_tags: Vec<String>,
    pub enabled: bool,
    pub auto_selectable: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_agent_reference"
    )]
    pub preferred_agent_id: Option<String>,
    pub sort_order: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePresetRequest {
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_preset_id"
    )]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)] pub description: Option<String>,
    #[serde(default)] pub routing_description: Option<String>,
    #[serde(default)] pub instructions: String,
    #[serde(default)] pub avatar: Option<String>,
    #[serde(default)] pub fallback_allowed: bool,
    #[serde(default)] pub targets: Vec<PresetTarget>,
    #[serde(default)] pub agent_preferences: Vec<AgentPreference>,
    #[serde(default)] pub model_preferences: Vec<ModelPreference>,
    #[serde(default)] pub included_skills: Vec<SkillBinding>,
    #[serde(default)] pub excluded_auto_skills: Vec<String>,
    #[serde(default)] pub knowledge_policy: PresetKnowledgePolicy,
    #[serde(default)] pub knowledge_bases: Vec<KnowledgeBaseBinding>,
    #[serde(default)] pub examples: Vec<String>,
    #[serde(default)] pub examples_i18n: HashMap<String, Vec<String>>,
    #[serde(default)] pub audience_tags: Vec<String>,
    #[serde(default)] pub scenario_tags: Vec<String>,
    #[serde(default)] pub name_i18n: HashMap<String, String>,
    #[serde(default)] pub description_i18n: HashMap<String, String>,
    #[serde(default)] pub instructions_i18n: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatePresetRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub routing_description: Option<String>,
    pub instructions: Option<String>,
    pub avatar: Option<String>,
    pub fallback_allowed: Option<bool>,
    pub targets: Option<Vec<PresetTarget>>,
    pub agent_preferences: Option<Vec<AgentPreference>>,
    pub model_preferences: Option<Vec<ModelPreference>>,
    pub included_skills: Option<Vec<SkillBinding>>,
    pub excluded_auto_skills: Option<Vec<String>>,
    pub knowledge_policy: Option<PresetKnowledgePolicy>,
    pub knowledge_bases: Option<Vec<KnowledgeBaseBinding>>,
    pub examples: Option<Vec<String>>,
    pub examples_i18n: Option<HashMap<String, Vec<String>>>,
    pub audience_tags: Option<Vec<String>>,
    pub scenario_tags: Option<Vec<String>>,
    pub name_i18n: Option<HashMap<String, String>>,
    pub description_i18n: Option<HashMap<String, String>>,
    pub instructions_i18n: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SetPresetStateRequest {
    pub enabled: Option<bool>,
    pub auto_selectable: Option<bool>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_agent_reference"
    )]
    pub preferred_agent_id: Option<String>,
    pub sort_order: Option<i32>,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PresetOverrides {
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_agent_reference"
    )]
    pub agent_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_provider_id"
    )]
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub instructions: Option<String>,
    #[serde(default)] pub include_skills: Vec<String>,
    #[serde(default)] pub exclude_skills: Vec<String>,
    pub knowledge_policy: Option<PresetKnowledgePolicy>,
    #[serde(default)]
    pub knowledge_base_ids: Option<Vec<KnowledgeBaseId>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePresetRequest {
    pub target: PresetTarget,
    #[serde(default)] pub locale: Option<String>,
    #[serde(default)] pub overrides: PresetOverrides,
}

/// Persist this execution-time materialization with the target object. Later
/// preset edits must never mutate an existing snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedPresetSnapshot {
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_reference")]
    pub preset_id: String,
    pub preset_revision: i64,
    pub preset_name: String,
    pub target: PresetTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub routing_description: Option<String>,
    #[serde(default)] pub instructions: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_agent_reference"
    )]
    pub resolved_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub resolved_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub resolved_agent_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub resolved_model: Option<ModelPreference>,
    #[serde(default)] pub included_skills: Vec<String>,
    #[serde(default)] pub excluded_auto_skills: Vec<String>,
    #[serde(default)] pub knowledge_policy: PresetKnowledgePolicy,
    #[serde(default)]
    pub knowledge_base_ids: Vec<KnowledgeBaseId>,
    #[serde(default)] pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetTagResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_preset_tag_reference")]
    pub key: String,
    pub dimension: PresetTagDimension,
    pub label: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")] pub label_i18n: HashMap<String, String>,
    pub sort_order: i32,
    pub builtin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePresetTagRequest { pub dimension: PresetTagDimension, pub label: String }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatePresetTagRequest { pub label: Option<String>, pub sort_order: Option<i32> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPresetsRequest { pub presets: Vec<CreatePresetRequest> }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportPresetsResult {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    #[serde(default)] pub errors: Vec<PresetImportError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetImportError { pub id: String, pub error: String }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER_ID: &str = "prov_018f1234-5678-7abc-8def-012345678990";
    const PRESET_ID: &str = "preset_018f1234-5678-7abc-8def-012345678991";
    #[test]
    fn provider_qualified_model_round_trips() {
        let model = ModelPreference { provider_id: Some(PROVIDER_ID.into()), model: "gpt-5".into(), required: true };
        let value = serde_json::to_value(&model).unwrap();
        assert_eq!(value["provider_id"], PROVIDER_ID);
        assert_eq!(serde_json::from_value::<ModelPreference>(value).unwrap(), model);
    }
    #[test]
    fn target_names_are_stable_snake_case() {
        assert_eq!(serde_json::to_string(&PresetTarget::ExecutionStep).unwrap(), "\"execution_step\"");
    }

    #[test]
    fn model_preference_rejects_platform_key_as_provider_id() {
        let raw = json!({ "provider_id": "openai", "model": "gpt-5" });
        assert!(serde_json::from_value::<ModelPreference>(raw).is_err());
    }

    #[test]
    fn create_preset_request_rejects_nonentity_ids() {
        for id in ["word-creator", ""] {
            let raw = json!({ "id": id, "name": "General" });
            assert!(serde_json::from_value::<CreatePresetRequest>(raw).is_err());
        }
    }

    #[test]
    fn preset_reference_accepts_catalog_key_and_canonical_user_id() {
        for id in ["word-creator", PRESET_ID] {
            let raw = json!({
                "preset_id": id,
                "preset_revision": 1,
                "preset_name": "General",
                "target": "conversation"
            });
            let snapshot: ResolvedPresetSnapshot = serde_json::from_value(raw).unwrap();
            assert_eq!(snapshot.preset_id, id);
        }
    }

    #[test]
    fn preset_reference_rejects_malformed_entity_namespace_value() {
        let raw = json!({
            "preset_id": "preset_not-a-uuid",
            "preset_revision": 1,
            "preset_name": "Bad",
            "target": "conversation"
        });
        assert!(serde_json::from_value::<ResolvedPresetSnapshot>(raw).is_err());
    }

    #[test]
    fn preset_overrides_reject_noncanonical_entity_ids() {
        let raw = json!({
            "target": "conversation",
            "overrides": {
                "provider_id": "openai",
                "knowledge_base_ids": ["knowledge-1"]
            }
        });
        assert!(serde_json::from_value::<ResolvePresetRequest>(raw).is_err());
    }

    #[test]
    fn knowledge_base_binding_uses_typed_canonical_id() {
        let id = KnowledgeBaseId::new();
        let binding: KnowledgeBaseBinding = serde_json::from_value(json!({
            "knowledge_base_id": id.as_str()
        }))
        .unwrap();
        let typed: &KnowledgeBaseId = &binding.knowledge_base_id;
        assert_eq!(typed, &id);

        assert!(serde_json::from_value::<KnowledgeBaseBinding>(json!({
            "knowledge_base_id": "kb_docs"
        }))
        .is_err());
    }

    #[test]
    fn agent_reference_accepts_catalog_keys_and_rejects_malformed_entity_claims() {
        for agent_id in ["nomi", "agent_builtin_nomi"] {
            let raw = json!({ "agent_id": agent_id });
            let preference: AgentPreference = serde_json::from_value(raw).unwrap();
            assert_eq!(preference.agent_id, agent_id);
        }
        let malformed = json!({ "agent_id": "agent_not-a-uuid" });
        assert!(serde_json::from_value::<AgentPreference>(malformed).is_err());
    }

    #[test]
    fn canonical_provider_fixture_is_uuid_v7() {
        let model = ModelPreference {
            provider_id: Some(PROVIDER_ID.into()),
            model: "gpt-5".into(),
            required: false,
        };
        let value = serde_json::to_value(&model).unwrap();
        assert!(serde_json::from_value::<ModelPreference>(value).is_ok());
    }
}
