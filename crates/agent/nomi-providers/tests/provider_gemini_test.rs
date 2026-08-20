use nomi_config::compat::ProviderCompat;
use nomi_providers::gemini::GeminiProvider;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{ContentBlock, Message, Role, StopReason};
use nomi_types::tool::{ToolDef, ToolImage};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn minimal_request() -> LlmRequest {
    LlmRequest {
        model: "gemini-3.6-flash".to_owned(),
        system: "You are helpful.".to_owned(),
        messages: vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "Hello".to_owned(),
            }],
        )],
        tools: Vec::new(),
        max_tokens: Some(512),
        thinking: None,
        reasoning_effort: None,
        retain_provider_round: false,
    }
}

fn text_sse() -> String {
    let first = json!({
        "candidates": [{
            "content": { "role": "model", "parts": [{ "text": "Hello " }] },
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": 12,
            "candidatesTokenCount": 2,
            "cachedContentTokenCount": 2
        }
    });
    let final_frame = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [
                    { "text": "world" },
                    { "text": "", "thoughtSignature": "final-signature" }
                ]
            },
            "finishReason": "STOP",
            "index": 0
        }],
        "usageMetadata": {
            "promptTokenCount": 12,
            "candidatesTokenCount": 8,
            "thoughtsTokenCount": 3,
            "cachedContentTokenCount": 2
        }
    });
    format!("data: {first}\n\ndata: {final_frame}\n\n")
}

async fn collect_events(mut receiver: tokio::sync::mpsc::Receiver<LlmEvent>) -> Vec<LlmEvent> {
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn native_multi_key_rotates_after_auth_failure() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
        ))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "rejected-key"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {
                "code": 401,
                "status": "UNAUTHENTICATED",
                "message": "API key not valid"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
        ))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "working-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(text_sse(), "text/event-stream"),
        )
        .expect(2)
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(
        " rejected-key,\n working-key ",
        &format!("{}/v1beta", server.uri()),
        ProviderCompat::gemini_defaults(),
    );
    for _ in 0..2 {
        let events = collect_events(provider.stream(&minimal_request()).await.unwrap()).await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, LlmEvent::TextDelta(text) if text == "Hello "))
        );
    }
    server.verify().await;
}

#[tokio::test]
async fn native_extra_body_preserves_unknown_fields_but_typed_fields_win() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(text_sse(), "text/event-stream"),
        )
        .expect(2)
        .mount(&server)
        .await;

    let mut compat = ProviderCompat::gemini_defaults();
    compat.extra_body = Some(
        json!({
            "safetySettings": [{"category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_ONLY_HIGH"}],
            "generationConfig": {
                "temperature": 0.45,
                "maxOutputTokens": 1
            },
            "contents": [{"role": "user", "parts": [{"text": "must-not-win"}]}],
            "model": "must-not-enter-body",
            "tools": [{"functionDeclarations":[{"name":"must-not-survive"}]}],
            "systemInstruction": {"parts":[{"text":"must-not-win"}]}
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let provider = GeminiProvider::new(
        "test-key",
        &format!("{}/v1beta", server.uri()),
        compat,
    );
    collect_events(provider.stream(&minimal_request()).await.unwrap()).await;
    let mut omitted = minimal_request();
    omitted.max_tokens = None;
    collect_events(provider.stream(&omitted).await.unwrap()).await;

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["safetySettings"][0]["threshold"], "BLOCK_ONLY_HIGH");
    assert_eq!(body["generationConfig"]["temperature"], 0.45);
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 512);
    assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello");
    assert!(body.get("model").is_none());
    assert!(body.get("tools").is_none());
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        "You are helpful."
    );
    let omitted_body: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(omitted_body["generationConfig"]["temperature"], 0.45);
    assert!(
        omitted_body["generationConfig"]
            .get("maxOutputTokens")
            .is_none()
    );
}

