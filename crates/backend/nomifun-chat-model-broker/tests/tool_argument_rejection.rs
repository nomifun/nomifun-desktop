use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use async_trait::async_trait;
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::*;
use serde_json::{Value, json};

struct NoTransport;
#[async_trait]
impl ProviderTransport for NoTransport {
    async fn open_stream(&self, _: ProviderWireRequest, _: CredentialLease) -> Result<ProviderWireStream, ChatModelError> {
        panic!("decoder tests must not invoke transport")
    }
}

fn adapters() -> Vec<Box<dyn ChatProtocolAdapter>> {
    let transport = Arc::new(NoTransport);
    vec![Box::new(OpenAiChatAdapter::new(transport.clone())),
        Box::new(OpenAiResponsesAdapter::new(transport.clone())),
        Box::new(AnthropicAdapter::new(transport.clone())),
        Box::new(BedrockAdapter::new(transport.clone())),
        Box::new(VertexAdapter::new(transport.clone())),
        Box::new(GeminiAdapter::new(transport))]
}

fn frame(event: &str, data: Value) -> ProviderWireFrame {
    ProviderWireFrame { event: event.into(), data, diagnostic: None }
}

fn decode(adapter: &dyn ChatProtocolAdapter, frames: Vec<ProviderWireFrame>) -> Vec<ChatModelEvent> {
    let mut decoder = adapter.new_frame_decoder().unwrap();
    frames.into_iter().flat_map(|frame| decoder.decode_frame(frame).unwrap()).collect()
}

fn assert_mixed(events: &[ChatModelEvent]) {
    assert!(events.iter().any(|event| matches!(event, ChatModelEvent::ToolCallCompleted { call }
        if call.call_id.as_ref() == "good" && call.arguments.0 == json!({"x":1}))));
    let diagnostic = events.iter().find_map(|event| match event {
        ChatModelEvent::ToolCallRejected { call_id, name, diagnostic }
            if call_id.as_ref() == "bad" && name == "write_file" => Some(diagnostic),
        _ => None,
    }).expect("bad proposal remains an explicit rejection");
    diagnostic.validate().unwrap();
    assert!(!serde_json::to_string(diagnostic).unwrap().contains("PRIVATE"));
    assert!(!events.iter().any(|event| matches!(event, ChatModelEvent::ToolCallCompleted { call }
        if call.call_id.as_ref() == "bad")));
    assert_eq!(events.iter().filter(|event| matches!(event, ChatModelEvent::Usage { .. })).count(), 1);
    assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })));
}

#[test]
fn normalized_malformed_arguments_never_become_a_raw_wrapper_for_any_protocol() {
    for adapter in adapters() {
        for (arguments, category) in [(json!("{\"PRIVATE\":1}}"), ToolArgumentDiagnosticCategory::Syntax),
            (json!("{\"PRIVATE\":"), ToolArgumentDiagnosticCategory::IncompleteJson),
            (json!(["PRIVATE"]), ToolArgumentDiagnosticCategory::RootType)] {
            let events = decode(adapter.as_ref(), vec![frame("tool_call.completed", json!({
                "call_id":"bad", "name":"write_file", "arguments":arguments
            })), frame("finish", json!({"finish_reason":"tool_calls"}))]);
            assert!(matches!(&events[0], ChatModelEvent::ToolCallRejected { diagnostic, .. }
                if diagnostic.category == category), "{:?}: {events:?}", adapter.protocol());
            assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls })));
        }
    }
}

#[test]
fn openai_normal_terminal_preserves_a_mixed_batch_usage_and_identity() {
    let adapter = OpenAiChatAdapter::new(Arc::new(NoTransport));
    assert_mixed(&decode(&adapter, vec![frame("json", json!({
        "id":"same-response", "choices":[{"index":0, "message":{"tool_calls":[
            {"id":"good","type":"function","function":{"name":"write_file","arguments":"{\"x\":1}"}},
            {"id":"bad","type":"function","function":{"name":"write_file","arguments":"{\"PRIVATE\":1}}"}}
        ]}, "finish_reason":"tool_calls"}], "usage":{"prompt_tokens":4,"completion_tokens":5}
    }))]));
}

