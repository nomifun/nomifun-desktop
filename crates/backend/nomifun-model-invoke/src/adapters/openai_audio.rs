//! The OpenAI-compatible audio family: `openai.audio_transcriptions`
//! (speech-to-text, ported from `nomifun-shell/src/stt_openai.rs`) and
//! `openai.audio_speech` (text-to-speech).
//!
//! Transcriptions: `POST` the dispatch target (conventionally
//! `{base}/v1/audio/transcriptions` with the single-`/v1` normalization,
//! `is_full_url` bases verbatim — both handled by
//! [`crate::call::ResolvedCall::dispatch_target`]) as multipart: `file` (the
//! audio bytes) + `model` + an explicit `response_format=json` (OpenAI
//! defaults to JSON but StepFun's `step-asr` contract requires the field),
//! plus optional `language` / `prompt` (from the request) and `temperature`
//! (from `extra.temperature`). The response's `text` field →
//! [`TaskResult::Transcript`] (language echoes the request language, model
//! echoes the called model — this API reports neither back).
//!
//! Speech: `POST` the dispatch target (conventionally `{base}/v1/audio/speech`)
//! as JSON `{model, input, voice (default "alloy"), response_format?}`; the
//! response is the RAW audio binary (capped at
//! [`crate::transport::MAX_ARTIFACT_BYTES`]) → a single-element
//! [`TaskResult::Assets`] whose MIME rides the requested format
//! ([`mime_for_speech_format`]), falling back to the response's `audio/*`
//! `Content-Type` and finally `audio/mpeg`.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::Value;

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{MAX_ARTIFACT_BYTES, error_from_response, net_err, read_body_capped};
use crate::types::{ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Voice sent when the request does not pick one (the OpenAI API requires the
/// field; "alloy" is its conventional default).
const DEFAULT_TTS_VOICE: &str = "alloy";

/// OpenAI-compatible `/audio/transcriptions` protocol.
pub struct OpenAiAudioTranscriptionsAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAiAudioTranscriptionsAdapter {
    fn id(&self) -> &'static str {
        "openai.audio_transcriptions"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.audio_transcriptions cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.dispatch_target().url;

        let file_part = Part::bytes(req.audio.bytes.clone())
            .file_name(format!("audio.{}", ext_for_audio_mime(&req.audio.mime)))
            .mime_str(&req.audio.mime)
            .map_err(|e| InvokeError::new(InvokeErrorKind::InvalidParams, format!("invalid audio mime: {e}")))?;

        let mut form = Form::new()
            .part("file", file_part)
            .text("model", call.model.clone())
            // OpenAI defaults this to JSON, but StepFun's `step-asr` contract
            // requires the field explicitly.
            .text("response_format", "json");

        let language = req.language.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(lang) = language {
            form = form.text("language", lang.to_owned());
        }
        if let Some(prompt) = req.prompt.as_deref().filter(|s| !s.is_empty()) {
            form = form.text("prompt", prompt.to_owned());
        }
        if let Some(temp) = req.extra.get("temperature").and_then(|v| v.as_f64()) {
            form = form.text("temperature", temp.to_string());
        }

        let rb = http.post(&url).timeout(REQUEST_TIMEOUT).multipart(form);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let body: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid transcription JSON: {e}")))?;
        let text = body["text"].as_str().unwrap_or("").to_owned();

        Ok(TaskOutcome::Done(TaskResult::Transcript {
            text,
            language: language.map(str::to_owned),
            model: Some(call.model.clone()),
        }))
    }
}

/// OpenAI-compatible `/audio/speech` protocol (text-to-speech).
pub struct OpenAiAudioSpeechAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAiAudioSpeechAdapter {
    fn id(&self) -> &'static str {
        "openai.audio_speech"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.audio_speech cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.dispatch_target().url;

        let mut body = serde_json::json!({
            "model": call.model,
            "input": req.text,
            "voice": req.voice.as_deref().unwrap_or(DEFAULT_TTS_VOICE),
        });
        if let Some(format) = req.format.as_deref() {
            body["response_format"] = Value::String(format.to_owned());
        }

        let rb = http.post(&url).timeout(REQUEST_TIMEOUT).json(&body);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        // The requested format pins the MIME; without one, trust the
        // response's audio/* Content-Type, else assume the API default (mp3).
        let mime = match req.format.as_deref() {
            Some(format) => mime_for_speech_format(format).to_owned(),
            None => resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
                .filter(|v| v.starts_with("audio/"))
                .unwrap_or_else(|| "audio/mpeg".to_owned()),
        };
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;

        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(mime),
        }])))
    }
}

/// MIME type of a `/audio/speech` `response_format`. Unknown formats fall back
/// to `audio/mpeg` (the API's own default output).
fn mime_for_speech_format(format: &str) -> &'static str {
    match format {
        "wav" => "audio/wav",
        "opus" => "audio/ogg",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "pcm" => "audio/pcm",
        _ => "audio/mpeg", // "mp3" and anything unrecognized
    }
}

