//! Native Zhipu asynchronous video generation.
//!
//! Zhipu's video API is not OpenAI's Sora protocol: it submits JSON to
//! `POST /api/paas/v4/videos/generations`, then polls
//! `GET /api/paas/v4/async-result/{id}` until `task_status` is terminal.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, ResolvedConnection};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{encode_b64, error_from_response, get_request, post_json};
use crate::types::{
    JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult, VideoGenRequest,
};

use super::has_endpoint_override;

const ADAPTER_ID: &str = "zhipu.video_jobs";
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct ZhipuVideoJobsAdapter;

#[async_trait]
impl ProtocolAdapter for ZhipuVideoJobsAdapter {
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
                format!("{ADAPTER_ID} cannot serve task {:?}", call.request.task()),
            ));
        };
        let resp = post_json(
            http,
            &submit_url(call),
            SUBMIT_TIMEOUT,
            &call.connection.auth,
            &build_submit_body(call, req)?,
        )
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::parse(format!("invalid Zhipu video submit JSON: {e}")))?;
        if is_failed_status(&value) {
            return Err(InvokeError::new(InvokeErrorKind::JobFailed, failure_message(&value)));
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| InvokeError::parse("Zhipu video submit response missing 'id'"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: ADAPTER_ID.into(),
            remote_id: id.into(),
            poll_state: json!({"status_url": status_url(call, id)}),
        }))
    }

    async fn poll(
        &self,
        http: &reqwest::Client,
        call: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let url = job
            .poll_state
            .get("status_url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| status_url(call, &job.remote_id));
        let resp = get_request(http, &url, POLL_TIMEOUT, &call.connection.auth).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::parse(format!("invalid Zhipu video status JSON: {e}")))?;
        match parse_status(&value)? {
            VideoState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: ADAPTER_ID.into(),
                remote_id: job.remote_id.clone(),
                poll_state: json!({"status_url": url}),
            })),
            VideoState::Failed(message) => Err(InvokeError::new(InvokeErrorKind::JobFailed, message)),
            VideoState::Done(urls) => Ok(TaskOutcome::Done(TaskResult::Assets(
                urls.into_iter()
                    .map(|url| ProducedAsset {
                        data: ProducedData::Url(url),
                        mime: Some("video/mp4".into()),
                    })
                    .collect(),
            ))),
        }
    }
}

fn submit_url(call: &ResolvedCall) -> String {
    if has_endpoint_override(&call.model_params) || call.connection.is_full_url {
        call.dispatch_target().url
    } else {
        api_url(&call.connection, "/videos/generations")
    }
}

fn api_url(connection: &ResolvedConnection, path: &str) -> String {
    format!("{}{}", connection.base_url.trim().trim_end_matches('/'), path)
}

fn status_url(call: &ResolvedCall, id: &str) -> String {
    if let Some(template) = call
        .model_params
        .get("poll_endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
    {
        let endpoint = template.replace("{id}", id).replace("{task_id}", id);
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            return endpoint;
        }
        let base = call.connection.base_url.trim().trim_end_matches('/');
        if endpoint.starts_with('/')
            && let Ok(mut parsed) = reqwest::Url::parse(base)
        {
            parsed.set_path(&endpoint);
            parsed.set_query(None);
            parsed.set_fragment(None);
            return parsed.to_string().trim_end_matches('/').to_string();
        }
        return format!("{base}/{}", endpoint.trim_start_matches('/'));
    }

    let base = call.connection.base_url.trim().trim_end_matches('/');
    if call.connection.is_full_url
        && let Some(root) = base.strip_suffix("/videos/generations")
    {
        return format!("{root}/async-result/{id}");
    }
    api_url(&call.connection, &format!("/async-result/{id}"))
}

fn merge_optional(body: &mut Map<String, Value>, model_params: &Value, extra: &Value) {
    const KEYS: &[&str] = &[
        "quality",
        "with_audio",
        "watermark_enabled",
        "fps",
        "duration",
        "movement_amplitude",
        "aspect_ratio",
        "style",
        "request_id",
        "user_id",
    ];
    for source in [model_params, extra] {
        let source = source.get("request_defaults").unwrap_or(source);
        for key in KEYS {
            if let Some(value) = source.get(*key) {
                body.insert((*key).into(), value.clone());
            }
        }
    }
}

fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", encode_b64(bytes))
}

fn build_submit_body(call: &ResolvedCall, req: &VideoGenRequest) -> Result<Value, InvokeError> {
    let mut body = Map::from_iter([
        ("model".into(), Value::String(call.model.clone())),
        ("prompt".into(), Value::String(req.prompt.clone())),
    ]);
    merge_optional(&mut body, &call.model_params, &req.extra);
    if let Some(size) = &req.size {
        body.insert("size".into(), Value::String(size.clone()));
    }
    if let Some(seconds) = req.seconds {
        // The currently catalogued CogVideo models expose 5/10 seconds in the
        // official API. Provider-native request defaults remain an escape
        // hatch for a future model family with a different fixed duration.
        if !matches!(seconds, 5 | 10) {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "Zhipu CogVideo duration must be 5 or 10 seconds",
            ));
        }
        body.insert("duration".into(), Value::from(seconds));
    }
    let images = req
        .inputs
        .iter()
        .filter(|input| input.role != "mask")
        .map(|input| Value::String(data_uri(&input.mime, &input.bytes)))
        .collect::<Vec<_>>();
    match images.as_slice() {
        [] => {}
        [one] => {
            body.insert("image_url".into(), one.clone());
        }
        _ => {
            body.insert("image_url".into(), Value::Array(images));
        }
    }
    Ok(Value::Object(body))
}

