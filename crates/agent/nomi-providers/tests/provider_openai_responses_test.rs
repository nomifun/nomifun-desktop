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

mod support;
use support::collect_events as collect;

fn request(retain_provider_round: bool) -> LlmRequest {
    LlmRequest {
        reasoning_effort: Some("high".into()),
        retain_provider_round,
        ..support::request("gpt-test", "system prompt", "hello", 321)
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

#[tokio::test]
async fn dropping_receiver_closes_a_stalled_initial_response() {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
        let mut content_length = None;
        loop {
            let mut line = String::new();
            assert_ne!(socket.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
        }
        let mut body = vec![0; content_length.expect("request must have a body length")];
        socket.read_exact(&mut body).await.unwrap();
        socket.get_mut().write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 100\r\nconnection: close\r\n\r\n",
        ).await.unwrap();
        // Keep the HTTP body pending. Closing the downstream must release it
        // without requiring another SSE event or waiting for a request timeout.
        timeout(Duration::from_secs(2), socket.read(&mut [0]))
            .await
            .expect("stalled response stayed open after receiver drop")
            .unwrap()
    });
    let provider = OpenAIResponsesProvider::new("key", &url, compat(false));
    let receiver = timeout(Duration::from_secs(2), provider.stream(&request(false)))
        .await
        .unwrap()
        .unwrap();
    drop(receiver);
    assert_eq!(server.await.unwrap(), 0);
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
async fn model_reasoning_default_applies_when_the_request_has_no_override() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_default_effort", false, "ok"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut provider_compat = compat(false);
    provider_compat.reasoning_effort = Some("medium".to_owned());
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), provider_compat);
    let mut request = request(false);
    request.reasoning_effort = None;
    collect(provider.stream(&request).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["reasoning"], json!({"effort": "medium"}));
}

#[tokio::test]
async fn native_reasoning_options_and_optional_tool_arguments_keep_their_semantics() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_native_options", false, "ok"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let mut provider_compat = compat(false);
    provider_compat.extra_body = Some(
        json!({
            "reasoning": {"summary": "auto", "effort": "injected"},
            "include": ["message.output_text.logprobs", "reasoning.encrypted_content", "message.output_text.logprobs"]
        })
            .as_object()
            .unwrap()
            .clone(),
    );
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), provider_compat);
    let mut request = request(false);
    request.reasoning_effort = None;
    let schema = json!({
        "type": "object",
        "properties": {"city": {"type": "string"}, "unit": {"type": "string"}},
        "required": ["city"]
    });
    request.tools = vec![
        ToolDef {
            name: "weather".into(),
            description: "Weather".into(),
            input_schema: schema.clone(),
            deferred: false,
        },
        ToolDef {
            name: "forecast".into(),
            description: "Forecast".into(),
            input_schema: schema.clone(),
            deferred: true,
        },
    ];
    let events = collect(provider.stream(&request).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(event, LlmEvent::Done { .. })));

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["reasoning"], json!({"summary": "auto"}));
    assert_eq!(body["include"], json!(["reasoning.encrypted_content", "message.output_text.logprobs"]));
    assert_eq!(body["tools"][0]["parameters"], schema);
    assert_eq!(body["tools"][0]["strict"], false);
    assert_eq!(body["tools"][1]["strict"], false);
}

#[tokio::test]
async fn malformed_responses_include_options_fail_before_transport() {
    let server = MockServer::start().await;
    for include in [json!("reasoning.encrypted_content"), json!([false]), json!([" "])] {
        let mut provider_compat = compat(false);
        provider_compat.extra_body = Some(json!({"include": include}).as_object().unwrap().clone());
        let provider = OpenAIResponsesProvider::new("key", &server.uri(), provider_compat);
        assert!(matches!(provider.stream(&request(false)).await, Err(ProviderError::Config(_))));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn include_null_omits_the_optional_provider_extension() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_no_include", false, "ok"), "text/event-stream",
        ))
        .expect(1).mount(&server).await;
    let mut provider_compat = compat(false);
    provider_compat.extra_body = Some(json!({"include": null}).as_object().unwrap().clone());
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), provider_compat);
    let events = collect(provider.stream(&request(false)).await.unwrap()).await;
    assert!(events.iter().any(|event| matches!(event, LlmEvent::Done { .. })));
    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert!(body.get("include").is_none());
}

