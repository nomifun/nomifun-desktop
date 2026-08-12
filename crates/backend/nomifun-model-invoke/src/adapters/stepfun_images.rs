//! `stepfun.images` — StepFun's native synchronous image generation and edit
//! protocols, shared by the regular `/v1` API and `/step_plan/v1` gateway.
//!
//! Although the endpoint names resemble OpenAI Images, the typed contracts are
//! narrower: one output per request, and image edit accepts exactly one input
//! image. Open-ended provider fields pass through after local transport/auth
//! metadata is removed, while typed task fields win last. Keeping a dedicated
//! adapter prevents NomiFun routing or credential fields from leaking onto the
//! wire without freezing future StepFun request options.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::{Map, Value};

use super::provider_body_fields;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{
    decode_b64, error_from_response, post_json, post_multipart, read_body_capped,
    MAX_ARTIFACT_BYTES,
};
use crate::types::{
    ImageEditRequest, ImageGenRequest, ProducedAsset, ProducedData, TaskOutcome, TaskRequest,
    TaskResult,
};

pub const ADAPTER_ID: &str = "stepfun.images";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
/// Adapter-local capability metadata. It may describe UI/editor policy, but it
/// is never a provider request field and therefore must be stripped at every
/// passthrough layer.
const GENERATION_OPTION_KEYS_PARAM: &str = "generation_option_keys";

/// Native StepFun `/images/generations` + `/images/edits` adapter.
pub struct StepFunImagesAdapter;

#[async_trait]
impl ProtocolAdapter for StepFunImagesAdapter {
    fn id(&self) -> &'static str {
        ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        match (call.task, &call.request) {
            (ModelTask::ImageGeneration, TaskRequest::ImageGeneration(req)) => {
                submit_generation(http, call, req).await
            }
            (ModelTask::ImageEdit, TaskRequest::ImageEdit(req)) => {
                submit_edit(http, call, req).await
            }
            other => Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!(
                    "{ADAPTER_ID} cannot serve resolved task {:?} with request {:?}",
                    other.0,
                    other.1.task()
                ),
            )),
        }
    }
}

async fn submit_generation(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageGenRequest,
) -> Result<TaskOutcome, InvokeError> {
    validate_single_output(req.count)?;
    let url = call.endpoint_url()?;

    let mut body = Map::new();
    merge_provider_options(&mut body, &call.model_params, &req.extra);

    // Typed task fields win over configured/request defaults. `quality` is
    // deliberately ignored: it is not part of StepFun's typed image contract,
    // while an explicitly provider-native field with that name may still be
    // supplied through `provider_params`/`extra`.
    if body.contains_key("n") {
        body.insert("n".into(), Value::from(req.count));
    }
    if let Some(size) = req.size.as_deref().map(str::trim).filter(|size| !size.is_empty()) {
        body.insert("size".into(), Value::String(size.to_owned()));
    }
    body.insert("model".into(), Value::String(call.model.clone()));
    body.insert("prompt".into(), Value::String(req.prompt.clone()));

    let response = post_json(
        http,
        &url,
        REQUEST_TIMEOUT,
        &call.connection.auth,
        &Value::Object(body),
    )
    .await?;
    parse_http_response(response).await
}

async fn submit_edit(
    http: &reqwest::Client,
    call: &ResolvedCall,
    req: &ImageEditRequest,
) -> Result<TaskOutcome, InvokeError> {
    validate_single_output(req.count)?;
    let image = validate_edit_inputs(req)?;
    let url = call.endpoint_url()?;

    let mut options = Map::new();
    merge_provider_options(&mut options, &call.model_params, &req.extra);
    // Multipart fields cannot be overwritten by appending a duplicate field,
    // so remove provider values owned by the typed task before building it.
    for typed_field in ["model", "prompt", "image", "mask"] {
        options.remove(typed_field);
    }
    if options.contains_key("n") {
        options.insert("n".into(), Value::from(req.count));
    }
    if let Some(size) = req.size.as_deref().map(str::trim).filter(|size| !size.is_empty()) {
        options.insert("size".into(), Value::String(size.to_owned()));
    }

    let build_form = || -> Result<Form, InvokeError> {
        let part = Part::bytes(image.bytes.clone())
            .file_name(format!("image.{}", extension_for_mime(&image.mime)))
            .mime_str(&image.mime)
            .map_err(|error| {
                InvokeError::new(
                    InvokeErrorKind::InvalidParams,
                    format!("invalid StepFun edit image MIME: {error}"),
                )
            })?;
        let mut form = Form::new()
            .text("model", call.model.clone())
            .text("prompt", req.prompt.clone())
            .part("image", part);
        for (key, value) in &options {
            form = form.text(key.clone(), multipart_value(value)?);
        }
        Ok(form)
    };

    let response = post_multipart(
        http,
        &url,
        REQUEST_TIMEOUT,
        &call.connection.auth,
        build_form,
    )
    .await?;
    parse_http_response(response).await
}

