//! Wire-level tests for the shared Chat verification round trip: every case
//! exercises the production broker serializers, the single-attempt executor's
//! SSE framing and the verification decoders with broker sequence rules.

use std::time::Duration;

use nomifun_api_types::ProviderHealthCheckErrorKind;
use nomifun_model_invoke::{
    AuthMaterial, AuthScheme, ChatVerificationFailure, ChatVerificationTarget,
    verify_chat_agent_round_trip,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = "setup-only-secret";
const MODEL: &str = "verify-model";
const OUTPUT_TOKENS: u32 = 1024;

fn target(protocol_id: &str, url: String, auth: AuthMaterial) -> ChatVerificationTarget {
    ChatVerificationTarget {
        protocol_id: protocol_id.to_owned(),
        url,
        model: MODEL.to_owned(),
        auth,
        provider_params: json!({}),
        configured_output_limit: Some(4096),
        region: None,
    }
}

fn bearer() -> AuthMaterial {
    AuthMaterial {
        scheme: AuthScheme::Bearer,
        credentials: json!({"api_keys": [KEY]}),
    }
}

fn anthropic_key() -> AuthMaterial {
    AuthMaterial {
        scheme: AuthScheme::HeaderKey("x-api-key".to_owned()),
        credentials: json!({"api_keys": [KEY]}),
    }
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

/// One mock answer per round: the body marker selects round two.
#[derive(Clone)]
struct Rounds {
    first: String,
    second: String,
    marker: &'static str,
}

impl Respond for Rounds {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&request.body);
        let stream = if body.contains(self.marker) {
            &self.second
        } else {
            &self.first
        };
        ResponseTemplate::new(200).set_body_raw(stream.clone(), "text/event-stream")
    }
}

fn anthropic_union(event: Value) -> Value {
    let mut event = event;
    let object = event.as_object_mut().unwrap();
    object.entry("content_block").or_insert(Value::Null);
    object.entry("delta").or_insert(Value::Null);
    object.entry("error").or_insert(Value::Null);
    object.entry("index").or_insert(json!(0));
    object.entry("message").or_insert(Value::Null);
    event
}

fn anthropic_tool_stream() -> String {
    sse(&[
        anthropic_union(json!({"type":"message_start","message":{"container":null,"content":[],
            "id":"msg_1","model":MODEL,"role":"assistant","stop_details":null,"stop_reason":null,
            "stop_sequence":null,"type":"message","usage":{"input_tokens":9,"output_tokens":1}}})),
        anthropic_union(json!({"type":"ping"})),
        anthropic_union(json!({"type":"content_block_start","index":0,
            "content_block":{"signature":"","thinking":"","type":"thinking"}})),
        anthropic_union(json!({"type":"content_block_delta","index":0,
            "delta":{"thinking":"ok","type":"thinking_delta"}})),
        anthropic_union(json!({"type":"content_block_delta","index":0,
            "delta":{"signature":"sig-ver","type":"signature_delta"}})),
        anthropic_union(json!({"type":"content_block_stop","index":0})),
        anthropic_union(json!({"type":"content_block_start","index":1,
            "content_block":{"id":"call_verify","input":{},"name":"nomifun_protocol_echo","type":"tool_use"}})),
        anthropic_union(json!({"type":"content_block_delta","index":1,
            "delta":{"partial_json":"{\"value\":\"ok\"}","type":"input_json_delta"}})),
        anthropic_union(json!({"type":"content_block_stop","index":1})),
        anthropic_union(json!({"type":"message_delta","index":0,
            "delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":7}})),
        anthropic_union(json!({"type":"message_stop"})),
    ])
}

