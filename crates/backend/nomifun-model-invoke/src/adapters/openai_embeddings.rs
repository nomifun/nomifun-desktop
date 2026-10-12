//! `openai.embeddings` — OpenAI-compatible synchronous `/embeddings` (new in
//! the invoke layer; no creation-crate port source).
//!
//! `POST` the dispatch target (conventionally `{base}/v1/embeddings`) with
//! `{model, input: [..]}`; the response `data[].embedding` vectors are
//! returned as [`TaskResult::Embeddings`], ordered by each item's `index`
//! field when present. Compatible responses without indices use array order;
//! partial, duplicate or out-of-range indices fail rather than misassigning
//! vectors to the caller's inputs.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_json};
use crate::types::{TaskOutcome, TaskRequest, TaskResult};

use super::json_request_body;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// OpenAI-compatible sync `/embeddings` protocol.
pub struct OpenAiEmbeddingsAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAiEmbeddingsAdapter {
    fn id(&self) -> &'static str {
        "openai.embeddings"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Embedding
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::Embedding(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.embeddings cannot serve task {:?}", call.request.task()),
            ));
        };
        if req.inputs.is_empty() {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                "embeddings requires at least one input string",
            ));
        }
        let url = call.endpoint_url()?;
        let body = json_request_body(
            &call.model_params,
            &req.extra,
            json!({ "model": call.model, "input": req.inputs }),
        )?;

        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid embeddings JSON", &e))?;
        let vectors = parse_embeddings_response(&value)?;
        if vectors.len() != req.inputs.len() {
            return Err(InvokeError::parse(format!(
                "embeddings response returned {} vectors for {} inputs",
                vectors.len(), req.inputs.len()
            )));
        }
        Ok(TaskOutcome::Done(TaskResult::Embeddings(vectors)))
    }
}

