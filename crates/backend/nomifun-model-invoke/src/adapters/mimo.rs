//! Xiaomi MiMo audio models use the chat-completions wire protocol rather
//! than the OpenAI `/audio/*` endpoints.
//!
//! - `mimo-v2.5-asr`: JSON `POST {base}/chat/completions` with one
//!   `input_audio` content part and top-level `asr_options`.
//! - `mimo-v2.5-tts*`: JSON `POST {base}/chat/completions` with the text to
//!   synthesize in an `assistant` message and top-level `audio` options. The
//!   response audio is base64 at `choices[0].message.audio.data`.
//!
//! The selected capability supplies the exact chat-completions endpoint, so a
//! custom gateway can use a non-standard route without adapter heuristics.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

use super::json_request_body;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_b64, encode_b64, error_from_response, post_json};
use crate::types::{ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult};

const ASR_ADAPTER_ID: &str = "mimo.chat_asr";
const TTS_ADAPTER_ID: &str = "mimo.chat_tts";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ASR_BASE64_BYTES: usize = 10 * 1024 * 1024;

fn audio_input_format(mime: &str) -> Result<&'static str, InvokeError> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/x-wav" | "audio/wave" => Ok("wav"),
        "audio/mpeg" | "audio/mp3" => Ok("mp3"),
        _ => Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("mimo-v2.5-asr only accepts WAV or MP3 audio, got {mime}"),
        )),
    }
}

fn output_mime(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "wav" => "audio/wav",
        "pcm" | "pcm16" => "audio/pcm",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "opus" => "audio/ogg",
        _ => "audio/mpeg",
    }
}

/// Recursively overlay JSON object values. This lets catalog-level defaults
/// such as `audio.speed` or `asr_options.language` coexist with request-level
/// overrides without dropping sibling fields.
fn merge_object(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (key, incoming) in source {
        merge_field(target, key, incoming);
    }
}

fn merge_field(target: &mut Map<String, Value>, key: &str, incoming: &Value) {
    if let (Some(Value::Object(existing)), Value::Object(patch)) = (target.get_mut(key), incoming) {
        merge_object(existing, patch);
    } else {
        target.insert(key.to_owned(), incoming.clone());
    }
}

fn merge_named_object(target: &mut Map<String, Value>, source: &Value, key: &str) {
    let Some(incoming) = source.get(key).and_then(Value::as_object) else {
        return;
    };
    let entry = target.entry(key.to_owned()).or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(existing) = entry {
        merge_object(existing, incoming);
    }
}

fn provider_or_parse_error(value: &Value, context: &str) -> InvokeError {
    let error = value.get("error");
    if let Some(error) = error {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("unknown provider error");
        let code = error.get("code").and_then(Value::as_str);
        let detail = code.map_or_else(|| message.to_owned(), |code| format!("{code}: {message}"));
        InvokeError::new(InvokeErrorKind::ProviderError, format!("MiMo {context} failed: {detail}"))
    } else {
        InvokeError::parse(format!("invalid MiMo {context} response: missing {context} result"))
    }
}

/// `mimo-v2.5-asr` over the MiMo chat-completions endpoint.
pub struct MiMoChatAsrAdapter;

#[async_trait]
impl ProtocolAdapter for MiMoChatAsrAdapter {
    fn id(&self) -> &'static str {
        ASR_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{ASR_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };

        let format = audio_input_format(&req.audio.mime)?;
        let encoded = encode_b64(&req.audio.bytes);
        if encoded.len() > MAX_ASR_BASE64_BYTES {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "mimo-v2.5-asr Base64 audio exceeds the 10 MiB provider limit",
            ));
        }

        let mut body = json_request_body(&call.model_params, &req.extra, json!({}))?
            .as_object()
            .cloned()
            .expect("shared JSON body helper always returns an object");
        merge_named_object(&mut body, &call.model_params, "asr_options");
        merge_named_object(&mut body, &req.extra, "asr_options");

        body.insert("model".into(), Value::String(call.model.clone()));
        body.insert(
            "messages".into(),
            json!([{
                "role": "user",
                "content": [{
                    "type": "input_audio",
                    "input_audio": {
                        "data": encoded,
                        "format": format,
                    }
                }]
            }]),
        );
        body.insert("stream".into(), Value::Bool(false));

        let language = req.language.as_deref().map(str::trim).filter(|value| !value.is_empty());
        if let Some(language) = language {
            let options = body.entry("asr_options").or_insert_with(|| json!({}));
            let Some(options) = options.as_object_mut() else {
                return Err(InvokeError::new(
                    InvokeErrorKind::InvalidParams,
                    "MiMo asr_options must be a JSON object",
                ));
            };
            options.insert("language".into(), Value::String(language.to_owned()));
        }

        let url = call.endpoint_url()?;
        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &Value::Object(body)).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|error| InvokeError::response_json("invalid MiMo ASR JSON response", &error))?;
        let Some(text) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.pointer("/message/content"))
            .and_then(Value::as_str)
        else {
            return Err(provider_or_parse_error(&value, "ASR"));
        };

        Ok(TaskOutcome::Done(TaskResult::Transcript {
            text: text.to_owned(),
            language: language.map(str::to_owned),
            model: Some(call.model.clone()),
        }))
    }
}

