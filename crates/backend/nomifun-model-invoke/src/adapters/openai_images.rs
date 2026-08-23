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
//! `response_format=b64_json` is requested from every model that accepts it (see
//! [`rejects_response_format`]); the parser also tolerates providers that return
//! a `url` instead (the caller fetches it).

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    ImageResponseBudget, error_from_response, inline_image_response_body_limit, post_json,
    post_multipart, read_json_capped, validate_image_request_count,
};
#[cfg(test)]
use crate::transport::MAX_IMAGE_RESPONSE_IMAGES;
use crate::types::{
    ImageEditRequest, ImageGenRequest, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult,
};

use super::{json_request_body, scalar_request_fields};

/// Generous per-call ceiling: image generation is often multi-second.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Does this model reject `response_format` outright?
///
/// OpenAI's GPT image family always returns base64 and refuses the parameter
/// with `400 unknown_parameter`, so sending it made every generation on those
/// models fail — including the health probe, which reuses
/// [`submit_generations`] and therefore reported the model permanently
/// unhealthy. `dall-e-*` is the opposite: it defaults to a URL and needs the
/// parameter to return b64.
///
/// This is an explicit opt-out for the one family that breaks, NOT a blanket
/// removal: `openai.images` also serves OpenAI-*compatible* gateways
/// (SiliconFlow and friends) that expect `response_format`, and dropping it for
/// everyone would regress them.
///
/// Aggregators prefix ids (`openai/gpt-image-1`), so match on the last segment.
fn rejects_response_format(model: &str) -> bool {
    let leaf = model.rsplit('/').next().unwrap_or(model).to_ascii_lowercase();
    leaf.starts_with("gpt-image") || leaf.starts_with("chatgpt-image")
}

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
    let expected_images = validate_image_request_count(req.count)?;
    let url = call.endpoint_url()?;
    let mut body = json!({
        "model": call.model,
        "prompt": req.prompt,
        "n": req.count,
    });
    if !rejects_response_format(&call.model) {
        body["response_format"] = Value::String("b64_json".to_owned());
    }
    if let Some(size) = &req.size {
        body["size"] = Value::String(size.clone());
    }
    if let Some(quality) = &req.quality {
        body["quality"] = Value::String(quality.clone());
    }
    // SD-style OpenAI-compatible gateways (e.g. SiliconFlow) require/accept
    // these generation knobs in the JSON body; whitelisted passthrough from
    // `extra` mirrors the legacy prober's minimal_json_body fidelity.
    for key in ["steps", "cfg_scale", "text_mode"] {
        if let Some(v) = req.extra.get(key) {
            body[key] = v.clone();
        }
    }
    if let Some(seed) = req.extra.get("seed").filter(|seed| !seed.is_null()) {
        let seed = seed
            .as_u64()
            .filter(|seed| *seed <= u64::from(u32::MAX))
            .ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::InvalidParams,
                    "openai.images seed must be an integer from 0 to 4294967295",
                )
            })?;
        body["seed"] = Value::from(seed);
    }
    let body = json_request_body(&call.model_params, &req.extra, body)?;

    let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    let value: Value = read_json_capped(
        resp,
        inline_image_response_body_limit(expected_images),
        "images",
    )
    .await?;
    Ok(TaskOutcome::Done(TaskResult::Assets(
        parse_images_response_limited(&value, expected_images)?,
    )))
}

async fn submit_edits(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageEditRequest,
) -> Result<TaskOutcome, InvokeError> {
    let expected_images = validate_image_request_count(req.count)?;
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
    let value: Value = read_json_capped(
        resp,
        inline_image_response_body_limit(expected_images),
        "images",
    )
    .await?;
    Ok(TaskOutcome::Done(TaskResult::Assets(
        parse_images_response_limited(&value, expected_images)?,
    )))
}

/// Parse an OpenAI images response body (`{ data: [ { b64_json?, url? } ] }`)
/// into artifacts, preferring inline base64 over a URL. Pure — unit tested with
/// fixtures.
#[cfg(test)]
pub(crate) fn parse_images_response(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    parse_images_response_limited(value, MAX_IMAGE_RESPONSE_IMAGES)
}

