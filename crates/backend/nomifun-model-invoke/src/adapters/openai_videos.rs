//! `openai.videos` — OpenAI-compatible asynchronous video generation (ported
//! from `nomifun-creation/src/adapters/openai_video.rs`).
//!
//! - submit → `POST` the dispatch target (conventionally `{base}/v1/videos`;
//!   a `params.endpoint` override IS honored here — that was a known P0 gap).
//!   Multipart: model/prompt/seconds/size + optional `input_reference` for
//!   i2v. Returns a remote job `{id,status}` → [`TaskOutcome::Pending`].
//! - poll → `GET {video_base}/{id}` where `video_base` is the submit URL with
//!   any query string stripped; on a completed status, fetch the bytes from
//!   `GET {video_base}/{id}/content` → [`TaskOutcome::Done`]; on a failed
//!   status → [`InvokeErrorKind::JobFailed`] (a terminal remote state, not a
//!   transient transport error).

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{MAX_ARTIFACT_BYTES, error_from_response, net_err, read_body_capped};
use crate::types::{
    JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, VideoGenRequest,
};

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(30);
/// Downloading a finished video can be large; allow more headroom.
const CONTENT_TIMEOUT: Duration = Duration::from_secs(300);

/// OpenAI-compatible async `/videos` submit→poll→content protocol.
pub struct OpenAiVideosAdapter;

const ADAPTER_ID: &str = "openai.videos";

#[async_trait]
impl ProtocolAdapter for OpenAiVideosAdapter {
    fn id(&self) -> &'static str {
        ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::VideoGeneration
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::VideoGeneration(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.videos cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.dispatch_target().url;

        let form = build_submit_form(&call.model, req)?;
        let rb = http.post(&url).timeout(SUBMIT_TIMEOUT).multipart(form);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid videos JSON: {e}")))?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| InvokeError::parse("videos submit response missing 'id'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: ADAPTER_ID.into(),
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
        let base = video_base(call);
        let status_url = format!("{base}/{}", job.remote_id);
        let rb = http.get(&status_url).timeout(POLL_TIMEOUT);
        let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid videos status JSON: {e}")))?;

        match parse_video_status(&value) {
            VideoStatus::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: ADAPTER_ID.into(),
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            VideoStatus::Failed(msg) => Err(InvokeError::new(InvokeErrorKind::JobFailed, msg)),
            VideoStatus::Completed => {
                let content_url = format!("{base}/{}/content", job.remote_id);
                let rb = http.get(&content_url).timeout(CONTENT_TIMEOUT);
                let resp = call.connection.auth.apply(rb)?.send().await.map_err(net_err)?;
                if !resp.status().is_success() {
                    return Err(error_from_response(resp).await);
                }
                let mime = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "video/mp4".to_string());
                let bytes = read_body_capped(resp, MAX_ARTIFACT_BYTES).await?;
                Ok(TaskOutcome::Done(TaskResult::Assets(vec![ProducedAsset {
                    data: ProducedData::Bytes(bytes),
                    mime: Some(mime),
                }])))
            }
        }
    }
}

/// Build the multipart submit form from the typed request.
fn build_submit_form(model: &str, req: &VideoGenRequest) -> Result<Form, InvokeError> {
    let mut form = Form::new().text("model", model.to_string()).text("prompt", req.prompt.clone());
    if let Some(seconds) = req.seconds {
        form = form.text("seconds", seconds.to_string());
    }
    if let Some(size) = &req.size {
        form = form.text("size", size.clone());
    }
    // i2v reference frame — the first reference/first_frame input.
    if let Some(reference) = req
        .inputs
        .iter()
        .find(|i| matches!(i.role.as_str(), "reference" | "first_frame"))
        .or_else(|| req.inputs.first())
    {
        let part = Part::bytes(reference.bytes.clone())
            .file_name("input_reference")
            .mime_str(&reference.mime)
            .map_err(|e| {
                InvokeError::new(InvokeErrorKind::InvalidParams, format!("invalid reference mime: {e}"))
            })?;
        form = form.part("input_reference", part);
    }
    Ok(form)
}

/// The videos collection URL for this call: the dispatch-target URL with any
/// query string stripped (the `/{id}` and `/{id}/content` sub-paths cannot
/// carry a query segment in the middle). This is what makes a
/// `params.endpoint` override effective for video, submit and poll alike.
fn video_base(call: &ResolvedCall) -> String {
    let url = call.dispatch_target().url;
    let no_query = url.split('?').next().unwrap_or(url.as_str());
    no_query.trim_end_matches('/').to_string()
}

/// The distilled state of a video job.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VideoStatus {
    Pending,
    Completed,
    Failed(String),
}

