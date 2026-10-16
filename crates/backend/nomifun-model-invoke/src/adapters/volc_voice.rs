//! `volc.asr_file` / `volc.tts_v3` — Volcengine speech domain (openspeech)
//! file ASR and v3 大模型 TTS (routing contract:
//! `docs/architecture/model-providers.zh.md`,
//! voice domain: `openspeech.bytedance.com`, credentials fully independent of
//! the Ark API key — these protocols always ride the `"voice"` connection
//! profile with the `volc_voice` multi-header auth scheme).
//!
//! Volcano voice v3 signature quirks, all honored here:
//! - the job id (`X-Api-Request-Id`) is CLIENT-generated (UUIDv7 via
//!   [`nomifun_common::generate_id`]); for ASR it is reused verbatim between
//!   submit and query and is persisted once as [`JobHandle::remote_id`];
//! - the job status lives in the RESPONSE HEADER `X-Api-Status-Code`
//!   (`20000000` ok / `20000001`|`20000002` processing), not the body;
//! - failure detail rides the `X-Api-Message` header.
//!
//! ASR submit: `POST {base}/api/v3/auc/bigmodel/submit` with the audio inline
//! as base64; query: `POST {base}/api/v3/auc/bigmodel/query` with an empty
//! JSON body + the same request id. A finished query's body carries
//! `result.text` → [`TaskResult::Transcript`].
//!
//! TTS ([`VolcTtsV3Adapter`]): synchronous
//! `POST {base}/api/v3/tts/unidirectional` (HTTP single-direction stream);
//! the response body is JSON-LINES — one `{code, data: <base64>}` object per
//! line, audio chunks aggregated in order, terminated by a sentinel line
//! (code `20000000`, no data) — with the `X-Api-Status-Code` header carrying
//! the overall verdict.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::json;

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, resolve_endpoint};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    MAX_ARTIFACT_BYTES, decode_b64, encode_b64, error_from_response, read_body_capped, send_with_rotation,
    response_secret_redactor,
};
use crate::types::{JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult};

use super::json_request_body;

const ADAPTER_ID: &str = "volc.asr_file";
/// Submit ships inline base64 audio; query is a cheap status read — both are
/// plain request/response round-trips capped identically.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

const STATUS_HEADER: &str = "X-Api-Status-Code";
const MESSAGE_HEADER: &str = "X-Api-Message";
const REQUEST_ID_HEADER: &str = "X-Api-Request-Id";

/// Map the request audio MIME onto the `audio.format` vocabulary: the known
/// containers map directly, anything else falls back to the subtype (the part
/// after `/`, parameters stripped). Pure — unit tested.
fn audio_format_from_mime(mime: &str) -> String {
    let mime = mime.split(';').next().unwrap_or(mime).trim();
    match mime {
        "audio/wav" => "wav".to_string(),
        "audio/mpeg" => "mp3".to_string(),
        "audio/mp4" => "mp4".to_string(),
        "audio/ogg" => "ogg".to_string(),
        other => other.rsplit('/').next().unwrap_or(other).to_string(),
    }
}

