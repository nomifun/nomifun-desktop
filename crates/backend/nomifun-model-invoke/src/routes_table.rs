//! The built-in platform routing table: `(platform, task)` → protocol id +
//! required connection role. This constant table is the single point that
//! replaces scattered platform special-casing; adapters are then looked up by
//! the returned protocol only (see [`crate::adapter::AdapterRegistry`]).

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

/// Resolve the built-in route for `(platform, task)`.
///
/// Defaults (any platform) are the OpenAI-compatible protocols; `gemini`,
/// `deepgram` and `ark`/`volcengine` override specific tasks. Volcano voice
/// tasks ride the dedicated `"voice"` connection profile.
pub fn platform_route(platform: &str, task: ModelTask) -> TaskRoute {
    use ModelTask::*;
    const fn route(protocol: &'static str) -> TaskRoute {
        TaskRoute { protocol, connection_role: None }
    }
    match (platform, task) {
        // -- gemini overrides ------------------------------------------------
        ("gemini", ImageGeneration | ImageEdit) => route("gemini.generate_content"),
        ("gemini", Chat) => route("gemini.generate_text"),
        // -- deepgram overrides ----------------------------------------------
        ("deepgram", SpeechRecognition) => route("deepgram.listen"),
        // -- Volcano (ark / volcengine) overrides; voice rides the "voice"
        //    connection profile (different domain + credentials) --------------
        ("ark" | "volcengine", ImageGeneration) => route("ark.images"),
        ("ark" | "volcengine", VideoGeneration) => route("ark.video_jobs"),
        ("ark" | "volcengine", SpeechRecognition) => {
            TaskRoute { protocol: "volc.asr_file", connection_role: Some("voice") }
        }
        ("ark" | "volcengine", SpeechSynthesis) => {
            TaskRoute { protocol: "volc.tts_v3", connection_role: Some("voice") }
        }
        // -- defaults: OpenAI-compatible protocols on the default connection --
        (_, Chat) => route("openai.chat_text"),
        (_, ImageGeneration | ImageEdit) => route("openai.images"),
        (_, VideoGeneration) => route("openai.videos"),
        (_, SpeechSynthesis) => route("openai.audio_speech"),
        (_, SpeechRecognition) => route("openai.audio_transcriptions"),
        (_, Embedding) => route("openai.embeddings"),
        (_, Rerank) => route("openai.rerank"),
    }
}

#[cfg(test)]
mod tests {
    use ModelTask::*;

    use super::*;

    fn route(protocol: &'static str) -> TaskRoute {
        TaskRoute { protocol, connection_role: None }
    }

    #[test]
    fn default_routes_cover_every_task() {
        // Table-driven full matrix for platforms without overrides.
        for platform in ["openai", "custom", "stepfun-plan", ""] {
            let cases = [
                (Chat, route("openai.chat_text")),
                (ImageGeneration, route("openai.images")),
                (ImageEdit, route("openai.images")),
                (VideoGeneration, route("openai.videos")),
                (SpeechSynthesis, route("openai.audio_speech")),
                (SpeechRecognition, route("openai.audio_transcriptions")),
                (Embedding, route("openai.embeddings")),
                (Rerank, route("openai.rerank")),
            ];
            for (task, want) in cases {
                assert_eq!(platform_route(platform, task), want, "({platform}, {task:?})");
            }
        }
    }

    #[test]
    fn gemini_overrides_image_and_chat_only() {
        let cases = [
            (ImageGeneration, route("gemini.generate_content")),
            (ImageEdit, route("gemini.generate_content")),
            (Chat, route("gemini.generate_text")),
            // Non-overridden tasks keep the defaults.
            (VideoGeneration, route("openai.videos")),
            (SpeechSynthesis, route("openai.audio_speech")),
            (SpeechRecognition, route("openai.audio_transcriptions")),
            (Embedding, route("openai.embeddings")),
            (Rerank, route("openai.rerank")),
        ];
        for (task, want) in cases {
            assert_eq!(platform_route("gemini", task), want, "(gemini, {task:?})");
        }
    }

    #[test]
    fn deepgram_overrides_speech_recognition_only() {
        assert_eq!(platform_route("deepgram", SpeechRecognition), route("deepgram.listen"));
        assert_eq!(platform_route("deepgram", Chat), route("openai.chat_text"));
        assert_eq!(platform_route("deepgram", SpeechSynthesis), route("openai.audio_speech"));
    }

    #[test]
    fn ark_and_volcengine_share_overrides_voice_rides_voice_role() {
        for platform in ["ark", "volcengine"] {
            let cases = [
                (ImageGeneration, route("ark.images")),
                (VideoGeneration, route("ark.video_jobs")),
                (SpeechRecognition, TaskRoute { protocol: "volc.asr_file", connection_role: Some("voice") }),
                (SpeechSynthesis, TaskRoute { protocol: "volc.tts_v3", connection_role: Some("voice") }),
                // Not overridden: ImageEdit / Chat / Embedding / Rerank fall to defaults.
                (ImageEdit, route("openai.images")),
                (Chat, route("openai.chat_text")),
                (Embedding, route("openai.embeddings")),
                (Rerank, route("openai.rerank")),
            ];
            for (task, want) in cases {
                assert_eq!(platform_route(platform, task), want, "({platform}, {task:?})");
            }
        }
    }
}
