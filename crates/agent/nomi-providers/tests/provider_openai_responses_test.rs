use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_config::compat::ProviderCompat;
use nomi_providers::openai_responses::OpenAIResponsesProvider;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomi_types::tool::ToolDef;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn request(retain_provider_round: bool) -> LlmRequest {
    LlmRequest {
        model: "gpt-test".into(),
        system: "system prompt".into(),
        messages: vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "hello".into(),
            }],
        )],
        tools: Vec::new(),
        max_tokens: Some(321),
        thinking: None,
        reasoning_effort: Some("high".into()),
        retain_provider_round,
    }
}

fn compat(chain_rounds: bool) -> ProviderCompat {
    ProviderCompat::merge(
        ProviderCompat::openai_responses_defaults(),
        ProviderCompat {
            chain_rounds: Some(chain_rounds),
            ..Default::default()
        },
    )
}

fn sse(events: Vec<(&str, Value)>) -> String {
    events
        .into_iter()
        .map(|(name, payload)| format!("event: {name}\ndata: {payload}\n\n"))
        .collect()
}

fn created(response_id: &str, store: bool) -> (&'static str, Value) {
    (
        "response.created",
        json!({
            "type": "response.created",
            "response": {
                "id": response_id,
                "status": "in_progress",
                "store": store,
                "output": []
            }
        }),
    )
}

fn completed_text(response_id: &str, store: bool, text: &str) -> String {
    let item = json!({
        "id": "msg_1",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    });
    sse(vec![
        created(response_id, store),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "msg_1", "type": "message", "status": "in_progress", "role": "assistant", "content": []}
            }),
        ),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "item_id": "msg_1",
                "delta": text
            }),
        ),
        (
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": item.clone()
            }),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {
                    "id": response_id,
                    "status": "completed",
                    "store": store,
                    "output": [item],
                    "usage": {
                        "input_tokens": 7,
                        "output_tokens": 3,
                        "input_tokens_details": {"cached_tokens": 2},
                        "output_tokens_details": {"reasoning_tokens": 1}
                    }
                }
            }),
        ),
    ])
}

async fn collect(mut receiver: tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn retention_requires_both_gates_and_uses_responses_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_stateless", false, "hello"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let events = collect(provider.stream(&request(false)).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(
        event,
        LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage
        } if usage.input_tokens == 7 && usage.output_tokens == 3 && usage.cache_read_tokens == 2 && usage.reasoning_tokens == 1
    )));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ProviderRoundId(_))));

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_output_tokens"], 321);
    assert_eq!(body["reasoning"], json!({"effort": "high"}));
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert!(body.get("previous_response_id").is_none());
}

#[tokio::test]
async fn extra_body_cannot_restore_an_omitted_ceiling_or_protocol_invariants() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_typed", false, "ok"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let mut provider_compat = compat(true);
    provider_compat.extra_body = Some(
        json!({
            "max_output_tokens": 999,
            "stream": false,
            "store": true,
            "previous_response_id": "resp_injected"
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), provider_compat);
    let mut request = request(false);
    request.max_tokens = None;
    collect(provider.stream(&request).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert!(body.get("max_output_tokens").is_none());
    assert!(body.get("previous_response_id").is_none());
    assert_eq!(body["stream"], true);
    assert_eq!(body["store"], false);
}

#[tokio::test]
async fn chaining_uses_only_the_newest_assistant_cursor_and_nonempty_suffix() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_next", true, "done"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let mut previous = Message::new(
        Role::Assistant,
        vec![ContentBlock::Text { text: "old".into() }],
    );
    previous.provider_round_id = Some("resp_previous".into());
    let mut chained = request(true);
    chained.messages = vec![
        Message::new(Role::User, vec![ContentBlock::Text { text: "old user".into() }]),
        previous,
        Message::new(Role::User, vec![ContentBlock::Text { text: "new user".into() }]),
    ];

    let events = collect(provider.stream(&chained).await.unwrap()).await;
    let cursor_index = events
        .iter()
        .position(|event| matches!(event, LlmEvent::ProviderRoundId(id) if id == "resp_next"))
        .unwrap();
    assert!(matches!(events[cursor_index + 1], LlmEvent::Done { .. }));

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["store"], true);
    assert_eq!(body["previous_response_id"], "resp_previous");
    assert_eq!(body["input"].as_array().unwrap().len(), 1);
    assert_eq!(body["input"][0]["content"][0]["text"], "new user");
}