fn compatible_responses_frames() -> Vec<Value> {
    vec![
        json!({"type":"response.created","response":{"id":"resp_compatible","object":"response","status":"in_progress"}}),
        json!({"type":"response.output_item.added","response_id":"resp_compatible","output_index":0,
            "item":{"type":"message","id":"msg_compatible","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.content_part.added","response_id":"resp_compatible","output_index":0,"content_index":0,
            "part":{"type":"output_text","text":""}}),
        json!({"type":"response.content_part.delta","response_id":"resp_compatible","output_index":0,"content_index":0,"delta":"hello"}),
        json!({"type":"response.output_item.done","response_id":"resp_compatible","output_index":0,
            "item":{"type":"message","id":"msg_compatible","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"hello"}]}}),
        json!({"type":"response.done","response":{"id":"resp_compatible","object":"response","status":"completed",
            "usage":{"input_tokens":12,"output_tokens":3}}}),
    ]
}

fn data_only_responses(frames: Vec<Value>) -> String {
    let mut body = ": OPENROUTER PROCESSING\n\n".to_owned();
    body.extend(frames.into_iter().map(|frame| format!("data: {frame}\n\n")));
    body.push_str("data: [DONE]\n\n");
    body
}

// The event names and nullable reasoning snapshot follow StepFun's published
// /v1/responses SSE example. Generic part names cover the user-reported path
// that previously rejected a reasoning item as a non-message.
fn stepfun_responses_frames(generic_reasoning_parts: bool, tool: bool) -> Vec<Value> {
    let added = if generic_reasoning_parts { "response.content_part.added" } else { "response.reasoning_part.added" };
    let done = if generic_reasoning_parts { "response.content_part.done" } else { "response.reasoning_part.done" };
    let mut frames = vec![
        json!({"type":"response.created","response":{"id":"resp_step","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_step","type":"reasoning","summary":[],"content":null,"encrypted_content":null,"status":"in_progress"}}),
    ];
    for (index, text) in ["用户要求问候。", "先核实工具结果。"].into_iter().enumerate() {
        frames.extend([
            json!({"type":added,"output_index":0,"item_id":"rs_step","content_index":index,"part":{"type":"reasoning_text","text":""}}),
            json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"rs_step","content_index":index,"delta":text}),
            json!({"type":"response.reasoning_text.done","output_index":0,"item_id":"rs_step","content_index":index,"text":text}),
            json!({"type":done,"output_index":0,"item_id":"rs_step","content_index":index,"part":{"type":"reasoning_text","text":text}}),
        ]);
    }
    let reasoning = json!({"id":"rs_step","type":"reasoning","summary":[],"content":null,"encrypted_content":null,"status":null});
    frames.push(json!({"type":"response.output_item.done","output_index":0,"item":reasoning}));
    let output = if tool {
        let function = json!({"id":"fc_step","type":"function_call","call_id":"call_step","name":"get_weather",
            "arguments":"{\"city\":\"北京\"}","status":"completed"});
        frames.extend([
            json!({"type":"response.output_item.added","output_index":1,
                "item":{"id":"fc_step","type":"function_call","call_id":"call_step","name":"get_weather","arguments":"","status":"in_progress"}}),
            json!({"type":"response.function_call_arguments.done","output_index":1,"item_id":"fc_step","name":"get_weather","arguments":"{\"city\":\"北京\"}"}),
            json!({"type":"response.output_item.done","output_index":1,"item":function}),
        ]);
        function
    } else {
        let message = json!({"id":"msg_step","type":"message","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":"你好","annotations":[]}]});
        frames.extend([
            json!({"type":"response.output_item.added","output_index":1,
                "item":{"id":"msg_step","type":"message","role":"assistant","content":[],"status":"in_progress"}}),
            json!({"type":"response.content_part.added","output_index":1,"item_id":"msg_step","content_index":0,
                "part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","output_index":1,"item_id":"msg_step","content_index":0,"delta":"你好"}),
            json!({"type":"response.output_text.done","output_index":1,"item_id":"msg_step","content_index":0,"text":"你好"}),
            json!({"type":"response.content_part.done","output_index":1,"item_id":"msg_step","content_index":0,
                "part":{"type":"output_text","text":"你好","annotations":[]}}),
            json!({"type":"response.output_item.done","output_index":1,"item":message}),
        ]);
        message
    };
    frames.push(json!({"type":"response.completed","response":{"id":"resp_step","status":"completed","output":[reasoning,output],
        "usage":{"input_tokens":14,"output_tokens":52,"output_tokens_details":{"reasoning_tokens":12}}}}));
    for (sequence, frame) in frames.iter_mut().enumerate() { frame["sequence_number"] = json!(sequence); }
    frames
}

