//! Native SiliconFlow media protocols.
//!
//! SiliconFlow's media APIs are not OpenAI media-compatible even though chat
//! uses an OpenAI-compatible endpoint:
//! - `siliconflow.images`: both generation and editing use JSON
//!   `POST /v1/images/generations`; edits add `image`/`image2`/`image3` data
//!   URIs, and successful responses use `images[].url`.
//! - `siliconflow.video_jobs`: `POST /v1/video/submit` returns `requestId`,
//!   then `POST /v1/video/status` (also JSON) is polled until it returns a
//!   video URL.
//! - `siliconflow.audio_speech`: `POST /v1/audio/speech` accepts provider-native
//!   JSON and returns a raw (optionally streamed) audio body.  SiliconFlow voice
//!   ids are model-scoped; this adapter never invents OpenAI's `alloy` voice.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, resolve_endpoint};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{MAX_ARTIFACT_BYTES, encode_b64, error_from_response, post_json, read_body_capped};
use crate::types::{
    ImageEditRequest, ImageGenRequest, JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest,
    TaskResult, TtsRequest, VideoGenRequest,
};

use super::json_request_body;

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
pub const AUDIO_SPEECH_ADAPTER_ID: &str = "siliconflow.audio_speech";
const VIDEO_ADAPTER_ID: &str = "siliconflow.video_jobs";

fn audio_speech_url(call: &ResolvedCall) -> Result<String, InvokeError> {
    call.endpoint_url()
}

fn image_url(call: &ResolvedCall) -> Result<String, InvokeError> {
    call.endpoint_url()
}

fn video_submit_url(call: &ResolvedCall) -> Result<String, InvokeError> {
    call.endpoint_url()
}

fn video_status_url(call: &ResolvedCall) -> Result<String, InvokeError> {
    let endpoint = call
        .model_params
        .get("poll_endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| InvokeError::config("siliconflow.video_jobs requires an injected status endpoint"))?;
    Ok(resolve_endpoint(&call.connection.base_url, endpoint))
}

/// Merge whitelisted provider-native optional parameters. Connection/model
/// defaults are applied first and per-request `extra` values override them.
fn merge_optional(body: &mut Map<String, Value>, model_params: &Value, extra: &Value, keys: &[&str]) {
    for source in [model_params, extra] {
        for key in keys {
            if let Some(value) = source.get(*key) {
                body.insert((*key).to_string(), value.clone());
            }
        }
    }
}

fn merge_image_parameters(body: &mut Map<String, Value>, model_params: &Value, extra: &Value) {
    merge_optional(
        body,
        model_params,
        extra,
        &["negative_prompt", "seed", "num_inference_steps", "guidance_scale"],
    );

    // Preserve the old generic SD aliases while putting the official
    // SiliconFlow field names on the wire. Exact native names win.
    if !body.contains_key("num_inference_steps") {
        if let Some(value) = extra.get("steps").or_else(|| model_params.get("steps")) {
            body.insert("num_inference_steps".into(), value.clone());
        }
    }
    if !body.contains_key("guidance_scale") {
        if let Some(value) = extra.get("cfg_scale").or_else(|| model_params.get("cfg_scale")) {
            body.insert("guidance_scale".into(), value.clone());
        }
    }
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode_b64(bytes))
}

// ---------------------------------------------------------------------------
// siliconflow.audio_speech
// ---------------------------------------------------------------------------

/// SiliconFlow JSON `/v1/audio/speech` protocol for both the `.cn` and `.com`
/// services.  The response is raw audio (`application/audio` in the official
/// schema), not JSON or SSE framing.
pub struct SiliconFlowAudioSpeechAdapter;