#[tokio::test]
async fn an_older_cursor_is_not_used_when_the_newest_assistant_has_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_next", true, "done"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let mut old = Message::new(Role::Assistant, vec![]);
    old.provider_round_id = Some("resp_old".into());
    let mut full = request(true);
    full.messages = vec![
        old,
        Message::new(Role::User, vec![ContentBlock::Text { text: "middle".into() }]),
        Message::new(Role::Assistant, vec![ContentBlock::Text { text: "draft".into() }]),
        Message::new(Role::User, vec![ContentBlock::Text { text: "latest".into() }]),
    ];
    collect(provider.stream(&full).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert!(body.get("previous_response_id").is_none());
    assert_eq!(body["input"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_cursor_with_an_empty_serialized_suffix_falls_back_to_full_snapshot() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_next", true, "done"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let mut previous = Message::new(
        Role::Assistant,
        vec![ContentBlock::Text {
            text: "preserved full snapshot".into(),
        }],
    );
    previous.provider_round_id = Some("resp_previous".into());
    let mut full = request(true);
    full.messages = vec![
        previous,
        Message::new(
            Role::System,
            vec![ContentBlock::Text {
                text: "ignored as top-level instructions".into(),
            }],
        ),
    ];
    collect(provider.stream(&full).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert!(body.get("previous_response_id").is_none());
    assert_eq!(body["input"][0]["role"], "assistant");
    assert_eq!(body["input"][0]["content"], "preserved full snapshot");
}

#[tokio::test]
async fn incomplete_function_call_is_never_executable_or_chainable() {
    let server = MockServer::start().await;
    let function = json!({
        "id": "fc_1",
        "type": "function_call",
        "status": "incomplete",
        "call_id": "call_1",
        "name": "Write",
        "arguments": "{\"path\":\"mini"
    });
    let body = sse(vec![
        created("resp_cut", true),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "fc_1", "type": "function_call", "status": "in_progress", "call_id": "call_1", "name": "Write", "arguments": ""}
            }),
        ),
        (
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "item_id": "fc_1",
                "delta": "{\"path\":\"mini"
            }),
        ),
        (
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": function.clone()
            }),
        ),
        (
            "response.incomplete",
            json!({
                "type": "response.incomplete",
                "response": {
                    "id": "resp_cut",
                    "status": "incomplete",
                    "store": true,
                    "output": [function],
                    "incomplete_details": {"reason": "max_output_tokens"},
                    "usage": {"input_tokens": 10, "output_tokens": 20}
                }
            }),
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;

    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let events = collect(provider.stream(&request(true)).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(
        event,
        LlmEvent::ToolUseTruncated { id, name, argument_bytes }
            if id == "call_1" && name == "Write" && *argument_bytes > 0
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        LlmEvent::Done { stop_reason: StopReason::MaxTokens, .. }
    )));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ToolUse { .. })));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ProviderRoundId(_))));
}