fn anthropic_text_stream() -> String {
    sse(&[
        anthropic_union(json!({"type":"message_start","message":{"container":null,"content":[],
            "id":"msg_2","model":MODEL,"role":"assistant","stop_details":null,"stop_reason":null,
            "stop_sequence":null,"type":"message","usage":{"input_tokens":11,"output_tokens":1}}})),
        anthropic_union(json!({"type":"content_block_start","index":0,
            "content_block":{"text":"","type":"text"}})),
        anthropic_union(json!({"type":"content_block_delta","index":0,
            "delta":{"text":"OK","type":"text_delta"}})),
        anthropic_union(json!({"type":"content_block_stop","index":0})),
        anthropic_union(json!({"type":"message_delta","index":0,
            "delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}})),
        anthropic_union(json!({"type":"message_stop"})),
    ])
}

fn responses_tool_stream() -> String {
    sse(&[
        json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-a"}}),
        json!({"type":"response.output_item.done","output_index":0,
            "item":{"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-b"}}),
        json!({"type":"response.output_item.added","output_index":1,
            "item":{"id":"fc_1","type":"function_call","status":"in_progress",
                "arguments":"","call_id":"call_1","name":"nomifun_protocol_echo"}}),
        json!({"type":"response.function_call_arguments.delta","output_index":1,
            "item_id":"fc_1","delta":"{\"value\":\"ok\"}"}),
        json!({"type":"response.function_call_arguments.done","output_index":1,
            "item_id":"fc_1","arguments":"{\"value\":\"ok\"}"}),
        json!({"type":"response.output_item.done","output_index":1,
            "item":{"id":"fc_1","type":"function_call","status":"completed",
                "arguments":"{\"value\":\"ok\"}","call_id":"call_1","name":"nomifun_protocol_echo"}}),
        json!({"type":"response.completed","response":{"id":"resp_1","status":"completed",
            "output":[
                {"id":"rs_1","type":"reasoning","content":[],"encrypted_content":"cipher-c"},
                {"id":"fc_1","type":"function_call","status":"completed",
                    "arguments":"{\"value\":\"ok\"}","call_id":"call_1","name":"nomifun_protocol_echo"}
            ],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}),
    ])
}

fn responses_text_stream() -> String {
    sse(&[
        json!({"type":"response.created","response":{"id":"resp_2","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"msg_2","type":"message","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"msg_2",
            "content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"msg_2",
            "content_index":0,"delta":"OK"}),
        json!({"type":"response.output_text.done","output_index":0,"item_id":"msg_2",
            "content_index":0,"text":"OK"}),
        json!({"type":"response.content_part.done","output_index":0,"item_id":"msg_2",
            "content_index":0,"part":{"type":"output_text","text":"OK","annotations":[]}}),
        json!({"type":"response.output_item.done","output_index":0,
            "item":{"id":"msg_2","type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"OK","annotations":[]}]}}),
        json!({"type":"response.completed","response":{"id":"resp_2","status":"completed",
            "output":[{"id":"msg_2","type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"OK","annotations":[]}]}],
            "usage":{"input_tokens":12,"output_tokens":2,"total_tokens":14}}}),
    ])
}

fn chat_text_stream() -> String {
    let frames = [
        json!({"id":"chatcmpl_1","choices":[{"index":0,
            "delta":{"role":"assistant","content":"OK"},"finish_reason":null}]}),
        json!({"id":"chatcmpl_1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":4,"completion_tokens":1}}),
    ];
    let mut body = frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    body
}

#[tokio::test]
async fn anthropic_round_trip_replays_signed_thinking_tool_call_and_result() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Rounds {
            first: anthropic_tool_stream(),
            second: anthropic_text_stream(),
            marker: "tool_result",
        })
        .expect(2)
        .mount(&server)
        .await;

    let target = target(
        "anthropic.messages",
        format!("{}/v1/messages", server.uri()),
        anthropic_key(),
    );
    verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_secs(10))
        .await
        .expect("the union-envelope exchange must verify");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.headers["x-api-key"], KEY);
        assert_eq!(request.headers["anthropic-version"], "2023-06-01");
    }
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let messages = second["messages"].as_array().unwrap();
    let assistant = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .expect("round two replays the assistant turn");
    let content = assistant["content"].as_array().unwrap();
    assert!(
        content.iter().any(|part| part["type"] == "thinking"
            && part["thinking"] == "ok"
            && part["signature"] == "sig-ver"),
        "round two must replay the signed thinking block: {second}"
    );
    let tool_use = content
        .iter()
        .find(|part| part["type"] == "tool_use")
        .expect("round two replays the tool call");
    assert_eq!(tool_use["name"], "nomifun_protocol_echo");
    assert_eq!(tool_use["input"], json!({"value": "ok"}));
    let result = messages
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|part| part["type"] == "tool_result")
        .expect("round two sends the local tool result");
    assert_eq!(result["tool_use_id"], tool_use["id"]);
}

#[tokio::test]
async fn responses_round_trip_replays_done_snapshot_ciphertext_and_call_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(Rounds {
            first: responses_tool_stream(),
            second: responses_text_stream(),
            marker: "function_call_output",
        })
        .expect(2)
        .mount(&server)
        .await;

    let target = target(
        "openai.responses",
        format!("{}/v1/responses", server.uri()),
        bearer(),
    );
    verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_secs(10))
        .await
        .expect("the Responses exchange must verify");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].headers["authorization"], format!("Bearer {KEY}"));
    let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
    let input = second["input"].as_array().unwrap();
    let reasoning = input
        .iter()
        .find(|item| item["type"] == "reasoning")
        .expect("round two replays the reasoning item");
    assert_eq!(
        reasoning["encrypted_content"], "cipher-b",
        "continuation ciphertext comes from output_item.done, never the terminal"
    );
    let output = input
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .expect("round two sends the local tool result");
    assert_eq!(output["call_id"], "call_1");
}

