//! `dashscope.images` / `dashscope.embeddings` — Alibaba DashScope (百炼)
//! native protocols (per
//! `docs/specs/2026-07-28-provider-protocol-variance.zh.md` §4, single domain
//! `dashscope.aliyuncs.com`, single Bearer key on the default connection, but
//! the body rides DashScope's own `input`/`parameters` wrapper — not the
//! OpenAI shape).
//!
//! - [`DashScopeImagesAdapter`] (`"dashscope.images"`, ImageGeneration):
//!   FORCED-async submit `POST {root}/api/v1/services/aigc/text2image/image-synthesis`
//!   with the mandatory `X-DashScope-Async: enable` header (omitting it is a
//!   400 upstream), body `{model, input: {prompt}, parameters: {size?, n?}}`
//!   → `output.task_id` → poll `GET {root}/api/v1/tasks/{id}` — DashScope's
//!   PLATFORM-UNIFIED task poller. Submit and poll endpoints are independently
//!   injected from the typed capability. Status vocab
//!   `PENDING/RUNNING` (and unknown) → Pending, `SUCCEEDED` →
//!   `output.results[].url` (24 h URLs, the caller fetches them),
//!   `FAILED/CANCELED` → [`InvokeErrorKind::JobFailed`] with
//!   `output.message`|`output.code`.
//! - [`DashScopeEmbeddingsAdapter`] (`"dashscope.embeddings"`, Embedding):
//!   sync `POST {root}/api/v1/services/embeddings/text-embedding/text-embedding`
//!   with `{model, input: {texts: [..]}}` (+ whitelisted `text_type`/
//!   `dimension` from `extra` under `parameters`) →
//!   `output.embeddings[].embedding`, re-ordered by `text_index` when every
//!   item carries one.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::{ResolvedCall, resolve_endpoint};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::manifest::expand_protocol_endpoint_template;
use crate::transport::{error_from_response, get_request, send_with_rotation};
use crate::types::{
    JobHandle, ProducedAsset, ProducedData, TaskOutcome, TaskRequest, TaskResult,
};

use super::json_request_body;

/// Submit is a cheap enqueue; generation happens behind the task id.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Poll round-trips are cheap status reads.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
/// Embeddings are synchronous.
const EMBEDDINGS_TIMEOUT: Duration = Duration::from_secs(60);

/// The mandatory async marker: same path serves sync/async semantics and the
/// wanx pipeline is async-only (400 without this header).
const ASYNC_HEADER: (&str, &str) = ("X-DashScope-Async", "enable");

const IMAGES_ADAPTER_ID: &str = "dashscope.images";
const EMBEDDINGS_ADAPTER_ID: &str = "dashscope.embeddings";

// ---------------------------------------------------------------------------
// dashscope.images
// ---------------------------------------------------------------------------

/// DashScope forced-async wanx image generation (submit → unified task poll).
pub struct DashScopeImagesAdapter;

