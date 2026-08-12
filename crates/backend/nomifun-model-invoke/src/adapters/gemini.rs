//! `gemini.generate_content` — Google Gemini native image generation/editing.
//!
//! The adapter posts the exact capability endpoint, normally
//! `{root}/v1beta/models/{model}:generateContent`. Chat is intentionally absent:
//! `gemini.generate_text` is an Agent protocol executed by `nomi-providers`,
//! not a one-shot model-invoke request.
//!
//! Response parsing tolerates both camelCase (`inlineData`/`mimeType`) and
//! snake_case (`inline_data`/`mime_type`). An empty result surfaces
//! `promptFeedback.blockReason` when present ([`InvokeErrorKind::ContentPolicy`]).

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_b64, encode_b64, error_from_response, post_json};
use crate::types::{
    InputAsset, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult,
};

use super::json_request_body;

/// Generous per-call ceiling: image generation is often multi-second.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Fire one `:generateContent` request and return the parsed response JSON.
async fn post_generate_content(
    http: &reqwest::Client,
    call: &ResolvedCall,
    url: &str,
    body: &Value,
) -> Result<Value, InvokeError> {
    let resp = post_json(http, url, REQUEST_TIMEOUT, &call.connection.auth, body).await?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    resp.json()
        .await
        .map_err(|e| InvokeError::response_json("invalid gemini JSON", &e))
}

/// The "model returned nothing" error: a `promptFeedback.blockReason` is a
/// policy refusal ([`InvokeErrorKind::ContentPolicy`]); otherwise the response
/// shape is simply missing the expected parts ([`InvokeErrorKind::ParseError`]).
fn no_output_error(value: &Value, what: &str) -> InvokeError {
    match value
        .get("promptFeedback")
        .and_then(|f| f.get("blockReason"))
        .and_then(|v| v.as_str())
    {
        Some(reason) => InvokeError::new(
            InvokeErrorKind::ContentPolicy,
            format!("gemini produced no {what}: {reason}"),
        ),
        None => InvokeError::parse(format!("gemini produced no {what}: no {what} parts in response")),
    }
}

/// `gemini.generate_content` — synchronous image generation/editing.
pub struct GeminiGenerateContentAdapter;

#[async_trait]
impl ProtocolAdapter for GeminiGenerateContentAdapter {
    fn id(&self) -> &'static str {
        "gemini.generate_content"
    }

    fn supports(&self, task: ModelTask) -> bool {
        matches!(task, ModelTask::ImageGeneration | ModelTask::ImageEdit)
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let (prompt, count, inputs, extra): (&str, u32, &[InputAsset], &Value) = match &call.request {
            TaskRequest::ImageGeneration(req) => (&req.prompt, req.count, &[], &req.extra),
            TaskRequest::ImageEdit(req) => (&req.prompt, req.count, &req.inputs, &req.extra),
            other => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("gemini.generate_content cannot serve task {:?}", other.task()),
                ));
            }
        };
        // One capability-supplied URL serves the whole count>1 loop.
        let url = call.endpoint_url()?;
        let body = json_request_body(
            &call.model_params,
            extra,
            build_generate_content_body(prompt, inputs),
        )?;

        // Gemini has no `n` parameter: count > 1 loops the request
        // sequentially, aggregating assets; any failure fails the call.
        let mut assets = Vec::new();
        for _ in 0..count.max(1) {
            let value = post_generate_content(http, call, &url, &body).await?;
            assets.extend(parse_gemini_assets(&value)?);
        }
        Ok(TaskOutcome::Done(TaskResult::Assets(assets)))
    }
}

/// Build the image `:generateContent` body: prompt text part + each input as
/// `inline_data`, requesting `responseModalities: ["TEXT","IMAGE"]`. Pure —
/// unit tested.
pub(crate) fn build_generate_content_body(prompt: &str, inputs: &[InputAsset]) -> Value {
    let mut parts: Vec<Value> = vec![json!({"text": prompt})];
    for input in inputs {
        parts.push(json!({
            "inline_data": { "mime_type": input.mime, "data": encode_b64(&input.bytes) }
        }));
    }
    json!({
        "contents": [{ "parts": parts }],
        "generationConfig": { "responseModalities": ["TEXT", "IMAGE"] }
    })
}

