//! Configuration recommendations from the current vendor-document audit.
//! Runtime dispatch uses only each saved capability's explicit protocol.
use nomifun_api_types::ModelTask;
use crate::provider_protocol_audit::{provider_protocol_audit, DocumentationStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRoute {
    pub protocol: &'static str,
    pub connection_role: Option<&'static str>,
}

/// No model-name guesses or cross-modality fallback. Model-specific selection
/// further restricts this platform-level hint in the configuration manifest.
pub fn preset_protocol_recommendation(platform: &str, task: ModelTask) -> Option<TaskRoute> {
    if matches!(platform, "custom" | "new-api" | "nomifun-model-gateway") { return None; }
    let audit = provider_protocol_audit(platform)?;
    let entry = audit.tasks.iter().find(|entry| entry.task == task)?;
    if !matches!(entry.status, DocumentationStatus::Supported | DocumentationStatus::CatalogRequired) { return None; }
    if task == ModelTask::Chat {
        return crate::model_profiles::recommended_chat_protocol(platform, None)
            .map(|protocol| TaskRoute {protocol, connection_role:entry.connection_role.as_deref()});
    }
    entry.protocols.iter().find(|protocol| crate::protocol_task_descriptor(protocol, task).is_some())
        .map(|protocol| TaskRoute {protocol:protocol.as_str(), connection_role:entry.connection_role.as_deref()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use ModelTask::*;
    #[test]
    fn retired_and_unimplemented_contracts_are_never_recommended() {
        for (platform, task) in [("openai", VideoGeneration), ("stepfun", ImageGeneration),
            ("stepfun-plan", ImageEdit), ("ark", SpeechRecognition), ("ark", SpeechSynthesis),
            ("zhipu", ImageGeneration), ("lingyi", Chat), ("vertex", Chat)] {
            assert_eq!(preset_protocol_recommendation(platform,task),None,"{platform} {task:?}");
        }
    }
    #[test]
    fn recommendations_reference_an_implemented_task_contract() {
        for preset in crate::platform_presets() {
            for task in crate::ALL_MODEL_TASKS {
                if let Some(route) = preset_protocol_recommendation(&preset.platform,task) {
                    assert!(crate::protocol_task_descriptor(route.protocol,task).is_some());
                }
            }
        }
    }
    #[test]
    fn native_speech_contracts_do_not_inherit_openai_multipart() {
        for (platform,expected) in [("minimax","minimax.speech_to_text"),
            ("siliconflow","siliconflow.audio_transcriptions"),("zhipu","zhipu.audio_transcriptions"),
            ("mimo-token-plan-cn","mimo.chat_asr"),("mimo-token-plan-sgp","mimo.chat_asr"),
            ("mimo-token-plan-ams","mimo.chat_asr")] {
            assert_eq!(preset_protocol_recommendation(platform,SpeechRecognition).unwrap().protocol,expected);
        }
    }
    #[test]
    fn custom_gateways_need_explicit_model_configuration() {
        for platform in ["custom","new-api","nomifun-model-gateway","typo-provider"] {
            for task in crate::ALL_MODEL_TASKS { assert_eq!(preset_protocol_recommendation(platform,task),None); }
        }
    }
}
