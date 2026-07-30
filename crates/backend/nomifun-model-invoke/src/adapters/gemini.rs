//! `gemini.generate_content` / `gemini.generate_text` — Google Gemini
//! `:generateContent` (ported from
//! `nomifun-creation/src/adapters/{gemini_image.rs, gemini_text.rs}`).
//!
//! Both adapters `POST {root}/v1beta/models/{model}:generateContent` (a
//! trailing `/v1beta` on the configured base is tolerated; `is_full_url`
//! bases are used verbatim; an explicit `params.endpoint` override wins over
//! both, routed through [`crate::call::ResolvedCall::dispatch_target`]). Auth
//! is applied declaratively via
//! [`crate::auth::AuthMaterial::apply`] — the resolver rewrites gemini
//! default connections to `header_key:x-goog-api-key`, so no header is
//! hardcoded here.
//!
//! - [`GeminiGenerateContentAdapter`] (`"gemini.generate_content"`) serves
//!   ImageGeneration + ImageEdit: prompt (+ input images as `inline_data`)
//!   with `generationConfig.responseModalities: ["TEXT","IMAGE"]`. Gemini has
//!   no `n` parameter, so `count > 1` loops the request sequentially and
//!   aggregates the produced assets (any failure fails the whole call).
//! - [`GeminiGenerateTextAdapter`] (`"gemini.generate_text"`) serves Chat:
//!   prompt as the sole text part, optional `system` → `systemInstruction`,
//!   `extra.max_tokens` → `generationConfig.maxOutputTokens`; the reply is the
//!   concatenation of `candidates[].content.parts[].text` →
//!   [`TaskResult::Text`].
//!
//! Response parsing tolerates both camelCase (`inlineData`/`mimeType`) and
//! snake_case (`inline_data`/`mime_type`). An empty result surfaces
//! `promptFeedback.blockReason` when present ([`InvokeErrorKind::ContentPolicy`]).

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::adapters::has_endpoint_override;
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{decode_b64, encode_b64, error_from_response, net_err};
use crate::types::{
    ChatTextRequest, InputAsset, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult,
};

/// Generous per-call ceiling: image generation is often multi-second.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// The Gemini `:generateContent` URL for a model. Gemini uses a
/// `/v1beta/models` scheme rather than `/v1`; a trailing `/v1beta` on the
/// configured base is tolerated (stripped then re-added) so both
/// `https://host` and `https://host/v1beta` resolve identically. A
/// full-url connection base is already the complete endpoint.
fn generate_content_url(conn: &ResolvedConnection, model: &str) -> String {
    let base = conn.base_url.trim().trim_end_matches('/');
    if conn.is_full_url {
        return base.to_string();
    }
    let root = base.strip_suffix("/v1beta").unwrap_or(base);
    format!("{root}/v1beta/models/{model}:generateContent")
}

/// The `:generateContent` URL for this call: an explicit `params.endpoint`
/// override wins (resolved verbatim by the single dispatch authority);
/// otherwise the conventional `/v1beta/models/{model}` path via
/// [`generate_content_url`].
fn call_url(call: &ResolvedCall) -> String {
    if has_endpoint_override(&call.model_params) {
        call.dispatch_target().url
    } else {
        generate_content_url(&call.connection, &call.model)
    }
}

/// Fire one `:generateContent` request and return the parsed response JSON.
async fn post_generate_content(
    http: &reqwest::Client,
    call: &ResolvedCall,
    url: &str,
    body: &Value,
) -> Result<Value, InvokeError> {
    let rb = http.post(url).timeout(REQUEST_TIMEOUT).json(body);
    let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(error_from_response(resp).await);
    }
    resp.json().await.map_err(|e| InvokeError::parse(format!("invalid gemini JSON: {e}")))
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
        let (prompt, count, inputs): (&str, u32, &[InputAsset]) = match &call.request {
            TaskRequest::ImageGeneration(req) => (&req.prompt, req.count, &[]),
            TaskRequest::ImageEdit(req) => (&req.prompt, req.count, &req.inputs),
            other => {
                return Err(InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("gemini.generate_content cannot serve task {:?}", other.task()),
                ));
            }
        };
        // One URL for the whole call (the count>1 loop reuses it): an
        // explicit `params.endpoint` override wins over convention.
        let url = call_url(call);
        let body = build_generate_content_body(prompt, inputs);

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