#[async_trait]
impl ProtocolAdapter for DashScopeImagesAdapter {
    fn id(&self) -> &'static str {
        IMAGES_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::ImageGeneration
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::ImageGeneration(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("dashscope.images cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.endpoint_url()?;

        // input/parameters wrapper (NOT the OpenAI flat shape).
        let mut parameters = json!({ "n": req.count });
        if let Some(size) = &req.size {
            // DashScope 官方 size 词表用 `1024*1024`（星号分隔）；此处按请求原样
            // 透传 —— 接入时需真实调用校准。
            parameters["size"] = Value::String(size.clone());
        }
        let body = json_request_body(&call.model_params, &req.extra, json!({
            "model": call.model,
            "input": { "prompt": req.prompt },
            "parameters": parameters,
        }))?;

        let resp = send_with_rotation(&call.connection.auth, || {
            Ok(http
                .post(&url)
                .timeout(SUBMIT_TIMEOUT)
                .header(ASYNC_HEADER.0, ASYNC_HEADER.1)
                .json(&body))
        })
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid dashscope submit JSON", &e))?;
        let id = value
            .get("output")
            .and_then(|o| o.get("task_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| InvokeError::parse("dashscope submit response missing output.task_id"))?;
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: IMAGES_ADAPTER_ID.into(),
            config_revision: call.config_revision,
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
        // Platform-unified poller `/api/v1/tasks/{id}` — shared by every
        // DashScope product, independently supplied from the submit endpoint.
        let poll_template = call
            .model_params
            .get("poll_endpoint")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| InvokeError::config("dashscope.images requires an injected poll endpoint"))?;
        let poll_endpoint = expand_protocol_endpoint_template(
            &call.protocol,
            call.task,
            "poll_endpoint",
            poll_template,
            &job.remote_id,
        )?;
        let url = call.credentialed_http_url(
            &resolve_endpoint(&call.connection.base_url, &poll_endpoint),
            "poll_endpoint",
        )?;
        let resp = get_request(http, &url, POLL_TIMEOUT, &call.connection.auth).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid dashscope task JSON", &e))?;

        match parse_task_status(&value)? {
            DashScopeTaskState::Pending => Ok(TaskOutcome::Pending(JobHandle {
                adapter_id: IMAGES_ADAPTER_ID.into(),
                config_revision: call.config_revision,
                remote_id: job.remote_id.clone(),
                poll_state: json!({}),
            })),
            DashScopeTaskState::Failed(msg) => Err(InvokeError::new(InvokeErrorKind::JobFailed, msg)),
            // 24 h provider URLs; the caller fetches them.
            DashScopeTaskState::Done(urls) => Ok(TaskOutcome::Done(TaskResult::Assets(
                urls.into_iter().map(|u| ProducedAsset { data: ProducedData::Url(u), mime: None }).collect(),
            ))),
        }
    }
}

/// The distilled state of a DashScope task.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DashScopeTaskState {
    Pending,
    /// `output.results[].url` of a SUCCEEDED task.
    Done(Vec<String>),
    Failed(String),
}

/// Map a DashScope task body (`output.task_status` in
/// `PENDING/RUNNING/SUCCEEDED/FAILED/CANCELED/UNKNOWN`) to
/// [`DashScopeTaskState`]. A SUCCEEDED task must carry at least one
/// `output.results[].url`; FAILED/CANCELED reports `output.message` |
/// `output.code` | the status itself. Unknown/absent statuses keep waiting.
/// Pure — unit tested.
pub(crate) fn parse_task_status(value: &Value) -> Result<DashScopeTaskState, InvokeError> {
    let output = value.get("output").unwrap_or(&Value::Null);
    let status = output.get("task_status").and_then(|v| v.as_str()).unwrap_or("").to_ascii_uppercase();
    match status.as_str() {
        "SUCCEEDED" => {
            let urls: Vec<String> = output
                .get("results")
                .and_then(|r| r.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("url").and_then(|u| u.as_str()))
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if urls.is_empty() {
                return Err(InvokeError::parse("dashscope task SUCCEEDED but missing output.results[].url"));
            }
            Ok(DashScopeTaskState::Done(urls))
        }
        "FAILED" | "CANCELED" | "CANCELLED" => {
            let msg = output
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| output.get("code").and_then(|c| c.as_str()))
                .unwrap_or(&status)
                .to_string();
            Ok(DashScopeTaskState::Failed(msg))
        }
        // "PENDING" | "RUNNING" | "UNKNOWN" | "" | anything else → keep waiting.
        _ => Ok(DashScopeTaskState::Pending),
    }
}

// ---------------------------------------------------------------------------
// dashscope.embeddings
// ---------------------------------------------------------------------------

/// DashScope synchronous text-embedding (input/parameters wrapper).
pub struct DashScopeEmbeddingsAdapter;

