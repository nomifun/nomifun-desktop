//! Native StepFun audio protocols shared by the pay-as-you-go `/v1` API and
//! the subscription `/step_plan/v1` gateway.
//!
//! This module deliberately does not implement StepFun's bidirectional
//! Realtime WebSocket protocols. [`ProtocolAdapter`] models one-shot and
//! pollable invocations; forcing a session-oriented socket into that contract
//! would lose turn/session semantics.
//!
//! Implemented protocol ids:
//! - [`StepFunAudioSpeechAdapter`] (`"stepfun.audio_speech"`): JSON
//!   `/audio/speech`, accepting provider-native request options and normalizing
//!   binary, `return_url` JSON and `stream_format=sse` responses.
//! - [`StepFunAsrSseAdapter`] (`"stepfun.asr_sse"`): JSON + Base64 audio to
//!   `/audio/asr/sse`, consuming the server-sent transcript events. The same
//!   wire format is available on regular and Step Plan bases.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Map, Value};

use super::provider_body_fields;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    decode_b64, encode_b64, error_from_response, read_body_capped, send_with_rotation,
    MAX_ARTIFACT_BYTES,
};
use crate::types::{
    AsrRequest, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest,
};

pub const AUDIO_SPEECH_ADAPTER_ID: &str = "stepfun.audio_speech";
pub const ASR_SSE_ADAPTER_ID: &str = "stepfun.asr_sse";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// StepFun JSON `/audio/speech` protocol.
pub struct StepFunAudioSpeechAdapter;

/// StepFun HTTP + SSE speech-recognition protocol.
pub struct StepFunAsrSseAdapter;

#[async_trait]
impl ProtocolAdapter for StepFunAudioSpeechAdapter {
    fn id(&self) -> &'static str {
        AUDIO_SPEECH_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{AUDIO_SPEECH_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };

        let url = call.endpoint_url()?;
        let body = build_tts_body(call, req)?;
        let wants_sse = body.get("stream_format").and_then(Value::as_str) == Some("sse");

        let resp = send_with_rotation(&call.connection.auth, || {
            let mut builder = http.post(&url).timeout(REQUEST_TIMEOUT).json(&body);
            if wants_sse {
                builder = builder.header(ACCEPT, "text/event-stream");
            }
            Ok(builder)
        })
        .await?;

        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }

        let content_type = response_content_type(&resp);
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;
        let format = body
            .get("response_format")
            .and_then(Value::as_str)
            .unwrap_or("mp3");
        let result = parse_tts_response(&bytes, content_type.as_deref(), wants_sse, format)?;
        Ok(TaskOutcome::Done(TaskResult::Assets(vec![result])))
    }
}

#[async_trait]
impl ProtocolAdapter for StepFunAsrSseAdapter {
    fn id(&self) -> &'static str {
        ASR_SSE_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{ASR_SSE_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };

        let url = call.endpoint_url()?;
        let body = build_asr_sse_body(call, req)?;
        let resp = send_with_rotation(&call.connection.auth, || {
            Ok(http
                .post(&url)
                .timeout(REQUEST_TIMEOUT)
                .header(ACCEPT, "text/event-stream")
                .json(&body))
        })
        .await?;

        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }

        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;
        let text = parse_asr_sse(&bytes)?;
        Ok(TaskOutcome::Done(TaskResult::Transcript {
            text,
            language: req.language.clone(),
            model: Some(call.model.clone()),
        }))
    }
}

