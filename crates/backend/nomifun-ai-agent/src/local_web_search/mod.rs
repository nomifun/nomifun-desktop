//! Independent browser-backed public search. Never aliases vendor web_search.
mod search_engine;

use async_trait::async_trait;
use nomi_browser_engine::headless_page::{HeadlessPageError, PageRequest, extract_page};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSearchBinding {
    pub schema_version: u32,
    pub runtime_build_digest: String,
    pub adapter_digest: String,
    pub browser_binary_digest: String,
    pub browser_product: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocalSearchError {
    #[error("The local browser search capacity is busy; try again later.")]
    Busy,
    #[error("The local search runtime no longer matches this Agent Snapshot.")]
    BindingChanged,
    #[error("Local browser search is unavailable.")]
    Unavailable,
    #[error("Local browser search was blocked by network policy or the search engine.")]
    Blocked,
    #[error("Local browser search timed out.")]
    Timeout,
    #[error("Local browser search was canceled.")]
    Canceled,
    #[error("The search engine requires human verification or consent.")]
    Challenge,
    #[error("The search page no longer matches this adapter.")]
    InvalidResult,
    #[error("Local search browser cleanup could not be proven.")]
    Cleanup,
    #[error("query must contain 1–2048 characters and limit must be 1–10.")]
    InvalidInput,
}
impl LocalSearchError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Busy => "WEB_RESEARCH_LOCAL_BUSY",
            Self::BindingChanged => "WEB_RESEARCH_LOCAL_BINDING_CHANGED",
            Self::Unavailable => "WEB_RESEARCH_LOCAL_UNAVAILABLE",
            Self::Blocked => "WEB_RESEARCH_LOCAL_BLOCKED",
            Self::Timeout => "WEB_RESEARCH_LOCAL_TIMEOUT",
            Self::Canceled => "WEB_RESEARCH_LOCAL_CANCELED",
            Self::Challenge => "WEB_RESEARCH_LOCAL_CHALLENGE",
            Self::InvalidResult => "WEB_RESEARCH_LOCAL_RESULT_INVALID",
            Self::Cleanup => "WEB_RESEARCH_LOCAL_CLEANUP_FAILED",
            Self::InvalidInput => "INVALID_PAYLOAD",
        }
    }
}
impl From<HeadlessPageError> for LocalSearchError {
    fn from(error: HeadlessPageError) -> Self {
        match error {
            HeadlessPageError::Busy => Self::Busy,
            HeadlessPageError::BindingChanged => Self::BindingChanged,
            HeadlessPageError::Unavailable => Self::Unavailable,
            HeadlessPageError::Blocked => Self::Blocked,
            HeadlessPageError::Timeout => Self::Timeout,
            HeadlessPageError::Canceled => Self::Canceled,
            HeadlessPageError::Challenge => Self::Challenge,
            HeadlessPageError::InvalidResult => Self::InvalidResult,
            HeadlessPageError::Cleanup => Self::Cleanup,
        }
    }
}