#[async_trait]
impl ProtocolAdapter for DashScopeEmbeddingsAdapter {
    fn id(&self) -> &'static str {
        EMBEDDINGS_ADAPTER_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Embedding
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::Embedding(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("dashscope.embeddings cannot serve task {:?}", call.request.task()),
            ));
        };
        if req.inputs.is_empty() {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "embeddings requires at least one input string",
            ));
        }
        let url = call.endpoint_url()?;

        let mut body = json!({
            "model": call.model,
            "input": { "texts": req.inputs },
        });
        // DashScope embedding knobs — whitelisted passthrough from `extra`
        // under the `parameters` wrapper.
        let mut parameters = serde_json::Map::new();
        for key in ["text_type", "dimension"] {
            if let Some(v) = req.extra.get(key) {
                parameters.insert(key.to_string(), v.clone());
            }
        }
        if !parameters.is_empty() {
            body["parameters"] = Value::Object(parameters);
        }

        let mut raw_extra = req.extra.clone();
        if let Some(raw_extra) = raw_extra.as_object_mut() {
            raw_extra.remove("text_type");
            raw_extra.remove("dimension");
        }
        let body = json_request_body(&call.model_params, &raw_extra, body)?;

        let resp = send_with_rotation(&call.connection.auth, || {
            Ok(http.post(&url).timeout(EMBEDDINGS_TIMEOUT).json(&body))
        })
        .await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid dashscope embeddings JSON", &e))?;
        Ok(TaskOutcome::Done(TaskResult::Embeddings(parse_embeddings_output(&value)?)))
    }
}

