//! Resolves a model pool and reusable presets into immutable, execution-scoped
//! Agent participant snapshots.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nomifun_api_types::{
    ExecutionModelPool, ExecutionModelRef, ModelTask, ModelTrait, ParticipantCapability,
    PresetOverrides, PresetTarget, ResolvedPresetSnapshot,
};
use nomifun_common::{
    AppError, MAX_AGENT_EXECUTION_MODELS, MAX_AGENT_EXECUTION_PARTICIPANTS, ProviderId,
    NOMI_AGENT_ID,
};
#[cfg(test)]
use nomifun_common::generate_id;
use nomifun_db::models::Provider;
use nomifun_db::{
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    NewAgentExecutionParticipant, ProviderModelCapabilityRow, ProviderModelRow,
};
use nomifun_preset::PresetService;

#[derive(Debug, Clone)]
struct ChatCatalogEntry {
    description: Option<String>,
    traits: ChatTraitProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatTraitProjection {
    modalities: Vec<String>,
    function_calling: bool,
    reasoning: bool,
    web_search: bool,
}

impl ChatTraitProjection {
    fn apply_to(&self, capability: &mut ParticipantCapability) {
        capability.modalities.clone_from(&self.modalities);
        capability.tools = self.function_calling;
        capability.reasoning = if self.reasoning { "high" } else { "low" }.to_owned();
        capability.web_search = self.web_search;
    }
}

fn project_chat_traits(
    provider_id: &str,
    model: &str,
    traits_json: &str,
) -> Result<ChatTraitProjection, AppError> {
    let traits: Vec<ModelTrait> = serde_json::from_str(traits_json).map_err(|error| {
        AppError::Internal(format!(
            "stored Chat capability traits for {provider_id}/{model} are invalid: {error}"
        ))
    })?;
    let mut modalities = Vec::new();
    let mut function_calling = false;
    let mut reasoning = false;
    let mut web_search = false;
    for model_trait in traits {
        let modality = match model_trait {
            ModelTrait::VisionInput => Some("vision"),
            ModelTrait::VideoInput => Some("video"),
            ModelTrait::AudioInput => Some("audio_input"),
            ModelTrait::AudioOutput => Some("audio_output"),
            ModelTrait::Realtime => Some("realtime"),
            ModelTrait::Streaming => Some("streaming"),
            ModelTrait::FunctionCalling => {
                function_calling = true;
                None
            }
            ModelTrait::Reasoning => {
                reasoning = true;
                None
            }
            ModelTrait::WebSearch => {
                web_search = true;
                None
            }
        };
        if let Some(modality) = modality
            && !modalities.iter().any(|value| value == modality)
        {
            modalities.push(modality.to_owned());
        }
    }
    Ok(ChatTraitProjection {
        modalities,
        function_calling,
        reasoning,
        web_search,
    })
}

fn model_task_wire(task: ModelTask) -> Result<String, AppError> {
    serde_json::to_value(task)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| AppError::Internal("failed to serialize model task".to_owned()))
}

fn build_chat_catalog(
    providers: &[Provider],
    model_rows: &[ProviderModelRow],
    capability_rows: &[ProviderModelCapabilityRow],
) -> Result<
    (
        HashMap<(String, String), ChatCatalogEntry>,
        Vec<ExecutionModelRef>,
    ),
    AppError,
