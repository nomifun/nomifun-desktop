//! InfiniAI GenStudio's native string-document rerank contract.
//! https://docs.infini-ai.com/openapi/maas-v1.openapi.yaml
//! The provider does not accept top_n. Rank every input once, then apply the
//! caller's requested limit locally instead of sending an unsupported field.

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use super::json_request_body;
use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_json, read_json_capped, response_secret_redactor};
use crate::types::{RerankRequest, RerankResult, TaskOutcome, TaskRequest, TaskResult};

const PROTOCOL_ID: &str = "infiniai.rerank";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
// Application transport ceilings, not guesses at the provider's token budget.
const MAX_DOCUMENTS: usize = 1024;
const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

pub struct InfiniAiRerankAdapter;

#[async_trait]
impl ProtocolAdapter for InfiniAiRerankAdapter {
    fn id(&self) -> &'static str {
        PROTOCOL_ID
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Rerank
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::Rerank(request) = &call.request else {
            return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "infiniai.rerank requires a rerank request"));
        };
        let (body, top_n) = request_body(call, request)?;
        let response = post_json(http, &call.endpoint_url()?, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !response.status().is_success() {
            return Err(error_from_response(response).await);
        }
        let redactor = response_secret_redactor(&response);
        let value: Value = read_json_capped(response, MAX_RESPONSE_BYTES, "InfiniAI rerank").await
            .map_err(|error| error.redacted(&redactor))?;
        let results = parse_results(&value, &request.documents, top_n)?;
        Ok(TaskOutcome::Done(TaskResult::Reranked(results)))
    }
}

fn invalid(message: &'static str) -> InvokeError {
    InvokeError::new(InvokeErrorKind::InvalidParams, message)
}

fn request_body(call: &ResolvedCall, request: &RerankRequest) -> Result<(Value, Option<usize>), InvokeError> {
    if call.model.trim().is_empty() || request.query.trim().is_empty() {
        return Err(invalid("InfiniAI rerank requires a model and non-empty query"));
    }
    if request.documents.is_empty() || request.documents.len() > MAX_DOCUMENTS {
        return Err(invalid("InfiniAI rerank requires 1-1024 documents per request"));
    }
    let input_bytes = request.documents.iter().try_fold(request.query.len().saturating_add(call.model.len()), |size, document| {
        size.checked_add(document.len())
    }).ok_or_else(|| invalid("InfiniAI rerank request exceeds the local byte limit"))?;
    if input_bytes > MAX_REQUEST_BYTES {
        return Err(invalid("InfiniAI rerank request exceeds the local byte limit"));
    }
    let mut body = json_request_body(&call.model_params, &request.extra, json!({
        "model": call.model, "query": request.query, "documents": request.documents,
    }))?;
    let fields = body.as_object_mut().expect("shared JSON request body is an object");
    let configured_top_n = fields.remove("top_n");
    let top_n = match request.top_n {
        Some(0) => return Err(invalid("InfiniAI rerank top_n must be positive")),
        Some(value) => Some(value as usize),
        None => match configured_top_n {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().filter(|value| *value > 0 && *value <= u32::MAX as u64)
                .ok_or_else(|| invalid("InfiniAI rerank top_n must be a positive integer"))? as usize),
        },
    };
    if fields.keys().any(|key| !matches!(key.as_str(), "model" | "query" | "documents" | "return_documents")) {
        return Err(invalid("InfiniAI rerank accepts only model/query/documents/return_documents; top_n is applied locally"));
    }
    if fields.get("return_documents").is_some_and(|value| !value.is_boolean()) {
        return Err(invalid("InfiniAI rerank return_documents must be a boolean"));
    }
    let encoded = serde_json::to_vec(&body).map_err(|_| invalid("InfiniAI rerank request cannot be serialized"))?;
    if encoded.len() > MAX_REQUEST_BYTES {
        return Err(invalid("InfiniAI rerank encoded request exceeds the local byte limit"));
    }
    Ok((body, top_n))
}

fn relevance_score(item: &Value) -> Result<f64, InvokeError> {
    // The official OpenAPI schema spells this relavance_score, while its
    // successful response example uses relevance_score. Accept both proven
    // spellings, but never resolve contradictory values by field order.
    let canonical = item.get("relevance_score");
    let documented_alias = item.get("relavance_score");
    let number = |value: &Value| value.as_f64().filter(|score| score.is_finite() && (*score as f32).is_finite());
    let score = canonical.or(documented_alias).and_then(number)
        .ok_or_else(|| InvokeError::parse("InfiniAI rerank result requires a finite relevance score"))?;
    if let (Some(canonical), Some(alias)) = (canonical, documented_alias) {
        if number(canonical) != number(alias) {
            return Err(InvokeError::parse("InfiniAI rerank result contains conflicting relevance scores"));
        }
    }
    Ok(score)
}

