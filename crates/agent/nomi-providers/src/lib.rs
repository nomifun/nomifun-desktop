pub mod anthropic;
pub mod anthropic_shared;
pub mod bedrock;
pub mod gemini;
pub mod openai;
pub mod retry;
pub mod vertex;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_net::secret_redaction::SecretRedactor;
use reqwest::header::HeaderMap;
use serde_json::Value;
use tokio::sync::mpsc;

use nomi_config::config::{Config, ProviderType};
use nomi_config::compat::ProviderCompat;
use nomi_types::llm::{LlmEvent, LlmRequest};

const MAX_DOUBLE_ENCODED_TOOL_ARGUMENT_BYTES: usize = 512 * 1024;
const MAX_PROVIDER_ERROR_BODY_BYTES: usize = 16 * 1024;
const PROVIDER_ERROR_BODY_READ_TIMEOUT: Duration = Duration::from_secs(2);
const TRUNCATED_PROVIDER_ERROR_BODY: &str = "\n[provider error body truncated]";

fn merge_json_value(target: &mut Value, incoming: &Value) {
    match (target, incoming) {
        (Value::Object(target), Value::Object(incoming)) => {
            for (key, value) in incoming {
                match target.get_mut(key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, incoming) => *target = incoming.clone(),
    }
}

/// Merge provider-native body extensions first, then recursively overlay the
/// serializer's typed protocol body. Unknown extensions survive while typed
/// model/messages/tools/token fields always remain authoritative.
pub(crate) fn request_body_with_extra(compat: &ProviderCompat, typed: Value) -> Value {
    let mut body = Value::Object(compat.extra_body());
    merge_json_value(&mut body, &typed);
    body
}

/// Unified interface for LLM API providers
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(&self, request: &LlmRequest)
    -> Result<mpsc::Receiver<LlmEvent>, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("HTTP error: {0}")]
    Http(reqwest::Error),
    #[error("API error {status}: {message}")]
    Api { status: u16, message: String },
    #[error("SSE parse error: {0}")]
    Parse(String),
    #[error("Rate limited, retry after {retry_after_ms}ms: {message}")]
    RateLimited {
        retry_after_ms: u64,
        message: String,
    },
    #[error("Prompt too long: {0}")]
    PromptTooLong(String),
    #[error("Connection error: {0}")]
    Connection(String),
    /// The HTTP transport closed cleanly, but the provider never emitted the
    /// protocol's commit marker. This is retryable only while no replay-unsafe
    /// content has crossed the provider boundary; the stream outcome carries
    /// that separate empty/partial distinction.
    #[error("Provider stream truncated: {0}")]
    StreamTruncated(String),
}