fn named_responses(frames: Vec<Value>) -> String {
    frames.into_iter().map(|frame| format!("event: {}\ndata: {frame}\n\n", frame["type"].as_str().unwrap())).collect()
}

#[tokio::test]
async fn stepfun_reasoning_parts_and_nullable_snapshot_finish_without_reclassifying_them_as_messages() {
    for generic in [false, true] {
        let events = protocol_error_events(named_responses(stepfun_responses_frames(generic, false)), false).await;
        assert!(!events.iter().any(|event| matches!(event, LlmEvent::Error(_))), "{events:?}");
        let thinking = events.iter().filter_map(|event| match event {
            LlmEvent::ThinkingDelta(text) => Some(text.as_str()), _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(thinking, ["用户要求问候。", "先核实工具结果。"]);
        assert!(events.iter().any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "你好")));
        assert!(events.iter().any(|event| matches!(event, LlmEvent::Done {stop_reason:StopReason::EndTurn,..})));
        let signature = events.iter().find_map(|event| match event { LlmEvent::ThinkingSignature(value) => Some(value), _ => None }).unwrap();
        let native: Value = serde_json::from_str(signature.strip_prefix("openai.responses.reasoning.v1:").unwrap()).unwrap();
        assert_eq!(native[0], json!({"id":"rs_step","type":"reasoning","summary":[],"content":null,"encrypted_content":null,"status":null}));
        assert!(!signature.contains("用户要求问候"), "streamed text must not replace the provider's null content");
    }
}

#[tokio::test]
async fn reasoning_part_identity_order_and_completed_text_stay_strict() {
    for mutation in 0..7 {
        let mut frames = stepfun_responses_frames(true, false);
        match mutation {
            0 => frames[2]["item_id"] = json!("different-item"),
            1 => frames[2]["part"]["type"] = json!("output_text"),
            2 => frames[2]["content_index"] = json!(128),
            3 => frames[5]["part"]["text"] = json!("changed completed text"),
            4 => { frames[6] = frames[3].clone(); frames[6]["sequence_number"] = json!(6); }
            5 => { frames[6] = frames[5].clone(); frames[6]["sequence_number"] = json!(6); }
            6 => { frames.pop(); }
            _ => unreachable!(),
        }
        let events = protocol_error_events(named_responses(frames), false).await;
        assert_failed_without_commit(&events);
    }
}

#[tokio::test]
async fn stepfun_reasoning_then_tool_roundtrip_keeps_actual_null_state_and_commits_only_at_terminal() {
    let server = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let attempts = calls.clone();
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(move |_: &Request| ResponseTemplate::new(200).set_body_raw(
            named_responses(stepfun_responses_frames(false, attempts.fetch_add(1, Ordering::SeqCst) == 0)), "text/event-stream",
        )).expect(2).mount(&server).await;
    let mut settings = compat(false);
    settings.extra_body = Some(json!({"include":null}).as_object().unwrap().clone());
    let provider = OpenAIResponsesProvider::new("key", &server.uri(), settings);
    let mut request = request(false);
    request.model = "step-5-preview".into();
    let first = collect(provider.stream(&request).await.unwrap()).await;
    assert!(!first.iter().any(|event| matches!(event, LlmEvent::Error(_))), "{first:?}");
    let signature = first.iter().find_map(|event| match event { LlmEvent::ThinkingSignature(value) => Some(value.clone()), _ => None }).unwrap();
    let tool = first.iter().position(|event| matches!(event, LlmEvent::ToolUse {id,name,input,..}
        if id=="call_step" && name=="get_weather" && input["city"]=="北京")).unwrap();
    assert!(matches!(first[tool+1],LlmEvent::Done {stop_reason:StopReason::ToolUse,..}));
    request.messages.extend([
        Message::new(Role::Assistant,vec![
            ContentBlock::Thinking {thinking:"用户要求问候。先核实工具结果。".into(),signature:Some(signature)},
            ContentBlock::ToolUse {id:"call_step".into(),name:"get_weather".into(),input:json!({"city":"北京"}),extra:None},
        ]),
        Message::new(Role::Tool,vec![ContentBlock::ToolResult {
            tool_use_id:"call_step".into(),content:"晴".into(),is_error:false,images:vec![],
        }]),
    ]);
    let second = collect(provider.stream(&request).await.unwrap()).await;
    assert!(second.iter().any(|event| matches!(event,LlmEvent::Done {stop_reason:StopReason::EndTurn,..})));
    let received=server.received_requests().await.unwrap();
    let body:Value=serde_json::from_slice(&received[1].body).unwrap();
    assert_eq!(body["input"][1],json!({"id":"rs_step","type":"reasoning","summary":[],"content":null,"encrypted_content":null,"status":null}));
    assert_eq!(body["input"][2]["type"],"function_call");
    assert_eq!(body["input"][2]["call_id"],"call_step");
    assert_eq!(body["input"][3],json!({"type":"function_call_output","call_id":"call_step","output":"晴"}));
    assert!(body.get("include").is_none());
}