#[tokio::test]
async fn completed_function_call_commits_atomically_with_cursor_then_done() {
    let server = MockServer::start().await;
    let arguments = r#"{"path":"miniapp.html"}"#;
    let function = json!({
        "id": "fc_complete",
        "type": "function_call",
        "status": "completed",
        "call_id": "call_complete",
        "name": "Write",
        "arguments": arguments
    });
    let body = sse(vec![
        created("resp_tool", true),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "fc_complete", "type": "function_call", "status": "in_progress", "call_id": "call_complete", "name": "Write", "arguments": ""}
            }),
        ),
        (
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "item_id": "fc_complete",
                "delta": arguments
            }),
        ),
        (
            "response.function_call_arguments.done",
            json!({
                "type": "response.function_call_arguments.done",
                "output_index": 0,
                "item_id": "fc_complete",
                "name": "Write",
                "arguments": arguments
            }),
        ),
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": function.clone()}),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_tool", "status": "completed", "store": true, "output": [function], "usage": null}
            }),
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let events = collect(provider.stream(&request(true)).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(
        event,
        LlmEvent::ToolUse { id, name, input, .. }
            if id == "call_complete" && name == "Write" && input["path"] == "miniapp.html"
    )));
    let cursor = events
        .iter()
        .position(|event| matches!(event, LlmEvent::ProviderRoundId(id) if id == "resp_tool"))
        .unwrap();
    assert!(matches!(
        events[cursor + 1],
        LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            ..
        }
    ));
}

#[derive(Clone)]
struct StaleOnceResponder {
    calls: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct ThreeBodyNegotiationResponder {
    calls: Arc<AtomicUsize>,
}

impl Respond for ThreeBodyNegotiationResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "Read");
        assert!(body["tools"][0].get("function").is_none());
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                assert_eq!(body["previous_response_id"], "resp_stale_schema");
                assert!(body["tools"][0]["parameters"].get("oneOf").is_some());
                ResponseTemplate::new(404).set_body_json(json!({
                    "error": {"message": "previous_response_id resp_stale_schema has expired"}
                }))
            }
            1 => {
                assert!(body.get("previous_response_id").is_none());
                assert!(body["tools"][0]["parameters"].get("oneOf").is_some());
                ResponseTemplate::new(400).set_body_json(json!({
                    "error": {"message": "TOOL_SCHEMA_INVALID: input_schema oneOf is unsupported at the top level"}
                }))
            }
            2 => {
                assert!(body.get("previous_response_id").is_none());
                assert!(body["tools"][0]["parameters"].get("oneOf").is_none());
                ResponseTemplate::new(200).set_body_raw(
                    completed_text("resp_negotiated", true, "ok"),
                    "text/event-stream",
                )
            }
            attempt => panic!("unexpected fourth negotiation body: {attempt}"),
        }
    }
}

impl Respond for StaleOnceResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            assert_eq!(body["previous_response_id"], "resp_stale");
            return ResponseTemplate::new(404).set_body_json(json!({
                "error": {"message": "previous_response_id resp_stale was not found"}
            }));
        }
        assert!(body.get("previous_response_id").is_none());
        ResponseTemplate::new(200)
            .set_body_raw(completed_text("resp_fresh", true, "ok"), "text/event-stream")
    }
}

#[tokio::test]
async fn stale_parent_negotiates_once_but_generic_endpoint_404_does_not() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(StaleOnceResponder {
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .expect(2)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let mut previous = Message::new(Role::Assistant, vec![]);
    previous.provider_round_id = Some("resp_stale".into());
    let mut chained = request(true);
    chained.messages = vec![previous, Message::new(Role::User, vec![])];
    let events = collect(provider.stream(&chained).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(event, LlmEvent::ProviderRoundId(id) if id == "resp_fresh")));
    server.verify().await;

    let wrong = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(404).set_body_string("POST /v1/responses not found"))
        .expect(1)
        .mount(&wrong)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &wrong.uri(), compat(true));
    let error = provider.stream(&chained).await.unwrap_err();
    assert!(matches!(error, ProviderError::Api { status: 404, .. }));
    assert!(error.to_string().contains("expected a POST /responses endpoint"));
    wrong.verify().await;
}

