//! Concrete protocol adapters, keyed by [`crate::adapter::ProtocolAdapter::id`].
//!
//! The OpenAI-compatible family (ported from `nomifun-creation/src/adapters`
//! onto the typed [`crate::types::TaskRequest`] input and declarative
//! [`crate::auth`] schemes):
//! - [`openai_images`] — sync `/images/{generations,edits}` (`"openai.images"`).
//! - [`openai_videos`] — async `/videos` submit→poll→content (`"openai.videos"`).
//! - [`openai_embeddings`] — sync `/embeddings` (`"openai.embeddings"`).
//! - [`openai_audio`] — sync multipart `/audio/transcriptions`
//!   (`"openai.audio_transcriptions"`, ported from `nomifun-shell/stt_openai`)
//!   and sync JSON→binary `/audio/speech` (`"openai.audio_speech"`).
//!
//! Platform-specific protocols:
//! - [`gemini`] — Google `:generateContent` images
//!   (`"gemini.generate_content"`). Chat protocols execute through the agent
//!   stack and are intentionally absent from this request-adapter registry.
//! - [`deepgram`] — Deepgram pre-recorded `/v1/listen` (`"deepgram.listen"`)
//!   and REST `/v1/speak` (`"deepgram.speak_rest"`).
//! - [`ark`] — Volcengine Ark `/api/v3` image generation (`"ark.images"`) and
//!   async video tasks (`"ark.video_jobs"`).
//! - [`volc_voice`] — Volcengine speech domain (openspeech) file ASR
//!   (`"volc.asr_file"`) and v3 大模型 TTS (`"volc.tts_v3"`), both riding the
//!   `"voice"` connection profile.
//! - [`dashscope`] — Alibaba DashScope forced-async image generation
//!   (`"dashscope.images"`) and sync embeddings (`"dashscope.embeddings"`),
//!   input/parameters wrapper protocol.
//! - [`minimax`] — MiniMax sync TTS (`"minimax.t2a"`, hex audio).
//! - [`mimo`] — MiMo ASR/TTS models serialized over specialized chat
//!   completions (`"mimo.chat_asr"` / `"mimo.chat_tts"`).
//! - [`siliconflow`] — SiliconFlow native JSON speech/image/edit and
//!   asynchronous video submit/status protocols.
//! - [`xai`] — xAI JSON image/edit, deferred video, `/tts`, and `/stt`
//!   protocols.
//! - [`zhipu`] — Zhipu v4 asynchronous video submit/result protocol.
//! - [`stepfun_images`] — StepFun/Step Plan native image generation/edit
//!   (`"stepfun.images"`).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::adapter::ProtocolAdapter;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::realtime::RealtimeProtocolAdapter;

/// Per-model routing/authentication keys which belong to NomiFun itself and
/// must never be copied into a provider request body, form or query object.
///
/// Provider-specific options remain deliberately open-ended.  Any adapter
/// which offers an opaque params/request-defaults passthrough must filter it
/// through [`provider_body_fields`] instead of copying the JSON object whole.
pub(crate) const LOCAL_TRANSPORT_PARAM_KEYS: &[&str] = &[
    "base_url",
    "base_url_override",
    "allow_cross_origin_credentials",
    "endpoint",
    "poll_endpoint",
    "content_endpoint",
    "realtime_endpoint",
    "request_defaults",
    "request_body",
    "protocol",
    "connection",
    "connection_id",
    "connection_role",
    "auth",
    "auth_scheme",
    "credentials",
    "api_key",
    "api_keys",
    "headers",
];

/// Return whether `key` is owned by NomiFun's local routing/auth transport.
///
/// Save-time validation and adapter serialization intentionally share this
/// single predicate so a locally interpreted field can never drift into a
/// provider request payload.
pub fn is_reserved_local_transport_param_key(key: &str) -> bool {
    LOCAL_TRANSPORT_PARAM_KEYS.contains(&key)
}

/// All locally owned transport/auth keys, for exhaustive save-time validation
/// tests and schema tooling. The returned slice is the same source consumed by
/// [`is_reserved_local_transport_param_key`] and [`provider_body_fields`].
pub fn reserved_local_transport_param_keys() -> &'static [&'static str] {
    LOCAL_TRANSPORT_PARAM_KEYS
}

/// Iterate the top-level provider payload fields in `source`, excluding all
/// local transport/credential metadata. Non-object values yield no fields.
///
/// Filtering is intentionally top-level: nested provider-native objects may
/// legitimately use generic names such as `headers` or `endpoint`.
pub(crate) fn provider_body_fields(source: &Value) -> impl Iterator<Item = (&String, &Value)> {
    source
        .as_object()
        .map(serde_json::Map::iter)
        .into_iter()
        .flatten()
        .filter(|(key, _)| !is_reserved_local_transport_param_key(key))
}

