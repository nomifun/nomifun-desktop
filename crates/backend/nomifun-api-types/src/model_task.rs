//! Unified multimodal capability taxonomy: the authoritative per-model
//! `ModelTask` / `ModelTrait` vocabulary that replaces the two legacy
//! vocabularies (`ModelType` here, `MediaCapability` in nomifun-creation).
//!
//! - [`ModelTask`] is the endpoint-determining "task" a model performs. It is
//!   what the dispatch/probe layer branches on to pick the right HTTP endpoint
//!   and request shape.
//! - [`ModelTrait`] is a within-task refinement (mostly for Chat models):
//!   whether a chat model accepts image input, calls functions, reasons, etc.
//! - [`ModelProfile`] is the authoritative per-model record persisted in the
//!   `model_profiles` table (keyed by `(provider_id, model)`), superseding the
//!   name-only heuristic as the runtime source of truth.
//!
//! [`derive_tasks_and_traits`] seeds a profile from the model name + platform
//! (used for backfill and as the default suggestion for newly-entered models);
//! it is a SEED, not the runtime authority — once a row exists (especially
//! `source = User`) the stored profile wins.

use serde::{Deserialize, Serialize};

use crate::model_capability::{base_model_name, infer_generation_capabilities, infer_model_modalities};
use crate::ModelType;

/// The endpoint-determining task a model performs. Wire values are snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ModelTask {
    /// Text / multimodal chat completions (`/chat/completions`).
    Chat,
    /// Text → image (`/images/generations`).
    ImageGeneration,
    /// Image(+mask)+text → image (`/images/edits`).
    ImageEdit,
    /// Text/image → video (`/videos`).
    VideoGeneration,
    /// Text → speech / TTS (`/audio/speech`).
    SpeechSynthesis,
    /// Speech → text / ASR (`/audio/transcriptions`).
    SpeechRecognition,
    /// Text → vector (`/embeddings`).
    Embedding,
    /// Query+documents → scores (`/rerank`).
    Rerank,
}

/// Within-task refinement of a model's abilities. Mostly modifies [`ModelTask::Chat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ModelTrait {
    /// Chat model accepts image input (vision understanding).
    VisionInput,
    /// Chat model supports tool/function calling.
    FunctionCalling,
    /// Chat model is a reasoning model.
    Reasoning,
    /// Chat model has built-in web search.
    WebSearch,
}

/// Provenance of a [`ModelProfile`]. User-authored profiles override inferred values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    /// Auto-derived from the model name/platform heuristic.
    #[default]
    Inferred,
    /// Explicitly set by the user in the UI (authoritative).
    User,
}

/// The authoritative per-model capability record. Identity is `(provider_id, model)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProfile {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub tasks: Vec<ModelTask>,
    pub traits: Vec<ModelTrait>,
    /// Free-form service config (image size/steps, tts voice, asr language,
    /// endpoint/request-shape overrides, timeout, …). See [`crate::dispatch_target`].
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub source: ProfileSource,
    pub updated_at: i64,
}

impl ModelProfile {
    /// The primary task used when a caller (e.g. the health probe) needs a
    /// single task and none was specified. Prefers the first declared task;
    /// falls back to [`ModelTask::Chat`].
    pub fn primary_task(&self) -> ModelTask {
        self.tasks.first().copied().unwrap_or(ModelTask::Chat)
    }
}

/// Request body for `POST /api/model-profiles` (upsert one profile).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileUpsertRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default)]
    pub tasks: Vec<ModelTask>,
    #[serde(default)]
    pub traits: Vec<ModelTrait>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    /// Defaults to `User` when omitted (this endpoint is the user-edit path).
    #[serde(default)]
    pub source: Option<ProfileSource>,
}

/// Body identifying a single profile (`POST /api/model-profiles/delete`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfileKeyRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
}

// --- Name/platform substring seeds (extend the model_capability.rs heuristic) ---

/// Substrings implying text-to-speech. `whisper` is excluded (that's ASR).
const TTS_INCLUDE: &[&str] = &["tts", "text-to-speech", "cosyvoice", "-voice", "speech-0", "sovits"];
/// Substrings implying speech recognition / transcription.
const ASR_INCLUDE: &[&str] =
    &["whisper", "asr", "transcrib", "speech-to-text", "sensevoice", "paraformer", "nova-2", "nova-3"];