> {
    let mut rows_by_provider: HashMap<&str, Vec<&ProviderModelRow>> = HashMap::new();
    for row in model_rows {
        rows_by_provider
            .entry(row.provider_id.as_str())
            .or_default()
            .push(row);
    }
    let chat_task = model_task_wire(ModelTask::Chat)?;
    let mut chat_capabilities: HashMap<(&str, &str), &ProviderModelCapabilityRow> = HashMap::new();
    for capability in capability_rows
        .iter()
        .filter(|capability| capability.task == chat_task)
    {
        if chat_capabilities
            .insert(
                (capability.provider_id.as_str(), capability.model.as_str()),
                capability,
            )
            .is_some()
        {
            return Err(AppError::Internal(format!(
                "duplicate persisted Chat capability for {}/{}",
                capability.provider_id, capability.model
            )));
        }
    }

    let mut catalog = HashMap::new();
    let mut catalog_order = Vec::new();
    for provider in providers.iter().filter(|provider| provider.enabled) {
        if ProviderId::try_from(provider.provider_id.as_str()).is_err() {
            return Err(AppError::Internal(
                "enabled provider has a non-canonical persisted id".to_owned(),
            ));
        }
        let Some(provider_rows) = rows_by_provider.get(provider.provider_id.as_str()) else {
            continue;
        };
        for row in provider_rows {
            let model = row.model.trim().to_owned();
            if model.is_empty() || model != row.model {
                return Err(AppError::Internal(format!(
                    "provider {} has an invalid persisted model id",
                    provider.provider_id
                )));
            }
            if !row.enabled {
                continue;
            }
            let Some(chat_capability) =
                chat_capabilities.get(&(provider.provider_id.as_str(), row.model.as_str()))
            else {
                continue;
            };
            let key = (provider.provider_id.clone(), model);
            let entry = ChatCatalogEntry {
                description: row.description.clone(),
                traits: project_chat_traits(&key.0, &key.1, &chat_capability.traits)?,
            };
            if catalog.insert(key.clone(), entry).is_none() {
                catalog_order.push(ExecutionModelRef {
                    provider_id: key.0,
                    model: key.1,
                });
            }
        }
    }
    Ok((catalog, catalog_order))
}

#[derive(Clone)]
pub(crate) struct ParticipantResolver {
    provider_repo: Arc<dyn IProviderRepository>,
    provider_model_repo: Arc<dyn IProviderModelRepository>,
    provider_model_capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
    preset_service: Arc<PresetService>,
}

impl ParticipantResolver {
    pub fn new(
        provider_repo: Arc<dyn IProviderRepository>,
        provider_model_repo: Arc<dyn IProviderModelRepository>,
        provider_model_capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
        preset_service: Arc<PresetService>,
    ) -> Self {
        Self {
            provider_repo,
            provider_model_repo,
            provider_model_capability_repo,
            preset_service,
        }
    }

