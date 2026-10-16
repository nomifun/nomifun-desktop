//! GLM-TTS synchronous native audio protocol.
//! https://docs.bigmodel.cn/api-reference/模型-api/文本转语音.md
//! Native defaults belong to this serializer, so clearing optional JSON never
//! reintroduces OpenAI's incompatible voice or output format defaults.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use super::json_request_body;
use super::audio_payload::validate_audio_payload;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_json, read_body_capped, response_secret_redactor};
use crate::types::{ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest};

const PROTOCOL_ID: &str = "zhipu.audio_speech";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_AUDIO_BYTES: u64 = 32 * 1024 * 1024;

pub struct ZhipuAudioSpeechAdapter;

#[async_trait]
impl ProtocolAdapter for ZhipuAudioSpeechAdapter {
    fn id(&self) -> &'static str {
        PROTOCOL_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(request) = &call.request else {
            return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "zhipu.audio_speech requires speech_synthesis"));
        };
        let body = request_body(call, request)?;
        let response = post_json(http, &call.endpoint_url()?, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        // A JSON error or SSE stream must never be persisted as an audio file.
        let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase());
        if content_type.as_deref().is_some_and(|mime| !mime.starts_with("audio/") && mime != "application/octet-stream") {
            return Err(InvokeError::parse("zhipu.audio_speech expected a synchronous binary audio response"));
        }
        let redactor = response_secret_redactor(&response);
        let bytes = read_body_capped(response, MAX_AUDIO_BYTES).await.map_err(|error| error.redacted(&redactor))?;
        if bytes.is_empty() {
            return Err(InvokeError::parse("zhipu.audio_speech returned empty audio"));
        }
        validate_audio_payload(&bytes,content_type.as_deref(),PROTOCOL_ID)?;
        let mime = match body["response_format"].as_str() {
            Some("pcm") => "audio/pcm",
            _ => "audio/wav",
        };
        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes), mime: Some(mime.to_owned()),
        }])))
    }
}

