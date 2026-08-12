//! `minimax.t2a` — MiniMax synchronous text-to-speech (protocol per
//! `docs/specs/2026-07-28-provider-protocol-variance.zh.md` §5: domain
//! `api.minimaxi.com` (国内) / `api.minimax.io` (国际) — the two platforms'
//! keys are NOT interchangeable; the connection's base_url picks the platform).
//!
//! MiniMax signature quirks, all honored here:
//! - current MiniMax t2a requests carry no transport query parameters;
//! - the returned audio (`data.audio`) is a **hex** string — NOT base64 —
//!   decoded via [`crate::transport::decode_hex`];
//! - failures often ride an HTTP 200 with a non-zero `base_resp.status_code`
//!   (mapped to [`InvokeErrorKind::ProviderError`] with the status message);
//! - the trailing `extra_info` block (durations/sizes) is ignored.
//!
//! `POST {base}/v1/t2a_v2` with `{model, text,
//! voice_setting: {voice_id}?, audio_setting: {format}?}` → a single
//! [`TaskResult::Assets`] audio artifact.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_hex, error_from_response, send_with_rotation};
use crate::types::{ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult};

use super::json_request_body;

const ADAPTER_ID: &str = "minimax.t2a";
/// Synthesis of long text can take a while; one synchronous round-trip.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// MIME for the requested audio format (MiniMax vocabulary: mp3/pcm/flac/wav —
/// default mp3). Unknown formats fall back to `audio/mpeg`.
fn mime_for_format(format: Option<&str>) -> &'static str {
    match format {
        Some("wav") => "audio/wav",
        Some("pcm") => "audio/pcm",
        Some("flac") => "audio/flac",
        _ => "audio/mpeg", // "mp3" / absent / unrecognized
    }
}

/// MiniMax synchronous t2a_v2 speech synthesis.
pub struct MiniMaxT2aAdapter;

#[async_trait]
impl ProtocolAdapter for MiniMaxT2aAdapter {
    fn id(&self) -> &'static str {
        ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("minimax.t2a cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.endpoint_url()?;

        let mut body = json!({
            "model": call.model,
            "text": req.text,
        });
        if let Some(voice) = req.voice.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            // 未提供 voice 时省略 voice_setting，交由 MiniMax 服务端默认音色
            // —— 接入时需真实调用校准（默认音色行为为二手资料）。
            body["voice_setting"] = json!({ "voice_id": voice });
        }
        if let Some(format) = req.format.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            // audio_setting.format 词表 mp3/pcm/flac/wav —— 接入时需真实调用校准。
            body["audio_setting"] = json!({ "format": format });
        }
        let body = json_request_body(&call.model_params, &req.extra, body)?;

        let resp = send_with_rotation(&call.connection.auth, || {
            Ok(http.post(&url).timeout(REQUEST_TIMEOUT).json(&body))
        })
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid minimax t2a JSON", &e))?;

        let bytes = parse_t2a_audio(&value)?;
        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(mime_for_format(req.format.as_deref()).to_owned()),
        }])))
    }
}

