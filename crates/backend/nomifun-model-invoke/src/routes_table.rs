//! Provider-preset recommendations for model configuration.
//!
//! This table is deliberately not consulted by runtime dispatch. Runtime uses
//! the protocol explicitly persisted on the provider-model task capability;
//! these entries only prefill a new configuration and drive the manifest API.

use nomifun_api_types::ModelTask;

/// Where a task on a platform is served: the protocol id and, when the
/// protocol rides a non-default connection profile, its role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskRoute {
    /// Protocol id ([`crate::adapter::ProtocolAdapter::id`] registry key).
    pub protocol: &'static str,
    /// Required connection role; `None` = the provider's default connection.
    pub connection_role: Option<&'static str>,
}

const fn route(protocol: &'static str) -> Option<TaskRoute> {
    Some(TaskRoute { protocol, connection_role: None })
}

fn openai_route(task: ModelTask) -> Option<TaskRoute> {
    use ModelTask::*;
    match task {
        Chat => route("openai.chat_text"),
        // Persistent WebSocket sessions use the dedicated realtime registry;
        // they must never be coerced into the one-shot OpenAI HTTP adapter.
        RealtimeConversation => None,
        ImageGeneration | ImageEdit => route("openai.images"),
        // OpenAI announced that the Sora video API shuts down permanently on
        // 2026-09-24 (openai-python marks every `videos` method deprecated with
        // that date, and the OpenAPI spec flags the whole `/videos` family), and
        // it publishes no successor path. So it is no longer offered as a
        // preset: nothing auto-selects it, and `sora-*` is no longer classified
        // as a video model. `openai.videos` stays in the registry until the
        // shutdown so already-saved capabilities keep resolving; delete the spec
        // and its adapter after that date.
        VideoGeneration => None,
        SpeechSynthesis => route("openai.audio_speech"),
        SpeechRecognition => route("openai.audio_transcriptions"),
        Embedding => route("openai.embeddings"),
        // No OpenAI rerank endpoint/adapter exists.
        Rerank => None,
    }
}