fn invalid(message: &'static str) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

fn request_body(call: &ResolvedCall, request: &TtsRequest) -> Result<Value, InvokeError> {
    if call.model.trim().is_empty() || request.text.trim().is_empty() || request.text.chars().count() > 1024 {
        return Err(invalid("zhipu.audio_speech requires a model and 1-1024 input characters"));
    }
    let mut typed = json!({"model": call.model, "input": request.text});
    if let Some(voice) = &request.voice { typed["voice"] = Value::String(voice.clone()); }
    if let Some(format) = &request.format { typed["response_format"] = Value::String(format.clone()); }
    let mut body = json_request_body(&call.model_params, &request.extra, typed)?;
    let fields = body.as_object_mut().expect("shared JSON request body is an object");
    if fields.keys().any(|key| !matches!(key.as_str(),
        "model" | "input" | "voice" | "response_format" | "stream" | "speed" | "volume" | "watermark_enabled")) {
        return Err(invalid("zhipu.audio_speech accepts only native synchronous speech fields; streaming encode_format is unsupported"));
    }
    if fields.get("stream").is_some_and(|value| value != &Value::Bool(false)) {
        return Err(invalid("zhipu.audio_speech supports stream=false only"));
    }
    fields.insert("stream".to_owned(), Value::Bool(false));
    let voice = fields.entry("voice").or_insert_with(|| Value::String("tongtong".to_owned()));
    // Official system voices coexist with provider-created cloned voices.
    // Preserve any explicit non-empty string rather than inventing a list.
    if !voice.as_str().is_some_and(|voice| !voice.trim().is_empty()) {
        return Err(invalid("zhipu.audio_speech voice must be a non-empty string"));
    }
    let format = fields.entry("response_format").or_insert_with(|| Value::String("wav".to_owned()));
    if !matches!(format.as_str(), Some("wav" | "pcm")) {
        return Err(invalid("zhipu.audio_speech response_format must be wav or pcm"));
    }
    if fields.get("speed").is_some_and(|value| !value.as_f64().is_some_and(|value| value.is_finite() && (0.5..=2.0).contains(&value))) {
        return Err(invalid("zhipu.audio_speech speed must be between 0.5 and 2"));
    }
    if fields.get("volume").is_some_and(|value| !value.as_f64().is_some_and(|value| value.is_finite() && value > 0.0 && value <= 10.0)) {
        return Err(invalid("zhipu.audio_speech volume must be greater than 0 and at most 10"));
    }
    if fields.get("watermark_enabled").is_some_and(|value| !value.is_boolean()) {
        return Err(invalid("zhipu.audio_speech watermark_enabled must be a boolean"));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use crate::adapters::test_support::call_with_endpoint;

    fn call(base: &str, voice: Option<&str>, format: Option<&str>, extra: Value) -> ResolvedCall {
        call_with_endpoint(base, "glm-tts", PROTOCOL_ID, "/api/paas/v4/audio/speech", TaskRequest::SpeechSynthesis(TtsRequest {
            text: "你好".to_owned(), voice: voice.map(str::to_owned), format: format.map(str::to_owned), extra,
        }))
    }

    fn audio_reply() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(b"native-audio".to_vec(), "audio/wav")
    }

    async fn sent_body(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        serde_json::from_slice(&requests[0].body).unwrap()
    }

    #[tokio::test]
    async fn empty_optional_params_use_native_voice_format_and_synchronous_defaults() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/paas/v4/audio/speech"))
            .and(header("authorization", "Bearer sk-test")).respond_with(audio_reply()).expect(1).mount(&server).await;
        let outcome = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), None, None, json!({}))).await.unwrap();
        assert_eq!(sent_body(&server).await, json!({"model":"glm-tts","input":"你好","voice":"tongtong","response_format":"wav","stream":false}));
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected binary audio"); };
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"native-audio"));
    }

    #[tokio::test]
    async fn configured_clone_voice_and_pcm_are_preserved_with_owned_model_and_input() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(audio_reply()).expect(1).mount(&server).await;
        let mut call = call(&server.uri(), None, None, json!({"model":"wrong-model","input":"wrong-input","volume":2}));
        call.model_params = json!({"endpoint":"/api/paas/v4/audio/speech","voice":"cloned-provider-voice","response_format":"pcm",
            "speed":0.5,"volume":1,"watermark_enabled":true,"api_key":"must-not-be-sent","headers":{"authorization":"wrong"}});
        let outcome = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert_eq!(sent_body(&server).await, json!({"model":"glm-tts","input":"你好","voice":"cloned-provider-voice","response_format":"pcm",
            "stream":false,"speed":0.5,"volume":2,"watermark_enabled":true}));
        assert!(matches!(outcome, TaskOutcome::Done(TaskResult::Assets(assets)) if assets[0].mime.as_deref() == Some("audio/pcm")));
    }

    #[tokio::test]
    async fn typed_values_win_over_extras_and_extras_win_over_configuration() {
        for (typed_voice, typed_format, expected_voice, expected_format) in [
            (Some("typed-clone"), Some("wav"), "typed-clone", "wav"),
            (None, None, "extra-clone", "pcm"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(audio_reply()).expect(1).mount(&server).await;
            let mut call = call(&server.uri(), typed_voice, typed_format, json!({"voice":"extra-clone","response_format":"pcm"}));
            call.model_params = json!({"endpoint":"/api/paas/v4/audio/speech","voice":"configured-clone","response_format":"wav"});
            ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
            let body = sent_body(&server).await;
            assert_eq!(body["voice"], expected_voice);
            assert_eq!(body["response_format"], expected_format);
            assert_eq!(body["stream"], false);
        }
    }

    #[tokio::test]
    async fn unsupported_options_are_rejected_before_any_http_request() {
        let server = MockServer::start().await;
        for (voice, format, extra) in [
            (Some(""), None, json!({})), (None, Some("mp3"), json!({})),
            (None, Some("flac"), json!({})), (None, None, json!({"response_format":"mp3"})),
            (None, None, json!({"voice":{ "id":"openai-voice" }})),
            (None, None, json!({"voice":null})), (None, None, json!({"stream":true})),
            (None, None, json!({"stream":"false"})), (None, None, json!({"encode_format":"base64"})),
            (None, None, json!({"instructions":"OpenAI-only option"})),
            (None, None, json!({"speed":0.4})), (None, None, json!({"speed":2.1})),
            (None, None, json!({"volume":0})), (None, None, json!({"volume":11})),
            (None, None, json!({"watermark_enabled":"true"})),
        ] {
            let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), voice, format, extra)).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        }
        for text in [String::new(), "中".repeat(1025)] {
            let mut call = call(&server.uri(), None, None, json!({}));
            let TaskRequest::SpeechSynthesis(request) = &mut call.request else { unreachable!() };
            request.text = text;
            let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn configured_invalid_format_and_streaming_are_not_hidden_by_defaults() {
        let server = MockServer::start().await;
        for configured in [json!({"response_format":"mp3"}), json!({"stream":true})] {
            let mut call = call(&server.uri(), None, None, json!({}));
            call.model_params = configured;
            call.model_params["endpoint"] = json!("/api/paas/v4/audio/speech");
            let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn binary_failures_and_non_audio_successes_never_become_audio_assets() {
        for response in [
            ResponseTemplate::new(200).set_body_raw(Vec::<u8>::new(), "audio/wav"),
            ResponseTemplate::new(200).set_body_json(json!({"error":{"message":"reflected sk-test"}})),
            ResponseTemplate::new(200).set_body_raw("{\"error\":\"sk-test\"}", "audio/wav"),
            ResponseTemplate::new(200).set_body_raw("<!doctype html><html>sk-test</html>", "audio/wav"),
            ResponseTemplate::new(200).set_body_raw("data: reflected sk-test\n\n", "text/event-stream"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(response).expect(1).mount(&server).await;
            let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), None, None, json!({}))).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::ParseError);
            assert!(!error.message.contains("sk-test"));
        }
    }

    #[tokio::test]
    async fn upstream_errors_are_redacted_and_audio_size_is_bounded() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)
            .set_body_json(json!({"error":{"code":"1000","message":"rejected sk-test"}}))).expect(1).mount(&server).await;
        let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), None, None, json!({}))).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Auth);
        assert_eq!(error.http_status, Some(401));
        assert!(!error.message.contains("sk-test"));

        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
            .set_body_raw(vec![0_u8; MAX_AUDIO_BYTES as usize + 1], "audio/wav")).expect(1).mount(&server).await;
        let error = ZhipuAudioSpeechAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), None, None, json!({}))).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("cap"));
    }
}