fn provider_object<'a>(
    source: &'a Value,
    label: &str,
) -> Result<Option<&'a Map<String, Value>>, InvokeError> {
    match source {
        Value::Null => Ok(None),
        Value::Object(object) => Ok(Some(object)),
        _ => Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("{label} must be a JSON object"),
        )),
    }
}

fn merge_json_value(target: &mut Value, incoming: &Value) {
    match (target, incoming) {
        (Value::Object(target), Value::Object(incoming)) => {
            for (key, value) in incoming {
                match target.get_mut(key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, incoming) => *target = incoming.clone(),
    }
}

fn merge_provider_object(
    target: &mut Map<String, Value>,
    source: &Value,
    label: &str,
) -> Result<(), InvokeError> {
    let Some(_) = provider_object(source, label)? else {
        return Ok(());
    };
    for (key, value) in provider_body_fields(source) {
        match target.get_mut(key) {
            Some(existing) => merge_json_value(existing, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(())
}

/// Build a JSON request body from open-ended provider defaults and per-call
/// extras, then recursively overlay the adapter's typed protocol fields.
///
/// This is the common "save means send" contract for JSON protocols:
/// configured provider fields are never silently dropped, request extras win
/// over configured defaults, and typed model/task invariants win last while
/// preserving unrelated nested provider fields.
pub(crate) fn json_request_body(
    configured: &Value,
    extra: &Value,
    typed: Value,
) -> Result<Value, InvokeError> {
    let mut body = Map::new();
    merge_provider_object(&mut body, configured, "capability provider_params")?;
    merge_provider_object(&mut body, extra, "request extra")?;
    let typed = typed.as_object().ok_or_else(|| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "adapter typed JSON request body must be an object",
        )
    })?;
    for (key, value) in typed {
        match body.get_mut(key) {
            Some(existing) => merge_json_value(existing, value),
            None => {
                body.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(Value::Object(body))
}

/// Convert provider params and request extras into a deterministic scalar map
/// for multipart fields or URL query parameters. These transports cannot
/// preserve JSON arrays, objects or null without inventing a provider-specific
/// encoding, so such values fail explicitly instead of disappearing.
pub(crate) fn scalar_request_fields(
    configured: &Value,
    extra: &Value,
) -> Result<BTreeMap<String, String>, InvokeError> {
    let mut fields = BTreeMap::new();
    for (label, source) in [
        ("capability provider_params", configured),
        ("request extra", extra),
    ] {
        let Some(_) = provider_object(source, label)? else {
            continue;
        };
        for (key, value) in provider_body_fields(source) {
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                Value::Bool(value) => value.to_string(),
                Value::Null | Value::Array(_) | Value::Object(_) => {
                    return Err(InvokeError::new(
                        InvokeErrorKind::InvalidParams,
                        format!(
                            "provider field {key:?} cannot be represented by this scalar request transport"
                        ),
                    ));
                }
            };
            fields.insert(key.clone(), value);
        }
    }
    Ok(fields)
}

pub mod ark;
pub mod dashscope;
pub mod deepgram;
pub mod gemini;
pub mod generic_rerank;
pub mod minimax;
pub mod mimo;
pub mod openai_audio;
pub mod openai_embeddings;
pub mod openai_images;
pub mod openai_videos;
pub mod siliconflow;
pub mod stepfun;
pub mod stepfun_images;
pub mod stepfun_realtime;
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
        Arc::new(openai_embeddings::OpenAiEmbeddingsAdapter),
        Arc::new(generic_rerank::GenericRerankAdapter),
        Arc::new(openai_audio::OpenAiAudioTranscriptionsAdapter),
        Arc::new(openai_audio::OpenAiAudioSpeechAdapter),
        Arc::new(gemini::GeminiGenerateContentAdapter),
        Arc::new(deepgram::DeepgramListenAdapter),
        Arc::new(deepgram::DeepgramSpeakRestAdapter),
        Arc::new(ark::ArkImagesAdapter),
        Arc::new(ark::ArkVideoJobsAdapter),
        Arc::new(volc_voice::VolcAsrFileAdapter),
        Arc::new(volc_voice::VolcTtsV3Adapter),
        Arc::new(dashscope::DashScopeImagesAdapter),
        Arc::new(dashscope::DashScopeEmbeddingsAdapter),
        Arc::new(minimax::MiniMaxT2aAdapter),
        Arc::new(mimo::MiMoChatAsrAdapter),
        Arc::new(mimo::MiMoChatTtsAdapter),
        Arc::new(siliconflow::SiliconFlowAudioSpeechAdapter),
        Arc::new(siliconflow::SiliconFlowImagesAdapter),
        Arc::new(siliconflow::SiliconFlowVideoJobsAdapter),
        Arc::new(stepfun::StepFunAudioSpeechAdapter),
        Arc::new(stepfun::StepFunAsrSseAdapter),
        Arc::new(stepfun_images::StepFunImagesAdapter),
        Arc::new(xai::XaiImagesJsonAdapter),
        Arc::new(xai::XaiVideoJobsAdapter),
        Arc::new(xai::XaiTtsAdapter),
        Arc::new(xai::XaiSttAdapter),
        Arc::new(zhipu::ZhipuVideoJobsAdapter),
    ]
}

/// Build the persistent-session adapter set. Keeping this list separate from
/// [`default_adapters`] prevents a WebSocket protocol from ever being selected
/// by the one-shot HTTP resolver.
pub fn default_realtime_adapters() -> Vec<Arc<dyn RealtimeProtocolAdapter>> {
    vec![Arc::new(stepfun_realtime::StepFunRealtimeAdapter::new())]
}

#[cfg(test)]
pub(crate) mod test_support {
    use serde_json::json;

    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::{ResolvedCall, ResolvedConnection};
    use crate::types::TaskRequest;

    /// A [`ResolvedCall`] with an explicit protocol and submit endpoint, as the
    /// capability resolver would produce it. Tests must name both so manifest
    /// and adapter endpoint contracts cannot silently drift.
    pub(crate) fn call_with_endpoint(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-000000000001".into(),
            config_revision: 1,
            platform: "openai".into(),
            model: model.into(),
            task,
            protocol: protocol.into(),
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                auth: AuthMaterial {
                    scheme: AuthScheme::Bearer,
                    credentials: json!({"api_keys": ["sk-test"]}),
                },
                extra: json!({}),
            },
            model_params: json!({"endpoint": endpoint}),
            request,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_api_types::ModelTask;
    use serde_json::json;

    use super::*;
    use crate::adapter::AdapterRegistry;

    #[test]
    fn transport_metadata_filter_excludes_every_local_key() {
        for key in [
            "base_url",
            "base_url_override",
            "allow_cross_origin_credentials",
            "endpoint",
            "poll_endpoint",
            "content_endpoint",
            "realtime_endpoint",
            "connection_role",
            "auth_scheme",
            "credentials",
            "api_key",
            "headers",
        ] {
            assert!(is_reserved_local_transport_param_key(key), "missing {key}");
        }
        assert!(!is_reserved_local_transport_param_key("temperature"));

        let mut object = serde_json::Map::new();
        for key in LOCAL_TRANSPORT_PARAM_KEYS {
            object.insert((*key).to_owned(), json!(format!("secret-{key}")));
        }
        object.insert("temperature".into(), json!(0.25));
        object.insert(
            "provider_options".into(),
            json!({"headers": {"x-provider-native": "allowed"}, "endpoint": "native-value"}),
        );

        let source = serde_json::Value::Object(object);
        let keys = provider_body_fields(&source)
            .map(|(key, _)| key.as_str())
            .collect::<BTreeSet<_>>();

        assert_eq!(keys, BTreeSet::from(["provider_options", "temperature"]));
        assert_eq!(
            source["provider_options"],
            json!({"headers": {"x-provider-native": "allowed"}, "endpoint": "native-value"})
        );
    }

    #[test]
    fn json_request_body_preserves_unknown_fields_and_typed_values_win_last() {
        let body = json_request_body(
            &json!({
                "endpoint": "https://must-not-leak.invalid",
                "temperature": 0.2,
                "generationConfig": {"candidateCount": 3, "maxOutputTokens": 1},
                "model": "configured-model"
            }),
            &json!({
                "temperature": 0.4,
                "generationConfig": {"responseMimeType": "application/json"},
                "headers": {"authorization": "must-not-leak"}
            }),
            json!({
                "model": "typed-model",
                "generationConfig": {"maxOutputTokens": 512}
            }),
        )
        .unwrap();

        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["model"], "typed-model");
        assert_eq!(body["generationConfig"]["candidateCount"], 3);
        assert_eq!(body["generationConfig"]["responseMimeType"], "application/json");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 512);
        assert!(body.get("endpoint").is_none());
        assert!(body.get("headers").is_none());
    }

    #[test]
    fn scalar_request_fields_preserve_scalars_and_reject_complex_values() {
        let fields = scalar_request_fields(
            &json!({
                "endpoint": "/must-not-leak",
                "temperature": 0.25,
                "seed": 42,
                "watermark": false
            }),
            &json!({"temperature": 0.5, "format": "wav"}),
        )
        .unwrap();
        assert_eq!(fields.get("temperature").map(String::as_str), Some("0.5"));
        assert_eq!(fields.get("seed").map(String::as_str), Some("42"));
        assert_eq!(fields.get("watermark").map(String::as_str), Some("false"));
        assert_eq!(fields.get("format").map(String::as_str), Some("wav"));
        assert!(!fields.contains_key("endpoint"));

        for unsupported in [json!(null), json!(["one"]), json!({"nested": true})] {
            let error = scalar_request_fields(&json!({"future": unsupported}), &json!({}))
                .unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains("future"));
        }
    }

    #[test]
    fn default_adapters_register_the_openai_family() {
        let registry = AdapterRegistry::new(default_adapters());
        for (protocol, task) in [
            ("openai.images", ModelTask::ImageGeneration),
            ("openai.images", ModelTask::ImageEdit),
            ("openai.videos", ModelTask::VideoGeneration),
            ("openai.embeddings", ModelTask::Embedding),
            ("generic.rerank", ModelTask::Rerank),
            ("openai.audio_transcriptions", ModelTask::SpeechRecognition),
            ("openai.audio_speech", ModelTask::SpeechSynthesis),
            ("gemini.generate_content", ModelTask::ImageGeneration),
            ("gemini.generate_content", ModelTask::ImageEdit),
            ("deepgram.listen", ModelTask::SpeechRecognition),
            ("deepgram.speak_rest", ModelTask::SpeechSynthesis),
            ("ark.images", ModelTask::ImageGeneration),
            ("ark.images", ModelTask::ImageEdit),
            ("ark.video_jobs", ModelTask::VideoGeneration),
            ("volc.asr_file", ModelTask::SpeechRecognition),
            ("volc.tts_v3", ModelTask::SpeechSynthesis),
            ("dashscope.images", ModelTask::ImageGeneration),
            ("dashscope.embeddings", ModelTask::Embedding),
            ("minimax.t2a", ModelTask::SpeechSynthesis),
            ("mimo.chat_asr", ModelTask::SpeechRecognition),
            ("mimo.chat_tts", ModelTask::SpeechSynthesis),
            ("siliconflow.audio_speech", ModelTask::SpeechSynthesis),
            ("siliconflow.images", ModelTask::ImageGeneration),
            ("siliconflow.images", ModelTask::ImageEdit),
            ("siliconflow.video_jobs", ModelTask::VideoGeneration),
            ("stepfun.audio_speech", ModelTask::SpeechSynthesis),
            ("stepfun.asr_sse", ModelTask::SpeechRecognition),
            ("stepfun.images", ModelTask::ImageGeneration),
            ("stepfun.images", ModelTask::ImageEdit),
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
        assert_eq!(default_adapters().len(), 29);
        // Tasks outside an adapter's declared support are refused.
        assert!(registry.get("openai.images", ModelTask::Chat).is_err());
        assert!(registry.get("openai.videos", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("openai.embeddings", ModelTask::Chat).is_err());
        assert!(registry.get("openai.audio_transcriptions", ModelTask::Chat).is_err());
        assert!(registry.get("openai.audio_speech", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("gemini.generate_content", ModelTask::Chat).is_err());
        assert!(registry.get("deepgram.listen", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("deepgram.speak_rest", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("ark.images", ModelTask::Chat).is_err());
        assert!(registry.get("ark.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("volc.asr_file", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("volc.tts_v3", ModelTask::SpeechRecognition).is_err());
        // dashscope.images serves ImageGeneration only (no edit endpoint mapped).
        assert!(registry.get("dashscope.images", ModelTask::ImageEdit).is_err());
        assert!(registry.get("dashscope.embeddings", ModelTask::Chat).is_err());
        assert!(registry.get("minimax.t2a", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("mimo.chat_asr", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("mimo.chat_tts", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("siliconflow.audio_speech", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("siliconflow.images", ModelTask::Chat).is_err());
        assert!(registry.get("siliconflow.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("stepfun.audio_speech", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("stepfun.asr_sse", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("stepfun.images", ModelTask::Chat).is_err());
        assert!(registry.get("xai.images_json", ModelTask::Chat).is_err());
        assert!(registry.get("xai.video_jobs", ModelTask::ImageGeneration).is_err());
        assert!(registry.get("xai.tts", ModelTask::SpeechRecognition).is_err());
        assert!(registry.get("xai.stt", ModelTask::SpeechSynthesis).is_err());
        assert!(registry.get("zhipu.video_jobs", ModelTask::ImageGeneration).is_err());
    }
}
