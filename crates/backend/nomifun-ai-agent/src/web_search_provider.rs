//! Authorized SearchProvider-backed implementation of
//! `web.research/search`.
//!
//! The production owner is OpenAI Responses built-in `web_search`, using the
//! exact search model and credential resolved on each invocation. It is
//! independent of the conversation model. No browser
//! scraping, consumer RSS endpoint, or model-supplied credential is accepted.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const WEB_RESEARCH_MODULE_ID: &str = "web.research";
pub const WEB_RESEARCH_SEARCH_ACTION_ID: &str = "web.research/search";
pub const WEB_RESEARCH_FETCH_ACTION_ID: &str = "web.research/fetch";
pub const WEB_RESEARCH_ACTION_IDS: [&str; 2] = [
    WEB_RESEARCH_SEARCH_ACTION_ID,
    WEB_RESEARCH_FETCH_ACTION_ID,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebResearchAction {
    Search,
    Fetch,
}

impl WebResearchAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::Search => WEB_RESEARCH_SEARCH_ACTION_ID,
            Self::Fetch => WEB_RESEARCH_FETCH_ACTION_ID,
        }
    }

    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            WEB_RESEARCH_SEARCH_ACTION_ID => Some(Self::Search),
            WEB_RESEARCH_FETCH_ACTION_ID => Some(Self::Fetch),
            _ => None,
        }
    }
}
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SESSION_CITATIONS: usize = 64;

pub struct SessionCitationStore {
    scope_id: String,
    inner: Mutex<(BTreeSet<String>, VecDeque<String>)>,
}