#[tokio::test]
async fn incomplete_or_malformed_native_reasoning_wrappers_never_reach_transport() {
    let server=MockServer::start().await;
    let provider=OpenAIResponsesProvider::new("key",&server.uri(),compat(false));
    for item in [
        json!({"id":"rs_bad","type":"reasoning","status":"incomplete","content":null}),
        json!({"id":"rs_bad","type":"reasoning","content":[{"type":"output_text","text":"not reasoning"}]}),
        json!({"id":"rs_bad","type":"reasoning","content":[{"type":"reasoning_text","text":"x".repeat(512*1024)}]}),
    ] {
        let mut replay=request(false);
        replay.messages=vec![Message::new(Role::Assistant,vec![ContentBlock::Thinking {
            thinking:String::new(),signature:Some(format!("openai.responses.reasoning.v1:{}",json!([item]))),
        }])];
        assert!(matches!(provider.stream(&replay).await,Err(ProviderError::Config(_))));
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn compatible_data_only_content_parts_and_done_summary_complete_without_retention() {
    let events = protocol_error_events(data_only_responses(compatible_responses_frames()), false).await;
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::Error(_))), "{events:?}");
    assert!(events.iter().any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "hello")));
    assert!(events.iter().any(|event| matches!(event,
        LlmEvent::Done { stop_reason:StopReason::EndTurn, usage } if usage.input_tokens == 12 && usage.output_tokens == 3)));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ProviderRoundId(_))));
}

#[tokio::test]
async fn compatible_reasoning_without_encryption_never_manufactures_a_continuation() {
    let events = protocol_error_events(data_only_responses(vec![
        json!({"type":"response.created","response":{"id":"resp_plain_reasoning","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"id":"rs_plain","type":"reasoning","summary":[]}}),
        json!({"type":"response.reasoning_summary_text.done","output_index":0,"item_id":"rs_plain","summary_index":0,"text":"summary"}),
        json!({"type":"response.output_item.done","output_index":0,
            "item":{"id":"rs_plain","type":"reasoning","summary":[{"type":"summary_text","text":"summary"}]}}),
        json!({"type":"response.done","response":{"id":"resp_plain_reasoning","status":"completed"}}),
    ]), false).await;
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::Error(_))), "{events:?}");
    assert!(events.iter().any(|event| matches!(event, LlmEvent::ThinkingDelta(text) if text == "summary")));
    assert!(events.iter().any(|event| matches!(event, LlmEvent::Done { .. })));
    assert!(!events.iter().any(|event| matches!(event, LlmEvent::ThinkingSignature(_) | LlmEvent::ProviderRoundId(_))));
}

#[tokio::test]
async fn compatible_envelopes_do_not_invent_items_or_terminal_authority() {
    for mutation in 0..5 {
        let mut frames = compatible_responses_frames();
        match mutation {
            0 => { frames.remove(4); }
            1 => { frames[3]["response_id"] = json!("other-response"); }
            2 => { frames[3]["content_index"] = json!(1); }
            3 => { frames.pop(); }
            4 => { frames.push(json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"msg_compatible","delta":"poison"})); }
            _ => unreachable!(),
        }
        let events = protocol_error_events(data_only_responses(frames), false).await;
        assert_failed_without_commit(&events);
    }
    // A compatible response without store:true cannot authorize a cursor.
    let events = protocol_error_events(data_only_responses(compatible_responses_frames()), true).await;
    assert_failed_without_commit(&events);
}