#[derive(Serialize)]
pub struct LocalSearchResult {
    pub citation_id: String,
    pub rank: usize,
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Resolved by the application, never by a model route or a browser preference.
#[derive(Clone)]
pub struct BrowserSearchProvider {
    chrome: PathBuf,
    locale: String,
    binding: LocalSearchBinding,
}
impl BrowserSearchProvider {
    pub fn for_locale(&self, locale: String) -> Result<Self, LocalSearchError> {
        if locale.is_empty()
            || locale.len() > 32
            || !locale
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(LocalSearchError::Unavailable);
        }
        let mut provider = self.clone();
        provider.locale = locale;
        Ok(provider)
    }
    pub async fn validate_binding(
        &self,
        expected: &LocalSearchBinding,
    ) -> Result<(), LocalSearchError> {
        self.validate_binding_files(expected).await?;
        // PageRequest verifies the live product before creating its page.
        // Session construction must not launch a second probe browser.
        Ok(())
    }
    async fn validate_binding_files(
        &self,
        expected: &LocalSearchBinding,
    ) -> Result<(), LocalSearchError> {
        if expected != &self.binding
            || expected.schema_version != 1
            || expected.runtime_build_digest
                != nomi_browser_engine::headless_page::implementation_digest()
            || expected.adapter_digest != Self::current_adapter_digest()
            || expected.browser_binary_digest != nomi_browser_engine::headless_page::binary_digest(self.chrome.clone()).await?
        {
            return Err(LocalSearchError::BindingChanged);
        }
        Ok(())
    }
    fn validate_installation(chrome: &std::path::Path, locale: &str) -> Result<(),LocalSearchError> {
        if !chrome.is_absolute()
            || !chrome.is_file()
            || locale.is_empty()
            || locale.len() > 32
            || !locale
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(LocalSearchError::Unavailable);
        }
        Ok(())
    }
    /// Host-only installed-release metadata. No browser process or web request
    /// is created here; live product identity is mandatory on the query path.
    pub async fn from_installed_release(chrome: PathBuf, locale: String, browser_product: String) -> Result<Self,LocalSearchError> {
        Self::validate_installation(&chrome,&locale)?;
        let digest=nomi_browser_engine::headless_page::installed_release_digest(&chrome,&browser_product).await?;
        Ok(Self::from_fingerprint(chrome,locale,browser_product,digest))
    }
    pub async fn new(chrome: PathBuf, locale: String) -> Result<Self, LocalSearchError> {
        Self::validate_installation(&chrome,&locale)?;
        let before = nomi_browser_engine::headless_page::binary_digest(chrome.clone()).await?;
        let browser_product =
            nomi_browser_engine::headless_page::probe_runtime(chrome.clone()).await?;
        if nomi_browser_engine::headless_page::binary_digest(chrome.clone()).await? != before {
            return Err(LocalSearchError::BindingChanged);
        }
        Ok(Self::from_fingerprint(chrome,locale,browser_product,before))
    }
    fn from_fingerprint(chrome:PathBuf,locale:String,browser_product:String,digest:String)->Self {
        let binding = LocalSearchBinding {
            schema_version: 1,
            runtime_build_digest: nomi_browser_engine::headless_page::implementation_digest(),
            adapter_digest: Self::current_adapter_digest(),
            browser_binary_digest: digest,
            browser_product,
        };
        Self {
            chrome,
            locale,
            binding,
        }
    }
    pub fn binding(&self) -> &LocalSearchBinding {
        &self.binding
    }
    fn current_adapter_digest() -> String {
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}\0{}",
                search_engine::ADAPTER_ID,
                include_str!("search_engine.rs")
            ))
        )
    }
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        cancel: CancellationToken,
    ) -> Result<Vec<LocalSearchResult>, LocalSearchError> {
        self.search_bound(&self.binding, query, limit, cancel).await
    }
    pub async fn search_bound(
        &self,
        expected: &LocalSearchBinding,
        query: &str,
        limit: usize,
        cancel: CancellationToken,
    ) -> Result<Vec<LocalSearchResult>, LocalSearchError> {
        if query.trim().is_empty() || query.chars().count() > 2048 || !(1..=10).contains(&limit) {
            return Err(LocalSearchError::InvalidInput);
        }
        self.validate_binding_files(expected).await?;
        let output = extract_page(
            self.chrome.clone(),
            PageRequest {
                url: search_engine::query_url(query, &self.locale),
                allowed_origins: search_engine::origins(),
                purpose: nomi_browser_engine::headless_page::PagePurpose::Search,
                extraction: search_engine::EXTRACTION,
                language: self.locale.clone(),
                expected_browser_product: Some(expected.browser_product.clone()),
            },
            cancel,
        )
        .await?;
        if output["state"] == "empty" {
            return Ok(Vec::new());
        }
        let rows = serde_json::from_value(output["results"].clone())
            .map_err(|_| LocalSearchError::InvalidResult)?;
        let mut results = search_engine::normalize(rows, limit)?;
        for result in &mut results {
            result.citation_id = citation_id(query, &result.url);
        }
        Ok(results)
    }
}

#[async_trait]
impl crate::web_search::SearchProvider for BrowserSearchProvider {
    fn provider_id(&self) -> &str {
        "nomifun.local.browser"
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
    ) -> Result<crate::web_search::SearchProviderResponse, crate::web_search::SearchProviderError>
    {
        let results = BrowserSearchProvider::search(
            self,
            query,
            count.min(10),
            CancellationToken::new(),
        )
        .await
        .map_err(|error| {
            use crate::web_search::SearchProviderErrorKind;
            let kind = match error {
                LocalSearchError::Timeout | LocalSearchError::Canceled => {
                    SearchProviderErrorKind::Timeout
                }
                LocalSearchError::InvalidInput => SearchProviderErrorKind::InvalidResponse,
                LocalSearchError::InvalidResult | LocalSearchError::Challenge => {
                    SearchProviderErrorKind::InvalidResponse
                }
                LocalSearchError::Unavailable | LocalSearchError::BindingChanged => {
                    SearchProviderErrorKind::NotConfigured
                }
                _ => SearchProviderErrorKind::Transport,
            };
            crate::web_search::SearchProviderError::new(kind, error.to_string())
        })?;
        if results.is_empty() {
            return Err(crate::web_search::SearchProviderError::new(
                crate::web_search::SearchProviderErrorKind::NoSources,
                "local browser search returned no public sources",
            ));
        }
        Ok(crate::web_search::SearchProviderResponse {
            answer: String::new(),
            results: results
                .into_iter()
                .map(|result| crate::web_search::SearchProviderResult {
                    source_id: result.citation_id,
                    title: result.title,
                    url: result.url,
                    snippet: result.snippet,
                })
                .collect(),
        })
    }
}