impl ProviderError {
    /// Remove every exact runtime credential representation before an error
    /// crosses the provider boundary or is written to a log/health record.
    pub(crate) fn redacted(self, redactor: &SecretRedactor) -> Self {
        match self {
            Self::Http(error) => Self::Http(error.without_url()),
            Self::Api { status, message } => Self::Api {
                status,
                message: redactor.redact(&message),
            },
            Self::Parse(message) => Self::Parse(redactor.redact(&message)),
            Self::RateLimited {
                retry_after_ms,
                message,
            } => Self::RateLimited {
                retry_after_ms,
                message: redactor.redact(&message),
            },
            Self::PromptTooLong(message) => Self::PromptTooLong(redactor.redact(&message)),
            Self::Connection(message) => Self::Connection(redactor.redact(&message)),
            Self::StreamTruncated(message) => {
                Self::StreamTruncated(redactor.redact(&message))
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            ProviderError::Http(error) => {
                error.is_connect()
                    || error.is_timeout()
                    || error.is_body()
                    // `Response::bytes_stream` can wrap a transport body reset
                    // in an outer reqwest Decode error (the inner source is
                    // still Body). On an empty stream this is safe to retry.
                    || error.is_decode()
                    || error.is_request()
            }
            ProviderError::RateLimited { .. }
            | ProviderError::Connection(_)
            | ProviderError::StreamTruncated(_) => true,
            // Transient server-side faults (500/502/503/504) from an overloaded
            // gateway are the most common spurious failure and are safe to retry
            // on the pre-response / empty-content paths. 4xx are terminal.
            ProviderError::Api { status, .. } => *status >= 500,
            _ => false,
        }
    }

    pub(crate) fn is_tool_schema_incompatible(&self) -> bool {
        let ProviderError::Api { message, .. } = self else {
            return false;
        };
        let lower = message.to_ascii_lowercase();
        lower.contains("tool_schema_invalid")
            || (lower.contains("input_schema")
                && lower.contains("top level")
                && ["oneof", "allof", "anyof"]
                    .iter()
                    .any(|keyword| lower.contains(keyword)))
    }

    /// A number of otherwise OpenAI-compatible gateways implement streaming
    /// but reject the optional `stream_options.include_usage` extension. This
    /// is safe to retry without the extension because it only removes token
    /// accounting metadata; response content and tool semantics are unchanged.
    pub(crate) fn is_stream_usage_options_incompatible(&self) -> bool {
        let ProviderError::Api {
            status: 400 | 404 | 422,
            message,
        } = self
        else {
            return false;
        };
        let lower = message.to_ascii_lowercase();
        let names_usage_extension =
            lower.contains("stream_options") || lower.contains("include_usage");
        let rejects_parameter = [
            "unsupported",
            "not supported",
            "unknown",
            "unrecognized",
            "not permitted",
            "not allowed",
            "extra_forbidden",
            "extra inputs",
            "invalid parameter",
        ]
        .iter()
        .any(|signal| lower.contains(signal));
        names_usage_extension && rejects_parameter
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        // Request URLs can contain query-key authentication. Preserve the
        // transport classification while ensuring the URL never reaches an
        // error string, warning, or persisted health diagnostic.
        Self::Http(error.without_url())
    }
}

/// Split the stored provider credential into individual API keys.
///
/// Provider settings persist multiple credentials as a comma-separated string;
/// older data may use one key per line. Keep this parser in the provider layer
/// so every Nomi execution path (interactive sessions, compaction and one-shot
/// sidecars) sends one credential per HTTP request rather than the whole list
/// as a single invalid bearer token.
pub(crate) fn parse_api_keys(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn is_api_key_rotation_error(error: &ProviderError) -> bool {
    matches!(
        error,
        ProviderError::Api {
            status: 401 | 403,
            ..
        } | ProviderError::RateLimited { .. }
    )
}

/// Send the initial streaming request with bounded transient-failure retry.
///
/// Shared by the API-key-based providers (Anthropic, OpenAI, Gemini): posts `body`
/// with `headers`, surfaces 429 as `RateLimited` (honouring `Retry-After`)
/// and any other non-2xx as `Api`.
pub(crate) async fn send_initial(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    body: &Value,
    redactor: &SecretRedactor,
) -> Result<reqwest::Response, ProviderError> {
    retry::with_initial_request_retry(|| async {
        let response = client
            .post(url)
            .headers(headers.clone())
            .json(body)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after_ms = parse_retry_after_ms(response.headers()).unwrap_or(5000);
        let body_text = read_provider_error_body(response, redactor).await;
        if status.as_u16() == 429 {
            return Err(ProviderError::RateLimited {
                retry_after_ms,
                message: non_empty_rate_limit_message(body_text),
            });
        }
        Err(ProviderError::Api {
            status: status.as_u16(),
            message: body_text,
        })
    })
    .await
}

/// Read an untrusted non-success response without allowing a provider or proxy
/// to turn diagnostics into an unbounded allocation. The returned text is
/// credential-aware and strips every embedded URL query before it can enter a
/// log, transcript, or frontend error payload.
pub(crate) async fn read_provider_error_body(
    response: reqwest::Response,
    redactor: &SecretRedactor,
) -> String {
    use futures::StreamExt;

    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(MAX_PROVIDER_ERROR_BODY_BYTES.min(1024));
    let mut truncated = false;
    let deadline = tokio::time::Instant::now() + PROVIDER_ERROR_BODY_READ_TIMEOUT;
    loop {
        let chunk = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(None) => break,
            Ok(Some(Err(_))) | Err(_) => {
                truncated = true;
                break;
            }
        };
        let remaining = MAX_PROVIDER_ERROR_BODY_BYTES.saturating_sub(body.len());
        if chunk.len() > remaining {
            body.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        body.extend_from_slice(&chunk);
        if body.len() == MAX_PROVIDER_ERROR_BODY_BYTES {
            // Do not perform one more read merely to distinguish an exactly
            // full body from a larger or stalled one. The diagnostic marker is
            // preferable to extending an error path by the idle-read timeout.
            truncated = true;
            break;
        }
    }

    if truncated {
        // Exact redaction cannot recognize a credential whose prefix is the
        // retained buffer tail and whose remainder fell beyond the cap (or a
        // failed/timed-out body read). Drop only the longest credential-prefix
        // suffix before decoding; this also covers encoded variants.
        let safe_boundary = redactor.redaction_safe_truncation_boundary(&body);
        body.truncate(safe_boundary);
    }
    let mut text = redactor.redact(&String::from_utf8_lossy(&body));
    if text.len() > MAX_PROVIDER_ERROR_BODY_BYTES {
        let mut boundary = MAX_PROVIDER_ERROR_BODY_BYTES;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        truncated = true;
    }
    if truncated {
        text.push_str(TRUNCATED_PROVIDER_ERROR_BODY);
    }
    text
}

/// Send the initial streaming request, rotating through the configured API
/// keys on auth/rate-limit rejections.
///
/// Starts at the last known-good key (`current_api_key`), advances on
/// `is_api_key_rotation_error` failures, and records the winning index back
/// into `current_api_key`. Returns the response together with the headers
/// that produced it so callers can reuse them for mid-stream retries.
pub(crate) async fn send_initial_with_key_rotation(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    api_keys: &[String],
    current_api_key: &AtomicUsize,
    provider_label: &'static str,
    build_headers: impl Fn(&str) -> Result<HeaderMap, ProviderError>,
) -> Result<(reqwest::Response, HeaderMap), ProviderError> {
    let redactor = SecretRedactor::new(api_keys);
    let mut last_error = None;
    let key_count = api_keys.len();
    let start_index = current_api_key.load(Ordering::Acquire) % key_count.max(1);

    for offset in 0..key_count {
        let index = (start_index + offset) % key_count;
        let api_key = &api_keys[index];
        let headers = build_headers(api_key)?;
        match send_initial(client, url, &headers, body, &redactor).await {
            Ok(response) => {
                current_api_key.store(index, Ordering::Release);
                return Ok((response, headers));
            }
            Err(error) if is_api_key_rotation_error(&error) && offset + 1 < key_count => {
                let next_index = (index + 1) % key_count;
                tracing::warn!(
                    target: "nomi_providers",
                    provider = provider_label,
                    key_index = index + 1,
                    key_count,
                    error = %error,
                    "provider rejected API key; trying the next configured key"
                );
                current_api_key.store(next_index, Ordering::Release);
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ProviderError::Connection("No usable API key configured".to_owned())
    }))
}

struct SecretRedactingProvider {
    inner: Arc<dyn LlmProvider>,
    redactor: SecretRedactor,
}

#[async_trait]
impl LlmProvider for SecretRedactingProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        let mut source = self
            .inner
            .stream(request)
            .await
            .map_err(|error| error.redacted(&self.redactor))?;
        let redactor = self.redactor.clone();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(event) = source.recv().await {
                let event = match event {
                    LlmEvent::Error(message) => LlmEvent::Error(redactor.redact(&message)),
                    other => other,
                };
                if tx.send(event).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}

fn provider_secret_redactor(config: &Config) -> SecretRedactor {
    let mut secrets = parse_api_keys(&config.api_key);
    if let Some(bedrock) = &config.bedrock {
        secrets.extend(
            [
                bedrock.access_key_id.as_ref(),
                bedrock.secret_access_key.as_ref(),
                bedrock.session_token.as_ref(),
            ]
            .into_iter()
            .flatten()
            .cloned(),
        );
    }
    SecretRedactor::new(secrets)
}

/// Parse the completed argument payload of a provider-emitted tool call.
///
/// OpenAI-compatible APIs encode function arguments as a JSON string, while
/// Anthropic-compatible streaming APIs deliver JSON fragments. In both cases
/// the completed payload must be a JSON object. A malformed payload must never
/// be replaced with `{}`: doing so turns a provider/protocol failure into a
/// seemingly valid no-argument tool call and can execute the wrong operation.
pub(crate) fn parse_tool_call_arguments(
    provider: &str,
    tool_name: &str,
    tool_id: &str,
    raw: &str,
) -> Result<Value, String> {
    if tool_name.trim().is_empty() {
        return Err(format!(
            "{provider} returned a tool call with a missing function name (call `{}`)",
            if tool_id.trim().is_empty() {
                "<missing>"
            } else {
                tool_id
            }
        ));
    }
    if tool_id.trim().is_empty() {
        return Err(format!(
            "{provider} returned tool `{tool_name}` without a call id"
        ));
    }

    let mut value = serde_json::from_str::<Value>(raw).map_err(|error| {
        format!(
            "{provider} returned malformed JSON arguments for tool `{tool_name}` (call `{tool_id}`): {error}"
        )
    })?;

    // A few OpenAI-compatible gateways double-encode the completed argument
    // object as one JSON string. Unwrap exactly one layer, with a hard size
    // bound, and still require an object. No field-level guessing or lossy
    // rewriting happens in the provider protocol parser.
    if let Value::String(encoded) = &value {
        if encoded.len() > MAX_DOUBLE_ENCODED_TOOL_ARGUMENT_BYTES {
            return Err(format!(
                "{provider} returned double-encoded arguments for tool `{tool_name}` (call `{tool_id}`) larger than the {MAX_DOUBLE_ENCODED_TOOL_ARGUMENT_BYTES}-byte safety limit"
            ));
        }
        value = serde_json::from_str::<Value>(encoded).map_err(|error| {
            format!(
                "{provider} returned malformed double-encoded JSON arguments for tool `{tool_name}` (call `{tool_id}`): {error}"
            )
        })?;
    }

    if !value.is_object() {
        return Err(format!(
            "{provider} returned non-object arguments for tool `{tool_name}` (call `{tool_id}`); expected a JSON object, got {}",
            json_value_kind(&value)
        ));
    }

    Ok(value)
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Parse a `Retry-After` HTTP header into milliseconds, honouring the provider's
/// requested backoff instead of a fixed guess. Supports the delta-seconds form
/// (what LLM gateways send); returns `None` for an absent, non-numeric, or
/// HTTP-date value (caller falls back to its default). Clamped to 120s so a
/// hostile/huge value can't wedge the agent.
pub(crate) fn parse_retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let secs: u64 = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(secs.saturating_mul(1000).min(120_000))
}

/// Connection timeout for provider HTTP clients. Bounds the TCP/TLS connect
/// phase so an unreachable or non-responsive gateway fails fast.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Idle read timeout for provider HTTP clients. Applies to each read of the
/// (streaming) response, so a gateway that accepts the request but then stalls
/// — sending no further bytes — surfaces an error instead of hanging the turn
/// forever. Active streaming resets this on every chunk, so it only trips on a
/// genuine stall. The health-check probe has its own 30s wrapper; the live
/// conversation path previously had NO timeout at all, which turned an upstream
/// stall into a silent freeze (no output, no error).
const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(120);

#[cfg(test)]
static HTTP_CLIENT_BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn http_client_build_count() -> usize {
    HTTP_CLIENT_BUILD_COUNT.load(Ordering::SeqCst)
}

/// Process-wide shared reqwest client for all LLM providers, configured with
/// connection and idle-read timeouts. Built exactly once (lazily, on first use)
/// so its keep-alive connection pool is reused across every request and every
/// provider. Previously a fresh client was built on every `stream()` call, which
/// gave each request an empty pool and thus a cold TCP+TLS handshake on the
/// first-token path of EVERY turn — the single largest avoidable首字 cost.
///
/// A stalled upstream produces a `reqwest` timeout error, which the SSE loop
/// converts into `LlmEvent::Error` (surfaced as `Nomi agent error: ...`) instead
/// of an indefinite hang. The detected proxy is captured at first build; a
/// runtime proxy change takes effect on the next app start.
pub(crate) fn http_client() -> Result<reqwest::Client, ProviderError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    match CLIENT.get_or_init(|| {
            #[cfg(test)]
            HTTP_CLIENT_BUILD_COUNT.fetch_add(1, Ordering::SeqCst);

            let builder = reqwest::Client::builder()
                .connect_timeout(HTTP_CONNECT_TIMEOUT)
                .read_timeout(HTTP_READ_TIMEOUT);
            nomifun_net::proxy::apply_detected_proxy(builder)
                .build()
                .map_err(|error| format!("Failed to build bounded provider HTTP client: {error}"))
        }) {
        Ok(client) => Ok(client.clone()),
        Err(message) => Err(ProviderError::Connection(message.clone())),
    }
}

pub(crate) fn non_empty_rate_limit_message(body: String) -> String {
    if body.trim().is_empty() {
        "HTTP 429 Too Many Requests".to_owned()
    } else {
        body
    }
}

/// Create a provider from resolved config
pub fn create_provider(config: &Config) -> Arc<dyn LlmProvider> {
    let compat = config.compat.clone();
    let redactor = provider_secret_redactor(config);

    let inner: Arc<dyn LlmProvider> = match config.provider {
        ProviderType::Anthropic => Arc::new(
            anthropic::AnthropicProvider::new(&config.api_key, &config.base_url, compat)
                .with_cache(config.prompt_caching),
        ),
        ProviderType::OpenAI => Arc::new(openai::OpenAIProvider::new(
            &config.api_key,
            &config.base_url,
            compat,
        )),
        ProviderType::Gemini => Arc::new(gemini::GeminiProvider::new(
            &config.api_key,
            &config.base_url,
            compat,
        )),
        ProviderType::Bedrock => {
            let bc = config.bedrock.clone().unwrap_or_default();
            let region = bc
                .region
                .clone()
                .or_else(|| std::env::var("AWS_REGION").ok())
                .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
                .unwrap_or_else(|| "us-east-1".to_string());
            let credentials = bedrock::credentials_from_config(&bc);
            Arc::new(bedrock::BedrockProvider::new(
                &region,
                credentials,
                config.prompt_caching,
                compat,
            ))
        }
        ProviderType::Vertex => {
            let vc = config.vertex.clone().unwrap_or_default();
            let project_id = vc.project_id.clone().unwrap_or_default();
            let region = vc
                .region
                .clone()
                .unwrap_or_else(|| "us-central1".to_string());
            let auth = vertex::auth_from_config(&vc);
            Arc::new(vertex::VertexProvider::new(
                &project_id,
                &region,
                auth,
                config.prompt_caching,
                compat,
            ))
        }
    };
    Arc::new(SecretRedactingProvider { inner, redactor })
}

#[cfg(test)]
mod retryable_tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomifun_net::secret_redaction::SecretRedactor;
    use tokio::sync::mpsc;

