//! `ark.images` / `ark.video_jobs` — Volcengine Ark (方舟) image generation
//! and asynchronous video tasks (protocol per
//! `docs/specs/2026-07-28-provider-protocol-variance.zh.md` §3, Ark domain:
//! `ark.cn-beijing.volces.com`, Bearer ARK_API_KEY on the default connection).
//!
//! Ark lives under `/api/v3` rather than the OpenAI `/v1` convention. The
//! selected capability supplies the exact protocol endpoint for both adapters.
//!
//! - [`ArkImagesAdapter`] (`"ark.images"`, ImageGeneration/ImageEdit): sync
//!   `POST {root}/api/v3/images/generations`, OpenAI-shaped body plus Ark
//!   private knobs (`watermark`/`seed`/`guidance_scale` whitelisted from
//!   `extra`). Image edits use the same endpoint and carry one or more ordered
//!   Base64 data URIs in Ark's `image` field; response reuses
//!   [`crate::adapters::openai_images::parse_images_response`] (url|b64_json).
//! - [`ArkVideoJobsAdapter`] (`"ark.video_jobs"`, VideoGeneration): async
//!   `POST {root}/api/v3/contents/generations/tasks` → `GET .../tasks/{id}`.
//!   Generation params use top-level `resolution`, `ratio`, and `duration`
//!   fields; an i2v first frame rides as a `content[]` data-URI `image_url`
//!   entry. Status
//!   vocabulary `queued/running` → Pending, `succeeded` → `content.video_url`
//!   (24 h URL, mime unknown), `failed/cancelled` →
//!   [`InvokeErrorKind::JobFailed`].

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, resolve_endpoint};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::manifest::expand_protocol_endpoint_template;
use crate::transport::{
    encode_b64, error_from_response, get_request, inline_image_response_body_limit, post_json,
    read_json_capped, validate_image_request_count,
};
use crate::types::{
    ImageEditRequest, InputAsset, JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest,
    TaskResult, VideoGenRequest,
};

use super::json_request_body;
use super::openai_images::parse_images_response_limited;

/// Generous ceiling for image generation / video-task submission.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
/// Poll round-trips are cheap status reads.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
/// Keep Ark aligned with the creation engine and Canvas product safety ceiling.
const MAX_ARK_IMAGE_EDIT_INPUTS: usize = 8;
/// Ark's documented per-reference upload limit for Seedream image input.
const MAX_ARK_INPUT_IMAGE_BYTES: usize = 10 * 1024 * 1024;

// ---------------------------------------------------------------------------
// ark.images
// ---------------------------------------------------------------------------

/// Ark synchronous `/api/v3/images/generations` (seedream family).
pub struct ArkImagesAdapter;

#[async_trait]
impl ProtocolAdapter for ArkImagesAdapter {
    fn id(&self) -> &'static str {
        "ark.images"
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let (body, expected_images) = build_ark_image_request(call)?;
        let url = call.endpoint_url()?;

        let resp = post_json(http, &url, SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = read_json_capped(
            resp,
            inline_image_response_body_limit(expected_images),
            "ark images",
        )
        .await?;
        Ok(TaskOutcome::Done(TaskResult::Assets(
            parse_images_response_limited(&value, expected_images)?,
        )))
    }
}

fn validate_ark_output_count(count: u32) -> Result<usize, InvokeError> {
    let count = validate_image_request_count(count)?;
    if count != 1 {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "ark.images does not support count greater than 1",
        ));
    }
    Ok(count)
}

fn ark_input_data_uri(input: &InputAsset, index: usize) -> Result<String, InvokeError> {
    if input.bytes.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("ark.images input image {index} is empty"),
        ));
    }
    if input.bytes.len() > MAX_ARK_INPUT_IMAGE_BYTES {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "ark.images input image {index} exceeds the {MAX_ARK_INPUT_IMAGE_BYTES}-byte limit"
            ),
        ));
    }
    let essence = input
        .mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let mime = match essence.as_str() {
        "image/png" => "image/png",
        "image/jpeg" | "image/jpg" => "image/jpeg",
        _ => {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "ark.images input image {index} has unsupported MIME {:?}; expected image/png or image/jpeg",
                    input.mime
                ),
            ));
        }
    };
    Ok(format!("data:{mime};base64,{}", encode_b64(&input.bytes)))
}