/// `gemini.generate_text` — synchronous single-turn text chat.
pub struct GeminiGenerateTextAdapter;

#[async_trait]
impl ProtocolAdapter for GeminiGenerateTextAdapter {
    fn id(&self) -> &'static str {
        "gemini.generate_text"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Chat
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::ChatText(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("gemini.generate_text cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call_url(call);
        let body = build_generate_text_body(req);
        let value = post_generate_content(http, call, &url, &body).await?;
        Ok(TaskOutcome::Done(TaskResult::Text(parse_gemini_text(&value)?)))
    }
}

/// Build the text `:generateContent` body. Pure — unit tested.
///
/// - The prompt is the sole text part (this path carries no multimodal inputs).
/// - A non-blank `system` (trimmed) → `systemInstruction`.
/// - `extra.max_tokens` (number) → `generationConfig.maxOutputTokens`, else omitted.
pub(crate) fn build_generate_text_body(req: &ChatTextRequest) -> Value {
    let mut body = json!({ "contents": [{ "parts": [{"text": req.prompt}] }] });
    if let Some(system) = req.system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
    }
    if let Some(max) = req.extra.get("max_tokens").and_then(|v| v.as_u64()) {
        body["generationConfig"] = json!({ "maxOutputTokens": max });
    }
    body
}