#[async_trait]
impl ProtocolAdapter for SiliconFlowAudioSpeechAdapter {
    fn id(&self) -> &'static str {
        AUDIO_SPEECH_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechSynthesis
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::SpeechSynthesis(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("{AUDIO_SPEECH_ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };

        let body = build_audio_speech_body(call, req)?;
        let url = audio_speech_url(call)?;
        let resp = post_json(http, &url, SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }

        // SiliconFlow documents `application/audio`, so the official response
        // header alone does not identify mp3/opus/wav/pcm.  An explicit request
        // format is authoritative; otherwise the provider default is mp3.  A
        // more specific audio/* header is still preserved when one is supplied.
        let response_content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_ascii_lowercase());
        let requested_format = body.get("response_format").and_then(Value::as_str);
        let mime = requested_format
            .map(siliconflow_speech_mime)
            .map(str::to_owned)
            .or_else(|| response_content_type.filter(|value| value.starts_with("audio/")))
            .unwrap_or_else(|| "audio/mpeg".to_owned());
        let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;

        Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
            data: ProducedData::Bytes(bytes),
            mime: Some(mime),
        }])))
    }
}

const SILICONFLOW_TTS_OPTIONAL_FIELDS: &[&str] = &[
    "stream",
    "sample_rate",
    "speed",
    "gain",
    "max_tokens",
    "references",
];

fn invalid_tts_param(message: impl Into<String>) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