#[tokio::test]
async fn function_outputs_keep_schema_refs_opaque_for_compatibility_gateways() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            completed_text("resp_tool_outputs", false, "ok"),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let schema = json!({
        "$defs": {"ModelRefParam": {"type": "object"}},
        "properties": {"model": {"$ref": "#/$defs/ModelRefParam"}}
    })
    .to_string();
    let safe = json!({"temperature": 27}).to_string();
    let mut request = request(false);
    request.messages = vec![
        Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "inspect tool outputs".to_owned(),
            }],
        ),
        Message::new(
            Role::Assistant,
            vec![
                ContentBlock::ToolUse {
                    id: "call_schema".to_owned(),
                    name: "get_schema".to_owned(),
                    input: json!({}),
                    extra: None,
                },
                ContentBlock::ToolUse {
                    id: "call_safe".to_owned(),
                    name: "get_weather".to_owned(),
                    input: json!({}),
                    extra: None,
                },
            ],
        ),
        Message::new(
            Role::User,
            vec![
                ContentBlock::ToolResult {
                    tool_use_id: "call_schema".to_owned(),
                    content: schema.clone(),
                    is_error: false,
                    images: Vec::new(),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "call_safe".to_owned(),
                    content: safe.clone(),
                    is_error: false,
                    images: Vec::new(),
                },
            ],
        ),
    ];

    let provider = OpenAIResponsesProvider::new("key", &server.uri(), compat(false));
    collect(provider.stream(&request).await.unwrap()).await;

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    let outputs = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .collect::<Vec<_>>();
    let schema_output = outputs
        .iter()
        .find(|item| item["call_id"] == "call_schema")
        .unwrap()["output"]
        .as_str()
        .unwrap();
    assert!(schema_output.starts_with(
        "Literal JSON tool result (`$ref` keys below are data, not media references):\n"
    ));
    assert!(schema_output.ends_with(&schema));
    assert!(serde_json::from_str::<Value>(schema_output).is_err());
    let safe_output = outputs
        .iter()
        .find(|item| item["call_id"] == "call_safe")
        .unwrap()["output"]
        .as_str()
        .unwrap();
    assert_eq!(safe_output, safe);
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
    let arguments = r#"{"path":"plugin.html"}"#;
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
            if id == "call_complete" && name == "Write" && input["path"] == "plugin.html"
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