/// Parse `output.embeddings[].embedding` into one vector per input, re-ordered
/// by `text_index` when every item declares one (array order otherwise). Pure
/// — unit tested.
pub(crate) fn parse_embeddings_output(value: &Value) -> Result<Vec<Vec<f32>>, InvokeError> {
    let data = value
        .get("output")
        .and_then(|o| o.get("embeddings"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("dashscope embeddings response missing output.embeddings"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("dashscope embeddings response output.embeddings is empty"));
    }
    let mut items: Vec<(Option<u64>, Vec<f32>)> = Vec::with_capacity(data.len());
    for item in data {
        let raw = item
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| InvokeError::parse("dashscope embeddings item missing 'embedding' array"))?;
        let mut vector = Vec::with_capacity(raw.len());
        for n in raw {
            let f = n
                .as_f64()
                .ok_or_else(|| InvokeError::parse("dashscope embedding vector contains a non-numeric element"))?;
            vector.push(f as f32);
        }
        items.push((item.get("text_index").and_then(|v| v.as_u64()), vector));
    }
    if items.iter().all(|(idx, _)| idx.is_some()) {
        items.sort_by_key(|(idx, _)| idx.unwrap_or(u64::MAX));
    }
    Ok(items.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::{EmbedRequest, ImageGenRequest};

    fn dashscope_call_with_endpoint(
        base_url: &str,
        model: &str,
        protocol: &str,
        endpoint: &str,
        request: TaskRequest,
    ) -> ResolvedCall {
        let mut call = call_with_endpoint(base_url, model, protocol, endpoint, request);
        call.platform = "dashscope".into();
        call
    }

    fn image_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let mut call = dashscope_call_with_endpoint(
            base_url,
            model,
            "dashscope.images",
            "/api/v1/services/aigc/text2image/image-synthesis",
            request,
        );
        call.model_params["poll_endpoint"] = Value::String("/api/v1/tasks/{id}".into());
        call
    }

    fn embedding_call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        dashscope_call_with_endpoint(
            base_url,
            model,
            "dashscope.embeddings",
            "/api/v1/services/embeddings/text-embedding/text-embedding",
            request,
        )
    }

    fn image_request(size: Option<&str>, count: u32) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "a fox".into(),
            count,
            size: size.map(str::to_string),
            quality: None,
            extra: json!({}),
        })
    }

    fn embed_request(inputs: &[&str], extra: Value) -> TaskRequest {
        TaskRequest::Embedding(EmbedRequest { inputs: inputs.iter().map(|s| s.to_string()).collect(), extra })
    }

    fn job(remote_id: &str) -> JobHandle {
        JobHandle { adapter_id: IMAGES_ADAPTER_ID.into(), config_revision: 1, remote_id: remote_id.into(), poll_state: json!({}) }
    }

    // -- pure status/parse fixtures --------------------------------------------

    #[test]
    fn task_status_vocabulary_maps_to_states() {
        for s in ["PENDING", "RUNNING", "UNKNOWN", "", "mystery"] {
            assert_eq!(
                parse_task_status(&json!({"output": {"task_status": s}})).unwrap(),
                DashScopeTaskState::Pending,
                "{s}"
            );
        }
        assert_eq!(parse_task_status(&json!({})).unwrap(), DashScopeTaskState::Pending);

        let done = json!({"output": {
            "task_status": "SUCCEEDED",
            "results": [{"url": "https://cdn/a.png"}, {"url": "https://cdn/b.png"}],
        }});
        assert_eq!(
            parse_task_status(&done).unwrap(),
            DashScopeTaskState::Done(vec!["https://cdn/a.png".into(), "https://cdn/b.png".into()])
        );
        // SUCCEEDED without any url is a parse error.
        let bare = json!({"output": {"task_status": "SUCCEEDED", "results": []}});
        assert_eq!(parse_task_status(&bare).unwrap_err().kind, InvokeErrorKind::ParseError);

        // failure message: output.message → output.code → status itself.
        let m = json!({"output": {"task_status": "FAILED", "code": "InvalidParameter", "message": "bad size"}});
        assert_eq!(parse_task_status(&m).unwrap(), DashScopeTaskState::Failed("bad size".into()));
        let c = json!({"output": {"task_status": "CANCELED", "code": "UserCanceled"}});
        assert_eq!(parse_task_status(&c).unwrap(), DashScopeTaskState::Failed("UserCanceled".into()));
        let bare_fail = json!({"output": {"task_status": "FAILED"}});
        assert_eq!(parse_task_status(&bare_fail).unwrap(), DashScopeTaskState::Failed("FAILED".into()));
    }

    #[test]
    fn embeddings_output_parses_and_reorders_by_text_index() {
        let v = json!({"output": {"embeddings": [
            {"text_index": 1, "embedding": [3.0, 4.0]},
            {"text_index": 0, "embedding": [1.0, 2.0]},
        ]}});
        assert_eq!(parse_embeddings_output(&v).unwrap(), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);

        // Without indices: array order.
        let plain = json!({"output": {"embeddings": [{"embedding": [1.0]}, {"embedding": [2.0]}]}});
        assert_eq!(parse_embeddings_output(&plain).unwrap(), vec![vec![1.0], vec![2.0]]);

        for bad in [
            json!({}),
            json!({"output": {}}),
            json!({"output": {"embeddings": []}}),
            json!({"output": {"embeddings": [{}]}}),
            json!({"output": {"embeddings": [{"embedding": [1.0, "x"]}]}}),
        ] {
            assert_eq!(parse_embeddings_output(&bad).unwrap_err().kind, InvokeErrorKind::ParseError, "input {bad}");
        }
    }

    // -- dashscope.images wiremock: full submit → poll chain ---------------------

    #[tokio::test]
    async fn images_submit_sends_async_header_wrapped_body_and_returns_pending() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/aigc/text2image/image-synthesis"))
            .and(header("authorization", "Bearer sk-test"))
            .and(header("X-DashScope-Async", "enable"))
            .and(body_partial_json(json!({
                "model": "wanx-v1",
                "input": {"prompt": "a fox"},
                "parameters": {"size": "1024*1024", "n": 2},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"task_id": "task-1", "task_status": "PENDING"},
                "request_id": "r-1",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "wanx-v1", image_request(Some("1024*1024"), 2));
        let out = DashScopeImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.adapter_id, "dashscope.images");
        assert_eq!(handle.remote_id, "task-1");
    }

    #[tokio::test]
    async fn images_submit_missing_task_id_is_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/aigc/text2image/image-synthesis"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output": {}})))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "wanx-v1", image_request(None, 1));
        let err = DashScopeImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn images_poll_pending_then_succeeded_yields_url_assets() {
        let server = MockServer::start().await;
        // First poll: still running.
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/task-1"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"output": {"task_status": "RUNNING"}})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second poll: succeeded with result urls.
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/task-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {
                    "task_status": "SUCCEEDED",
                    "results": [{"url": "https://cdn/a.png"}],
                }
            })))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "wanx-v1", image_request(None, 1));
        let http = reqwest::Client::new();

        let first = DashScopeImagesAdapter.poll(&http, &call, &job("task-1")).await.unwrap();
        let TaskOutcome::Pending(handle) = first else { panic!("expected Pending on RUNNING") };
        assert_eq!(handle.remote_id, "task-1");

        let second = DashScopeImagesAdapter.poll(&http, &call, &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = second else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Url(u) if u == "https://cdn/a.png"));
        assert_eq!(assets[0].mime, None, "URL asset mime is unknown until fetched");
    }

    #[tokio::test]
    async fn images_poll_failed_is_job_failed_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/task-9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"task_status": "FAILED", "code": "InvalidParameter", "message": "prompt rejected"}
            })))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "wanx-v1", image_request(None, 1));
        let err = DashScopeImagesAdapter.poll(&reqwest::Client::new(), &call, &job("task-9")).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::JobFailed);
        assert_eq!(err.message, "prompt rejected");
    }

    #[tokio::test]
    async fn images_submit_and_poll_use_independent_capability_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/custom/wanx"))
            .and(header("X-DashScope-Async", "enable"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output": {"task_id": "t-c"}})))
            .expect(1)
            .mount(&server)
            .await;
        // Submit and poll are independently supplied by the capability.
        Mock::given(method("GET"))
            .and(path("/api/v1/tasks/t-c"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"task_status": "SUCCEEDED", "results": [{"url": "https://cdn/c.png"}]}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut call = dashscope_call_with_endpoint(
            &server.uri(),
            "wanx-v1",
            "dashscope.images",
            "/custom/wanx",
            image_request(None, 1),
        );
        call.model_params["poll_endpoint"] = Value::String("/api/v1/tasks/{id}".into());
        let http = reqwest::Client::new();
        let out = DashScopeImagesAdapter.submit(&http, &call).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        let done = DashScopeImagesAdapter.poll(&http, &call, &handle).await.unwrap();
        assert!(matches!(done, TaskOutcome::Done(TaskResult::Assets(a)) if a.len() == 1));
    }

    #[tokio::test]
    async fn images_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/aigc/text2image/image-synthesis"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad dashscope key"))
            .mount(&server)
            .await;

        let call = image_call(&server.uri(), "wanx-v1", image_request(None, 1));
        let err = DashScopeImagesAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("bad dashscope key"), "message: {}", err.message);
    }

    // -- dashscope.embeddings wiremock -------------------------------------------

    #[tokio::test]
    async fn embeddings_posts_wrapped_body_and_parses_vectors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/embeddings/text-embedding/text-embedding"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "text-embedding-v2",
                "input": {"texts": ["alpha", "beta"]},
                "parameters": {"text_type": "query"},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"embeddings": [
                    {"text_index": 1, "embedding": [0.5, 0.25]},
                    {"text_index": 0, "embedding": [1.0, 2.0]},
                ]},
                "usage": {"total_tokens": 4},
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = embed_request(&["alpha", "beta"], json!({"text_type": "query", "steps": 3}));
        let call = embedding_call(&server.uri(), "text-embedding-v2", request);
        let out = DashScopeEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Embeddings(vectors)) = out else { panic!("expected Embeddings") };
        // Re-ordered by text_index despite the shuffled response.
        assert_eq!(vectors, vec![vec![1.0, 2.0], vec![0.5, 0.25]]);

        // Non-whitelisted extras must not leak; parameters omitted entirely
        // when no whitelisted knob is present.
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body["parameters"].get("steps").is_none(), "non-whitelisted extra leaked");
    }

    #[tokio::test]
    async fn embeddings_without_knobs_omits_parameters() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/embeddings/text-embedding/text-embedding"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"embeddings": [{"embedding": [1.0]}]}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = embedding_call(&server.uri(), "text-embedding-v2", embed_request(&["x"], json!({})));
        DashScopeEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("parameters").is_none(), "empty parameters must be omitted");
    }

    #[tokio::test]
    async fn embeddings_empty_inputs_are_invalid_params_without_a_request() {
        let call = embedding_call("http://127.0.0.1:9", "text-embedding-v2", embed_request(&[], json!({})));
        let err = DashScopeEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn embeddings_upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/services/embeddings/text-embedding/text-embedding"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = embedding_call(&server.uri(), "text-embedding-v2", embed_request(&["x"], json!({})));
        let err = DashScopeEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