#[derive(Debug, PartialEq, Eq)]
enum VideoState {
    Pending,
    Done(Vec<String>),
    Failed(String),
}

fn is_failed_status(value: &Value) -> bool {
    matches!(
        value.get("task_status").and_then(Value::as_str).unwrap_or("").to_ascii_uppercase().as_str(),
        "FAIL" | "FAILED"
    )
}

fn failure_message(value: &Value) -> String {
    value
        .get("error")
        .and_then(|error| error.get("message").and_then(Value::as_str).or_else(|| error.as_str()))
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("Zhipu video generation failed")
        .to_owned()
}

fn parse_status(value: &Value) -> Result<VideoState, InvokeError> {
    let status = value
        .get("task_status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_uppercase();
    match status.as_str() {
        "SUCCESS" | "SUCCEEDED" => {
            let results = value
                .get("video_result")
                .and_then(Value::as_array)
                .ok_or_else(|| InvokeError::parse("Zhipu video succeeded but missing 'video_result'"))?;
            let urls = results
                .iter()
                .filter_map(|item| item.get("url").and_then(Value::as_str))
                .filter(|url| !url.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if urls.is_empty() {
                return Err(InvokeError::parse(
                    "Zhipu video succeeded but 'video_result' contains no URL",
                ));
            }
            Ok(VideoState::Done(urls))
        }
        "FAIL" | "FAILED" => Ok(VideoState::Failed(failure_message(value))),
        // PROCESSING and unknown non-terminal states remain pollable.
        _ => Ok(VideoState::Pending),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;
    use crate::types::InputAsset;

    fn test_http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn input(bytes: &[u8]) -> InputAsset {
        InputAsset {
            id: None,
            role: "image".into(),
            bytes: bytes.into(),
            mime: "image/png".into(),
        }
    }

    #[tokio::test]
    async fn submit_and_poll_use_the_native_v4_job_protocol() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/paas/v4/videos/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "cogvideox-3",
                "prompt": "waves",
                "size": "1920x1080",
                "duration": 5,
                "with_audio": true,
                "image_url": "data:image/png;base64,aGk="
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "cogvideox-3", "id": "job-1", "task_status": "PROCESSING"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/paas/v4/async-result/job-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "job-1",
                "task_status": "SUCCESS",
                "video_result": [{"url": "https://cdn.example/video.mp4"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "waves".into(),
            seconds: Some(5),
            size: Some("1920x1080".into()),
            inputs: vec![input(b"hi")],
            extra: json!({"with_audio": true}),
        });
        let call = call(
            &format!("{}/api/paas/v4", server.uri()),
            "cogvideox-3",
            request,
        );
        let http = test_http();
        let TaskOutcome::Pending(job) = ZhipuVideoJobsAdapter.submit(&http, &call).await.unwrap()
        else {
            panic!("expected pending job")
        };
        assert_eq!(job.remote_id, "job-1");
        assert_eq!(
            job.poll_state["status_url"],
            format!("{}/api/paas/v4/async-result/job-1", server.uri())
        );

        let TaskOutcome::Done(TaskResult::Assets(assets)) =
            ZhipuVideoJobsAdapter.poll(&http, &call, &job).await.unwrap()
        else {
            panic!("expected video result")
        };
        assert!(matches!(&assets[0].data, ProducedData::Url(url) if url.ends_with("video.mp4")));
        assert_eq!(assets[0].mime.as_deref(), Some("video/mp4"));
    }

    #[test]
    fn status_parser_distinguishes_pending_failed_and_malformed_success() {
        assert_eq!(parse_status(&json!({"task_status": "PROCESSING"})).unwrap(), VideoState::Pending);
        assert_eq!(
            parse_status(&json!({"task_status": "FAIL", "message": "rejected"})).unwrap(),
            VideoState::Failed("rejected".into())
        );
        assert_eq!(
            parse_status(&json!({"task_status": "SUCCESS", "video_result": []}))
                .unwrap_err()
                .kind,
            InvokeErrorKind::ParseError
        );
    }

    #[test]
    fn poll_endpoint_override_accepts_task_id_template() {
        let request = TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "p".into(),
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        });
        let mut call = call("https://open.bigmodel.cn/api/paas/v4", "cogvideox-3", request);
        call.model_params = json!({"poll_endpoint": "https://status.example/jobs/{task_id}"});
        assert_eq!(status_url(&call, "job-9"), "https://status.example/jobs/job-9");
    }

    #[test]
    fn typed_duration_rejects_values_outside_the_current_cogvideo_contract() {
        let request = VideoGenRequest {
            prompt: "p".into(),
            seconds: Some(6),
            size: None,
            inputs: vec![],
            extra: json!({}),
        };
        let call = call(
            "https://open.bigmodel.cn/api/paas/v4",
            "cogvideox-3",
            TaskRequest::VideoGeneration(request.clone()),
        );
        assert_eq!(build_submit_body(&call, &request).unwrap_err().kind, InvokeErrorKind::InvalidParams);
    }
}
