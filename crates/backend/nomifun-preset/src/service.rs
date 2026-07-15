//! Preset catalog CRUD and the single execution-time resolver.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use nomifun_api_types::*;
use nomifun_common::{AgentId, AppError, KnowledgeBaseId, PresetId, PresetTagId, ProviderId};
use nomifun_db::{
    CreatePresetTagParams, IAgentMetadataRepository, IPresetRepository, IPresetStateRepository,
    IPresetTagRepository, IProviderRepository, PresetRecord, PresetWriteParams,
    UpdatePresetTagParams, UpsertPresetStateParams,
};
use nomifun_extension::{ExtensionRegistry, ResolvedPreset};

use crate::builtin::{AvatarAsset, BuiltinPreset, BuiltinPresetRegistry};
use nomifun_extension::{PresetClassifier, PresetRuleDispatcher};

pub struct PresetService {
    repo: Arc<dyn IPresetRepository>,
    state_repo: Arc<dyn IPresetStateRepository>,
    tag_repo: Arc<dyn IPresetTagRepository>,
    agent_repo: Arc<dyn IAgentMetadataRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
    builtin: Arc<BuiltinPresetRegistry>,
    extension_registry: ExtensionRegistry,
    user_data_dir: PathBuf,
}

#[async_trait::async_trait]
impl PresetClassifier for PresetService {
    async fn classify(&self, id: &str) -> PresetSource { self.classify_source(id).await }
}

#[async_trait::async_trait]
impl PresetRuleDispatcher for PresetService {
    async fn read_rule(&self, id: &str, locale: Option<&str>) -> Result<String, AppError> {
        let preset = self.get(id).await?;
        Ok(locale.and_then(|l| localized_value(&preset.instructions_i18n, l)).unwrap_or(preset.instructions))
    }
    async fn write_rule(&self, id: &str, _locale: Option<&str>, content: &str) -> Result<(), AppError> {
        self.update(id, UpdatePresetRequest { instructions: Some(content.to_string()), ..Default::default() }).await?;
        Ok(())
    }
    async fn delete_rule(&self, id: &str) -> Result<bool, AppError> {
        self.update(id, UpdatePresetRequest { instructions: Some(String::new()), ..Default::default() }).await?;
        Ok(true)
    }
    async fn read_skill(&self, _id: &str, _locale: Option<&str>) -> Result<String, AppError> { Ok(String::new()) }
    async fn write_skill(&self, _id: &str, _locale: Option<&str>, _content: &str) -> Result<(), AppError> {
        Err(AppError::BadRequest("Preset skills are managed through included_skills".into()))
    }
    async fn delete_skill(&self, _id: &str) -> Result<bool, AppError> { Ok(false) }
}

