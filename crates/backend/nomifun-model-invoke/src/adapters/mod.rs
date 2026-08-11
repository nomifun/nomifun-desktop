//! Concrete protocol adapters, keyed by [`crate::adapter::ProtocolAdapter::id`].
//!
//! The OpenAI-compatible family (ported from `nomifun-creation/src/adapters`
//! onto the typed [`crate::types::TaskRequest`] input and declarative
//! [`crate::auth`] schemes):
//! - [`openai_images`] — sync `/images/{generations,edits}` (`"openai.images"`).
//! - [`openai_videos`] — async `/videos` submit→poll→content (`"openai.videos"`).
//! - [`openai_chat_text`] — sync non-streaming `/chat/completions`
//!   (`"openai.chat_text"`).
//! - [`openai_embeddings`] — sync `/embeddings` (`"openai.embeddings"`).
//! - [`openai_audio`] — sync multipart `/audio/transcriptions`
//!   (`"openai.audio_transcriptions"`, ported from `nomifun-shell/stt_openai`)
//!   and sync JSON→binary `/audio/speech` (`"openai.audio_speech"`).
//!
//! Platform-specific protocols:
//! - [`gemini`] — Google `:generateContent` (`"gemini.generate_content"` for
//!   images, `"gemini.generate_text"` for chat).
//! - [`deepgram`] — Deepgram pre-recorded `/v1/listen` (`"deepgram.listen"`).
//! - [`ark`] — Volcengine Ark `/api/v3` image generation (`"ark.images"`) and
//!   async video tasks (`"ark.video_jobs"`).
//! - [`volc_voice`] — Volcengine speech domain (openspeech) file ASR
//!   (`"volc.asr_file"`) and v3 大模型 TTS (`"volc.tts_v3"`), both riding the
//!   `"voice"` connection profile.
//! - [`dashscope`] — Alibaba DashScope forced-async image generation
//!   (`"dashscope.images"`) and sync embeddings (`"dashscope.embeddings"`),
//!   input/parameters wrapper protocol.
//! - [`minimax`] — MiniMax sync TTS (`"minimax.t2a"`, hex audio; optional
//!   legacy GroupId query).
//! - [`mimo`] — MiMo ASR/TTS models serialized over specialized chat
//!   completions (`"mimo.chat_asr"` / `"mimo.chat_tts"`).
//! - [`siliconflow`] — SiliconFlow native JSON image/edit and asynchronous
//!   video submit/status protocols.
//! - [`xai`] — xAI JSON image/edit, deferred video, `/tts`, and `/stt`
//!   protocols.
//! - [`zhipu`] — Zhipu v4 asynchronous video submit/result protocol.

use std::sync::Arc;

use crate::adapter::ProtocolAdapter;

/// Whether the per-model params carry a non-empty `endpoint` override.
///
/// Adapters that compose their own URLs (non-`/v1` conventions: ark, gemini,
/// deepgram) consult this so an explicit `params.endpoint` still wins — routed
/// through [`crate::call::ResolvedCall::dispatch_target`], the single dispatch
/// authority (its rule 1, the zero-code escape hatch).
pub(crate) fn has_endpoint_override(params: &serde_json::Value) -> bool {
    params
        .get("endpoint")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
}

pub mod ark;
pub mod dashscope;
pub mod deepgram;
pub mod gemini;
pub mod generic_rerank;
pub mod minimax;
pub mod mimo;
pub mod openai_audio;
pub mod openai_chat_text;
pub mod openai_embeddings;
pub mod openai_images;
pub mod openai_videos;
pub mod siliconflow;
pub mod volc_voice;
pub mod xai;
pub mod zhipu;

/// Build the standard adapter set registered on the service at assembly time.
/// Adapters are stateless — the shared HTTP client is passed per call by the
/// invoke service layer.
pub fn default_adapters() -> Vec<Arc<dyn ProtocolAdapter>> {
    vec![
        Arc::new(openai_images::OpenAiImagesAdapter),
        Arc::new(openai_videos::OpenAiVideosAdapter),
        Arc::new(openai_chat_text::OpenAiChatTextAdapter),
        Arc::new(openai_embeddings::OpenAiEmbeddingsAdapter),
        Arc::new(generic_rerank::GenericRerankAdapter),
        Arc::new(openai_audio::OpenAiAudioTranscriptionsAdapter),
        Arc::new(openai_audio::OpenAiAudioSpeechAdapter),
        Arc::new(gemini::GeminiGenerateContentAdapter),
        Arc::new(gemini::GeminiGenerateTextAdapter),
        Arc::new(deepgram::DeepgramListenAdapter),
        Arc::new(ark::ArkImagesAdapter),
        Arc::new(ark::ArkVideoJobsAdapter),
        Arc::new(volc_voice::VolcAsrFileAdapter),
        Arc::new(volc_voice::VolcTtsV3Adapter),
        Arc::new(dashscope::DashScopeImagesAdapter),
        Arc::new(dashscope::DashScopeEmbeddingsAdapter),
        Arc::new(minimax::MiniMaxT2aAdapter),
        Arc::new(mimo::MiMoChatAsrAdapter),
        Arc::new(mimo::MiMoChatTtsAdapter),
        Arc::new(siliconflow::SiliconFlowImagesAdapter),
        Arc::new(siliconflow::SiliconFlowVideoJobsAdapter),
        Arc::new(xai::XaiImagesJsonAdapter),
        Arc::new(xai::XaiVideoJobsAdapter),
        Arc::new(xai::XaiTtsAdapter),
        Arc::new(xai::XaiSttAdapter),
        Arc::new(zhipu::ZhipuVideoJobsAdapter),
    ]
}