#[tokio::test]
async fn stale_and_schema_negotiation_is_monotonic_and_bounded_to_three_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ThreeBodyNegotiationResponder {
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .expect(3)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let mut previous = Message::new(Role::Assistant, vec![]);
    previous.provider_round_id = Some("resp_stale_schema".into());
    let mut chained = request(true);
    chained.messages = vec![
        previous,
        Message::new(Role::User, vec![ContentBlock::Text { text: "next".into() }]),
    ];
    chained.tools.push(ToolDef {
        name: "Read".into(),
        description: "read a file".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "oneOf": [{"required": ["path"]}]
        }),
        deferred: false,
    });
    let events = collect(provider.stream(&chained).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(event, LlmEvent::Done { .. })));
    server.verify().await;
}

#[tokio::test]
async fn encrypted_reasoning_is_wrapped_opaquely_and_replayed_statelessly() {
    let server = MockServer::start().await;
    let reasoning = json!({
        "id": "rs_1",
        "type": "reasoning",
        "status": "completed",
        "summary": [],
        "encrypted_content": "opaque-ciphertext"
    });
    let response = sse(vec![
        created("resp_reasoning", false),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "rs_1", "type": "reasoning", "status": "in_progress", "summary": []}
            }),
        ),
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": reasoning.clone()}),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_reasoning", "status": "completed", "store": false, "output": [reasoning], "usage": null}
            }),
        ),
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(response, "text/event-stream"))
        .expect(2)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(false));

    let first = collect(provider.stream(&request(false)).await.unwrap()).await;
    let signature = first
        .iter()
        .find_map(|event| match event {
            LlmEvent::ThinkingSignature(signature) => Some(signature.clone()),
            _ => None,
        })
        .expect("opaque reasoning wrapper");
    assert!(signature.starts_with("openai.responses.reasoning.v1:"));
    assert!(!signature.contains("system prompt"));

    let mut replay = request(false);
    replay.messages = vec![
        Message::new(
            Role::Assistant,
            vec![ContentBlock::Thinking {
                thinking: String::new(),
                signature: Some(signature),
            }],
        ),
        Message::new(Role::User, vec![ContentBlock::Text { text: "continue".into() }]),
    ];
    collect(provider.stream(&replay).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let second: Value = serde_json::from_slice(&received[1].body).unwrap();
    assert_eq!(second["store"], false);
    assert_eq!(second["input"][0]["type"], "reasoning");
    assert_eq!(second["input"][0]["encrypted_content"], "opaque-ciphertext");
}

async fn protocol_error_events(body: String, retain: bool) -> Vec<LlmEvent> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(true));
    let events = collect(provider.stream(&request(retain)).await.unwrap()).await;
    server.verify().await;
    events
}

fn assert_failed_without_commit(events: &[LlmEvent]) {
    assert!(events.iter().any(|event| matches!(event, LlmEvent::Error(_))));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::Done { .. })));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ProviderRoundId(_))));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ToolUse { .. })));
}

#[tokio::test]
async fn top_level_official_error_event_fails_without_a_terminal_commit() {
    let body = sse(vec![(
        "error",
        json!({
            "type": "error",
            "code": "invalid_request_error",
            "message": "request was rejected",
            "param": "input"
        }),
    )]);
    let events = protocol_error_events(body, true).await;
    assert_failed_without_commit(&events);
    assert!(events.iter().any(
        |event| matches!(event, LlmEvent::Error(message) if message.contains("request was rejected"))
    ));
}

#[tokio::test]
async fn terminal_commit_is_withheld_when_a_poison_frame_follows() {
    let body = sse(vec![
        created("resp_poison", true),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_poison", "status": "completed", "store": true, "output": [], "usage": null}
            }),
        ),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "item_id": "msg_poison",
                "delta": "must not be accepted"
            }),
        ),
    ]);
    let events = protocol_error_events(body, true).await;
    assert_failed_without_commit(&events);
}

#[tokio::test]
async fn completed_round_rejects_an_incomplete_function_item() {
    let function = json!({
        "id": "fc_bad",
        "type": "function_call",
        "status": "incomplete",
        "call_id": "call_bad",
        "name": "Write",
        "arguments": "{}"
    });
    let body = sse(vec![
        created("resp_bad", true),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_bad", "status": "completed", "store": true, "output": [function], "usage": null}
            }),
        ),
    ]);
    let events = protocol_error_events(body, true).await;
    assert_failed_without_commit(&events);
}