    use super::{
        read_provider_error_body, LlmProvider, ProviderError, SecretRedactingProvider,
        MAX_PROVIDER_ERROR_BODY_BYTES, TRUNCATED_PROVIDER_ERROR_BODY,
    };
    use super::{
        is_api_key_rotation_error, parse_api_keys, parse_retry_after_ms,
        parse_tool_call_arguments, MAX_DOUBLE_ENCODED_TOOL_ARGUMENT_BYTES,
    };

    const REFLECTED_SECRET: &str = "sk live/+?=token";
    const URL_ENCODED_SECRET: &str = "sk%20live%2F%2B%3F%3Dtoken";

    struct ReflectingProvider {
        initial_error: Option<String>,
        stream_error: Option<String>,
    }

    #[async_trait]
    impl LlmProvider for ReflectingProvider {
        async fn stream(
            &self,
            _request: &LlmRequest,
        ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
            if let Some(message) = &self.initial_error {
                return Err(ProviderError::Api {
                    status: 401,
                    message: message.clone(),
                });
            }
            let (tx, rx) = mpsc::channel(1);
            if let Some(message) = &self.stream_error {
                tx.send(LlmEvent::Error(message.clone())).await.unwrap();
            }
            drop(tx);
            Ok(rx)
        }
    }

    fn empty_request() -> LlmRequest {
        LlmRequest {
            model: "test".to_owned(),
            system: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: 1,
            thinking: None,
            reasoning_effort: None,
        }
    }