pub(crate) fn parse_images_response_limited(
    value: &Value,
    max_images: usize,
) -> Result<Vec<ProducedAsset>, InvokeError> {
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("images response missing 'data' array"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("images response 'data' array is empty"));
    }
    let mut budget = ImageResponseBudget::new(max_images)?;
    // Reject an oversized batch before decoding even its first inline member.
    budget.ensure_additional_count(data.len(), "images response")?;
    let mut out = Vec::with_capacity(data.len());
    for (index, item) in data.iter().enumerate() {
        if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
            let bytes = budget.decode_base64(b64, &format!("images data[{index}].b64_json"))?;
            out.push(ProducedAsset {
                data: ProducedData::Bytes(bytes),
                // OpenAI-compatible providers may honor output_format=jpeg or
                // webp. Never invent PNG from the transport field name: carry
                // a real MIME declaration when supplied and otherwise let the
                // downstream verified artifact path sniff the bytes.
                mime: response_image_mime(item),
            });
        } else if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
            budget.accept_url("images response")?;
            out.push(ProducedAsset { data: ProducedData::Url(url.to_string()), mime: None });
        } else {
            return Err(InvokeError::parse("images data item has neither b64_json nor url"));
        }
    }
    Ok(out)
}

fn response_image_mime(item: &Value) -> Option<String> {
    ["mime_type", "mimeType", "content_type", "contentType", "mime"]
        .into_iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .map(str::to_string)
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
    fn image_contract_response_limits_preserve_real_mime_and_leave_unknown_for_sniffing() {
        // "aGk=" is base64("hi").
        let v = json!({"data": [{"b64_json": "aGk="}]});
        let out = parse_images_response(&v).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0].data {
            ProducedData::Bytes(b) => assert_eq!(b, b"hi"),
            _ => panic!("expected bytes"),
        }
        assert_eq!(out[0].mime, None, "unknown output formats must be sniffed downstream");

        let typed = parse_images_response(&json!({
            "data": [{"b64_json": "aGk=", "mime_type": "image/webp"}]
        }))
        .unwrap();
        assert_eq!(typed[0].mime.as_deref(), Some("image/webp"));
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
    fn image_contract_response_limits_reject_too_many_items_before_decoding_inline_data() {
        let value = json!({
            "data": (0..=MAX_IMAGE_RESPONSE_IMAGES)
                .map(|_| json!({"b64_json": "not valid base64"}))
                .collect::<Vec<_>>()
        });
        let error = parse_images_response(&value).unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("returned 9 images"), "{}", error.message);
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

    #[test]
    fn only_the_gpt_image_family_rejects_response_format() {
        for model in ["gpt-image-1", "gpt-image-2", "GPT-Image-1-mini", "openai/gpt-image-1", "chatgpt-image-latest"] {
            assert!(rejects_response_format(model), "{model} must not be sent response_format");
        }
        // dall-e needs it to return b64 instead of a URL, and the OpenAI-compatible
        // gateways on this adapter expect it too.
        for model in ["dall-e-2", "dall-e-3", "Kwai-Kolors/Kolors", "stabilityai/sd3", "flux-pro"] {
            assert!(!rejects_response_format(model), "{model} still needs response_format");
        }
    }

    #[tokio::test]
    async fn image_contract_omits_response_format_for_gpt_image_models() {
        // Regression: the adapter hardcoded `response_format: "b64_json"`, which
        // the GPT image family answers with 400 `unknown_parameter`. That made
        // every generation on OpenAI's current image models fail on the default
        // configuration, and — because the health probe shares this path — pinned
        // them to "unhealthy" in settings with no way for a user to override it.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let call = generation_call(&server.uri(), "gpt-image-1", gen_request(Some("1024x1024"), None));
        OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(
            body.get("response_format").is_none(),
            "gpt-image rejects response_format; body was {body}"
        );
        assert_eq!(body["model"], "gpt-image-1");
        assert_eq!(body["size"], "1024x1024");
    }

    #[tokio::test]
    async fn image_contract_openai_posts_seed_and_decodes_b64_without_invented_mime() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "dall-e-2",
                "prompt": "a fox",
                "n": 2,
                "response_format": "b64_json",
                "size": "512x512",
                "quality": "high",
                "seed": 42,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let mut request = gen_request(Some("512x512"), Some("high"));
        let TaskRequest::ImageGeneration(image_request) = &mut request else { unreachable!() };
        image_request.extra = json!({"seed": 42});
        let call = generation_call(&server.uri(), "dall-e-2", request);
        let out = OpenAiImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));
        assert_eq!(assets[0].mime, None, "the adapter must not invent PNG MIME");
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
