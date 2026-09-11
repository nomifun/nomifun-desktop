//! Agnes media protocols.
//!
//! Agnes image generation looks superficially OpenAI-compatible, but its
//! queue rejects OpenAI's top-level `quality` / `response_format` fields and
//! image editing is JSON on the generations endpoint. Agnes Video v2.0 is a
//! separate JSON async-job contract: submit returns a `video_id`, polling uses
//! `/agnesapi?video_id=...&model_name=...`, and the completed status body
//! carries the output URL directly.

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
    ImageEditRequest, InputAsset, JobHandle, ProducedAsset, ProducedData, TaskOutcome,
    TaskRequest, TaskResult, VideoGenRequest,
};

use super::json_request_body;
use super::openai_images::parse_images_response_limited;

const IMAGE_ADAPTER_ID: &str = "agnes.images";
const VIDEO_ADAPTER_ID: &str = "agnes.video_jobs";
const IMAGE_MODEL: &str = "agnes-image-2.1-flash";
const VIDEO_MODEL: &str = "agnes-video-v2.0";
const DEFAULT_IMAGE_SIZE: &str = "1024x1024";
const DEFAULT_VIDEO_WIDTH: u32 = 1152;
const DEFAULT_VIDEO_HEIGHT: u32 = 768;
const DEFAULT_FRAME_RATE: u32 = 24;
const DEFAULT_NUM_FRAMES: u32 = 121;
const MAX_NUM_FRAMES: u32 = 441;
const MAX_INPUT_IMAGES: usize = 8;
const MAX_INPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(360);
const VIDEO_SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const VIDEO_POLL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_VIDEO_STATUS_BYTES: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// agnes.images
// ---------------------------------------------------------------------------

pub struct AgnesImagesAdapter;

#[async_trait]
impl ProtocolAdapter for AgnesImagesAdapter {
    fn id(&self) -> &'static str {
        IMAGE_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        let (body, expected_images) = build_image_body(call)?;
        let url = call.endpoint_url()?;
        let response = post_json(
            http,
            &url,
            IMAGE_TIMEOUT,
            &call.connection.auth,
            &body,
        )
        .await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        let value: Value = read_json_capped(
            response,
            inline_image_response_body_limit(expected_images),
            "Agnes images",
        )
        .await?;
        Ok(TaskOutcome::Done(TaskResult::Assets(
            parse_images_response_limited(&value, expected_images)?,
        )))
    }
}

fn validate_image_model(model: &str) -> Result<(), InvokeError> {
    if model.trim() == IMAGE_MODEL {
        Ok(())
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "agnes.images implements the {IMAGE_MODEL:?} contract; got {model:?}"
            ),
        ))
    }
}

fn image_data_uri(input: &InputAsset, index: usize, protocol: &str) -> Result<String, InvokeError> {
    if input.bytes.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("{protocol} input image {index} is empty"),
        ));
    }
    if input.bytes.len() > MAX_INPUT_IMAGE_BYTES {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "{protocol} input image {index} exceeds the {MAX_INPUT_IMAGE_BYTES}-byte limit"
            ),
        ));
    }
    let mime = input
        .mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !mime.starts_with("image/") || mime.len() <= "image/".len() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("{protocol} input {index} must be an image"),
        ));
    }
    Ok(format!("data:{mime};base64,{}", encode_b64(&input.bytes)))
}

fn edit_images(request: &ImageEditRequest) -> Result<Vec<String>, InvokeError> {
    if request.inputs.iter().any(|input| input.role == "mask") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "agnes.images does not support a separate mask input",
        ));
    }
    if request.inputs.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "agnes.images image editing requires at least one input image",
        ));
    }
    if request.inputs.len() > MAX_INPUT_IMAGES {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "agnes.images supports at most {MAX_INPUT_IMAGES} input images, got {}",
                request.inputs.len()
            ),
        ));
    }
    request
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| image_data_uri(input, index + 1, IMAGE_ADAPTER_ID))
        .collect()
}

