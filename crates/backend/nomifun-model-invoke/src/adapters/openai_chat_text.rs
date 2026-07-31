//! `openai.chat_text` — OpenAI-compatible synchronous non-streaming
//! `/chat/completions` (ported from
//! `nomifun-creation/src/adapters/openai_chat.rs`, narrowed to the typed
//! [`ChatTextRequest`]: single-turn text in, text out — no multimodal inputs
//! on this path).
//!
//! `POST` the dispatch target (conventionally `{base}/v1/chat/completions`).
//! [`ChatTextRequest::prompt`] is the user message; a non-blank
//! [`ChatTextRequest::system`] prepends a system message. `extra.max_tokens`
//! is forwarded only when present (the port source read the same key off its
//! opaque params). The reply is read from `choices[0].message.content` →
//! [`TaskResult::Text`].

use std::time::Duration;

use async_trait::async_trait;
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};

use crate::adapter::ProtocolAdapter;
use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::transport::{error_from_response, post_json};
use crate::types::{ChatTextRequest, TaskOutcome, TaskRequest, TaskResult};

/// Text generation is usually fast; reasoning models can be slower.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI-compatible sync non-streaming `/chat/completions` protocol.
pub struct OpenAiChatTextAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAiChatTextAdapter {
    fn id(&self) -> &'static str {
        "openai.chat_text"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Chat
    }

    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
        let TaskRequest::ChatText(req) = &call.request else {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("openai.chat_text cannot serve task {:?}", call.request.task()),
            ));
        };
        let url = call.dispatch_target().url;
        let body = build_chat_body(&call.model, req);

        let resp = post_json(http, &url, REQUEST_TIMEOUT, &call.connection.auth, &body).await?;
        if !resp.status().is_success() {
            return Err(error_from_response(resp).await);
        }
        let value: Value =
            resp.json().await.map_err(|e| InvokeError::parse(format!("invalid chat JSON: {e}")))?;
        Ok(TaskOutcome::Done(TaskResult::Text(parse_chat_response(&value)?)))
    }
}

/// Build the `chat/completions` request body from the typed request. Pure —
/// unit tested.
///
/// - A non-blank `system` (trimmed) → a leading `system` message.
/// - `prompt` is the plain-string user message content.
/// - `extra.max_tokens` (number) is forwarded only when present.
pub(crate) fn build_chat_body(model: &str, req: &ChatTextRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = req.system.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": req.prompt}));

    let mut body = json!({ "model": model, "messages": messages });
    if let Some(max) = req.extra.get("max_tokens").and_then(|v| v.as_u64()) {
        body["max_tokens"] = json!(max);
    }
    body
}

/// Extract the assistant reply from a `chat/completions` body:
/// `choices[0].message.content`. Content may be a plain string or an array of
/// `{type:"text",text}` segments (concatenated). Pure — unit tested.
pub(crate) fn parse_chat_response(value: &Value) -> Result<String, InvokeError> {
    let choices = value
        .get("choices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InvokeError::parse("chat response missing 'choices' array"))?;
    let first = choices.first().ok_or_else(|| InvokeError::parse("chat response 'choices' is empty"))?;
    let content = first
        .get("message")
        .and_then(|m| m.get("content"))
        .ok_or_else(|| InvokeError::parse("chat choice missing message.content"))?;

    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        return Err(InvokeError::parse("chat response produced empty content"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::test_support::call;

    fn chat(prompt: &str, system: Option<&str>, extra: Value) -> ChatTextRequest {
        ChatTextRequest { prompt: prompt.into(), system: system.map(str::to_string), extra }
    }

    // -- ported pure-parser / body-builder fixtures -------------------------

    #[test]
    fn parse_string_content() {
        let v = json!({"choices": [{"message": {"role": "assistant", "content": "hello world"}}]});
        assert_eq!(parse_chat_response(&v).unwrap(), "hello world");
    }

    #[test]
    fn parse_array_content_concatenates() {
        let v = json!({"choices": [{"message": {"content": [
            {"type": "text", "text": "foo "},
            {"type": "text", "text": "bar"}
        ]}}]});
        assert_eq!(parse_chat_response(&v).unwrap(), "foo bar");
    }

    #[test]
    fn parse_errors_on_missing_or_empty() {
        for bad in [
            json!({}),
            json!({"choices": []}),
            json!({"choices": [{}]}),
            json!({"choices": [{"message": {"content": ""}}]}),
            json!({"choices": [{"message": {"content": "   "}}]}),
        ] {
            let err = parse_chat_response(&bad).unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::ParseError, "input {bad}");
        }
    }

    #[test]
    fn body_plain_user_message_by_default() {
        let body = build_chat_body("gpt-4o", &chat("say hi", None, json!({})));
        assert_eq!(body["model"], "gpt-4o");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "say hi");
        // max_tokens omitted by default
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn body_prepends_system_and_forwards_extra_max_tokens() {
        let body = build_chat_body("m", &chat("hi", Some("  be terse "), json!({"max_tokens": 128})));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "be terse"); // trimmed
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(body["max_tokens"], 128);
    }

    #[test]
    fn body_blank_system_is_ignored() {
        let body = build_chat_body("m", &chat("hi", Some("   "), json!({})));
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    // -- wiremock request/response tests ------------------------------------

    #[tokio::test]
    async fn chat_posts_messages_and_returns_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "be terse"},
                    {"role": "user", "content": "say hi"},
                ],
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "hello from the model"}}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let request = TaskRequest::ChatText(chat("say hi", Some("be terse"), json!({})));
        let call = call(&server.uri(), "gpt-4o-mini", request);
        let out = OpenAiChatTextAdapter.submit(&reqwest::Client::new(), &call).await.unwrap();
        let TaskOutcome::Done(TaskResult::Text(text)) = out else { panic!("expected Done(Text)") };
        assert_eq!(text, "hello from the model");
    }

    #[tokio::test]
    async fn upstream_401_maps_to_auth_kind() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;

        let request = TaskRequest::ChatText(chat("hi", None, json!({})));
        let call = call(&server.uri(), "gpt-4o-mini", request);
        let err = OpenAiChatTextAdapter.submit(&reqwest::Client::new(), &call).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert_eq!(err.http_status, Some(401));
    }
}
