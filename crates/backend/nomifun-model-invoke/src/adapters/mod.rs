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
//!
//! Later tasks append the ark / volc adapters to [`default_adapters`].

use std::sync::Arc;

use crate::adapter::ProtocolAdapter;

pub mod deepgram;
pub mod gemini;
pub mod openai_audio;
pub mod openai_chat_text;
pub mod openai_embeddings;
pub mod openai_images;
pub mod openai_videos;

/// Build the standard adapter set registered on the service at assembly time.
/// Adapters are stateless — the shared HTTP client is passed per call by the
/// orchestration layer.
pub fn default_adapters() -> Vec<Arc<dyn ProtocolAdapter>> {
    vec![
        Arc::new(openai_images::OpenAiImagesAdapter),
        Arc::new(openai_videos::OpenAiVideosAdapter),
        Arc::new(openai_chat_text::OpenAiChatTextAdapter),
        Arc::new(openai_embeddings::OpenAiEmbeddingsAdapter),
        Arc::new(openai_audio::OpenAiAudioTranscriptionsAdapter),
        Arc::new(openai_audio::OpenAiAudioSpeechAdapter),
        Arc::new(gemini::GeminiGenerateContentAdapter),
        Arc::new(gemini::GeminiGenerateTextAdapter),
        Arc::new(deepgram::DeepgramListenAdapter),
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
            ("openai.audio_transcriptions", ModelTask::SpeechRecognition),
            ("openai.audio_speech", ModelTask::SpeechSynthesis),
            ("gemini.generate_content", ModelTask::ImageGeneration),
            ("gemini.generate_content", ModelTask::ImageEdit),
            ("gemini.generate_text", ModelTask::Chat),
            ("deepgram.listen", ModelTask::SpeechRecognition),
        ] {
            let adapter = registry.get(protocol, task).expect("registered + supported");
            assert_eq!(adapter.id(), protocol);
        }
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
    }
}
