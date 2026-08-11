//! `generic.rerank` — the common JSON `/rerank` protocol used by several
//! OpenAI-compatible gateways even though OpenAI itself has no rerank API.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Map, Value, json};

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

        let url = call.dispatch_target().url;
        let body = build_body(&call.model, &call.model_params, req);
        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value = resp.json().await.map_err(|e| InvokeError::parse(format!("invalid rerank JSON: {e}")))?;
        Ok(TaskOutcome::Done(TaskResult::Reranked(parse_results(&value)?)))
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
    Ok(())
}

fn merge_options(body: &mut Map<String, Value>, source: &Value) {
    let Some(options) = source.as_object() else { return };
    for (key, value) in options {
        // Routing metadata is local-only; core typed fields cannot be replaced
        // by an opaque extra object.
        if matches!(
            key.as_str(),
            "endpoint" | "poll_endpoint" | "protocol" | "model" | "query" | "documents" | "top_n"
        ) || value.is_null()
        {
            continue;
        }
        body.insert(key.clone(), value.clone());
    }
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

fn parse_results(value: &Value) -> Result<Vec<RerankResult>, InvokeError> {
    let raw = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("rerank response missing 'results' array"))?;
    if raw.is_empty() {
        return Err(InvokeError::parse("rerank response 'results' array is empty"));
    }

    raw.iter()
        .map(|item| {
            let index = item
                .get("index")
                .and_then(Value::as_u64)
                .ok_or_else(|| InvokeError::parse("rerank result missing numeric 'index'"))?
                as usize;
            let relevance_score = item
                .get("relevance_score")
                .or_else(|| item.get("score"))
                .and_then(Value::as_f64)
                .ok_or_else(|| InvokeError::parse("rerank result missing numeric relevance score"))?
                as f32;
            let document = match item.get("document") {
                Some(Value::String(text)) => Some(text.clone()),
                Some(Value::Object(document)) => document.get("text").and_then(Value::as_str).map(str::to_owned),
                _ => None,
            };
            Ok(RerankResult { index, relevance_score, document })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;

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
        ]}))
        .unwrap();
        assert_eq!(results[0], RerankResult { index: 0, relevance_score: 0.9, document: Some("apple".into()) });
        assert_eq!(results[1], RerankResult { index: 1, relevance_score: 0.1, document: Some("stone".into()) });
    }

    #[test]
    fn parser_rejects_malformed_responses() {
        for value in [json!({}), json!({"results": []}), json!({"results": [{}]})] {
            assert_eq!(parse_results(&value).unwrap_err().kind, InvokeErrorKind::ParseError);
        }
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
}
