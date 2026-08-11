//! Native xAI media protocols.
//!
//! xAI's text/chat APIs are OpenAI-compatible, but its media surface is not a
//! single OpenAI-shaped family:
//! - image edits are JSON (including data-URI inputs), not multipart;
//! - video generation submits to `/videos/generations` and polls
//!   `/videos/{request_id}`;
//! - batch TTS is `/tts` with `{text, voice_id, language}` and raw audio;
//! - batch STT is `/stt` multipart and does not take an OpenAI `model` field.
//!
//! The configured xAI base normally already ends in `/v1`.  The URL helper
//! tolerates an origin-only base as well, but never rewrites a versioned xAI
//! root through the generic OpenAI dispatch convention.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::{Map, Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    MAX_ARTIFACT_BYTES, decode_b64, encode_b64, error_from_response, get_request, post_json,
    post_multipart, read_body_capped,
};
use crate::types::{
    AsrRequest, ImageEditRequest, ImageGenRequest, JobHandle, ProducedAsset, ProducedData,
    TaskOutcome, TaskRequest, TaskResult, TtsRequest, VideoGenRequest,
};

const MEDIA_TIMEOUT: Duration = Duration::from_secs(180);
const TTS_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const STT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

const IMAGES_ADAPTER_ID: &str = "xai.images_json";
const VIDEO_ADAPTER_ID: &str = "xai.video_jobs";
const TTS_ADAPTER_ID: &str = "xai.tts";
const STT_ADAPTER_ID: &str = "xai.stt";

fn non_empty_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// Per-model request defaults may be stored directly in `params` or grouped
/// under `request_defaults`.  The grouped form keeps protocol wiring keys such
/// as `endpoint` separate from upstream request fields.
fn configured_value<'a>(params: &'a Value, key: &str) -> Option<&'a Value> {
    params.get("request_defaults").and_then(Value::as_object).and_then(|o| o.get(key)).or_else(|| params.get(key))
}

fn resolve_endpoint_override(base_url: &str, endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_owned()
    } else {
        let base = base_url.trim().trim_end_matches('/');
        if endpoint.starts_with('/') {
            format!("{base}{endpoint}")
        } else {
            format!("{base}/{endpoint}")
        }
    }
}

fn xai_default_url(connection: &ResolvedConnection, path: &str) -> String {
    let base = connection.base_url.trim().trim_end_matches('/');
    if connection.is_full_url {
        return base.to_owned();
    }
    if base.ends_with("/v1") {
        format!("{base}{path}")
    } else {
        format!("{base}/v1{path}")
    }
}

fn xai_submit_url(call: &ResolvedCall, path: &str) -> String {
    non_empty_str(&call.model_params, "endpoint")
        .map(|endpoint| resolve_endpoint_override(&call.connection.base_url, endpoint))
        .unwrap_or_else(|| xai_default_url(&call.connection, path))
}

fn xai_video_status_url(call: &ResolvedCall, request_id: &str) -> String {
    let override_endpoint = non_empty_str(&call.model_params, "poll_endpoint")
        .or_else(|| non_empty_str(&call.model_params, "status_endpoint"));
    if let Some(endpoint) = override_endpoint {
        let resolved = resolve_endpoint_override(&call.connection.base_url, endpoint);
        return if resolved.contains("{request_id}") {
            resolved.replace("{request_id}", request_id)
        } else {
            format!("{}/{request_id}", resolved.trim_end_matches('/'))
        };
    }

    // A full-url connection commonly points at the submit endpoint.  Polling
    // is its sibling `/videos/{id}`, not `/videos/generations/{id}`.
    if call.connection.is_full_url {
        let submit = call.connection.base_url.trim().trim_end_matches('/');
        if let Some(videos_base) = submit.strip_suffix("/generations") {
            return format!("{videos_base}/{request_id}");
        }
        return format!("{submit}/{request_id}");
    }
    xai_default_url(&call.connection, &format!("/videos/{request_id}"))
}