/// Recursively merge JSON objects. Arrays and scalar values replace the old
/// value. Used only for provider request options; typed task fields are written
/// after merging and therefore cannot be overridden by opaque extras.
fn merge_object(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, value) in patch {
        if let (Some(Value::Object(existing)), Value::Object(incoming)) = (target.get_mut(key), value) {
            merge_object(existing, incoming);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// Merge flat provider-native options while stripping local routing/auth
/// metadata. Nested objects remain provider-native and merge recursively.
fn merge_provider_object(target: &mut Map<String, Value>, source: &Value) {
    for (key, value) in provider_body_fields(source) {
        if let (Some(Value::Object(existing)), Value::Object(incoming)) =
            (target.get_mut(key), value)
        {
            merge_object(existing, incoming);
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// Apply configured flat fields first, then per-request flat fields.
/// Transport-only keys are never put on the provider wire.
fn merge_request_options(target: &mut Map<String, Value>, configured: &Value, extra: &Value) {
    merge_provider_object(target, configured);
    merge_provider_object(target, extra);
}

fn non_empty(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

fn build_tts_body(call: &ResolvedCall, req: &TtsRequest) -> Result<Value, InvokeError> {
    let mut body = Map::new();
    merge_request_options(&mut body, &call.model_params, &req.extra);

    // StepFun requires a provider voice. Do not inherit OpenAI's `alloy`
    // default: it is not a StepFun voice id.
    if let Some(voice) = req.voice.as_deref().map(str::trim).filter(|voice| !voice.is_empty()) {
        body.insert("voice".into(), Value::String(voice.to_owned()));
    }
    if non_empty(body.get("voice")).is_none() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "StepFun TTS requires a non-empty provider voice id",
        ));
    }

    if let Some(format) = req.format.as_deref().map(str::trim).filter(|format| !format.is_empty()) {
        body.insert("response_format".into(), Value::String(format.to_owned()));
    }

    // Typed task identity always wins over opaque provider options.
    body.insert("model".into(), Value::String(call.model.clone()));
    body.insert("input".into(), Value::String(req.text.clone()));
    Ok(Value::Object(body))
}

fn take_object(map: &mut Map<String, Value>, key: &str) -> Map<String, Value> {
    map.remove(key).and_then(|value| value.as_object().cloned()).unwrap_or_default()
}

fn merge_value_object(target: &mut Map<String, Value>, value: Option<Value>) {
    if let Some(Value::Object(object)) = value {
        merge_object(target, &object);
    }
}

fn build_asr_sse_body(call: &ResolvedCall, req: &AsrRequest) -> Result<Value, InvokeError> {
    let mut root = Map::new();
    merge_request_options(&mut root, &call.model_params, &req.extra);

    let mut audio = take_object(&mut root, "audio");
    let mut input = take_object(&mut audio, "input");
    let mut transcription = take_object(&mut input, "transcription");
    let mut format = take_object(&mut input, "format");

    // Convenience shapes are accepted in addition to the exact official
    // `audio.input.{transcription,format}` nesting.
    merge_value_object(&mut transcription, root.remove("transcription"));
    merge_value_object(&mut transcription, root.remove("asr_options"));
    merge_value_object(&mut format, root.remove("format"));
    merge_value_object(&mut format, root.remove("audio_format"));

    for key in ["language", "hotwords", "prompt", "enable_itn"] {
        if let Some(value) = root.remove(key) {
            transcription.insert(key.into(), value);
        }
    }
    for (alias, canonical) in [
        ("sample_rate", "rate"),
        ("bit_depth", "bits"),
        ("channels", "channel"),
    ] {
        if let Some(value) = root.remove(alias) {
            format.insert(canonical.into(), value);
        }
    }

    if let Some(language) = req.language.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        transcription.insert("language".into(), Value::String(language.to_owned()));
    }
    if let Some(prompt) = req.prompt.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        transcription.insert("prompt".into(), Value::String(prompt.to_owned()));
    }
    transcription.insert("model".into(), Value::String(call.model.clone()));

    if non_empty(format.get("type")).is_none() {
        format.insert("type".into(), Value::String(asr_container_from_mime(&req.audio.mime)?.into()));
    }
    validate_pcm_format(&format)?;

    input.insert("transcription".into(), Value::Object(transcription));
    input.insert("format".into(), Value::Object(format));
    audio.insert("data".into(), Value::String(encode_b64(&req.audio.bytes)));
    audio.insert("input".into(), Value::Object(input));
    root.insert("audio".into(), Value::Object(audio));
    Ok(Value::Object(root))
}

fn asr_container_from_mime(mime: &str) -> Result<&'static str, InvokeError> {
    let mime = mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase();
    match mime.as_str() {
        "audio/mpeg" | "audio/mp3" => Ok("mp3"),
        "audio/ogg" => Ok("ogg"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("wav"),
        "audio/pcm" | "audio/l16" | "application/octet-stream" => Ok("pcm"),
        _ => Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "StepFun ASR cannot infer an official audio format from MIME {mime:?}; set extra.format.type"
            ),
        )),
    }
}