/// Map a videos status body to a [`VideoStatus`]. Tolerant of the common status
/// vocabulary across OpenAI-compatible video APIs. Pure — unit tested.
pub(crate) fn parse_video_status(value: &Value) -> VideoStatus {
    let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
    match status.as_str() {
        "completed" | "succeeded" | "success" | "done" => VideoStatus::Completed,
        "failed" | "error" | "cancelled" | "canceled" => {
            let msg = value
                .get("error")
                .and_then(|e| e.get("message").and_then(|m| m.as_str()).or_else(|| e.as_str()))
                .unwrap_or("video generation failed")
                .to_string();
            VideoStatus::Failed(msg)
        }
        // "queued" | "in_progress" | "running" | "processing" | "" → keep waiting.
        _ => VideoStatus::Pending,
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;
    use crate::types::InputAsset;

    // -- ported pure-parser fixtures ---------------------------------------

    #[test]
    fn status_completed_variants() {
        for s in ["completed", "succeeded", "success", "done"] {
            assert_eq!(parse_video_status(&json!({"status": s})), VideoStatus::Completed);
        }
    }

    #[test]
    fn status_pending_variants() {
        for s in ["queued", "in_progress", "running", "processing", ""] {
            assert_eq!(parse_video_status(&json!({"status": s})), VideoStatus::Pending);
        }
        assert_eq!(parse_video_status(&json!({})), VideoStatus::Pending);
    }

    #[test]
    fn status_failed_carries_message() {
        let v = json!({"status": "failed", "error": {"message": "moderation blocked"}});
        assert_eq!(parse_video_status(&v), VideoStatus::Failed("moderation blocked".into()));
        // string error form
        let v2 = json!({"status": "error", "error": "boom"});
        assert_eq!(parse_video_status(&v2), VideoStatus::Failed("boom".into()));
        // no detail → default message
        let v3 = json!({"status": "failed"});
        assert_eq!(parse_video_status(&v3), VideoStatus::Failed("video generation failed".into()));
    }

    // -- wiremock submit → poll → content chain ------------------------------

    fn video_request(inputs: Vec<InputAsset>) -> TaskRequest {
        TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "a wave".into(),
            seconds: Some(4),
            size: Some("1280x720".into()),
            inputs,
            extra: json!({}),
        })
    }

    fn job(remote_id: &str) -> JobHandle {
        JobHandle { adapter_id: ADAPTER_ID.into(), remote_id: remote_id.into(), poll_state: json!({}) }
    }

    #[tokio::test]
    async fn submit_posts_multipart_and_returns_pending_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_string_contains("name=\"model\""))
            .and(body_string_contains("name=\"prompt\""))
            .and(body_string_contains("name=\"seconds\""))
            .and(body_string_contains("name=\"size\""))
            .and(body_string_contains("name=\"input_reference\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;

        let inputs =
            vec![InputAsset { id: None, role: "first_frame".into(), bytes: b"frame".to_vec(), mime: "image/png".into() }];
        let call = call(&server.uri(), "sora-2", video_request(inputs));
        let out = OpenAiVideosAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.adapter_id, "openai.videos");
        assert_eq!(handle.remote_id, "vid_1");
        assert_eq!(handle.poll_state, json!({}));
    }

    #[tokio::test]
    async fn submit_missing_id_is_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "queued"})))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "sora-2", video_request(vec![]));
        let err = OpenAiVideosAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn poll_pending_then_completed_downloads_content() {
        let server = MockServer::start().await;
        // First poll: still running.
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "in_progress"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second poll: completed → content fetch.
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_1", "status": "completed"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_1/content"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4; charset=binary")
                    .set_body_bytes(b"mp4-bytes".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let call = call(&server.uri(), "sora-2", video_request(vec![]));
        let http = reqwest::Client::new();

        let first = OpenAiVideosAdapter.poll(&http, &call, &job("vid_1")).await.unwrap();
        let TaskOutcome::Pending(handle) = first else { panic!("expected Pending on in_progress") };
        assert_eq!(handle.remote_id, "vid_1");
        assert_eq!(handle.adapter_id, "openai.videos");

        let second = OpenAiVideosAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = second else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"mp4-bytes"));
        // Content-Type parameters are stripped down to the bare MIME.
        assert_eq!(assets[0].mime.as_deref(), Some("video/mp4"));
    }

    #[tokio::test]
    async fn poll_failed_status_is_job_failed_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/vid_2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "vid_2", "status": "failed", "error": {"message": "moderation blocked"}
            })))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "sora-2", video_request(vec![]));
        let err = OpenAiVideosAdapter.poll(&reqwest::Client::new(), &call, &job("vid_2")).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "moderation blocked");
    }

    #[tokio::test]
    async fn params_endpoint_override_applies_to_submit_poll_and_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/video-jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "vid_9", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/custom/video-jobs/vid_9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "succeeded"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/custom/video-jobs/vid_9/content"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"custom-bytes".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = call(&server.uri(), "sora-2", video_request(vec![]));
        call.model_params = json!({"endpoint": "/custom/video-jobs"});
        let http = reqwest::Client::new();

        let out = OpenAiVideosAdapter.submit(&http, &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.remote_id, "vid_9");

        let done = OpenAiVideosAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = done else { panic!("expected Done(Assets)") };
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"custom-bytes"));
        // No content-type header → default mime.
        assert_eq!(assets[0].mime.as_deref(), Some("video/mp4"));
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "sora-2", video_request(vec![]));
        let err = OpenAiVideosAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
