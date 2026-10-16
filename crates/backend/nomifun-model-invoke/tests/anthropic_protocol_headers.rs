//! Exercise the production broker encoder/decoder and actual HTTP executor.
//! A permissive fake ProviderTransport would miss mandatory protocol headers.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures_util::StreamExt;
use nomifun_chat_model_broker::{
    AnthropicAdapter, ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError,
    ChatModelEvent, ChatProtocol, ChatProtocolAdapter, ChatRole, ChatToolResultPart,
    CredentialLease, CredentialTarget, PromptCachePolicy, ProviderTransport, ProviderWireFrame,
    ProviderWireRequest, ProviderWireStream, recorded_conformance_fixtures,
};
use nomifun_model_invoke::{
    AuthMaterial, AuthScheme, InvokeError, OpaqueCredentialLease, OpaqueCredentialResolver,
    SingleAttemptFrame, SingleAttemptFraming, SingleAttemptHttpExecutor, SingleAttemptRequest,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ENDPOINT: &str = "/relay/anthropic/v1/messages";

struct FixtureCredentials(AuthScheme);

#[async_trait]
impl OpaqueCredentialResolver for FixtureCredentials {
    async fn resolve(&self, lease: &OpaqueCredentialLease) -> Result<AuthMaterial, InvokeError> {
        assert_eq!(lease.handle(), "fixture-lease");
        Ok(AuthMaterial { scheme: self.0.clone(), credentials: json!({"api_keys":["fixture-secret"]}) })
    }
}

struct EncodingOnlyTransport;

#[async_trait]
impl ProviderTransport for EncodingOnlyTransport {
    async fn open_stream(&self, _: ProviderWireRequest, _: CredentialLease) -> Result<ProviderWireStream, ChatModelError> {
        panic!("the integration test must use the actual HTTP executor");
    }
}

fn executor(scheme: AuthScheme) -> SingleAttemptHttpExecutor {
    SingleAttemptHttpExecutor::new(reqwest::Client::builder().no_proxy().build().unwrap(), Arc::new(FixtureCredentials(scheme)))
}

async fn send(executor: &SingleAttemptHttpExecutor, protocol: &str, url: String, body: Value) -> Vec<SingleAttemptFrame> {
    executor.open_stream(SingleAttemptRequest {
        protocol: protocol.to_owned(), url, model: "claude-recorded".to_owned(), body,
        credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
        timeout: Duration::from_secs(5), idle_timeout: Duration::from_secs(5),
        framing: SingleAttemptFraming::Sse, region: None,
    }).await.unwrap().collect::<Vec<_>>().await.into_iter().map(Result::unwrap).collect()
}

fn decode(adapter: &AnthropicAdapter, frames: Vec<SingleAttemptFrame>) -> Vec<ChatModelEvent> {
    let mut decoder = adapter.new_frame_decoder().expect("production stateful Messages decoder");
    frames.into_iter().flat_map(|frame| decoder.decode_frame(ProviderWireFrame {
        event: frame.event, data: frame.data, diagnostic: frame.diagnostic,
    }).unwrap()).collect()
}

fn sse(events: Vec<Value>) -> String {
    events.into_iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect()
}

fn message_start(id: &str) -> Value {
    json!({"type":"message_start","message":{"id":id,"type":"message","role":"assistant","content":[],
        "stop_reason":null,"usage":{"input_tokens":10,"output_tokens":0}}})
}

fn message_end(reason: &str) -> Vec<Value> {
    vec![json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},"usage":{"output_tokens":12}}),
        json!({"type":"message_stop"})]
}