/// Extract and hex-decode `data.audio`. A missing audio field surfaces the
/// `base_resp.status_code`/`status_msg` failure vocabulary as
/// [`InvokeErrorKind::ProviderError`] (MiniMax reports most failures on an
/// HTTP 200); invalid hex is a parse error; `extra_info` is ignored. Pure —
/// unit tested.
pub(crate) fn parse_t2a_audio(value: &Value) -> Result<Vec<u8>, InvokeError> {
    let audio = value.get("data").and_then(|d| d.get("audio")).and_then(|a| a.as_str());
    if let Some(hex) = audio.filter(|s| !s.trim().is_empty()) {
        return decode_hex(hex)
            .filter(|b| !b.is_empty())
            .ok_or_else(|| InvokeError::parse("minimax t2a data.audio is not valid hex"));
    }
    // No audio: fold the in-body failure vocabulary into a provider error.
    let code = value.get("base_resp").and_then(|b| b.get("status_code")).and_then(|c| c.as_i64());
    let msg = value
        .get("base_resp")
        .and_then(|b| b.get("status_msg"))
        .and_then(|m| m.as_str())
        .unwrap_or("response carries no data.audio");
    match code {
        Some(code) if code != 0 => Err(InvokeError::new(
            InvokeErrorKind::ProviderError,
            format!("minimax t2a failed (base_resp.status_code {code}): {msg}"),
        )),
        _ => Err(InvokeError::parse(format!("minimax t2a produced no audio: {msg}"))),
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::TtsRequest;

    fn minimax_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let base_url = base_url.trim_end_matches('/');
        let base_url = if base_url.ends_with("/v1") {
            base_url.to_owned()
        } else {
            format!("{base_url}/v1")
        };
        let mut call = call_with_endpoint(&base_url, model, "minimax.t2a", "/t2a_v2", request);
        call.platform = "minimax".into();
        call
    }

    fn tts(text: &str, voice: Option<&str>, format: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.into(),
            voice: voice.map(str::to_string),
            format: format.map(str::to_string),
            extra: json!({}),
        })
    }

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    // -- pure helpers ------------------------------------------------------------

    #[test]
    fn t2a_audio_parses_hex_and_maps_failure_vocabulary() {
        // "68656c6c6f" is hex("hello").
        let ok = json!({"data": {"audio": "68656c6c6f", "status": 2}, "extra_info": {"audio_length": 1}});
        assert_eq!(parse_t2a_audio(&ok).unwrap(), b"hello");

        // Bad hex → parse error.
        let bad_hex = json!({"data": {"audio": "zz-not-hex"}});
        assert_eq!(parse_t2a_audio(&bad_hex).unwrap_err().kind, InvokeErrorKind::ParseError);

        // In-body failure vocabulary (HTTP 200) → ProviderError with code+msg.
        let refused = json!({"base_resp": {"status_code": 1004, "status_msg": "invalid api key"}});
        let err = parse_t2a_audio(&refused).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("1004"), "message: {}", err.message);
        assert!(err.message.contains("invalid api key"), "message: {}", err.message);

        // No audio and no failure code → parse error.
        assert_eq!(parse_t2a_audio(&json!({})).unwrap_err().kind, InvokeErrorKind::ParseError);
        let ok_code = json!({"base_resp": {"status_code": 0, "status_msg": "success"}});
        assert_eq!(parse_t2a_audio(&ok_code).unwrap_err().kind, InvokeErrorKind::ParseError);
    }

    // -- wiremock full chain -------------------------------------------------------

    #[tokio::test]
    async fn t2a_posts_current_body_without_query_and_decodes_hex_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "speech-01-turbo",
                "text": "你好世界",
                "voice_setting": {"voice_id": "female-shaonv"},
                "audio_setting": {"format": "wav"},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {"audio": "68656c6c6f", "status": 2},
                "extra_info": {"audio_length": 1160, "audio_size": 5},
                "base_resp": {"status_code": 0, "status_msg": "success"},
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = minimax_call(
            &server.uri(),
            "speech-01-turbo",
            tts("你好世界", Some("female-shaonv"), Some("wav")),
        );
        let out = MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hello"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn t2a_omits_voice_and_audio_settings_when_absent_and_defaults_mpeg() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {"audio": "6869"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = minimax_call(&server.uri(), "speech-01", tts("hi", None, None));
        let out = MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("voice_setting").is_none(), "voice_setting must be omitted");
        assert!(body.get("audio_setting").is_none(), "audio_setting must be omitted");
    }

    #[tokio::test]
    async fn t2a_base_with_trailing_v1_does_not_double() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"audio": "6869"}})))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let call = minimax_call(&base, "speech-01", tts("hi", None, None));
        MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap();
    }

    #[tokio::test]
    async fn t2a_sends_no_query_and_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"audio": "6869"}})))
            .expect(1)
            .mount(&server)
            .await;

        let call = minimax_call(&server.uri(), "speech-01", tts("hi", None, None));
        MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.query().is_none(), "unexpected query: {}", requests[0].url);
    }

    #[tokio::test]
    async fn t2a_http_200_with_failure_vocabulary_is_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 2013, "status_msg": "invalid params, voice not found"}
            })))
            .mount(&server)
            .await;

        let call = minimax_call(&server.uri(), "speech-01", tts("hi", None, None));
        let err = MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("2013"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn t2a_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad minimax key"))
            .mount(&server)
            .await;

        let call = minimax_call(&server.uri(), "speech-01", tts("hi", None, None));
        let err = MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }

    #[tokio::test]
    async fn t2a_rejects_non_tts_request_locally() {
        use crate::types::{AsrRequest, InputAsset};
        let request = TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: vec![1], mime: "audio/wav".into() },
            language: None,
            prompt: None,
            extra: json!({}),
        });
        let call = minimax_call("http://127.0.0.1:9", "speech-01", request);
        let err = MiniMaxT2aAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }
}