#[tokio::test]
async fn terminal_message_must_cover_every_streamed_content_index() {
    let terminal = json!({
        "id": "msg_sparse",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": "visible", "annotations": []}]
    });
    let body = sse(vec![
        created("resp_sparse", true),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "msg_sparse", "type": "message", "status": "in_progress", "role": "assistant", "content": []}
            }),
        ),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 1,
                "item_id": "msg_sparse",
                "delta": "hidden"
            }),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_sparse", "status": "completed", "store": true, "output": [terminal], "usage": null}
            }),
        ),
    ]);
    let events = protocol_error_events(body, true).await;
    assert_failed_without_commit(&events);
}

#[tokio::test]
async fn output_item_done_rejects_duplicate_done_and_later_delta() {
    let item = json!({
        "id": "msg_done",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": "ok", "annotations": []}]
    });
    for tail in [
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": item.clone()}),
        ),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "item_id": "msg_done",
                "delta": "late"
            }),
        ),
    ] {
        let body = sse(vec![
            created("resp_done", true),
            (
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": {"id": "msg_done", "type": "message", "status": "in_progress", "role": "assistant", "content": []}
                }),
            ),
            (
                "response.output_item.done",
                json!({"type": "response.output_item.done", "output_index": 0, "item": item.clone()}),
            ),
            tail,
        ]);
        let events = protocol_error_events(body, true).await;
        assert_failed_without_commit(&events);
    }
}

#[tokio::test]
async fn message_content_indices_and_cross_frame_aggregate_are_bounded() {
    let item_added = || {
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "msg_bounded", "type": "message", "status": "in_progress", "role": "assistant", "content": []}
            }),
        )
    };
    let chunk = "x".repeat(300 * 1024);
    let aggregate_body = sse(vec![
        created("resp_aggregate", true),
        item_added(),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "item_id": "msg_bounded",
                "delta": chunk
            }),
        ),
        (
            "response.refusal.delta",
            json!({
                "type": "response.refusal.delta",
                "output_index": 0,
                "content_index": 1,
                "item_id": "msg_bounded",
                "delta": "y".repeat(300 * 1024)
            }),
        ),
    ]);
    let aggregate_events = protocol_error_events(aggregate_body, true).await;
    assert_failed_without_commit(&aggregate_events);
    assert!(aggregate_events.iter().any(
        |event| matches!(event, LlmEvent::Error(message) if message.contains("aggregate safety limit"))
    ));

    let index_body = sse(vec![
        created("resp_index", true),
        item_added(),
        (
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 128,
                "item_id": "msg_bounded",
                "delta": "out of range"
            }),
        ),
    ]);
    let index_events = protocol_error_events(index_body, true).await;
    assert_failed_without_commit(&index_events);
    assert!(index_events.iter().any(
        |event| matches!(event, LlmEvent::Error(message) if message.contains("128-part safety limit"))
    ));
}

#[tokio::test]
async fn terminal_reasoning_must_match_output_item_done() {
    let done = json!({
        "id": "rs_mismatch",
        "type": "reasoning",
        "status": "completed",
        "summary": [],
        "encrypted_content": "cipher-one"
    });
    let terminal = json!({
        "id": "rs_mismatch",
        "type": "reasoning",
        "status": "completed",
        "summary": [],
        "encrypted_content": "cipher-two"
    });
    let body = sse(vec![
        created("resp_reasoning_bad", true),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "rs_mismatch", "type": "reasoning", "status": "in_progress", "summary": []}
            }),
        ),
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": done}),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_reasoning_bad", "status": "completed", "store": true, "output": [terminal], "usage": null}
            }),
        ),
    ]);
    let events = protocol_error_events(body, true).await;
    assert_failed_without_commit(&events);
}