fn validate_audio_speech_options(body: &Map<String, Value>) -> Result<(), InvokeError> {
    if let Some(voice) = body.get("voice").and_then(Value::as_str)
        && !voice.contains(':')
    {
        return Err(invalid_tts_param(format!(
            "SiliconFlow TTS voice {voice:?} must be a provider voice id such as '<model>:<voice>' or 'speech:<voice-id>'; unscoped OpenAI voice ids are not supported"
        )));
    }

    if let Some(value) = body.get("stream")
        && !value.is_boolean()
    {
        return Err(invalid_tts_param("SiliconFlow TTS field 'stream' must be a boolean"));
    }

    if let Some(value) = body.get("sample_rate") {
        let sample_rate = value
            .as_u64()
            .ok_or_else(|| invalid_tts_param("SiliconFlow TTS field 'sample_rate' must be an integer"))?;
        let format = body.get("response_format").and_then(Value::as_str).unwrap_or("mp3");
        let allowed = match format {
            "opus" => sample_rate == 48_000,
            "wav" | "pcm" => matches!(sample_rate, 8_000 | 16_000 | 24_000 | 32_000 | 44_100),
            // SiliconFlow defaults to mp3 when response_format is omitted.
            _ => matches!(sample_rate, 32_000 | 44_100),
        };
        if !allowed {
            return Err(invalid_tts_param(format!(
                "SiliconFlow TTS sample_rate {sample_rate} is not supported for response_format {format:?}"
            )));
        }
    }

    for (field, min, max) in [("speed", 0.25, 4.0), ("gain", -10.0, 10.0)] {
        if let Some(value) = body.get(field) {
            let number = value
                .as_f64()
                .ok_or_else(|| invalid_tts_param(format!("SiliconFlow TTS field '{field}' must be a number")))?;
            if !(min..=max).contains(&number) {
                return Err(invalid_tts_param(format!(
                    "SiliconFlow TTS field '{field}' must be between {min} and {max}"
                )));
            }
        }
    }

    if let Some(value) = body.get("max_tokens")
        && value.as_u64().filter(|tokens| *tokens > 0).is_none()
    {
        return Err(invalid_tts_param(
            "SiliconFlow TTS field 'max_tokens' must be a positive integer",
        ));
    }

    if let Some(value) = body.get("references") {
        let references = value
            .as_array()
            .filter(|references| !references.is_empty())
            .ok_or_else(|| invalid_tts_param("SiliconFlow TTS field 'references' must be a non-empty array"))?;
        for (index, reference) in references.iter().enumerate() {
            let reference = reference.as_object().ok_or_else(|| {
                invalid_tts_param(format!("SiliconFlow TTS references[{index}] must be an object"))
            })?;
            for field in ["audio", "text"] {
                if reference
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(invalid_tts_param(format!(
                        "SiliconFlow TTS references[{index}].{field} must be a non-empty string"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn configured_string<'a>(model_params: &'a Value, extra: &'a Value, key: &str) -> Result<Option<&'a str>, InvokeError> {
    let Some(value) = extra.get(key).or_else(|| model_params.get(key)) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(str::trim)
        .map(|value| (!value.is_empty()).then_some(value))
        .ok_or_else(|| invalid_tts_param(format!("SiliconFlow TTS field '{key}' must be a string")))
}

fn build_audio_speech_body(call: &ResolvedCall, req: &TtsRequest) -> Result<Value, InvokeError> {
    if req.text.trim().is_empty() {
        return Err(invalid_tts_param("SiliconFlow TTS input must not be empty"));
    }

    let mut body = Map::from_iter([
        ("model".into(), Value::String(call.model.clone())),
        ("input".into(), Value::String(req.text.clone())),
    ]);
    merge_optional(
        &mut body,
        &call.model_params,
        &req.extra,
        SILICONFLOW_TTS_OPTIONAL_FIELDS,
    );

    // A typed request voice wins over a configured default.  Absence stays
    // absent: unlike the OpenAI adapter, this must never synthesize `alloy`.
    let voice = match req.voice.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(voice) => Some(voice),
        None => configured_string(&call.model_params, &req.extra, "voice")?,
    };
    if let Some(voice) = voice {
        body.insert("voice".into(), Value::String(voice.to_owned()));
    }

    let format = match req.format.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(format) => Some(format),
        None => configured_string(&call.model_params, &req.extra, "response_format")?,
    };
    if let Some(format) = format {
        if !matches!(format, "mp3" | "opus" | "wav" | "pcm") {
            return Err(invalid_tts_param(format!(
                "SiliconFlow TTS response_format {format:?} must be mp3, opus, wav, or pcm"
            )));
        }
        body.insert("response_format".into(), Value::String(format.to_owned()));
    }

    let body = json_request_body(&call.model_params, &req.extra, Value::Object(body))?;
    let body = body
        .as_object()
        .expect("shared JSON body helper always returns an object");
    validate_audio_speech_options(body)?;
    if body.contains_key("voice") && body.contains_key("references") {
        return Err(invalid_tts_param(
            "SiliconFlow TTS fields 'voice' and 'references' are mutually exclusive",
        ));
    }
    Ok(Value::Object(body.clone()))
}

fn siliconflow_speech_mime(format: &str) -> &'static str {
    match format {
        "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        _ => "audio/mpeg",
    }
}

// ---------------------------------------------------------------------------
// siliconflow.images
// ---------------------------------------------------------------------------

pub struct SiliconFlowImagesAdapter;

#[async_trait]
impl ProtocolAdapter for SiliconFlowImagesAdapter {
    fn id(&self) -> &'static str {
        "siliconflow.images"
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let body = match &call.request {
            TaskRequest::ImageGeneration(req) => build_image_generation_body(call, req)?,
            TaskRequest::ImageEdit(req) => build_image_edit_body(call, req)?,
            other => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("siliconflow.images cannot serve task {:?}", other.task()),
                ));
            }
        };

        let resp = post_json(http, &image_url(call)?, SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid SiliconFlow images JSON", &e))?;
        Ok(TaskOutcome::Done(TaskResult::Assets(parse_images(&value)?)))
    }
}

fn build_image_generation_body(call: &ResolvedCall, req: &ImageGenRequest) -> Result<Value, InvokeError> {
    let mut body = Map::from_iter([
        ("model".into(), Value::String(call.model.clone())),
        ("prompt".into(), Value::String(req.prompt.clone())),
    ]);
    merge_image_parameters(&mut body, &call.model_params, &req.extra);
    // One image is the API default. `batch_size` is only accepted by models
    // that advertise batching (currently Kolors), so avoid sending it for the
    // overwhelmingly common single-image request.
    if req.count != 1 {
        body.insert("batch_size".into(), Value::from(req.count));
    }
    if let Some(size) = &req.size {
        body.insert("image_size".into(), Value::String(size.clone()));
    }
    json_request_body(&call.model_params, &req.extra, Value::Object(body))
}

fn build_image_edit_body(call: &ResolvedCall, req: &ImageEditRequest) -> Result<Value, InvokeError> {
    if req.count != 1 {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "SiliconFlow image editing supports exactly one output image",
        ));
    }
    let images: Vec<_> = req.inputs.iter().filter(|input| input.role != "mask").take(3).collect();
    if images.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "SiliconFlow image editing requires at least one non-mask input image",
        ));
    }

    let mut body = Map::from_iter([
        ("model".into(), Value::String(call.model.clone())),
        ("prompt".into(), Value::String(req.prompt.clone())),
    ]);
    merge_image_parameters(&mut body, &call.model_params, &req.extra);
    // Current Qwen image-edit models reject image_size. A caller that really
    // needs it can still supply the native field explicitly in `extra` or
    // model params, but the generic typed `size` is intentionally not mapped.
    merge_optional(&mut body, &call.model_params, &req.extra, &["image_size"]);

    for (index, input) in images.into_iter().enumerate() {
        let field = if index == 0 { "image".to_string() } else { format!("image{}", index + 1) };
        body.insert(field, Value::String(data_uri(&input.mime, &input.bytes)));
    }
    json_request_body(&call.model_params, &req.extra, Value::Object(body))
}