/// A trimmed, non-empty response header value.
fn header_str(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Whether an `X-Api-Status-Code` means "accepted / still processing":
/// `20000000` ok, `20000001`/`20000002` processing.
fn is_accepted(code: &str) -> bool {
    matches!(code, "20000000" | "20000001" | "20000002")
}

/// Volcengine voice-domain file ASR: submit → query, status in headers.
pub struct VolcAsrFileAdapter;

#[async_trait]
impl ProtocolAdapter for VolcAsrFileAdapter {
    fn id(&self) -> &'static str {
        ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("volc.asr_file cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.endpoint_url()?;
        // Client-generated job id, reused verbatim by every later query.
        let request_id = nomifun_common::generate_id();
        let body = json_request_body(&call.model_params, &req.extra, json!({
            "user": { "uid": "nomifun" },
            "audio": {
                "format": audio_format_from_mime(&req.audio.mime),
                "data": encode_b64(&req.audio.bytes),
            },
            "request": { "model_name": call.model },
        }))?;

        let rb_build = || {
            Ok(http.post(&url).timeout(REQUEST_TIMEOUT).header(REQUEST_ID_HEADER, &request_id).json(&body))
        };
        let resp = send_with_rotation(&call.connection.auth, rb_build).await?;
        let response_redactor = response_secret_redactor(&resp);

        match header_str(&resp, STATUS_HEADER) {
            // Accepted (or already processing) → the request id IS the job.
            Some(code) if is_accepted(&code) => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: request_id,
                poll_state: json!({}),
            })),
            Some(code) => {
                let detail = match header_str(&resp, MESSAGE_HEADER) {
                    Some(msg) => response_redactor.redact(&msg),
                    None => response_redactor
                        .redact(&resp.text().await.unwrap_or_default())
                        .chars()
                        .take(500)
                        .collect(),
                };
                Err(InvokeError::new(
                    InvokeErrorKind::ProviderError,
                    format!("volc asr submit rejected (X-Api-Status-Code {code}): {detail}"),
                ))
            }
            // No protocol status at all: classify a plain HTTP failure
            // normally; a 2xx without the header is an unintelligible reply.
            None if !resp.status().is_success() => Err(error_from_response(resp).await),
            None => Err(InvokeError::parse("volc asr submit response missing X-Api-Status-Code header")),
        }
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let poll_endpoint = call
            .model_params
            .get("poll_endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| InvokeError::config("volc.asr_file requires an injected poll endpoint"))?;
        let url = call.credentialed_http_url(
            &resolve_endpoint(&call.connection.base_url, poll_endpoint),
            "poll_endpoint",
        )?;
        let request_id = job.remote_id.trim();
        if request_id.is_empty() {
            return Err(InvokeError::config("volc.asr_file job remote_id must not be empty"));
        }

        let rb_build = || {
            Ok(http
                .post(&url)
                .timeout(REQUEST_TIMEOUT)
                .header(REQUEST_ID_HEADER, request_id)
                .json(&json!({})))
        };
        let resp = send_with_rotation(&call.connection.auth, rb_build).await?;
        let response_redactor = response_secret_redactor(&resp);

        match header_str(&resp, STATUS_HEADER).as_deref() {
            Some("20000000") => {
                let value: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| InvokeError::response_json("invalid volc asr query JSON", &e))?;
                let text = value
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| InvokeError::parse("volc asr query succeeded but missing result.text"))?;
                Ok(TaskOutcome::Done(TaskResult::Transcript {
                    text: text.to_string(),
                    language: None,
                    model: Some(call.model.clone()),
                }))
            }
            Some("20000001") | Some("20000002") => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            // Terminal remote failure (45xxxxxx …): message header, else the code.
            Some(code) => {
                let msg = response_redactor.redact(
                    &header_str(&resp, MESSAGE_HEADER).unwrap_or_else(|| code.to_string()),
                );
                Err(InvokeError::new(InvokeErrorKind::JobFailed, msg))
            }
            None if !resp.status().is_success() => Err(error_from_response(resp).await),
            None => Err(InvokeError::parse("volc asr query response missing X-Api-Status-Code header")),
        }
    }
}

// ---------------------------------------------------------------------------
// volc.tts_v3
// ---------------------------------------------------------------------------

const TTS_ADAPTER_ID: &str = "volc.tts_v3";
/// One synchronous streamed round-trip; long text takes a while to synthesize.
const TTS_TIMEOUT: Duration = Duration::from_secs(120);

/// Volcengine voice-domain v3 大模型 TTS: single-direction HTTP stream whose
/// body is JSON-lines of base64 audio chunks.
pub struct VolcTtsV3Adapter;