fn apply_allowed(
    body: &mut Map<String, Value>,
    configured: &Value,
    request_extra: &Value,
    keys: &[&str],
) {
    for key in keys {
        if let Some(value) = configured_value(configured, key) {
            body.insert((*key).to_owned(), value.clone());
        }
        // Per-invocation options override configured defaults.
        if let Some(value) = request_extra.get(key) {
            body.insert((*key).to_owned(), value.clone());
        }
    }
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode_b64(bytes))
}

fn reduced_ratio(size: &str) -> Option<(u32, u32)> {
    let (width, height) = size.trim().split_once('x')?;
    let width = width.parse::<u32>().ok()?;
    let height = height.parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    fn gcd(mut a: u32, mut b: u32) -> u32 {
        while b != 0 {
            let rem = a % b;
            a = b;
            b = rem;
        }
        a
    }
    let divisor = gcd(width, height);
    Some((width / divisor, height / divisor))
}

fn xai_aspect_ratio(size: Option<&str>, video: bool) -> Option<&'static str> {
    let ratio = reduced_ratio(size?)?;
    match ratio {
        (1, 1) => Some("1:1"),
        (16, 9) => Some("16:9"),
        (9, 16) => Some("9:16"),
        (4, 3) => Some("4:3"),
        (3, 4) => Some("3:4"),
        (3, 2) => Some("3:2"),
        (2, 3) => Some("2:3"),
        // These are supported for images, but not xAI video generation.
        (2, 1) if !video => Some("2:1"),
        (1, 2) if !video => Some("1:2"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// xai.images_json
// ---------------------------------------------------------------------------

/// xAI synchronous image generation/editing.  Unlike OpenAI, edits are JSON
/// and local image inputs ride as base64 data URLs.
pub struct XaiImagesJsonAdapter;

#[async_trait]
impl ProtocolAdapter for XaiImagesJsonAdapter {
    fn id(&self) -> &'static str {
        IMAGES_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        match &call.request {
            TaskRequest::ImageGeneration(req) => submit_image_generation(http, call, req).await,
            TaskRequest::ImageEdit(req) => submit_image_edit(http, call, req).await,
            other => Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{IMAGES_ADAPTER_ID} cannot serve task {:?}", other.task()),
            )),
        }
    }
}

fn image_generation_body(call: &ResolvedCall, req: &ImageGenRequest) -> Value {
    let mut body = Map::new();
    apply_allowed(
        &mut body,
        &call.model_params,
        &req.extra,
        &["aspect_ratio", "resolution", "response_format", "storage_options", "user"],
    );
    body.insert("model".into(), Value::String(call.model.clone()));
    body.insert("prompt".into(), Value::String(req.prompt.clone()));
    body.insert("n".into(), Value::from(req.count));
    if !body.contains_key("aspect_ratio")
        && let Some(ratio) = xai_aspect_ratio(req.size.as_deref(), false)
    {
        body.insert("aspect_ratio".into(), Value::String(ratio.into()));
    }
    Value::Object(body)
}

async fn submit_image_generation(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageGenRequest,
) -> Result<TaskOutcome, InvokeError> {
    let url = xai_submit_url(call, "/images/generations");
    let body = image_generation_body(call, req);
    let resp = post_json(http, &url, MEDIA_TIMEOUT, &call.connection.auth, &body).await?;
    parse_image_http_response(resp).await
}

fn image_edit_body(call: &ResolvedCall, req: &ImageEditRequest) -> Result<Value, InvokeError> {
    if req.inputs.iter().any(|input| input.role == "mask") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "xAI JSON image edits do not support a mask input",
        ));
    }
    if req.inputs.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "xAI images/edits requires at least one input image",
        ));
    }

    let mut body = Map::new();
    apply_allowed(
        &mut body,
        &call.model_params,
        &req.extra,
        &["aspect_ratio", "resolution", "response_format", "storage_options", "user"],
    );
    body.insert("model".into(), Value::String(call.model.clone()));
    body.insert("prompt".into(), Value::String(req.prompt.clone()));
    body.insert("n".into(), Value::from(req.count));
    if !body.contains_key("aspect_ratio")
        && let Some(ratio) = xai_aspect_ratio(req.size.as_deref(), false)
    {
        body.insert("aspect_ratio".into(), Value::String(ratio.into()));
    }

    let images = req
        .inputs
        .iter()
        .map(|input| json!({"type": "image_url", "url": data_uri(&input.mime, &input.bytes)}))
        .collect::<Vec<_>>();
    if images.len() == 1 {
        body.insert("image".into(), images.into_iter().next().expect("one image"));
    } else {
        body.insert("images".into(), Value::Array(images));
    }
    Ok(Value::Object(body))
}