fn parse_images(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    let images = value
        .get("images")
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("SiliconFlow images response missing 'images' array"))?;
    if images.is_empty() {
        return Err(InvokeError::parse("SiliconFlow images response 'images' array is empty"));
    }
    images
        .iter()
        .map(|image| {
            let url = image
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| InvokeError::parse("SiliconFlow image result missing 'url'"))?;
            Ok(ProducedAsset { data: ProducedData::Url(url.to_string()), mime: None })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// siliconflow.video_jobs
// ---------------------------------------------------------------------------

pub struct SiliconFlowVideoJobsAdapter;

#[async_trait]
impl ProtocolAdapter for SiliconFlowVideoJobsAdapter {
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
                format!("siliconflow.video_jobs cannot serve task {:?}", call.request.task()),
            ));
        };
        let body = build_video_submit_body(call, req)?;
        let resp = post_json(http, &video_submit_url(call)?, SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid SiliconFlow video submit JSON", &e))?;
        let request_id = value
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| InvokeError::parse("SiliconFlow video submit response missing 'requestId'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
            config_revision: call.config_revision,
            remote_id: request_id.to_string(),
            poll_state: json!({}),
        }))
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let body = json!({"requestId": job.remote_id});
        let status_url = call.credentialed_http_url(&video_status_url(call)?, "poll_endpoint")?;
        let resp = post_json(http, &status_url, POLL_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid SiliconFlow video status JSON", &e))?;

        match parse_video_status(&value)? {
            SiliconFlowVideoState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            SiliconFlowVideoState::Failed(reason) => {
                Err(InvokeError::new(InvokeErrorKind::JobFailed, reason))
            }
            SiliconFlowVideoState::Done(urls) => Ok(TaskOutcome::Done(TaskResult::Assets(
                urls.into_iter()
                    .map(|url| ProducedAsset { data: ProducedData::Url(url), mime: None })
                    .collect(),
            ))),
        }
    }
}

fn build_video_submit_body(call: &ResolvedCall, req: &VideoGenRequest) -> Result<Value, InvokeError> {
    let mut body = Map::from_iter([
        ("model".into(), Value::String(call.model.clone())),
        ("prompt".into(), Value::String(req.prompt.clone())),
    ]);
    merge_optional(
        &mut body,
        &call.model_params,
        &req.extra,
        &["negative_prompt", "seed", "image_size"],
    );
    if let Some(size) = &req.size {
        body.insert("image_size".into(), Value::String(size.clone()));
    }
    if let Some(input) = req.inputs.first() {
        body.insert("image".into(), Value::String(data_uri(&input.mime, &input.bytes)));
    }
    json_request_body(&call.model_params, &req.extra, Value::Object(body))
}

#[derive(Debug, PartialEq, Eq)]
enum SiliconFlowVideoState {
    Pending,
    Done(Vec<String>),
    Failed(String),
}