#[test]
fn native_responses_and_message_adapters_preserve_mixed_batches() {
    let transport = Arc::new(NoTransport);
    let responses = OpenAiResponsesAdapter::new(transport.clone());
    assert_mixed(&decode(&responses, vec![frame("json", json!({
        "id":"resp_same", "object":"response", "status":"completed", "output":[
            {"id":"item_good","type":"function_call","status":"completed","call_id":"good","name":"write_file","arguments":"{\"x\":1}"},
            {"id":"item_bad","type":"function_call","status":"completed","call_id":"bad","name":"write_file","arguments":"{\"PRIVATE\":"}
        ], "usage":{"input_tokens":4,"output_tokens":5}
    }))]));
    let messages: Vec<Box<dyn ChatProtocolAdapter>> = vec![
        Box::new(AnthropicAdapter::new(transport.clone())), Box::new(BedrockAdapter::new(transport.clone())),
        Box::new(VertexAdapter::new(transport))];
    for adapter in messages {
        assert_mixed(&decode(adapter.as_ref(), vec![frame("json", json!({
            "id":"msg_same", "type":"message", "role":"assistant", "stop_reason":"tool_use", "content":[
                {"type":"tool_use","id":"good","name":"write_file","input":{"x":1}},
                {"type":"tool_use","id":"bad","name":"write_file","input":["PRIVATE"]}
            ], "usage":{"input_tokens":4,"output_tokens":5}
        }))]));
    }
}

#[test]
fn gemini_root_type_rejection_waits_for_normal_terminal() {
    let adapter = GeminiAdapter::new(Arc::new(NoTransport));
    let frames = vec![frame("json", json!({"responseId":"gemini_same", "candidates":[{
        "content":{"parts":[
            {"functionCall":{"id":"good","name":"write_file","args":{"x":1}}},
            {"functionCall":{"id":"bad","name":"write_file","args":["PRIVATE"]}}
        ]}, "finishReason":"STOP"}], "usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":5}
    }))];
    assert_mixed(&decode(&adapter, frames));
}

#[test]
fn length_terminal_never_promotes_raw_chat_or_gemini_arguments() {
    let chat = OpenAiChatAdapter::new(Arc::new(NoTransport));
    let chat_events = decode(&chat, vec![frame("json", json!({"id":"length", "choices":[{
        "index":0,"message":{"tool_calls":[{"id":"bad","type":"function","function":{
            "name":"write_file","arguments":"{\"PRIVATE\":"}}]}, "finish_reason":"length"}],
        "usage":{"prompt_tokens":4,"completion_tokens":5}}))]);
    let gemini = GeminiAdapter::new(Arc::new(NoTransport));
    let gemini_events = decode(&gemini, vec![frame("json", json!({"responseId":"length", "candidates":[{
        "content":{"parts":[{"functionCall":{"id":"bad","name":"write_file","args":["PRIVATE"]}}]},
        "finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":4,"candidatesTokenCount":5}}))]);
    for events in [chat_events, gemini_events] {
        assert!(events.iter().any(|event| matches!(event, ChatModelEvent::ToolCallDelta { call_id, .. }
            if call_id.as_ref() == "bad")), "truncated proposals keep their identities");
        assert!(!events.iter().any(|event| matches!(event, ChatModelEvent::ToolCallCompleted { .. } | ChatModelEvent::ToolCallRejected { .. })));
        assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::MaxOutputTokens })));
    }
}

#[test]
fn messages_streamed_invalid_arguments_allow_later_blocks_and_terminal_precedence() {
    for (arguments, category) in [("{\"PRIVATE\":1}}", ToolArgumentDiagnosticCategory::Syntax),
        ("{\"PRIVATE\":", ToolArgumentDiagnosticCategory::IncompleteJson)] {
        for reason in ["tool_use", "max_tokens"] {
            let adapter = AnthropicAdapter::new(Arc::new(NoTransport));
            let mut frames = vec![frame("message_start", json!({"type":"message_start", "message":{
                "id":"same","type":"message","role":"assistant","content":[],"stop_reason":null,
                "usage":{"input_tokens":4,"output_tokens":0}
            }})), frame("content_block_start", json!({"type":"content_block_start","index":0,
                "content_block":{"type":"tool_use","id":"bad","name":"write_file","input":{}}})),
                frame("content_block_delta", json!({"type":"content_block_delta","index":0,
                    "delta":{"type":"input_json_delta","partial_json":arguments}})),
                frame("content_block_stop", json!({"type":"content_block_stop","index":0}))];
            if reason == "tool_use" {
                frames.extend([frame("content_block_start", json!({"type":"content_block_start","index":1,
                    "content_block":{"type":"tool_use","id":"good","name":"write_file","input":{"x":1}}})),
                    frame("content_block_stop", json!({"type":"content_block_stop","index":1}))]);
            }
            frames.extend([frame("message_delta", json!({"type":"message_delta","delta":{"stop_reason":reason},
                "usage":{"output_tokens":5}})), frame("message_stop", json!({"type":"message_stop"}))]);
            let events = decode(&adapter, frames);
            assert!(events.iter().any(|event| matches!(event, ChatModelEvent::ToolCallRejected { diagnostic, .. }
                if diagnostic.category == category)));
            if reason == "tool_use" { assert_mixed(&events); }
            else { assert!(matches!(events.last(), Some(ChatModelEvent::Completed { finish_reason: ChatFinishReason::MaxOutputTokens }))); }
        }
    }
}