    pub async fn resolve(
        &self,
        pool: &ExecutionModelPool,
        lead_model: Option<&ExecutionModelRef>,
    ) -> Result<Vec<NewAgentExecutionParticipant>, AppError> {
        pool.validate().map_err(AppError::BadRequest)?;
        let providers = self
            .provider_repo
            .list()
            .await
            .map_err(|error| AppError::Internal(format!("list model providers: {error}")))?;
        // Identity/display ordering lives on provider_models; executable Agent
        // membership additionally requires one exact persisted Chat capability.
        let model_rows = self
            .provider_model_repo
            .list()
            .await
            .map_err(|error| AppError::Internal(format!("list provider models: {error}")))?;
        let capability_rows = self
            .provider_model_capability_repo
            .list()
            .await
            .map_err(|error| {
                AppError::Internal(format!("list provider model capabilities: {error}"))
            })?;
        let (catalog, catalog_order) =
            build_chat_catalog(&providers, &model_rows, &capability_rows)?;

        let requested = match pool {
            ExecutionModelPool::Single { model } => vec![model.clone()],
            ExecutionModelPool::Automatic => {
                let mut models = Vec::with_capacity(MAX_AGENT_EXECUTION_MODELS);
                if let Some(lead) = lead_model {
                    models.push(lead.clone());
                }
                models.extend(
                    catalog_order
                        .iter()
                        .filter(|candidate| Some(*candidate) != lead_model)
                        .take(MAX_AGENT_EXECUTION_MODELS.saturating_sub(models.len()))
                        .cloned(),
                );
                models
            }
            ExecutionModelPool::Range { models } => {
                if models.len() > MAX_AGENT_EXECUTION_MODELS {
                    return Err(AppError::BadRequest(format!(
                        "execution model range exceeds {MAX_AGENT_EXECUTION_MODELS} models"
                    )));
                }
                models.clone()
            }
        };
        let mut seen = HashSet::new();
        let mut models = Vec::new();
        for model in requested {
            let key = (model.provider_id, model.model);
            if !catalog.contains_key(&key) {
                return Err(AppError::BadRequest(format!(
                    "model {}/{} is missing, disabled, or has no Chat capability",
                    key.0, key.1
                )));
            }
            if seen.insert(key.clone()) {
                models.push(ExecutionModelRef {
                    provider_id: key.0,
                    model: key.1,
                });
            }
        }
        if models.is_empty() {
            return Err(AppError::ProviderUnavailable(
                "no enabled provider/model with a Chat capability can participate in this execution"
                    .to_owned(),
            ));
        }

        if let Some(lead) = lead_model {
            let Some(index) = models.iter().position(|model| model == lead) else {
                return Err(AppError::BadRequest(
                    "lead_model must belong to the resolved execution model pool".to_owned(),
                ));
            };
            models.swap(0, index);
        }

        let mut snapshots = Vec::new();
        for model in &models {
            let entry = catalog
                .get(&(model.provider_id.clone(), model.model.clone()))
                .expect("resolved model must remain in the immutable Chat catalog");
            let description = entry
                .description
                .clone()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            let mut capability = derive_capability(&[], &[], None);
            entry.traits.apply_to(&mut capability);
            snapshots.push(NewAgentExecutionParticipant {
                participant_id:
                    nomifun_common::AgentExecutionParticipantId::new().into_string(),
                source_agent_id: NOMI_AGENT_ID.to_owned(),
                preset_id: None,
                preset_revision: None,
                preset_snapshot: None,
                provider_id: Some(model.provider_id.clone()),
                model: Some(model.model.clone()),
                role: None,
                capability: Some(
                    serde_json::to_string(&capability)
                    .map_err(|error| AppError::Internal(format!("encode capability: {error}")))?,
                ),
                constraints: None,
                description,
                system_prompt: None,
                enabled_skills: "[]".to_owned(),
                disabled_builtin_skills: "[]".to_owned(),
                sort_order: snapshots.len() as i64,
            });
        }

        // Presets enrich routing but never widen the caller's model pool.
        let mut presets = match self.preset_service.list().await {
            Ok(presets) => presets,
            Err(error) => {
                tracing::warn!(%error, "participant resolution continuing without presets");
                return Ok(snapshots);
            }
        };
        presets.sort_by(|left, right| left.preset_id.cmp(&right.preset_id));
        for preset in presets
            .into_iter()
            .filter(|preset| preset.enabled && preset.auto_selectable)
        {
            if snapshots.len() >= MAX_AGENT_EXECUTION_PARTICIPANTS {
                tracing::warn!(
                    limit = MAX_AGENT_EXECUTION_PARTICIPANTS,
                    "execution participant budget reached; remaining automatic presets were not materialized"
                );
                break;
            }
            let resolved = match self
                .preset_service
                .resolve(
                    &preset.preset_id,
                    PresetTarget::ExecutionStep,
                    None,
                    PresetOverrides::default(),
                )
                .await
            {
                Ok(resolved) => resolved,
                Err(error) => {
                    tracing::warn!(preset_id = %preset.preset_id, %error, "skipping unresolved execution preset");
                    continue;
                }
            };
            let Some(resolved_model) = resolved.resolved_model.as_ref() else {
                continue;
            };
            let pair = models.iter().find(|candidate| {
                candidate.model == resolved_model.model
                    && resolved_model
                        .provider_id
                        .as_ref()
                        .is_none_or(|expected| expected == &candidate.provider_id)
            });
            let Some(pair) = pair else {
                continue;
            };
            let provider_id = pair.provider_id.clone();
            let model = pair.model.clone();
            let description = resolved
                .routing_description
                .clone()
                .or(preset.description.clone());
            let mut capability = derive_capability(
                &preset.audience_tags,
                &preset.scenario_tags,
                description.as_deref(),
            );
            catalog
                .get(&(provider_id.clone(), model.clone()))
                .expect("preset model must remain in the immutable Chat catalog")
                .traits
                .apply_to(&mut capability);
            snapshots.push(NewAgentExecutionParticipant {
                participant_id:
                    nomifun_common::AgentExecutionParticipantId::new().into_string(),
                source_agent_id: resolved
                    .resolved_agent_id
                    .clone()
                    .unwrap_or_else(|| NOMI_AGENT_ID.to_owned()),
                preset_id: Some(preset.preset_id),
                preset_revision: Some(resolved.preset_revision),
                preset_snapshot: Some(serde_json::to_string(&resolved).map_err(|error| {
                    AppError::Internal(format!("encode preset snapshot: {error}"))
                })?),
                provider_id: Some(provider_id),
                model: Some(model),
                role: Some(preset.name),
                capability: Some(serde_json::to_string(&capability).map_err(|error| {
                    AppError::Internal(format!("encode participant capability: {error}"))
                })?),
                constraints: None,
                description,
                system_prompt: (!resolved.instructions.trim().is_empty())
                    .then_some(resolved.instructions.clone()),
                enabled_skills: serde_json::to_string(&resolved.included_skills).map_err(
                    |error| AppError::Internal(format!("encode participant skills: {error}")),
                )?,
                disabled_builtin_skills: serde_json::to_string(&resolved.excluded_auto_skills)
                    .map_err(|error| {
                        AppError::Internal(format!("encode participant exclusions: {error}"))
                    })?,
                sort_order: snapshots.len() as i64,
            });
        }
        Ok(snapshots)
    }