fn ark_edit_images(request: &ImageEditRequest) -> Result<Value, InvokeError> {
    if request.inputs.iter().any(|input| input.role == "mask") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "ark.images does not support a separate mask input; send a visibly marked reference image instead",
        ));
    }
    if request.inputs.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "ark.images image editing requires at least one input image",
        ));
    }
    if request.inputs.len() > MAX_ARK_IMAGE_EDIT_INPUTS {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "ark.images image editing supports at most {MAX_ARK_IMAGE_EDIT_INPUTS} input images, got {}",
                request.inputs.len()
            ),
        ));
    }

    let mut images = request
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| ark_input_data_uri(input, index + 1))
        .collect::<Result<Vec<_>, _>>()?;
    if images.len() == 1 {
        let image = images.pop().ok_or_else(|| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "ark.images image editing lost its validated input image",
            )
        })?;
        Ok(Value::String(image))
    } else {
        Ok(Value::Array(images.into_iter().map(Value::String).collect()))
    }
}

fn build_ark_image_request(call: &ResolvedCall) -> Result<(Value, usize), InvokeError> {
    let (prompt, count, size, extra, image) = match &call.request {
        TaskRequest::ImageGeneration(request) => {
            if request.quality.is_some() {
                return Err(InvokeError::new(
                    InvokeErrorKind::InvalidParams,
                    "ark.images does not support the quality parameter",
                ));
            }
            (
                &request.prompt,
                request.count,
                request.size.as_ref(),
                &request.extra,
                None,
            )
        }
        TaskRequest::ImageEdit(request) => (
            &request.prompt,
            request.count,
            request.size.as_ref(),
            &request.extra,
            Some(ark_edit_images(request)?),
        ),
        other => {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("ark.images cannot serve task {:?}", other.task()),
            ));
        }
    };
    let expected_images = validate_ark_output_count(count)?;
    let mut typed = json!({
        "model": call.model,
        "prompt": prompt,
        "response_format": "b64_json",
    });
    if let Some(size) = size {
        typed["size"] = Value::String(size.clone());
    }
    if let Some(image) = image {
        typed["image"] = image;
    }
    Ok((
        json_request_body(&call.model_params, extra, typed)?,
        expected_images,
    ))
}

// ---------------------------------------------------------------------------
// ark.video_jobs
// ---------------------------------------------------------------------------

const VIDEO_ADAPTER_ID: &str = "ark.video_jobs";

/// Ark asynchronous `/api/v3/contents/generations/tasks` submit→poll
/// (seedance family).
pub struct ArkVideoJobsAdapter;

#[async_trait]
impl ProtocolAdapter for ArkVideoJobsAdapter {
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
                format!("ark.video_jobs cannot serve task {:?}", call.request.task()),
            ));
        };
        validate_ark_video_model(&call.model)?;
        let url = call.endpoint_url()?;
        let body = json_request_body(
            &call.model_params,
            &req.extra,
            build_video_submit_body(&call.model, req)?,
        )?;

        let resp = post_json(http, &url, SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid ark task JSON", &e))?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| InvokeError::parse("ark task submit response missing 'id'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
            config_revision: call.config_revision,
            remote_id: id.to_string(),
            poll_state: json!({}),
        }))
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let template = call
            .model_params
            .get("poll_endpoint")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InvokeError::config("ark.video_jobs requires an injected poll endpoint")
            })?;
        let endpoint = expand_protocol_endpoint_template(
            &call.protocol,
            call.task,
            "poll_endpoint",
            template,
            &job.remote_id,
        )?;
        let status_url = call.credentialed_http_url(
            &resolve_endpoint(&call.connection.base_url, &endpoint),
            "poll_endpoint",
        )?;
        let resp = get_request(http, &status_url, POLL_TIMEOUT, &call.connection.auth).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid ark task status JSON", &e))?;

        match parse_video_task_status(&value)? {
            ArkTaskState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            ArkTaskState::Failed(msg) => Err(InvokeError::new(InvokeErrorKind::JobFailed, msg)),
            // The provider URL is short-lived (~24 h); the caller fetches it.
            ArkTaskState::Done(video_url) => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                ProducedAsset { data: ProducedData::Url(video_url), mime: None },
            ]))),
        }
    }
}