/// A plausible upload filename extension for the audio MIME (some providers
/// sniff the extension rather than the part's content type).
fn ext_for_audio_mime(mime: &str) -> &'static str {
    match mime {
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/webm" => "webm",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;
    use crate::types::{AsrRequest, InputAsset};

    fn asr(language: Option<&str>, prompt: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: b"RIFFdata".to_vec(), mime: "audio/wav".into() },
            language: language.map(str::to_string),
            prompt: prompt.map(str::to_string),
            extra,
        })
    }

    #[test]
    fn audio_ext_mapping() {
        assert_eq!(ext_for_audio_mime("audio/wav"), "wav");
        assert_eq!(ext_for_audio_mime("audio/mpeg"), "mp3");
        assert_eq!(ext_for_audio_mime("audio/mp4"), "m4a");
        assert_eq!(ext_for_audio_mime("audio/ogg"), "ogg");
        assert_eq!(ext_for_audio_mime("audio/flac"), "flac");
        assert_eq!(ext_for_audio_mime("audio/webm"), "webm");
        assert_eq!(ext_for_audio_mime("application/octet-stream"), "bin");
    }

    #[tokio::test]
    async fn transcriptions_posts_multipart_fields_and_parses_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_string_contains("name=\"file\""))
            .and(body_string_contains("name=\"model\""))
            .and(body_string_contains("name=\"response_format\""))
            .and(body_string_contains("json"))
            .and(body_string_contains("name=\"language\""))
            .and(body_string_contains("name=\"prompt\""))
            .and(body_string_contains("name=\"temperature\""))
            .and(body_string_contains("0.2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "hello world"})))
            .expect(1)
            .mount(&server)
            .await;

        let request = asr(Some("en"), Some("technical terms"), json!({"temperature": 0.2}));
        let call = call(&server.uri(), "whisper-1", request);
        let out = OpenAiAudioTranscriptionsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello world");
        assert_eq!(language.as_deref(), Some("en"));
        assert_eq!(model.as_deref(), Some("whisper-1"));
    }

    #[tokio::test]
    async fn transcriptions_omits_optional_fields_when_absent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "hi"})))
            .expect(1)
            .mount(&server)
            .await;

        let call = call(&server.uri(), "whisper-1", asr(None, None, json!({})));
        let out = OpenAiAudioTranscriptionsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { language, .. }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(language, None);

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(!body.contains("name=\"language\""), "language must be omitted");
        assert!(!body.contains("name=\"prompt\""), "prompt must be omitted");
        assert!(!body.contains("name=\"temperature\""), "temperature must be omitted");
    }

    #[tokio::test]
    async fn versioned_base_url_is_normalized_once() {
        // Ported from stt_openai.rs: a base already carrying `/v1` (StepFun
        // style) must not gain a second version segment.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "hi"})))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let call = call(&base, "step-asr", asr(None, None, json!({})));
        OpenAiAudioTranscriptionsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn full_transcription_url_is_preserved() {
        // Ported from stt_openai.rs: an is_full_url base is the endpoint.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/transcribe"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "hi"})))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = call(&format!("{}/custom/transcribe", server.uri()), "whisper-1", asr(None, None, json!({})));
        call.connection.is_full_url = true;
        OpenAiAudioTranscriptionsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "whisper-1", asr(None, None, json!({})));
        let err = OpenAiAudioTranscriptionsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }

    // -- openai.audio_speech --------------------------------------------------

    use wiremock::matchers::body_partial_json;

    use crate::types::{ProducedData, TtsRequest};

    fn tts(text: &str, voice: Option<&str>, format: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.into(),
            voice: voice.map(str::to_string),
            format: format.map(str::to_string),
            extra: json!({}),
        })
    }

    #[test]
    fn speech_format_mime_mapping() {
        assert_eq!(mime_for_speech_format("mp3"), "audio/mpeg");
        assert_eq!(mime_for_speech_format("wav"), "audio/wav");
        assert_eq!(mime_for_speech_format("opus"), "audio/ogg");
        assert_eq!(mime_for_speech_format("aac"), "audio/aac");
        assert_eq!(mime_for_speech_format("flac"), "audio/flac");
        assert_eq!(mime_for_speech_format("pcm"), "audio/pcm");
        assert_eq!(mime_for_speech_format("something-else"), "audio/mpeg");
    }

    #[tokio::test]
    async fn speech_posts_json_with_default_voice_and_returns_binary_asset() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "tts-1",
                "input": "hello world",
                "voice": "alloy",
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3fake-mp3-bytes".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = call(&server.uri(), "tts-1", tts("hello world", None, None));
        let out = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"ID3fake-mp3-bytes"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));

        // No format requested → response_format must be omitted from the body.
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("response_format").is_none(), "response_format must be omitted");
    }

    #[tokio::test]
    async fn speech_passes_voice_and_format_and_maps_format_mime() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(body_partial_json(json!({
                "model": "tts-1",
                "input": "hi",
                "voice": "nova",
                "response_format": "wav",
            })))
            // A deliberately wrong Content-Type: the requested format wins.
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/octet-stream")
                    .set_body_bytes(b"RIFFwav".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = call(&server.uri(), "tts-1", tts("hi", Some("nova"), Some("wav")));
        let out = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn speech_without_format_trusts_audio_content_type_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/wav; charset=binary")
                    .set_body_bytes(b"RIFFwav".to_vec()),
            )
            .mount(&server)
            .await;

        let call = call(&server.uri(), "tts-1", tts("hi", None, None));
        let out = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn speech_without_format_and_non_audio_content_type_defaults_to_mpeg() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/octet-stream")
                    .set_body_bytes(b"bytes".to_vec()),
            )
            .mount(&server)
            .await;

        let call = call(&server.uri(), "tts-1", tts("hi", None, None));
        let out = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));
    }

    #[tokio::test]
    async fn speech_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "tts-1", tts("hi", None, None));
        let err = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }

    #[tokio::test]
    async fn speech_rejects_non_tts_request_locally() {
        let call = call("http://127.0.0.1:9", "tts-1", asr(None, None, json!({})));
        let err = OpenAiAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }
}