#[tokio::test]
async fn chat_unsupported_parameter_is_transport_incompatible() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "code": "unsupported_parameter",
                "type": "invalid_request_error",
                "message": "Function tools with reasoning_effort are not supported for gpt-6-sol in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'."
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let target = target(
        "openai.chat_text",
        format!("{}/v1/chat/completions", server.uri()),
        bearer(),
    );
    let failure = verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_secs(10))
        .await
        .expect_err("a 400 contract rejection cannot verify");
    let ChatVerificationFailure::Transport(_) = &failure else {
        panic!("expected Transport, got {failure:?}");
    };
    let diagnosis = failure.diagnosis().expect("transport failures carry a diagnosis");
    assert!(diagnosis.incompatible);
    assert_eq!(diagnosis.error_kind, ProviderHealthCheckErrorKind::InvalidRequest);
    assert_eq!(diagnosis.http_status, Some(400));
}

#[tokio::test]
async fn text_without_the_tool_is_inconclusive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(chat_text_stream(), "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;

    let target = target(
        "openai.chat_text",
        format!("{}/v1/chat/completions", server.uri()),
        bearer(),
    );
    let failure = verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_secs(10))
        .await
        .expect_err("a tool-free answer cannot prove tool support");
    let ChatVerificationFailure::Inconclusive(message) = &failure else {
        panic!("expected Inconclusive, got {failure:?}");
    };
    assert!(message.contains("without the verification tool"), "{message}");
    assert!(failure.diagnosis().is_none());
}

#[tokio::test]
async fn unauthorized_is_diagnosed_without_string_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"type": "authentication_error", "message": "invalid api key"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let target = target(
        "openai.chat_text",
        format!("{}/v1/chat/completions", server.uri()),
        bearer(),
    );
    let failure = verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_secs(10))
        .await
        .expect_err("401 cannot verify");
    let diagnosis = failure.diagnosis().expect("401 carries a diagnosis");
    assert_eq!(diagnosis.error_kind, ProviderHealthCheckErrorKind::Unauthorized);
    assert_eq!(diagnosis.http_status, Some(401));
    assert!(!diagnosis.incompatible);
    assert!(!failure.message().contains(KEY), "the key must never appear in diagnostics");
}

#[tokio::test]
async fn deadline_produces_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(chat_text_stream(), "text/event-stream")
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let target = target(
        "openai.chat_text",
        format!("{}/v1/chat/completions", server.uri()),
        bearer(),
    );
    let failure = verify_chat_agent_round_trip(&target, OUTPUT_TOKENS, Duration::from_millis(150))
        .await
        .expect_err("the exchange deadline must fire");
    assert!(matches!(failure, ChatVerificationFailure::Timeout));
    assert_eq!(
        failure.diagnosis().map(|diagnosis| diagnosis.error_kind),
        Some(ProviderHealthCheckErrorKind::Timeout)
    );
}
