//! ASR, TTS and one-shot vision against the platform's model layer.
//!
//! ASR and TTS go through `ModelInvokeService`, while one-shot vision uses the
//! shared Agent provider path so it can send an inline image block. Routing the
//! vision request through a conversation would self-nest (the device is already
//! inside a tool call on that conversation) and exceed the firmware's 30 s HTTP
//! ceiling.

use std::sync::Arc;
use std::time::Duration;

use nomifun_api_types::{
    SpeechToTextConfig, TEXT_TO_SPEECH_PREFERENCE_KEY, TextToSpeechConfig,
};
use nomifun_model_invoke::ModelInvokeService;
use nomifun_model_invoke::types::{
    AsrRequest, InputAsset, ModelRef, ProducedData, TaskOutcome, TaskRequest, TaskResult,
    TtsRequest,
};
use serde_json::Value;

use crate::audio::{AudioBuffer, decode_container};
use crate::protocol::DOWNLINK_SAMPLE_RATE;
use crate::services::{SpeechContext, SpeechServices};

/// Vision must answer well inside the firmware's fixed 30 s HTTP timeout,
/// otherwise the device hangs up while we are still waiting. The
/// `/robot/vision/explain` handler deliberately adds no second timeout — one
/// ceiling, held here, next to the call it bounds.
pub const VISION_TIMEOUT_SECS: u64 = 25;
/// The install-wide speech-recognition preference key.
const SPEECH_TO_TEXT_PREFERENCE_KEY: &str = "tools.speechToText";

/// Read a global client preference.
#[async_trait::async_trait]
pub trait PreferenceReader: Send + Sync {
    async fn get(&self, key: &str) -> Option<Value>;
}

/// Complete one-shot image question passed to the application's Agent Chat
/// bridge. The bridge resolves the exact persisted Chat capability; this crate
/// never sees credentials or infers a serializer from provider identity.
#[derive(Debug, Clone)]
pub struct VisionCompletionRequest {
    pub provider_id: String,
    pub model: String,
    pub jpeg: Vec<u8>,
    pub question: String,
}

#[async_trait::async_trait]
pub trait VisionCompletionExecutor: Send + Sync {
    async fn complete(&self, request: VisionCompletionRequest) -> anyhow::Result<String>;
}

/// Read a companion's model slots.
#[async_trait::async_trait]
pub trait CompanionSlotReader: Send + Sync {
    /// `(provider_id, model)` of the companion's ASR slot, if set.
    async fn asr_slot(&self, companion_id: &str) -> Option<(String, String)>;
    /// `(provider_id, model, voice)` of the companion's TTS slot, if set.
    async fn tts_slot(&self, companion_id: &str) -> Option<(String, String, Option<String>)>;
    /// `(provider_id, model)` for vision: the companion's `vision_model`, else
    /// its main chat model when that model has the vision trait.
    async fn vision_slot(&self, companion_id: &str) -> Option<(String, String)>;
}

/// First non-empty of the two candidates. Companion slots always win: a
/// per-robot voice is the whole point of having the slot.
fn pick_model(
    companion: Option<(String, String)>,
    global: Option<(String, String)>,
) -> Option<(String, String)> {
    companion.or(global)
}

/// Parse the `tools.textToSpeech` preference value.
///
/// Delegates to [`TextToSpeechConfig::from_preferences`], the single source of
/// truth for this key — two independent parsers for one preference is exactly
/// the drift the model-provider spec already records as debt. The tests below
/// therefore also pin that the shared parser still satisfies this contract.
fn parse_global_tts(value: &Value) -> Option<(String, String, Option<String>)> {
    let prefs = nomifun_api_types::ClientPreferencesResponse::from([(
        TEXT_TO_SPEECH_PREFERENCE_KEY.to_owned(),
        value.clone(),
    )]);
    let config = TextToSpeechConfig::from_preferences(&prefs)?;
    Some((config.provider_id, config.model, config.voice))
}

/// Parse the `tools.speechToText` preference value down to a model reference.
fn parse_global_asr(value: &Value) -> Option<(String, String)> {
    let config = serde_json::from_value::<SpeechToTextConfig>(value.clone()).ok()?;
    if !config.enabled {
        return None;
    }
    let provider_id = config.provider_id?;
    let model = config.model?;
    Some((provider_id, model))
}