    /// Preserve the authenticated caller's frozen preset as the first Agent
    /// participant without widening the already-resolved model authority.
    pub(crate) fn prepend_frozen_lead(
        participants: &mut Vec<NewAgentExecutionParticipant>,
        snapshot: &ResolvedPresetSnapshot,
        lead_model: Option<&ExecutionModelRef>,
    ) -> Result<(), AppError> {
        if let Some(index) = participants.iter().position(|participant| {
            participant.preset_id.as_deref() == Some(snapshot.preset_id.as_str())
                && participant.preset_revision == Some(snapshot.preset_revision)
                && lead_model.is_none_or(|lead| {
                    participant.provider_id.as_deref() == Some(lead.provider_id.as_str())
                        && participant.model.as_deref() == Some(lead.model.as_str())
                })
        }) {
            let participant = participants.remove(index);
            participants.insert(0, participant);
            for (index, participant) in participants.iter_mut().enumerate() {
                participant.sort_order = index as i64;
            }
            return Ok(());
        }
        let model = lead_model
            .cloned()
            .or_else(|| {
                let resolved = snapshot.resolved_model.as_ref()?;
                Some(ExecutionModelRef {
                    provider_id: resolved.provider_id.clone()?,
                    model: resolved.model.clone(),
                })
            })
            .or_else(|| {
                participants.iter().find_map(|participant| {
                    Some(ExecutionModelRef {
                        provider_id: participant.provider_id.clone()?,
                        model: participant.model.clone()?,
                    })
                })
            })
            .ok_or_else(|| {
                AppError::BadRequest(
                    "the calling Agent preset has no model inside the execution authority"
                        .to_owned(),
                )
            })?;
        ExecutionModelPool::Single {
            model: model.clone(),
        }
        .validate()
        .map_err(AppError::BadRequest)?;
        let matching_model_index = participants.iter().position(|participant| {
            participant.provider_id.as_deref() == Some(model.provider_id.as_str())
                && participant.model.as_deref() == Some(model.model.as_str())
        });
        let Some(matching_model_index) = matching_model_index else {
            return Err(AppError::BadRequest(format!(
                "the calling Agent model {}/{} is outside the execution model pool",
                model.provider_id, model.model
            )));
        };

        // The authenticated frozen Agent is the concrete lead identity for
        // this model. Replace the first matching template/base participant at
        // every size so participant count and model authority never widen.
        let inherited_model_capability = participants[matching_model_index]
            .capability
            .as_deref()
            .map(|raw| {
                serde_json::from_str::<ParticipantCapability>(raw).map_err(|error| {
                    AppError::Internal(format!(
                        "decode matching participant capability for frozen preset: {error}"
                    ))
                })
            })
            .transpose()?;
        participants.remove(matching_model_index);

        let mut lead_capability = derive_capability(
            &[],
            &[],
            snapshot.routing_description.as_deref(),
        );
        if let Some(inherited) = inherited_model_capability.as_ref() {
            copy_model_trait_capability(inherited, &mut lead_capability);
        }

        for participant in participants.iter_mut() {
            participant.sort_order += 1;
        }
        participants.insert(
            0,
            NewAgentExecutionParticipant {
                participant_id:
                    nomifun_common::AgentExecutionParticipantId::new().into_string(),
                source_agent_id: snapshot
                    .resolved_agent_id
                    .clone()
                    .unwrap_or_else(|| NOMI_AGENT_ID.to_owned()),
                preset_id: Some(snapshot.preset_id.clone()),
                preset_revision: Some(snapshot.preset_revision),
                preset_snapshot: Some(serde_json::to_string(snapshot).map_err(|error| {
                    AppError::Internal(format!("encode calling Agent preset snapshot: {error}"))
                })?),
                provider_id: Some(model.provider_id),
                model: Some(model.model),
                role: Some(snapshot.preset_name.clone()),
                capability: Some(
                    serde_json::to_string(&lead_capability)
                    .map_err(|error| {
                        AppError::Internal(format!("encode calling Agent capability: {error}"))
                    })?,
                ),
                constraints: None,
                description: snapshot.routing_description.clone(),
                system_prompt: (!snapshot.instructions.trim().is_empty())
                    .then(|| snapshot.instructions.clone()),
                enabled_skills: serde_json::to_string(&snapshot.included_skills).map_err(
                    |error| {
                        AppError::Internal(format!("encode calling Agent skills: {error}"))
                    },
                )?,
                disabled_builtin_skills: serde_json::to_string(
                    &snapshot.excluded_auto_skills,
                )
                .map_err(|error| {
                    AppError::Internal(format!(
                        "encode calling Agent builtin exclusions: {error}"
                    ))
                })?,
                sort_order: 0,
            },
        );
        Ok(())
    }

