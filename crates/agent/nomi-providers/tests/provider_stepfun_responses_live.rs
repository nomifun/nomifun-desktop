//! Explicit opt-in regression for StepFun's actual Responses stream.
//! Credentials enter through one stdin line and are never formatted or logged.

use std::io;
use std::time::Duration;

use nomi_config::compat::ProviderCompat;
use nomi_providers::openai_responses::OpenAIResponsesProvider;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomi_types::tool::ToolDef;
use serde_json::{Value, json};

const SENTINEL: &str = "NOMIFUN_LIVE_STEPFUN_ECHO_OK";

fn error_class(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::Config(_) => "configuration",
        ProviderError::Http(_) | ProviderError::Connection(_) => "transport",
        ProviderError::Api { status: 400..=499, .. } => "http_client_status",
        ProviderError::Api { .. } => "http_server_status",
        ProviderError::RateLimited { .. } => "rate_limit",
        ProviderError::Parse(_) => "protocol_parse",
        ProviderError::PromptTooLong(_) => "context_limit",
        ProviderError::StreamTruncated(_) => "stream_truncated",
        ProviderError::NonApiResponse { .. } => "non_api_response",
    }
}

fn value_kind(value: Option<&Value>) -> &'static str {
    match value {
        None => "missing",
        Some(Value::Null) => "null",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
        Some(Value::String(_)) => "string",
        Some(_) => "scalar",
    }
}

async fn call(provider: &OpenAIResponsesProvider, request: &LlmRequest) -> Vec<LlmEvent> {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut receiver = match provider.stream(request).await {
            Ok(receiver) => receiver,
            Err(error) => panic!("live StepFun request failed: {}", error_class(&error)),
        };
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            assert!(!matches!(event, LlmEvent::Error(_)), "live StepFun stream failed: provider_stream_error");
            events.push(event);
        }
        events
    }).await.unwrap_or_else(|_| panic!("live StepFun request failed: timeout"))
}

