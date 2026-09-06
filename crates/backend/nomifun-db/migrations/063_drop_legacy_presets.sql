-- AP-6 clean cut: the canonical AgentPreset lineage is stored only in
-- `nomi_agent_presets` and `nomi_agent_preset_revisions`.
--
-- Keep the historical migrations immutable, but remove the retired preset
-- aggregate from the current physical schema. Child tables are dropped before
-- their catalog/root tables.

DROP TABLE IF EXISTS preset_agent_preferences;
DROP TABLE IF EXISTS preset_examples;
DROP TABLE IF EXISTS preset_knowledge_bases;
DROP TABLE IF EXISTS preset_knowledge_policy;
DROP TABLE IF EXISTS preset_localizations;
DROP TABLE IF EXISTS preset_model_preferences;
DROP TABLE IF EXISTS preset_skill_bindings;
DROP TABLE IF EXISTS preset_tag_bindings;
DROP TABLE IF EXISTS preset_targets;
DROP TABLE IF EXISTS preset_user_state;
DROP TABLE IF EXISTS preset_tags;
DROP TABLE IF EXISTS presets;