    fn assert_secret_absent(message: &str) {
        assert!(!message.contains(REFLECTED_SECRET), "raw secret leaked: {message}");
        assert!(
            !message.contains(URL_ENCODED_SECRET),
            "URL-encoded secret leaked: {message}"
        );
        assert!(message.contains("[REDACTED]"), "redaction marker missing: {message}");
    }

    #[tokio::test]
    async fn provider_error_body_is_bounded_and_sanitized() {
        let secret = "gateway-secret";
        let body = format!(
            "Post \"https://gateway.test/responses?access_token={secret}\": EOF\n{}",
            "x".repeat(MAX_PROVIDER_ERROR_BODY_BYTES + 4096)
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body(body)
                .expect("test response"),
        );

        let sanitized =
            read_provider_error_body(response, &SecretRedactor::default()).await;

        assert!(!sanitized.contains(secret));
        assert!(sanitized.contains("https://gateway.test/responses?<redacted>"));
        assert!(sanitized.ends_with(TRUNCATED_PROVIDER_ERROR_BODY));
        assert!(
            sanitized.len()
                <= MAX_PROVIDER_ERROR_BODY_BYTES + TRUNCATED_PROVIDER_ERROR_BODY.len()
        );
    }

    #[tokio::test]
    async fn truncated_provider_body_drops_raw_and_encoded_secret_prefixes_at_the_cap() {
        let redactor = SecretRedactor::new([REFLECTED_SECRET]);
        for reflected in [REFLECTED_SECRET, URL_ENCODED_SECRET] {
            let retained_secret_prefix_len = reflected.len() - 3;
            let padding_len = MAX_PROVIDER_ERROR_BODY_BYTES - retained_secret_prefix_len;
            let body = format!("{}{}", "x".repeat(padding_len), reflected);
            let response = reqwest::Response::from(
                http::Response::builder()
                    .status(500)
                    .body(body)
                    .expect("test response"),
            );

            let sanitized = read_provider_error_body(response, &redactor).await;

            assert_eq!(
                sanitized,
                format!("{}{}", "x".repeat(padding_len), TRUNCATED_PROVIDER_ERROR_BODY),
                "a credential prefix crossing the byte cap must be removed: {reflected}"
            );
            assert!(
                sanitized.len()
                    <= MAX_PROVIDER_ERROR_BODY_BYTES + TRUNCATED_PROVIDER_ERROR_BODY.len()
            );
        }
    }