    /// Make an explicitly selected calling model the deterministic planner
    /// lead of a template without adding a participant or widening authority.
    pub(crate) fn promote_lead_model(
        &self,
        participants: &mut Vec<NewAgentExecutionParticipant>,
        lead_model: &ExecutionModelRef,
    ) -> Result<(), AppError> {
        promote_model_to_front(participants, lead_model)
    }
}

fn promote_model_to_front(
    participants: &mut Vec<NewAgentExecutionParticipant>,
    lead_model: &ExecutionModelRef,
) -> Result<(), AppError> {
    ExecutionModelPool::Single {
        model: lead_model.clone(),
    }
    .validate()
    .map_err(AppError::BadRequest)?;
    let index = participants
        .iter()
        .position(|participant| {
            participant.provider_id.as_deref() == Some(lead_model.provider_id.as_str())
                && participant.model.as_deref() == Some(lead_model.model.as_str())
        })
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "the calling Agent model {}/{} is not present in the selected collaboration template",
                lead_model.provider_id, lead_model.model
            ))
        })?;
    let participant = participants.remove(index);
    participants.insert(0, participant);
    for (index, participant) in participants.iter_mut().enumerate() {
        participant.sort_order = index as i64;
    }
    Ok(())
}

fn derive_capability(
    audience_tags: &[String],
    scenario_tags: &[String],
    description: Option<&str>,
) -> ParticipantCapability {
    const KEYWORDS: &[(&str, &str)] = &[
        ("cod", "coding"),
        ("program", "coding"),
        ("develop", "coding"),
        ("writ", "writing"),
        ("文案", "writing"),
        ("research", "research"),
        ("调研", "research"),
        ("search", "research"),
        ("analy", "analysis"),
        ("分析", "analysis"),
        ("design", "design"),
        ("设计", "design"),
        ("translat", "translation"),
        ("翻译", "translation"),
        ("plan", "planning"),
        ("规划", "planning"),
    ];
    let mut inputs: Vec<String> = audience_tags
        .iter()
        .chain(scenario_tags)
        .map(|value| value.to_lowercase())
        .collect();
    if let Some(description) = description {
        inputs.push(description.to_lowercase());
    }
    let mut strengths = Vec::new();
    for (needle, strength) in KEYWORDS {
        if inputs.iter().any(|value| value.contains(needle))
            && !strengths.iter().any(|value| value == strength)
        {
            strengths.push((*strength).to_owned());
        }
    }
    ParticipantCapability {
        strengths,
        modalities: vec![],
        tools: false,
        web_search: false,
        reasoning: "low".to_owned(),
        cost_tier: "standard".to_owned(),
        speed_tier: "standard".to_owned(),
    }
}