async fn submit_image_edit(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageEditRequest,
) -> Result<TaskOutcome, InvokeError> {
    let url = xai_submit_url(call, "/images/edits");
    let body = image_edit_body(call, req)?;
    let resp = post_json(http, &url, MEDIA_TIMEOUT, &call.connection.auth, &body).await?;
    parse_image_http_response(resp).await
}

async fn parse_image_http_response(resp: reqwest::Response) -> Result<TaskOutcome, InvokeError> {
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    let value: Value = resp.json().await.map_err(|e| InvokeError::parse(format!("invalid xAI images JSON: {e}")))?;
    Ok(TaskOutcome::Done(TaskResult::Assets(parse_images_response(&value)?)))
}

fn parse_images_response(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("xAI images response missing 'data' array"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("xAI images response 'data' array is empty"));
    }
    data.iter()
        .map(|item| {
            let mime = item.get("mime_type").and_then(Value::as_str).map(str::to_owned);
            if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
                let bytes = decode_b64(b64)
                    .ok_or_else(|| InvokeError::parse("xAI images b64_json is not valid base64"))?;
                Ok(ProducedAsset {
                    data: ProducedData::Bytes(bytes),
                    mime: Some(mime.unwrap_or_else(|| "image/jpeg".into())),
                })
            } else if let Some(url) = item.get("url").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                Ok(ProducedAsset { data: ProducedData::Url(url.into()), mime })
            } else {
                Err(InvokeError::parse("xAI images data item has neither b64_json nor url"))
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// xai.video_jobs
// ---------------------------------------------------------------------------

/// xAI asynchronous video generation: submit `/videos/generations`, then poll
/// `/videos/{request_id}` until the temporary `video.url` is available.
pub struct XaiVideoJobsAdapter;

#[async_trait]
impl ProtocolAdapter for XaiVideoJobsAdapter {
    fn id(&self) -> &'static str {
        VIDEO_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::VideoGeneration
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::VideoGeneration(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{VIDEO_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = xai_submit_url(call, "/videos/generations");
        let body = video_generation_body(call, req);
        let resp = post_json(http, &url, MEDIA_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid xAI video submit JSON: {e}")))?;
        let request_id = value
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| InvokeError::parse("xAI video submit response missing 'request_id'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
            remote_id: request_id.into(),
            poll_state: json!({"status_url": xai_video_status_url(call, request_id)}),
        }))
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let status_url = non_empty_str(&job.poll_state, "status_url")
            .map(str::to_owned)
            .unwrap_or_else(|| xai_video_status_url(call, &job.remote_id));
        let resp = get_request(http, &status_url, POLL_TIMEOUT, &call.connection.auth).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid xAI video status JSON: {e}")))?;
        match parse_video_status(&value)? {
            XaiVideoState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
                remote_id: job.remote_id.clone(),
                poll_state: json!({"status_url": status_url}),
            })),
            XaiVideoState::Done(url) => Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
                data: ProducedData::Url(url),
                mime: Some("video/mp4".into()),
            }]))),
            XaiVideoState::Failed(message) => Err(InvokeError::new(InvokeErrorKind::JobFailed, message)),
        }
    }
}