/// MiMo V2.5 TTS family over the MiMo chat-completions endpoint.
pub struct MiMoChatTtsAdapter;

#[async_trait]
impl ProtocolAdapter for MiMoChatTtsAdapter {
    fn id(&self) -> &'static str {
        TTS_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{TTS_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };

        let mut body = json_request_body(&call.model_params, &req.extra, json!({}))?
            .as_object()
            .cloned()
            .expect("shared JSON body helper always returns an object");
        merge_named_object(&mut body, &call.model_params, "audio");
        merge_named_object(&mut body, &req.extra, "audio");

        let mut messages = Vec::new();
        // Voice-design and voice-clone callers can provide preceding user or
        // preceding flat `messages`; the actual text to synthesize is always
        // the typed assistant message and wins.
        if let Some(configured) = body.remove("messages").and_then(|value| value.as_array().cloned()) {
            messages.extend(configured.into_iter().filter(|message| {
                message.get("role").and_then(Value::as_str) != Some("assistant")
            }));
        }
        messages.push(json!({"role": "assistant", "content": req.text}));

        let audio = body.entry("audio").or_insert_with(|| json!({}));
        let Some(audio) = audio.as_object_mut() else {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "MiMo audio options must be a JSON object",
            ));
        };
        let format = req
            .format
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| audio.get("format").and_then(Value::as_str))
            .unwrap_or("wav")
            .to_owned();
        audio.insert("format".into(), Value::String(format.clone()));
        if let Some(voice) = req
            .voice
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| call.model_params.get("voice").and_then(Value::as_str))
        {
            audio.insert("voice".into(), Value::String(voice.to_owned()));
        }

        body.insert("model".into(), Value::String(call.model.clone()));
        body.insert("messages".into(), Value::Array(messages));
        body.insert("stream".into(), Value::Bool(false));

        let url = call.endpoint_url()?;
        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &Value::Object(body)).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|error| InvokeError::response_json("invalid MiMo TTS JSON response", &error))?;
        let Some(data) = value.pointer("/choices/0/message/audio/data").and_then(Value::as_str) else {
            return Err(provider_or_parse_error(&value, "TTS"));
        };
        let bytes = decode_b64(data).filter(|bytes| !bytes.is_empty()).ok_or_else(|| {
            InvokeError::parse("invalid MiMo TTS response: choices[0].message.audio.data is not valid Base64")
        })?;

        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(output_mime(&format).to_owned()),
        }])))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::{AsrRequest, InputAsset, TtsRequest};

    fn mimo_base(base_url: &str) -> String {
        let base_url = base_url.trim_end_matches('/');
        if base_url.ends_with("/v1") {
            base_url.to_owned()
        } else {
            format!("{base_url}/v1")
        }
    }

    fn mimo_asr_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        call_with_endpoint(&mimo_base(base_url), model, "mimo.chat_asr", "/chat/completions", request)
    }

    fn mimo_tts_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        call_with_endpoint(&mimo_base(base_url), model, "mimo.chat_tts", "/chat/completions", request)
    }

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn asr(mime: &str, language: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".into(),
                bytes: b"RIFFdata".to_vec(),
                mime: mime.into(),
            },
            language: language.map(str::to_owned),
            prompt: None,
            extra: json!({}),
        })
    }

    fn tts(text: &str, voice: Option<&str>, format: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.into(),
            voice: voice.map(str::to_owned),
            format: format.map(str::to_owned),
            extra: json!({}),
        })
    }

    #[test]
    fn transport_metadata_never_enters_mimo_request_body() {
        let mut provider_params = serde_json::Map::new();
        for key in crate::adapters::LOCAL_TRANSPORT_PARAM_KEYS {
            provider_params.insert((*key).to_owned(), json!(format!("secret-{key}")));
        }
        provider_params.insert("temperature".into(), json!(0.3));
        provider_params.insert(
            "provider_options".into(),
            json!({"headers": {"x-provider-native": "allowed"}}),
        );

        let body = json_request_body(
            &Value::Object(provider_params),
            &json!({}),
            json!({}),
        )
        .unwrap();

        assert_eq!(body.get("temperature"), Some(&json!(0.3)));
        assert_eq!(
            body.get("provider_options"),
            Some(&json!({"headers": {"x-provider-native": "allowed"}}))
        );
        for key in crate::adapters::LOCAL_TRANSPORT_PARAM_KEYS {
            assert!(body.get(*key).is_none(), "local key {key} leaked into MiMo body");
        }
    }

    #[tokio::test]
    async fn asr_posts_chat_json_and_parses_transcript() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "mimo-v2.5-asr",
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "input_audio": {"data": "UklGRmRhdGE=", "format": "wav"}
                    }]
                }],
                "asr_options": {"language": "en"},
                "stream": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "hello world"}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let call = mimo_asr_call(&base, "mimo-v2.5-asr", asr("audio/wav", Some("en")));
        let out = MiMoChatAsrAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello world");
        assert_eq!(language.as_deref(), Some("en"));
        assert_eq!(model.as_deref(), Some("mimo-v2.5-asr"));
    }

    #[tokio::test]
    async fn tts_posts_assistant_text_merges_audio_params_and_decodes_base64() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "model": "mimo-v2.5-tts",
                "messages": [{"role": "assistant", "content": "hello"}],
                "audio": {"format": "wav", "voice": "Chloe", "speed": 1.1},
                "stream": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"audio": {"data": "UklGRmF1ZGlv"}}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let mut call = mimo_tts_call(&base, "mimo-v2.5-tts", tts("hello", Some("Chloe"), Some("wav")));
        call.model_params = json!({"endpoint": "/chat/completions", "audio": {"speed": 1.1}});
        let out = MiMoChatTtsAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"RIFFaudio"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn tts_flat_params_can_supply_voice_design_instruction() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({
                "messages": [
                    {"role": "user", "content": "Give me a young male tone."},
                    {"role": "assistant", "content": "Read this."}
                ],
                "audio": {"format": "wav", "optimize_text_preview": true}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"audio": {"data": "aGk="}}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let mut call = mimo_tts_call(&base, "mimo-v2.5-tts-voicedesign", tts("Read this.", None, None));
        call.model_params = json!({
            "endpoint": "/chat/completions",
            "messages": [{"role": "user", "content": "Give me a young male tone."}],
            "audio": {"optimize_text_preview": true}
        });
        MiMoChatTtsAdapter.submit(&test_http(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn upstream_auth_and_in_body_errors_are_classified() {
        let auth_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&auth_server)
            .await;
        let auth_base = format!("{}/v1", auth_server.uri());
        let auth_call = mimo_asr_call(&auth_base, "mimo-v2.5-asr", asr("audio/wav", None));
        let err = MiMoChatAsrAdapter
            .submit(&test_http(), &auth_call)
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));

        let body_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "error": {"code": "model_not_found", "message": "retired"}
            })))
            .mount(&body_server)
            .await;
        let body_base = format!("{}/v1", body_server.uri());
        let body_call = mimo_tts_call(&body_base, "mimo-v2.5-tts", tts("hi", None, None));
        let err = MiMoChatTtsAdapter
            .submit(&test_http(), &body_call)
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("model_not_found"));
    }

    #[tokio::test]
    async fn local_validation_and_parse_failures_are_classified() {
        let invalid_call = mimo_asr_call(
            "http://127.0.0.1:9/v1",
            "mimo-v2.5-asr",
            asr("audio/ogg", None),
        );
        let err = MiMoChatAsrAdapter
            .submit(&test_http(), &invalid_call)
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
        assert_eq!(err.http_status, None);

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"audio": {"data": "not base64!"}}}]
            })))
            .mount(&server)
            .await;
        let base = format!("{}/v1", server.uri());
        let parse_call = mimo_tts_call(&base, "mimo-v2.5-tts", tts("hi", None, None));
        let err = MiMoChatTtsAdapter
            .submit(&test_http(), &parse_call)
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn adapters_reject_the_other_audio_task_locally() {
        let asr_call = mimo_asr_call("http://127.0.0.1:9", "mimo-v2.5-asr", asr("audio/wav", None));
        let err = MiMoChatTtsAdapter.submit(&test_http(), &asr_call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);

        let tts_call = mimo_tts_call("http://127.0.0.1:9", "mimo-v2.5-tts", tts("hi", None, None));
        let err = MiMoChatAsrAdapter.submit(&test_http(), &tts_call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }
}