/// Merge provider-native fields in this order:
/// configured flat values -> per-call flat values. [`provider_body_fields`]
/// excludes local URL/auth metadata. Provider fields otherwise stay
/// open-ended so newly documented StepFun options do not require a release.
fn merge_provider_options(target: &mut Map<String, Value>, configured: &Value, extra: &Value) {
    for source in [configured, extra] {
        merge_provider_source(target, source);
    }
}

fn merge_provider_source(target: &mut Map<String, Value>, source: &Value) {
    for (key, value) in provider_body_fields(source) {
        if key != GENERATION_OPTION_KEYS_PARAM {
            target.insert(key.clone(), value.clone());
        }
    }
}

fn validate_single_output(count: u32) -> Result<(), InvokeError> {
    if count == 1 {
        Ok(())
    } else {
        Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("StepFun image APIs currently support exactly one output, got count={count}"),
        ))
    }
}

fn validate_edit_inputs(req: &ImageEditRequest) -> Result<&crate::types::InputAsset, InvokeError> {
    if req.inputs.iter().any(|input| input.role == "mask") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "StepFun images/edits does not support a mask input",
        ));
    }
    let images = req.inputs.iter().collect::<Vec<_>>();
    if images.len() != 1 {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("StepFun images/edits requires exactly one input image, got {}", images.len()),
        ));
    }
    let image = images[0];
    if image.bytes.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "StepFun images/edits input image is empty",
        ));
    }
    if !image.mime.to_ascii_lowercase().starts_with("image/") {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("StepFun images/edits requires an image MIME, got {:?}", image.mime),
        ));
    }
    Ok(image)
}

fn multipart_value(value: &Value) -> Result<String, InvokeError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "StepFun image edit option values must be strings, numbers, or booleans",
        )),
    }
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

async fn parse_http_response(response: reqwest::Response) -> Result<TaskOutcome, InvokeError> {
    if !response.status().is_success() {
        return Err(error_from_response(response).await);
    }
    let bytes = read_body_capped(response, MAX_ARTIFACT_BYTES).await?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| InvokeError::parse(format!("invalid StepFun images JSON: {error}")))?;
    Ok(TaskOutcome::Done(TaskResult::Assets(parse_images_response(&value)?)))
}

fn parse_images_response(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    if let Some(error) = value.get("error") {
        return Err(classify_body_error(error));
    }
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("StepFun images response missing data array"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("StepFun images response data array is empty"));
    }

    let mut assets = Vec::with_capacity(data.len());
    for item in data {
        if item.get("finish_reason").and_then(Value::as_str) == Some("content_filtered") {
            return Err(InvokeError::new(
                InvokeErrorKind::ContentPolicy,
                "StepFun image request was stopped by content filtering",
            ));
        }
        if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
            let bytes = decode_b64(encoded)
                .filter(|bytes| !bytes.is_empty())
                .ok_or_else(|| InvokeError::parse("StepFun images b64_json is not valid Base64"))?;
            assets.push(ProducedAsset {
                data: ProducedData::Bytes(bytes),
                mime: Some("image/png".into()),
            });
        } else if let Some(url) = item
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            assets.push(ProducedAsset { data: ProducedData::Url(url.to_owned()), mime: None });
        } else {
            return Err(InvokeError::parse(
                "StepFun images data item contains neither b64_json nor url",
            ));
        }
    }
    Ok(assets)
}