fn validate_pcm_format(format: &Map<String, Value>) -> Result<(), InvokeError> {
    if non_empty(format.get("type")) != Some("pcm") {
        return Ok(());
    }
    let missing: Vec<&str> = ["codec", "rate", "bits", "channel"]
        .into_iter()
        .filter(|key| !format.contains_key(*key))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("StepFun PCM ASR requires format fields: {}", missing.join(", ")),
        ))
    }
}

fn response_content_type(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase())
}

fn mime_for_audio_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "opus" => "audio/opus",
        "pcm" => "audio/pcm",
        _ => "audio/mpeg",
    }
}

fn looks_like_json(bytes: &[u8]) -> bool {
    matches!(bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()), Some(b'{') | Some(b'['))
}

fn parse_tts_response(
    bytes: &[u8],
    content_type: Option<&str>,
    wants_sse: bool,
    format: &str,
) -> Result<ProducedAsset, InvokeError> {
    if wants_sse || content_type == Some("text/event-stream") {
        let audio = parse_tts_sse(bytes)?;
        return Ok(ProducedAsset {
            data: ProducedData::Bytes(audio),
            mime: Some(mime_for_audio_format(format).into()),
        });
    }

    if content_type == Some("application/json") || looks_like_json(bytes) {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| InvokeError::parse(format!("invalid StepFun TTS JSON: {error}")))?;
        if let Some(error) = response_error(&value, "StepFun TTS") {
            return Err(error);
        }
        if let Some(url) = value
            .get("data")
            .and_then(|data| data.get("url"))
            .and_then(Value::as_str)
            .or_else(|| value.get("url").and_then(Value::as_str))
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            return Ok(ProducedAsset {
                data: ProducedData::Url(url.to_owned()),
                mime: Some(mime_for_audio_format(format).into()),
            });
        }
        if let Some(encoded) = value
            .get("data")
            .and_then(|data| data.get("audio"))
            .and_then(Value::as_str)
            .or_else(|| value.get("audio").and_then(Value::as_str))
        {
            let decoded = decode_b64(encoded)
                .filter(|decoded| !decoded.is_empty())
                .ok_or_else(|| InvokeError::parse("StepFun TTS JSON audio is not valid Base64"))?;
            return Ok(ProducedAsset {
                data: ProducedData::Bytes(decoded),
                mime: Some(mime_for_audio_format(format).into()),
            });
        }
        return Err(InvokeError::parse("StepFun TTS JSON contains neither data.url nor Base64 audio"));
    }

    if bytes.is_empty() {
        return Err(InvokeError::parse("StepFun TTS returned an empty audio body"));
    }
    let mime = content_type
        .filter(|content_type| content_type.starts_with("audio/"))
        .unwrap_or_else(|| mime_for_audio_format(format));
    Ok(ProducedAsset { data: ProducedData::Bytes(bytes.to_vec()), mime: Some(mime.to_owned()) })
}

#[derive(Debug)]
struct SseJsonEvent {
    name: Option<String>,
    data: Value,
}

fn parse_sse_json(bytes: &[u8], context: &str) -> Result<Vec<SseJsonEvent>, InvokeError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| InvokeError::parse(format!("{context} SSE is not UTF-8: {error}")))?;
    let mut current_name: Option<String> = None;
    let mut events = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            current_name = None;
            continue;
        }
        if let Some(name) = line.strip_prefix("event:") {
            current_name = Some(name.trim().to_owned());
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value = serde_json::from_str(data)
            .map_err(|error| InvokeError::parse(format!("invalid {context} SSE JSON: {error}")))?;
        events.push(SseJsonEvent { name: current_name.clone(), data: value });
    }
    Ok(events)
}

fn event_type<'a>(event: &'a SseJsonEvent) -> Option<&'a str> {
    event.data.get("type").and_then(Value::as_str).or(event.name.as_deref())
}