impl PresetService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: Arc<dyn IPresetRepository>,
        state_repo: Arc<dyn IPresetStateRepository>,
        tag_repo: Arc<dyn IPresetTagRepository>,
        agent_repo: Arc<dyn IAgentMetadataRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
        builtin: Arc<BuiltinPresetRegistry>,
        extension_registry: ExtensionRegistry,
        user_data_dir: PathBuf,
    ) -> Self {
        migrate_asset_directories(&user_data_dir);
        Self { repo, state_repo, tag_repo, agent_repo, provider_repo, builtin, extension_registry, user_data_dir }
    }

    pub async fn classify_source(&self, id: &str) -> PresetSource {
        if self.builtin.has(id) { PresetSource::Builtin }
        else if self.extension_registry.has_preset(id).await { PresetSource::Extension }
        else { PresetSource::User }
    }

    pub async fn list(&self) -> Result<Vec<PresetResponse>, AppError> {
        let states = self.state_repo.get_all().await?;
        let state_map: HashMap<_, _> = states.into_iter().map(|s| (s.preset_id.clone(), s)).collect();
        let mut output = Vec::new();
        for item in self.builtin.all() { output.push(self.builtin_response(item, state_map.get(&item.id))); }
        for record in self.repo.list().await? {
            let mut response = record_to_response(&record)?;
            self.hydrate_migrated_instructions(&mut response);
            output.push(response);
        }
        for item in self.extension_registry.get_presets().await { output.push(extension_to_response(&item)); }
        output.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| b.last_used_at.cmp(&a.last_used_at)));
        let ids: Vec<_> = output.iter().map(|p| p.id.as_str()).collect();
        let _ = self.state_repo.delete_orphans(&ids).await;
        Ok(output)
    }

    pub async fn get(&self, id: &str) -> Result<PresetResponse, AppError> {
        match self.classify_source(id).await {
            PresetSource::Builtin => {
                let item = self.builtin.get(id).ok_or_else(|| AppError::NotFound(format!("preset '{id}' not found")))?;
                let state = self.state_repo.get(id).await?;
                Ok(self.builtin_response(item, state.as_ref()))
            }
            PresetSource::Extension => {
                let item = self.extension_registry.get_preset_by_id(id).await
                    .ok_or_else(|| AppError::NotFound(format!("preset '{id}' not found")))?;
                Ok(extension_to_response(&item))
            }
            PresetSource::User => {
                PresetId::parse(id).map_err(|error| {
                    AppError::BadRequest(format!("invalid user preset id: {error}"))
                })?;
                let record = self.repo.get(id).await?
                    .ok_or_else(|| AppError::NotFound(format!("preset '{id}' not found")))?;
                let mut response = record_to_response(&record)?;
                self.hydrate_migrated_instructions(&mut response);
                Ok(response)
            }
        }
    }

    /// Filesystem-authored rules from pre-034 installs are renamed into the
    /// Preset namespace at startup. Until the user next edits the preset (which
    /// writes relational `instructions`), hydrate those files so migration is
    /// lossless. Locale files use `{id}.{locale}.md`.
    fn hydrate_migrated_instructions(&self, preset: &mut PresetResponse) {
        let dir = self.user_data_dir.join("preset-instructions");
        let needs_legacy_hydration = preset.instructions.is_empty();
        if preset.instructions.is_empty() {
            preset.instructions = std::fs::read_to_string(dir.join(format!("{}.md", preset.id)))
                .unwrap_or_default();
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let prefix = format!("{}.", preset.id);
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let Some(locale) = name.strip_prefix(&prefix).and_then(|v| v.strip_suffix(".md")) else {
                    continue;
                };
                if !locale.is_empty()
                    && !preset.instructions_i18n.contains_key(locale)
                    && let Ok(content) = std::fs::read_to_string(entry.path())
                {
                    preset.instructions_i18n.insert(locale.to_owned(), content);
                }
            }
        }
        // The former free-form template skill note was not an executable Skill.
        // Preserve it as supplemental preset instructions instead of retaining
        // a second, ambiguous capability model.
        if needs_legacy_hydration && let Ok(note) = std::fs::read_to_string(
            self.user_data_dir
                .join("preset-legacy-skill-notes")
                .join(format!("{}.md", preset.id)),
        ) && !note.trim().is_empty()
        {
            if !preset.instructions.is_empty() {
                preset.instructions.push_str("\n\n");
            }
            preset.instructions.push_str(note.trim());
        }
    }

    pub async fn create(&self, request: CreatePresetRequest) -> Result<PresetResponse, AppError> {
        validate_request(
            &request.name,
            &request.agent_preferences,
            &request.model_preferences,
            &request.knowledge_policy,
            &request.knowledge_bases,
        )?;
        let id = match request.id.clone() {
            Some(value) => PresetId::parse(value)
                .map_err(|error| AppError::BadRequest(format!("invalid user preset id: {error}")))?
                .into_string(),
            None => PresetId::new().into_string(),
        };
        if self.builtin.has(&id) || self.extension_registry.has_preset(&id).await {
            return Err(AppError::Conflict(format!("preset id '{id}' already exists")));
        }
        let params = write_from_create(id.clone(), request);
        let record = self.repo.create(&params).await?;
        let state = self.state_repo.upsert(&UpsertPresetStateParams {
            preset_id: id, enabled: true, auto_selectable: false,
            preferred_agent_id: None, sort_order: 0, last_used_at: None,
        }).await?;
        let mut response = record_to_response(&record)?;
        apply_state(&mut response, Some(&state));
        Ok(response)
    }

    pub async fn update(&self, id: &str, request: UpdatePresetRequest) -> Result<PresetResponse, AppError> {
        if self.classify_source(id).await != PresetSource::User {
            return Err(AppError::Forbidden("Copy bundled presets before editing them".into()));
        }
        PresetId::parse(id)
            .map_err(|error| AppError::BadRequest(format!("invalid user preset id: {error}")))?;
        let existing = self.get(id).await?;
        let merged = merge_update(existing, request);
        validate_request(
            &merged.name,
            &merged.agent_preferences,
            &merged.model_preferences,
            &merged.knowledge_policy,
            &merged.knowledge_bases,
        )?;
        let params = write_from_response(merged);
        let record = self.repo.update(id, &params).await?
            .ok_or_else(|| AppError::NotFound(format!("preset '{id}' not found")))?;
        record_to_response(&record)
    }

    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        if self.classify_source(id).await != PresetSource::User {
            return Err(AppError::Forbidden("Bundled presets cannot be deleted".into()));
        }
        PresetId::parse(id)
            .map_err(|error| AppError::BadRequest(format!("invalid user preset id: {error}")))?;
        if !self.repo.delete(id).await? { return Err(AppError::NotFound(format!("preset '{id}' not found"))); }
        let _ = self.state_repo.delete(id).await;
        for dir in ["preset-instructions", "preset-avatars"] {
            remove_files_with_stem(&self.user_data_dir.join(dir), id);
        }
        Ok(())
    }

    pub async fn set_state(&self, id: &str, request: SetPresetStateRequest) -> Result<PresetResponse, AppError> {
        let current = self.get(id).await?;
        if current.source == PresetSource::Extension {
            return Err(AppError::Forbidden("Extension presets are managed by their extension".into()));
        }
        let existing = self.state_repo.get(id).await?;
        let preferred_agent_id = match request.preferred_agent_id {
            Some(value) => {
                validate_agent_reference(&value)?;
                Some(value)
            }
            None => existing.as_ref().and_then(|state| state.preferred_agent_id.clone()),
        };
        self.state_repo.upsert(&UpsertPresetStateParams {
            preset_id: id.to_string(),
            enabled: request.enabled.or_else(|| existing.as_ref().map(|s| s.enabled)).unwrap_or(true),
            auto_selectable: request.auto_selectable.or_else(|| existing.as_ref().map(|s| s.auto_selectable)).unwrap_or(false),
            preferred_agent_id,
            sort_order: request.sort_order.or_else(|| existing.as_ref().map(|s| s.sort_order)).unwrap_or(0),
            last_used_at: request.last_used_at.or_else(|| existing.and_then(|s| s.last_used_at)),
        }).await?;
        self.get(id).await
    }

    /// Resolve one preset into an immutable snapshot. Explicit overrides win,
    /// then ordered preset preferences, then an enabled catalog fallback when
    /// the preset permits fallback.
    pub async fn resolve(
        &self,
        id: &str,
        target: PresetTarget,
        locale: Option<&str>,
        overrides: PresetOverrides,
    ) -> Result<ResolvedPresetSnapshot, AppError> {
        if let Some(provider_id) = overrides.provider_id.as_deref() {
            ProviderId::parse(provider_id).map_err(|error| {
                AppError::BadRequest(format!("invalid provider_id override: {error}"))
            })?;
        }
        if let Some(agent_id) = overrides.agent_id.as_deref() {
            validate_agent_reference(agent_id)?;
        }
        let preset = self.get(id).await?;
        if !preset.enabled { return Err(AppError::BadRequest(format!("preset '{id}' is disabled"))); }
        if !preset.targets.is_empty() && !preset.targets.contains(&target) {
            return Err(AppError::BadRequest(format!("preset '{id}' cannot target {target:?}")));
        }
        let mut warnings = Vec::new();
        let instructions = overrides.instructions.clone().unwrap_or_else(|| {
            locale.and_then(|l| localized_value(&preset.instructions_i18n, l)).unwrap_or_else(|| preset.instructions.clone())
        });

        let resolved_agent_id = if let Some(agent_id) = overrides.agent_id {
            Some(self.resolve_agent(&agent_id, true, &mut warnings).await?)
        } else {
            let mut selected = None;
            if let Some(agent_id) = preset.preferred_agent_id.as_deref() {
                match self.resolve_agent(agent_id, false, &mut warnings).await {
                    Ok(id) => selected = Some(id),
                    Err(error) if preset.fallback_allowed => warnings.push(error.to_string()),
                    Err(error) => return Err(error),
                }
            }
            for pref in &preset.agent_preferences {
                if selected.is_some() { break; }
                match self.resolve_agent(&pref.agent_id, pref.required, &mut warnings).await {
                    Ok(id) => { selected = Some(id); break; }
                    Err(error) if !pref.required && preset.fallback_allowed => warnings.push(error.to_string()),
                    Err(error) => return Err(error),
                }
            }
            if selected.is_none() && preset.fallback_allowed {
                selected = self.agent_repo.list_all().await?.into_iter().find(|a| a.enabled).map(|a| a.id);
                if selected.is_some() { warnings.push("Agent preference unavailable; used the first enabled agent".into()); }
            }
            selected
        };

        let resolved_agent = if let Some(agent_id) = resolved_agent_id.as_ref() {
            self.agent_repo.get(agent_id).await?
        } else {
            None
        };
        let resolved_agent_type = resolved_agent.as_ref().map(|agent| agent.agent_type.clone());
        let resolved_agent_backend = resolved_agent.as_ref().and_then(|agent| agent.backend.clone());

        let resolved_model = if let Some(model) = overrides.model {
            let preference = ModelPreference { provider_id: overrides.provider_id, model, required: true };
            Some(self.resolve_model_preference(&preference, &mut warnings).await?)
        } else {
            let mut selected = None;
            for preference in &preset.model_preferences {
                match self.resolve_model_preference(preference, &mut warnings).await {
                    Ok(resolved) => { selected = Some(resolved); break; }
                    Err(error) if !preference.required && preset.fallback_allowed => {
                        warnings.push(error.to_string());
                    }
                    Err(error) => return Err(error),
                }
            }
            selected
        };

        let mut skills: Vec<String> = preset.included_skills.iter().map(|s| s.skill_name.clone()).collect();
        skills.extend(overrides.include_skills);
        let excluded: HashSet<_> = overrides.exclude_skills.into_iter().collect();
        skills.retain(|s| !excluded.contains(s));
        dedupe(&mut skills);

        let knowledge_policy = overrides.knowledge_policy.unwrap_or(preset.knowledge_policy);
        let knowledge_base_ids = match overrides.knowledge_base_ids {
            Some(ids) => ids,
            None => preset
                .knowledge_bases
                .into_iter()
                .map(|binding| binding.knowledge_base_id)
                .collect(),
        };
        Ok(ResolvedPresetSnapshot {
            preset_id: preset.id, preset_revision: preset.revision, preset_name: preset.name,
            target, routing_description: preset.routing_description, instructions,
            resolved_agent_id, resolved_agent_type, resolved_agent_backend,
            resolved_model, included_skills: skills,
            excluded_auto_skills: preset.excluded_auto_skills, knowledge_policy,
            knowledge_base_ids, warnings,
        })
    }

    async fn resolve_agent(&self, value: &str, required: bool, warnings: &mut Vec<String>) -> Result<String, AppError> {
        validate_agent_reference(value)?;
        if let Some(row) = self.agent_repo.get(value).await? {
            if row.enabled {
                validate_agent_reference(&row.id).map_err(|error| {
                    AppError::Internal(format!("stored agent identity is invalid: {error}"))
                })?;
                return Ok(row.id);
            }
        }
        if let Some(row) = self.agent_repo.find_builtin_by_backend(value).await? {
            if row.enabled {
                validate_agent_reference(&row.id).map_err(|error| {
                    AppError::Internal(format!("stored agent identity is invalid: {error}"))
                })?;
                return Ok(row.id);
            }
        }
        if !required { warnings.push(format!("Agent preference '{value}' is unavailable")); }
        Err(AppError::BadRequest(format!("agent preference '{value}' is unavailable")))
    }

    async fn resolve_model_preference(
        &self,
        preference: &ModelPreference,
        warnings: &mut Vec<String>,
    ) -> Result<ModelPreference, AppError> {
        if let Some(provider_id) = preference.provider_id.as_deref() {
            ProviderId::parse(provider_id).map_err(|error| {
                AppError::BadRequest(format!("invalid model preference provider_id: {error}"))
            })?;
        }
        let providers = self.provider_repo.list().await?;
        let candidates: Vec<_> = providers.into_iter().filter(|p| p.enabled && preference.provider_id.as_ref().is_none_or(|id| id == &p.id)).collect();
        for provider in candidates {
            let models: Vec<String> = serde_json::from_str(&provider.models).unwrap_or_default();
            if models.iter().any(|m| m == &preference.model) {
                if preference.provider_id.is_none() {
                    warnings.push(format!(
                        "Unqualified model '{}' resolved to provider '{}'",
                        preference.model, provider.id
                    ));
                }
                return Ok(ModelPreference {
                    provider_id: Some(provider.id),
                    model: preference.model.clone(),
                    required: preference.required,
                });
            }
        }
        Err(AppError::BadRequest(format!("model '{}' is unavailable", preference.model)))
    }

    pub async fn import(&self, request: ImportPresetsRequest) -> Result<ImportPresetsResult, AppError> {
        let mut result = ImportPresetsResult::default();
        for item in request.presets {
            let id = item.id.clone().unwrap_or_default();
            match self.create(item).await {
                Ok(_) => result.imported += 1,
                Err(AppError::Conflict(_)) => result.skipped += 1,
                Err(error) => { result.failed += 1; result.errors.push(PresetImportError { id, error: error.to_string() }); }
            }
        }
        Ok(result)
    }

    pub async fn list_tags(&self) -> Result<Vec<PresetTagResponse>, AppError> {
        let mut tags: Vec<_> = self.builtin.tags().iter().map(|t| PresetTagResponse {
            key: t.key.clone(), dimension: parse_dimension(&t.dimension), label: t.label.clone(),
            label_i18n: t.label_i18n.clone(), sort_order: t.sort_order, builtin: true,
        }).collect();
        tags.extend(self.tag_repo.list().await?.into_iter().map(|t| PresetTagResponse {
            key: t.key, dimension: parse_dimension(&t.dimension), label: t.label,
            label_i18n: HashMap::new(), sort_order: t.sort_order, builtin: false,
        }));
        Ok(tags)
    }

    pub async fn create_tag(&self, request: CreatePresetTagRequest) -> Result<PresetTagResponse, AppError> {
        let label = request.label.trim();
        if label.is_empty() { return Err(AppError::BadRequest("tag label is required".into())); }
        let key = PresetTagId::new().into_string();
        let dimension = dimension_str(request.dimension);
        let row = self.tag_repo.create(&CreatePresetTagParams { key: &key, dimension, label, sort_order: 0 }).await?;
        Ok(PresetTagResponse { key: row.key, dimension: request.dimension, label: row.label, label_i18n: HashMap::new(), sort_order: row.sort_order, builtin: false })
    }

    pub async fn update_tag(&self, key: &str, request: UpdatePresetTagRequest) -> Result<PresetTagResponse, AppError> {
        if self.builtin.tags().iter().any(|t| t.key == key) { return Err(AppError::Forbidden("Built-in tags cannot be edited".into())); }
        PresetTagId::parse(key)
            .map_err(|error| AppError::BadRequest(format!("invalid user preset tag id: {error}")))?;
        let row = self.tag_repo.update(key, &UpdatePresetTagParams { label: request.label.as_deref(), sort_order: request.sort_order }).await?
            .ok_or_else(|| AppError::NotFound(format!("preset tag '{key}' not found")))?;
        Ok(PresetTagResponse { key: row.key, dimension: parse_dimension(&row.dimension), label: row.label, label_i18n: HashMap::new(), sort_order: row.sort_order, builtin: false })
    }

    pub async fn delete_tag(&self, key: &str) -> Result<(), AppError> {
        if self.builtin.tags().iter().any(|t| t.key == key) { return Err(AppError::Forbidden("Built-in tags cannot be deleted".into())); }
        PresetTagId::parse(key)
            .map_err(|error| AppError::BadRequest(format!("invalid user preset tag id: {error}")))?;
        if !self.tag_repo.delete(key).await? { return Err(AppError::NotFound(format!("preset tag '{key}' not found"))); }
        Ok(())
    }

    pub async fn avatar_asset(&self, id: &str) -> Option<AvatarAsset> {
        match self.classify_source(id).await {
            PresetSource::Builtin => self.builtin.avatar_asset(id),
            PresetSource::Extension => None,
            PresetSource::User => find_asset(&self.user_data_dir.join("preset-avatars"), id),
        }
    }

    fn builtin_response(&self, item: &BuiltinPreset, state: Option<&nomifun_db::PresetUserStateRow>) -> PresetResponse {
        let instructions = self.builtin.rule_bytes(&item.id, "en-US").and_then(|v| String::from_utf8(v).ok()).unwrap_or_default();
        let mut instructions_i18n = HashMap::new();
        for locale in ["zh-CN", "en-US"] {
            if let Some(value) = self.builtin.rule_bytes(&item.id, locale).and_then(|v| String::from_utf8(v).ok()) { instructions_i18n.insert(locale.to_string(), value); }
        }
        let mut response = PresetResponse {
            id: item.id.clone(), revision: 1, source: PresetSource::Builtin, source_key: Some(item.id.clone()),
            name: item.name.clone(), name_i18n: item.name_i18n.clone(), description: item.description.clone(),
            description_i18n: item.description_i18n.clone(), routing_description: item.description.clone(), instructions,
            instructions_i18n, avatar: item.avatar.clone(), fallback_allowed: true, targets: default_targets(),
            agent_preferences: vec![AgentPreference { agent_id: item.preferred_agent_id.clone(), required: false }],
            model_preferences: item.models.iter().map(|m| ModelPreference { provider_id: None, model: m.clone(), required: false }).collect(),
            included_skills: item.enabled_skills.iter().chain(item.custom_skill_names.iter()).map(|s| SkillBinding { skill_name: s.clone(), required: false }).collect(),
            excluded_auto_skills: item.disabled_builtin_skills.clone(), knowledge_policy: Default::default(), knowledge_bases: vec![],
            examples: item.prompts.clone(), examples_i18n: item.prompts_i18n.clone(),
            audience_tags: item.audience_tags.clone(), scenario_tags: item.scenario_tags.clone(),
            enabled: true, auto_selectable: false, preferred_agent_id: None,
            sort_order: 0, last_used_at: None,
        };
        apply_state(&mut response, state); response
    }
}