#[cfg(test)]
pub(crate) mod test_support {
    use serde_json::json;

    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::{ResolvedCall, ResolvedConnection};
    use crate::types::TaskRequest;

    /// A [`ResolvedCall`] against `base_url` (non-full-url, bearer `sk-test`),
    /// as the resolver would produce it for a plain OpenAI-compatible provider.
    pub(crate) fn call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-000000000001".into(),
            platform: "openai".into(),
            model: model.into(),
            task,
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                is_full_url: false,
                auth: AuthMaterial {
                    scheme: AuthScheme::Bearer,
                    credentials: json!({"api_keys": ["sk-test"]}),
                },
                extra: json!({}),
            },
            model_params: json!({}),
            request,
        }
    }
}

#[cfg(test)]
mod tests {
    use nomifun_api_types::ModelTask;

    use super::*;
    use crate::adapter::AdapterRegistry;

    #[test]
    fn default_adapters_register_the_openai_family() {
        let registry = AdapterRegistry::new(default_adapters());
        for (protocol, task) in [
            ("openai.images", ModelTask::ImageGeneration),
            ("openai.images", ModelTask::ImageEdit),
            ("openai.videos", ModelTask::VideoGeneration),
            ("openai.chat_text", ModelTask::Chat),
            ("openai.embeddings", ModelTask::Embedding),
            ("generic.rerank", ModelTask::Rerank),
            ("openai.audio_transcriptions", ModelTask::SpeechRecognition),
            ("openai.audio_speech", ModelTask::SpeechSynthesis),
            ("gemini.generate_content", ModelTask::ImageGeneration),
            ("gemini.generate_content", ModelTask::ImageEdit),
            ("gemini.generate_text", ModelTask::Chat),
            ("deepgram.listen", ModelTask::SpeechRecognition),
            ("ark.images", ModelTask::ImageGeneration),
            ("ark.video_jobs", ModelTask::VideoGeneration),
            ("volc.asr_file", ModelTask::SpeechRecognition),
            ("volc.tts_v3", ModelTask::SpeechSynthesis),
            ("dashscope.images", ModelTask::ImageGeneration),
            ("dashscope.embeddings", ModelTask::Embedding),
            ("minimax.t2a", ModelTask::SpeechSynthesis),
            ("mimo.chat_asr", ModelTask::SpeechRecognition),
            ("mimo.chat_tts", ModelTask::SpeechSynthesis),
            ("siliconflow.images", ModelTask::ImageGeneration),
            ("siliconflow.images", ModelTask::ImageEdit),
            ("siliconflow.video_jobs", ModelTask::VideoGeneration),
            ("xai.images_json", ModelTask::ImageGeneration),
            ("xai.images_json", ModelTask::ImageEdit),
            ("xai.video_jobs", ModelTask::VideoGeneration),
            ("xai.tts", ModelTask::SpeechSynthesis),
            ("xai.stt", ModelTask::SpeechRecognition),
            ("zhipu.video_jobs", ModelTask::VideoGeneration),
        ] {
            let adapter = registry.get(protocol, task).expect("registered + supported");
            assert_eq!(adapter.id(), protocol);
        }
        assert_eq!(default_adapters().len(), 26);
        // Tasks outside an adapter's declared support are refused.
        assert!(registry.get("openai.images", ModelTask::Chat).is_err());
        assert!(registry.get("openai.videos", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("openai.chat_text", ModelTask::Embedding).is_err());
        assert!(registry.get("openai.embeddings", ModelTask::Chat).is_err());
        assert!(registry.get("openai.audio_transcriptions", ModelTask::Chat).is_err());
        assert!(registry.get("openai.audio_speech", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("gemini.generate_content", ModelTask::Chat).is_err());
        assert!(registry.get("gemini.generate_text", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("deepgram.listen", ModelTask::SpeechSynthesis).is_err());
        // ark.images serves ImageGeneration only (ImageEdit stays on openai.images).
        assert!(registry.get("ark.images", ModelTask::ImageEdit).is_err());
        assert!(registry.get("ark.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("volc.asr_file", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("volc.tts_v3", ModelTask::SpeechRecognition).is_err());
        // dashscope.images serves ImageGeneration only (no edit endpoint mapped).
        assert!(registry.get("dashscope.images", ModelTask::ImageEdit).is_err());
        assert!(registry.get("dashscope.embeddings", ModelTask::Chat).is_err());
        assert!(registry.get("minimax.t2a", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("mimo.chat_asr", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("mimo.chat_tts", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("siliconflow.images", ModelTask::Chat).is_err());
        assert!(registry.get("siliconflow.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("xai.images_json", ModelTask::Chat).is_err());
        assert!(registry.get("xai.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("xai.tts", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("xai.stt", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("zhipu.video_jobs", ModelTask::ImageGeneration).is_err());
    }
}