fn build_image_body(call: &ResolvedCall) -> Result<(Value, usize), InvokeError> {
    validate_image_model(&call.model)?;
    let (prompt, count, size, extra, images) = match &call.request {
        TaskRequest::ImageGeneration(request) => (
            &request.prompt,
            request.count,
            request.size.as_deref(),
            &request.extra,
            None,
        ),
        TaskRequest::ImageEdit(request) => (
            &request.prompt,
            request.count,
            request.size.as_deref(),
            &request.extra,
            Some(edit_images(request)?),
        ),
        other => {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("agnes.images cannot serve task {:?}", other.task()),
            ));
        }
    };
    let expected_images = validate_image_request_count(count)?;
    let mut typed = json!({
        "model": call.model,
        "prompt": prompt,
        "n": count,
        "size": size.unwrap_or(DEFAULT_IMAGE_SIZE),
        "extra_body": {
            "response_format": if images.is_some() { "b64_json" } else { "url" },
        },
    });
    if let Some(images) = images {
        typed["extra_body"]["image"] = Value::Array(
            images.into_iter().map(Value::String).collect(),
        );
    }

    let mut body = json_request_body(&call.model_params, extra, typed)?;
    let object = body.as_object_mut().ok_or_else(|| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "Agnes image request body must be an object",
        )
    })?;
    // These OpenAI-style top-level fields are specifically rejected by the
    // Agnes text-image queue. `response_format` is owned by `extra_body`.
    object.remove("quality");
    object.remove("response_format");
    Ok((body, expected_images))
}

// ---------------------------------------------------------------------------
// agnes.video_jobs
// ---------------------------------------------------------------------------

pub struct AgnesVideoJobsAdapter;

#[async_trait]
impl ProtocolAdapter for AgnesVideoJobsAdapter {
    fn id(&self) -> &'static str {
        VIDEO_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::VideoGeneration
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::VideoGeneration(request) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("agnes.video_jobs cannot serve task {:?}", call.request.task()),
            ));
        };
        validate_video_model(&call.model)?;
        let body = json_request_body(
            &call.model_params,
            &request.extra,
            build_video_body(&call.model, &call.model_params, request)?,
        )?;
        let url = call.endpoint_url()?;
        let response = post_json(
            http,
            &url,
            VIDEO_SUBMIT_TIMEOUT,
            &call.connection.auth,
            &body,
        )
        .await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        let value: Value = read_json_capped(
            response,
            MAX_VIDEO_STATUS_BYTES,
            "Agnes video submit",
        )
        .await?;
        let id = value
            .get("video_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InvokeError::parse("Agnes video submit response missing 'video_id'")
            })?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
            config_revision: call.config_revision,
            remote_id: id.to_owned(),
            poll_state: json!({}),
        }))
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        validate_video_model(&call.model)?;
        let url = video_poll_url(call, &job.remote_id)?;
        let response = get_request(
            http,
            &url,
            VIDEO_POLL_TIMEOUT,
            &call.connection.auth,
        )
        .await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        let value: Value = read_json_capped(
            response,
            MAX_VIDEO_STATUS_BYTES,
            "Agnes video status",
        )
        .await?;
        match parse_video_status(&value)? {
            AgnesVideoState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            AgnesVideoState::Failed(message) => {
                Err(InvokeError::new(InvokeErrorKind::JobFailed, message))
            }
            AgnesVideoState::Done(url) => Ok(TaskOutcome::Done(TaskResult::Assets(vec![
                ProducedAsset {
                    data: ProducedData::Url(url),
                    mime: Some("video/mp4".into()),
                },
            ]))),
        }
    }
}

fn validate_video_model(model: &str) -> Result<(), InvokeError> {
    if model.trim() == VIDEO_MODEL {
        Ok(())
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "agnes.video_jobs implements the {VIDEO_MODEL:?} contract; got {model:?}"
            ),
        ))
    }
}