fn video_generation_body(call: &ResolvedCall, req: &VideoGenRequest) -> Value {
    let mut body = Map::new();
    apply_allowed(
        &mut body,
        &call.model_params,
        &req.extra,
        &[
            "aspect_ratio",
            "duration",
            "image",
            "output",
            "reference_audios",
            "reference_images",
            "resolution",
            "storage_options",
            "user",
        ],
    );
    body.insert("model".into(), Value::String(call.model.clone()));
    body.insert("prompt".into(), Value::String(req.prompt.clone()));
    if let Some(seconds) = req.seconds {
        body.insert("duration".into(), Value::from(seconds));
    }
    if !body.contains_key("aspect_ratio")
        && let Some(ratio) = xai_aspect_ratio(req.size.as_deref(), true)
    {
        body.insert("aspect_ratio".into(), Value::String(ratio.into()));
    }
    if !body.contains_key("resolution")
        && let Some(size) = req.size.as_deref().filter(|s| matches!(*s, "480p" | "720p" | "1080p"))
    {
        body.insert("resolution".into(), Value::String(size.into()));
    }

    if !req.inputs.is_empty() {
        let inputs = req
            .inputs
            .iter()
            .map(|input| json!({"url": data_uri(&input.mime, &input.bytes)}))
            .collect::<Vec<_>>();
        if inputs.len() == 1 {
            body.insert("image".into(), inputs.into_iter().next().expect("one image"));
        } else {
            body.insert("reference_images".into(), Value::Array(inputs));
            body.remove("image");
        }
    }
    Value::Object(body)
}

#[derive(Debug, PartialEq, Eq)]
enum XaiVideoState {
    Pending,
    Done(String),
    Failed(String),
}

fn parse_video_status(value: &Value) -> Result<XaiVideoState, InvokeError> {
    let status = value.get("status").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    match status.as_str() {
        "done" => {
            let url = value
                .get("video")
                .and_then(|video| video.get("url"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| InvokeError::parse("xAI video status is done but missing video.url"))?;
            Ok(XaiVideoState::Done(url.into()))
        }
        "failed" | "expired" => {
            let message = value
                .get("error")
                .and_then(|error| error.get("message").and_then(Value::as_str).or_else(|| error.as_str()))
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or(&status)
                .to_owned();
            Ok(XaiVideoState::Failed(message))
        }
        // pending and unknown/empty statuses are non-terminal.
        _ => Ok(XaiVideoState::Pending),
    }
}

// ---------------------------------------------------------------------------
// xai.tts
// ---------------------------------------------------------------------------

/// xAI batch text-to-speech (`POST /tts`).
pub struct XaiTtsAdapter;

#[async_trait]
impl ProtocolAdapter for XaiTtsAdapter {
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
        let url = xai_submit_url(call, "/tts");
        let body = tts_body(call, req);
        let resp = post_json(http, &url, TTS_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase());
        let codec = body
            .get("output_format")
            .and_then(|format| format.get("codec"))
            .and_then(Value::as_str)
            .unwrap_or("mp3");
        let mime = content_type
            .as_deref()
            .filter(|value| value.starts_with("audio/"))
            .map(str::to_owned)
            .unwrap_or_else(|| mime_for_codec(codec).into());
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;
        let with_timestamps = body.get("with_timestamps").and_then(Value::as_bool).unwrap_or(false);
        let audio = if content_type.as_deref() == Some("application/json") || with_timestamps {
            let envelope: Value = serde_json::from_slice(&bytes)
                .map_err(|e| InvokeError::parse(format!("invalid xAI TTS JSON envelope: {e}")))?;
            let encoded = envelope
                .get("audio")
                .and_then(Value::as_str)
                .ok_or_else(|| InvokeError::parse("xAI TTS JSON envelope missing 'audio'"))?;
            decode_b64(encoded).ok_or_else(|| InvokeError::parse("xAI TTS audio is not valid base64"))?
        } else {
            bytes
        };

        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(audio),
            mime: Some(mime),
        }])))
    }
}