fn copy_model_trait_capability(
    source: &ParticipantCapability,
    target: &mut ParticipantCapability,
) {
    target.modalities.clone_from(&source.modalities);
    target.tools = source.tools;
    target.web_search = source.web_search;
    target.reasoning.clone_from(&source.reasoning);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{PresetKnowledgePolicy, PresetTarget};

    const PROVIDER_1: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PROVIDER_2: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const LEAD_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000010";
    const OUTSIDE_PROVIDER: &str = "0190f5fe-7c00-7a00-8000-000000000099";
    const NOMI_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";
    const LEAD_PRESET_ID: &str = "0190f5fe-7c00-7a00-8000-000000000115";

    fn participant(
        participant_id: &str,
        provider_id: &str,
        model: &str,
        sort_order: i64,
    ) -> NewAgentExecutionParticipant {
        NewAgentExecutionParticipant {
            participant_id: participant_id.to_owned(),
            source_agent_id: NOMI_AGENT_ID.to_owned(),
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            provider_id: Some(provider_id.to_owned()),
            model: Some(model.to_owned()),
            role: None,
            capability: None,
            constraints: None,
            description: None,
            system_prompt: None,
            enabled_skills: "[]".to_owned(),
            disabled_builtin_skills: "[]".to_owned(),
            sort_order,
        }
    }

    fn snapshot() -> ResolvedPresetSnapshot {
        ResolvedPresetSnapshot {
            preset_id: LEAD_PRESET_ID.to_owned(),
            preset_revision: 7,
            preset_name: "Lead".to_owned(),
            target: PresetTarget::ExecutionStep,
            routing_description: None,
            instructions: "lead instructions".to_owned(),
            resolved_agent_id: Some(NOMI_AGENT_ID.to_owned()),
            resolved_agent_type: None,
            resolved_agent_backend: None,
            resolved_model: None,
            included_skills: vec![],
            excluded_auto_skills: vec![],
            knowledge_policy: PresetKnowledgePolicy::default(),
            knowledge_base_ids: vec![],
            warnings: vec![],
        }
    }

    fn provider(provider_id: &str, enabled: bool) -> Provider {
        Provider {
            id: 1,
            provider_id: provider_id.to_owned(),
            platform: "test".to_owned(),
            name: "Test".to_owned(),
            base_url: "https://example.invalid".to_owned(),
            auth_scheme: "bearer".to_owned(),
            credentials_encrypted: nomifun_common::encrypt_string(
                r#"{"api_keys":["test-only"]}"#,
                &[0x42; 32],
            )
            .unwrap(),
            enabled,
            bedrock_config: None,
            sort_order: 0,
            config_revision: 1,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn model_row(provider_id: &str, model: &str, enabled: bool) -> ProviderModelRow {
        ProviderModelRow {
            id: 1,
            provider_id: provider_id.to_owned(),
            model: model.to_owned(),
            enabled,
            sort_order: 0,
            description: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn capability_row(
        provider_id: &str,
        model: &str,
        task: &str,
        traits: &str,
    ) -> ProviderModelCapabilityRow {
        ProviderModelCapabilityRow {
            id: 1,
            provider_id: provider_id.to_owned(),
            model: model.to_owned(),
            task: task.to_owned(),
            traits: traits.to_owned(),
            protocol: "test.protocol".to_owned(),
            connection_role: "default".to_owned(),
            base_url_override: None,
            endpoint: None,
            poll_endpoint: None,
            content_endpoint: None,
            realtime_endpoint: None,
            allow_cross_origin_credentials: false,
            provider_params: "{}".to_owned(),
            context_limit: None,
            health: None,
            health_checked_at: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn catalog_requires_enabled_provider_model_and_exact_chat_capability() {
        let providers = vec![provider(PROVIDER_1, true), provider(PROVIDER_2, false)];
        let models = vec![
            model_row(PROVIDER_1, "chat", true),
            model_row(PROVIDER_1, "image-only", true),
            model_row(PROVIDER_1, "disabled-chat", false),
            model_row(PROVIDER_2, "disabled-provider-chat", true),
        ];
        let capabilities = vec![
            capability_row(PROVIDER_1, "chat", "chat", "[]"),
            capability_row(PROVIDER_1, "image-only", "image_generation", "[]"),
            capability_row(PROVIDER_1, "disabled-chat", "chat", "[]"),
            capability_row(PROVIDER_2, "disabled-provider-chat", "chat", "[]"),
        ];

        let (catalog, order) = build_chat_catalog(&providers, &models, &capabilities).unwrap();

        assert_eq!(catalog.len(), 1);
        assert!(catalog.contains_key(&(PROVIDER_1.to_owned(), "chat".to_owned())));
        assert_eq!(
            order,
            vec![ExecutionModelRef {
                provider_id: PROVIDER_1.to_owned(),
                model: "chat".to_owned(),
            }]
        );
    }

    #[test]
    fn participant_capability_comes_only_from_persisted_chat_traits() {
        let providers = vec![provider(PROVIDER_1, true)];
        let models = vec![
            model_row(PROVIDER_1, "gpt-4o-vision-looking-name", true),
            model_row(PROVIDER_1, "opaque-model", true),
        ];
        let capabilities = vec![
            capability_row(
                PROVIDER_1,
                "gpt-4o-vision-looking-name",
                "chat",
                r#"["function_calling","reasoning","web_search","video_input"]"#,
            ),
            capability_row(
                PROVIDER_1,
                "opaque-model",
                "chat",
                r#"["vision_input","video_input","audio_input","audio_output","realtime","streaming","function_calling","reasoning","web_search"]"#,
            ),
        ];

        let (catalog, _) = build_chat_catalog(&providers, &models, &capabilities).unwrap();

        let named_projection = &catalog
            [&(PROVIDER_1.to_owned(), "gpt-4o-vision-looking-name".to_owned())]
            .traits;
        assert_eq!(named_projection.modalities, ["video"]);
        assert!(!named_projection.modalities.iter().any(|value| value == "vision"));
        let mut named_capability = derive_capability(&[], &[], None);
        named_projection.apply_to(&mut named_capability);
        assert!(named_capability.tools);
        assert!(named_capability.web_search);
        assert_eq!(named_capability.reasoning, "high");

        let opaque_projection =
            &catalog[&(PROVIDER_1.to_owned(), "opaque-model".to_owned())].traits;
        assert_eq!(
            opaque_projection.modalities,
            [
                "vision",
                "video",
                "audio_input",
                "audio_output",
                "realtime",
                "streaming",
            ]
        );
        let mut opaque_capability = derive_capability(&[], &[], None);
        opaque_projection.apply_to(&mut opaque_capability);
        assert!(opaque_capability.tools);
        assert!(opaque_capability.web_search);
        assert_eq!(opaque_capability.reasoning, "high");

        let no_traits = project_chat_traits(PROVIDER_1, "plain", "[]").unwrap();
        let mut plain_capability = derive_capability(&[], &[], None);
        no_traits.apply_to(&mut plain_capability);
        assert!(plain_capability.modalities.is_empty());
        assert!(!plain_capability.tools);
        assert!(!plain_capability.web_search);
        assert_eq!(plain_capability.reasoning, "low");
    }

    #[test]
    fn malformed_persisted_chat_traits_fail_closed() {
        let error = build_chat_catalog(
            &[provider(PROVIDER_1, true)],
            &[model_row(PROVIDER_1, "broken", true)],
            &[capability_row(PROVIDER_1, "broken", "chat", "not-json")],
        )
        .unwrap_err();

        assert!(matches!(error, AppError::Internal(_)));
        assert!(error.to_string().contains("traits"));
    }

    #[test]
    fn explicit_template_lead_is_promoted_without_widening_authority() {
        let mut participants = vec![
            participant("0190f5fe-7c00-7a00-8000-000000000101", PROVIDER_1, "m1", 0),
            participant("0190f5fe-7c00-7a00-8000-000000000102", PROVIDER_2, "m2", 1),
        ];
        promote_model_to_front(
            &mut participants,
            &ExecutionModelRef {
                provider_id: PROVIDER_2.to_owned(),
                model: "m2".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(participants.len(), 2);
        assert_eq!(
            participants[0].participant_id,
            "0190f5fe-7c00-7a00-8000-000000000102"
        );
        assert_eq!(participants[0].sort_order, 0);
        assert_eq!(participants[1].sort_order, 1);
    }

    #[test]
    fn explicit_template_lead_must_belong_to_the_template() {
        let mut participants = vec![participant(
            "0190f5fe-7c00-7a00-8000-000000000101",
            PROVIDER_1,
            "m1",
            0,
        )];
        let error = promote_model_to_front(
            &mut participants,
            &ExecutionModelRef {
                provider_id: OUTSIDE_PROVIDER.to_owned(),
                model: "m2".to_owned(),
            },
        )
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)));
        assert_eq!(participants.len(), 1);
    }

    #[test]
    fn frozen_lead_replaces_a_matching_model_at_every_template_size() {
        for size in [2_usize, MAX_AGENT_EXECUTION_PARTICIPANTS] {
            let mut participants = vec![
                participant(
                    "0190f5fe-7c00-7a00-8000-000000000101",
                    PROVIDER_2,
                    "m2",
                    0,
                ),
                participant(
                    "0190f5fe-7c00-7a00-8000-000000000102",
                    LEAD_PROVIDER,
                    "lead-model",
                    1,
                ),
            ];
            participants[1].capability = Some(
                serde_json::to_string(&ParticipantCapability {
                    strengths: vec![],
                    modalities: vec!["vision".to_owned()],
                    tools: true,
                    web_search: true,
                    reasoning: "high".to_owned(),
                    cost_tier: "standard".to_owned(),
                    speed_tier: "standard".to_owned(),
                })
                .unwrap(),
            );
            while participants.len() < size {
                let index = participants.len();
                participants.push(participant(
                    &generate_id(),
                    PROVIDER_2,
                    &format!("model-{index}"),
                    index as i64,
                ));
            }

            ParticipantResolver::prepend_frozen_lead(
                &mut participants,
                &snapshot(),
                Some(&ExecutionModelRef {
                    provider_id: LEAD_PROVIDER.to_owned(),
                    model: "lead-model".to_owned(),
                }),
            )
            .unwrap();

            assert_eq!(participants.len(), size);
            assert_eq!(
                participants[0].preset_id.as_deref(),
                Some(LEAD_PRESET_ID)
            );
            assert_eq!(participants[0].sort_order, 0);
            let frozen_capability: ParticipantCapability =
                serde_json::from_str(participants[0].capability.as_deref().unwrap()).unwrap();
            assert_eq!(frozen_capability.modalities, ["vision"]);
            assert!(frozen_capability.tools);
            assert!(frozen_capability.web_search);
            assert_eq!(frozen_capability.reasoning, "high");
            assert!(!participants.iter().any(|participant| {
                participant.participant_id == "0190f5fe-7c00-7a00-8000-000000000102"
            }));
        }
    }

    #[test]
    fn frozen_lead_replaces_the_first_same_model_participant_deterministically() {
        let mut participants = vec![
            participant(
                "0190f5fe-7c00-7a00-8000-000000000101",
                LEAD_PROVIDER,
                "lead-model",
                0,
            ),
            participant(
                "0190f5fe-7c00-7a00-8000-000000000102",
                LEAD_PROVIDER,
                "lead-model",
                1,
            ),
            participant(
                "0190f5fe-7c00-7a00-8000-000000000103",
                PROVIDER_2,
                "m2",
                2,
            ),
        ];
        ParticipantResolver::prepend_frozen_lead(
            &mut participants,
            &snapshot(),
            Some(&ExecutionModelRef {
                provider_id: LEAD_PROVIDER.to_owned(),
                model: "lead-model".to_owned(),
            }),
        )
        .unwrap();

        assert_eq!(participants.len(), 3);
        assert_eq!(
            participants[0].preset_id.as_deref(),
            Some(LEAD_PRESET_ID)
        );
        assert!(!participants.iter().any(|participant| {
            participant.participant_id == "0190f5fe-7c00-7a00-8000-000000000101"
        }));
        assert!(participants.iter().any(|participant| {
            participant.participant_id == "0190f5fe-7c00-7a00-8000-000000000102"
        }));
    }
}