/// Turn a synthesised asset into PCM. `audio/pcm` is already at the rate we
/// requested, so it bypasses the decoder entirely (that is why `format: "pcm"`
/// is preferred: no container, no guessing).
fn audio_from_asset(bytes: &[u8], mime: Option<&str>) -> anyhow::Result<AudioBuffer> {
    if mime.is_some_and(|m| m.contains("pcm")) {
        let pcm = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return Ok(AudioBuffer {
            pcm,
            sample_rate: DOWNLINK_SAMPLE_RATE,
        });
    }
    decode_container(bytes, mime)
}

/// The real [`SpeechServices`].
pub struct RobotSpeech {
    invoke: Arc<ModelInvokeService>,
    slots: Arc<dyn CompanionSlotReader>,
    prefs: Arc<dyn PreferenceReader>,
    vision: Arc<dyn VisionCompletionExecutor>,
}

impl RobotSpeech {
    pub fn new(
        invoke: Arc<ModelInvokeService>,
        slots: Arc<dyn CompanionSlotReader>,
        prefs: Arc<dyn PreferenceReader>,
        vision: Arc<dyn VisionCompletionExecutor>,
    ) -> Self {
        Self {
            invoke,
            slots,
            prefs,
            vision,
        }
    }
}

#[async_trait::async_trait]
impl SpeechServices for RobotSpeech {
    async fn transcribe(&self, ctx: &SpeechContext, wav: Vec<u8>) -> anyhow::Result<String> {
        let global = self
            .prefs
            .get(SPEECH_TO_TEXT_PREFERENCE_KEY)
            .await
            .as_ref()
            .and_then(parse_global_asr);
        let (provider_id, model) =
            pick_model(self.slots.asr_slot(&ctx.companion_id).await, global)
                .ok_or_else(|| anyhow::anyhow!("no speech-recognition model configured"))?;

        let request = TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".to_owned(),
                bytes: wav,
                mime: "audio/wav".to_owned(),
            },
            language: None,
            prompt: None,
            extra: Value::Object(Default::default()),
        });
        let outcome = self
            .invoke
            .invoke(&ModelRef { provider_id, model }, request)
            .await?;
        match outcome {
            TaskOutcome::Done(TaskResult::Transcript { text, .. }) => Ok(text),
            TaskOutcome::Done(_) => {
                anyhow::bail!("speech recognition returned a non-transcript result")
            }
            TaskOutcome::Pending(_) => {
                anyhow::bail!("speech recognition must be synchronous for a live conversation")
            }
        }
    }

    async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer> {
        let global = self
            .prefs
            .get(TEXT_TO_SPEECH_PREFERENCE_KEY)
            .await
            .as_ref()
            .and_then(parse_global_tts);
        let (provider_id, model, voice) = self
            .slots
            .tts_slot(&ctx.companion_id)
            .await
            .or(global)
            .ok_or_else(|| anyhow::anyhow!("no speech-synthesis model configured"))?;

        let request = TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.to_owned(),
            voice,
            // `pcm` is the one format that comes back without a container, and
            // the OpenAI contract makes it 24 kHz mono — exactly what we told
            // the device to expect.
            format: Some("pcm".to_owned()),
            extra: Value::Object(Default::default()),
        });
        let outcome = self
            .invoke
            .invoke(&ModelRef { provider_id, model }, request)
            .await?;
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else {
            anyhow::bail!("speech synthesis returned no audio");
        };
        let asset = assets
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("empty audio result"))?;
        let bytes = match asset.data {
            ProducedData::Bytes(bytes) => bytes,
            // A URL would need a second fetch inside a live turn; the adapters
            // inline audio, so this is a contract break, not a slow path.
            ProducedData::Url(url) => {
                anyhow::bail!("synthesised audio came back as a URL ({url}), not bytes")
            }
        };
        let audio = audio_from_asset(&bytes, asset.mime.as_deref())?;
        if audio.pcm.is_empty() {
            // Returning silence would look like a working turn with a mute
            // robot: the downlink queues `tts stop` behind the (absent) audio
            // and the device falls straight back to listening.
            anyhow::bail!("speech synthesis produced no samples");
        }
        Ok(audio)
    }

    async fn explain_image(
        &self,
        ctx: &SpeechContext,
        jpeg: Vec<u8>,
        question: &str,
    ) -> anyhow::Result<String> {
        let (provider_id, model) = self
            .slots
            .vision_slot(&ctx.companion_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("未配置视觉模型"))?;

        let answer = tokio::time::timeout(
            Duration::from_secs(VISION_TIMEOUT_SECS),
            self.vision.complete(VisionCompletionRequest {
                provider_id,
                model,
                jpeg,
                question: question.to_owned(),
            }),
        )
        .await
        .map_err(|_| anyhow::anyhow!("视觉模型响应太慢"))??;
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn asr_prefers_the_companion_slot_over_the_global_preference() {
        let companion = Some(("p-companion".to_owned(), "whisper-1".to_owned()));
        let global = Some(("p-global".to_owned(), "gpt-4o-transcribe".to_owned()));
        assert_eq!(
            pick_model(companion.clone(), global.clone()),
            Some(("p-companion".to_owned(), "whisper-1".to_owned()))
        );
        assert_eq!(pick_model(None, global.clone()), global);
        assert_eq!(pick_model(None, None), None);
    }

    #[test]
    fn global_tts_preference_is_parsed_into_a_model_and_voice() {
        // `provider_id` is a canonical `ProviderId` (a UUID): the shared
        // `TextToSpeechConfig` parser validates it, and this test exists to pin
        // that this crate keeps agreeing with that one parser.
        let provider = "0190f5fe-7c00-7a00-8000-0000000000aa";
        let value = json!({ "provider_id": provider, "model": "tts-1", "voice": "alloy" });
        let parsed = parse_global_tts(&value).unwrap();
        assert_eq!(parsed.0, provider);
        assert_eq!(parsed.1, "tts-1");
        assert_eq!(parsed.2.as_deref(), Some("alloy"));

        let no_voice = json!({ "provider_id": provider, "model": "tts-1" });
        assert_eq!(parse_global_tts(&no_voice).unwrap().2, None);
        assert!(
            parse_global_tts(&json!({ "model": "tts-1" })).is_none(),
            "provider_id is required"
        );
        assert!(
            parse_global_tts(&json!({ "provider_id": "prov_legacy", "model": "tts-1" })).is_none(),
            "a non-canonical provider id is not a usable reference"
        );
        assert!(parse_global_tts(&json!("nonsense")).is_none());
    }

    #[test]
    fn global_asr_preference_uses_the_shared_catalog_config() {
        let value = json!({
            "enabled": true,
            "provider_id": "0190f5fe-7c00-7a00-8000-0000000000aa",
            "model": "whisper-1",
        });
        assert_eq!(
            parse_global_asr(&value),
            Some((
                "0190f5fe-7c00-7a00-8000-0000000000aa".to_owned(),
                "whisper-1".to_owned()
            ))
        );
        assert!(
            parse_global_asr(&json!({
                "enabled": false,
                "provider_id": "0190f5fe-7c00-7a00-8000-0000000000aa",
                "model": "whisper-1"
            }))
            .is_none()
        );
        assert!(parse_global_asr(&json!({ "enabled": true })).is_none());
        assert!(
            parse_global_asr(&json!({
                "enabled": true,
                "provider": "openai",
                "provider_id": "0190f5fe-7c00-7a00-8000-0000000000aa",
                "model": "whisper-1"
            }))
            .is_none(),
            "retired provider enums must not be accepted"
        );
        assert!(
            parse_global_asr(&json!({ "provider_id": " ", "model": "whisper-1" })).is_none(),
            "a blank provider id is not a reference"
        );
        assert!(parse_global_asr(&json!("nonsense")).is_none());
    }

    #[test]
    fn pcm_assets_skip_the_container_decoder() {
        // A raw PCM asset is already at the rate the device expects.
        let pcm_bytes: Vec<u8> = vec![0x00, 0x01, 0x02, 0x03];
        let audio = audio_from_asset(&pcm_bytes, Some("audio/pcm")).unwrap();
        assert_eq!(audio.sample_rate, crate::protocol::DOWNLINK_SAMPLE_RATE);
        assert_eq!(audio.pcm.len(), 2, "four bytes are two 16-bit samples");
    }

    #[test]
    fn container_assets_go_through_symphonia() {
        let wav = crate::audio::pcm_to_wav(&vec![0i16; 2400], 24_000);
        let audio = audio_from_asset(&wav, Some("audio/wav")).unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert_eq!(audio.pcm.len(), 2400);
    }

    #[test]
    fn an_unparseable_asset_is_an_error_not_silence() {
        assert!(audio_from_asset(b"not audio", Some("audio/mpeg")).is_err());
    }

    #[test]
    fn odd_length_pcm_does_not_panic() {
        let audio = audio_from_asset(&[0x00, 0x01, 0x02], Some("audio/pcm")).unwrap();
        assert_eq!(audio.pcm.len(), 1, "the trailing half sample is dropped");
    }
}