#[tokio::test]
async fn native_request_preserves_multimodal_tools_ids_and_signatures() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-3.6-flash:streamGenerateContent",
        ))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", "test-gemini-key"))
        .and(header("content-type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(text_sse(), "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut request = minimal_request();
    request.messages = vec![
        Message::new(
            Role::System,
            vec![ContentBlock::Text {
                text: "Use metric units.".to_owned(),
            }],
        ),
        Message::new(
            Role::User,
            vec![
                ContentBlock::Text {
                    text: "Inspect this image".to_owned(),
                },
                ContentBlock::Image {
                    media_type: "image/png".to_owned(),
                    data: "USER_IMAGE".to_owned(),
                },
            ],
        ),
        Message::new(
            Role::Assistant,
            vec![ContentBlock::ToolUse {
                id: "call-weather-1".to_owned(),
                name: "get_weather".to_owned(),
                input: json!({ "city": "Shanghai" }),
                extra: Some(json!({ "thoughtSignature": "tool-signature" })),
            }],
        ),
        Message::new(
            Role::User,
            vec![ContentBlock::ToolResult {
                tool_use_id: "call-weather-1".to_owned(),
                content: json!({ "temperature": 27 }).to_string(),
                is_error: false,
                images: vec![ToolImage {
                    media_type: "image/jpeg".to_owned(),
                    data: "TOOL_IMAGE".to_owned(),
                }],
            }],
        ),
    ];
    request.tools = vec![ToolDef {
        name: "get_weather".to_owned(),
        description: "Get weather".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"],
            "additionalProperties": false
        }),
        deferred: false,
    }];

    let provider = GeminiProvider::new(
        "test-gemini-key",
        &format!("{}/v1beta", server.uri()),
        ProviderCompat::gemini_defaults(),
    );
    let events = collect_events(provider.stream(&request).await.unwrap()).await;

    let text = events
        .iter()
        .filter_map(|event| match event {
            LlmEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "Hello world");
    assert!(events.iter().any(
        |event| matches!(event, LlmEvent::ThinkingSignature(signature) if signature == "final-signature")
    ));
    assert!(matches!(
        events.last(),
        Some(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage
        }) if usage.input_tokens == 12
            && usage.output_tokens == 11
            && usage.cache_read_tokens == 2
    ));

    let requests = server.received_requests().await.unwrap();
    let received = &requests[0];
    assert!(received.headers.get("authorization").is_none());
    assert_eq!(received.url.query(), Some("alt=sse"));
    let body: Value = serde_json::from_slice(&received.body).unwrap();
    assert!(body.get("model").is_none());
    assert_eq!(body["generationConfig"]["maxOutputTokens"], 512);
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "You are helpful.");
    assert_eq!(body["systemInstruction"]["parts"][1]["text"], "Use metric units.");
    assert_eq!(body["contents"][0]["parts"][1]["inlineData"]["data"], "USER_IMAGE");
    assert_eq!(body["contents"][1]["parts"][0]["functionCall"]["id"], "call-weather-1");
    assert_eq!(body["contents"][1]["parts"][0]["thoughtSignature"], "tool-signature");
    assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["id"], "call-weather-1");
    assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["name"], "get_weather");
    assert_eq!(body["contents"][2]["parts"][0]["functionResponse"]["parts"][0]["inlineData"]["data"], "TOOL_IMAGE");
    assert_eq!(body["tools"][0]["functionDeclarations"][0]["name"], "get_weather");
    assert!(body["tools"][0]["functionDeclarations"][0]["parameters"]
        .get("additionalProperties")
        .is_none());
}

#[tokio::test]
async fn streamed_function_call_is_atomically_committed_with_provider_metadata() {
    let server = MockServer::start().await;
    let frame = json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{
                    "functionCall": {
                        "id": "gemini-call-42",
                        "name": "Read",
                        "args": { "path": "README.md" }
                    },
                    "thoughtSignature": "opaque-signature"
                }]
            },
            "finishReason": "STOP"
        }],
        "usageMetadata": {
            "promptTokenCount": 20,
            "candidatesTokenCount": 5
        }
    });
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(format!("data: {frame}\n\n"), "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(
        "test-key",
        &format!("{}/v1beta", server.uri()),
        ProviderCompat::gemini_defaults(),
    );
    let events = collect_events(provider.stream(&minimal_request()).await.unwrap()).await;

    assert!(matches!(
        &events[0],
        LlmEvent::ToolUse { id, name, input, extra }
            if id == "gemini-call-42"
                && name == "Read"
                && input["path"] == "README.md"
                && extra.as_ref().and_then(|value| value["thoughtSignature"].as_str())
                    == Some("opaque-signature")
    ));
    assert!(matches!(
        events.last(),
        Some(LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage
        }) if usage.input_tokens == 20 && usage.output_tokens == 5
    ));
}

#[tokio::test]
async fn google_context_overflow_is_prompt_too_long_not_generic_400() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "code": 400,
                "status": "INVALID_ARGUMENT",
                "message": "The input token count (2000000) exceeds the maximum number of tokens allowed (1048576)."
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(
        "test-key",
        &format!("{}/v1beta", server.uri()),
        ProviderCompat::gemini_defaults(),
    );
    let error = provider.stream(&minimal_request()).await.unwrap_err();
    assert!(
        matches!(error, ProviderError::PromptTooLong(message) if message.contains("input token count"))
    );
}

#[tokio::test]
async fn prompt_block_in_successful_sse_is_terminal_error_without_done() {
    let server = MockServer::start().await;
    let frame = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(format!("data: {frame}\n\n"), "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let provider = GeminiProvider::new(
        "test-key",
        &format!("{}/v1beta", server.uri()),
        ProviderCompat::gemini_defaults(),
    );
    let events = collect_events(provider.stream(&minimal_request()).await.unwrap()).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], LlmEvent::Error(message) if message.contains("SAFETY")));
}