/// Concatenate `candidates[].content.parts[].text`. Surfaces a
/// `promptFeedback.blockReason` when the model returned no text. Pure —
/// unit tested.
pub(crate) fn parse_gemini_text(value: &Value) -> Result<String, InvokeError> {
    let candidates = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("gemini response missing 'candidates'"))?;

    let mut out = String::new();
    for cand in candidates {
        let Some(parts) = cand.get("content").and_then(|c| c.get("parts")).and_then(|p| p.as_array()) else {
            continue;
        };
        for part in parts {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                out.push_str(t);
            }
        }
    }

    if out.trim().is_empty() {
        return Err(no_output_error(value, "text"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::types::{ImageEditRequest, ImageGenRequest};

    /// A gemini [`ResolvedCall`] as the resolver produces it: platform
    /// `gemini`, default connection rewritten to `header_key:x-goog-api-key`.
    fn gemini_call(base_url: &str, is_full_url: bool, model: &str, request: TaskRequest) -> ResolvedCall {
        let task = request.task();
        ResolvedCall {
            provider_id: "018f0000-0000-7000-8000-0000000000aa".into(),
            platform: "gemini".into(),
            model: model.into(),
            task,
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: base_url.into(),
                is_full_url,
                auth: AuthMaterial {
                    scheme: AuthScheme::HeaderKey("x-goog-api-key".into()),
                    credentials: json!({"api_keys": ["g-key"]}),
                },
                extra: json!({}),
            },
            model_params: json!({}),
            request,
        }
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

    // -- URL composition (ported gemini_generate_url fixtures) ---------------

    #[test]
    fn url_composed_from_root_and_tolerates_trailing_v1beta() {
        let conn = |base: &str, full: bool| ResolvedConnection {
            role: "default".into(),
            base_url: base.into(),
            is_full_url: full,
            auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({}) },
            extra: json!({}),
        };
        assert_eq!(
            generate_content_url(&conn("https://generativelanguage.googleapis.com", false), "gemini-2.5-flash-image"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent"
        );
        // trailing /v1beta (and trailing slash) tolerated
        assert_eq!(
            generate_content_url(&conn("https://host/v1beta/", false), "m"),
            "https://host/v1beta/models/m:generateContent"
        );
        // full-url base used verbatim
        assert_eq!(
            generate_content_url(&conn("https://proxy.example/custom:generateContent", true), "m"),
            "https://proxy.example/custom:generateContent"
        );
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
    fn parse_text_concatenates_parts() {
        let v = json!({
            "candidates": [{ "content": { "parts": [
                {"text": "gemini says "},
                {"text": "hi"}
            ]}}]
        });
        assert_eq!(parse_gemini_text(&v).unwrap(), "gemini says hi");
    }

    #[test]
    fn parse_no_text_surfaces_block_reason() {
        let v = json!({"candidates": [], "promptFeedback": {"blockReason": "SAFETY"}});
        let err = parse_gemini_text(&v).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ContentPolicy);
        assert!(err.message.contains("SAFETY"), "{}", err.message);
        assert_eq!(parse_gemini_text(&json!({})).unwrap_err().kind, InvokeErrorKind::ParseError);
    }

    // -- pure body-builder fixtures (ported from gemini_text) ----------------

    #[test]
    fn text_body_prompt_only_has_no_config() {
        let req = ChatTextRequest { prompt: "greet me".into(), system: None, extra: json!({}) };
        let body = build_generate_text_body(&req);
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["text"], "greet me");
        assert!(body.get("systemInstruction").is_none());
        assert!(body.get("generationConfig").is_none());
    }

    #[test]
    fn text_body_carries_system_and_max_tokens() {
        let req = ChatTextRequest {
            prompt: "p".into(),
            system: Some(" sys ".into()),
            extra: json!({"max_tokens": 64}),
        };
        let body = build_generate_text_body(&req);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 64);
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

        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", gen_request(1));
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

        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", gen_request(2));
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 2, "count=2 must aggregate one asset per request");
    }

    #[tokio::test]
    async fn generate_content_params_endpoint_override_wins() {
        // Whole-branch review Finding 1: params.endpoint (dispatch rule 1)
        // must win over the /v1beta convention. The custom path is the only
        // mounted mock; count=2 proves the loop rides the override too.
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

        let mut call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", gen_request(2));
        call.model_params = json!({"endpoint": "/custom/gemini"});
        let out = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 2));
    }

    #[tokio::test]
    async fn generate_text_params_endpoint_override_wins() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/gemini-text"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [{"text": "custom hi"}]}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ChatText(ChatTextRequest { prompt: "hi".into(), system: None, extra: json!({}) });
        let mut call = gemini_call(&server.uri(), false, "gemini-2.5-flash", request);
        call.model_params = json!({"endpoint": "/custom/gemini-text"});
        let out = GeminiGenerateTextAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        assert!(matches!(out, TaskOutcome::Done(TaskResult::Text(t)) if t == "custom hi"));
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
        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", request);
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

        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", gen_request(1));
        let err = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ContentPolicy);
        assert!(err.message.contains("SAFETY"), "message: {}", err.message);
    }

    #[tokio::test]
    async fn generate_text_posts_body_and_returns_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
            .and(header("x-goog-api-key", "g-key"))
            .and(body_partial_json(json!({
                "contents": [{"parts": [{"text": "say hi"}]}],
                "systemInstruction": {"parts": [{"text": "be terse"}]},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [{"text": "hello from gemini"}]}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ChatText(ChatTextRequest {
            prompt: "say hi".into(),
            system: Some("be terse".into()),
            extra: json!({}),
        });
        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash", request);
        let out = GeminiGenerateTextAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Text(text)) = out else { panic!("expected Done(Text)") };
        assert_eq!(text, "hello from gemini");
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash-image", gen_request(1));
        let err = GeminiGenerateContentAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));

        let request = TaskRequest::ChatText(ChatTextRequest { prompt: "hi".into(), system: None, extra: json!({}) });
        let call = gemini_call(&server.uri(), false, "gemini-2.5-flash", request);
        let err = GeminiGenerateTextAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