/// Parse `candidates[].content.parts[].inlineData{mimeType,data}` into image
/// artifacts. Accepts both camelCase (`inlineData`/`mimeType`) and snake_case
/// (`inline_data`/`mime_type`) shapes. Pure — unit tested.
pub(crate) fn parse_gemini_assets(value: &Value) -> Result<Vec<ProducedAsset>, InvokeError> {
    let candidates = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("gemini response missing 'candidates'"))?;

    let mut out = Vec::new();
    for cand in candidates {
        let Some(parts) = cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) else {
            continue;
        };
        for part in parts {
            let Some(inline) = part.get("inlineData").or_else(|| part.get("inline_data")) else { continue };
            let Some(data) = inline.get("data").and_then(|v| v.as_str()) else { continue };
            let bytes =
                decode_b64(data).ok_or_else(|| InvokeError::parse("gemini inlineData is not valid base64"))?;
            let mime = inline
                .get("mimeType")
                .or_else(|| inline.get("mime_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("image/png")
                .to_string();
            out.push(ProducedAsset { data: ProducedData::Bytes(bytes), mime: Some(mime) });
        }
    }

    if out.is_empty() {
        return Err(no_output_error(value, "image"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::ResolvedConnection;
    use crate::types::{ImageEditRequest, ImageGenRequest};

    /// A gemini [`ResolvedCall`] as the resolver produces it: platform
    /// `gemini`, default connection rewritten to `header_key:x-goog-api-key`.
    fn gemini_call(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-0000000000aa".into(),
            config_revision: 1,
            platform: "gemini".into(),
            model: model.into(),
            task,
            protocol: protocol.into(),
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                auth: AuthMaterial {
                    scheme: AuthScheme::HeaderKey("x-goog-api-key".into()),
                    credentials: json!({"api_keys": ["g-key"]}),
                },
                extra: json!({}),
            },
            model_params: json!({"endpoint": endpoint}),
            request,
        }
    }

    fn content_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        gemini_call(
            base_url,
            model,
            "gemini.generate_content",
            "/v1beta/models/{model}:generateContent",
            request,
        )
    }

    fn gen_request(count: u32) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a fox".into(),
            count,
            size: None,
            quality: None,
            extra: json!({}),
        })
    }

    // -- ported pure-parser fixtures -----------------------------------------

    #[test]
    fn parse_camel_and_snake_inline_data() {
        let v = json!({
            "candidates": [{
                "content": { "parts": [
                    {"text": "here you go"},
                    {"inlineData": {"mimeType": "image/png", "data": "aGk="}}
                ]}
            }]
        });
        let out = parse_gemini_assets(&v).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0].data {
            ProducedData::Bytes(b) => assert_eq!(b, b"hi"),
            _ => panic!("expected bytes"),
        }
        assert_eq!(out[0].mime.as_deref(), Some("image/png"));

        let v2 = json!({
            "candidates": [{ "content": { "parts": [
                {"inline_data": {"mime_type": "image/jpeg", "data": "aGk="}}
            ]}}]
        });
        let out2 = parse_gemini_assets(&v2).unwrap();
        assert_eq!(out2[0].mime.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn parse_no_image_surfaces_block_reason_as_content_policy() {
        let v = json!({"candidates": [], "promptFeedback": {"blockReason": "SAFETY"}});
        let err = parse_gemini_assets(&v).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ContentPolicy);
        assert!(err.message.contains("SAFETY"), "{}", err.message);
    }

    #[test]
    fn parse_missing_candidates_or_empty_is_parse_error() {
        assert_eq!(parse_gemini_assets(&json!({})).unwrap_err().kind, InvokeErrorKind::ParseError);
        assert_eq!(parse_gemini_assets(&json!({"candidates": []})).unwrap_err().kind, InvokeErrorKind::ParseError);
    }

    #[test]
    fn content_body_attaches_inputs_as_inline_data() {
        let inputs = vec![InputAsset { id: None, role: "image".into(), bytes: b"hi".to_vec(), mime: "image/png".into() }];
        let body = build_generate_content_body("p", &inputs);
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["text"], "p");
        // "aGk=" is base64("hi")
        assert_eq!(parts[1]["inline_data"]["mime_type"], "image/png");
        assert_eq!(parts[1]["inline_data"]["data"], "aGk=");
        assert_eq!(body["generationConfig"]["responseModalities"], json!(["TEXT", "IMAGE"]));
    }

    // -- wiremock request/response tests -------------------------------------

    #[tokio::test]
    async fn generate_content_sends_goog_key_header_and_decodes_b64() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash-image:generateContent"))
            .and(header("x-goog-api-key", "g-key"))
            .and(body_partial_json(json!({
                "contents": [{"parts": [{"text": "a fox"}]}],
                "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [
                    {"inlineData": {"mimeType": "image/png", "data": "aGk="}}
                ]}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = content_call(&server.uri(), "gemini-2.5-flash-image", gen_request(1));
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));
        assert_eq!(assets[0].mime.as_deref(), Some("image/png"));
    }

    #[tokio::test]
    async fn generate_content_count_two_loops_two_requests_and_aggregates() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash-image:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [
                    {"inlineData": {"mimeType": "image/png", "data": "aGk="}}
                ]}}]
            })))
            .expect(2)
            .mount(&server)
            .await;

        let call = content_call(&server.uri(), "gemini-2.5-flash-image", gen_request(2));
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 2, "count=2 must aggregate one asset per request");
    }

    #[tokio::test]
    async fn generate_content_params_endpoint_override_wins() {
        // The custom capability endpoint is reused by every iteration.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/gemini"))
            .and(header("x-goog-api-key", "g-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [
                    {"inlineData": {"mimeType": "image/png", "data": "aGk="}}
                ]}}]
            })))
            .expect(2)
            .mount(&server)
            .await;

        let call = gemini_call(
            &server.uri(),
            "gemini-2.5-flash-image",
            "gemini.generate_content",
            "/custom/gemini",
            gen_request(2),
        );
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 2));
    }

    #[tokio::test]
    async fn generate_content_edit_sends_inputs_as_inline_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash-image:generateContent"))
            .and(body_partial_json(json!({
                "contents": [{"parts": [
                    {"text": "add a hat"},
                    {"inline_data": {"mime_type": "image/png", "data": "aGk="}}
                ]}],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [
                    {"inline_data": {"mime_type": "image/png", "data": "aGk="}}
                ]}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "add a hat".into(),
            count: 1,
            size: None,
            inputs: vec![InputAsset { id: None, role: "image".into(), bytes: b"hi".to_vec(), mime: "image/png".into() }],
            extra: json!({}),
        });
        let call = content_call(&server.uri(), "gemini-2.5-flash-image", request);
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 1));
    }

    #[tokio::test]
    async fn generate_content_block_reason_surfaces_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash-image:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [],
                "promptFeedback": {"blockReason": "SAFETY"}
            })))
            .mount(&server)
            .await;

        let call = content_call(&server.uri(), "gemini-2.5-flash-image", gen_request(1));
        let err = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ContentPolicy);
        assert!(err.message.contains("SAFETY"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = content_call(&server.uri(), "gemini-2.5-flash-image", gen_request(1));
        let err = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));

    }
}