fn validate_request(
    name: &str,
    agents: &[AgentPreference],
    models: &[ModelPreference],
    policy: &PresetKnowledgePolicy,
    _knowledge_bases: &[KnowledgeBaseBinding],
) -> Result<(), AppError> {
    if name.trim().is_empty() { return Err(AppError::BadRequest("name is required".into())); }
    for agent in agents {
        validate_agent_reference(&agent.agent_id)?;
    }
    if models.iter().any(|m| m.model.trim().is_empty()) { return Err(AppError::BadRequest("model preference requires model".into())); }
    for provider_id in models.iter().filter_map(|model| model.provider_id.as_deref()) {
        ProviderId::parse(provider_id).map_err(|error| {
            AppError::BadRequest(format!("invalid model preference provider_id: {error}"))
        })?;
    }
    if !matches!(policy.mode.as_str(), "inherit" | "staged" | "direct") {
        return Err(AppError::BadRequest("knowledge policy mode must be inherit, staged, or direct".into()));
    }
    if policy.eagerness.as_deref().is_some_and(|value| !matches!(value, "conservative" | "aggressive")) {
        return Err(AppError::BadRequest("knowledge eagerness must be conservative or aggressive".into()));
    }
    Ok(())
}

fn validate_agent_reference(value: &str) -> Result<(), AppError> {
    let stable_catalog_key = !value.is_empty()
        && value.len() <= 255
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':')
        })
        && (!value.starts_with("agent_") || value.starts_with("agent_builtin_"));
    if AgentId::parse(value).is_ok() || stable_catalog_key {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "agent reference must be a canonical custom-agent ID or stable builtin/extension key"
                .into(),
        ))
    }
}