fn citation_id(query: &str, url: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(format!("{query}\0{url}")));
    format!("nomi-local-search-{}", &digest[..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn installed_release_is_pinned_without_running_the_file() {
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join("chrome.exe");
        std::fs::write(&path,b"not an executable").unwrap();
        let provider=BrowserSearchProvider::from_installed_release(path.clone(),"en-US".into(),"Chrome/152.0.1.2".into()).await.unwrap();
        provider.validate_binding(provider.binding()).await.unwrap();
        assert_eq!(provider.binding().browser_product,"Chrome/152.0.1.2");
        std::fs::write(&path,b"changed release").unwrap();
        assert_eq!(provider.validate_binding(provider.binding()).await.unwrap_err(),LocalSearchError::BindingChanged);
        for product in ["Chrome/119.0.0.0","Edg/152.0.1.2","Chrome/152.0.1","Chrome/152.0.1.x"] {
            assert!(BrowserSearchProvider::from_installed_release(path.clone(),"en-US".into(),product.into()).await.is_err());
        }
    }
    async fn fixture_provider(path: PathBuf) -> BrowserSearchProvider {
        BrowserSearchProvider {
            chrome: path.clone(),
            locale: "en-US".into(),
            binding: LocalSearchBinding {
                schema_version: 1,
                runtime_build_digest: nomi_browser_engine::headless_page::implementation_digest(),
                adapter_digest: BrowserSearchProvider::current_adapter_digest(),
                browser_binary_digest: nomi_browser_engine::headless_page::binary_digest(path).await.unwrap(),
                browser_product: "fixture".into(),
            },
        }
    }
    #[tokio::test]
    async fn citation_provenance_is_scoped_to_the_same_session() {
        let citations = crate::web_search::SessionCitationStore::default();
        let local = citation_id("query", "https://example.com/local");
        assert!(local.len() <= 64);
        assert_eq!(local, citation_id("query", "https://example.com/local"));
        let derived = citations.record(
            "nomi.local.browser",
            &local,
            "Local source",
            "https://example.com/local",
        );
        assert!(derived.markdown.contains("Local source"));
    }

    #[tokio::test]
    #[ignore = "real public search; requires NOMIFUN_SEARCH_CHROME and public network access"]
    async fn real_browser_search_returns_organic_sources() {
        let _=tracing_subscriber::fmt().with_env_filter("nomi_browser_engine::headless_page=debug").try_init();
        let provider = BrowserSearchProvider::new(
            PathBuf::from(std::env::var_os("NOMIFUN_SEARCH_CHROME").expect("Chromium binary")),
            "en-US".into(),
        )
        .await
        .unwrap();
        let results = provider
            .search("Tauri WebView2 documentation", 3, CancellationToken::new())
            .await
            .expect("real browser organic results");
        assert!(!results.is_empty());
        assert!(results.len() <= 3);
        for result in &results {
            assert!(result.citation_id.starts_with("nomi-local-search-"));
            assert!(!result.title.is_empty());
        }
        println!(
            "WEB_RESEARCH_LOCAL_REAL_PASS {}",
            serde_json::to_string(&results).unwrap()
        );
    }

    #[tokio::test]
    async fn changed_snapshot_or_browser_binary_is_rejected_before_launch() {
        use std::io::Write;
        let mut executable = tempfile::NamedTempFile::new().unwrap();
        executable.write_all(b"first").unwrap();
        let provider = fixture_provider(executable.path().to_owned()).await;
        let mut changed = provider.binding().clone();
        changed.adapter_digest = "different".into();
        assert!(matches!(
            provider
                .search_bound(&changed, "query", 1, CancellationToken::new())
                .await,
            Err(LocalSearchError::BindingChanged)
        ));
        executable.write_all(b"changed").unwrap();
        assert!(matches!(
            provider.search("query", 1, CancellationToken::new()).await,
            Err(LocalSearchError::BindingChanged)
        ));
    }
}