fn tts_body(call: &ResolvedCall, req: &TtsRequest) -> Value {
    let mut body = Map::new();
    apply_allowed(
        &mut body,
        &call.model_params,
        &req.extra,
        &[
            "language",
            "optimize_streaming_latency",
            "output_format",
            "speed",
            "text_normalization",
            "voice_id",
            "with_timestamps",
        ],
    );
    body.insert("text".into(), Value::String(req.text.clone()));
    if let Some(voice) = req.voice.as_deref().map(str::trim).filter(|voice| !voice.is_empty()) {
        body.insert("voice_id".into(), Value::String(voice.into()));
    } else if !body.contains_key("voice_id") {
        body.insert("voice_id".into(), Value::String("eve".into()));
    }
    if !body.contains_key("language") {
        body.insert("language".into(), Value::String("auto".into()));
    }
    if let Some(format) = req.format.as_deref().map(str::trim).filter(|format| !format.is_empty()) {
        let output = body.entry("output_format").or_insert_with(|| json!({}));
        if !output.is_object() {
            *output = json!({});
        }
        output.as_object_mut().expect("object").insert("codec".into(), Value::String(format.into()));
    }
    Value::Object(body)
}

fn mime_for_codec(codec: &str) -> &'static str {
    match codec {
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        "mulaw" | "ulaw" => "audio/basic",
        "alaw" => "audio/alaw",
        _ => "audio/mpeg",
    }
}

// ---------------------------------------------------------------------------
// xai.stt
// ---------------------------------------------------------------------------

/// xAI batch speech-to-text (`POST /stt`, multipart).  Option fields are
/// intentionally appended before `file`, as required for streamable uploads.
pub struct XaiSttAdapter;

#[async_trait]
impl ProtocolAdapter for XaiSttAdapter {
    fn id(&self) -> &'static str {
        STT_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechRecognition(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{STT_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = xai_submit_url(call, "/stt");
        let options = stt_options(call, req);
        let build_form = || -> Result<Form, InvokeError> {
            let mut form = Form::new();
            // xAI explicitly requires every option to precede the file part.
            for (key, value) in &options {
                form = form.text(key.clone(), value.clone());
            }
            let file = Part::bytes(req.audio.bytes.clone())
                .file_name(format!("audio.{}", extension_for_audio_mime(&req.audio.mime)))
                .mime_str(&req.audio.mime)
                .map_err(|e| InvokeError::new(InvokeErrorKind::InvalidParams, format!("invalid audio mime: {e}")))?;
            Ok(form.part("file", file))
        };
        let resp = post_multipart(http, &url, STT_TIMEOUT, &call.connection.auth, build_form).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let body: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid xAI STT JSON: {e}")))?;
        let text = body
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| InvokeError::parse("xAI STT response missing 'text'"))?
            .to_owned();
        let detected_language = body
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|language| !language.is_empty())
            .map(str::to_owned)
            .or_else(|| req.language.clone());
        Ok(TaskOutcome::Done(TaskResult::Transcript {
            text,
            language: detected_language,
            model: Some(call.model.clone()),
        }))
    }
}

fn value_as_form_fields(key: &str, value: &Value, output: &mut Vec<(String, String)>) {
    match value {
        Value::String(value) => output.push((key.into(), value.clone())),
        Value::Bool(value) => output.push((key.into(), value.to_string())),
        Value::Number(value) => output.push((key.into(), value.to_string())),
        // `keyterm` is repeatable; arrays encode repeated multipart fields.
        Value::Array(values) if key == "keyterm" => {
            for value in values {
                if let Some(value) = value.as_str().map(str::trim).filter(|value| !value.is_empty()) {
                    output.push((key.into(), value.into()));
                }
            }
        }
        _ => {}
    }
}

fn stt_options(call: &ResolvedCall, req: &AsrRequest) -> Vec<(String, String)> {
    const KEYS: &[&str] = &[
        "audio_format",
        "sample_rate",
        "language",
        "format",
        "multichannel",
        "channels",
        "diarize",
        "keyterm",
        "filler_words",
        "vad_threshold",
    ];
    let mut merged = Map::new();
    apply_allowed(&mut merged, &call.model_params, &req.extra, KEYS);
    if let Some(language) = req.language.as_deref().map(str::trim).filter(|language| !language.is_empty()) {
        merged.insert("language".into(), Value::String(language.into()));
    }
    // xAI has no OpenAI-style free-form transcription prompt.  Do not silently
    // reinterpret it as `keyterm` (which is limited to 50 characters and has
    // narrower semantics); configured/request-extra `keyterm` values above are
    // the explicit escape hatch.

    let mut output = Vec::new();
    for key in KEYS {
        if let Some(value) = merged.get(*key) {
            value_as_form_fields(key, value, &mut output);
        }
    }
    output
}

