//! `ark.images` / `ark.video_jobs` — Volcengine Ark (方舟) image generation
//! and asynchronous video tasks (protocol per
//! `docs/specs/2026-07-28-provider-protocol-variance.zh.md` §3, Ark domain:
//! `ark.cn-beijing.volces.com`, Bearer ARK_API_KEY on the default connection).
//!
//! Ark lives under `/api/v3` rather than the OpenAI `/v1` convention, so both
//! adapters compose their own URLs via [`ark_v3_url`] instead of the
//! conventional dispatch path; a `params.endpoint` override still wins for
//! images (routed through [`crate::call::ResolvedCall::dispatch_target`]) and
//! for video (submit + poll alike, via [`video_tasks_base`]).
//!
//! - [`ArkImagesAdapter`] (`"ark.images"`, ImageGeneration): sync
//!   `POST {root}/api/v3/images/generations`, OpenAI-shaped body plus Ark
//!   private knobs (`watermark`/`seed`/`guidance_scale` whitelisted from
//!   `extra`); response reuses
//!   [`crate::adapters::openai_images::parse_images_response`] (url|b64_json).
//! - [`ArkVideoJobsAdapter`] (`"ark.video_jobs"`, VideoGeneration): async
//!   `POST {root}/api/v3/contents/generations/tasks` → `GET .../tasks/{id}`.
//!   Generation params are encoded *inside the prompt text* (` --resolution
//!   {size} --duration {seconds}` suffix — Ark's signature quirk); an i2v
//!   first frame rides as a `content[]` data-URI `image_url` entry. Status
//!   vocabulary `queued/running` → Pending, `succeeded` → `content.video_url`
//!   (24 h URL, mime unknown), `failed/cancelled` →
//!   [`InvokeErrorKind::JobFailed`].

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{encode_b64, error_from_response, net_err};
use crate::types::{
    JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, VideoGenRequest,
};

use super::has_endpoint_override;
use super::openai_images::parse_images_response;

/// Generous ceiling for image generation / video-task submission.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
/// Poll round-trips are cheap status reads.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// Compose an Ark endpoint: `{root}/api/v3{path}`. The configured base is
/// tolerated with or without a trailing `/api/v3` (stripped then re-added) so
/// both `https://ark.cn-beijing.volces.com` and
/// `https://ark.cn-beijing.volces.com/api/v3` resolve identically. A full-url
/// connection base is already the complete endpoint (no path appended).
fn ark_v3_url(conn: &ResolvedConnection, path: &str) -> String {
    let base = conn.base_url.trim().trim_end_matches('/');
    if conn.is_full_url {
        return base.to_string();
    }
    let root = base.strip_suffix("/api/v3").unwrap_or(base);
    format!("{root}/api/v3{path}")
}

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
        task == ModelTask::ImageGeneration
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::ImageGeneration(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("ark.images cannot serve task {:?}", call.request.task()),
            ));
        };
        // Ark does not follow the `/v1` convention, so the URL is composed
        // here — but an explicit per-model `endpoint` override still wins
        // (resolved by the single dispatch authority).
        let url = if has_endpoint_override(&call.model_params) {
            call.dispatch_target().url
        } else {
            ark_v3_url(&call.connection, "/images/generations")
        };

        let mut body = json!({
            "model": call.model,
            "prompt": req.prompt,
            "response_format": "b64_json",
        });
        if let Some(size) = &req.size {
            body["size"] = Value::String(size.clone());
        }
        // Ark private generation knobs — whitelisted passthrough from `extra`.
        for key in ["watermark", "seed", "guidance_scale"] {
            if let Some(v) = req.extra.get(key) {
                body[key] = v.clone();
            }
        }

        let rb = http.post(&url).timeout(SUBMIT_TIMEOUT).json(&body);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid ark images JSON: {e}")))?;
        Ok(TaskOutcome::Done(TaskResult::Assets(parse_images_response(&value)?)))
    }
}

// ---------------------------------------------------------------------------
// ark.video_jobs
// ---------------------------------------------------------------------------

const VIDEO_ADAPTER_ID: &str = "ark.video_jobs";
const VIDEO_TASKS_PATH: &str = "/contents/generations/tasks";