fn parse_results(value: &Value, documents: &[String], top_n: Option<usize>) -> Result<Vec<RerankResult>, InvokeError> {
    let results = value.get("results").and_then(Value::as_array)
        .ok_or_else(|| InvokeError::parse("InfiniAI rerank response requires a results array"))?;
    // Without an upstream top_n, the documented API scores every input. A
    // partial set cannot establish which documents belong in the local top N.
    if results.is_empty() || results.len() != documents.len() || results.len() > MAX_DOCUMENTS {
        return Err(InvokeError::parse("InfiniAI rerank response must score every input document exactly once"));
    }
    let mut seen = vec![false; documents.len()];
    let mut ranked = Vec::with_capacity(results.len());
    for item in results {
        let index = item.get("index").and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok()).filter(|index| *index < documents.len())
            .ok_or_else(|| InvokeError::parse("InfiniAI rerank result has an invalid document index"))?;
        if std::mem::replace(&mut seen[index], true) {
            return Err(InvokeError::parse("InfiniAI rerank response contains a duplicate document index"));
        }
        let score = relevance_score(item)?;
        let document = match item.get("document") {
            None | Some(Value::Null) => None,
            Some(document) => {
                let text = document.as_object().and_then(|document| document.get("text")).and_then(Value::as_str)
                    .filter(|text| *text == documents[index])
                    .ok_or_else(|| InvokeError::parse("InfiniAI rerank returned document does not match its input index"))?;
                Some(text.to_owned())
            }
        };
        ranked.push((score, RerankResult { index, relevance_score: score as f32, document }));
    }
    // Keep full wire precision when selecting top N; normalize to the shared
    // f32 result type only after the ordering decision. Ties use input order.
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0).then_with(|| left.1.index.cmp(&right.1.index)));
    if let Some(limit) = top_n { ranked.truncate(limit); }
    Ok(ranked.into_iter().map(|(_, result)| result).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use crate::adapters::test_support::call_with_endpoint;

    fn request() -> RerankRequest {
        RerankRequest { query: "fruit".into(), documents: vec!["apple".into(), "stone".into(), "pear".into()], top_n: Some(2), extra: json!({}) }
    }

    fn call(base: &str, request: RerankRequest) -> ResolvedCall {
        call_with_endpoint(base, "bge-reranker-v2-m3", PROTOCOL_ID, "/rerank", TaskRequest::Rerank(request))
    }

    fn results() -> Value {
        json!({"results":[{"index":1,"relevance_score":0.2},{"index":2,"relevance_score":0.9},{"index":0,"relevance_score":0.8}]})
    }

    fn client() -> reqwest::Client { reqwest::Client::builder().no_proxy().build().unwrap() }

    #[tokio::test]
    async fn native_request_omits_top_n_and_returns_the_best_results_in_score_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/maas/v1/rerank")).and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[
                {"index":1,"relevance_score":0.2,"document":{"text":"stone"}},
                {"index":2,"relevance_score":0.9,"document":{"text":"pear"}},
                {"index":0,"relevance_score":0.8,"document":{"text":"apple"}}
            ]}))).expect(1).mount(&server).await;
        let mut request = request(); request.extra = json!({"return_documents":true,"top_n":3});
        let mut call = call(&format!("{}/maas/v1",server.uri()),request);
        call.model_params = json!({"endpoint":"/rerank","top_n":1,"model":"wrong","query":"wrong","documents":["wrong"],
            "api_key":"must-not-leak","headers":{"authorization":"wrong"},"request_defaults":{"secret":"wrong"}});
        let TaskOutcome::Done(TaskResult::Reranked(ranked)) = InfiniAiRerankAdapter.submit(&client(),&call).await.unwrap() else {panic!("expected reranked results")};
        assert_eq!(ranked.iter().map(|result|result.index).collect::<Vec<_>>(),vec![2,0]);
        assert_eq!(ranked[0].document.as_deref(),Some("pear"));
        let sent:Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(sent,json!({"model":"bge-reranker-v2-m3","query":"fruit","documents":["apple","stone","pear"],"return_documents":true}));
    }

    #[tokio::test]
    async fn configured_local_limit_and_native_boolean_respect_request_precedence() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(results())).expect(1).mount(&server).await;
        let mut request = request(); request.top_n=None; request.extra=json!({"top_n":1,"return_documents":false});
        let mut call = call(&server.uri(),request); call.model_params=json!({"endpoint":"/rerank","top_n":2,"return_documents":true});
        let TaskOutcome::Done(TaskResult::Reranked(ranked)) = InfiniAiRerankAdapter.submit(&client(),&call).await.unwrap() else {panic!("expected reranked results")};
        assert_eq!(ranked.len(),1); assert_eq!(ranked[0].index,2);
        let sent:Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert!(sent.get("top_n").is_none()); assert_eq!(sent["return_documents"],false);
    }

    #[tokio::test]
    async fn documented_score_alias_and_wire_precision_do_not_change_top_n_selection() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[
            {"index":2,"relevance_score":0.900000002},
            {"index":0,"relevance_score":0.900000001},
            {"index":1,"relavance_score":0.900000002}
        ]}))).expect(1).mount(&server).await;
        let TaskOutcome::Done(TaskResult::Reranked(ranked)) = InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request())).await.unwrap() else {panic!("expected reranked results")};
        assert_eq!(ranked.iter().map(|result|result.index).collect::<Vec<_>>(),vec![1,2]);
    }

    #[tokio::test]
    async fn malformed_or_partial_rankings_never_become_successful_top_n_results() {
        let mut duplicate = results(); duplicate["results"][0]["index"]=json!(0);
        let mut out_of_range = results(); out_of_range["results"][0]["index"]=json!(3);
        let mut non_integer = results(); non_integer["results"][0]["index"]=json!(0.5);
        let mut overflow = results(); overflow["results"][0]["relevance_score"]=json!(1e300);
        let mut conflict = results(); conflict["results"][0]["relavance_score"]=json!(0.7);
        let mut wrong_document = results(); wrong_document["results"][0]["document"]=json!({"text":"not the indexed input"});
        for reply in [json!({}),json!({"results":[]}),json!({"results":[{"index":0,"relevance_score":0.9}]}),
            duplicate,out_of_range,non_integer,overflow,conflict,wrong_document] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(reply)).expect(1).mount(&server).await;
            let error = InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request())).await.unwrap_err();
            assert_eq!(error.kind,InvokeErrorKind::ParseError);
        }
    }

    #[tokio::test]
    async fn unsupported_parameters_and_local_request_bounds_fail_before_http() {
        let server = MockServer::start().await;
        for extra in [json!({"rank_fields":["text"]}),json!({"truncate":false}),json!({"truncate_direction":"END"}),
            json!({"return_documents":"true"}),json!({"top_n":0}),json!({"top_n":1.5})] {
            let mut request = request(); request.top_n=None; request.extra=extra;
            let error = InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request)).await.unwrap_err();
            assert_eq!(error.kind,InvokeErrorKind::InvalidParams);
        }
        let mut cases = Vec::new();
        let mut empty = request(); empty.documents.clear(); cases.push(empty);
        let mut too_many = request(); too_many.documents=vec!["a".into();MAX_DOCUMENTS+1]; cases.push(too_many);
        let mut oversized = request(); oversized.query="x".repeat(MAX_REQUEST_BYTES+1); cases.push(oversized);
        let mut zero = request(); zero.top_n=Some(0); cases.push(zero);
        for request in cases {
            assert_eq!(InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request)).await.unwrap_err().kind,InvokeErrorKind::InvalidParams);
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn authentication_and_json_failures_preserve_error_kinds_without_leaking_credentials() {
        for (status,body,kind) in [(401,"{\"error\":{\"message\":\"rejected sk-test\"}}",InvokeErrorKind::Auth),
            (200,"{\"results\": sk-test}",InvokeErrorKind::ParseError)] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status).set_body_raw(body,"application/json")).expect(1).mount(&server).await;
            let error = InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request())).await.unwrap_err();
            assert_eq!(error.kind,kind); assert!(!error.message.contains("sk-test"));
        }
    }

    #[tokio::test]
    async fn response_bytes_are_capped_before_parsing() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_raw(" ".repeat(MAX_RESPONSE_BYTES as usize+1),"application/json"))
            .expect(1).mount(&server).await;
        let error = InfiniAiRerankAdapter.submit(&client(),&call(&server.uri(),request())).await.unwrap_err();
        assert_eq!(error.kind,InvokeErrorKind::ProviderError);
        assert!(error.message.contains("cap"));
    }
}
