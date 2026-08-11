//! Page retrieval for the crawler.
//!
//! Static pages ride `nomifun-knowledge`'s SSRF-guarded [`HttpFetcher`] — the
//! guard (pre-connect address validation, address pinning, per-hop redirect
//! re-validation) is not reimplemented here. Browser rendering arrives in
//! stage B via `BrowserSessionHub`; this crate never launches a browser.

use nomifun_common::AppError;
use nomifun_knowledge::source_url::{HttpFetcher, Validators};

use crate::error::CrawlError;
use crate::model::RenderMode;
use crate::politeness::parse_retry_after;

/// Below this much extracted text, a page that also carries script mount
/// points is probably an unrendered SPA shell.
const UNRENDERED_TEXT_THRESHOLD: usize = 200;

#[derive(Debug, Clone)]
pub struct FetchOutput {
    pub final_url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub retry_after_ms: Option<i64>,
    pub body: String,
    pub truncated: bool,
    /// Set when `Auto` would have escalated to a browser but stage A cannot.
    /// Recorded rather than silently ignored so the gap is visible in the UI.
    pub wanted_render: bool,
}

impl FetchOutput {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn is_not_modified(&self) -> bool {
        self.status == 304
    }

    pub fn is_html(&self) -> bool {
        self.content_type
            .as_deref()
            .map(|ct| {
                let ct = ct.to_ascii_lowercase();
                ct.contains("text/html") || ct.contains("application/xhtml")
            })
            // No content-type at all: sniff rather than discard the page.
            .unwrap_or_else(|| looks_like_html(&self.body))
    }
}

#[async_trait::async_trait]
pub trait CrawlFetcher: Send + Sync {
    async fn fetch(
        &self,
        url: &str,
        validators: &Validators,
        mode: RenderMode,
    ) -> Result<FetchOutput, CrawlError>;
}

pub struct HttpCrawlFetcher {
    inner: HttpFetcher,
}

impl HttpCrawlFetcher {
    pub fn new(user_agent: &str) -> Self {
        Self { inner: HttpFetcher::new().user_agent(user_agent) }
    }

    pub fn from_fetcher(inner: HttpFetcher) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl CrawlFetcher for HttpCrawlFetcher {
    async fn fetch(
        &self,
        url: &str,
        validators: &Validators,
        mode: RenderMode,
    ) -> Result<FetchOutput, CrawlError> {
        if mode == RenderMode::Browser {
            return Err(CrawlError::App(AppError::BadRequest(
                "browser render mode is not available yet (stage B)".into(),
            )));
        }

        let raw = self.inner.fetch_raw(url, validators).await?;
        let body = raw.body_text();
        let wanted_render = mode == RenderMode::Auto && looks_unrendered(&body);

        Ok(FetchOutput {
            final_url: raw.final_url,
            status: raw.status,
            content_type: raw.content_type,
            etag: raw.etag,
            last_modified: raw.last_modified,
            retry_after_ms: raw.retry_after.as_deref().and_then(parse_retry_after),
            body,
            truncated: raw.truncated,
            wanted_render,
        })
    }
}

fn looks_like_html(body: &str) -> bool {
    let head = body.trim_start().get(..512).unwrap_or(body).to_ascii_lowercase();
    head.contains("<!doctype html") || head.contains("<html") || head.contains("<body")
}

/// A near-empty body next to a script bundle and a mount node is the classic
/// client-rendered shell.
fn looks_unrendered(body: &str) -> bool {
    if !looks_like_html(body) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    let has_mount = lower.contains("id=\"root\"")
        || lower.contains("id=\"app\"")
        || lower.contains("id='root'")
        || lower.contains("id='app'");
    let has_script = lower.contains("<script");
    has_mount && has_script && visible_text_len(&lower) < UNRENDERED_TEXT_THRESHOLD
}

/// Rough visible-text length: strip every tag and the script/style bodies.
fn visible_text_len(lower_html: &str) -> usize {
    let mut out = 0usize;
    let mut in_tag = false;
    let mut skip_to: Option<&str> = None;
    let bytes = lower_html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(close) = skip_to {
            if lower_html[i..].starts_with(close) {
                i += close.len();
                skip_to = None;
            } else {
                i += 1;
            }
            continue;
        }
        if lower_html[i..].starts_with("<script") {
            skip_to = Some("</script>");
            i += 7;
            continue;
        }
        if lower_html[i..].starts_with("<style") {
            skip_to = Some("</style>");
            i += 6;
            continue;
        }
        match bytes[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            c if !in_tag && !c.is_ascii_whitespace() => out += 1,
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_decides_html_when_present() {
        let mut out = sample();
        out.content_type = Some("text/html; charset=utf-8".into());
        assert!(out.is_html());
        out.content_type = Some("application/pdf".into());
        assert!(!out.is_html());
    }

    #[test]
    fn missing_content_type_falls_back_to_sniffing() {
        let mut out = sample();
        out.content_type = None;
        out.body = "<!doctype html><html><body>hi</body></html>".into();
        assert!(out.is_html());
        out.body = "plain text file".into();
        assert!(!out.is_html());
    }

    #[test]
    fn spa_shell_is_flagged_as_unrendered() {
        let shell = r#"<!doctype html><html><body><div id="root"></div>
            <script src="/bundle.js"></script></body></html>"#;
        assert!(looks_unrendered(shell));
    }

    #[test]
    fn a_real_page_with_scripts_is_not_flagged() {
        let page = format!(
            r#"<!doctype html><html><body><div id="root"><article><p>{}</p></article></div>
            <script src="/analytics.js"></script></body></html>"#,
            "substantial body copy ".repeat(30)
        );
        assert!(!looks_unrendered(&page));
    }

    #[test]
    fn non_html_is_never_unrendered() {
        assert!(!looks_unrendered("{\"json\": true}"));
    }

    #[tokio::test]
    async fn browser_mode_is_refused_rather_than_silently_downgraded() {
        let fetcher = HttpCrawlFetcher::new("test");
        let err = fetcher
            .fetch("https://example.com", &Validators::default(), RenderMode::Browser)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("stage B"), "got: {err}");
    }

    fn sample() -> FetchOutput {
        FetchOutput {
            final_url: "https://example.com/".into(),
            status: 200,
            content_type: None,
            etag: None,
            last_modified: None,
            retry_after_ms: None,
            body: String::new(),
            truncated: false,
            wanted_render: false,
        }
    }

    /// The header has to survive the whole way from the wire to the throttle;
    /// a plausible-looking `None` here silently disables `Retry-After`.
    #[tokio::test]
    async fn retry_after_header_reaches_the_fetch_output() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "42")
                    .set_body_string("slow down"),
            )
            .mount(&server)
            .await;

        let fetcher = HttpCrawlFetcher::from_fetcher(
            nomifun_knowledge::source_url::HttpFetcher::new().allow_private_for_tests(),
        );
        let out = fetcher
            .fetch(&server.uri(), &Validators::default(), RenderMode::Http)
            .await
            .unwrap();

        assert_eq!(out.status, 429);
        assert_eq!(out.retry_after_ms, Some(42_000));
    }
}