/// Return the verified configuration recommendation for `(platform, task)`.
///
/// `None` means there is no unconditional platform default. In particular,
/// `custom` and `new-api` stay absent here so runtime probes never guess a
/// protocol. The model-aware configuration manifest may separately recommend
/// a registry-verified generic protocol for `custom`; callers must persist it.
pub fn preset_protocol_recommendation(platform: &str, task: ModelTask) -> Option<TaskRoute> {
    use ModelTask::*;

    match (platform, task) {
        ("custom" | "new-api", _) => None,

        // OpenAI native endpoints. OpenAI does not expose rerank.
        ("openai", task) => openai_route(task),

        // Gemini native generateContent adapters.
        ("gemini", ImageGeneration | ImageEdit) => route("gemini.generate_content"),
        ("gemini", Chat) => route("gemini.generate_text"),

        // These Chat transports execute through the Nomi provider layer.
        ("anthropic", Chat) => route("anthropic.messages"),
        ("bedrock", Chat) => route("bedrock.anthropic_messages"),

        // Deepgram's REST speech endpoints. Voice Agent WebSocket realtime is
        // intentionally not routed until a dedicated session adapter ships.
        ("deepgram", SpeechRecognition) => route("deepgram.listen"),
        ("deepgram", SpeechSynthesis) => route("deepgram.speak_rest"),

        // Volcano Ark multimodal adapters. Voice uses a distinct connection
        // because its domain and credentials differ from Ark's default API.
        ("ark" | "volcengine", ImageGeneration) => route("ark.images"),
        ("ark" | "volcengine", VideoGeneration) => route("ark.video_jobs"),
        ("ark" | "volcengine", SpeechRecognition) => Some(TaskRoute {
            protocol: "volc.asr_file",
            connection_role: Some("voice"),
        }),
        ("ark" | "volcengine", SpeechSynthesis) => Some(TaskRoute {
            protocol: "volc.tts_v3",
            connection_role: Some("voice"),
        }),

        // DashScope native input/parameters protocols.
        ("dashscope" | "alibaba", ImageGeneration) => route("dashscope.images"),
        ("dashscope" | "alibaba", Embedding) => route("dashscope.embeddings"),

        // MiniMax TTS has a provider-specific request/response codec.
        ("minimax", SpeechSynthesis) => route("minimax.t2a"),
        // MiMo audio models use specialized chat-completions serializers.
        ("mimo", SpeechRecognition) => route("mimo.chat_asr"),
        ("mimo", SpeechSynthesis) => route("mimo.chat_tts"),
        // SiliconFlow media endpoints have native JSON, raw-audio and async schemas.
        ("siliconflow", SpeechSynthesis) => route("siliconflow.audio_speech"),
        ("siliconflow", ImageGeneration | ImageEdit) => route("siliconflow.images"),
        ("siliconflow", VideoGeneration) => route("siliconflow.video_jobs"),
        // xAI media APIs only resemble OpenAI by naming; their bodies and job
        // lifecycle require dedicated adapters.
        ("xai", ImageGeneration | ImageEdit) => route("xai.images_json"),
        ("xai", VideoGeneration) => route("xai.video_jobs"),
        ("xai", SpeechSynthesis) => route("xai.tts"),
        ("xai", SpeechRecognition) => route("xai.stt"),
        // Zhipu video uses its own v4 async-result job lifecycle.
        ("zhipu", VideoGeneration) => route("zhipu.video_jobs"),

        // Verified task-specific OpenAI-compatible endpoints. These entries
        // are deliberately enumerated instead of inferred from Chat support.
        (
            "novita"
            | "openrouter"
            | "siliconflow"
            | "ppio"
            | "infiniai"
            | "qianfan"
            | "hunyuan"
            | "hunyuan-global",
            Embedding,
        ) => {
            route("openai.embeddings")
        }
        ("siliconflow", SpeechRecognition) => route("openai.audio_transcriptions"),
        ("stepfun" | "stepfun-plan", SpeechRecognition) => route("stepfun.asr_sse"),
        ("stepfun" | "stepfun-plan", SpeechSynthesis) => route("stepfun.audio_speech"),
        ("stepfun" | "stepfun-plan", ImageGeneration | ImageEdit) => route("stepfun.images"),
        ("stepfun" | "stepfun-plan", RealtimeConversation) => route("stepfun.realtime_s2s"),
        // CTYun documents an OpenAI-compatible image surface. Zhipu's
        // similarly named endpoints do not accept the generic OpenAI image /
        // transcription bodies (for example the generic serializer injects
        // fields absent from Zhipu's schemas), so those combinations remain
        // deny-by-default until native serializers ship.
        ("ctyun", ImageGeneration) => route("openai.images"),
        ("ctyun" | "zhipu", Embedding) => route("openai.embeddings"),
        ("siliconflow" | "ppio" | "qianfan" | "ctyun" | "zhipu", Rerank) => {
            route("generic.rerank")
        }

        // These presets explicitly expose an OpenAI-compatible chat endpoint.
        // Their other modalities are not inherited: each needs a verified
        // task-specific entry above (or a future native adapter).
        (
            "deepseek"
            | "mimo"
            | "mimo-token-plan-cn"
            | "mimo-token-plan-sgp"
            | "mimo-token-plan-ams"
            | "minimax"
            | "minimax-code"
            | "minimax-coding-plan"
            | "novita"
            | "openrouter"
            | "dashscope"
            | "alibaba"
            | "dashscope-coding"
            | "siliconflow"
            | "zhipu"
            | "glm-coding-plan"
            | "moonshot-cn"
            | "moonshot-global"
            | "xai"
            | "ark"
            | "volcengine"
            | "ark-coding-plan"
            | "ark-agent-plan"
            | "qianfan"
            | "qianfan-coding-plan"
            | "hunyuan"
            | "hunyuan-global"
            | "lingyi"
            | "poe"
            | "ppio"
            | "modelscope"
            | "infiniai"
            | "ctyun"
            | "stepfun"
            | "stepfun-plan",
            Chat,
        ) => route("openai.chat_text"),

        // Includes native-only presets without a shipped invoke adapter
        // (Anthropic, Bedrock and Vertex AI), unknown platform values, and all
        // unverified provider/task combinations.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ModelTask::*;

    use super::*;

    fn plain(protocol: &'static str) -> Option<TaskRoute> {
        route(protocol)
    }

    fn platform_route(platform: &str, task: ModelTask) -> Option<TaskRoute> {
        preset_protocol_recommendation(platform, task)
    }

    #[test]
    fn custom_gateways_have_no_unconditional_preset_defaults() {
        for platform in ["custom", "new-api"] {
            for task in [
                Chat,
                RealtimeConversation,
                ImageGeneration,
                ImageEdit,
                VideoGeneration,
                SpeechSynthesis,
                SpeechRecognition,
                Embedding,
                Rerank,
            ] {
                assert_eq!(platform_route(platform, task), None, "({platform}, {task:?})");
            }
        }
    }

    #[test]
    fn openai_has_no_rerank_or_video_route() {
        assert_eq!(platform_route("openai", Chat), plain("openai.chat_text"));
        assert_eq!(platform_route("openai", ImageGeneration), plain("openai.images"));
        assert_eq!(platform_route("openai", ImageEdit), plain("openai.images"));
        // Sora shuts down 2026-09-24 with no successor path, so OpenAI video is
        // no longer a preset. The adapter stays registered for saved rows.
        assert_eq!(platform_route("openai", VideoGeneration), None);
        assert_eq!(platform_route("openai", SpeechSynthesis), plain("openai.audio_speech"));
        assert_eq!(platform_route("openai", SpeechRecognition), plain("openai.audio_transcriptions"));
        assert_eq!(platform_route("openai", Embedding), plain("openai.embeddings"));
        assert_eq!(platform_route("openai", Rerank), None);
    }

    #[test]
    fn gemini_routes_only_shipped_native_modalities() {
        assert_eq!(platform_route("gemini", Chat), plain("gemini.generate_text"));
        assert_eq!(platform_route("gemini", ImageGeneration), plain("gemini.generate_content"));
        assert_eq!(platform_route("gemini", ImageEdit), plain("gemini.generate_content"));
        for task in [VideoGeneration, SpeechSynthesis, SpeechRecognition, Embedding, Rerank] {
            assert_eq!(platform_route("gemini", task), None, "(gemini, {task:?})");
        }
    }

    #[test]
    fn provider_specific_adapters_do_not_leak_openai_fallbacks() {
        assert_eq!(platform_route("deepgram", SpeechRecognition), plain("deepgram.listen"));
        assert_eq!(platform_route("deepgram", SpeechSynthesis), plain("deepgram.speak_rest"));
        assert_eq!(platform_route("deepgram", Chat), None);
        assert_eq!(platform_route("deepgram", RealtimeConversation), None);

        for platform in ["dashscope", "alibaba"] {
            assert_eq!(platform_route(platform, Chat), plain("openai.chat_text"));
            assert_eq!(platform_route(platform, ImageGeneration), plain("dashscope.images"));
            assert_eq!(platform_route(platform, Embedding), plain("dashscope.embeddings"));
            for task in [ImageEdit, VideoGeneration, SpeechSynthesis, SpeechRecognition, Rerank] {
                assert_eq!(platform_route(platform, task), None, "({platform}, {task:?})");
            }
        }

        assert_eq!(platform_route("minimax", Chat), plain("openai.chat_text"));
        assert_eq!(platform_route("minimax", SpeechSynthesis), plain("minimax.t2a"));
        assert_eq!(platform_route("minimax", ImageGeneration), None);
        assert_eq!(platform_route("minimax", VideoGeneration), None);
    }

    #[test]
    fn ark_and_volcengine_keep_native_routes_and_voice_role() {
        for platform in ["ark", "volcengine"] {
            assert_eq!(platform_route(platform, Chat), plain("openai.chat_text"));
            assert_eq!(platform_route(platform, ImageGeneration), plain("ark.images"));
            assert_eq!(platform_route(platform, VideoGeneration), plain("ark.video_jobs"));
            assert_eq!(
                platform_route(platform, SpeechRecognition),
                Some(TaskRoute { protocol: "volc.asr_file", connection_role: Some("voice") })
            );
            assert_eq!(
                platform_route(platform, SpeechSynthesis),
                Some(TaskRoute { protocol: "volc.tts_v3", connection_role: Some("voice") })
            );
            for task in [ImageEdit, Embedding, Rerank] {
                assert_eq!(platform_route(platform, task), None, "({platform}, {task:?})");
            }
        }
    }

    #[test]
    fn verified_chat_compatibility_does_not_imply_other_modalities() {
        for platform in [
            "deepseek",
            "mimo",
            "openrouter",
            "moonshot-cn",
            "poe",
        ] {
            assert_eq!(platform_route(platform, Chat), plain("openai.chat_text"), "{platform}");
            assert_eq!(platform_route(platform, ImageEdit), None, "{platform}");
            assert_eq!(platform_route(platform, Rerank), None, "{platform}");
        }
    }

    #[test]
    fn verified_openai_compatible_non_chat_tasks_are_explicit() {
        for platform in [
            "novita",
            "openrouter",
            "siliconflow",
            "ppio",
            "infiniai",
            "qianfan",
            "hunyuan",
            "hunyuan-global",
        ] {
            assert_eq!(platform_route(platform, Embedding), plain("openai.embeddings"), "{platform}");
        }
        assert_eq!(platform_route("siliconflow", SpeechRecognition), plain("openai.audio_transcriptions"));
        assert_eq!(platform_route("siliconflow", SpeechSynthesis), plain("siliconflow.audio_speech"));
        assert_eq!(platform_route("siliconflow", ImageGeneration), plain("siliconflow.images"));
        assert_eq!(platform_route("siliconflow", ImageEdit), plain("siliconflow.images"));
        assert_eq!(platform_route("siliconflow", VideoGeneration), plain("siliconflow.video_jobs"));

        assert_eq!(platform_route("mimo", SpeechRecognition), plain("mimo.chat_asr"));
        assert_eq!(platform_route("mimo", SpeechSynthesis), plain("mimo.chat_tts"));
        assert_eq!(platform_route("mimo-token-plan-cn", SpeechRecognition), None);

        assert_eq!(platform_route("xai", ImageGeneration), plain("xai.images_json"));
        assert_eq!(platform_route("xai", ImageEdit), plain("xai.images_json"));
        assert_eq!(platform_route("xai", VideoGeneration), plain("xai.video_jobs"));
        assert_eq!(platform_route("xai", SpeechSynthesis), plain("xai.tts"));
        assert_eq!(platform_route("xai", SpeechRecognition), plain("xai.stt"));
        assert_eq!(platform_route("xai", Embedding), None);

        assert_eq!(platform_route("ctyun", ImageGeneration), plain("openai.images"));
        assert_eq!(platform_route("zhipu", ImageGeneration), None);
        assert_eq!(platform_route("ctyun", ImageEdit), None);
        assert_eq!(platform_route("zhipu", ImageEdit), None);
        assert_eq!(platform_route("zhipu", VideoGeneration), plain("zhipu.video_jobs"));
        for platform in ["stepfun", "stepfun-plan"] {
            assert_eq!(platform_route(platform, ImageGeneration), plain("stepfun.images"));
            assert_eq!(platform_route(platform, ImageEdit), plain("stepfun.images"));
            assert_eq!(
                platform_route(platform, RealtimeConversation),
                plain("stepfun.realtime_s2s")
            );
        }
        assert_eq!(platform_route("ctyun", Embedding), plain("openai.embeddings"));
        assert_eq!(platform_route("zhipu", Embedding), plain("openai.embeddings"));
        for platform in ["siliconflow", "ppio", "qianfan", "ctyun", "zhipu"] {
            assert_eq!(platform_route(platform, Rerank), plain("generic.rerank"), "{platform}");
        }
        assert_eq!(platform_route("novita", Rerank), None);
        assert_eq!(platform_route("infiniai", Rerank), None);
        assert_eq!(platform_route("zhipu", SpeechRecognition), None);
        assert_eq!(platform_route("stepfun", SpeechRecognition), plain("stepfun.asr_sse"));
        assert_eq!(platform_route("stepfun", SpeechSynthesis), plain("stepfun.audio_speech"));
        assert_eq!(platform_route("stepfun-plan", SpeechSynthesis), plain("stepfun.audio_speech"));
        assert_eq!(platform_route("stepfun-plan", SpeechRecognition), plain("stepfun.asr_sse"));
    }

    #[test]
    fn native_only_and_unknown_platforms_are_deny_by_default() {
        assert_eq!(platform_route("anthropic", Chat), plain("anthropic.messages"));
        assert_eq!(
            platform_route("bedrock", Chat),
            plain("bedrock.anthropic_messages")
        );
        for platform in ["gemini-vertex-ai", "vertex-ai", "", "typo-provider"] {
            for task in [
                Chat,
                RealtimeConversation,
                ImageGeneration,
                ImageEdit,
                VideoGeneration,
                SpeechSynthesis,
                SpeechRecognition,
                Embedding,
                Rerank,
            ] {
                assert_eq!(platform_route(platform, task), None, "({platform}, {task:?})");
            }
        }
    }
}