#[async_trait]
impl ProtocolAdapter for VolcTtsV3Adapter {
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
                format!("volc.tts_v3 cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.endpoint_url()?;
        // v3 语音要求客户端发号：X-Api-Request-Id 由我们生成（同 ASR）。
        let request_id = nomifun_common::generate_id();

        // The capability model identifies a resource/profile, not the
        // optional native voice-cloning engine. Only explicit native config
        // may supply req_params.model.
        let mut req_params = json!({
            "text": req.text,
        });
        if let Some(voice) = req.voice.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            req_params["speaker"] = serde_json::Value::String(voice.to_string());
        }
        if let Some(format) = req.format.as_deref() {
            let format = format.trim();
            if format.is_empty() {return Err(InvokeError::new(InvokeErrorKind::InvalidParams,"volc.tts_v3 format must not be empty"));}
            req_params["audio_params"] = json!({"format":format});
        }
        let mut body = json_request_body(
            &call.model_params,
            &req.extra,
            json!({ "req_params": req_params }),
        )?;
        let params = body["req_params"].as_object_mut().expect("typed req_params object");
        if !params.get("speaker").and_then(serde_json::Value::as_str).is_some_and(|speaker| !speaker.trim().is_empty()) {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams,
                "volc.tts_v3 requires an explicit voice or configured req_params.speaker"));
        }
        if !params.entry("audio_params").or_insert_with(|| json!({})).is_object() {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams,"volc.tts_v3 req_params.audio_params must be an object"));
        }

        let resp = send_with_rotation(&call.connection.auth, || {
            Ok(http.post(&url).timeout(TTS_TIMEOUT).header(REQUEST_ID_HEADER, &request_id).json(&body))
        })
        .await?;
        let response_redactor = response_secret_redactor(&resp);

        // Header verdict first (the voice-domain house rule), then the body.
        match header_str(&resp, STATUS_HEADER) {
            Some(code) if !is_accepted(&code) => {
                let detail = match header_str(&resp, MESSAGE_HEADER) {
                    Some(msg) => response_redactor.redact(&msg),
                    None => response_redactor
                        .redact(&resp.text().await.unwrap_or_default())
                        .chars()
                        .take(500)
                        .collect(),
                };
                return Err(InvokeError::new(
                    InvokeErrorKind::ProviderError,
                    format!("volc tts rejected (X-Api-Status-Code {code}): {detail}"),
                ));
            }
            None if !resp.status().is_success() => return Err(error_from_response(resp).await),
            // Accepted code, or a 2xx without the header (tolerated: the
            // JSON-lines body itself carries per-line codes).
            _ => {}
        }

        let raw = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;
        let text = String::from_utf8_lossy(&raw);
        let bytes = aggregate_tts_json_lines(&text).map_err(|error| error.redacted(&response_redactor))?;

        // Match the actual requested native encoding, whose default is mp3.
        let mime = match body["req_params"]["audio_params"]["format"].as_str() {
            Some("wav") => "audio/wav",
            Some("pcm") => "audio/pcm",
            Some("ogg" | "ogg_opus" | "opus") => "audio/ogg",
            _ => "audio/mpeg",
        };
        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(mime.to_owned()),
        }])))
    }
}