fn classify_body_error(error: &Value) -> InvokeError {
    let code = error
        .get("code")
        .map(|code| code.as_str().map(str::to_owned).unwrap_or_else(|| code.to_string()))
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("provider returned an error object");
    let signal = format!("{code} {message}").to_ascii_lowercase();
    let kind = if signal.contains("quota") || signal.contains("insufficient_balance") {
        InvokeErrorKind::QuotaExhausted
    } else if signal.contains("rate") && signal.contains("limit") {
        InvokeErrorKind::RateLimited
    } else if signal.contains("auth") || signal.contains("api_key") || signal.contains("api key") {
        InvokeErrorKind::Auth
    } else if signal.contains("content") && (signal.contains("filter") || signal.contains("policy")) {
        InvokeErrorKind::ContentPolicy
    } else if signal.contains("invalid") || signal.contains("parameter") || signal.contains("argument") {
        InvokeErrorKind::InvalidParams
    } else {
        InvokeErrorKind::ProviderError
    };
    InvokeError::new(kind, format!("StepFun images: {message}"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::InputAsset;

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn generation(count: u32, extra: Value) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "采菊东篱下".into(),
            count,
            size: Some("1024x1024".into()),
            // This OpenAI-only field must never reach StepFun.
            quality: Some("hd".into()),
            extra,
        })
    }

    fn edit(inputs: Vec<InputAsset>, extra: Value) -> TaskRequest {
        TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "让角色骑自行车".into(),
            count: 1,
            size: None,
            inputs,
            extra,
        })
    }

    fn input(role: &str, bytes: &[u8], mime: &str) -> InputAsset {
        InputAsset { id: None, role: role.into(), bytes: bytes.to_vec(), mime: mime.into() }
    }

    fn stepfun_call_with_endpoint(
        base: &str,
        platform: &str,
        model: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let mut call = call_with_endpoint(base, model, "stepfun.images", endpoint, request);
        call.platform = platform.into();
        call
    }

    fn generation_call(base: &str, platform: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        stepfun_call_with_endpoint(base, platform, model, "/images/generations", request)
    }

    fn edit_call(base: &str, platform: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        stepfun_call_with_endpoint(base, platform, model, "/images/edits", request)
    }

    #[test]
    fn adapter_supports_only_image_generation_and_edit() {
        assert!(StepFunImagesAdapter.supports(ModelTask::ImageGeneration));
        assert!(StepFunImagesAdapter.supports(ModelTask::ImageEdit));
        assert!(!StepFunImagesAdapter.supports(ModelTask::Chat));
    }

    #[tokio::test]
    async fn generation_passes_open_provider_fields_and_decodes_b64() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "user-configured-step-image",
                "prompt": "采菊东篱下",
                "size": "1024x1024",
                "response_format": "b64_json",
                "seed": 7,
                "steps": 8,
                "cfg_scale": 1.0,
                "negative_prompt": "模糊",
                "text_mode": true,
                "quality": "ultra",
                "future_provider_option": {"mode": "v2"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"b64_json": "aGk=", "finish_reason": "success", "seed": 7}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = generation_call(
            &format!("{}/v1", server.uri()),
            "stepfun",
            "user-configured-step-image",
            generation(
                1,
                json!({
                    "response_format": "b64_json",
                    "seed": 7,
                    "steps": 8,
                    "cfg_scale": 1.0,
                    "negative_prompt": "模糊",
                    "text_mode": true,
                    "quality": "ultra",
                    "future_provider_option": {"mode": "v2"},
                    "endpoint": "must-not-leak",
                    "api_key": "must-not-leak"
                }),
            ),
        );
        call.model_params = json!({
            "endpoint": "/images/generations",
            "base_url": "https://must-not-leak.example",
            "generation_option_keys": ["negative_prompt", "text_mode"],
            "steps": 50,
            "headers": {"secret": "x"}
        });
        let outcome = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected image assets") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(bytes) if bytes == b"hi"));

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        for forbidden in ["n", "endpoint", "api_key", "base_url", "headers", "generation_option_keys"] {
            assert!(body.get(forbidden).is_none(), "transport/OpenAI field leaked: {forbidden} in {body}");
        }
        assert_eq!(body["steps"], 8);
        assert_eq!(body["quality"], "ultra");
        assert_eq!(body["future_provider_option"], json!({"mode": "v2"}));
    }

    #[tokio::test]
    async fn model_id_does_not_change_open_provider_field_passthrough() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({
                "model": "step-image-edit-2",
                "seed": 17,
                "steps": 30,
                "cfg_scale": 7.5,
                "negative_prompt": "not selected by model id",
                "text_mode": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"url": "https://res.stepfun.com/image.png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = generation_call(
            &format!("{}/v1", server.uri()),
            "stepfun",
            "step-image-edit-2",
            generation(
                1,
                json!({
                    "seed": 17,
                    "steps": 30,
                    "cfg_scale": 7.5,
                    "generation_option_keys": ["negative_prompt", "text_mode"],
                    "negative_prompt": "not selected by model id",
                    "text_mode": true
                }),
            ),
        );
        StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["negative_prompt"], "not selected by model id");
        assert_eq!(body["text_mode"], true);
        assert!(body.get("generation_option_keys").is_none());
    }

    #[test]
    fn adapter_control_key_is_stripped_while_unknown_provider_fields_stay_open() {
        for control_value in [json!("anything"), json!(["negative_prompt", 1])] {
            let configured = json!({
                "generation_option_keys": control_value,
                "future_direct": {"revision": 1},
                "future_default": [1, 2],
                "endpoint": "must-not-leak"
            });
            let extra = json!({
                "future_direct": {"revision": 2},
                "future_request": "enabled",
                "api_key": "must-not-leak"
            });
            let mut fields = Map::new();
            merge_provider_options(&mut fields, &configured, &extra);

            assert_eq!(fields["future_direct"], json!({"revision": 2}));
            assert_eq!(fields["future_default"], json!([1, 2]));
            assert_eq!(fields["future_request"], "enabled");
            assert!(!fields.contains_key("generation_option_keys"));
            assert!(!fields.contains_key("endpoint"));
            assert!(!fields.contains_key("api_key"));
        }
    }

    #[tokio::test]
    async fn resolved_task_and_typed_request_must_match() {
        let mut call = generation_call(
            "https://example.invalid/v1",
            "stepfun",
            "custom-image-model",
            generation(1, json!({})),
        );
        call.task = ModelTask::ImageEdit;

        let error = StepFunImagesAdapter
            .submit(&test_http(), &call)
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::UnsupportedTask);
    }

    #[tokio::test]
    async fn plan_generation_uses_plan_path_and_returns_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/step_plan/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"url": "https://res.stepfun.com/image.png", "finish_reason": "success"}]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let call = generation_call(
            &format!("{}/step_plan/v1", server.uri()),
            "stepfun-plan",
            "step-image-edit-2",
            generation(1, json!({})),
        );
        let outcome = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected image assets") };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://res.stepfun.com/image.png"));
    }

    #[tokio::test]
    async fn regular_edit_uses_v1_multipart_endpoint_and_returns_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(body_string_contains("name=\"model\""))
            .and(body_string_contains("step-image-edit-2"))
            .and(body_string_contains("name=\"image\""))
            .and(body_string_contains("filename=\"image.png\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"url": "https://res.stepfun.com/edited.png"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = edit_call(
            &format!("{}/v1", server.uri()),
            "stepfun",
            "step-image-edit-2",
            edit(vec![input("image", b"png-image", "image/png")], json!({})),
        );
        call.model_params = json!({
            "endpoint": "/images/edits",
            "model": "must-not-override-model",
            "prompt": "must-not-override-prompt",
            "image": "must-not-override-image",
            "mask": "must-not-add-mask"
        });
        let outcome = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else {
            panic!("expected image assets")
        };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url == "https://res.stepfun.com/edited.png"));
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        for forbidden in [
            "must-not-override-model",
            "must-not-override-prompt",
            "must-not-override-image",
            "must-not-add-mask",
        ] {
            assert!(!body.contains(forbidden), "typed field was overridden: {forbidden}\n{body}");
        }
    }

    #[tokio::test]
    async fn typed_count_overrides_provider_n_and_multiple_outputs_are_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({"n": 1})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;
        let call = generation_call(
            &format!("{}/v1", server.uri()),
            "stepfun",
            "step-image-edit-2",
            generation(1, json!({"n": 99})),
        );
        StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();

        let invalid = generation_call(
            "http://127.0.0.1:9/v1",
            "stepfun",
            "step-image-edit-2",
            generation(2, json!({})),
        );
        let error = StepFunImagesAdapter.submit(&test_http(), &invalid).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(error.message.contains("exactly one"));
    }

    #[tokio::test]
    async fn plan_edit_sends_one_image_and_native_scalar_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/step_plan/v1/images/edits"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_string_contains("name=\"model\""))
            .and(body_string_contains("step-image-edit-2"))
            .and(body_string_contains("name=\"image\""))
            .and(body_string_contains("filename=\"image.webp\""))
            .and(body_string_contains("name=\"prompt\""))
            .and(body_string_contains("name=\"seed\""))
            .and(body_string_contains("name=\"steps\""))
            .and(body_string_contains("name=\"cfg_scale\""))
            .and(body_string_contains("name=\"negative_prompt\""))
            .and(body_string_contains("name=\"text_mode\""))
            .and(body_string_contains("name=\"response_format\""))
            .and(body_string_contains("name=\"n\""))
            .and(body_string_contains("name=\"quality\""))
            .and(body_string_contains("name=\"future_scalar\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"b64_json": "aGk=", "finish_reason": "success"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = edit_call(
            &format!("{}/step_plan/v1", server.uri()),
            "stepfun-plan",
            "step-image-edit-2",
            edit(
                vec![input("image", b"webp-image", "image/webp")],
                json!({
                    "seed": 1,
                    "steps": 8,
                    "cfg_scale": 1.0,
                    "negative_prompt": "模糊",
                    "text_mode": true,
                    "response_format": "b64_json",
                    "n": 9,
                    "quality": "hd",
                    "future_scalar": "enabled",
                    "generation_option_keys": ["anything"],
                    "credentials": "must-not-leak"
                }),
            ),
        );
        let outcome = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap();
        assert!(matches!(outcome, TaskOutcome::Done(TaskResult::Assets(assets)) if assets.len() == 1));

        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        for forbidden in ["name=\"credentials\"", "name=\"generation_option_keys\""] {
            assert!(!body.contains(forbidden), "field leaked into multipart: {forbidden}\n{body}");
        }
    }

    #[tokio::test]
    async fn multipart_rejects_complex_open_provider_values_before_network() {
        for value in [json!(null), json!({"nested": true}), json!(["one", "two"])] {
            let call = edit_call(
                "http://127.0.0.1:9/v1",
                "stepfun",
                "custom-image-model",
                edit(
                    vec![input("image", b"png-image", "image/png")],
                    json!({"future_complex": value}),
                ),
            );
            let error = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
            assert!(error.message.contains("strings, numbers, or booleans"));
        }
    }

    #[tokio::test]
    async fn edit_rejects_mask_multiple_or_non_image_inputs() {
        let cases = [
            vec![input("image", b"img", "image/png"), input("mask", b"mask", "image/png")],
            vec![input("image", b"a", "image/png"), input("image", b"b", "image/png")],
            vec![input("image", b"text", "text/plain")],
        ];
        for inputs in cases {
            let call = edit_call(
                "http://127.0.0.1:9/v1",
                "stepfun",
                "step-image-edit-2",
                edit(inputs, json!({})),
            );
            let error = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        }
    }

    #[tokio::test]
    async fn http_and_body_failures_are_classified() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let call = generation_call(
            &format!("{}/v1", server.uri()),
            "stepfun",
            "step-image-edit-2",
            generation(1, json!({})),
        );
        let error = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Auth);

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"finish_reason": "content_filtered"}]
            })))
            .mount(&server)
            .await;
        let error = StepFunImagesAdapter.submit(&test_http(), &call).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ContentPolicy);
    }
}
