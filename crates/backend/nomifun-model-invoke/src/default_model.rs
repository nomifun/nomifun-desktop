//! Automatic media model selection. A configured default stays exact; when
//! none exists, the host picks the highest-ranked compatible model.

use std::collections::HashMap;

use nomifun_api_types::{CapabilityHealth, HealthStatus, ModelTask};
use nomifun_db::IClientPreferenceRepository;

use crate::{InvokeError, ModelInvokeService, ModelRef, ResolvedTaskConfig};

pub fn default_model_preference_key(task: ModelTask) -> Option<&'static str> {
    Some(match task {
        ModelTask::ImageGeneration => "models.default.imageGeneration",
        ModelTask::ImageEdit => "models.default.imageEdit",
        ModelTask::VideoGeneration => "models.default.videoGeneration",
        ModelTask::MusicGeneration => "models.default.musicGeneration",
        ModelTask::SpeechSynthesis => "models.default.speechSynthesis",
        ModelTask::Chat => "nomi.defaultModel",
        _ => return None,
    })
}

fn model_management_link(task: ModelTask) -> &'static str {
    match task {
        ModelTask::ImageGeneration => "nomifun://model-management/image",
        ModelTask::ImageEdit => "nomifun://model-management/image-edit",
        ModelTask::VideoGeneration => "nomifun://model-management/video",
        ModelTask::MusicGeneration => "nomifun://model-management/music",
        ModelTask::SpeechSynthesis => "nomifun://model-management/tts",
        ModelTask::Chat => "nomifun://model-management/chat",
        _ => "nomifun://model-management/models",
    }
}

fn choose_model(task: ModelTask, default: Option<ModelRef>, candidates: Vec<ModelRef>) -> Result<ModelRef, InvokeError> {
    let link = model_management_link(task);
    if let Some(selected) = default {
        if candidates.iter().any(|candidate| candidate.provider_id == selected.provider_id && candidate.model == selected.model) {
            return Ok(selected);
        }
        return Err(InvokeError::config(format!("The configured default model is unavailable for this task; update its default in [Model Management]({link})")));
    }
    candidates.into_iter().next().ok_or_else(|| InvokeError::config(
        format!("No enabled model supports this generation task; configure one in [Model Management]({link})")
    ))
}

impl ModelInvokeService {
    /// Current locally invocable task models, ranked for automatic selection:
    /// healthy before untested, then provider priority and model priority.
    /// No network request or credential material is returned.
    pub async fn available_task_models(&self, task: ModelTask) -> Result<Vec<ModelRef>, InvokeError> {
        let capabilities = self.provider_model_capability_repo.list().await.map_err(|error| InvokeError::config(error.to_string()))?;
        let task_key = serde_json::to_value(task).expect("ModelTask is serializable");
        let mut candidates = Vec::new();
        for capability in capabilities {
            if Some(capability.task.as_str()) != task_key.as_str() { continue; }
            let health = capability.health.as_deref()
                .and_then(|value| serde_json::from_str::<CapabilityHealth>(value).ok())
                .map(|health| health.status)
                .unwrap_or(HealthStatus::Unknown);
            if health == HealthStatus::Unhealthy { continue; }
            let model = ModelRef { provider_id: capability.provider_id, model: capability.model };
            match self.validate(&model, task).await {
                Ok(()) => candidates.push((health, model)),
                Err(error) if error.is_catalog_failure() => return Err(error),
                Err(_) => {}
            }
        }
        if candidates.len() > 1 {
            let providers = self.provider_repo.list().await.map_err(|error| InvokeError::config(error.to_string()))?;
            let models = self.provider_model_repo.list().await.map_err(|error| InvokeError::config(error.to_string()))?;
            let provider_rank = providers.into_iter().enumerate()
                .map(|(rank, provider)| (provider.provider_id, rank))
                .collect::<HashMap<_, _>>();
            let model_rank = models.into_iter().enumerate()
                .map(|(rank, model)| ((model.provider_id, model.model), rank))
                .collect::<HashMap<_, _>>();
            candidates.sort_by_key(|(health, model)| (
                if *health == HealthStatus::Healthy { 0 } else { 1 },
                provider_rank.get(&model.provider_id).copied().unwrap_or(usize::MAX),
                model_rank.get(&(model.provider_id.clone(), model.model.clone())).copied().unwrap_or(usize::MAX),
                model.provider_id.clone(),
                model.model.clone(),
            ));
        }
        Ok(candidates.into_iter().map(|(_, model)| model).collect())
    }

    pub async fn configured_task_default(
        &self, task: ModelTask, preferences: Option<&dyn IClientPreferenceRepository>,
    ) -> Result<Option<ModelRef>, InvokeError> {
        let key = default_model_preference_key(task).ok_or_else(|| InvokeError::config("This task has no automatic generation default"))?;
        let Some(preferences) = preferences else { return Ok(None); };
        let saved = preferences.get_by_keys(&[key]).await
            .map_err(|error| InvokeError::config(format!("Cannot read generation default: {error}")))?;
        saved.iter().find(|row| row.key == key).map(|row| {
            let value: nomifun_api_types::AgentChatModelSelectionDto = serde_json::from_str(&row.value)
                .map_err(|_| InvokeError::config("The saved generation default is malformed"))?;
            if value.provider_id.trim().is_empty() || value.provider_id.trim() != value.provider_id || value.model.trim().is_empty() || value.model.trim() != value.model {
                return Err(InvokeError::config("The saved generation default is malformed"));
            }
            Ok(ModelRef { provider_id: value.provider_id, model: value.model })
        }).transpose()
    }

    pub async fn resolve_automatic_task_model(
        &self,
        task: ModelTask,
        preferences: &dyn IClientPreferenceRepository,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        let default = self.configured_task_default(task, Some(preferences)).await?;
        let candidates = self.available_task_models(task).await?;
        let selected = choose_model(task, default, candidates)?;
        self.resolve_task_config(&selected, task).await
    }

    /// Retained for callers using the previous method name. Missing defaults
    /// now follow the same ranked automatic route as `resolve_automatic_task_model`.
    pub async fn resolve_default_task_model(
        &self,
        task: ModelTask,
        preferences: &dyn IClientPreferenceRepository,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        self.resolve_automatic_task_model(task, preferences).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(name: &str) -> ModelRef { ModelRef { provider_id: "provider".into(), model: name.into() } }
    #[test]
    fn automatic_route_uses_ranked_candidates_only_without_a_default() {
        assert_eq!(choose_model(ModelTask::ImageGeneration, None, vec![model("only")]).unwrap().model, "only");
        assert_eq!(choose_model(ModelTask::ImageGeneration, None, vec![model("first"), model("second")]).unwrap().model, "first");
        assert!(choose_model(ModelTask::ImageGeneration, Some(model("removed")), vec![model("only")]).is_err());
        assert_eq!(choose_model(ModelTask::ImageGeneration, Some(model("second")), vec![model("first"), model("second")]).unwrap().model, "second");
    }

    #[test]
    fn unavailable_automatic_routes_include_the_model_management_link() {
        let error = choose_model(ModelTask::VideoGeneration, None, vec![])
            .unwrap_err();
        assert!(error.message.contains("nomifun://model-management/video"));
    }
    #[test]
    fn music_and_speech_have_distinct_defaults() {
        assert_ne!(default_model_preference_key(ModelTask::MusicGeneration), default_model_preference_key(ModelTask::SpeechSynthesis));
    }
}
