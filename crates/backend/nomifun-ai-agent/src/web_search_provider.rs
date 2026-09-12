//! Authorized SearchProvider-backed implementation of `web.search`.
//!
//! The production owner is OpenAI Responses built-in `web_search`, using the
//! exact model route and credential already resolved by the server. No browser
//! scraping, consumer RSS endpoint, or model-supplied credential is accepted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";
pub const CITATION_RENDER_TOOL_NAME: &str = "citation_render";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_QUERY_CHARS: usize = 2048;
const MAX_RESULTS: usize = 20;
const DEFAULT_RESULTS: usize = 5;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SESSION_CITATIONS: usize = 64;

#[derive(Clone, Debug)]
struct CitationRecord {
    title: String,
    url: String,
}

#[derive(Default)]
pub struct SessionCitationStore {
    inner: Mutex<(BTreeMap<String, CitationRecord>, VecDeque<String>)>,
}

impl SessionCitationStore {
    fn insert(&self, id: String, title: String, url: String) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.0.contains_key(&id) {
            inner.1.push_back(id.clone());
        }
        inner.0.insert(id, CitationRecord { title, url });
        while inner.0.len() > MAX_SESSION_CITATIONS {
            if let Some(oldest) = inner.1.pop_front() {
                inner.0.remove(&oldest);
            }
        }
    }

    fn get(&self, id: &str) -> Option<CitationRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
            .get(id)
            .cloned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SearchProviderResult {
    pub source_id: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SearchProviderResponse {
    pub answer: String,
    pub results: Vec<SearchProviderResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProviderErrorKind {
    Timeout,
    Transport,
    UpstreamRejected,
    ResponseTooLarge,
    InvalidResponse,
    NoSources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchProviderError {
    pub kind: SearchProviderErrorKind,
    /// Trusted diagnostic retained for internal observability. Tool adapters
    /// must never project this field into model-visible output.
    pub internal_message: String,
}

impl SearchProviderError {
    pub fn new(kind: SearchProviderErrorKind, internal_message: impl Into<String>) -> Self {
        Self {
            kind,
            internal_message: internal_message.into(),
        }
    }
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    async fn search(
        &self,
        query: &str,
        count: usize,
    ) -> Result<SearchProviderResponse, SearchProviderError>;
}

/// Server-built OpenAI Responses search connection. The secret is retained in
/// memory only and never serialized into a Tool input or result.
#[derive(Clone)]
pub struct OpenAiResponsesSearchProvider {
    client: Client,
    endpoint: Url,
    api_key: String,
    model: String,
}

impl OpenAiResponsesSearchProvider {
    pub fn new(endpoint: &str, api_key: String, model: String) -> Result<Self, String> {
        let endpoint = Url::parse(endpoint)
            .map_err(|_| "OpenAI Responses search endpoint is invalid".to_owned())?;
        if !matches!(endpoint.scheme(), "https" | "http")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || api_key.trim().is_empty()
            || model.trim().is_empty()
        {
            return Err(
                "OpenAI Responses search requires an exact HTTP endpoint, bearer credential, and model"
                    .to_owned(),
            );
        }
        Ok(Self {
            client: nomifun_net::http_client_no_redirect().map_err(|error| error.to_string())?,
            endpoint,
            api_key,
            model,
        })
    }
}

#[async_trait]
impl SearchProvider for OpenAiResponsesSearchProvider {
    fn provider_id(&self) -> &str {
        "openai.responses.web_search"
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
    ) -> Result<SearchProviderResponse, SearchProviderError> {
        let response = tokio::time::timeout(
            SEARCH_TIMEOUT,
            self.client
                .post(self.endpoint.clone())
                .bearer_auth(&self.api_key)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&json!({
                    "model": self.model,
                    "input": query,
                    "tools": [{"type": "web_search"}],
                    "tool_choice": {"type": "web_search"},
                    "include": ["web_search_call.action.sources"],
                    "max_tool_calls": 1,
                    "store": false,
                }))
                .send(),
        )
        .await
        .map_err(|_| {
            SearchProviderError::new(
                SearchProviderErrorKind::Timeout,
                "OpenAI Responses web search timed out",
            )
        })?
        .map_err(|error| {
            SearchProviderError::new(SearchProviderErrorKind::Transport, error.to_string())
        })?;
        if !response.status().is_success() {
            return Err(SearchProviderError::new(
                SearchProviderErrorKind::UpstreamRejected,
                format!(
                    "OpenAI Responses web search returned HTTP {}",
                    response.status()
                ),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(SearchProviderError::new(
                SearchProviderErrorKind::ResponseTooLarge,
                "web search response exceeded the 2 MiB limit",
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                SearchProviderError::new(SearchProviderErrorKind::Transport, error.to_string())
            })?;
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(SearchProviderError::new(
                    SearchProviderErrorKind::ResponseTooLarge,
                    "web search response exceeded the 2 MiB limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes).map_err(|error| {
            SearchProviderError::new(
                SearchProviderErrorKind::InvalidResponse,
                format!("OpenAI Responses web search returned invalid JSON: {error}"),
            )
        })?;
        parse_responses_sources(&body, count)
    }
}

fn parse_responses_sources(
    body: &Value,
    limit: usize,
) -> Result<SearchProviderResponse, SearchProviderError> {
    let output = body
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SearchProviderError::new(
                SearchProviderErrorKind::InvalidResponse,
                "OpenAI Responses web search omitted output items",
            )
        })?;
    let answer = output
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find_map(|content| content.get("text").and_then(Value::as_str))
        .unwrap_or_default()
        .chars()
        .take(512)
        .collect::<String>();
    let mut seen = BTreeSet::new();
    let mut results = Vec::new();
    for source in output
        .iter()
        .filter_map(|item| item.get("action"))
        .filter_map(|action| action.get("sources").and_then(Value::as_array))
        .flatten()
    {
        let Some(url) = source.get("url").and_then(Value::as_str) else {
            continue;
        };
        let parsed = match Url::parse(url) {
            Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
            _ => continue,
        };
        let canonical_url = parsed.to_string();
        if !seen.insert(canonical_url.clone()) {
            continue;
        }
        let title = source
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| parsed.host_str().unwrap_or("Web source"));
        let source_id = source
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!("{:x}", Sha256::digest(canonical_url.as_bytes()))[..16].to_owned()
            });
        results.push(SearchProviderResult {
            source_id,
            title: title.to_owned(),
            url: canonical_url,
            snippet: source
                .get("snippet")
                .or_else(|| source.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect(),
        });
        if results.len() == limit {
            break;
        }
    }
    if results.is_empty() {
        return Err(SearchProviderError::new(
            SearchProviderErrorKind::NoSources,
            "OpenAI Responses web search returned no URL sources",
        ));
    }
    Ok(SearchProviderResponse { answer, results })
}

#[derive(Clone)]
pub struct WebSearchTool {
    provider: Arc<dyn SearchProvider>,
    citations: Arc<SessionCitationStore>,
}

impl WebSearchTool {
    pub fn new(provider: Arc<dyn SearchProvider>) -> Self {
        Self {
            provider,
            citations: Arc::new(SessionCitationStore::default()),
        }
    }

    pub fn with_citations(
        provider: Arc<dyn SearchProvider>,
        citations: Arc<SessionCitationStore>,
    ) -> Self {
        Self {
            provider,
            citations,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    query: String,
    #[serde(default = "default_result_count")]
    limit: usize,
}

const fn default_result_count() -> usize {
    DEFAULT_RESULTS
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        WEB_SEARCH_TOOL_NAME
    }
    fn description(&self) -> &str {
        "Search through the exact authorized OpenAI Responses model connection and return bounded, citable URL sources."
    }
    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_RESULTS}
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }
    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let input = match serde_json::from_value::<SearchInput>(input) {
            Ok(input) => input,
            Err(_) => return search_error("INVALID_PAYLOAD", "query and optional limit are required"),
        };
        let query = input.query.trim();
        if query.is_empty()
            || input.query.chars().count() > MAX_QUERY_CHARS
            || !(1..=MAX_RESULTS).contains(&input.limit)
        {
            return search_error(
                "INVALID_PAYLOAD",
                "query must contain 1 to 2048 characters and limit must be from 1 to 20",
            );
        }
        match self.provider.search(query, input.limit).await {
            Ok(response) => {
                let provider_id = self.provider.provider_id();
                let results = response
                    .results
                    .into_iter()
                    .map(|result| {
                        let digest = format!(
                            "{:x}",
                            Sha256::digest(
                                format!("{provider_id}\0{}\0{}", result.source_id, result.url)
                                    .as_bytes()
                            )
                        );
                        let citation_id = format!("web-search-{}", &digest[..16]);
                        self.citations.insert(
                            citation_id.clone(),
                            result.title.clone(),
                            result.url.clone(),
                        );
                        json!({
                            "citation_id": citation_id,
                            "source_id": result.source_id,
                            "title": result.title,
                            "url": result.url,
                            "snippet": result.snippet,
                        })
                    })
                    .collect::<Vec<_>>();
                ToolResult::text(
                    json!({
                        "query": query,
                        "provider": provider_id,
                        "answer": response.answer,
                        "results": results
                    })
                    .to_string(),
                )
            }
            Err(error) => {
                let internal_message =
                    nomi_redact::redact_secrets(&error.internal_message);
                tracing::warn!(
                    provider = self.provider.provider_id(),
                    error_kind = ?error.kind,
                    error = %internal_message,
                    "authorized web search provider failed"
                );
                let (code, message) = match error.kind {
                    SearchProviderErrorKind::Timeout => (
                        "WEB_SEARCH_TIMEOUT",
                        "The authorized web search timed out.",
                    ),
                    SearchProviderErrorKind::ResponseTooLarge => (
                        "WEB_SEARCH_RESPONSE_TOO_LARGE",
                        "The authorized web search response exceeded its safe size limit.",
                    ),
                    SearchProviderErrorKind::InvalidResponse
                    | SearchProviderErrorKind::NoSources => (
                        "WEB_SEARCH_RESPONSE_INVALID",
                        "The authorized web search returned an unusable response.",
                    ),
                    SearchProviderErrorKind::Transport
                    | SearchProviderErrorKind::UpstreamRejected => (
                        "WEB_SEARCH_FAILED",
                        "The authorized web search provider could not complete the request.",
                    ),
                };
                search_error(code, message)
            }
        }
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    fn execution_timeout(&self, _input: &Value) -> Duration {
        SEARCH_TIMEOUT + Duration::from_secs(5)
    }
    fn max_result_size(&self) -> usize {
        128 * 1024
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CitationRenderInput {
    citation_ids: Vec<String>,
}

pub struct CitationRenderTool {
    citations: Arc<SessionCitationStore>,
}

impl CitationRenderTool {
    pub fn new(citations: Arc<SessionCitationStore>) -> Self {
        Self { citations }
    }
}

#[async_trait]
impl Tool for CitationRenderTool {
    fn name(&self) -> &str {
        CITATION_RENDER_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Render exact Markdown citations previously returned by this AgentSession's web_search tool. Unknown or expired citation IDs are rejected."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "citation_ids": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 20,
                    "items": {"type": "string", "minLength": 1, "maxLength": 64}
                }
            },
            "required": ["citation_ids"],
            "additionalProperties": false
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let input = match serde_json::from_value::<CitationRenderInput>(input) {
            Ok(input) if (1..=20).contains(&input.citation_ids.len()) => input,
            _ => return search_error("INVALID_PAYLOAD", "citation_ids must contain 1 to 20 IDs"),
        };
        let mut seen = BTreeSet::new();
        let mut rendered = Vec::with_capacity(input.citation_ids.len());
        for id in input.citation_ids {
            let id = id.trim();
            if id.is_empty() || !seen.insert(id.to_owned()) {
                return search_error("INVALID_PAYLOAD", "citation IDs must be non-empty and unique");
            }
            let Some(record) = self.citations.get(id) else {
                return search_error(
                    "CITATION_NOT_FOUND",
                    "citation ID is unknown or expired for this AgentSession",
                );
            };
            rendered.push(json!({
                "citation_id": id,
                "title": record.title,
                "url": record.url,
                "markdown": format!("[{}]({})", record.title, record.url),
            }));
        }
        ToolResult::text(json!({"citations": rendered}).to_string())
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

fn search_error(code: &str, message: &str) -> ToolResult {
    ToolResult {
        content: json!({"code": code, "message": message}).to_string(),
        is_error: true,
        images: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{body_json, header, method, path}};

    struct EmptyProvider;

    struct LeakyFailureProvider;

    #[async_trait]
    impl SearchProvider for EmptyProvider {
        fn provider_id(&self) -> &str {
            "test.empty"
        }

        async fn search(
            &self,
            _query: &str,
            _count: usize,
        ) -> Result<SearchProviderResponse, SearchProviderError> {
            Err(SearchProviderError::new(
                SearchProviderErrorKind::Transport,
                "not invoked",
            ))
        }
    }

    #[async_trait]
    impl SearchProvider for LeakyFailureProvider {
        fn provider_id(&self) -> &str {
            "test.private.search"
        }

        async fn search(
            &self,
            _query: &str,
            _count: usize,
        ) -> Result<SearchProviderResponse, SearchProviderError> {
            Err(SearchProviderError::new(
                SearchProviderErrorKind::Transport,
                "POST https://private-search.internal/v1/responses?token=secret-token failed; api_key=sk-012345678901234567890123",
            ))
        }
    }

    #[test]
    fn tool_input_matches_the_canonical_wave1_contract() {
        let tool = WebSearchTool::new(Arc::new(EmptyProvider));
        assert_eq!(
            tool.input_schema(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "minLength": 1, "maxLength": 2048},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 20}
                },
                "required": ["query"]
            })
        );
    }

    #[tokio::test]
    async fn responses_search_uses_builtin_tool_and_parses_sources() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer secret-token"))
            .and(body_json(json!({
                "model":"gpt-search","input":"rust agents",
                "tools":[{"type":"web_search"}],"tool_choice":{"type":"web_search"},
                "include":["web_search_call.action.sources"],"max_tool_calls":1,"store":false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"output":[
                {"type":"web_search_call","action":{"sources":[
                    {"id":"src-1","url":"https://example.com/rust","title":"Rust Agents"},
                    {"id":"src-2","url":"https://example.org/two","title":"Second"}
                ]}},
                {"type":"message","content":[{"type":"output_text","text":"Grounded answer"}]}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        let provider = OpenAiResponsesSearchProvider::new(
            &format!("{}/v1/responses", server.uri()),
            "secret-token".into(),
            "gpt-search".into(),
        ).unwrap();
        let citations = Arc::new(SessionCitationStore::default());
        let result = WebSearchTool::with_citations(Arc::new(provider), Arc::clone(&citations))
            .execute(json!({"query":"rust agents","limit":2})).await;
        assert!(!result.is_error, "{}", result.content);
        let body: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(body["provider"], "openai.responses.web_search");
        assert_eq!(body["answer"], "Grounded answer");
        assert_eq!(body["results"][0]["source_id"], "src-1");
        assert_eq!(body["results"][0]["snippet"], "");
        let citation_id = body["results"][0]["citation_id"].as_str().unwrap();
        let rendered = CitationRenderTool::new(citations)
            .execute(json!({"citation_ids":[citation_id]}))
            .await;
        assert!(!rendered.is_error, "{}", rendered.content);
        let rendered: Value = serde_json::from_str(&rendered.content).unwrap();
        assert_eq!(
            rendered["citations"][0]["markdown"],
            "[Rust Agents](https://example.com/rust)"
        );
    }

    #[tokio::test]
    async fn response_without_content_length_still_enforces_streaming_limit() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_RESPONSE_BYTES + 1]))
            .mount(&server).await;
        let provider = OpenAiResponsesSearchProvider::new(
            &server.uri(), "secret-token".into(), "gpt-search".into(),
        ).unwrap();
        let error = provider.search("oversize", 1).await.unwrap_err();
        assert_eq!(error.kind, SearchProviderErrorKind::ResponseTooLarge);
        assert!(error.internal_message.contains("2 MiB"));
    }

    #[tokio::test]
    async fn provider_transport_diagnostics_never_enter_model_visible_json() {
        let result = WebSearchTool::new(Arc::new(LeakyFailureProvider))
            .execute(json!({"query": "private endpoint", "limit": 1}))
            .await;
        assert!(result.is_error);
        let payload: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(payload["code"], "WEB_SEARCH_FAILED");
        assert_eq!(
            payload["message"],
            "The authorized web search provider could not complete the request."
        );
        for forbidden in [
            "private-search.internal",
            "secret-token",
            "sk-012345678901234567890123",
            "api_key",
        ] {
            assert!(
                !result.content.contains(forbidden),
                "model-visible search error leaked {forbidden}: {}",
                result.content
            );
        }
    }

    #[tokio::test]
    async fn citation_renderer_rejects_ids_not_returned_in_this_session() {
        let result = CitationRenderTool::new(Arc::new(SessionCitationStore::default()))
            .execute(json!({"citation_ids":["web-search-forged"]}))
            .await;
        assert!(result.is_error);
        assert_eq!(
            serde_json::from_str::<Value>(&result.content).unwrap()["code"],
            "CITATION_NOT_FOUND"
        );
    }

    #[test]
    fn parser_rejects_missing_or_non_http_sources() {
        assert!(parse_responses_sources(&json!({"output":[]}), 5).is_err());
        assert!(parse_responses_sources(
            &json!({"output":[{"action":{"sources":[{"url":"file:///secret"}]}}]}), 5
        ).is_err());
    }
}