fn parse_tts_sse(bytes: &[u8]) -> Result<Vec<u8>, InvokeError> {
    let events = parse_sse_json(bytes, "StepFun TTS")?;
    let mut audio = Vec::new();
    for event in events {
        match event_type(&event) {
            Some("speech.audio.delta") => {
                let encoded = event
                    .data
                    .get("audio")
                    .and_then(Value::as_str)
                    .ok_or_else(|| InvokeError::parse("StepFun TTS audio delta has no audio field"))?;
                let decoded = decode_b64(encoded)
                    .ok_or_else(|| InvokeError::parse("StepFun TTS audio delta is not valid Base64"))?;
                audio.extend_from_slice(&decoded);
            }
            Some("speech.audio.error") | Some("error") => {
                return Err(error_from_event(&event.data, "StepFun TTS SSE"));
            }
            // `response.subtitle` cannot be represented by the current
            // TaskResult; it is intentionally ignored while audio is retained.
            _ => {}
        }
    }
    if audio.is_empty() {
        Err(InvokeError::parse("StepFun TTS SSE produced no audio chunks"))
    } else {
        Ok(audio)
    }
}

fn parse_asr_sse(bytes: &[u8]) -> Result<String, InvokeError> {
    let events = parse_sse_json(bytes, "StepFun ASR")?;
    let mut deltas = String::new();
    let mut final_text: Option<String> = None;
    for event in events {
        match event_type(&event) {
            Some("transcript.text.delta") => {
                if let Some(delta) = event.data.get("delta").and_then(Value::as_str) {
                    deltas.push_str(delta);
                }
            }
            Some("transcript.text.done") => {
                if let Some(text) = event.data.get("text").and_then(Value::as_str) {
                    final_text = Some(text.to_owned());
                }
            }
            Some("error") | Some("transcript.text.error") => {
                return Err(error_from_event(&event.data, "StepFun ASR SSE"));
            }
            _ => {}
        }
    }

    final_text
        .filter(|text| !text.is_empty())
        .or_else(|| (!deltas.is_empty()).then_some(deltas))
        .ok_or_else(|| InvokeError::parse("StepFun ASR SSE produced no transcript"))
}

fn response_error(value: &Value, context: &str) -> Option<InvokeError> {
    value.get("error").map(|error| error_from_event(error, context))
}