impl Default for SessionCitationStore {
    fn default() -> Self {
        Self {
            scope_id: uuid::Uuid::now_v7().to_string(),
            inner: Mutex::new((BTreeSet::new(), VecDeque::new())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DerivedWebCitation {
    pub citation_id: String,
    pub markdown: String,
}

impl SessionCitationStore {
    pub fn for_scope(scope_id: impl Into<String>) -> Result<Self, String> {
        let scope_id = scope_id.into();
        if scope_id.trim().is_empty() || scope_id.len() > 512 {
            return Err("web citation scope must contain 1 to 512 bytes".to_owned());
        }
        Ok(Self {
            scope_id,
            inner: Mutex::new((BTreeSet::new(), VecDeque::new())),
        })
    }

    pub(crate) fn record(
        &self,
        provider_id: &str,
        source_id: &str,
        title: &str,
        url: &str,
    ) -> DerivedWebCitation {
        let digest = format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{}\0{provider_id}\0{source_id}\0{url}",
                    self.scope_id
                )
                .as_bytes()
            )
        );
        let citation_id = format!("web-search-{}", &digest[..16]);
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !inner.0.contains(&citation_id) {
            inner.1.push_back(citation_id.clone());
        }
        inner.0.insert(citation_id.clone());
        while inner.0.len() > MAX_SESSION_CITATIONS {
            if let Some(oldest) = inner.1.pop_front() {
                inner.0.remove(&oldest);
            }
        }
        drop(inner);
        DerivedWebCitation {
            citation_id,
            markdown: markdown_citation(title, url),
        }
    }

    #[cfg(test)]
    fn contains(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0
            .contains(id)
    }

}

fn markdown_citation(title: &str, url: &str) -> String {
    let title = title
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let destination = url.replace('<', "%3C").replace('>', "%3E");
    format!("[{title}](<{destination}>)")
}

pub async fn search_with_derived_citations(
    provider: &dyn SearchProvider,
    citations: &SessionCitationStore,
    query: &str,
    limit: usize,
) -> Result<Value, SearchProviderError> {
    let response = provider.search(query, limit).await?;
    let provider_id = provider.provider_id();
    let results = response
        .results
        .into_iter()
        .map(|result| {
            let citation = citations.record(
                provider_id,
                &result.source_id,
                &result.title,
                &result.url,
            );
            json!({
                "citation_id": citation.citation_id,
                "citation_markdown": citation.markdown,
                "source_id": result.source_id,
                "title": result.title,
                "url": result.url,
                "snippet": result.snippet,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "query": query,
        "answer": response.answer,
        "results": results,
    }))
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
    NotConfigured,
    AmbiguousModel,
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

/// Resolve only configured, enabled search models at tool execution time.
/// Constructing a General Agent never needs a search account, and discovery
/// never upgrades a Chat-only model or contacts an unconfigured public service.
pub struct CatalogSearchProvider {
    invoke: Arc<nomifun_model_invoke::ModelInvokeService>,
    conversation_model: nomifun_model_invoke::ModelRef,
}

impl CatalogSearchProvider {
    pub fn new(invoke: Arc<nomifun_model_invoke::ModelInvokeService>, conversation_model: nomifun_model_invoke::ModelRef) -> Self {
        Self { invoke, conversation_model }
    }

    async fn resolve(&self) -> Result<OpenAiResponsesSearchProvider, SearchProviderError> {
        let capabilities = self.invoke.provider_model_capability_repo().list().await
            .map_err(|error| SearchProviderError::new(SearchProviderErrorKind::NotConfigured, error.to_string()))?;
        let mut candidates = Vec::new();
        for capability in capabilities {
            if capability.task != "chat" || capability.protocol != "openai.responses"
                || !nomifun_api_types::parse_persisted_model_traits(&capability.traits)
                    .is_ok_and(|traits| traits.contains(&nomifun_api_types::ModelTrait::WebSearch)) {
                continue;
            }
            // Reuse the Chat resolver's enabled-model, protocol, connection,
            // endpoint, origin and decrypted-credential checks verbatim.
            let Ok(fields) = crate::factory::provider_config::resolve_provider_fields(
                &self.invoke, &capability.provider_id, &capability.model,
            ).await else { continue; };
            if fields.provider == "openai-responses" && fields.supports_web_search {
                candidates.push((nomifun_model_invoke::ModelRef { provider_id: capability.provider_id, model: capability.model }, fields));
            }
        }
        let models = candidates.iter().map(|(model, _)| model.clone()).collect::<Vec<_>>();
        let index = select_search_model(&self.conversation_model, &models)
            .map_err(|kind| SearchProviderError::new(kind, "No unambiguous configured native web-search model is available"))?;
        let (_, fields) = candidates.swap_remove(index);
        OpenAiResponsesSearchProvider::new(fields.base_url.as_deref().unwrap_or_default(), fields.api_key, fields.model)
            .map_err(|message| SearchProviderError::new(SearchProviderErrorKind::NotConfigured, message))
    }
}

fn select_search_model(conversation: &nomifun_model_invoke::ModelRef, candidates: &[nomifun_model_invoke::ModelRef]) -> Result<usize, SearchProviderErrorKind> {
    if let Some(index) = candidates.iter().position(|candidate| candidate.provider_id == conversation.provider_id && candidate.model == conversation.model) {
        return Ok(index);
    }
    match candidates.len() {
        0 => Err(SearchProviderErrorKind::NotConfigured),
        1 => Ok(0),
        _ => Err(SearchProviderErrorKind::AmbiguousModel),
    }
}

#[async_trait]
impl SearchProvider for CatalogSearchProvider {
    fn provider_id(&self) -> &str { "configured.openai.responses.web_search" }
    async fn search(&self, query: &str, count: usize) -> Result<SearchProviderResponse, SearchProviderError> {
        self.resolve().await?.search(query, count).await
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{body_json, header, method, path}};

    #[test]
    fn search_model_selection_is_independent_exact_and_never_order_based() {
        use nomifun_model_invoke::ModelRef;
        let model = |name: &str| ModelRef { provider_id: "provider".into(), model: name.into() };
        let conversation = model("chat-only");
        assert_eq!(select_search_model(&conversation, &[]), Err(SearchProviderErrorKind::NotConfigured));
        assert_eq!(select_search_model(&conversation, &[model("search")]), Ok(0));
        assert_eq!(select_search_model(&conversation, &[model("first"), model("second")]), Err(SearchProviderErrorKind::AmbiguousModel));
        assert_eq!(select_search_model(&model("second"), &[model("first"), model("second")]), Ok(1));
        assert_eq!(conversation.model, "chat-only");
    }

    #[tokio::test]
    async fn catalog_search_resolves_lazily_without_changing_the_chat_model() {
        use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
            SqliteProviderRepository, SqliteProviderModelRepository, SqliteProviderModelCapabilityRepository,
            SqliteProviderConnectionRepository, init_database_memory};
        use nomifun_model_invoke::{ModelInvokeService, ModelRef, AdapterRegistry, default_adapters};
        let server = MockServer::start().await;
        let database = init_database_memory().await.unwrap();
        let pool = database.pool().clone();
        let providers = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let key = [0x42; 32];
        let credential = nomifun_common::encrypt_string(r#"{"api_keys":["search-test-key"]}"#, &key).unwrap();
        let conversation = ModelRef { provider_id: nomifun_common::ProviderId::new().to_string(), model: "chat-only".into() };
        let invoke = Arc::new(ModelInvokeService::new(
            providers.clone(), Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool)), key,
            reqwest::Client::builder().no_proxy().build().unwrap(), AdapterRegistry::new(default_adapters()),
        ));
        let search = Arc::new(CatalogSearchProvider::new(invoke, conversation.clone()));
        let unavailable = search.search("test", 5).await.unwrap_err();
        assert_eq!(unavailable.kind, SearchProviderErrorKind::NotConfigured);
        assert!(server.received_requests().await.unwrap().is_empty());
        let search_id = nomifun_common::ProviderId::new().to_string();
        providers.create(CreateProviderParams {
            provider_id: Some(&search_id), platform: "openai", name: "Independent search",
            base_url: &server.uri(), auth_scheme: "bearer", credentials_encrypted: &credential,
            enabled: true, bedrock_config: None, sort_order: None,
        }, &NewProviderModel { model: "search-only", enabled: true, sort_order: 0, description: None,
            capabilities: &[NewProviderModelCapability { task: "chat", traits: r#"["web_search"]"#,
                protocol: "openai.responses", connection_role: "default", endpoint: Some("/v1/responses"),
                provider_params: "{}", context_limit: Some(131072), ..Default::default() }],
        }, &[]).await.unwrap();
        // The same already-created Tool observes the newly configured backend.
        let resolved = search.resolve().await.unwrap();
        assert_eq!(resolved.model, "search-only");
        assert_eq!(resolved.endpoint.path(), "/v1/responses");
        assert_eq!(resolved.api_key, "search-test-key");
        assert_eq!(search.conversation_model.provider_id, conversation.provider_id);
        assert_eq!(search.conversation_model.model, "chat-only");
        assert!(server.received_requests().await.unwrap().is_empty(), "model discovery must stay local");
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
        let citations = SessionCitationStore::default();
        let body = search_with_derived_citations(&provider, &citations, "rust agents", 2)
            .await
            .unwrap();
        assert_eq!(body["answer"], "Grounded answer");
        assert_eq!(body["results"][0]["source_id"], "src-1");
        assert_eq!(body["results"][0]["snippet"], "");
        let citation_id = body["results"][0]["citation_id"].as_str().unwrap();
        assert_eq!(
            body["results"][0]["citation_markdown"],
            "[Rust Agents](<https://example.com/rust>)"
        );
        assert!(citations.contains(citation_id));
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

    #[test]
    fn citations_are_result_derived_escaped_and_session_scoped() {
        let first = SessionCitationStore::default();
        let second = SessionCitationStore::default();
        let one = first.record(
            "provider",
            "source",
            "Title [unsafe]",
            "https://example.com/a_(b)",
        );
        let two = second.record(
            "provider",
            "source",
            "Title [unsafe]",
            "https://example.com/a_(b)",
        );
        assert_ne!(one.citation_id, two.citation_id);
        assert_eq!(
            one.markdown,
            "[Title \\[unsafe\\]](<https://example.com/a_(b)>)"
        );
        assert!(first.contains(&one.citation_id));
        assert!(!second.contains(&one.citation_id));
    }

    #[test]
    fn web_research_authoring_surface_has_no_citation_or_provider_action() {
        assert_eq!(
            WEB_RESEARCH_ACTION_IDS,
            ["web.research/search", "web.research/fetch"]
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