#[test]
fn protocol_identity_and_argument_source_contradictions_still_fail_closed() {
    let responses = OpenAiResponsesAdapter::new(Arc::new(NoTransport));
    let mut decoder = responses.new_frame_decoder().unwrap();
    decoder.decode_frame(frame("response.created", json!({"type":"response.created","response":{"id":"resp"}}))).unwrap();
    decoder.decode_frame(frame("response.output_item.added", json!({"type":"response.output_item.added","output_index":0,
        "item":{"type":"function_call","id":"item","call_id":"bad","name":"write_file","arguments":"{"}}))).unwrap();
    let error = decoder.decode_frame(frame("response.output_item.done", json!({"type":"response.output_item.done","output_index":0,
        "item":{"type":"function_call","id":"item","call_id":"different","name":"write_file","arguments":"{}","status":"completed"}}))).unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
    let messages = AnthropicAdapter::new(Arc::new(NoTransport));
    let mut decoder = messages.new_frame_decoder().unwrap();
    decoder.decode_frame(frame("message_start", json!({"type":"message_start","message":{"id":"msg","type":"message",
        "role":"assistant","content":[],"stop_reason":null,"usage":{"input_tokens":1,"output_tokens":0}}}))).unwrap();
    decoder.decode_frame(frame("content_block_start", json!({"type":"content_block_start","index":0,
        "content_block":{"type":"tool_use","id":"bad","name":"write_file","input":{"x":1}}}))).unwrap();
    let error = decoder.decode_frame(frame("content_block_delta", json!({"type":"content_block_delta","index":0,
        "delta":{"type":"input_json_delta","partial_json":"{"}}))).unwrap_err();
    assert_eq!(error.code, ChatModelErrorCode::ProtocolViolation);
}

struct CountingParser(Arc<AtomicUsize>);
impl ToolArgumentParser for CountingParser {
    fn parse(&self, input: ToolArgumentInput<'_>) -> Result<StrictJsonValue, ToolArgumentDiagnostic> {
        self.0.fetch_add(1, Ordering::SeqCst);
        JsonToolArgumentParser.parse(input)
    }
}

#[test]
fn parser_injection_and_attempt_state_are_isolated_even_when_ids_repeat() {
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = OpenAiChatAdapter::with_argument_parser(Arc::new(NoTransport), Arc::new(CountingParser(calls.clone())));
    let mut first = adapter.new_frame_decoder().unwrap();
    let mut second = adapter.new_frame_decoder().unwrap();
    first.decode_frame(frame("message", json!({"id":"same", "choices":[{"index":0,"delta":{"tool_calls":[
        {"index":0,"id":"bad","function":{"name":"write_file","arguments":"{"}}
    ]}}]}))).unwrap();
    let healthy = second.decode_frame(frame("json", json!({"id":"same", "choices":[{"index":0,
        "message":{"tool_calls":[{"id":"good","type":"function","function":{"name":"write_file","arguments":"{\"x\":1}"}}]},
        "finish_reason":"tool_calls"}]}))).unwrap();
    assert!(healthy.iter().any(|event| matches!(event, ChatModelEvent::ToolCallCompleted { call } if call.call_id.as_ref() == "good")));
    let rejected = first.decode_frame(frame("json", json!({"id":"same", "choices":[{"index":0,"finish_reason":"tool_calls"}]}))).unwrap();
    assert!(rejected.iter().any(|event| matches!(event, ChatModelEvent::ToolCallRejected { call_id, .. } if call_id.as_ref() == "bad")));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