fn record_to_response(record: &PresetRecord) -> Result<PresetResponse, AppError> {
    let p = record.preset.as_ref().ok_or_else(|| AppError::Internal("preset aggregate missing root".into()))?;
    if p.source_kind == "user" {
        PresetId::parse(&p.id).map_err(|error| {
            AppError::Internal(format!("stored user preset id '{}' is not canonical: {error}", p.id))
        })?;
    }
    for preference in &record.model_preferences {
        if let Some(provider_id) = preference.provider_id.as_deref() {
            ProviderId::parse(provider_id).map_err(|error| {
                AppError::Internal(format!(
                    "stored preset model provider_id '{provider_id}' is not canonical: {error}"
                ))
            })?;
        }
    }
    let knowledge_bases = record
        .knowledge_bases
        .iter()
        .map(|binding| {
            KnowledgeBaseId::parse(&binding.knowledge_base_id)
                .map(|knowledge_base_id| KnowledgeBaseBinding {
                    knowledge_base_id,
                    required: binding.required,
                })
                .map_err(|error| {
                    AppError::Internal(format!(
                        "stored preset knowledge_base_id '{}' is not canonical: {error}",
                        binding.knowledge_base_id
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for preference in &record.agent_preferences {
        validate_agent_reference(&preference.agent_id).map_err(|error| {
            AppError::Internal(format!("stored preset agent identity is invalid: {error}"))
        })?;
    }
    if let Some(state) = record.user_state.as_ref()
        && let Some(agent_id) = state.preferred_agent_id.as_deref()
    {
        validate_agent_reference(agent_id).map_err(|error| {
            AppError::Internal(format!(
                "stored preset preferred_agent_id is invalid: {error}"
            ))
        })?;
    }
    let mut name_i18n = HashMap::new(); let mut description_i18n = HashMap::new(); let mut instructions_i18n = HashMap::new();
    for l in &record.localizations {
        if let Some(v) = &l.name { name_i18n.insert(l.locale.clone(), v.clone()); }
        if let Some(v) = &l.description { description_i18n.insert(l.locale.clone(), v.clone()); }
        if let Some(v) = &l.instructions { instructions_i18n.insert(l.locale.clone(), v.clone()); }
    }
    let policy = record.knowledge_policy.as_ref().map(|k| PresetKnowledgePolicy { enabled: k.enabled, mode: k.mode.clone(), writeback: k.writeback, eagerness: k.eagerness.clone(), grounded: k.grounded }).unwrap_or_default();
    let mut response = PresetResponse {
        id: p.id.clone(), revision: p.revision, source: match p.source_kind.as_str() { "builtin" => PresetSource::Builtin, "extension" => PresetSource::Extension, _ => PresetSource::User },
        source_key: p.source_key.clone(), name: p.name.clone(), name_i18n, description: p.description.clone(), description_i18n,
        routing_description: p.routing_description.clone(), instructions: p.instructions.clone(), instructions_i18n, avatar: p.avatar.clone(), fallback_allowed: p.fallback_allowed,
        targets: record.targets.iter().filter_map(|v| parse_target(v)).collect(),
        agent_preferences: record.agent_preferences.iter().map(|v| AgentPreference { agent_id: v.agent_id.clone(), required: v.required }).collect(),
        model_preferences: record.model_preferences.iter().map(|v| ModelPreference { provider_id: v.provider_id.clone(), model: v.model.clone(), required: v.required }).collect(),
        included_skills: record.skill_bindings.iter().filter(|v| v.binding == "include").map(|v| SkillBinding { skill_name: v.skill_name.clone(), required: v.required }).collect(),
        excluded_auto_skills: record.skill_bindings.iter().filter(|v| v.binding == "exclude_auto").map(|v| v.skill_name.clone()).collect(),
        knowledge_policy: policy, knowledge_bases,
        examples: record.examples.iter().filter(|v| v.locale.is_empty()).map(|v| v.prompt.clone()).collect(),
        examples_i18n: collect_examples_i18n(&record.examples),
        audience_tags: record.tag_bindings.iter().filter(|v| v.dimension == "audience").map(|v| v.tag_key.clone()).collect(),
        scenario_tags: record.tag_bindings.iter().filter(|v| v.dimension == "scenario").map(|v| v.tag_key.clone()).collect(),
        enabled: true, auto_selectable: false, preferred_agent_id: None,
        sort_order: 0, last_used_at: None,
    };
    apply_state(&mut response, record.user_state.as_ref()); Ok(response)
}

fn extension_to_response(item: &ResolvedPreset) -> PresetResponse {
    PresetResponse {
        id: item.id.clone(), revision: 1, source: PresetSource::Extension, source_key: Some(format!("{}:{}", item.extension_name, item.id)),
        name: item.name.clone(), name_i18n: HashMap::new(), description: item.description.clone(), description_i18n: HashMap::new(),
        routing_description: item.description.clone(), instructions: item.system_prompt.clone().or_else(|| item.context.clone()).unwrap_or_default(), instructions_i18n: HashMap::new(),
        avatar: item.icon.clone(), fallback_allowed: true, targets: default_targets(),
        agent_preferences: item.preferred_agent_id.iter().map(|v| AgentPreference { agent_id: v.clone(), required: false }).collect(),
        model_preferences: item.models.iter().map(|v| ModelPreference { provider_id: None, model: v.clone(), required: false }).collect(),
        included_skills: item.enabled_skills.iter().map(|v| SkillBinding { skill_name: v.clone(), required: false }).collect(), excluded_auto_skills: vec![],
        knowledge_policy: Default::default(), knowledge_bases: vec![], examples: item.prompts.clone(),
        examples_i18n: HashMap::new(), audience_tags: vec![], scenario_tags: vec![],
        enabled: true, auto_selectable: false, preferred_agent_id: None,
        sort_order: 0, last_used_at: None,
    }
}

fn write_from_create(id: String, r: CreatePresetRequest) -> PresetWriteParams {
    let localizations = collect_localizations(&r.name_i18n, &r.description_i18n, &r.instructions_i18n);
    let examples = flatten_examples(r.examples, r.examples_i18n);
    PresetWriteParams { id: id.clone(), source_kind: "user".into(), source_key: Some(id), name: r.name.trim().into(), description: r.description, routing_description: r.routing_description, instructions: r.instructions, avatar: r.avatar, fallback_allowed: r.fallback_allowed,
        localizations, targets: target_strings(&r.targets), agent_preferences: r.agent_preferences.into_iter().map(|v| (v.agent_id, v.required)).collect(), model_preferences: r.model_preferences.into_iter().map(|v| (v.provider_id, v.model, v.required)).collect(),
        skill_bindings: r.included_skills.into_iter().map(|v| (v.skill_name,"include".into(),v.required)).chain(r.excluded_auto_skills.into_iter().map(|v| (v,"exclude_auto".into(),false))).collect(),
        knowledge_policy: (r.knowledge_policy.enabled,r.knowledge_policy.mode,r.knowledge_policy.writeback,r.knowledge_policy.eagerness,r.knowledge_policy.grounded),
        knowledge_bases: r.knowledge_bases.into_iter().map(|v| (v.knowledge_base_id.to_string(),v.required)).collect(), examples,
        tag_bindings: r.audience_tags.into_iter().map(|v| (v,"audience".into())).chain(r.scenario_tags.into_iter().map(|v| (v,"scenario".into()))).collect() }
}

fn write_from_response(r: PresetResponse) -> PresetWriteParams {
    let localizations = collect_localizations(&r.name_i18n, &r.description_i18n, &r.instructions_i18n);
    let examples = flatten_examples(r.examples, r.examples_i18n);
    PresetWriteParams { id:r.id.clone(),source_kind:"user".into(),source_key:Some(r.id),name:r.name,description:r.description,routing_description:r.routing_description,instructions:r.instructions,avatar:r.avatar,fallback_allowed:r.fallback_allowed,
        localizations,targets:target_strings(&r.targets),agent_preferences:r.agent_preferences.into_iter().map(|v|(v.agent_id,v.required)).collect(),model_preferences:r.model_preferences.into_iter().map(|v|(v.provider_id,v.model,v.required)).collect(),
        skill_bindings:r.included_skills.into_iter().map(|v|(v.skill_name,"include".into(),v.required)).chain(r.excluded_auto_skills.into_iter().map(|v|(v,"exclude_auto".into(),false))).collect(),
        knowledge_policy:(r.knowledge_policy.enabled,r.knowledge_policy.mode,r.knowledge_policy.writeback,r.knowledge_policy.eagerness,r.knowledge_policy.grounded),knowledge_bases:r.knowledge_bases.into_iter().map(|v|(v.knowledge_base_id.to_string(),v.required)).collect(),examples,
        tag_bindings:r.audience_tags.into_iter().map(|v|(v,"audience".into())).chain(r.scenario_tags.into_iter().map(|v|(v,"scenario".into()))).collect() }
}

fn merge_update(mut p: PresetResponse, r: UpdatePresetRequest) -> PresetResponse {
    if let Some(v)=r.name {p.name=v} if r.description.is_some(){p.description=r.description} if r.routing_description.is_some(){p.routing_description=r.routing_description}
    if let Some(v)=r.instructions{p.instructions=v} if r.avatar.is_some(){p.avatar=r.avatar} if let Some(v)=r.fallback_allowed{p.fallback_allowed=v}
    if let Some(v)=r.targets{p.targets=v} if let Some(v)=r.agent_preferences{p.agent_preferences=v} if let Some(v)=r.model_preferences{p.model_preferences=v}
    if let Some(v)=r.included_skills{p.included_skills=v} if let Some(v)=r.excluded_auto_skills{p.excluded_auto_skills=v} if let Some(v)=r.knowledge_policy{p.knowledge_policy=v}
    if let Some(v)=r.knowledge_bases{p.knowledge_bases=v} if let Some(v)=r.examples{p.examples=v} if let Some(v)=r.examples_i18n{p.examples_i18n=v} if let Some(v)=r.audience_tags{p.audience_tags=v} if let Some(v)=r.scenario_tags{p.scenario_tags=v}
    if let Some(v)=r.name_i18n{p.name_i18n=v} if let Some(v)=r.description_i18n{p.description_i18n=v} if let Some(v)=r.instructions_i18n{p.instructions_i18n=v} p
}

fn apply_state(response:&mut PresetResponse,state:Option<&nomifun_db::PresetUserStateRow>){if let Some(s)=state{response.enabled=s.enabled;response.auto_selectable=s.auto_selectable;response.preferred_agent_id=s.preferred_agent_id.clone();response.sort_order=s.sort_order;response.last_used_at=s.last_used_at}}
fn default_targets()->Vec<PresetTarget>{vec![PresetTarget::Conversation,PresetTarget::ExecutionStep,PresetTarget::Companion,PresetTarget::Cron]}
fn target_strings(v:&[PresetTarget])->Vec<String>{v.iter().map(|v|match v{PresetTarget::Conversation=>"conversation",PresetTarget::ExecutionStep=>"execution_step",PresetTarget::Companion=>"companion",PresetTarget::PublicCompanion=>"public_companion",PresetTarget::Cron=>"cron"}.into()).collect()}
fn parse_target(v:&str)->Option<PresetTarget>{match v{"conversation"=>Some(PresetTarget::Conversation),"execution_step"=>Some(PresetTarget::ExecutionStep),"companion"=>Some(PresetTarget::Companion),"public_companion"=>Some(PresetTarget::PublicCompanion),"cron"=>Some(PresetTarget::Cron),_=>None}}
fn dimension_str(v:PresetTagDimension)->&'static str{match v{PresetTagDimension::Audience=>"audience",PresetTagDimension::Scenario=>"scenario"}}
fn parse_dimension(v:&str)->PresetTagDimension{if v=="scenario"{PresetTagDimension::Scenario}else{PresetTagDimension::Audience}}
fn localized_value(map:&HashMap<String,String>,locale:&str)->Option<String>{map.get(locale).cloned().or_else(||map.get(locale.split('-').next().unwrap_or(locale)).cloned())}
fn dedupe(values:&mut Vec<String>){let mut seen=HashSet::new();values.retain(|v|seen.insert(v.clone()))}
fn collect_localizations(names:&HashMap<String,String>,descriptions:&HashMap<String,String>,instructions:&HashMap<String,String>)->Vec<(String,Option<String>,Option<String>,Option<String>,Option<String>)>{let keys:HashSet<_>=names.keys().chain(descriptions.keys()).chain(instructions.keys()).cloned().collect();keys.into_iter().map(|k|(k.clone(),names.get(&k).cloned(),descriptions.get(&k).cloned(),None,instructions.get(&k).cloned())).collect()}
fn collect_examples_i18n(rows:&[nomifun_db::PresetExampleRow])->HashMap<String,Vec<String>>{let mut output=HashMap::new();for row in rows.iter().filter(|row|!row.locale.is_empty()){output.entry(row.locale.clone()).or_insert_with(Vec::new).push(row.prompt.clone());}output}
fn flatten_examples(defaults:Vec<String>,localized:HashMap<String,Vec<String>>)->Vec<(String,String)>{defaults.into_iter().map(|value|(String::new(),value)).chain(localized.into_iter().flat_map(|(locale,values)|values.into_iter().map(move|value|(locale.clone(),value)))).collect()}
fn find_asset(dir:&std::path::Path,id:&str)->Option<AvatarAsset>{for e in std::fs::read_dir(dir).ok()?.flatten(){if e.path().file_stem()?.to_str()?==id{return Some(AvatarAsset{bytes:std::fs::read(e.path()).ok()?,extension:e.path().extension().and_then(|v|v.to_str()).map(str::to_lowercase)})}}None}
fn remove_files_with_stem(dir:&std::path::Path,id:&str){if let Ok(entries)=std::fs::read_dir(dir){for e in entries.flatten(){if e.path().file_stem().and_then(|v|v.to_str())==Some(id){let _=std::fs::remove_file(e.path());}}}}
fn migrate_asset_directories(root:&std::path::Path){for (old,new) in [("assistant-rules","preset-instructions"),("assistant-skills","preset-legacy-skill-notes"),("assistant-avatars","preset-avatars")] {let old=root.join(old);let new=root.join(new);if old.exists()&&!new.exists(){let _=std::fs::rename(old,new);}}}