    #[tokio::test]
    async fn truncated_provider_body_keeps_an_unrelated_plain_tail() {
        let ordinary_tail = "ordinary diagnostic tail";
        let padding_len = MAX_PROVIDER_ERROR_BODY_BYTES - ordinary_tail.len();
        let body = format!(
            "{}{}overflow beyond cap",
            "x".repeat(padding_len),
            ordinary_tail
        );
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body(body)
                .expect("test response"),
        );

        let sanitized = read_provider_error_body(
            response,
            &SecretRedactor::new(["gateway-secret"]),
        )
        .await;

        assert!(sanitized.ends_with(&format!(
            "{ordinary_tail}{TRUNCATED_PROVIDER_ERROR_BODY}"
        )));
        assert_eq!(
            sanitized.len(),
            MAX_PROVIDER_ERROR_BODY_BYTES + TRUNCATED_PROVIDER_ERROR_BODY.len()
        );
    }

    #[tokio::test]
    async fn redaction_marker_expansion_still_respects_the_diagnostic_cap() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body("x".repeat(MAX_PROVIDER_ERROR_BODY_BYTES + 1))
                .expect("test response"),
        );

        let sanitized =
            read_provider_error_body(response, &SecretRedactor::new(["x"])).await;

        assert!(!sanitized.contains('x'));
        assert!(sanitized.ends_with(TRUNCATED_PROVIDER_ERROR_BODY));
        assert_eq!(
            sanitized.len(),
            MAX_PROVIDER_ERROR_BODY_BYTES + TRUNCATED_PROVIDER_ERROR_BODY.len()
        );
    }

    #[tokio::test]
    async fn small_provider_error_body_is_not_marked_truncated() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body("upstream EOF".to_owned())
                .expect("test response"),
        );

        let body = read_provider_error_body(response, &SecretRedactor::default()).await;

        assert_eq!(body, "upstream EOF");
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_provider_error_body_has_its_own_short_deadline() {
        let stalled = futures::stream::pending::<Result<Vec<u8>, std::io::Error>>();
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(500)
                .body(reqwest::Body::wrap_stream(stalled))
                .expect("test response"),
        );

        let body = read_provider_error_body(response, &SecretRedactor::default()).await;

        assert_eq!(body, TRUNCATED_PROVIDER_ERROR_BODY);
    }

    #[tokio::test]
    async fn provider_boundary_redacts_reflected_secrets_from_errors_and_stream_events() {
        let redactor = SecretRedactor::new([REFLECTED_SECRET]);
        let initial = SecretRedactingProvider {
            inner: Arc::new(ReflectingProvider {
                initial_error: Some(format!(
                    "upstream echoed raw={REFLECTED_SECRET} encoded={URL_ENCODED_SECRET}"
                )),
                stream_error: None,
            }),
            redactor: redactor.clone(),
        };
        let error = initial.stream(&empty_request()).await.unwrap_err();
        assert_secret_absent(&error.to_string());

        let streaming = SecretRedactingProvider {
            inner: Arc::new(ReflectingProvider {
                initial_error: None,
                stream_error: Some(format!(
                    "upstream echoed raw={REFLECTED_SECRET} encoded={URL_ENCODED_SECRET}"
                )),
            }),
            redactor,
        };
        let mut events = streaming.stream(&empty_request()).await.unwrap();
        let event = events.recv().await.expect("reflected stream error event");
        let LlmEvent::Error(message) = event else {
            panic!("expected stream error event");
        };
        assert_secret_absent(&message);
    }

    #[test]
    fn parse_retry_after_seconds_clamped() {
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
        let mut h = HeaderMap::new();
        h.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        assert_eq!(parse_retry_after_ms(&h), Some(30_000));

        let mut huge = HeaderMap::new();
        huge.insert(RETRY_AFTER, HeaderValue::from_static("99999"));
        assert_eq!(parse_retry_after_ms(&huge), Some(120_000)); // clamped

        // Absent / non-numeric (HTTP-date) -> None (caller uses its default).
        assert_eq!(parse_retry_after_ms(&HeaderMap::new()), None);
        let mut date = HeaderMap::new();
        date.insert(RETRY_AFTER, HeaderValue::from_static("Wed, 21 Oct 2025 07:28:00 GMT"));
        assert_eq!(parse_retry_after_ms(&date), None);
    }

    #[test]
    fn transient_5xx_is_retryable_but_4xx_is_not() {
        // Transient server-side faults (overloaded gateways) are the most common
        // spurious failure and are safe to retry on the pre-response / empty
        // paths; client errors (4xx) are terminal. (Phase 1)
        let api = |status| ProviderError::Api {
            status,
            message: "x".to_string(),
        };
        assert!(api(500).is_retryable());
        assert!(api(502).is_retryable());
        assert!(api(503).is_retryable());
        assert!(api(504).is_retryable());
        assert!(!api(400).is_retryable());
        assert!(!api(404).is_retryable());
        assert!(!api(429).is_retryable(), "429 is surfaced as RateLimited, not Api");

        assert!(
            ProviderError::RateLimited {
                retry_after_ms: 0,
                message: "x".to_string()
            }
            .is_retryable()
        );
        assert!(ProviderError::Connection("x".to_string()).is_retryable());
        assert!(ProviderError::StreamTruncated("x".to_string()).is_retryable());
        assert!(!ProviderError::PromptTooLong("x".to_string()).is_retryable());
        assert!(!ProviderError::Parse("x".to_string()).is_retryable());
    }

    #[test]
    fn tool_schema_classifier_accepts_bedrock_gateway_signals() {
        let reason = ProviderError::Api {
            status: 500,
            message: r#"{"reason":"TOOL_SCHEMA_INVALID"}"#.into(),
        };
        let wording = ProviderError::Api {
            status: 400,
            message: "input_schema does not support oneOf, allOf, or anyOf at the top level".into(),
        };
        assert!(reason.is_tool_schema_incompatible());
        assert!(wording.is_tool_schema_incompatible());
    }

    #[test]
    fn tool_schema_classifier_rejects_unrelated_failures() {
        let errors = [
            ProviderError::Api {
                status: 500,
                message: "upstream unavailable".into(),
            },
            ProviderError::Api {
                status: 400,
                message: "input_schema is malformed".into(),
            },
            ProviderError::Connection("input_schema connection reset".into()),
        ];
        assert!(
            errors
                .iter()
                .all(|error| !error.is_tool_schema_incompatible())
        );
    }

    #[test]
    fn stream_usage_options_classifier_is_narrow() {
        for message in [
            "unknown parameter: stream_options",
            r#"{\"detail\":[{\"loc\":[\"body\",\"stream_options\"],\"type\":\"extra_forbidden\"}]}"#,
            "include_usage is not supported",
        ] {
            assert!(ProviderError::Api {
                status: 400,
                message: message.into(),
            }
            .is_stream_usage_options_incompatible());
        }

        for error in [
            ProviderError::Api {
                status: 500,
                message: "unknown parameter: stream_options".into(),
            },
            ProviderError::Api {
                status: 400,
                message: "streaming is unsupported".into(),
            },
            ProviderError::Connection("stream_options reset".into()),
        ] {
            assert!(!error.is_stream_usage_options_incompatible());
        }
    }

    #[test]
    fn api_key_list_supports_comma_and_legacy_newline_separators() {
        assert_eq!(
            parse_api_keys(" key-one,\nkey-two\r\n, key-three "),
            vec!["key-one", "key-two", "key-three"]
        );
        assert!(parse_api_keys(" , \n ").is_empty());
    }

    #[test]
    fn auth_and_rate_limit_errors_rotate_api_keys() {
        for status in [401, 403] {
            assert!(is_api_key_rotation_error(&ProviderError::Api {
                status,
                message: "rejected".into(),
            }));
        }
        assert!(is_api_key_rotation_error(&ProviderError::RateLimited {
            retry_after_ms: 1000,
            message: "limited".into(),
        }));
        assert!(!is_api_key_rotation_error(&ProviderError::Api {
            status: 400,
            message: "bad request".into(),
        }));
        assert!(!is_api_key_rotation_error(&ProviderError::Api {
            status: 500,
            message: "server error".into(),
        }));
    }

    #[test]
    fn tool_call_arguments_require_valid_json_object() {
        assert_eq!(
            parse_tool_call_arguments("test", "no_args", "call_ok", "{}")
                .expect("an explicit empty object is a valid no-argument call"),
            serde_json::json!({})
        );
        assert_eq!(
            parse_tool_call_arguments(
                "test",
                "update",
                "call_update",
                r#"{"kb_id":"kb_1"}"#,
            )
            .expect("object arguments should parse")["kb_id"],
            "kb_1"
        );

        let double_encoded =
            serde_json::to_string(&serde_json::json!({"path": "README.md"}).to_string()).unwrap();
        assert_eq!(
            parse_tool_call_arguments(
                "test",
                "read_file",
                "call_double",
                &double_encoded,
            )
            .expect("one whole-object JSON string layer should be unwrapped"),
            serde_json::json!({"path": "README.md"})
        );

        let triple_encoded = serde_json::to_string(&double_encoded).unwrap();
        let triple_error =
            parse_tool_call_arguments("test", "read_file", "call_triple", &triple_encoded)
                .expect_err("the compatibility parser must unwrap at most one layer");
        assert!(triple_error.contains("non-object arguments"));
        assert!(triple_error.contains("string"));

        let oversized_inner = "x".repeat(MAX_DOUBLE_ENCODED_TOOL_ARGUMENT_BYTES + 1);
        let oversized_outer = serde_json::to_string(&oversized_inner).unwrap();
        let oversized_error =
            parse_tool_call_arguments("test", "read_file", "call_large", &oversized_outer)
                .expect_err("double-encoded arguments must be bounded");
        assert!(oversized_error.contains("safety limit"));

        let malformed =
            parse_tool_call_arguments("test", "update", "call_bad", r#"{"kb_id":]"#)
                .expect_err("malformed JSON must fail instead of becoming an empty object");
        assert!(malformed.contains("malformed JSON arguments"));
        assert!(malformed.contains("call_bad"));

        let non_object = parse_tool_call_arguments("test", "update", "call_array", "[]")
            .expect_err("tool arguments must be an object");
        assert!(non_object.contains("non-object arguments"));
        assert!(non_object.contains("array"));

        let missing_name = parse_tool_call_arguments("test", " ", "call_named", "{}")
            .expect_err("a call without a function name must fail");
        assert!(missing_name.contains("missing function name"));
        assert!(missing_name.contains("call_named"));

        let missing_id = parse_tool_call_arguments("test", "update", "", "{}")
            .expect_err("a call without an id must fail");
        assert!(missing_id.contains("without a call id"));
    }
}
