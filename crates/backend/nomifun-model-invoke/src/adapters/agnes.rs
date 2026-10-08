//! Agnes media protocols.
//!
//! Agnes image generation looks superficially OpenAI-compatible, but its
//! queue rejects OpenAI's top-level `quality` / `response_format` fields and
//! image editing is JSON on the generations endpoint. Agnes Video 2.5 is a
//! separate JSON async-job contract: submit returns a `video_id`, polling uses
//! `/agnesapi?video_id=...&model_name=...`, and the completed status body
//! carries the output URL directly.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::{AgnesModelContract, ModelTask, agnes_model_contract};
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
    TaskRequest, TaskResult,
};

use super::json_request_body;
use super::openai_images::parse_images_response_limited;

const IMAGE_ADAPTER_ID: &str = "agnes.images";
const VIDEO_ADAPTER_ID: &str = "agnes.video_jobs";
#[cfg(test)]
const IMAGE_MODEL: &str = "agnes-image-2.1-flash";
#[cfg(test)]
const VIDEO_MODEL: &str = "agnes-video-2.5-flash";
const DEFAULT_IMAGE_SIZE: &str = "1024x1024";
const MAX_INPUT_IMAGES: usize = 8;
const MAX_INPUT_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(360);
const VIDEO_SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const VIDEO_POLL_TIMEOUT: Duration = Duration::from_secs(60);
/// A conservative floor avoids bursts when several desktop jobs poll together.
const VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(10);
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
    if agnes_model_contract(model) == Some(AgnesModelContract::Image) {
        Ok(())
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!(
                "agnes.images supports Agnes Image 2.0, 2.1 and 2.5 Flash; got {model:?}"
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
    });
    if let Some(size) = size {
        typed["size"] = Value::String(size.to_owned());
    }
    if let Some(images) = images {
        typed["extra_body"] = json!({"response_format": "b64_json"});
        typed["extra_body"]["image"] = Value::Array(
            images.into_iter().map(Value::String).collect(),
        );
    } else {
        // Agnes documents `return_base64` as the text-to-image switch. Inline
        // output avoids a second, unauthenticated fetch of a short-lived URL
        // and cannot race provider-side object publication.
        typed["return_base64"] = Value::Bool(true);
    }

    let mut body = json_request_body(&call.model_params, extra, typed)?;
    let object = body.as_object_mut().ok_or_else(|| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "Agnes image request body must be an object",
        )
    })?;
    // Agnes requires a size. Supply the local fallback only when neither the
    // capability nor this request selected one; configured 2K–4K tiers must
    // survive a caller that leaves its optional typed size unset.
    object
        .entry("size")
        .or_insert_with(|| Value::String(DEFAULT_IMAGE_SIZE.into()));
    // Current image models publish exact pixel mappings for their resolution
    // tiers. Normalize only those mappings; legacy 2.0 pixel sizes remain valid.
    if !call.model.trim().eq_ignore_ascii_case("agnes-image-2.0-flash") {
        let selected = object.get("size").and_then(Value::as_str).unwrap_or_default();
        let ratios = [
            ("1:1", 1024, 1024), ("3:4", 864, 1152), ("4:3", 1152, 864),
            ("16:9", 1312, 736), ("9:16", 736, 1312), ("2:3", 832, 1248),
            ("3:2", 1248, 832), ("21:9", 1568, 672),
        ];
        let mapped = (1..=4).find_map(|tier| {
            ratios.iter().find_map(|(ratio, width, height)| {
                (selected == format!("{}x{}", width * tier, height * tier))
                    .then_some((tier, *ratio))
            })
        });
        if let Some((tier, ratio)) = mapped {
            object.insert("size".into(), json!(format!("{tier}K")));
            object.insert("ratio".into(), json!(ratio));
        }
    }
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

    fn recommended_poll_interval(&self) -> Option<Duration> {
        Some(VIDEO_POLL_INTERVAL)
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
        let body = video::build_video_body(&call.model, &call.model_params, request)?;
        let url = call.endpoint_url()?;
        let response = video::submit(http, &url, &call.connection.auth, &body).await?;
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
        video::validate_video_model(&call.model)?;
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

mod video;

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
                .or_else(|| value.get("video_url").and_then(Value::as_str))
                .or_else(|| {
                    value
                        .get("metadata")
                        .and_then(|metadata| {
                            metadata
                                .get("url")
                                .or_else(|| metadata.get("video_url"))
                        })
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| {
                    InvokeError::parse(
                        "Agnes video completed but response missing a result URL",
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
        "queued" | "pending" | "in_progress" | "processing" | "running" | "generating" => {
            Ok(AgnesVideoState::Pending)
        }
        _ => Err(InvokeError::parse("Agnes video response has no recognized status")),
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::VideoGenRequest;

    fn input(bytes: &'static [u8]) -> InputAsset {
        InputAsset {
            id: None,
            role: "reference".into(),
            bytes: bytes.to_vec(),
            mime: "image/png".into(),
        }
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
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

    #[test]
    fn image_size_defaults_preserve_configured_and_request_tiers() {
        for (configured, extra, typed_size, expected) in [
            (json!({}), json!({}), None, "1K"),
            (json!({"size": "4K"}), json!({}), None, "4K"),
            (json!({"size": "4K"}), json!({"size": "2K"}), None, "2K"),
            (json!({"size": "4K"}), json!({"size": "2K"}), Some("3K"), "3K"),
        ] {
            let mut call = call_with_endpoint(
                "https://unused.invalid/v1", IMAGE_MODEL, IMAGE_ADAPTER_ID,
                "/images/generations", TaskRequest::ImageGeneration(crate::types::ImageGenRequest {
                    prompt: "detailed scene".into(), count: 1,
                    size: typed_size.map(str::to_owned), quality: None, extra,
                }),
            );
            call.model_params = configured;
            let (body, _) = build_image_body(&call).unwrap();
            assert_eq!(body["size"], expected);
        }
    }

    #[test]
    fn current_image_pixel_choices_become_native_tiers_without_stale_ratio_defaults() {
        let request = TaskRequest::ImageGeneration(crate::types::ImageGenRequest {
            prompt: "scene".into(), count: 1, size: Some("2624x1472".into()), quality: None,
            extra: json!({"ratio": "1:1"}),
        });
        let mut call = call_with_endpoint("https://unused.invalid/v1", "agnes-image-2.5-flash",
            IMAGE_ADAPTER_ID, "/images/generations", request);
        call.model_params["ratio"] = json!("9:16");
        let (body, _) = build_image_body(&call).unwrap();
        assert_eq!(body["size"], "2K");
        assert_eq!(body["ratio"], "16:9");
    }
    #[tokio::test]
    async fn text_to_image_requests_inline_base64_without_openai_only_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": IMAGE_MODEL,
                "prompt": "a fox",
                "n": 1,
                "size": "1024x768",
                "return_base64": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"b64_json": "aGk="}]
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
            .submit(&http(), &call)
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            TaskOutcome::Done(TaskResult::Assets(ref assets))
                if matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hi")
        ));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("quality").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("extra_body").is_none());
    }

    #[tokio::test]
    async fn image_edit_uses_generations_json_and_extra_body_data_uri() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({
                "model": IMAGE_MODEL,
                "prompt": "make it blue",
                "size": "1K",
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
            quality: None,
            inputs: vec![input(b"hi")],
            extra: json!({}),
        });
        let outcome = AgnesImagesAdapter
            .submit(&http(), &image_call(&server, request))
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            TaskOutcome::Done(TaskResult::Assets(ref assets))
                if matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hi")
        ));
    }


    fn video_request() -> VideoGenRequest {
        VideoGenRequest { prompt: "ocean waves".into(), seconds: Some(5),
            size: Some("1280x720".into()), resolution: None, inputs: vec![], extra: json!({}) }
    }

    #[tokio::test]
    async fn every_documented_image_model_uses_the_same_generation_and_edit_contract() {
        let server = MockServer::start().await;
        for model in ["agnes-image-2.0-flash", "agnes-image-2.1-flash", "agnes-image-2.5-flash"] {
            Mock::given(method("POST")).and(path("/v1/images/generations"))
                .and(body_partial_json(json!({"model":model,
                    "size":if model == "agnes-image-2.0-flash" { "1024x1024" } else { "1K" }})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"b64_json":"aGk="}]})))
                .expect(2).mount(&server).await;
            for request in [
                TaskRequest::ImageGeneration(crate::types::ImageGenRequest {
                    prompt:"fox".into(), count:1, size:None, quality:None, extra:json!({}) }),
                TaskRequest::ImageEdit(ImageEditRequest {
                    prompt:"blue fox".into(), count:1, size:None, quality:None,
                    inputs:vec![input(b"hi")], extra:json!({}) }),
            ] {
                let mut call = image_call(&server, request);
                call.model = model.into();
                assert!(matches!(AgnesImagesAdapter.submit(&http(), &call).await.unwrap(), TaskOutcome::Done(_)));
            }
        }
        assert!(validate_image_model("agnes-image-future").is_err());
        assert!(validate_image_model("agnes-3.0-flash").is_err());
    }

    #[tokio::test]
    async fn current_video_models_submit_native_json_and_poll_with_video_id_and_model_name() {
        let server = MockServer::start().await;
        for model in ["agnes-video-2.5", "agnes-video-2.5-flash"] {
            Mock::given(method("POST")).and(path("/v1/videos"))
                .and(header("authorization", "Bearer sk-test"))
                .and(body_partial_json(json!({"model":model, "seconds":"5", "size":"720P",
                    "aspect_ratio":"16:9", "mode":"text", "n":1})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id":"task_different", "video_id":"video_1", "status":"queued"})))
                .expect(1).mount(&server).await;
            Mock::given(method("GET")).and(path("/agnesapi"))
                .and(query_param("video_id","video_1")).and(query_param("model_name",model))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "status":"completed", "internal_status":"pending", "progress":100,
                    "url":"https://cdn.example/video.mp4"})))
                .expect(1).mount(&server).await;
            let mut call = video_call(&server, TaskRequest::VideoGeneration(video_request()));
            call.model = model.into();
            let TaskOutcome::Pending(job) = AgnesVideoJobsAdapter.submit(&http(), &call).await.unwrap()
                else { panic!("pending job expected") };
            assert_eq!(job.remote_id, "video_1");
            assert!(matches!(AgnesVideoJobsAdapter.poll(&http(), &call, &job).await.unwrap(),
                TaskOutcome::Done(TaskResult::Assets(ref assets))
                    if matches!(&assets[0].data, ProducedData::Url(url) if url.ends_with("video.mp4"))));
        }
        for request in server.received_requests().await.unwrap() {
            if request.method.as_str() == "POST" {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                for key in ["width", "height", "num_frames", "frame_rate", "extra_body"] {
                    assert!(body.get(key).is_none(), "retired field {key}");
                }
            }
        }
    }

    #[test]
    fn video_preserves_native_defaults_and_typed_fields_take_precedence() {
        let mut request = video_request();
        request.seconds = None; request.size = None;
        let body = video::build_video_body("agnes-video-2.5", &json!({
            "size":"2K", "seconds":"8", "aspect_ratio":"9:16", "seed":1,
            "extra_body":{"custom":true}, "endpoint":"/must-not-leak"
        }), &request).unwrap();
        assert_eq!(body["size"], "2K"); assert_eq!(body["seconds"], "8");
        assert_eq!(body["aspect_ratio"], "9:16"); assert_eq!(body["custom"], true);
        assert!(body.get("endpoint").is_none()); assert!(body.get("extra_body").is_none());
        request.seconds=Some(4); request.size=Some("1920x1080".into());
        request.extra=json!({"seed":42});
        let body=video::build_video_body("agnes-video-2.5", &body, &request).unwrap();
        assert_eq!(body["seconds"], "4"); assert_eq!(body["size"], "1080P");
        assert_eq!(body["aspect_ratio"], "16:9"); assert_eq!(body["seed"], 42);
        request.size=None; request.resolution=Some("720p".into());
        assert_eq!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).unwrap()["size"], "720P");
    }

    #[test]
    fn video_reference_and_keyframe_modes_have_distinct_media_fields() {
        let mut request=video_request();
        request.inputs=vec![input(b"first"),input(b"second"),input(b"third")];
        request.extra=json!({"mode":"text", "first_frame":"stale", "extra_body":{"images":["stale"], "seed":42}});
        let body=video::build_video_body(VIDEO_MODEL, &json!({}), &request).unwrap();
        assert_eq!(body["mode"], "reference"); assert_eq!(body["images"].as_array().unwrap().len(), 3);
        assert_eq!(body["images"][0], "data:image/png;base64,Zmlyc3Q=");
        assert_eq!(body["seed"], 42); assert!(body.get("first_frame").is_none());
        request.inputs=vec![InputAsset{role:"last_frame".into(),..input(b"last")}];
        let body=video::build_video_body(VIDEO_MODEL, &json!({}), &request).unwrap();
        assert_eq!(body["mode"], "keyframe"); assert!(body.get("images").is_none());
        assert!(body.get("first_frame").is_none()); assert!(body.get("last_frame").is_some());
        request.inputs.insert(0, InputAsset{role:"first_frame".into(),..input(b"first")});
        assert!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).unwrap().get("first_frame").is_some());
        request.inputs.push(input(b"mixed"));
        assert!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).is_err());
    }

    #[test]
    fn video_flattens_each_extra_layer_before_applying_request_precedence() {
        let mut request = video_request();
        request.extra = json!({"extra_body": {
            "model": "cannot-shadow-model", "prompt": "cannot-shadow-prompt",
            "seed": 42, "endpoint": "/cannot-leak", "custom": {"request": true}
        }});
        let body = video::build_video_body(VIDEO_MODEL, &json!({
            "seed": 1, "extra_body": {"custom": {"configured": true}}
        }), &request).unwrap();
        assert_eq!(body["seed"], 42);
        assert_eq!(body["model"], VIDEO_MODEL);
        assert_eq!(body["prompt"], request.prompt);
        assert_eq!(body["custom"], json!({"configured": true, "request": true}));
        assert!(body.get("extra_body").is_none());
        assert!(body.get("endpoint").is_none());
    }

    #[test]
    fn video_enforces_model_specific_limits_and_rejects_retired_contracts_before_http() {
        let mut request=video_request();
        request.size=Some("1920x1080".into());
        assert!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).is_err());
        assert!(video::build_video_body("agnes-video-2.5", &json!({}), &request).is_ok());
        request.size=None;
        for seconds in [0,3,13,15] {
            request.seconds=Some(seconds);
            assert!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).is_err());
        }
        request.seconds=Some(5);
        request.inputs=vec![input(b"image");6];
        assert!(video::build_video_body(VIDEO_MODEL, &json!({}), &request).is_err());
        assert!(video::build_video_body("agnes-video-2.5", &json!({}), &request).is_ok());
        request.inputs.clear();
        for extra in [json!({"mode":"reference"}), json!({"size":"auto"}), json!({"n":2}),
            json!({"num_frames":121}), json!({"width":1280}), json!({"aspect_ratio":"2:3"}),
            json!({"videos":[{"url":"https://cdn.example/input.mp4"}],"mode":"reference"}),
            json!({"mode":"keyframe","first_frame":""})] {
            request.extra=extra;
            assert!(video::build_video_body(VIDEO_MODEL,&json!({}),&request).is_err());
        }
        let error=video::validate_video_model("agnes-video-v2.0").unwrap_err();
        assert!(error.message.contains("taken offline"));
        assert!(video::validate_video_model("agnes-video-future").is_err());
    }

    #[test]
    fn video_adapter_exposes_a_conservative_status_query_interval() {
        assert_eq!(
            AgnesVideoJobsAdapter.recommended_poll_interval(),
            Some(Duration::from_secs(10))
        );
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
        assert_eq!(
            parse_video_status(&json!({"status": "completed", "video_url": "https://cdn/legacy.mp4"})).unwrap(),
            AgnesVideoState::Done("https://cdn/legacy.mp4".into())
        );
        assert!(parse_video_status(&json!({"status": "completed"})).is_err());
        assert!(parse_video_status(&json!({"message": "not a job"})).is_err());
    }
}