/// The video-tasks collection URL for this call: an explicit `params.endpoint`
/// override wins (the dispatch-target URL with any query string stripped —
/// the poll's `/{id}` sub-path cannot carry a mid-URL query segment);
/// otherwise the conventional Ark `/api/v3` path. Submit and poll both ride
/// this, so an override moves the whole job lifecycle.
fn video_tasks_base(call: &ResolvedCall) -> String {
    if has_endpoint_override(&call.model_params) {
        let url = call.dispatch_target().url;
        let no_query = url.split('?').next().unwrap_or(url.as_str());
        no_query.trim_end_matches('/').to_string()
    } else {
        ark_v3_url(&call.connection, VIDEO_TASKS_PATH)
    }
}

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
        let url = video_tasks_base(call);
        let body = build_video_submit_body(&call.model, req);

        let rb = http.post(&url).timeout(SUBMIT_TIMEOUT).json(&body);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid ark task JSON: {e}")))?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| InvokeError::parse("ark task submit response missing 'id'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: VIDEO_ADAPTER_ID.into(),
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
        let status_url = format!("{}/{}", video_tasks_base(call), job.remote_id);
        let rb = http.get(&status_url).timeout(POLL_TIMEOUT);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid ark task status JSON: {e}")))?;

        match parse_video_task_status(&value)? {
            ArkTaskState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: VIDEO_ADAPTER_ID.into(),
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

/// Build the Ark video-task submit body. Generation parameters are encoded as
/// a ` --resolution {size} --duration {seconds}` suffix inside the prompt text
/// (Ark's in-prompt parameter encoding); the first input asset (i2v first
/// frame), when present, rides as a data-URI `image_url` content entry.
/// Pure — unit tested.
pub(crate) fn build_video_submit_body(model: &str, req: &VideoGenRequest) -> Value {
    let mut text = req.prompt.clone();
    if let Some(size) = &req.size {
        text.push_str(&format!(" --resolution {size}"));
    }
    if let Some(seconds) = req.seconds {
        text.push_str(&format!(" --duration {seconds}"));
    }
    let mut content = vec![json!({"type": "text", "text": text})];
    if let Some(input) = req.inputs.first() {
        content.push(json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", input.mime, encode_b64(&input.bytes)) }
        }));
    }
    json!({ "model": model, "content": content })
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
    use crate::adapters::test_support::call;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::types::{ImageGenRequest, InputAsset};

    fn ark_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let mut call = call(base_url, model, request);
        call.platform = "ark".into();
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
        JobHandle { adapter_id: VIDEO_ADAPTER_ID.into(), remote_id: remote_id.into(), poll_state: json!({}) }
    }

    // -- URL composition ------------------------------------------------------

    #[test]
    fn ark_v3_url_tolerates_trailing_api_v3_and_full_url() {
        let conn = |base: &str, full: bool| ResolvedConnection {
            role: "default".into(),
            base_url: base.into(),
            is_full_url: full,
            auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({}) },
            extra: json!({}),
        };
        assert_eq!(
            ark_v3_url(&conn("https://ark.cn-beijing.volces.com", false), "/images/generations"),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
        // Trailing /api/v3 (and trailing slash) tolerated — no doubling.
        assert_eq!(
            ark_v3_url(&conn("https://ark.cn-beijing.volces.com/api/v3/", false), "/images/generations"),
            "https://ark.cn-beijing.volces.com/api/v3/images/generations"
        );
        // Full-url base used verbatim, no path appended.
        assert_eq!(
            ark_v3_url(&conn("https://proxy.example/exact/endpoint", true), "/images/generations"),
            "https://proxy.example/exact/endpoint"
        );
    }

    // -- pure body/status fixtures ---------------------------------------------

    #[test]
    fn video_body_encodes_params_in_prompt_text() {
        let TaskRequest::VideoGeneration(req) = video_request(Some("720x480"), Some(5), vec![]) else {
            unreachable!()
        };
        let body = build_video_submit_body("seedance-pro", &req);
        assert_eq!(body["model"], "seedance-pro");
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "a wave --resolution 720x480 --duration 5");
    }

    #[test]
    fn video_body_without_params_has_bare_prompt() {
        let TaskRequest::VideoGeneration(req) = video_request(None, None, vec![]) else { unreachable!() };
        let body = build_video_submit_body("seedance-pro", &req);
        assert_eq!(body["content"][0]["text"], "a wave");
    }

    #[test]
    fn video_body_appends_first_input_as_data_uri_image() {
        // "aGk=" is base64("hi").
        let inputs = vec![
            InputAsset { id: None, role: "first_frame".into(), bytes: b"hi".to_vec(), mime: "image/png".into() },
            InputAsset { id: None, role: "extra".into(), bytes: b"nope".to_vec(), mime: "image/png".into() },
        ];
        let TaskRequest::VideoGeneration(req) = video_request(None, None, inputs) else { unreachable!() };
        let body = build_video_submit_body("seedance-pro", &req);
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "only the first input asset is attached");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,aGk=");
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
    async fn images_posts_api_v3_body_with_whitelisted_extras_and_decodes_b64() {
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
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let extra = json!({"watermark": false, "seed": 42, "steps": 9});
        let call = ark_call(&server.uri(), "doubao-seedream", image_request(Some("1024x1024"), extra));
        let out = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));

        // Non-whitelisted extras (steps) must not leak into the body.
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("steps").is_none(), "non-whitelisted extra leaked");
        assert!(body.get("size").is_some());
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
        let call = ark_call(&base, "doubao-seedream", image_request(None, json!({})));
        let out = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert!(matches!(&assets[0].data, ProducedData::Url(u) if u == "https://cdn/x.png"));
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

        let mut call = ark_call(&server.uri(), "doubao-seedream", image_request(None, json!({})));
        call.model_params = json!({"endpoint": "/custom/images"});
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

        let call = ark_call(&server.uri(), "doubao-seedream", image_request(None, json!({})));
        let err = ArkImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("bad ark key"), "message: {}", err.message);
    }

    // -- ark.video_jobs wiremock --------------------------------------------------

    #[tokio::test]
    async fn video_submit_posts_prompt_suffix_and_returns_pending_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v3/contents/generations/tasks"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "doubao-seedance",
                "content": [{"type": "text", "text": "a wave --resolution 720x480 --duration 5"}],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "cgt-1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;

        let call = ark_call(&server.uri(), "doubao-seedance", video_request(Some("720x480"), Some(5), vec![]));
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
        let call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, inputs));
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

        let call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
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

        let call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
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

        let call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let err = ArkVideoJobsAdapter.poll(&reqwest::Client::new(), &call, &job("cgt-9")).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "content blocked");
    }

    #[tokio::test]
    async fn video_params_endpoint_override_applies_to_submit_and_poll() {
        // Task 9 review fix: an explicit params.endpoint moves the WHOLE job
        // lifecycle — the custom path is the only place anything is mounted.
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

        let mut call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        call.model_params = json!({"endpoint": "/custom/video-tasks"});
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

        let call = ark_call(&server.uri(), "doubao-seedance", video_request(None, None, vec![]));
        let err = ArkVideoJobsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