fn error_from_event(value: &Value, context: &str) -> InvokeError {
    let nested = value.get("error").unwrap_or(value);
    let code = nested
        .get("code")
        .and_then(|code| code.as_str().map(str::to_owned).or_else(|| Some(code.to_string())))
        .unwrap_or_default();
    let message = nested
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("provider emitted an error event");
    let signal = format!("{code} {message}").to_ascii_lowercase();
    let kind = if signal.contains("quota") || signal.contains("insufficient_balance") {
        InvokeErrorKind::QuotaExhausted
    } else if signal.contains("rate") && signal.contains("limit") {
        InvokeErrorKind::RateLimited
    } else if signal.contains("auth") || signal.contains("api_key") || signal.contains("api key") {
        InvokeErrorKind::Auth
    } else if signal.contains("invalid") || signal.contains("parameter") || signal.contains("argument") {
        InvokeErrorKind::InvalidParams
    } else if signal.contains("content") && (signal.contains("policy") || signal.contains("filter")) {
        InvokeErrorKind::ContentPolicy
    } else {
        InvokeErrorKind::ProviderError
    };
    InvokeError::new(kind, format!("{context}: {message}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::{InputAsset, ProducedData};

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn tts(extra: Value) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: "你好，StepFun".into(),
            voice: Some("cixingnansheng".into()),
            format: Some("wav".into()),
            extra,
        })
    }

    fn asr(mime: &str, extra: Value) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".into(),
                bytes: b"RIFF-fake-audio".to_vec(),
                mime: mime.into(),
            },
            language: Some("zh".into()),
            prompt: Some("Nomifun 是产品名".into()),
            extra,
        })
    }

    fn stepfun_call_with_endpoint(
        base: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let mut call = call_with_endpoint(base, model, protocol, endpoint, request);
        call.platform = "stepfun".into();
        call
    }

    fn tts_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        stepfun_call_with_endpoint(base, model, "stepfun.audio_speech", "/audio/speech", request)
    }

    fn asr_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        stepfun_call_with_endpoint(base, model, "stepfun.asr_sse", "/audio/asr/sse", request)
    }

    #[test]
    fn adapters_advertise_only_their_audio_task() {
        assert!(StepFunAudioSpeechAdapter.supports(ModelTask::SpeechSynthesis));
        assert!(!StepFunAudioSpeechAdapter.supports(ModelTask::SpeechRecognition));
        assert!(StepFunAsrSseAdapter.supports(ModelTask::SpeechRecognition));
        assert!(!StepFunAsrSseAdapter.supports(ModelTask::SpeechSynthesis));
    }

    #[test]
    fn transport_metadata_never_enters_stepfun_audio_body() {
        let mut call = tts_call(
            "https://api.stepfun.com/v1",
            "stepaudio-2.5-tts",
            tts(json!({
                "temperature": 0.15,
                "base_url": "https://request-secret.example/v1",
                "pitch": 1.05,
                "api_keys": ["request-secret"]
            })),
        );
        call.model_params = json!({
            "base_url": "https://route-secret.example/v1",
            "allow_cross_origin_credentials": true,
            "endpoint": "/route-only",
            "connection_role": "voice",
            "api_key": "model-secret",
            "headers": {"x-api-key": "model-secret"},
            "speed": 0.8,
            "sample_rate": 24000,
            "bitrate": 128000,
            "credentials": {"api_keys": ["body-secret"]}
        });

        let TaskRequest::SpeechSynthesis(request) = &call.request else {
            unreachable!()
        };
        let body = build_tts_body(&call, request).unwrap();

        assert_eq!(body["speed"], 0.8);
        assert_eq!(body["sample_rate"], 24000);
        assert_eq!(body["bitrate"], 128000);
        assert_eq!(body["temperature"], 0.15);
        assert_eq!(body["pitch"], 1.05);
        for key in crate::adapters::LOCAL_TRANSPORT_PARAM_KEYS {
            assert!(body.get(key).is_none(), "local key {key} leaked into StepFun body: {body}");
        }
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(!serialized.contains("secret"), "credential material leaked: {serialized}");
    }

    #[tokio::test]
    async fn tts_posts_plan_json_merges_params_and_returns_binary() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/step_plan/v1/audio/speech"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "stepaudio-2.5-tts",
                "input": "你好，StepFun",
                "voice": "cixingnansheng",
                "response_format": "wav",
                "speed": 1.25,
                "sample_rate": 24000,
                "instruction": "沉稳且温暖",
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/wav")
                    .set_body_bytes(b"RIFF-result".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/step_plan/v1", server.uri());
        let mut call = tts_call(
            &base,
            "stepaudio-2.5-tts",
            tts(json!({"instruction": "沉稳且温暖", "speed": 1.25})),
        );
        call.model_params = json!({
            "endpoint": "/audio/speech",
            "sample_rate": 24000,
            "speed": 0.8
        });
        let result = StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = result else { panic!("expected audio assets") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"RIFF-result"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn tts_parses_return_url_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "created": 1,
                "data": {"url": "https://cdn.example/speech.mp3", "subtitles": []}
            })))
            .mount(&server)
            .await;
        let mut request = tts(json!({"return_url": true}));
        let TaskRequest::SpeechSynthesis(ref mut typed) = request else { unreachable!() };
        typed.format = Some("mp3".into());
        let call = tts_call(&format!("{}/v1", server.uri()), "step-tts-mini", request);
        let result = StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = result else { panic!("expected audio assets") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://cdn.example/speech.mp3"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));
    }

    #[tokio::test]
    async fn tts_decodes_sse_audio_and_ignores_subtitles() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"type\":\"speech.audio.delta\",\"audio\":\"aGVs\"}\n\n",
            "data: {\"type\":\"response.subtitle\",\"data\":{\"text\":\"hello\"}}\n\n",
            "data: {\"type\":\"speech.audio.delta\",\"audio\":\"bG8=\"}\n\n",
            "data: {\"type\":\"speech.audio.done\",\"audio\":\"\"}\n\n",
            "data: [DONE]\n\n",
        );
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("accept", "text/event-stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .mount(&server)
            .await;
        let call = tts_call(
            &format!("{}/v1", server.uri()),
            "stepaudio-2.5-tts",
            tts(json!({"stream_format": "sse", "timestamp": true})),
        );
        let result = StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = result else { panic!("expected audio assets") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hello"));
    }

    #[tokio::test]
    async fn tts_requires_stepfun_voice_without_sending_request() {
        let mut request = tts(json!({}));
        let TaskRequest::SpeechSynthesis(ref mut typed) = request else { unreachable!() };
        typed.voice = None;
        let call = tts_call("http://127.0.0.1:9/v1", "step-tts-mini", request);
        let error = StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("voice"));
    }

    #[tokio::test]
    async fn endpoint_override_wins_for_tts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"audio".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let call = stepfun_call_with_endpoint(
            &server.uri(),
            "step-tts-mini",
            "stepfun.audio_speech",
            "/custom/speech",
            tts(json!({})),
        );
        StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn plan_asr_posts_nested_base64_json_and_prefers_done_text() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"type\":\"transcript.text.delta\",\"delta\":\"错的\"}\r\n\r\n",
            "data: {\"type\":\"transcript.text.done\",\"text\":\"最终文本\"}\r\n\r\n",
        );
        Mock::given(method("POST"))
            .and(path("/step_plan/v1/audio/asr/sse"))
            .and(header("authorization", "Bearer sk-test"))
            .and(header("accept", "text/event-stream"))
            .and(body_partial_json(json!({
                "audio": {
                    "data": "UklGRi1mYWtlLWF1ZGlv",
                    "input": {
                        "transcription": {
                            "model": "stepaudio-2.5-asr",
                            "language": "zh",
                            "prompt": "Nomifun 是产品名",
                            "hotwords": ["Nomifun"],
                            "enable_itn": true
                        },
                        "format": {"type": "wav"}
                    }
                }
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .expect(1)
            .mount(&server)
            .await;
        let base = format!("{}/step_plan/v1", server.uri());
        let mut call = asr_call(
            &base,
            "stepaudio-2.5-asr",
            asr("audio/wav", json!({"hotwords": ["Nomifun"]})),
        );
        call.platform = "stepfun-plan".into();
        call.model_params = json!({
            "endpoint": "/audio/asr/sse",
            "enable_itn": true
        });

        let result = StepFunAsrSseAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = result else {
            panic!("expected transcript")
        };
        assert_eq!(text, "最终文本");
        assert_eq!(language.as_deref(), Some("zh"));
        assert_eq!(model.as_deref(), Some("stepaudio-2.5-asr"));
    }

    #[tokio::test]
    async fn regular_asr_uses_same_sse_protocol_and_delta_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/asr/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_string(concat!(
                "event: transcript.text.delta\n",
                "data: {\"delta\":\"你\"}\n\n",
                "event: transcript.text.delta\n",
                "data: {\"delta\":\"好\"}\n\n",
            )))
            .mount(&server)
            .await;
        let call = asr_call(
            &format!("{}/v1", server.uri()),
            "stepaudio-2.5-asr",
            asr("audio/wav", json!({})),
        );
        let result = StepFunAsrSseAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, .. }) = result else { panic!("expected transcript") };
        assert_eq!(text, "你好");
    }

    #[tokio::test]
    async fn asr_sse_error_event_is_classified() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/asr/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "data: {\"type\":\"error\",\"code\":\"invalid_api_key\",\"message\":\"bad api key\"}\n\n",
            ))
            .mount(&server)
            .await;
        let call = asr_call(
            &format!("{}/v1", server.uri()),
            "stepaudio-2.5-asr",
            asr("audio/wav", json!({})),
        );
        let error = StepFunAsrSseAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Auth);
    }

    #[tokio::test]
    async fn http_errors_use_shared_classification() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(422).set_body_string("voice not found"))
            .mount(&server)
            .await;
        let call = tts_call(&format!("{}/v1", server.uri()), "step-tts-mini", tts(json!({})));
        let error = StepFunAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert_eq!(error.http_status, Some(422));
    }

    #[tokio::test]
    async fn pcm_asr_requires_explicit_wire_format() {
        let call = asr_call(
            "http://127.0.0.1:9/v1",
            "stepaudio-2.5-asr",
            asr("audio/pcm", json!({"format": {"type": "pcm", "rate": 16000}})),
        );
        let error = StepFunAsrSseAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("codec"));
        assert!(error.message.contains("bits"));
        assert!(error.message.contains("channel"));
    }

    #[tokio::test]
    async fn asr_endpoint_override_wins() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/asr"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "data: {\"type\":\"transcript.text.done\",\"text\":\"ok\"}\n\n",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let call = stepfun_call_with_endpoint(
            &server.uri(),
            "stepaudio-2.5-asr",
            "stepfun.asr_sse",
            "/custom/asr",
            asr("audio/wav", json!({})),
        );
        StepFunAsrSseAdapter.submit(&test_http(), &call).await.unwrap();
    }
}
