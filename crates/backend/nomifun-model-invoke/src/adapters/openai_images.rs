//! `openai.images` — OpenAI-compatible synchronous image generation/editing
//! (ported from `nomifun-creation/src/adapters/openai_images.rs`).
//!
//! - [`crate::types::TaskRequest::ImageGeneration`] → `POST` the dispatch
//!   target (conventionally `{base}/v1/images/generations`, JSON body).
//! - [`crate::types::TaskRequest::ImageEdit`] → `POST` the dispatch target
//!   (conventionally `{base}/v1/images/edits`, multipart: `image`(s) +
//!   optional `mask` + prompt/n/size).
//!
//! Both are synchronous — [`TaskOutcome::Done`] carries the artifacts inline.
//! `response_format=b64_json` is requested; the parser also tolerates providers
//! that return a `url` instead (the caller fetches it).

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_b64, error_from_response, post_json, post_multipart};
use crate::types::{
    ImageEditRequest, ImageGenRequest, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult,
};

use super::{json_request_body, scalar_request_fields};

/// Generous per-call ceiling: image generation is often multi-second.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// OpenAI-compatible sync `/images/{generations,edits}` protocol.
pub struct OpenAiImagesAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAiImagesAdapter {
    fn id(&self) -> &'static str {
        "openai.images"
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        match &call.request {
            TaskRequest::ImageGeneration(req) => submit_generations(http, call, req).await,
            TaskRequest::ImageEdit(req) => submit_edits(http, call, req).await,
            other => Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.images cannot serve task {:?}", other.task()),
            )),
        }
    }
}

async fn submit_generations(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageGenRequest,
) -> Result<TaskOutcome, InvokeError> {
    let url = call.endpoint_url()?;
    let mut body = json!({
        "model": call.model,
        "prompt": req.prompt,
        "n": req.count,
        "response_format": "b64_json",
    });
    if let Some(size) = &req.size {
        body["size"] = Value::String(size.clone());
    }
    if let Some(quality) = &req.quality {
        body["quality"] = Value::String(quality.clone());
    }
    let body = json_request_body(&call.model_params, &req.extra, body)?;

    let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| InvokeError::response_json("invalid images JSON", &e))?;
    Ok(TaskOutcome::Done(TaskResult::Assets(parse_images_response(&value)?)))
}

async fn submit_edits(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageEditRequest,
) -> Result<TaskOutcome, InvokeError> {
    let url = call.endpoint_url()?;

    let images: Vec<_> = req.inputs.iter().filter(|i| i.role != "mask").collect();
    if images.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "images/edits requires at least one input image (role != 'mask')",
        ));
    }
    // Single image → `image`; multiple → `image[]` (gpt-image-1 multi-ref).
    let image_field = if images.len() == 1 { "image" } else { "image[]" };

    let mut text_fields = scalar_request_fields(&call.model_params, &req.extra)?;
    for binary_field in ["image", "image[]", "mask"] {
        text_fields.remove(binary_field);
    }
    text_fields.insert("model".into(), call.model.clone());
    text_fields.insert("prompt".into(), req.prompt.clone());
    text_fields.insert("n".into(), req.count.to_string());
    if let Some(size) = &req.size {
        text_fields.insert("size".into(), size.clone());
    }

    // Built per attempt: multipart forms cannot be cloned, and rotation may
    // need to resend.
    let build_form = || -> Result<Form, InvokeError> {
        let mut form = Form::new();
        for (key, value) in &text_fields {
            form = form.text(key.clone(), value.clone());
        }
        for (idx, input) in images.iter().enumerate() {
            let part = Part::bytes(input.bytes.clone())
                .file_name(format!("image_{idx}.{}", ext_for_mime(&input.mime)))
                .mime_str(&input.mime)
                .map_err(|e| InvokeError::new(InvokeErrorKind::InvalidParams, format!("invalid image mime: {e}")))?;
            form = form.part(image_field, part);
        }
        if let Some(mask) = req.inputs.iter().find(|i| i.role == "mask") {
            let part = Part::bytes(mask.bytes.clone())
                .file_name("mask.png")
                .mime_str(&mask.mime)
                .map_err(|e| InvokeError::new(InvokeErrorKind::InvalidParams, format!("invalid mask mime: {e}")))?;
            form = form.part("mask", part);
        }
        Ok(form)
    };

    let resp = post_multipart(http, &url, REQUEST_TIMEOUT, &call.connection.auth, build_form).await?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| InvokeError::response_json("invalid images JSON", &e))?;
    Ok(TaskOutcome::Done(TaskResult::Assets(parse_images_response(&value)?)))
}

/// Parse an OpenAI images response body (`{ data: [ { b64_json?, url? } ] }`)
/// into artifacts, preferring inline base64 over a URL. Pure — unit tested with
/// fixtures.
pub(crate) fn parse_images_response(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("images response missing 'data' array"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("images response 'data' array is empty"));
    }
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
            let bytes =
                decode_b64(b64).ok_or_else(|| InvokeError::parse("images b64_json is not valid base64"))?;
            out.push(ProducedAsset { data: ProducedData::Bytes(bytes), mime: Some("image/png".into()) });
        } else if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
            out.push(ProducedAsset { data: ProducedData::Url(url.to_string()), mime: None });
        } else {
            return Err(InvokeError::parse("images data item has neither b64_json nor url"));
        }
    }
    Ok(out)
}

fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::InputAsset;

    fn image_call(base_url: &str, model: &str, endpoint: &str, request: TaskRequest) -> ResolvedCall {
        let base_url = format!("{}/v1", base_url.trim_end_matches('/'));
        call_with_endpoint(&base_url, model, "openai.images", endpoint, request)
    }

    fn generation_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        image_call(base_url, model, "/images/generations", request)
    }

    fn edit_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        image_call(base_url, model, "/images/edits", request)
    }

    // -- ported pure-parser fixtures ---------------------------------------

    #[test]
    fn parse_b64_response() {
        // "aGk=" is base64("hi").
        let v = json!({"data": [{"b64_json": "aGk="}]});
        let out = parse_images_response(&v).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0].data {
            ProducedData::Bytes(b) => assert_eq!(b, b"hi"),
            _ => panic!("expected bytes"),
        }
        assert_eq!(out[0].mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn parse_url_response() {
        let v = json!({"data": [{"url": "https://cdn/x.png"}, {"url": "https://cdn/y.png"}]});
        let out = parse_images_response(&v).unwrap();
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0].data, ProducedData::Url(u) if u == "https://cdn/x.png"));
    }

    #[test]
    fn parse_errors_on_empty_or_missing() {
        for bad in [
            json!({}),
            json!({"data": []}),
            json!({"data": [{}]}),
            json!({"data": [{"b64_json": "!!!not base64!!!"}]}),
        ] {
            let err = parse_images_response(&bad).unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::ParseError, "input {bad}");
        }
    }

    #[test]
    fn ext_mapping() {
        assert_eq!(ext_for_mime("image/jpeg"), "jpg");
        assert_eq!(ext_for_mime("image/webp"), "webp");
        assert_eq!(ext_for_mime("image/gif"), "gif");
        assert_eq!(ext_for_mime("image/png"), "png");
        assert_eq!(ext_for_mime("application/octet-stream"), "png");
    }

    // -- wiremock request/response tests ------------------------------------

    fn gen_request(size: Option<&str>, quality: Option<&str>) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a fox".into(),
            count: 2,
            size: size.map(str::to_string),
            quality: quality.map(str::to_string),
            extra: json!({}),
        })
    }

    fn image_input(role: &str, bytes: &[u8], mime: &str) -> InputAsset {
        InputAsset { id: None, role: role.into(), bytes: bytes.to_vec(), mime: mime.into() }
    }

    #[tokio::test]
    async fn generations_posts_typed_json_body_and_decodes_b64() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "gpt-image-1",
                "prompt": "a fox",
                "n": 2,
                "response_format": "b64_json",
                "size": "512x512",
                "quality": "high",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let call = generation_call(&server.uri(), "gpt-image-1", gen_request(Some("512x512"), Some("high")));
        let out = OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));
        assert_eq!(assets[0].mime.as_deref(), Some("image/png"));
    }

    #[tokio::test]
    async fn generations_omits_absent_size_and_quality() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .mount(&server)
            .await;

        let call = generation_call(&server.uri(), "gpt-image-1", gen_request(None, None));
        OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("size").is_none());
        assert!(body.get("quality").is_none());
    }

    #[tokio::test]
    async fn edits_multi_image_uses_bracketed_field_name_and_mask() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_string_contains("name=\"image[]\""))
            .and(body_string_contains("name=\"mask\""))
            .and(body_string_contains("name=\"model\""))
            .and(body_string_contains("name=\"prompt\""))
            .and(body_string_contains("name=\"size\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "add a hat".into(),
            count: 1,
            size: Some("1024x1024".into()),
            inputs: vec![
                image_input("image", b"img-a", "image/png"),
                image_input("image", b"img-b", "image/jpeg"),
                image_input("mask", b"mask-bytes", "image/png"),
            ],
            extra: json!({}),
        });
        let call = edit_call(&server.uri(), "gpt-image-1", request);
        let out = OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 1));
    }

    #[tokio::test]
    async fn edits_single_image_uses_plain_field_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(body_string_contains("name=\"image\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "p".into(),
            count: 1,
            size: None,
            inputs: vec![image_input("image", b"img-a", "image/png")],
            extra: json!({}),
        });
        let call = edit_call(&server.uri(), "gpt-image-1", request);
        OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(!body.contains("name=\"image[]\""), "single image must not use the bracketed field");
    }

    #[tokio::test]
    async fn edits_without_input_image_is_invalid_params() {
        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "p".into(),
            count: 1,
            size: None,
            inputs: vec![image_input("mask", b"mask-bytes", "image/png")],
            extra: json!({}),
        });
        let call = edit_call("http://127.0.0.1:9", "gpt-image-1", request);
        let err = OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = generation_call(&server.uri(), "gpt-image-1", gen_request(None, None));
        let err = OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("bad key"), "message: {}", err.message);
    }
}
