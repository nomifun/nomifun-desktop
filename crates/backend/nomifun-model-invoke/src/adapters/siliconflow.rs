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

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{encode_b64, error_from_response, post_json};
use crate::types::{
    ImageEditRequest, ImageGenRequest, JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest,
    TaskResult, VideoGenRequest,
};

use super::has_endpoint_override;

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
const VIDEO_ADAPTER_ID: &str = "siliconflow.video_jobs";

/// Build a native SiliconFlow `/v1` endpoint while accepting configured bases
/// with or without a trailing `/v1`.
fn siliconflow_v1_url(connection: &ResolvedConnection, path: &str) -> String {
    let base = connection.base_url.trim().trim_end_matches('/');
    if connection.is_full_url {
        return base.to_string();
    }
    let root = base.strip_suffix("/v1").unwrap_or(base);
    format!("{root}/v1{path}")
}

fn image_url(call: &ResolvedCall) -> String {
    if has_endpoint_override(&call.model_params) {
        call.dispatch_target().url
    } else {
        siliconflow_v1_url(&call.connection, "/images/generations")
    }
}

fn video_submit_url(call: &ResolvedCall) -> String {
    if has_endpoint_override(&call.model_params) {
        call.dispatch_target().url
    } else {
        siliconflow_v1_url(&call.connection, "/video/submit")
    }
}

fn video_status_url(call: &ResolvedCall) -> String {
    if let Some(endpoint) = call
        .model_params
        .get("poll_endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            return endpoint.to_string();
        }
        let base = call.connection.base_url.trim().trim_end_matches('/');
        if endpoint.starts_with('/') {
            if let Ok(mut parsed) = reqwest::Url::parse(base) {
                parsed.set_path(endpoint);
                parsed.set_query(None);
                parsed.set_fragment(None);
                return parsed.to_string().trim_end_matches('/').to_string();
            }
        }
        return format!("{base}/{}", endpoint.trim_start_matches('/'));
    }

    if has_endpoint_override(&call.model_params) || call.connection.is_full_url {
        let submit = video_submit_url(call);
        let no_query = submit.split('?').next().unwrap_or(submit.as_str()).trim_end_matches('/');
        if let Some(prefix) = no_query.strip_suffix("/submit") {
            return format!("{prefix}/status");
        }
    }
    siliconflow_v1_url(&call.connection, "/video/status")
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
            TaskRequest::ImageGeneration(req) => build_image_generation_body(call, req),
            TaskRequest::ImageEdit(req) => build_image_edit_body(call, req)?,
            other => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("siliconflow.images cannot serve task {:?}", other.task()),
                ));
            }
        };

        let resp = post_json(http, &image_url(call), SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::parse(format!("invalid SiliconFlow images JSON: {e}")))?;
        Ok(TaskOutcome::Done(TaskResult::Assets(parse_images(&value)?)))
    }
}

fn build_image_generation_body(call: &ResolvedCall, req: &ImageGenRequest) -> Value {
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
    Value::Object(body)
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
    Ok(Value::Object(body))
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
        let body = build_video_submit_body(call, req);
        let resp = post_json(http, &video_submit_url(call), SUBMIT_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::parse(format!("invalid SiliconFlow video submit JSON: {e}")))?;
        let request_id = value
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| InvokeError::parse("SiliconFlow video submit response missing 'requestId'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
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
        let resp = post_json(http, &video_status_url(call), POLL_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::parse(format!("invalid SiliconFlow video status JSON: {e}")))?;

        match parse_video_status(&value)? {
            SiliconFlowVideoState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
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

fn build_video_submit_body(call: &ResolvedCall, req: &VideoGenRequest) -> Value {
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
    Value::Object(body)
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
    use crate::adapters::test_support::call;
    use crate::types::InputAsset;

    fn siliconflow_call(base: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let mut call = call(base, model, request);
        call.platform = "siliconflow".into();
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
        JobHandle { adapter_id: VIDEO_ADAPTER_ID.into(), remote_id: id.into(), poll_state: json!({}) }
    }

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
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
        let mut call = siliconflow_call(&format!("{}/v1", server.uri()), "Kwai-Kolors/Kolors", request);
        call.model_params = json!({"num_inference_steps": 30, "guidance_scale": 7.5});
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
        let call = siliconflow_call(&server.uri(), "Qwen/Qwen-Image-Edit-2509", request);
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

        let call = siliconflow_call(&server.uri(), "Wan-AI/Wan2.2-I2V-A14B", video_request());
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

        let call = siliconflow_call(&server.uri(), "Wan-AI/Wan2.2-I2V-A14B", video_request());
        let http = test_http();
        let pending = SiliconFlowVideoJobsAdapter.poll(&http, &call, &job("req-1")).await.unwrap();
        let TaskOutcome::Pending(handle) = pending else { panic!("expected pending") };
        let done = SiliconFlowVideoJobsAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = done else { panic!("expected assets") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://cdn.test/video.mp4"));
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

        let call = siliconflow_call(&server.uri(), "Wan-AI/Wan2.2-T2V-A14B", video_request());
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