fn positive_u32_field(value: Option<&Value>, field: &str) -> Result<Option<u32>, InvokeError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let parsed = value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!("agnes.video_jobs {field} must be a positive integer"),
            )
        })?;
    Ok(Some(parsed))
}

fn request_u32_field(
    configured: &Value,
    extra: &Value,
    field: &str,
) -> Result<Option<u32>, InvokeError> {
    positive_u32_field(
        extra.get(field).or_else(|| configured.get(field)),
        field,
    )
}

fn video_dimensions(size: Option<&str>) -> Result<(u32, u32), InvokeError> {
    let Some(size) = size.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok((DEFAULT_VIDEO_WIDTH, DEFAULT_VIDEO_HEIGHT));
    };
    let normalized = size.to_ascii_lowercase();
    let Some((width, height)) = normalized.split_once('x') else {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("Agnes video size {size:?} must be WIDTHxHEIGHT"),
        ));
    };
    let parse = |value: &str, dimension: &str| {
        value.trim().parse::<u32>().map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!("Agnes video size {size:?} has an invalid {dimension}"),
            )
        })
    };
    let width = parse(width, "width")?;
    let height = parse(height, "height")?;
    if width == 0 || height == 0 || width % 8 != 0 || height % 8 != 0 {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Agnes video dimensions must be positive multiples of 8, got {width}x{height}"
            ),
        ));
    }
    Ok((width, height))
}

fn frames_for_seconds(seconds: u32, frame_rate: u32) -> Result<u32, InvokeError> {
    let raw = seconds.checked_mul(frame_rate).ok_or_else(|| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "Agnes video duration is too large",
        )
    })?;
    let groups = raw.saturating_sub(1).saturating_add(4) / 8;
    let frames = groups
        .checked_mul(8)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "Agnes video frame count is too large",
            )
        })?;
    if frames > MAX_NUM_FRAMES {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Agnes Video v2.0 supports at most {MAX_NUM_FRAMES} frames; {seconds}s at {frame_rate} fps requires {frames}"
            ),
        ));
    }
    Ok(frames)
}

fn validate_num_frames(frames: u32) -> Result<u32, InvokeError> {
    if frames <= MAX_NUM_FRAMES && frames % 8 == 1 {
        Ok(frames)
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "agnes.video_jobs num_frames must be at most {MAX_NUM_FRAMES} and satisfy 8n+1, got {frames}"
            ),
        ))
    }
}

fn build_video_body(
    model: &str,
    configured: &Value,
    request: &VideoGenRequest,
) -> Result<Value, InvokeError> {
    validate_video_model(model)?;
    let (width, height) = video_dimensions(request.size.as_deref())?;
    let frame_rate = request_u32_field(configured, &request.extra, "frame_rate")?
        .unwrap_or(DEFAULT_FRAME_RATE);
    if !(1..=60).contains(&frame_rate) {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("agnes.video_jobs frame_rate must be from 1 to 60, got {frame_rate}"),
        ));
    }
    let num_frames = match request.seconds {
        Some(seconds) => frames_for_seconds(seconds, frame_rate)?,
        None => validate_num_frames(
            request_u32_field(configured, &request.extra, "num_frames")?
                .unwrap_or(DEFAULT_NUM_FRAMES),
        )?,
    };

    if request.inputs.len() > 1 {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "Agnes Video v2.0 image-to-video accepts one input image, got {}",
                request.inputs.len()
            ),
        ));
    }
    let mut body = json!({
        "model": model,
        "prompt": request.prompt,
        "width": width,
        "height": height,
        "num_frames": num_frames,
        "frame_rate": frame_rate,
    });
    if let Some(input) = request.inputs.first() {
        body["image"] = Value::String(image_data_uri(input, 1, VIDEO_ADAPTER_ID)?);
    }
    Ok(body)
}