/// Parse an OpenAI embeddings response body
/// (`{ data: [ { embedding: [..], index? } ] }`) into one vector per input.
/// Indices must either be absent from every item or cover each input once.
/// Array order is used only for entirely unindexed compatible responses.
pub(crate) fn parse_embeddings_response(value: &Value) -> Result<Vec<Vec<f32>>, InvokeError> {
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("embeddings response missing 'data' array"))?;
    if data.is_empty() {
        return Err(InvokeError::parse("embeddings response 'data' array is empty"));
    }
    let mut items: Vec<(Option<u64>, Vec<f32>)> = Vec::with_capacity(data.len());
    for item in data {
        let raw = item
            .get("embedding")
            .and_then(|v| v.as_array())
            .ok_or_else(|| InvokeError::parse("embeddings data item missing 'embedding' array"))?;
        let mut vector = Vec::with_capacity(raw.len());
        for n in raw {
            let f = n
                .as_f64()
                .ok_or_else(|| InvokeError::parse("embedding vector contains a non-numeric element"))?;
            vector.push(f as f32);
        }
        let index = item.get("index").map(|index| {
            index.as_u64().ok_or_else(|| InvokeError::parse(
                "embedding data item has an invalid 'index'"
            ))
        }).transpose()?;
        items.push((index, vector));
    }
    if items.iter().any(|(idx, _)| idx.is_some()) {
        if items.iter().any(|(idx, _)| idx.is_none()) {
            return Err(InvokeError::parse("embeddings response contains partial indices"));
        }
        items.sort_by_key(|(idx, _)| idx.unwrap_or(u64::MAX));
        if items.iter().enumerate().any(|(expected, (index, _))| *index != Some(expected as u64)) {
            return Err(InvokeError::parse(
                "embeddings response indices must cover every input once without gaps or duplicates"
            ));
        }
    }
    Ok(items.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;
    use crate::types::EmbedRequest;

    fn call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        let base_url = format!("{}/v1", base_url.trim_end_matches('/'));
        call_with_endpoint(&base_url, model, "openai.embeddings", "/embeddings", request)
    }

    fn embed_request(inputs: &[&str]) -> TaskRequest {
        TaskRequest::Embedding(EmbedRequest {
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            extra: json!({}),
        })
    }

    // -- pure-parser fixtures -------------------------------------------------

    #[test]
    fn parse_keeps_array_order_without_indices() {
        let v = json!({"data": [
            {"embedding": [1.0, 2.0]},
            {"embedding": [3.0, 4.0]},
        ]});
        assert_eq!(parse_embeddings_response(&v).unwrap(), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    }

    #[test]
    fn parse_reorders_by_index_when_all_present() {
        let v = json!({"data": [
            {"index": 1, "embedding": [3.0, 4.0]},
            {"index": 0, "embedding": [1.0, 2.0]},
        ]});
        assert_eq!(parse_embeddings_response(&v).unwrap(), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    }

    #[test]
    fn parse_rejects_ambiguous_or_invalid_indices() {
        for data in [
            json!([{"index": 1, "embedding": [3.0]}, {"embedding": [1.0]}]),
            json!([{"index": 0, "embedding": [3.0]}, {"index": 0, "embedding": [1.0]}]),
            json!([{"index": 1, "embedding": [3.0]}, {"index": 2, "embedding": [1.0]}]),
            json!([{"index": -1, "embedding": [3.0]}]),
            json!([{"index": "0", "embedding": [3.0]}]),
            json!([{"index": null, "embedding": [3.0]}]),
        ] {
            let error = parse_embeddings_response(&json!({"data": data})).unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::ParseError);
        }
    }

    #[test]
    fn parse_errors_on_missing_or_malformed() {
        for bad in [
            json!({}),
            json!({"data": []}),
            json!({"data": [{}]}),
            json!({"data": [{"embedding": "not an array"}]}),
            json!({"data": [{"embedding": [1.0, "x"]}]}),
        ] {
            let err = parse_embeddings_response(&bad).unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::ParseError, "input {bad}");
        }
    }

    // -- wiremock request/response tests ------------------------------------

    #[tokio::test]
    async fn embeddings_posts_inputs_and_parses_vectors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "text-embedding-3-small",
                "input": ["alpha", "beta"],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": [
                    {"object": "embedding", "index": 1, "embedding": [0.5, 0.25]},
                    {"object": "embedding", "index": 0, "embedding": [1.0, 2.0]},
                ],
                "model": "text-embedding-3-small",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let call = call(&server.uri(), "text-embedding-3-small", embed_request(&["alpha", "beta"]));
        let out = OpenAiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Embeddings(vectors)) = out else {
            panic!("expected Done(Embeddings)")
        };
        // Re-ordered by index despite the shuffled response.
        assert_eq!(vectors, vec![vec![1.0, 2.0], vec![0.5, 0.25]]);
    }

    #[tokio::test]
    async fn empty_inputs_are_invalid_params_without_a_request() {
        let call = call("http://127.0.0.1:9", "text-embedding-3-small", embed_request(&[]));
        let err = OpenAiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn embeddings_reject_misassigned_vectors_and_wrong_batch_size_on_the_wire() {
        let server = MockServer::start().await;
        for data in [
            json!([{"index": 1, "embedding": [3.0]}, {"embedding": [1.0]}]),
            json!([{"index": 0, "embedding": [3.0]}, {"index": 0, "embedding": [1.0]}]),
            json!([{"index": 0, "embedding": [3.0]}]),
            json!([{"index": 0, "embedding": [1.0]}, {"index": 1, "embedding": [2.0]}, {"index": 2, "embedding": [3.0]}]),
        ] {
            server.reset().await;
            Mock::given(method("POST"))
                .and(path("/v1/embeddings"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": data})))
                .expect(1)
                .mount(&server)
                .await;
            let call = call(&server.uri(), "gateway-embedding", embed_request(&["alpha", "beta"]));
            let error = OpenAiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::ParseError);
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let call = call(&server.uri(), "text-embedding-3-small", embed_request(&["x"]));
        let err = OpenAiEmbeddingsAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
    }
}
