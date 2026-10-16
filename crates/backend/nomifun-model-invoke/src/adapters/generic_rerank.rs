//! `generic.rerank` — the common JSON `/rerank` protocol used by several
//! OpenAI-compatible gateways even though OpenAI itself has no rerank API.

use std::{collections::BTreeSet, time::Duration};

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

use super::provider_body_fields;
use super::retrieval_results::{finite_f32, result_index};
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_json};
use crate::types::{RerankRequest, RerankResult, TaskOutcome, TaskRequest, TaskResult};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Synchronous JSON `/rerank` adapter shared by verified compatible providers.
pub struct GenericRerankAdapter;

#[async_trait]
impl ProtocolAdapter for GenericRerankAdapter {
    fn id(&self) -> &'static str {
        "generic.rerank"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Rerank
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::Rerank(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("generic.rerank cannot serve task {:?}", call.request.task()),
            ));
        };
        validate_request(req)?;

        let url = call.endpoint_url()?;
        let body = build_body(&call.model, &call.model_params, req);
        let max_results = body.get("top_n").and_then(Value::as_u64)
            .and_then(|limit| usize::try_from(limit).ok()).unwrap_or(req.documents.len()).min(req.documents.len());
        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| InvokeError::response_json("invalid rerank JSON", &e))?;
        Ok(TaskOutcome::Done(TaskResult::Reranked(parse_results(&value, req.documents.len(), max_results)?)))
    }
}

fn validate_request(req: &RerankRequest) -> Result<(), InvokeError> {
    if req.query.trim().is_empty() {
        return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "rerank query must not be empty"));
    }
    if req.documents.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "rerank requires at least one document",
        ));
    }
    if req.top_n == Some(0) {
        return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "rerank top_n must be positive"));
    }
    Ok(())
}

fn merge_option_object(body: &mut Map<String, Value>, source: &Value) {
    for (key, value) in provider_body_fields(source) {
        // Core typed fields cannot be replaced by opaque options. Local
        // routing/auth metadata has already been removed by the shared helper.
        if matches!(
            key.as_str(),
            "model" | "query" | "documents" | "top_n"
        ) {
            continue;
        }
        body.insert(key.clone(), value.clone());
    }
}

fn merge_options(body: &mut Map<String, Value>, source: &Value) {
    merge_option_object(body, source);
}

fn build_body(model: &str, model_params: &Value, req: &RerankRequest) -> Value {
    let mut body = Map::new();
    merge_options(&mut body, model_params);
    merge_options(&mut body, &req.extra);
    body.insert("model".into(), json!(model));
    body.insert("query".into(), json!(req.query));
    body.insert("documents".into(), json!(req.documents));
    if let Some(top_n) = req.top_n {
        body.insert("top_n".into(), json!(top_n));
    }
    Value::Object(body)
}