#[tokio::test]
async fn broker_messages_and_tool_continuation_both_send_the_required_version_and_auth() {
    let server = MockServer::start().await;
    // No successful fixture matches unless both protocol and credential
    // headers are correct. This would reject the former production request.
    Mock::given(method("POST")).and(path(ENDPOINT))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"type":"error","error":{
            "type":"invalid_request_error","message":"anthropic-version header is required"}})))
        .with_priority(10).expect(1).mount(&server).await;
    let mut fixture = recorded_conformance_fixtures().into_iter().find(|fixture| fixture.protocol == ChatProtocol::Anthropic).unwrap();
    fixture.request.input.messages = vec![ChatMessage { role: ChatRole::User,
        content: vec![ChatContentPart::Text { text: "Use lookup and report the result.".to_owned() }], provider_round_id: None }];
    fixture.request.input.reasoning = None;
    fixture.request.input.prompt_cache = PromptCachePolicy::Disabled;
    let adapter = AnthropicAdapter::new(Arc::new(EncodingOnlyTransport));
    let credential = CredentialLease::new(fixture.route.credential_ref.clone(), CredentialTarget::for_route(&fixture.route), "fixture-lease");
    let first = adapter.encode_request(&fixture.request, &fixture.route, &credential).unwrap();
    assert_eq!(first.body["tools"][0]["name"], "lookup");
    assert!(first.body.get("anthropic-version").is_none());
    assert!(first.body.get("headers").is_none());
    // Reproduce the old production shape on the same endpoint before sending
    // the corrected executor requests. JSON + x-api-key alone is rejected.
    let missing_version = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(format!("{}{ENDPOINT}", server.uri()))
        .header("x-api-key", "fixture-secret").json(&first.body).send().await.unwrap();
    assert_eq!(missing_version.status(), reqwest::StatusCode::BAD_REQUEST);
    let rejection: Value = missing_version.json().await.unwrap();
    assert_eq!(rejection["error"]["message"], "anthropic-version header is required");
    let mut first_events = vec![message_start("msg_tool_round"),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_lookup","name":"lookup","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"fixture\"}"}}),
        json!({"type":"content_block_stop","index":0})];
    first_events.extend(message_end("tool_use"));
    Mock::given(method("POST")).and(path(ENDPOINT)).and(header("x-api-key", "fixture-secret"))
        .and(header("anthropic-version", "2023-06-01")).and(body_json(first.body.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(first_events), "text/event-stream"))
        .with_priority(1).expect(1).mount(&server).await;
    let executor = executor(AuthScheme::HeaderKey("x-api-key".to_owned()));
    let events = decode(&adapter, send(&executor, "anthropic.messages", format!("{}{ENDPOINT}", server.uri()), first.body).await);
    assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })));
    let call = events.into_iter().find_map(|event| match event { ChatModelEvent::ToolCallCompleted { call } => Some(call), _ => None }).unwrap();
    assert_eq!(call.name, "lookup");
    assert_eq!(call.arguments.0, json!({"query":"fixture"}));
    fixture.request.input.messages.push(ChatMessage { role: ChatRole::Assistant, content: vec![ChatContentPart::ToolCall {
        call_id: call.call_id.clone(), name: call.name, arguments: call.arguments, provider_metadata: call.provider_metadata,
    }], provider_round_id: None });
    fixture.request.input.messages.push(ChatMessage { role: ChatRole::Tool, content: vec![ChatContentPart::ToolResult {
        call_id: call.call_id, output: vec![ChatToolResultPart::Text { text: "static fixture result".to_owned() }], is_error: false,
    }], provider_round_id: None });
    let second = adapter.encode_request(&fixture.request, &fixture.route, &credential).unwrap();
    assert_eq!(second.body["messages"][1]["content"][0]["type"], "tool_use");
    assert_eq!(second.body["messages"][2]["content"][0]["type"], "tool_result");
    assert_eq!(second.body["messages"][2]["content"][0]["tool_use_id"], "tool_lookup");
    let mut second_events = vec![message_start("msg_final_round"),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Lookup completed."}}),
        json!({"type":"content_block_stop","index":0})];
    second_events.extend(message_end("end_turn"));
    Mock::given(method("POST")).and(path(ENDPOINT)).and(header("x-api-key", "fixture-secret"))
        .and(header("anthropic-version", "2023-06-01")).and(body_json(second.body.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(second_events), "text/event-stream"))
        .with_priority(1).expect(1).mount(&server).await;
    let events = decode(&adapter, send(&executor, "anthropic.messages", format!("{}{ENDPOINT}", server.uri()), second.body).await);
    assert!(events.iter().any(|event| matches!(event, ChatModelEvent::OutputTextDelta { text } if text == "Lookup completed.")));
    assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].headers.contains_key("anthropic-version"));
    for request in requests.into_iter().skip(1) {
        assert_eq!(request.headers.get_all("anthropic-version").iter().count(), 1);
        assert_eq!(request.headers["anthropic-version"], "2023-06-01");
        assert_eq!(request.headers["x-api-key"], "fixture-secret");
        assert_eq!(request.headers["content-type"], "application/json");
        assert!(!request.headers.contains_key("authorization"));
    }
}

#[tokio::test]
async fn non_streaming_messages_also_apply_version_without_replacing_declared_bearer_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path(ENDPOINT)).and(header("authorization", "Bearer fixture-secret"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type":"message","id":"msg_json","role":"assistant",
            "content":[{"type":"text","text":"Complete JSON reply"}],"stop_reason":"end_turn","usage":{"input_tokens":2,"output_tokens":3}})))
        .expect(1).mount(&server).await;
    let frames = send(&executor(AuthScheme::Bearer), "anthropic.messages", format!("{}{ENDPOINT}", server.uri()),
        json!({"model":"claude-recorded","messages":[{"role":"user","content":"hello"}],"max_tokens":2048,"stream":false})).await;
    let adapter = AnthropicAdapter::new(Arc::new(EncodingOnlyTransport));
    let events = decode(&adapter, frames);
    assert!(events.iter().any(|event| matches!(event, ChatModelEvent::OutputTextDelta { text } if text == "Complete JSON reply")));
    assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed })));
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn unrelated_chat_protocols_never_receive_the_messages_version_header() {
    for protocol in ["openai.chat_text", "openai.responses", "gemini.generate_text"] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/native-endpoint"))
            .and(header("authorization", "Bearer fixture-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"))
            .expect(1).mount(&server).await;
        send(&executor(AuthScheme::Bearer), protocol, format!("{}/native-endpoint", server.uri()), json!({"stream":true})).await;
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("anthropic-version"), "{protocol}");
        assert_eq!(requests[0].headers["authorization"], "Bearer fixture-secret");
    }
}