#[tokio::test]
#[ignore = "requires NOMIFUN_LIVE_STEPFUN_RESPONSES_VERIFY=1 and one credential line on stdin"]
async fn stepfun_actual_reasoning_parts_survive_a_harmless_tool_roundtrip() {
    if std::env::var("NOMIFUN_LIVE_STEPFUN_RESPONSES_VERIFY").as_deref() != Ok("1") {
        return;
    }
    let mut line = String::new();
    let read = io::stdin().read_line(&mut line)
        .unwrap_or_else(|_| panic!("live StepFun credential stdin unavailable"));
    assert!(read > 0 && !line.trim().is_empty(), "live StepFun credential stdin is empty");
    let credential = line.trim_end_matches(['\r', '\n']);
    let mut compat = ProviderCompat::openai_responses_defaults();
    compat.chain_rounds = Some(false);
    compat.extra_body = Some(json!({"include":null,"tool_choice":"auto"}).as_object().unwrap().clone());
    let provider = OpenAIResponsesProvider::new(credential, "https://api.stepfun.com", compat);
    let mut request = LlmRequest {
        model:"step-5-preview".into(),
        system:format!("For this harmless protocol check, call nomifun_live_echo exactly once with sentinel {SENTINEL} before answering. After receiving its tool result, answer with exactly that result. Do not call another tool."),
        messages:vec![Message::new(Role::User,vec![ContentBlock::Text {text:"Run the harmless echo check.".into()}])],
        tools:vec![ToolDef {
            name:"nomifun_live_echo".into(),description:"Return the given sentinel unchanged; no side effects.".into(),
            input_schema:json!({"type":"object","properties":{"sentinel":{"type":"string","enum":[SENTINEL]}},
                "required":["sentinel"],"additionalProperties":false}),deferred:false,
        }],
        max_tokens:Some(4096),thinking:None,reasoning_effort:Some("low".into()),retain_provider_round:false,
    };
    let first = call(&provider, &request).await;
    assert!(matches!(first.last(),Some(LlmEvent::Done {stop_reason:StopReason::ToolUse,..})),
        "live StepFun first round did not finish with the requested tool");
    let calls = first.iter().filter_map(|event| match event {
        LlmEvent::ToolUse {id,name,input,extra} => Some((id.clone(),name.clone(),input.clone(),extra.clone())),
        _ => None,
    }).collect::<Vec<_>>();
    assert!(calls.len()==1 && calls[0].1=="nomifun_live_echo" && calls[0].2["sentinel"]==SENTINEL,
        "live StepFun tool proposal did not match the harmless sentinel schema");
    let thinking_deltas = first.iter().filter(|event| matches!(event,LlmEvent::ThinkingDelta(_))).count();
    let signature_count = first.iter().filter(|event| matches!(event,LlmEvent::ThinkingSignature(_))).count();
    println!("live_stepfun_native thinking_delta_count={} signature_count={}",thinking_deltas,signature_count);
    let signature = first.iter().find_map(|event| match event {
        LlmEvent::ThinkingSignature(value) => Some(value.clone()), _ => None,
    });
    if let Some(signature) = &signature {
        let native: Value = serde_json::from_str(signature.strip_prefix("openai.responses.reasoning.v1:")
            .unwrap_or_else(|| panic!("live StepFun continuation has an invalid envelope")))
            .unwrap_or_else(|_| panic!("live StepFun continuation has invalid native JSON"));
        let items = native.as_array()
            .unwrap_or_else(|| panic!("live StepFun continuation is not a native item array"));
        println!("live_stepfun_native shape_count={}",items.len());
        for item in items {
            let has_cipher = item.get("encrypted_content").and_then(Value::as_str).is_some_and(|value| !value.is_empty());
            println!("live_stepfun_native content_kind={} has_cipher={} status_kind={}",
                value_kind(item.get("content")),has_cipher,value_kind(item.get("status")));
        }
        assert!(!items.is_empty() && items.iter().all(|item|
            item["type"]=="reasoning"
                && item.get("id").and_then(Value::as_str).is_some_and(|value| !value.is_empty())
                && (item.get("content").is_some_and(|value| value.is_null() || value.is_array())
                    || item.get("encrypted_content").and_then(Value::as_str).is_some_and(|value| !value.is_empty()))),
            "live StepFun omitted valid actual native reasoning continuation");
    } else {
        assert!(thinking_deltas==0, "live StepFun emitted reasoning without native continuation");
    }
    let thinking = first.iter().filter_map(|event| match event {
        LlmEvent::ThinkingDelta(text) => Some(text.as_str()), _ => None,
    }).collect::<String>();
    let first_visible = first.iter().filter_map(|event| match event {
        LlmEvent::TextDelta(text) => Some(text.as_str()), _ => None,
    }).collect::<String>();
    let (id,name,input,extra) = calls.into_iter().next().unwrap();
    let replayed_native_reasoning = signature.is_some();
    let mut assistant_content = Vec::new();
    if let Some(signature) = signature {
        assistant_content.push(ContentBlock::Thinking {thinking,signature:Some(signature)});
    }
    if !first_visible.is_empty() {
        assistant_content.push(ContentBlock::Text {text:first_visible});
    }
    assistant_content.push(ContentBlock::ToolUse {id:id.clone(),name,input,extra});
    request.messages.extend([
        Message::new(Role::Assistant,assistant_content),
        Message::new(Role::Tool,vec![ContentBlock::ToolResult {
            tool_use_id:id,content:SENTINEL.into(),is_error:false,images:vec![],
        }]),
    ]);
    let second = call(&provider, &request).await;
    assert!(matches!(second.last(),Some(LlmEvent::Done {stop_reason:StopReason::EndTurn,..})),
        "live StepFun follow-up did not finish with visible text");
    let visible = second.iter().filter_map(|event| match event {
        LlmEvent::TextDelta(text) => Some(text.as_str()), _ => None,
    }).collect::<String>();
    assert!(visible.contains(SENTINEL), "live StepFun follow-up omitted the sentinel answer");
    println!("live_stepfun_responses outcome=passed model_rounds=2 native_reasoning_replay={}",
        if replayed_native_reasoning {"verified"} else {"not_required"});
}