/// Build the current Ark video-task submit body. Generation parameters use
/// top-level `resolution`, `ratio`, and `duration` fields; the first input asset
/// (i2v first frame), when present, rides as a data-URI `image_url` content entry.
/// Pure — unit tested.
fn ark_seedance_api_model_id_hint(model: &str) -> Option<&'static str> {
    match model.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "doubao-seedance-1.5-pro" => Some("doubao-seedance-1-5-pro-251215"),
        "doubao-seedance-2.0-mini" => Some("doubao-seedance-2-0-mini-260615"),
        _ => None,
    }
}

fn validate_ark_video_model(model: &str) -> Result<(), InvokeError> {
    let normalized = model.trim().to_ascii_lowercase().replace('_', "-");
    if let Some(model_id) = ark_seedance_api_model_id_hint(model) {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Ark video model {model:?} is a console display name, not an API Model ID; \
                 use {model_id:?} exactly or an \"ep-\" inference endpoint ID shown in \
                 the Ark console"
            ),
        ));
    }
    if normalized.starts_with("doubao-seed-") && !normalized.starts_with("doubao-seedance-") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Ark video generation cannot use Doubao Seed chat model {model:?}; \
                 Seed and Seedance are different model families. Configure the exact \
                 Seedance Model ID (normally starting with \"doubao-seedance-\") or \
                 an \"ep-\" inference endpoint ID shown in the Ark console"
            ),
        ));
    }
    Ok(())
}

fn reduced_ratio(width: u32, height: u32) -> Option<String> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut a = width;
    let mut b = height;
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    Some(format!("{}:{}", width / a, height / a))
}

fn apply_ark_video_size(body: &mut Value, size: &str) -> Result<(), InvokeError> {
    let normalized = size.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Ok(());
    }
    if matches!(normalized.as_str(), "480p" | "720p" | "1080p") {
        body["resolution"] = Value::String(normalized);
        return Ok(());
    }
    if matches!(
        normalized.as_str(),
        "adaptive" | "1:1" | "16:9" | "9:16" | "4:3" | "3:4" | "21:9"
    ) {
        body["ratio"] = Value::String(normalized);
        return Ok(());
    }

    let Some((width, height)) = normalized.split_once('x') else {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Ark video size {size:?} is not supported; use 480p/720p/1080p, \
                 a supported aspect ratio, or WIDTHxHEIGHT"
            ),
        ));
    };
    let width = width.trim().parse::<u32>().map_err(|_| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("Ark video size {size:?} has an invalid width"),
        )
    })?;
    let height = height.trim().parse::<u32>().map_err(|_| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("Ark video size {size:?} has an invalid height"),
        )
    })?;
    let short_edge = width.min(height);
    let resolution = match short_edge {
        480 => "480p",
        720 => "720p",
        1080 => "1080p",
        _ => {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "Ark video size {size:?} has unsupported short edge {short_edge}; \
                     expected 480, 720, or 1080 pixels"
                ),
            ));
        }
    };
    let ratio = reduced_ratio(width, height).ok_or_else(|| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("Ark video size {size:?} must use non-zero dimensions"),
        )
    })?;
    if !matches!(ratio.as_str(), "1:1" | "16:9" | "9:16" | "4:3" | "3:4" | "21:9") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("Ark video size {size:?} has unsupported aspect ratio {ratio}"),
        ));
    }
    body["resolution"] = Value::String(resolution.to_owned());
    body["ratio"] = Value::String(ratio);
    Ok(())
}

pub(crate) fn build_video_submit_body(
    model: &str,
    req: &VideoGenRequest,
) -> Result<Value, InvokeError> {
    validate_ark_video_model(model)?;
    let mut content = vec![json!({"type": "text", "text": req.prompt})];
    if let Some(input) = req.inputs.first() {
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", input.mime, encode_b64(&input.bytes)) }
        }));
    }
    let mut body = json!({ "model": model, "content": content });
    if let Some(size) = &req.size {
        apply_ark_video_size(&mut body, size)?;
    }
    if let Some(seconds) = req.seconds {
        body["duration"] = json!(seconds);
    }
    Ok(body)
}

/// The distilled state of an Ark video task.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArkTaskState {
    Pending,
    /// `content.video_url` of a succeeded task.
    Done(String),
    Failed(String),
}