/// Substrings implying embedding models.
const EMBEDDING_INCLUDE: &[&str] = &["embed", "text-embedding", "bge-", "gte-", "-e5-"];
/// Substrings implying rerank models.
const RERANK_INCLUDE: &[&str] = &["rerank"];
/// Substrings implying image editing (in addition to image generation).
const IMAGE_EDIT_INCLUDE: &[&str] = &["edit", "inpaint"];

fn push_unique(tasks: &mut Vec<ModelTask>, task: ModelTask) {
    if !tasks.contains(&task) {
        tasks.push(task);
    }
}

/// Provider/model combinations whose official task is known and whose name is
/// either ambiguous or actively misleading to generic substring inference.
/// Keep this table intentionally small: live provider catalogs decide which
/// model IDs are available, while this function only supplies their task
/// metadata to the provider -> modality -> model picker.
fn verified_provider_profile(
    platform: &str,
    model: &str,
) -> Option<(Vec<ModelTask>, Vec<ModelTrait>)> {
    use ModelTask::*;
    let base = base_model_name(model);

    match platform {
        "mimo" | "mimo-token-plan-cn" | "mimo-token-plan-sgp" | "mimo-token-plan-ams" => {
            match base.as_str() {
                "mimo-v2.5-pro" | "mimo-v2.5-pro-ultraspeed" => Some((vec![Chat], vec![])),
                "mimo-v2.5" => Some((vec![Chat], vec![ModelTrait::VisionInput])),
                "mimo-v2.5-asr" => Some((vec![SpeechRecognition], vec![])),
                "mimo-v2.5-tts" | "mimo-v2.5-tts-voicedesign" | "mimo-v2.5-tts-voiceclone" => {
                    Some((vec![SpeechSynthesis], vec![]))
                }
                _ => None,
            }
        }
        "minimax" | "minimax-code" | "minimax-coding-plan" => match base.as_str() {
            "minimax-m3" => Some((vec![Chat], vec![ModelTrait::VisionInput])),
            "minimax-m2.7" | "minimax-m2.7-highspeed" => Some((vec![Chat], vec![])),
            "minimax-h3" | "minimax-hailuo-2.3" | "minimax-hailuo-2.3-fast"
            | "minimax-hailuo-02" => Some((vec![VideoGeneration], vec![])),
            "image-01" | "image-01-live" => Some((vec![ImageGeneration], vec![])),
            "speech-2.8-hd" | "speech-2.8-turbo" => Some((vec![SpeechSynthesis], vec![])),
            _ => None,
        },
        "openai" => match base.as_str() {
            "gpt-image-2" | "gpt-image-1" | "gpt-image-1.5" | "gpt-image-1-mini"
            | "chatgpt-image-latest" => Some((vec![ImageGeneration, ImageEdit], vec![])),
            "dall-e-2" => Some((vec![ImageGeneration, ImageEdit], vec![])),
            "dall-e-3" => Some((vec![ImageGeneration], vec![])),
            _ if base.starts_with("sora-2") => Some((vec![VideoGeneration], vec![])),
            _ => None,
        },
        "xai" => match base.as_str() {
            "xai-tts" => Some((vec![SpeechSynthesis], vec![])),
            "xai-stt" => Some((vec![SpeechRecognition], vec![])),
            "grok-imagine-image" | "grok-imagine-image-quality" => {
                Some((vec![ImageGeneration, ImageEdit], vec![]))
            }
            "grok-imagine-video" | "grok-imagine-video-1.5" => {
                Some((vec![VideoGeneration], vec![]))
            }
            _ => None,
        },
        "stepfun" | "stepfun-plan" => match base.as_str() {
            "stepaudio-2.5-asr" => Some((vec![SpeechRecognition], vec![])),
            "stepaudio-2.5-tts" => Some((vec![SpeechSynthesis], vec![])),
            "step-image-edit-2" => Some((vec![ImageGeneration, ImageEdit], vec![])),
            _ => None,
        },
        "gemini" => match base.as_str() {
            "gemini-3.1-flash-image" | "gemini-3.1-flash-lite-image"
            | "gemini-3-pro-image" | "gemini-2.5-flash-image" => {
                Some((vec![ImageGeneration, ImageEdit], vec![]))
            }
            _ => None,
        },
        "zhipu" => match base.as_str() {
            "glm-image" | "cogview-4-250304" | "cogview-4" | "cogview-3-flash" => {
                Some((vec![ImageGeneration], vec![]))
            }
            "cogvideox-3" | "cogvideox-2" | "cogvideox-flash" => {
                Some((vec![VideoGeneration], vec![]))
            }
            "glm-asr-2512" => Some((vec![SpeechRecognition], vec![])),
            "glm-tts" => Some((vec![SpeechSynthesis], vec![])),
            "embedding-3" | "embedding-2" => Some((vec![Embedding], vec![])),
            "rerank" => Some((vec![Rerank], vec![])),
            "glm-5v-turbo" | "glm-4.6v" | "autoglm-phone" | "glm-4.6v-flash"
            | "glm-4.6v-flashx" | "glm-4v-flash" | "glm-4.1v-thinking-flashx"
            | "glm-4.1v-thinking-flash" => {
                Some((vec![Chat], vec![ModelTrait::VisionInput]))
            }
            // GLM-4-Voice is an audio-input/output chat model, not ordinary
            // speech synthesis; the current task taxonomy records Chat only.
            "glm-4-voice" => Some((vec![Chat], vec![])),
            _ => None,
        },
        "moonshot-cn" | "moonshot-global" => match base.as_str() {
            "kimi-k3" | "kimi-k2.7-code" | "kimi-k2.7-code-highspeed" | "kimi-k2.6"
            | "kimi-k2.5" => Some((vec![Chat], vec![ModelTrait::VisionInput])),
            _ if base.contains("vision-preview") => {
                Some((vec![Chat], vec![ModelTrait::VisionInput]))
            }
            _ => None,
        },
        "lingyi" if base == "yi-vision-v2" => {
            Some((vec![Chat], vec![ModelTrait::VisionInput]))
        }
        "hunyuan" | "hunyuan-global" => match base.as_str() {
            "hy-vision-2.0-instruct" | "hunyuan-t1-vision-20250916"
            | "hunyuan-turbos-vision-video-20250728" | "youtu-vita" => {
                Some((vec![Chat], vec![ModelTrait::VisionInput]))
            }
            "kinfra-text-embedding-0.6b" | "kinfra-text-embedding-4b" => {
                Some((vec![Embedding], vec![]))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Seed a model's `(tasks, traits)` from its platform + name.
///
/// Platform acts as a first-class authority where it is unambiguous. Otherwise
/// the model name drives the classification. A model that matches no
/// specialized (image/video/audio/embedding/rerank) signal is treated as a
/// Chat model.
pub fn derive_tasks_and_traits(platform: &str, model: &str) -> (Vec<ModelTask>, Vec<ModelTrait>) {
    if let Some(profile) = verified_provider_profile(platform, model) {
        return profile;
    }

    let base = base_model_name(model);
    let mut tasks: Vec<ModelTask> = Vec::new();
    let mut traits: Vec<ModelTrait> = Vec::new();

    // 1. Generation capabilities from the existing name heuristic.
    for cap in infer_generation_capabilities(model) {
        match cap {
            ModelType::ImageGeneration => push_unique(&mut tasks, ModelTask::ImageGeneration),
            ModelType::VideoGeneration => push_unique(&mut tasks, ModelTask::VideoGeneration),
            _ => {}
        }
    }

    // 2. Broader image signal: an "image" model id that the family list missed.
    if base.contains("image") {
        push_unique(&mut tasks, ModelTask::ImageGeneration);
    }
    // 3. Image editing signal (only meaningful for image models).
    if !tasks.is_empty()
        && (tasks.contains(&ModelTask::ImageGeneration))
        && IMAGE_EDIT_INCLUDE.iter().any(|k| base.contains(k))
    {
        push_unique(&mut tasks, ModelTask::ImageEdit);
    }

    // 4. Audio / embedding / rerank (mutually exclusive families, checked in priority order).
    if RERANK_INCLUDE.iter().any(|k| base.contains(k)) {
        push_unique(&mut tasks, ModelTask::Rerank);
    } else if EMBEDDING_INCLUDE.iter().any(|k| base.contains(k)) {
        push_unique(&mut tasks, ModelTask::Embedding);
    } else if ASR_INCLUDE.iter().any(|k| base.contains(k)) {
        push_unique(&mut tasks, ModelTask::SpeechRecognition);
    } else if TTS_INCLUDE.iter().any(|k| base.contains(k)) {
        push_unique(&mut tasks, ModelTask::SpeechSynthesis);
    }

    // 5. Vision-input trait (a vision model is a Chat model that accepts images).
    if infer_model_modalities(model).iter().any(|m| m == "vision") {
        traits.push(ModelTrait::VisionInput);
    }

    // 6. Default: no specialized task means it is a Chat model.
    if tasks.is_empty() {
        tasks.push(ModelTask::Chat);
    }

    (tasks, traits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks_of(platform: &str, model: &str) -> Vec<ModelTask> {
        derive_tasks_and_traits(platform, model).0
    }

    #[test]
    fn chat_model_is_chat() {
        assert_eq!(tasks_of("openai", "gpt-4o-mini"), vec![ModelTask::Chat]);
        assert_eq!(tasks_of("deepseek", "deepseek-chat"), vec![ModelTask::Chat]);
    }

    #[test]
    fn vision_chat_model_has_chat_task_and_vision_trait() {
        let (tasks, traits) = derive_tasks_and_traits("openai", "gpt-4o");
        assert_eq!(tasks, vec![ModelTask::Chat]);
        assert!(traits.contains(&ModelTrait::VisionInput));
    }

    #[test]
    fn verified_mimo_audio_models_use_chat_wire_protocol_but_audio_tasks() {
        assert_eq!(tasks_of("mimo", "mimo-v2.5-asr"), vec![ModelTask::SpeechRecognition]);
        assert_eq!(tasks_of("mimo", "mimo-v2.5-tts"), vec![ModelTask::SpeechSynthesis]);
        assert_eq!(tasks_of("mimo", "mimo-v2.5-tts-voiceclone"), vec![ModelTask::SpeechSynthesis]);
    }

    #[test]
    fn mimo_pro_is_not_mistagged_as_vision_by_family_substring() {
        let (_, pro_traits) = derive_tasks_and_traits("mimo", "mimo-v2.5-pro");
        let (_, omni_traits) = derive_tasks_and_traits("mimo", "mimo-v2.5");
        assert!(!pro_traits.contains(&ModelTrait::VisionInput));
        assert!(omni_traits.contains(&ModelTrait::VisionInput));
    }

    #[test]
    fn gpt_image_models_declare_generation_and_editing() {
        assert_eq!(
            tasks_of("openai", "gpt-image-2"),
            vec![ModelTask::ImageGeneration, ModelTask::ImageEdit]
        );
    }

    #[test]
    fn current_minimax_media_models_do_not_fall_back_to_chat() {
        assert_eq!(tasks_of("minimax", "MiniMax-H3"), vec![ModelTask::VideoGeneration]);
        assert_eq!(tasks_of("minimax", "speech-2.8-hd"), vec![ModelTask::SpeechSynthesis]);
    }

    #[test]
    fn verified_multi_task_image_models_are_filterable_in_both_picker_modes() {
        for (platform, model) in [
            ("gemini", "gemini-3.1-flash-image"),
            ("stepfun-plan", "step-image-edit-2"),
            ("xai", "grok-imagine-image"),
        ] {
            let tasks = tasks_of(platform, model);
            assert!(tasks.contains(&ModelTask::ImageGeneration), "{platform}/{model}");
            assert!(tasks.contains(&ModelTask::ImageEdit), "{platform}/{model}");
        }
    }

    #[test]
    fn stepfun_plan_router_is_chat() {
        // Step Plan is a subscription chat-completions gateway. Treating the
        // whole platform as image-only made its router/chat models impossible
        // to select for conversations.
        assert_eq!(tasks_of("stepfun-plan", "step-router-v1"), vec![ModelTask::Chat]);
    }

    #[test]
    fn dall_e_is_image_generation() {
        assert!(tasks_of("openai", "dall-e-3").contains(&ModelTask::ImageGeneration));
    }

    #[test]
    fn whisper_is_speech_recognition_not_tts() {
        let tasks = tasks_of("openai", "whisper-1");
        assert!(tasks.contains(&ModelTask::SpeechRecognition));
        assert!(!tasks.contains(&ModelTask::SpeechSynthesis));
        assert!(!tasks.contains(&ModelTask::Chat));
    }

    #[test]
    fn tts_is_speech_synthesis() {
        assert!(tasks_of("openai", "gpt-4o-mini-tts").contains(&ModelTask::SpeechSynthesis));
        assert!(tasks_of("stepfun", "step-tts-mini").contains(&ModelTask::SpeechSynthesis));
        assert!(tasks_of("stepfun", "stepaudio-2.5-tts").contains(&ModelTask::SpeechSynthesis));
    }

    #[test]
    fn stepfun_speech_models_classify_so_the_robot_voice_slots_resolve() {
        // The robot's ASR/TTS slots only accept models the catalog classifies as
        // speech_recognition / speech_synthesis; a mis-typed row is rejected by
        // the invoke task gate and the device stays silent. See 2026-08-08 spec.
        for asr in ["stepaudio-2.5-asr", "step-asr"] {
            let tasks = tasks_of("stepfun", asr);
            assert!(tasks.contains(&ModelTask::SpeechRecognition), "{asr} must be ASR");
            assert!(!tasks.contains(&ModelTask::Chat), "{asr} must not fall back to chat");
        }
        for tts in ["stepaudio-2.5-tts", "step-tts-mini"] {
            assert!(
                tasks_of("stepfun", tts).contains(&ModelTask::SpeechSynthesis),
                "{tts} must be TTS"
            );
        }
    }

    #[test]
    fn stepfun_vision_models_carry_the_vision_trait() {
        for m in ["step-1v-32k", "step-1v-8k", "step-1o-turbo-vision", "step-3.7-flash"] {
            let (tasks, traits) = derive_tasks_and_traits("stepfun", m);
            assert_eq!(tasks, vec![ModelTask::Chat], "{m} is a vision-capable chat model");
            assert!(traits.contains(&ModelTrait::VisionInput), "{m} must accept image input");
        }
        // The text-only flagship must NOT be tagged vision by the `step-3.7` rule.
        let (_, traits) = derive_tasks_and_traits("stepfun", "step-3.5-flash");
        assert!(!traits.contains(&ModelTrait::VisionInput));
    }

    #[test]
    fn embedding_and_rerank() {
        assert!(tasks_of("openai", "text-embedding-3-large").contains(&ModelTask::Embedding));
        assert!(tasks_of("jina", "bge-reranker-v2").contains(&ModelTask::Rerank));
    }

    #[test]
    fn video_generation() {
        assert!(tasks_of("openai", "sora-2").contains(&ModelTask::VideoGeneration));
    }

    #[test]
    fn primary_task_prefers_first() {
        let p = ModelProfile {
            provider_id: "018f1234-5678-7abc-8def-012345678990".into(),
            model: "m".into(),
            tasks: vec![ModelTask::ImageGeneration, ModelTask::ImageEdit],
            traits: vec![],
            params: serde_json::Value::Null,
            source: ProfileSource::User,
            updated_at: 0,
        };
        assert_eq!(p.primary_task(), ModelTask::ImageGeneration);
        let empty = ModelProfile { tasks: vec![], ..p };
        assert_eq!(empty.primary_task(), ModelTask::Chat);
    }

    #[test]
    fn wire_format_is_snake_case() {
        assert_eq!(serde_json::to_string(&ModelTask::ImageGeneration).unwrap(), "\"image_generation\"");
        assert_eq!(serde_json::to_string(&ModelTask::SpeechRecognition).unwrap(), "\"speech_recognition\"");
        assert_eq!(serde_json::to_string(&ModelTrait::VisionInput).unwrap(), "\"vision_input\"");
        assert_eq!(serde_json::to_string(&ProfileSource::Inferred).unwrap(), "\"inferred\"");
    }

    #[test]
    fn model_profile_upsert_rejects_noncanonical_provider_id() {
        let raw = serde_json::json!({
            "provider_id": "openai",
            "model": "gpt-5",
            "tasks": ["chat"]
        });
        assert!(serde_json::from_value::<ModelProfileUpsertRequest>(raw).is_err());
    }
}