fn parse_results(value: &Value, document_count: usize, max_results: usize) -> Result<Vec<RerankResult>, InvokeError> {
    let raw = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("rerank response missing 'results' array"))?;
    if raw.is_empty() {
        return Err(InvokeError::parse("rerank response 'results' array is empty"));
    }
    if raw.len() > document_count || raw.len() > max_results {
        return Err(InvokeError::parse("rerank result count exceeds the input documents or requested top_n"));
    }
    let mut seen = BTreeSet::new();
    let mut results = Vec::with_capacity(raw.len());
    for item in raw {
        let index = result_index(item.get("index").ok_or_else(|| InvokeError::parse("rerank result is missing its input index"))?,
            document_count, &mut seen, "rerank response")?;
        let score = item.get("relevance_score").or_else(|| item.get("score"))
            .ok_or_else(|| InvokeError::parse("rerank result is missing its relevance score"))?;
        let relevance_score = finite_f32(score, "rerank relevance score")?;
        if let (Some(canonical), Some(alias)) = (item.get("relevance_score"), item.get("score")) {
            finite_f32(alias, "rerank score alias")?;
            if canonical.as_f64() != alias.as_f64() {
                return Err(InvokeError::parse("rerank result contains conflicting relevance scores"));
            }
        }
        let document = match item.get("document") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Object(document)) => Some(document.get("text").and_then(Value::as_str)
                .ok_or_else(|| InvokeError::parse("rerank returned document requires string text"))?.to_owned()),
            Some(_) => return Err(InvokeError::parse("rerank returned document has an unsupported shape")),
        };
        results.push(RerankResult { index, relevance_score, document });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call_with_endpoint;

    fn call(base_url: &str, model: &str, request: TaskRequest) -> ResolvedCall {
        call_with_endpoint(base_url, model, "generic.rerank", "/rerank", request)
    }

    fn request() -> TaskRequest {
        TaskRequest::Rerank(RerankRequest {
            query: "best fruit".into(),
            documents: vec!["apple".into(), "stone".into()],
            top_n: Some(1),
            extra: json!({"return_documents": true}),
        })
    }

    #[test]
    fn parser_accepts_both_common_score_and_document_shapes() {
        let results = parse_results(&json!({"results": [
            {"index": 0, "relevance_score": 0.9, "document": {"text": "apple"}},
            {"index": 1, "score": 0.1, "document": "stone"}
        ]}), 2, 2)
        .unwrap();
        assert_eq!(results[0], RerankResult { index: 0, relevance_score: 0.9, document: Some("apple".into()) });
        assert_eq!(results[1], RerankResult { index: 1, relevance_score: 0.1, document: Some("stone".into()) });
    }

    #[test]
    fn parser_rejects_malformed_responses() {
        for value in [json!({}), json!({"results": []}), json!({"results": [{}]})] {
            assert_eq!(parse_results(&value, 2, 2).unwrap_err().kind, InvokeErrorKind::ParseError);
        }
    }

    #[test]
    fn parser_preserves_zero_negative_scores_and_legitimate_filtered_top_n_results() {
        let value = json!({"results":[{"index":1,"relevance_score":0.0},{"index":0,"score":-7.5}]});
        let results = parse_results(&value, 3, 3).unwrap();
        assert_eq!(results.iter().map(|result| result.index).collect::<Vec<_>>(), vec![1,0]);
        assert_eq!(results[0].relevance_score, 0.0);
        assert_eq!(results[1].relevance_score, -7.5);
        assert!(parse_results(&json!({"results":[{"index":2,"score":1.5}]}), 3, 2).is_ok(),
            "top_n is an upper bound, not a promise to return exactly N results");
    }

    #[test]
    fn transport_metadata_never_enters_rerank_body() {
        let params = json!({
            "base_url": "https://other.example/v1",
            "allow_cross_origin_credentials": true,
            "endpoint": "/rerank",
            "poll_endpoint": "/jobs/{id}",
            "content_endpoint": "/jobs/{id}/content",
            "realtime_endpoint": "wss://socket.example/realtime",
            "protocol": "generic.rerank",
            "connection": {"role": "voice"},
            "connection_id": "connection-secret-id",
            "connection_role": "voice",
            "auth": {"token": "secret"},
            "auth_scheme": "bearer",
            "credentials": {"api_keys": ["secret"]},
            "api_key": "secret",
            "api_keys": ["secret"],
            "headers": {"x-secret": "secret"},
            "temperature": 0.2,
            "top_p": 0.9,
            "max_chunks_per_doc": 4
        });
        let req = RerankRequest {
            query: "best fruit".into(),
            documents: vec!["apple".into()],
            top_n: Some(1),
            extra: json!({
                "base_url": "https://request.example/v1",
                "connection_role": "request-role",
                "return_documents": true
            }),
        };

        let body = build_body("reranker", &params, &req);
        let object = body.as_object().unwrap();
        for local_key in [
            "base_url",
            "allow_cross_origin_credentials",
            "endpoint",
            "poll_endpoint",
            "content_endpoint",
            "realtime_endpoint",
            "protocol",
            "connection",
            "connection_id",
            "connection_role",
            "auth",
            "auth_scheme",
            "credentials",
            "api_key",
            "api_keys",
            "headers",
        ] {
            assert!(!object.contains_key(local_key), "local key {local_key} leaked into request body: {body}");
        }
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["max_chunks_per_doc"], 4);
        assert_eq!(body["return_documents"], true);
        assert_eq!(body["model"], "reranker");
        assert!(!serde_json::to_string(&body).unwrap().contains("secret"));
    }

    #[tokio::test]
    async fn posts_rerank_json_and_normalizes_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/rerank"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "bge-reranker-v2-m3",
                "query": "best fruit",
                "documents": ["apple", "stone"],
                "top_n": 1,
                "return_documents": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": [{"index": 0, "relevance_score": 0.99, "document": {"text": "apple"}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base_url = format!("{}/v1", server.uri());
        let call = call(&base_url, "bge-reranker-v2-m3", request());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let output = GenericRerankAdapter.submit(&client, &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Reranked(results)) = output else { panic!("expected rerank results") };
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].index, 0);
    }

    #[tokio::test]
    async fn rejects_empty_documents_locally() {
        let req = RerankRequest { query: "q".into(), documents: vec![], top_n: None, extra: json!({}) };
        let call = call("http://127.0.0.1:9", "reranker", TaskRequest::Rerank(req));
        let err = GenericRerankAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::InvalidParams);
    }

    #[tokio::test]
    async fn invalid_indices_scores_and_document_shapes_never_become_successful_rankings() {
        let server = MockServer::start().await;
        let mut replies: Vec<String> = [
            json!({"results":[]}),
            json!({"results":[{"index":0,"score":0.9},{"index":0,"score":0.8}]}),
            json!({"results":[{"index":2,"score":0.9}]}),
            json!({"results":[{"index":-1,"score":0.9}]}),
            json!({"results":[{"index":0.5,"score":0.9}]}),
            json!({"results":[{"index":"0","score":0.9}]}),
            json!({"results":[{"index":0,"score":null}]}),
            json!({"results":[{"index":0,"score":1e100}]}),
            json!({"results":[{"index":0,"relevance_score":0.8,"score":0.9}]}),
            json!({"results":[{"index":0,"score":0.9,"document":{}}]}),
            json!({"results":[{"index":0,"score":0.9,"document":7}]}),
        ].into_iter().map(|value| value.to_string()).collect();
        replies.push(r#"{"results":[{"index":0,"score":NaN}]}"#.to_owned());
        for reply in replies {
            server.reset().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(reply, "application/json"))
                .expect(1).mount(&server).await;
            let call = call(&server.uri(), "reranker", TaskRequest::Rerank(RerankRequest {
                query: "query".into(), documents: vec!["one".into(),"two".into()], top_n: None, extra:json!({}),
            }));
            let error = GenericRerankAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::ParseError);
        }
    }

    #[tokio::test]
    async fn response_count_obeys_top_n_without_truncating_excess_rows() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(body_partial_json(json!({"top_n":1})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[{"index":0,"score":0},{"index":1,"score":-1}]})))
            .expect(1).mount(&server).await;
        let error = GenericRerankAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), "reranker", request())).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn zero_top_n_is_rejected_before_http() {
        let server = MockServer::start().await;
        let mut request = request();
        let TaskRequest::Rerank(request_body) = &mut request else { unreachable!() };
        request_body.top_n = Some(0);
        let error = GenericRerankAdapter.submit(&reqwest::Client::new(), &call(&server.uri(), "reranker", request)).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::InvalidParams);
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
