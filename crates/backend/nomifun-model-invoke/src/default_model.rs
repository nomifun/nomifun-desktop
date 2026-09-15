//! Exact default model selection for automatic media tools. Explicit defaults
//! never fall back to another model when disabled or invalid.

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

fn choose_model(default: Option<ModelRef>, mut candidates: Vec<ModelRef>) -> Result<ModelRef, InvokeError> {
    if let Some(selected) = default {
        if candidates.iter().any(|candidate| candidate.provider_id == selected.provider_id && candidate.model == selected.model) {
            return Ok(selected);
        }
        return Err(InvokeError::config("The configured default model is unavailable for this task; update its default in model settings"));
    }
    if candidates.len() == 1 { return Ok(candidates.remove(0)); }
    Err(InvokeError::config(if candidates.is_empty() {
        "No enabled model supports this generation task; configure one in model settings"
    } else {
        "Several models support this generation task; choose its exact default in model settings"
    }))
}

impl ModelInvokeService {
    /// Current locally invocable task models. No network request or credential
    /// material is returned; callers may expose these exact identities to a model.
    pub async fn available_task_models(&self, task: ModelTask) -> Result<Vec<ModelRef>, InvokeError> {
        let capabilities = self.provider_model_capability_repo.list().await.map_err(|error| InvokeError::config(error.to_string()))?;
        let task_key = serde_json::to_value(task).expect("ModelTask is serializable");
        let mut candidates = Vec::new();
        for capability in capabilities {
            if Some(capability.task.as_str()) != task_key.as_str()
                || capability.health.as_deref().and_then(|value| serde_json::from_str::<CapabilityHealth>(value).ok())
                    .is_some_and(|health| health.status == HealthStatus::Unhealthy) { continue; }
            let model = ModelRef { provider_id: capability.provider_id, model: capability.model };
            match self.validate(&model, task).await {
                Ok(()) => candidates.push(model),
                Err(error) if error.is_catalog_failure() => return Err(error),
                Err(_) => {}
            }
        }
        Ok(candidates)
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

    pub async fn resolve_default_task_model(
        &self,
        task: ModelTask,
        preferences: &dyn IClientPreferenceRepository,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        let default = self.configured_task_default(task, Some(preferences)).await?;
        let candidates = self.available_task_models(task).await?;
        let selected = choose_model(default, candidates)?;
        self.resolve_task_config(&selected, task).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(name: &str) -> ModelRef { ModelRef { provider_id: "provider".into(), model: name.into() } }
    #[test]
    fn defaults_are_exact_and_ambiguous_inventory_never_chooses_by_order() {
        assert_eq!(choose_model(None, vec![model("only")]).unwrap().model, "only");
        assert!(choose_model(None, vec![model("first"), model("second")]).is_err());
        assert!(choose_model(Some(model("removed")), vec![model("only")]).is_err());
        assert_eq!(choose_model(Some(model("second")), vec![model("first"), model("second")]).unwrap().model, "second");
    }
    #[test]
    fn music_and_speech_have_distinct_defaults() {
        assert_ne!(default_model_preference_key(ModelTask::MusicGeneration), default_model_preference_key(ModelTask::SpeechSynthesis));
    }
}