fn parse_video_status(value: &Value) -> Result<SiliconFlowVideoState, InvokeError> {
    let status = value.get("status").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    match status.as_str() {
        "succeed" | "succeeded" | "success" => parse_completed_video_urls(value),
        "failed" | "failure" => {
            let reason = value
                .get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .unwrap_or("SiliconFlow video generation failed")
                .to_string();
            Ok(SiliconFlowVideoState::Failed(reason))
        }
        // Some historical responses omitted `status` while already carrying
        // results; recognize those rather than polling a completed job forever.
        "" if value.pointer("/results/videos").and_then(Value::as_array).is_some() => {
            parse_completed_video_urls(value)
        }
        // InQueue / InProgress (and unknown transient states) remain pending.
        _ => Ok(SiliconFlowVideoState::Pending),
    }
}

fn parse_completed_video_urls(value: &Value) -> Result<SiliconFlowVideoState, InvokeError> {
    let videos = value
        .get("results")
        .and_then(|results| results.get("videos"))
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("SiliconFlow video succeeded but missing results.videos"))?;
    let urls: Vec<String> = videos
        .iter()
        .filter_map(|video| video.get("url").and_then(Value::as_str))
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .collect();
    if urls.is_empty() {
        return Err(InvokeError::parse(
            "SiliconFlow video succeeded but results.videos contains no URL",
        ));
    }
    Ok(SiliconFlowVideoState::Done(urls))
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::InputAsset;

    fn siliconflow_base(base: &str) -> String {
        let base = base.trim_end_matches('/');
        if base.ends_with("/v1") {
            base.to_owned()
        } else {
            format!("{base}/v1")
        }
    }

    fn siliconflow_call_with_endpoint(
        base: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let mut call = call_with_endpoint(base, model, protocol, endpoint, request);
        call.platform = "siliconflow".into();
        call
    }

    fn audio_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        siliconflow_call_with_endpoint(
            &siliconflow_base(base),
            model,
            AUDIO_SPEECH_ADAPTER_ID,
            "/audio/speech",
            request,
        )
    }

    fn image_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        siliconflow_call_with_endpoint(
            &siliconflow_base(base),
            model,
            "siliconflow.images",
            "/images/generations",
            request,
        )
    }

    fn video_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let mut call = siliconflow_call_with_endpoint(
            &siliconflow_base(base),
            model,
            VIDEO_ADAPTER_ID,
            "/video/submit",
            request,
        );
        call.model_params["poll_endpoint"] = Value::String("/video/status".into());
        call
    }

    fn image_input(bytes: &[u8]) -> InputAsset {
        InputAsset { id: None, role: "image".into(), bytes: bytes.to_vec(), mime: "image/png".into() }
    }

    fn video_request() -> TaskRequest {
        TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "a wave".into(),
            seconds: Some(5),
            size: Some("1280x720".into()),
            inputs: vec![image_input(b"hi")],
            extra: json!({"negative_prompt": "blur", "seed": 9}),
        })
    }

    fn job(id: &str) -> JobHandle {
        JobHandle { adapter_id: VIDEO_ADAPTER_ID.into(), config_revision: 1, remote_id: id.into(), poll_state: json!({}) }
    }

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn tts_request(voice: Option<&str>, format: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::SpeechSynthesis(TtsRequest {
            text: "你好，SiliconFlow".into(),
            voice: voice.map(str::to_owned),
            format: format.map(str::to_owned),
            extra,
        })
    }

    #[test]
    fn audio_speech_uses_the_official_cn_and_global_paths() {
        for (base, expected) in [
            (
                "https://api.siliconflow.cn/v1",
                "https://api.siliconflow.cn/v1/audio/speech",
            ),
            (
                "https://api.siliconflow.com/v1",
                "https://api.siliconflow.com/v1/audio/speech",
            ),
        ] {
            let call = audio_call(base, "FunAudioLLM/CosyVoice2-0.5B", tts_request(None, None, json!({})));
            assert_eq!(audio_speech_url(&call).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn audio_speech_forwards_open_provider_fields_and_returns_raw_audio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer sk-test"))
            .and(header("content-type", "application/json"))
            .and(body_partial_json(json!({
                "model": "FunAudioLLM/CosyVoice2-0.5B",
                "input": "你好，SiliconFlow",
                "voice": "FunAudioLLM/CosyVoice2-0.5B:alex",
                "response_format": "mp3",
                "stream": true,
                "sample_rate": 32000,
                "speed": 1.25,
                "gain": 1,
                "max_tokens": 128,
                "future_option": "enabled"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/audio")
                    .set_body_bytes(b"ID3siliconflow".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let request = tts_request(
            Some("FunAudioLLM/CosyVoice2-0.5B:alex"),
            Some("mp3"),
            json!({
                "stream": true,
                "speed": 1.25,
                "future_option": "enabled",
                "response_format": 42
            }),
        );
        let mut call = audio_call(&format!("{}/v1", server.uri()), "FunAudioLLM/CosyVoice2-0.5B", request);
        call.model_params = json!({
            "endpoint": "/audio/speech",
            "stream": false,
            "sample_rate": 32000,
            "speed": 1.0,
            "gain": 1,
            "max_tokens": 128,
            "voice": 42,
            "future_option": "model-default"
        });

        let out = SiliconFlowAudioSpeechAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected audio asset") };
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].mime.as_deref(), Some("audio/mpeg"));
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"ID3siliconflow"));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["future_option"], "enabled");
        assert_eq!(body["stream"], json!(true), "request extras override model defaults");
        assert_eq!(body["response_format"], json!("mp3"), "typed format wins over raw params");
    }

    #[test]
    fn audio_speech_never_invents_alloy_and_rejects_voice_with_references() {
        let references = json!([{
            "audio": "data:audio/wav;base64,UklGRg==",
            "text": "参考音频文本"
        }]);
        let call = audio_call(
            "https://api.siliconflow.cn/v1",
            "fnlp/MOSS-TTSD-v0.5",
            tts_request(None, Some("wav"), json!({"references": references.clone()})),
        );
        let body = build_audio_speech_body(
            &call,
            match &call.request {
                TaskRequest::SpeechSynthesis(req) => req,
                _ => unreachable!(),
            },
        )
        .unwrap();
        assert!(body.get("voice").is_none());
        assert!(!body.to_string().contains("alloy"));
        assert_eq!(body["references"], references);

        let conflicting = audio_call(
            "https://api.siliconflow.cn/v1",
            "fnlp/MOSS-TTSD-v0.5",
            tts_request(Some("fnlp/MOSS-TTSD-v0.5:alex"), None, json!({"references": references})),
        );
        let error = build_audio_speech_body(
            &conflicting,
            match &conflicting.request {
                TaskRequest::SpeechSynthesis(req) => req,
                _ => unreachable!(),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("mutually exclusive"));

        let openai_voice = audio_call(
            "https://api.siliconflow.com/v1",
            "FunAudioLLM/CosyVoice2-0.5B",
            tts_request(Some("alloy"), None, json!({})),
        );
        let error = build_audio_speech_body(
            &openai_voice,
            match &openai_voice.request {
                TaskRequest::SpeechSynthesis(req) => req,
                _ => unreachable!(),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("unscoped OpenAI voice ids"));
    }

    #[tokio::test]
    async fn generation_posts_native_url_and_parses_images_urls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "Kwai-Kolors/Kolors",
                "prompt": "a fox",
                "image_size": "1024x1024",
                "batch_size": 2,
                "num_inference_steps": 30,
                "guidance_scale": 6.5
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "images": [{"url": "https://cdn.test/one.png"}, {"url": "https://cdn.test/two.png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a fox".into(),
            count: 2,
            size: Some("1024x1024".into()),
            quality: None,
            extra: json!({"guidance_scale": 6.5}),
        });
        let mut call = image_call(&format!("{}/v1", server.uri()), "Kwai-Kolors/Kolors", request);
        call.model_params = json!({
            "endpoint": "/images/generations",
            "num_inference_steps": 30,
            "guidance_scale": 7.5
        });
        let out = SiliconFlowImagesAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected assets") };
        assert_eq!(assets.len(), 2);
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://cdn.test/one.png"));
    }

    #[tokio::test]
    async fn edit_uses_same_json_endpoint_and_data_uri_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({
                "model": "Qwen/Qwen-Image-Edit-2509",
                "prompt": "add a hat",
                "image": "data:image/png;base64,aGk=",
                "image2": "data:image/png;base64,dHdv",
                "num_inference_steps": 22
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "images": [{"url": "https://cdn.test/edit.png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "add a hat".into(),
            count: 1,
            size: Some("1024x1024".into()),
            inputs: vec![image_input(b"hi"), image_input(b"two")],
            extra: json!({"num_inference_steps": 22}),
        });
        let call = image_call(&server.uri(), "Qwen/Qwen-Image-Edit-2509", request);
        let out = SiliconFlowImagesAdapter.submit(&test_http(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(assets)) if assets.len() == 1));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("image_size").is_none(), "typed size must not break Qwen image-edit");
    }

    #[tokio::test]
    async fn video_submit_returns_request_id_and_posts_native_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/video/submit"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "Wan-AI/Wan2.2-I2V-A14B",
                "prompt": "a wave",
                "image_size": "1280x720",
                "image": "data:image/png;base64,aGk=",
                "negative_prompt": "blur",
                "seed": 9
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"requestId": "req-1"})))
            .expect(1)
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "Wan-AI/Wan2.2-I2V-A14B", video_request());
        let out = SiliconFlowVideoJobsAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected pending") };
        assert_eq!(handle.adapter_id, VIDEO_ADAPTER_ID);
        assert_eq!(handle.remote_id, "req-1");
    }

    #[tokio::test]
    async fn video_poll_posts_request_id_then_parses_result_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/video/status"))
            .and(body_partial_json(json!({"requestId": "req-1"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "InProgress"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/video/status"))
            .and(body_partial_json(json!({"requestId": "req-1"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "Succeed",
                "results": {"videos": [{"url": "https://cdn.test/video.mp4"}]}
            })))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "Wan-AI/Wan2.2-I2V-A14B", video_request());
        let http = test_http();
        let pending = SiliconFlowVideoJobsAdapter.poll(&http, &call, &job("req-1")).await.unwrap();
        let TaskOutcome::Pending(handle) = pending else { panic!("expected pending") };
        let done = SiliconFlowVideoJobsAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = done else { panic!("expected assets") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://cdn.test/video.mp4"));
    }

    #[tokio::test]
    async fn credentialed_cross_origin_poll_requires_explicit_authorization() {
        let selected = MockServer::start().await;
        let foreign = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/video/status"))
            .and(body_partial_json(json!({"requestId": "req-foreign"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "Succeed",
                "results": {"videos": [{"url": "https://cdn.test/video.mp4"}]}
            })))
            .expect(1)
            .mount(&foreign)
            .await;

        let mut call = video_call(&selected.uri(), "Wan-AI/Wan2.2-T2V-A14B", video_request());
        call.model_params = json!({
            "endpoint": "/video/submit",
            "poll_endpoint": format!("{}/video/status", foreign.uri())
        });
        let error = SiliconFlowVideoJobsAdapter
            .poll(&test_http(), &call, &job("req-foreign"))
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Config);
        assert!(error.message.contains("allow_cross_origin_credentials"));
        assert!(foreign.received_requests().await.unwrap().is_empty());

        call.model_params["allow_cross_origin_credentials"] = Value::Bool(true);
        let result = SiliconFlowVideoJobsAdapter
            .poll(&test_http(), &call, &job("req-foreign"))
            .await
            .unwrap();
        assert!(matches!(result, TaskOutcome::Done(TaskResult::Assets(_))));
    }

    #[tokio::test]
    async fn video_failed_status_is_terminal_job_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/video/status"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "Failed", "reason": "moderation blocked"
            })))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "Wan-AI/Wan2.2-T2V-A14B", video_request());
        let error = SiliconFlowVideoJobsAdapter
            .poll(&test_http(), &call, &job("req-2"))
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::JobFailed);
        assert_eq!(error.message, "moderation blocked");
    }

    #[test]
    fn parsers_reject_missing_media_urls() {
        assert_eq!(parse_images(&json!({"images": []})).unwrap_err().kind, InvokeErrorKind::ParseError);
        assert_eq!(
            parse_video_status(&json!({"status": "Succeed", "results": {"videos": []}}))
                .unwrap_err()
                .kind,
            InvokeErrorKind::ParseError
        );
    }
}