#[tokio::test]
async fn reasoning_text_and_multiple_summary_parts_finish_independently() {
    let reasoning = json!({
        "id": "rs_parts",
        "type": "reasoning",
        "summary": [
            {"type": "summary_text", "text": "First summary."},
            {"type": "summary_text", "text": "Second summary."}
        ],
        "content": [{"type": "reasoning_text", "text": "Reasoning text."}],
        "encrypted_content": "opaque-parts"
    });
    let mut events = vec![
        created("resp_parts", false),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {"id": "rs_parts", "type": "reasoning", "summary": []}
            }),
        ),
    ];
    for (delta_type, done_type, index_field, part_index, text) in [
        ("response.reasoning_summary_text.delta", "response.reasoning_summary_text.done", "summary_index", 0, "First summary."),
        ("response.reasoning_summary_text.delta", "response.reasoning_summary_text.done", "summary_index", 1, "Second summary."),
        ("response.reasoning_text.delta", "response.reasoning_text.done", "content_index", 0, "Reasoning text."),
    ] {
        for (event_type, field) in [(delta_type, "delta"), (done_type, "text")] {
            let mut payload = json!({
                "type": event_type,
                "output_index": 0,
                "item_id": "rs_parts",
            });
            payload[index_field] = json!(part_index);
            payload[field] = json!(text);
            events.push((event_type, payload));
        }
    }
    events.extend([
        (
            "response.output_item.done",
            json!({"type": "response.output_item.done", "output_index": 0, "item": reasoning.clone()}),
        ),
        (
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {"id": "resp_parts", "status": "completed", "store": false, "output": [reasoning]}
            }),
        ),
    ]);
    let actual = protocol_error_events(sse(events), false).await;
    assert!(!actual.iter().any(|event| matches!(event, LlmEvent::Error(_))), "{actual:?}");
    let thinking = actual.iter().filter_map(|event| match event {
        LlmEvent::ThinkingDelta(text) => Some(text.as_str()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(thinking, ["First summary.", "Second summary.", "Reasoning text."]);
    assert!(actual.iter().any(|event| matches!(event, LlmEvent::ThinkingSignature(_))));
    assert!(actual.iter().any(|event| matches!(event, LlmEvent::Done { .. })));
}

#[tokio::test]
async fn reasoning_summary_indices_and_done_only_aggregate_are_bounded() {
    for (parts, expected_error) in [
        (vec![(128, "small".to_owned())], "summary_index"),
        (vec![(0, "a".repeat(270_000)), (1, "b".repeat(270_000))], "safety limit"),
    ] {
        let mut events = vec![
            created("resp_reasoning_bounds", false),
            (
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": {"id": "rs_bounds", "type": "reasoning", "summary": []}
                }),
            ),
        ];
        for (index, text) in parts {
            events.push((
                "response.reasoning_summary_text.done",
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "output_index": 0,
                    "item_id": "rs_bounds",
                    "summary_index": index,
                    "text": text
                }),
            ));
        }
        let actual = protocol_error_events(sse(events), false).await;
        assert_failed_without_commit(&actual);
        assert!(actual.iter().any(|event| matches!(
            event, LlmEvent::Error(message) if message.contains(expected_error)
        )));
    }
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

fn structured_api_error(events: &[LlmEvent]) -> Value {
    let message=events.iter().find_map(|event| match event {
        LlmEvent::Error(message) => Some(message), _ => None,
    }).expect("provider error event");
    let body=message.strip_prefix("API error 422: ").expect("structured API error envelope");
    serde_json::from_str(body).expect("complete machine-readable provider error JSON")
}

#[tokio::test]
async fn responses_error_events_preserve_machine_codes_types_params_and_human_messages() {
    for code in ["unsupported_protocol","authentication_error","insufficient_quota"] {
        let original=json!({"type":"error","code":code,"message":"The upstream request was rejected.",
            "param":"input","details":{"retry":false}});
        let events=protocol_error_events(sse(vec![("error",original.clone())]),false).await;
        assert_failed_without_commit(&events);
        assert_eq!(structured_api_error(&events),json!({"error":original}));
    }
}

#[tokio::test]
async fn failed_responses_preserve_original_error_before_optional_output_validation() {
    for original in [
        json!({"code":"unsupported_protocol","type":"invalid_request_error",
            "message":"This route does not implement Responses.","param":"input","details":{"supported":"chat"}}),
        json!("The upstream rejected this request."),
    ] {
        let events=protocol_error_events(sse(vec![
            created("resp_failed_machine",false),
            ("response.failed",json!({"type":"response.failed","response":{
                "id":"resp_failed_machine","status":"failed","error":original
            }})),
        ]),false).await;
        assert_failed_without_commit(&events);
        assert_eq!(structured_api_error(&events),json!({"error":original}));
    }
}

#[tokio::test]
async fn complete_responses_sse_error_json_is_redacted_without_losing_machine_evidence() {
    let server=MockServer::start().await;
    let secret="response-runtime-secret/+fixture";
    let payload=json!({"type":"error","code":"unsupported_protocol","message":"Request rejected.",
        "param":"input","credential_echo":secret});
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(vec![("error",payload)]),"text/event-stream"))
        .expect(1).mount(&server).await;
    let provider=OpenAIResponsesProvider::new(secret,&server.uri(),compat(false));
    let events=collect(provider.stream(&request(false)).await.unwrap()).await;
    assert_failed_without_commit(&events);
    let body=structured_api_error(&events);
    assert_eq!(body["error"]["code"],"unsupported_protocol");
    assert_eq!(body["error"]["credential_echo"],"[REDACTED]");
    assert!(!serde_json::to_string(&events.iter().filter_map(|event|match event {
        LlmEvent::Error(message)=>Some(message),_=>None,
    }).collect::<Vec<_>>()).unwrap().contains(secret));
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