fn extension_for_audio_mime(mime: &str) -> &'static str {
    match mime {
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/webm" => "webm",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;
    use crate::types::{InputAsset, TtsRequest};

    fn xai_base(server: &MockServer) -> String {
        format!("{}/v1", server.uri())
    }

    fn test_http() -> reqwest::Client {
        // The developer machine may export an HTTP proxy.  Loopback WireMock
        // requests must bypass it or the proxy answers 502 before the mock is
        // reached.
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn input(role: &str, mime: &str, bytes: &[u8]) -> InputAsset {
        InputAsset { id: None, role: role.into(), bytes: bytes.into(), mime: mime.into() }
    }

    #[tokio::test]
    async fn image_generation_uses_v1_json_and_infers_aspect_ratio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "grok-imagine-image-quality",
                "prompt": "a red fox",
                "n": 2,
                "aspect_ratio": "1:1",
                "resolution": "2k"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"url": "https://cdn/image.jpeg", "mime_type": "image/jpeg"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a red fox".into(),
            count: 2,
            size: Some("1024x1024".into()),
            quality: None,
            extra: json!({}),
        });
        let mut call = call(&xai_base(&server), "grok-imagine-image-quality", request);
        call.model_params = json!({"request_defaults": {"resolution": "2k"}});
        let outcome = XaiImagesJsonAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected image assets") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://cdn/image.jpeg"));
        assert_eq!(assets[0].mime.as_deref(), Some("image/jpeg"));
    }

    #[tokio::test]
    async fn image_edit_is_json_data_uri_not_multipart() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(json!({
                "model": "grok-imagine-image-quality",
                "prompt": "add a hat",
                "image": {"type": "image_url", "url": "data:image/png;base64,aGk="}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"b64_json": "aGk=", "mime_type": "image/png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "add a hat".into(),
            count: 1,
            size: None,
            inputs: vec![input("image", "image/png", b"hi")],
            extra: json!({}),
        });
        let call = call(&xai_base(&server), "grok-imagine-image-quality", request);
        let outcome = XaiImagesJsonAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected image assets") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hi"));
    }

    #[test]
    fn image_edit_uses_images_for_multiple_inputs_and_rejects_mask() {
        let request = ImageEditRequest {
            prompt: "merge".into(),
            count: 1,
            size: None,
            inputs: vec![input("image", "image/png", b"a"), input("image", "image/jpeg", b"b")],
            extra: json!({}),
        };
        let multi_call =
            call("https://api.x.ai/v1", "grok-imagine-image-quality", TaskRequest::ImageEdit(request.clone()));
        let body = image_edit_body(&multi_call, &request).unwrap();
        assert_eq!(body["images"].as_array().unwrap().len(), 2);
        assert!(body.get("image").is_none());

        let masked = ImageEditRequest {
            inputs: vec![input("image", "image/png", b"a"), input("mask", "image/png", b"m")],
            ..request
        };
        let masked_call =
            call("https://api.x.ai/v1", "grok-imagine-image-quality", TaskRequest::ImageEdit(masked.clone()));
        assert_eq!(image_edit_body(&masked_call, &masked).unwrap_err().kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn video_submit_and_poll_use_distinct_official_paths() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos/generations"))
            .and(body_partial_json(json!({
                "model": "grok-imagine-video-1.5",
                "prompt": "waves",
                "duration": 6,
                "resolution": "720p",
                "image": {"url": "data:image/png;base64,aGk="}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"request_id": "req-1"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "done",
                "video": {"url": "https://vidgen.x.ai/tmp.mp4"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "waves".into(),
            seconds: Some(6),
            size: Some("720p".into()),
            inputs: vec![input("image", "image/png", b"hi")],
            extra: json!({}),
        });
        let call = call(&xai_base(&server), "grok-imagine-video-1.5", request);
        let http = test_http();
        let TaskOutcome::Pending(job) = XaiVideoJobsAdapter.submit(&http, &call).await.unwrap()
        else {
            panic!("expected pending job")
        };
        assert_eq!(job.adapter_id, VIDEO_ADAPTER_ID);
        assert_eq!(job.remote_id, "req-1");
        assert_eq!(job.poll_state["status_url"], format!("{}/v1/videos/req-1", server.uri()));

        let outcome = XaiVideoJobsAdapter.poll(&http, &call, &job).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected video asset") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url.ends_with("tmp.mp4")));
        assert_eq!(assets[0].mime.as_deref(), Some("video/mp4"));
    }

    #[test]
    fn video_status_parser_handles_pending_failures_and_missing_url() {
        assert_eq!(parse_video_status(&json!({"status": "pending"})).unwrap(), XaiVideoState::Pending);
        assert_eq!(
            parse_video_status(&json!({"status": "expired", "error": {"message": "gone"}})).unwrap(),
            XaiVideoState::Failed("gone".into())
        );
        assert_eq!(parse_video_status(&json!({"status": "done"})).unwrap_err().kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn tts_posts_native_shape_and_returns_raw_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tts"))
            .and(body_partial_json(json!({
                "text": "hello",
                "voice_id": "ara",
                "language": "zh",
                "output_format": {"codec": "wav", "sample_rate": 24000},
                "speed": 1.1
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/wav")
                    .set_body_bytes(b"RIFFaudio".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::SpeechSynthesis(TtsRequest {
            text: "hello".into(),
            voice: Some("ara".into()),
            format: Some("wav".into()),
            extra: json!({"language": "zh", "speed": 1.1}),
        });
        let mut call = call(&xai_base(&server), "xai-tts", request);
        call.model_params = json!({"request_defaults": {"output_format": {"codec": "mp3", "sample_rate": 24000}}});
        let outcome = XaiTtsAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected audio asset") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"RIFFaudio"));
        assert_eq!(assets[0].mime.as_deref(), Some("audio/wav"));
    }

    #[tokio::test]
    async fn stt_posts_options_before_file_and_parses_transcript() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/stt"))
            .and(body_string_contains("name=\"language\""))
            .and(body_string_contains("name=\"diarize\""))
            .and(body_string_contains("name=\"keyterm\""))
            .and(body_string_contains("name=\"file\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "text": "hello world",
                "language": "en",
                "duration": 1.2
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::SpeechRecognition(AsrRequest {
            audio: input("audio", "audio/mpeg", b"audio"),
            language: Some("en".into()),
            prompt: Some("Nomifun".into()),
            extra: json!({"diarize": true}),
        });
        let mut call = call(&xai_base(&server), "xai-stt", request);
        call.model_params = json!({"request_defaults": {"keyterm": ["Nomifun"]}});
        let outcome = XaiSttAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, language, model }) = outcome else {
            panic!("expected transcript")
        };
        assert_eq!(text, "hello world");
        assert_eq!(language.as_deref(), Some("en"));
        assert_eq!(model.as_deref(), Some("xai-stt"));

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        let language_pos = body.find("name=\"language\"").unwrap();
        let file_pos = body.find("name=\"file\"").unwrap();
        assert!(language_pos < file_pos, "xAI requires STT options before the file part");
        assert!(!body.contains("name=\"model\""), "xAI STT does not take an OpenAI model field");
    }

    #[test]
    fn endpoint_and_poll_overrides_are_resolved_independently() {
        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "p".into(),
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        });
        let mut call = call("https://api.x.ai/v1", "grok-imagine-video-1.5", request);
        call.model_params = json!({
            "endpoint": "/custom/video-submit",
            "poll_endpoint": "https://status.example/jobs/{request_id}"
        });
        assert_eq!(xai_submit_url(&call, "/videos/generations"), "https://api.x.ai/v1/custom/video-submit");
        assert_eq!(xai_video_status_url(&call, "r-9"), "https://status.example/jobs/r-9");
    }
}
