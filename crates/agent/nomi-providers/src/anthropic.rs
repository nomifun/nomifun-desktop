use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use nomi_config::compat::{self, ProviderCompat};
use nomi_types::llm::{LlmEvent, LlmRequest, ThinkingConfig};

use super::anthropic_shared;
use crate::{LlmProvider, ProviderError};

pub struct AnthropicProvider {
    api_keys: Vec<String>,
    current_api_key: AtomicUsize,
    base_url: String,
    cache_enabled: bool,
    compat: ProviderCompat,
    sanitize_tool_schemas: AtomicBool,
}

impl AnthropicProvider {
    pub fn new(api_key: &str, base_url: &str, compat: ProviderCompat) -> Self {
        Self {
            api_keys: crate::parse_api_keys(api_key),
            current_api_key: AtomicUsize::new(0),
            base_url: base_url.to_string(),
            cache_enabled: true,
            compat,
            sanitize_tool_schemas: AtomicBool::new(false),
        }
    }

    fn should_sanitize_tool_schemas(&self) -> bool {
        self.compat.sanitize_schema() || self.sanitize_tool_schemas.load(Ordering::Acquire)
    }

    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.cache_enabled = enabled;
        self
    }

    fn build_headers(&self, api_key: &str) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        let api_key = HeaderValue::from_str(api_key)
            .map_err(|e| ProviderError::Connection(format!("Invalid x-api-key header: {}", e)))?;
        headers.insert("x-api-key", api_key);
        headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if self.cache_enabled {
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static("prompt-caching-2024-07-31"),
            );
        }
        Ok(headers)
    }

    fn build_request_body(&self, request: &LlmRequest, sanitize_tool_schemas: bool) -> Value {
        // Build system prompt with optional cache_control
        let system = if self.cache_enabled {
            json!([{
                "type": "text",
                "text": &request.system,
                "cache_control": { "type": "ephemeral" }
            }])
        } else {
            json!(&request.system)
        };

        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "system": system,
            "messages": anthropic_shared::build_messages(&request.messages, &self.compat),
            "stream": true
        });

        if !request.tools.is_empty() {
            let mut tools = anthropic_shared::build_tools(&request.tools);
            if sanitize_tool_schemas {
                for tool in &mut tools {
                    if let Some(schema) = tool.get("input_schema").cloned() {
                        tool["input_schema"] = compat::sanitize_json_schema(&schema);
                    }
                }
            }
            // Mark last tool with cache_control to cache the entire tools block
            if let Some(last) = tools.last_mut().filter(|_| self.cache_enabled) {
                last["cache_control"] = json!({ "type": "ephemeral" });
            }
            body["tools"] = json!(tools);
        }

        if let Some(ThinkingConfig::Enabled { budget_tokens }) = &request.thinking {
            body["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget_tokens
            });
        }

        let mut body = crate::request_body_with_extra(&self.compat, body);
        let object = body
            .as_object_mut()
            .expect("typed Anthropic request body is an object");
        if request.tools.is_empty() {
            object.remove("tools");
        }
        if request.thinking.is_none() {
            object.remove("thinking");
        }
        body
    }

    async fn send_initial_with_key_rotation(
        &self,
        client: &reqwest::Client,
        url: &str,
        body: &Value,
    ) -> Result<(reqwest::Response, HeaderMap), ProviderError> {
        crate::send_initial_with_key_rotation(
            client,
            url,
            body,
            &self.api_keys,
            &self.current_api_key,
            "anthropic",
            |api_key| self.build_headers(api_key),
        )
        .await
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        // `base_url + api_path` is resolved exactly once. NomiFun's task
        // capability resolver supplies a complete endpoint as `base_url` and
        // an explicit empty path; standalone Anthropic configuration receives
        // `/v1/messages` from `anthropic_defaults`.
        let url = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            self.compat.api_path()
        );
        let client = crate::http_client()?;
        let sanitize_tool_schemas = self.should_sanitize_tool_schemas();
        let mut body = self.build_request_body(request, sanitize_tool_schemas);

        tracing::debug!(target: "nomi_providers", body = %serde_json::to_string_pretty(&body).unwrap_or_default(), "outgoing request");

        let (response, headers) = match self
            .send_initial_with_key_rotation(&client, &url, &body)
            .await
        {
            Ok(result) => result,
            Err(error)
                if !request.tools.is_empty()
                    && !sanitize_tool_schemas
                    && error.is_tool_schema_incompatible() =>
            {
                let ProviderError::Api { status, .. } = &error else {
                    unreachable!("schema classifier only accepts API errors");
                };
                tracing::warn!(
                    target: "nomi_providers",
                    provider = "anthropic",
                    status,
                    "provider rejected tool schemas; retrying with Bedrock-compatible schema roots"
                );
                body = self.build_request_body(request, true);
                let (response, headers) = self
                    .send_initial_with_key_rotation(&client, &url, &body)
                    .await?;
                self.sanitize_tool_schemas.store(true, Ordering::Release);
                (response, headers)
            }
            Err(error) => return Err(error),
        };

        let (tx, rx) = mpsc::channel(64);
        let client = client.clone();
        let url_clone = url.clone();
        let redactor = nomifun_net::secret_redaction::SecretRedactor::new(&self.api_keys);

        tokio::spawn(async move {
            let Some(outcome) = crate::retry::until_receiver_closed(
                &tx,
                anthropic_shared::process_sse_stream(response, &tx),
            )
            .await
            else {
                return;
            };
            crate::retry::finish_stream_with_retry(
                outcome,
                &tx,
                || {
                    crate::retry::send_and_check(
                        &client,
                        &url_clone,
                        &headers,
                        &body,
                        &redactor,
                    )
                },
                |resp| anthropic_shared::process_sse_stream(resp, &tx),
            )
            .await;
        });

        Ok(rx)
    }
}