/// Map an Ark task-status body (`queued/running/succeeded/failed/cancelled`)
/// to [`ArkTaskState`]. A succeeded task must carry `content.video_url`; a
/// failed/cancelled one reports `error.message` | `error` string | the status
/// itself. Unknown/absent statuses keep waiting. Pure — unit tested.
pub(crate) fn parse_video_task_status(value: &Value) -> Result<ArkTaskState, InvokeError> {
    let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
    match status.as_str() {
        "succeeded" => {
            let url = value
                .get("content")
                .and_then(|c| c.get("video_url"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| InvokeError::parse("ark task succeeded but missing content.video_url"))?;
            Ok(ArkTaskState::Done(url.to_string()))
        }
        "failed" | "cancelled" | "canceled" => {
            let msg = value
                .get("error")
                .and_then(|e| e.get("message").and_then(|m| m.as_str()).or_else(|| e.as_str()))
                .unwrap_or(&status)
                .to_string();
            Ok(ArkTaskState::Failed(msg))
        }
        // "queued" | "running" | "" | anything unknown → keep waiting.
        _ => Ok(ArkTaskState::Pending),
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::{ImageGenRequest, InputAsset};

    fn ark_base(base_url: &str) -> String {
        let base_url = base_url.trim_end_matches('/');
        if base_url.ends_with("/api/v3") {
            base_url.to_owned()
        } else {
            format!("{base_url}/api/v3")
        }
    }

    fn ark_call_with_endpoint(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let mut call = call_with_endpoint(base_url, model, protocol, endpoint, request);
        call.platform = "ark".into();
        call
    }

    fn image_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        ark_call_with_endpoint(&ark_base(base_url), model, "ark.images", "/images/generations", request)
    }

    fn video_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let mut call = ark_call_with_endpoint(
            &ark_base(base_url),
            model,
            "ark.video_jobs",
            "/contents/generations/tasks",
            request,
        );
        call.model_params["poll_endpoint"] =
            Value::String("/contents/generations/tasks/{id}".into());
        call
    }

    fn image_request(size: Option<&str>, extra: Value) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a fox".into(),
            count: 1,
            size: size.map(str::to_string),
            quality: None,
            extra,
        })
    }

    fn edit_request(inputs: Vec<InputAsset>) -> TaskRequest {
        TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "combine image one and image two".into(),
            count: 1,
            size: Some("2048x2048".into()),
            inputs,
            extra: json!({"watermark": false}),
        })
    }

    fn image_input(role: &str, mime: &str, bytes: &[u8]) -> InputAsset {
        InputAsset {
            id: None,
            role: role.into(),
            bytes: bytes.to_vec(),
            mime: mime.into(),
        }
    }

    fn video_request(size: Option<&str>, seconds: Option<u32>, inputs: Vec<InputAsset>) -> TaskRequest {
        TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "a wave".into(),
            seconds,
            size: size.map(str::to_string),
            inputs,
            extra: json!({}),
        })
    }

    fn job(remote_id: &str) -> JobHandle {
        JobHandle { adapter_id: VIDEO_ADAPTER_ID.into(), config_revision: 1, remote_id: remote_id.into(), poll_state: json!({}) }
    }

    // -- pure body/status fixtures ---------------------------------------------

    #[test]
    fn video_body_uses_current_top_level_parameters() {
        let TaskRequest::VideoGeneration(req) = video_request(Some("1280x720"), Some(5), vec![]) else {
            unreachable!()
        };
        let body =
            build_video_submit_body("doubao-seedance-2-0-mini-260615", &req).unwrap();
        assert_eq!(body["model"], "doubao-seedance-2-0-mini-260615");
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "a wave");
        assert_eq!(body["resolution"], "720p");
        assert_eq!(body["ratio"], "16:9");
        assert_eq!(body["duration"], 5);
    }

    #[test]
    fn video_body_without_params_has_bare_prompt() {
        let TaskRequest::VideoGeneration(req) = video_request(None, None, vec![]) else { unreachable!() };
        let body = build_video_submit_body("doubao-seedance-2-0-mini-260615", &req).unwrap();
        assert_eq!(body["content"][0]["text"], "a wave");
        assert!(body.get("resolution").is_none());
        assert!(body.get("ratio").is_none());
        assert!(body.get("duration").is_none());
    }

    #[test]
    fn video_body_appends_first_input_as_data_uri_image() {
        // "aGk=" is base64("hi").
        let inputs = vec![
            InputAsset { id: None, role: "first_frame".into(), bytes: b"hi".to_vec(), mime: "image/png".into() },
            InputAsset { id: None, role: "extra".into(), bytes: b"nope".to_vec(), mime: "image/png".into() },
        ];
        let TaskRequest::VideoGeneration(req) = video_request(None, None, inputs) else { unreachable!() };
        let body = build_video_submit_body("doubao-seedance-2-0-mini-260615", &req).unwrap();
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "only the first input asset is attached");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,aGk=");
    }

    #[test]
    fn video_body_rejects_doubao_seed_chat_models() {
        let TaskRequest::VideoGeneration(req) = video_request(None, None, vec![]) else {
            unreachable!()
        };
        for model in ["Doubao-Seed-2.0-mini", "doubao-seed-2-0-mini-260428"] {
            let error = build_video_submit_body(model, &req).unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains("Seed and Seedance"), "{}", error.message);
        }
    }

    #[test]
    fn video_body_rejects_console_display_names_with_exact_api_id_hint() {
        let TaskRequest::VideoGeneration(req) = video_request(None, None, vec![]) else {
            unreachable!()
        };
        for (display_name, model_id) in [
            (
                "Doubao-Seedance-1.5-pro",
                "doubao-seedance-1-5-pro-251215",
            ),
            (
                "Doubao-Seedance-2.0-mini",
                "doubao-seedance-2-0-mini-260615",
            ),
        ] {
            let error = build_video_submit_body(display_name, &req).unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains("console display name"), "{}", error.message);
            assert!(error.message.contains(model_id), "{}", error.message);
        }
    }

    #[test]
    fn video_body_rejects_unrepresentable_dimensions() {
        let TaskRequest::VideoGeneration(req) = video_request(Some("720x480"), None, vec![])
        else {
            unreachable!()
        };
        let error =
            build_video_submit_body("doubao-seedance-2-0-mini-260615", &req).unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("unsupported aspect ratio"));
    }

    #[test]
    fn task_status_vocabulary_maps_to_states() {
        for s in ["queued", "running", "", "mystery"] {
            assert_eq!(parse_video_task_status(&json!({"status": s})).unwrap(), ArkTaskState::Pending, "{s}");
        }
        assert_eq!(parse_video_task_status(&json!({})).unwrap(), ArkTaskState::Pending);

        let done = json!({"status": "succeeded", "content": {"video_url": "https://cdn/v.mp4"}});
        assert_eq!(parse_video_task_status(&done).unwrap(), ArkTaskState::Done("https://cdn/v.mp4".into()));
        // succeeded without a video_url is a parse error.
        let err = parse_video_task_status(&json!({"status": "succeeded"})).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);

        // failure message: error.message → error string → status itself.
        let v = json!({"status": "failed", "error": {"message": "content blocked"}});
        assert_eq!(parse_video_task_status(&v).unwrap(), ArkTaskState::Failed("content blocked".into()));
        let v2 = json!({"status": "cancelled", "error": "user cancelled"});
        assert_eq!(parse_video_task_status(&v2).unwrap(), ArkTaskState::Failed("user cancelled".into()));
        let v3 = json!({"status": "failed"});
        assert_eq!(parse_video_task_status(&v3).unwrap(), ArkTaskState::Failed("failed".into()));
    }

    // -- ark.images wiremock ----------------------------------------------------

    #[tokio::test]
    async fn images_posts_api_v3_body_with_open_provider_fields_and_decodes_b64() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "doubao-seedream",
                "prompt": "a fox",
                "response_format": "b64_json",
                "size": "1024x1024",
                "watermark": false,
                "seed": 42,
                "steps": 9,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let extra = json!({"watermark": false, "seed": 42, "steps": 9});
        let call = image_call(&server.uri(), "doubao-seedream", image_request(Some("1024x1024"), extra));
        let out = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));

        // Unknown provider-native fields are deliberately forwarded so newly
        // documented Ark options work without a NomiFun release.
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["steps"], 9);
        assert!(body.get("size").is_some());
    }

    #[tokio::test]
    async fn image_edit_posts_ordered_reference_data_uris_to_the_generation_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "doubao-seedream-5-0-260128",
                "prompt": "combine image one and image two",
                "image": [
                    "data:image/png;base64,b25l",
                    "data:image/jpeg;base64,dHdv"
                ],
                "size": "2048x2048",
                "response_format": "b64_json",
                "watermark": false
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"b64_json": "aGk="}]})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let request = edit_request(vec![
            image_input("reference", "image/png", b"one"),
            image_input("reference", "image/jpeg", b"two"),
        ]);
        let call = image_call(&server.uri(), "doubao-seedream-5-0-260128", request);
        let out = ArkImagesAdapter
            .submit(&reqwest::Client::new(), &call)
            .await
            .unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 1));
    }

    #[test]
    fn image_edit_uses_a_scalar_for_one_reference_and_typed_input_wins() {
        let request = edit_request(vec![image_input(
            "reference",
            "image/jpg; charset=binary",
            b"one",
        )]);
        let mut call = image_call(
            "https://ark.cn-beijing.volces.com/api/v3",
            "ep-seedream",
            request,
        );
        call.model_params["image"] = Value::String("must-not-win".into());

        let (body, expected_images) = build_ark_image_request(&call).unwrap();

        assert_eq!(expected_images, 1);
        assert_eq!(body["image"], "data:image/jpeg;base64,b25l");
        assert_eq!(body["watermark"], false);
    }

    #[test]
    fn image_edit_rejects_unsafe_or_unsupported_reference_inputs_without_truncation() {
        let cases = [
            (
                edit_request(vec![]),
                "requires at least one input image",
            ),
            (
                edit_request(
                    (0..=MAX_ARK_IMAGE_EDIT_INPUTS)
                        .map(|_| image_input("reference", "image/png", b"x"))
                        .collect(),
                ),
                "supports at most 8 input images",
            ),
            (
                edit_request(vec![image_input("mask", "image/png", b"mask")]),
                "does not support a separate mask",
            ),
            (
                edit_request(vec![image_input("reference", "image/webp", b"webp")]),
                "unsupported MIME",
            ),
            (
                edit_request(vec![image_input(
                    "reference",
                    "image/png",
                    &vec![0; MAX_ARK_INPUT_IMAGE_BYTES + 1],
                )]),
                "exceeds the 10485760-byte limit",
            ),
        ];

        for (request, expected) in cases {
            let call = image_call(
                "https://ark.cn-beijing.volces.com/api/v3",
                "ep-seedream",
                request,
            );
            let error = build_ark_image_request(&call).unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains(expected), "{}", error.message);
        }
    }

    #[tokio::test]
    async fn images_base_url_with_api_v3_suffix_does_not_double() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"url": "https://cdn/x.png"}]})))
            .expect(1)
            .mount(&server)
            .await;

        let base = format!("{}/api/v3", server.uri());
        let call = image_call(&base, "doubao-seedream", image_request(None, json!({})));
        let out = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert!(matches!(&assets[0].data, ProducedData::Url(u) if u == "https://cdn/x.png"));
    }

    #[tokio::test]
    async fn image_contract_response_limits_reject_more_results_than_requested() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"url": "https://cdn/a.png"},
                {"url": "https://cdn/b.png"}
            ]})))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "doubao-seedream", image_request(None, json!({})));
        let error = ArkImagesAdapter
            .submit(&reqwest::Client::new(), &call)
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("returned 2 images"), "{}", error.message);
    }

    #[tokio::test]
    async fn image_contract_ark_rejects_unsupported_count_and_quality_before_network() {
        for (count, quality, parameter) in [
            (2, None, "count"),
            (1, Some("high"), "quality"),
        ] {
            let request = TaskRequest::ImageGeneration(ImageGenRequest {
                prompt: "a fox".into(),
                count,
                size: None,
                quality: quality.map(str::to_string),
                extra: json!({}),
            });
            let call = image_call("http://127.0.0.1:9", "doubao-seedream", request);
            let error = ArkImagesAdapter
                .submit(&reqwest::Client::new(), &call)
                .await
                .unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains(parameter), "{}", error.message);
        }
    }

    #[tokio::test]
    async fn images_params_endpoint_override_wins() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/images"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let call = ark_call_with_endpoint(
            &server.uri(),
            "doubao-seedream",
            "ark.images",
            "/custom/images",
            image_request(None, json!({})),
        );
        let out = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 1));
    }

    #[tokio::test]
    async fn images_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad ark key"))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "doubao-seedream", image_request(None, json!({})));
        let err = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("bad ark key"), "message: {}", err.message);
    }

    // -- ark.video_jobs wiremock --------------------------------------------------

    #[tokio::test]
    async fn video_submit_posts_top_level_params_and_returns_pending_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/contents/generations/tasks"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "doubao-seedance-2-0-mini-260615",
                "content": [{"type": "text", "text": "a wave"}],
                "resolution": "720p",
                "ratio": "16:9",
                "duration": 5,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "cgt-1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;

        let call = video_call(
            &server.uri(),
            "doubao-seedance-2-0-mini-260615",
            video_request(Some("1280x720"), Some(5), vec![]),
        );
        let out = ArkVideoJobsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.adapter_id, "ark.video_jobs");
        assert_eq!(handle.remote_id, "cgt-1");
        assert_eq!(handle.poll_state, json!({}));
    }

    #[tokio::test]
    async fn video_submit_attaches_first_frame_as_data_uri() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/contents/generations/tasks"))
            .and(body_partial_json(json!({
                "content": [
                    {"type": "text", "text": "a wave"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,aGk="}},
                ],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "cgt-2"})))
            .expect(1)
            .mount(&server)
            .await;

        let inputs =
            vec![InputAsset { id: None, role: "first_frame".into(), bytes: b"hi".to_vec(), mime: "image/png".into() }];
        let call = video_call(&server.uri(), "doubao-seedance", video_request(None, None, inputs));
        let out = ArkVideoJobsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Pending(h) if h.remote_id == "cgt-2"));
    }

    #[tokio::test]
    async fn video_submit_missing_id_is_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/contents/generations/tasks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "queued"})))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let err = ArkVideoJobsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn video_poll_queued_then_succeeded_yields_url_asset() {
        let server = MockServer::start().await;
        // First poll: still queued.
        Mock::given(method("GET"))
            .and(path("/api/v3/contents/generations/tasks/cgt-1"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "cgt-1", "status": "queued"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second poll: succeeded with the (short-lived) video URL.
        Mock::given(method("GET"))
            .and(path("/api/v3/contents/generations/tasks/cgt-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cgt-1", "status": "succeeded", "content": {"video_url": "https://cdn/v.mp4"}
            })))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let http = reqwest::Client::new();

        let first = ArkVideoJobsAdapter.poll(&http, &call, &job("cgt-1")).await.unwrap();
        let TaskOutcome::Pending(handle) = first else { panic!("expected Pending on queued") };
        assert_eq!(handle.remote_id, "cgt-1");
        assert_eq!(handle.adapter_id, "ark.video_jobs");

        let second = ArkVideoJobsAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = second else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Url(u) if u == "https://cdn/v.mp4"));
        assert_eq!(assets[0].mime, None, "URL asset mime is unknown until fetched");
    }

    #[tokio::test]
    async fn video_poll_failed_is_job_failed_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/contents/generations/tasks/cgt-9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cgt-9", "status": "failed", "error": {"message": "content blocked"}
            })))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let err = ArkVideoJobsAdapter.poll(&reqwest::Client::new(), &call, &job("cgt-9")).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "content blocked");
    }

    #[tokio::test]
    async fn video_capability_endpoints_apply_independently() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/video-tasks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "cgt-c1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/custom/video-tasks/cgt-c1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cgt-c1", "status": "succeeded", "content": {"video_url": "https://cdn/c.mp4"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = ark_call_with_endpoint(
            &server.uri(),
            "doubao-seedance",
            "ark.video_jobs",
            "/custom/video-tasks",
            video_request(None, None, vec![]),
        );
        call.model_params["poll_endpoint"] = Value::String("/custom/video-tasks/{id}".into());
        let http = reqwest::Client::new();

        let out = ArkVideoJobsAdapter.submit(&http, &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.remote_id, "cgt-c1");

        let done = ArkVideoJobsAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = done else { panic!("expected Done(Assets)") };
        assert!(matches!(&assets[0].data, ProducedData::Url(u) if u == "https://cdn/c.mp4"));
    }

    #[tokio::test]
    async fn video_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/contents/generations/tasks"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad ark key"))
            .mount(&server)
            .await;

        let call = video_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let err = ArkVideoJobsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