/// Aggregate a v3 TTS JSON-lines body: each non-blank line is one JSON object;
/// code 0 permits audio/subtitle frames; code 20000000 is the required terminal
/// success. Error codes take priority over audio data. A missing terminal,
/// trailing frames, or an aggregate without audio fails closed.
pub(crate) fn aggregate_tts_json_lines(body: &str) -> Result<Vec<u8>, InvokeError> {
    let mut out: Vec<u8> = Vec::new();
    let mut terminal = false;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if terminal {return Err(InvokeError::parse("volc tts stream has data after its terminal frame"));}
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| {
            let snippet: String = line.chars().take(200).collect();
            InvokeError::parse(format!("volc tts stream line is not JSON ({e}): {snippet}"))
        })?;
        let code = value.get("code").and_then(|code| code.as_i64())
            .ok_or_else(|| InvokeError::parse("volc tts stream frame requires an integer code"))?;
        if code == 20_000_000 {
            if value.get("data").is_some_and(|data| !data.is_null() && data.as_str() != Some("")) {
                return Err(InvokeError::parse("volc tts terminal frame unexpectedly contains audio data"));
            }
            terminal = true;
            continue;
        }
        // A failed frame may still contain data; never accept that audio.
        if code != 0 {
            let msg = value
                .get("message")
                .and_then(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| code.to_string());
            return Err(InvokeError::new(
                InvokeErrorKind::ProviderError,
                format!("volc tts failed (code {code}): {msg}"),
            ));
        }
        match value.get("data") {
            None | Some(serde_json::Value::Null) => {},
            Some(serde_json::Value::String(data)) if data.is_empty() => {},
            Some(serde_json::Value::String(data)) => {
                let chunk=decode_b64(data).ok_or_else(|| InvokeError::parse("volc tts chunk data is not valid base64"))?;
                out.extend_from_slice(&chunk);
            }
            Some(_) => return Err(InvokeError::parse("volc tts chunk data must be base64 text or null")),
        }
    }
    if !terminal {return Err(InvokeError::parse("volc tts stream closed without its terminal success frame"));}
    if out.is_empty() {
        return Err(InvokeError::parse("volc tts stream produced no audio chunks"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::ResolvedConnection;
    use crate::types::{AsrRequest, InputAsset};

    /// A voice-role [`ResolvedCall`] as the resolver produces it for the
    /// volcano voice route: `volc_voice` multi-header scheme fed from the
    /// connection profile's decrypted credentials.
    fn voice_call_with_endpoint(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        poll_endpoint: Option<&str>,
        request: TaskRequest,
    ) -> ResolvedCall {
        let task = request.task();
        let mut model_params = json!({"endpoint": endpoint});
        if let Some(poll_endpoint) = poll_endpoint {
            model_params["poll_endpoint"] = json!(poll_endpoint);
        }
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-0000000000cc".into(),
            config_revision: 1,
            platform: "ark".into(),
            model: model.into(),
            task,
            protocol: protocol.into(),
            connection: ResolvedConnection {
                role: "voice".into(),
                base_url: base_url.into(),
                auth: AuthMaterial {
                    scheme: AuthScheme::parse("volc_voice").unwrap(),
                    credentials: json!({
                        "app_key": "app-1",
                        "access_key": "ak-1",
                        "resource_id": "volc.bigasr.auc",
                    }),
                },
                extra: json!({}),
            },
            model_params,
            request,
        }
    }

    fn asr_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        voice_call_with_endpoint(
            base_url,
            model,
            "volc.asr_file",
            "/api/v3/auc/bigmodel/submit",
            Some("/api/v3/auc/bigmodel/query"),
            request,
        )
    }

    fn tts_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        voice_call_with_endpoint(
            base_url,
            model,
            "volc.tts_v3",
            "/api/v3/tts/unidirectional",
            None,
            request,
        )
    }

    fn asr(mime: &str) -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: b"RIFFdata".to_vec(), mime: mime.into() },
            language: None,
            prompt: None,
            extra: json!({}),
        })
    }

    fn job(remote_id: &str, poll_state: Value) -> JobHandle {
        JobHandle { adapter_id: ADAPTER_ID.into(), config_revision: 1, remote_id: remote_id.into(), poll_state }
    }

    // -- pure helpers ----------------------------------------------------------

    #[test]
    fn audio_format_maps_known_mimes_and_strips_prefix() {
        assert_eq!(audio_format_from_mime("audio/wav"), "wav");
        assert_eq!(audio_format_from_mime("audio/mpeg"), "mp3");
        assert_eq!(audio_format_from_mime("audio/mp4"), "mp4");
        assert_eq!(audio_format_from_mime("audio/ogg"), "ogg");
        // Unknown subtype: strip the type prefix (and any parameters).
        assert_eq!(audio_format_from_mime("audio/flac"), "flac");
        assert_eq!(audio_format_from_mime("audio/wav; rate=16000"), "wav");
        assert_eq!(audio_format_from_mime("audio/amr; x=y"), "amr");
        // No slash at all → used as-is.
        assert_eq!(audio_format_from_mime("pcm"), "pcm");
    }

    // -- submit ---------------------------------------------------------------

    #[tokio::test]
    async fn submit_sends_four_headers_inline_audio_and_returns_pending_handle() {
        let server = MockServer::start().await;
        // "UklGRmRhdGE=" is base64("RIFFdata").
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .and(header("X-Api-App-Key", "app-1"))
            .and(header("X-Api-Access-Key", "ak-1"))
            .and(header("X-Api-Resource-Id", "volc.bigasr.auc"))
            .and(body_partial_json(json!({
                "user": {"uid": "nomifun"},
                "audio": {"format": "wav", "data": "UklGRmRhdGE="},
                "request": {"model_name": "bigmodel-asr"},
            })))
            .respond_with(ResponseTemplate::new(200).insert_header(STATUS_HEADER, "20000000"))
            .expect(1)
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let out = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.adapter_id, "volc.asr_file");
        // The job id is the client-generated UUIDv7 request id and is sent as
        // the X-Api-Request-Id header on every operation.
        assert!(!handle.remote_id.is_empty());
        assert_eq!(handle.poll_state, json!({}));
        let requests = server.received_requests().await.unwrap();
        let sent_id = requests[0].headers.get(REQUEST_ID_HEADER).unwrap().to_str().unwrap();
        assert_eq!(sent_id, handle.remote_id);
    }

    #[tokio::test]
    async fn submit_processing_codes_are_also_pending() {
        for code in ["20000001", "20000002"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v3/auc/bigmodel/submit"))
                .respond_with(ResponseTemplate::new(200).insert_header(STATUS_HEADER, code))
                .mount(&server)
                .await;
            let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
            let out = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
            assert!(matches!(out, TaskOutcome::Pending(_)), "code {code}");
        }
    }

    #[tokio::test]
    async fn submit_error_code_is_provider_error_with_message_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "45000001")
                    .insert_header(MESSAGE_HEADER, "invalid audio format"),
            )
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("45000001"), "message: {}", err.message);
        assert!(err.message.contains("invalid audio format"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn submit_error_code_without_message_header_falls_back_to_body_snippet() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "55000000")
                    .set_body_string("internal voice error"),
            )
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("internal voice error"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn submit_missing_status_header_on_2xx_is_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
        assert!(err.message.contains("X-Api-Status-Code"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn submit_plain_http_401_without_status_header_maps_to_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad voice creds"))
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }

    // -- poll -------------------------------------------------------------------

    #[tokio::test]
    async fn poll_reuses_submit_request_id_and_parses_transcript() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .and(header(REQUEST_ID_HEADER, "req-123"))
            .and(header("X-Api-App-Key", "app-1"))
            .and(header("X-Api-Access-Key", "ak-1"))
            .and(header("X-Api-Resource-Id", "volc.bigasr.auc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "20000000")
                    .set_body_json(json!({"result": {"text": "hello volc"}})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let handle = job("req-123", json!({}));
        let out = VolcAsrFileAdapter.poll(&reqwest::Client::new(), &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello volc");
        assert_eq!(language, None);
        assert_eq!(model.as_deref(), Some("bigmodel-asr"));
        // The query body is an empty JSON object.
        let requests = server.received_requests().await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&requests[0].body).unwrap(), json!({}));
    }

    #[tokio::test]
    async fn poll_uses_the_single_remote_id_source() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .and(header(REQUEST_ID_HEADER, "req-fallback"))
            .respond_with(ResponseTemplate::new(200).insert_header(STATUS_HEADER, "20000001"))
            .expect(1)
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let handle = job("req-fallback", Value::Null);
        let out = VolcAsrFileAdapter.poll(&reqwest::Client::new(), &call, &handle).await.unwrap();
        let TaskOutcome::Pending(next) = out else { panic!("expected Pending on processing code") };
        assert_eq!(next.remote_id, "req-fallback");
        assert_eq!(next.poll_state, json!({}));
    }

    #[tokio::test]
    async fn poll_failure_code_is_job_failed_with_message_falling_back_to_code() {
        // With an X-Api-Message header → that message.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "45000151")
                    .insert_header(MESSAGE_HEADER, "audio too long"),
            )
            .mount(&server)
            .await;
        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter
            .poll(&reqwest::Client::new(), &call, &job("r1", json!({})))
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "audio too long");

        // Without one → the status code itself.
        let bare = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .respond_with(ResponseTemplate::new(200).insert_header(STATUS_HEADER, "45000000"))
            .mount(&bare)
            .await;
        let call = asr_call(&bare.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter
            .poll(&reqwest::Client::new(), &call, &job("r1", json!({})))
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "45000000");
    }

    #[tokio::test]
    async fn poll_success_missing_result_text_is_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "20000000")
                    .set_body_json(json!({"result": {}})),
            )
            .mount(&server)
            .await;

        let call = asr_call(&server.uri(), "bigmodel-asr", asr("audio/wav"));
        let err = VolcAsrFileAdapter
            .poll(&reqwest::Client::new(), &call, &job("r1", json!({})))
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
        assert!(err.message.contains("result.text"), "message: {}", err.message);
    }

    // -- volc.tts_v3 --------------------------------------------------------------

    use crate::types::TtsRequest;

    fn tts(text: &str, voice: Option<&str>, format: Option<&str>) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: text.into(),
            voice: voice.map(str::to_string),
            format: format.map(str::to_string),
            extra: json!({}),
        })
    }

    #[test]
    fn tts_json_lines_aggregate_in_order_and_ignore_sentinel() {
        // "aGVs" = b64("hel"), "bG8=" = b64("lo"); sentinel line has no data.
        let body = "\
{\"code\":0,\"data\":\"aGVs\"}\n\
{\"code\":0,\"data\":\"bG8=\"}\n\
\n\
{\"code\":20000000,\"message\":\"OK\"}\n";
        assert_eq!(aggregate_tts_json_lines(body).unwrap(), b"hello");
    }

    #[test]
    fn tts_json_lines_failure_line_is_provider_error_with_message() {
        let body = "{\"code\":0,\"data\":\"aGVs\"}\n{\"code\":45000001,\"message\":\"invalid speaker\"}\n";
        let err = aggregate_tts_json_lines(body).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("45000001"), "message: {}", err.message);
        assert!(err.message.contains("invalid speaker"), "message: {}", err.message);

        // Without a message field → the code itself.
        let bare = aggregate_tts_json_lines("{\"code\":55000000}\n").unwrap_err();
        assert!(bare.message.contains("55000000"), "message: {}", bare.message);
    }

    #[test]
    fn tts_json_lines_malformed_or_empty_are_parse_errors() {
        for bad in ["not json at all\n", "{\"code\":0,\"data\":\"!!!bad-b64!!!\"}\n"] {
            let err = aggregate_tts_json_lines(bad).unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::ParseError, "input {bad:?}");
        }
        // Only sentinels, no audio at all.
        let err = aggregate_tts_json_lines("{\"code\":20000000}\n").unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
        assert!(err.message.contains("no audio"), "message: {}", err.message);
    }

    #[test]
    fn tts_json_lines_require_a_terminal_and_never_accept_audio_from_failed_frames() {
        for invalid in [
            "{\"code\":0,\"data\":\"aGk=\"}\n",
            "{\"data\":\"aGk=\"}\n{\"code\":20000000}\n",
            "{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":20000000,\"data\":\"aGk=\"}\n",
            "{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":20000000}\n{\"code\":0,\"data\":\"aGk=\"}\n",
        ] {assert_eq!(aggregate_tts_json_lines(invalid).unwrap_err().kind,InvokeErrorKind::ParseError);}
        let failed=aggregate_tts_json_lines("{\"code\":45000000,\"data\":\"aGk=\",\"message\":\"speaker denied\"}\n").unwrap_err();
        assert_eq!(failed.kind,InvokeErrorKind::ProviderError);
        assert!(failed.message.contains("speaker denied"));
        assert_eq!(aggregate_tts_json_lines("{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":0,\"data\":null,\"sentence\":{}}\n{\"code\":20000000,\"data\":null}\n").unwrap(),b"hi");
    }

    #[tokio::test]
    async fn tts_stream_failures_redact_credentials_and_eof_never_returns_partial_audio() {
        for (reply,kind) in [
            ("{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":45000000,\"data\":\"aGk=\",\"message\":\"rejected ak-1\"}\n",InvokeErrorKind::ProviderError),
            ("{\"code\":0,\"data\":\"aGk=\"}\n",InvokeErrorKind::ParseError),
        ] {
            let server=MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(reply)).expect(1).mount(&server).await;
            let call=tts_call(&server.uri(),"resource-profile",tts("hi",Some("voice-fixture"),Some("mp3")));
            let error=VolcTtsV3Adapter.submit(&reqwest::Client::new(),&call).await.unwrap_err();
            assert_eq!(error.kind,kind);assert!(!error.message.contains("ak-1"));
        }
    }

    #[tokio::test]
    async fn tts_sends_four_headers_req_params_body_and_aggregates_stream() {
        let server = MockServer::start().await;
        let stream_body = "{\"code\":0,\"data\":\"aGVs\"}\n{\"code\":0,\"data\":\"bG8=\"}\n{\"code\":20000000}\n";
        Mock::given(method("POST"))
            .and(path("/api/v3/tts/unidirectional"))
            .and(header("X-Api-App-Key", "app-1"))
            .and(header("X-Api-Access-Key", "ak-1"))
            .and(header("X-Api-Resource-Id", "volc.bigasr.auc"))
            .and(body_partial_json(json!({
                "req_params": {"text": "你好", "speaker": "zh_female_1", "audio_params": {}},
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "20000000")
                    .set_body_string(stream_body),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = tts_call(&server.uri(), "seed-tts", tts("你好", Some("zh_female_1"), None));
        let out = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hello"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));

        // The request id is CLIENT-generated and rides the fourth header.
        let requests = server.received_requests().await.unwrap();
        let sent_id = requests[0].headers.get(REQUEST_ID_HEADER).unwrap().to_str().unwrap();
        assert!(!sent_id.is_empty(), "X-Api-Request-Id must be client-generated");
        let body:Value=serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body["req_params"].get("model").is_none(),"a directory/resource model must not be injected as an engine ID");
    }

    #[tokio::test]
    async fn tts_requires_a_speaker_before_http_and_maps_format_to_wire_and_mime() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/tts/unidirectional"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "20000000")
                    .set_body_string("{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":20000000,\"data\":null}\n"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = tts_call(&server.uri(), "seed-tts", tts("hi", None, Some("wav")));
        let error=VolcTtsV3Adapter.submit(&reqwest::Client::new(),&call).await.unwrap_err();
        assert_eq!(error.kind,InvokeErrorKind::InvalidParams);
        assert!(server.received_requests().await.unwrap().is_empty());
        let configured=tts_call(&server.uri(),"seed-tts",tts("hi",Some("voice-fixture"),Some("wav")));
        let out = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &configured).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["req_params"]["speaker"],"voice-fixture");
        assert_eq!(body["req_params"]["audio_params"]["format"],"wav");
    }

    #[tokio::test]
    async fn tts_typed_format_overrides_configured_encoding_without_losing_audio_options() {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/v3/tts/unidirectional"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"code\":0,\"data\":\"aGk=\"}\n{\"code\":20000000,\"data\":null}\n")).expect(2).mount(&server).await;
        for (typed,expected,mime) in [(Some("wav"),"wav","audio/wav"),(None,"pcm","audio/pcm")] {
            let mut call=tts_call(&server.uri(),"seed-tts",tts("hi",None,typed));
            call.model_params["req_params"]=json!({"speaker":"voice-fixture","model":"seed-tts-2.0-expressive","audio_params":{"format":"pcm","sample_rate":24000}});
            let output=VolcTtsV3Adapter.submit(&reqwest::Client::new(),&call).await.unwrap();
            let TaskOutcome::Done(TaskResult::Assets(assets))=output else {panic!("expected audio asset");};
            assert_eq!(assets[0].mime.as_deref(),Some(mime));
            let requests=server.received_requests().await.unwrap();let body:Value=serde_json::from_slice(&requests.last().unwrap().body).unwrap();
            assert_eq!(body["req_params"]["audio_params"],json!({"format":expected,"sample_rate":24000}));
            assert_eq!(body["req_params"]["model"],"seed-tts-2.0-expressive");
        }
    }

    #[tokio::test]
    async fn tts_error_status_header_is_provider_error_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/tts/unidirectional"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "45000001")
                    .insert_header(MESSAGE_HEADER, "invalid resource id"),
            )
            .mount(&server)
            .await;

        let call = tts_call(&server.uri(), "seed-tts", tts("hi", Some("voice-fixture"), None));
        let err = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("45000001"), "message: {}", err.message);
        assert!(err.message.contains("invalid resource id"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn tts_in_stream_failure_line_maps_to_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/tts/unidirectional"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(STATUS_HEADER, "20000000")
                    .set_body_string("{\"code\":45000151,\"message\":\"text too long\"}\n"),
            )
            .mount(&server)
            .await;

        let call = tts_call(&server.uri(), "seed-tts", tts("hi", Some("voice-fixture"), None));
        let err = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ProviderError);
        assert!(err.message.contains("text too long"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn tts_plain_http_401_without_status_header_maps_to_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/tts/unidirectional"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad voice creds"))
            .mount(&server)
            .await;

        let call = tts_call(&server.uri(), "seed-tts", tts("hi", Some("voice-fixture"), None));
        let err = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }

    #[tokio::test]
    async fn tts_rejects_non_tts_request_locally() {
        let call = asr_call("http://127.0.0.1:9", "seed-tts", asr("audio/wav"));
        let err = VolcTtsV3Adapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
    }
}