fn video_poll_url(call: &ResolvedCall, remote_id: &str) -> Result<String, InvokeError> {
    let template = call
        .model_params
        .get("poll_endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            InvokeError::config("agnes.video_jobs requires an injected poll endpoint")
        })?;
    let endpoint = expand_protocol_endpoint_template(
        &call.protocol,
        call.task,
        "poll_endpoint",
        template,
        remote_id,
    )?;
    let resolved = resolve_endpoint(&call.connection.base_url, &endpoint);
    let mut parsed = reqwest::Url::parse(&resolved)
        .map_err(|_| InvokeError::config("Agnes video poll endpoint is not a valid URL"))?;
    let retained = parsed
        .query_pairs()
        .filter(|(key, _)| key != "model_name")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    {
        let mut query = parsed.query_pairs_mut();
        for (key, value) in retained {
            query.append_pair(&key, &value);
        }
        query.append_pair("model_name", &call.model);
    }
    call.credentialed_http_url(parsed.as_str(), "poll_endpoint")
}

#[derive(Debug, PartialEq, Eq)]
enum AgnesVideoState {
    Pending,
    Done(String),
    Failed(String),
}

fn parse_video_status(value: &Value) -> Result<AgnesVideoState, InvokeError> {
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match status.as_str() {
        "completed" | "succeeded" | "success" | "done" => {
            let url = value
                .get("url")
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("metadata")
                        .and_then(|metadata| metadata.get("url"))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| {
                    InvokeError::parse(
                        "Agnes video completed but response missing 'url' or 'metadata.url'",
                    )
                })?;
            Ok(AgnesVideoState::Done(url.to_owned()))
        }
        "failed" | "error" | "cancelled" | "canceled" => {
            let message = value
                .get("error")
                .and_then(|error| {
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| error.as_str())
                })
                .map(str::trim)
                .filter(|message| !message.is_empty())
                .unwrap_or("Agnes video generation failed");
            Ok(AgnesVideoState::Failed(message.to_owned()))
        }
        _ => Ok(AgnesVideoState::Pending),
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;

    fn input(bytes: &'static [u8]) -> InputAsset {
        InputAsset {
            id: None,
            role: "reference".into(),
            bytes: bytes.to_vec(),
            mime: "image/png".into(),
        }
    }

    fn image_call(server: &MockServer, request: TaskRequest) -> ResolvedCall {
        let mut call = call_with_endpoint(
            &format!("{}/v1", server.uri()),
            IMAGE_MODEL,
            IMAGE_ADAPTER_ID,
            "/images/generations",
            request,
        );
        call.platform = "agnes".into();
        call
    }

    fn video_call(server: &MockServer, request: TaskRequest) -> ResolvedCall {
        let mut call = call_with_endpoint(
            &format!("{}/v1", server.uri()),
            VIDEO_MODEL,
            VIDEO_ADAPTER_ID,
            "/videos",
            request,
        );
        call.platform = "agnes".into();
        call.model_params["poll_endpoint"] = Value::String(format!(
            "{}/agnesapi?video_id={{id}}",
            server.uri()
        ));
        call
    }

    #[tokio::test]
    async fn text_to_image_omits_quality_and_top_level_response_format() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": IMAGE_MODEL,
                "prompt": "a fox",
                "n": 1,
                "size": "1024x768",
                "extra_body": {"response_format": "url"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"url": "https://cdn.example/fox.png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageGeneration(crate::types::ImageGenRequest {
            prompt: "a fox".into(),
            count: 1,
            size: Some("1024x768".into()),
            quality: Some("auto".into()),
            extra: json!({}),
        });
        let call = image_call(&server, request);
        let outcome = AgnesImagesAdapter
            .submit(&reqwest::Client::new(), &call)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            TaskOutcome::Done(TaskResult::Assets(ref assets))
                if matches!(&assets[0].data, ProducedData::Url(url) if url.ends_with("fox.png"))
        ));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("quality").is_none());
        assert!(body.get("response_format").is_none());
    }

    #[tokio::test]
    async fn image_edit_uses_generations_json_and_extra_body_data_uri() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({
                "model": IMAGE_MODEL,
                "prompt": "make it blue",
                "size": DEFAULT_IMAGE_SIZE,
                "extra_body": {
                    "image": ["data:image/png;base64,aGk="],
                    "response_format": "b64_json"
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"b64_json": "aGk=", "mime_type": "image/png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "make it blue".into(),
            count: 1,
            size: None,
            inputs: vec![input(b"hi")],
            extra: json!({}),
        });
        let outcome = AgnesImagesAdapter
            .submit(&reqwest::Client::new(), &image_call(&server, request))
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            TaskOutcome::Done(TaskResult::Assets(ref assets))
                if matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hi")
        ));
    }

    #[tokio::test]
    async fn video_submit_is_json_and_prefers_video_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": VIDEO_MODEL,
                "prompt": "ocean waves",
                "width": 1280,
                "height": 720,
                "num_frames": 121,
                "frame_rate": 24,
                "image": "data:image/png;base64,aGk="
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "task_1",
                "video_id": "video_1",
                "status": "queued"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "ocean waves".into(),
            seconds: Some(5),
            size: Some("1280x720".into()),
            inputs: vec![input(b"hi")],
            extra: json!({}),
        });
        let outcome = AgnesVideoJobsAdapter
            .submit(&reqwest::Client::new(), &video_call(&server, request))
            .await
            .unwrap();
        let TaskOutcome::Pending(job) = outcome else {
            panic!("expected pending Agnes video job")
        };
        assert_eq!(job.remote_id, "video_1");
        assert_eq!(job.adapter_id, VIDEO_ADAPTER_ID);
    }

    #[tokio::test]
    async fn video_poll_adds_model_name_and_returns_completed_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/agnesapi"))
            .and(query_param("video_id", "video_1"))
            .and(query_param("model_name", VIDEO_MODEL))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "video_id": "video_1",
                "status": "completed",
                "progress": 100,
                "url": "https://cdn.example/video.mp4"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "ocean waves".into(),
            seconds: Some(5),
            size: None,
            inputs: vec![],
            extra: json!({}),
        });
        let call = video_call(&server, request);
        let job = JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
            config_revision: call.config_revision,
            remote_id: "video_1".into(),
            poll_state: json!({}),
        };
        let outcome = AgnesVideoJobsAdapter
            .poll(&reqwest::Client::new(), &call, &job)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            TaskOutcome::Done(TaskResult::Assets(ref assets))
                if matches!(&assets[0].data, ProducedData::Url(url) if url.ends_with("video.mp4"))
        ));
    }

    #[test]
    fn video_frame_contract_rounds_to_8n_plus_1_and_enforces_the_limit() {
        assert_eq!(frames_for_seconds(5, 24).unwrap(), 121);
        assert_eq!(frames_for_seconds(10, 24).unwrap(), 241);
        assert_eq!(frames_for_seconds(15, 24).unwrap(), 361);
        assert!(frames_for_seconds(20, 24).is_err());
        assert!(validate_num_frames(120).is_err());
        assert_eq!(validate_num_frames(441).unwrap(), 441);
    }

    #[test]
    fn video_status_handles_pending_failure_and_newer_metadata_url() {
        assert_eq!(
            parse_video_status(&json!({"status": "in_progress", "progress": 45})).unwrap(),
            AgnesVideoState::Pending
        );
        assert_eq!(
            parse_video_status(&json!({"status": "failed", "error": {"message": "blocked"}})).unwrap(),
            AgnesVideoState::Failed("blocked".into())
        );
        assert_eq!(
            parse_video_status(&json!({"status": "completed", "metadata": {"url": "https://cdn/v.mp4"}})).unwrap(),
            AgnesVideoState::Done("https://cdn/v.mp4".into())
        );
        assert!(parse_video_status(&json!({"status": "completed"})).is_err());
    }
}
