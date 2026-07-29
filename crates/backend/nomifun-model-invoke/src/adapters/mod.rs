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
//!
//! Later tasks append the gemini / deepgram / ark / volc adapters to
//! [`default_adapters`].

use std::sync::Arc;

use crate::adapter::ProtocolAdapter;

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
        ] {
            let adapter = registry.get(protocol, task).expect("registered + supported");
            assert_eq!(adapter.id(), protocol);
        }
        // Tasks outside an adapter's declared support are refused.
        assert!(registry.get("openai.images", ModelTask::Chat).is_err());
        assert!(registry.get("openai.videos", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("openai.chat_text", ModelTask::Embedding).is_err());
        assert!(registry.get("openai.embeddings", ModelTask::Chat).is_err());
    }
}
