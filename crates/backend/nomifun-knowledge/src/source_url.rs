//! URL knowledge source: SSRF-guarded fetching, HTML→Markdown conversion,
//! and snapshot formatting for `{kb_root}/snapshots/{slug}.md` files.
//!
//! SSRF baseline: only http(s) URLs; the host is resolved BEFORE connecting
//! and every resolved address must be public (loopback, private, link-local,
//! CGNAT, unspecified, multicast and v4-mapped equivalents are rejected).
//! The validated addresses are pinned onto the client (`resolve_to_addrs`)
//! so the connection cannot re-resolve elsewhere, redirects are disabled in
//! reqwest and followed manually (≤ [`MAX_REDIRECTS`] hops) with the full
//! validation re-applied per hop.

use std::time::Duration;

use nomifun_common::{AppError, KnowledgeSourceItemId};
use nomifun_net::egress::{
    BodyOverflowPolicy, SafeHttpClient, SafeHttpError, SafeHttpErrorKind,
    redacted_url, validate_untrusted_url,
};
use url::Url;

/// Base-root-relative directory holding URL snapshots.
pub const SNAPSHOT_REL_DIR: &str = "snapshots";

/// Whole-request timeout per hop.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Response bodies are truncated beyond this size.
pub const FETCH_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Persisted snapshot bodies are truncated beyond this size (applies when no
/// completer is available to condense an oversized page).
pub const SNAPSHOT_MAX_BYTES: usize = 256 * 1024;
/// Maximum manual redirect hops.
pub const MAX_REDIRECTS: usize = 3;
/// Slug length cap (ASCII chars).
pub const SLUG_MAX_LEN: usize = 80;

/// A fetched page, converted to markdown.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    /// URL after redirects (the one the content actually came from).
    pub final_url: String,
    /// `<title>` of the page when it was HTML.
    pub title: Option<String>,
    pub markdown: String,
    /// True when the response body exceeded the size cap and was cut.
    pub truncated: bool,
}

/// Page-fetching seam for knowledge URL sources (same late-wire pattern as
/// [`crate::autogen::KnowledgeCompleter`]). The knowledge crate ships the
/// trait plus its HTTP implementation ([`HttpFetcher`]); a heavier
/// browser-rendering backend (`BrowserFetcher`) lives in `nomifun-ai-agent`
/// and is late-wired via [`crate::service::KnowledgeService::with_url_fetcher`],
/// so the knowledge crate never gains a browser-engine dependency (the P3
/// anti-cycle decision ②).
#[async_trait::async_trait]
pub trait PageFetcher: Send + Sync {
    /// Fetch `raw_url` and return its markdown body (+ title / final URL /
    /// truncation flag). Same contract as the original `UrlFetcher::fetch_page`.
    async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, AppError>;
}

/// SSRF-guarded HTTP page fetcher (the first [`PageFetcher`] implementation;
/// formerly `UrlFetcher`). Plain reqwest GET with HTML→markdown conversion —
/// no JS rendering. `Default` uses the production limits; tests loosen them
/// via the builder methods.
#[derive(Debug, Clone)]
pub struct HttpFetcher {
    timeout: Duration,
    max_bytes: usize,
    allow_private: bool,
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self {
            timeout: FETCH_TIMEOUT,
            max_bytes: FETCH_MAX_BYTES,
            allow_private: false,
        }
    }
}

#[async_trait::async_trait]
impl PageFetcher for HttpFetcher {
    async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, AppError> {
        // Delegate to the inherent method so direct `HttpFetcher::fetch_page`
        // callers (no trait import needed) and `dyn PageFetcher` share one body.
        HttpFetcher::fetch_page(self, raw_url).await
    }
}

impl HttpFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Disable the private/local address guard. ONLY for tests (mock HTTP
    /// servers bind to loopback).
    pub fn allow_private_for_tests(mut self) -> Self {
        self.allow_private = true;
        self
    }

    /// Fetch `raw_url` and convert the response to markdown. Every hop is
    /// SSRF-validated; bodies larger than the cap are truncated, not failed.
    pub async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, AppError> {
        let mut client = SafeHttpClient::new(self.timeout, self.max_bytes)
            .max_redirects(MAX_REDIRECTS)
            .overflow_policy(BodyOverflowPolicy::Truncate)
            .user_agent("NomiFun-Knowledge/1.0");
        if self.allow_private {
            client = client.allow_private_for_tests();
        }
        let response = client.get(raw_url).await.map_err(map_safe_http_error)?;
        if !response.status.is_success() {
            return Err(AppError::BadGateway(format!(
                "fetch failed: HTTP {} for {}",
                response.status,
                redacted_url(&response.final_url)
            )));
        }
        let content_type = response
            .headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok());
        let text = String::from_utf8_lossy(&response.body).into_owned();
        let (title, markdown) = if looks_like_html(content_type, &text) {
            html_to_markdown(&text)
        } else {
            (None, text)
        };
        Ok(FetchedPage {
            final_url: response.final_url.to_string(),
            title,
            markdown,
            truncated: response.truncated,
        })
    }
}

fn map_safe_http_error(error: SafeHttpError) -> AppError {
    match error.kind() {
        SafeHttpErrorKind::InvalidUrl | SafeHttpErrorKind::ForbiddenTarget => {
            AppError::BadRequest(error.to_string())
        }
        SafeHttpErrorKind::Timeout => AppError::Timeout(error.to_string()),
        SafeHttpErrorKind::ClientBuild => AppError::Internal(error.to_string()),
        SafeHttpErrorKind::Dns
        | SafeHttpErrorKind::Network
        | SafeHttpErrorKind::InvalidRedirect
        | SafeHttpErrorKind::TooManyRedirects
        | SafeHttpErrorKind::BodyTooLarge
        | SafeHttpErrorKind::BodyRead => AppError::BadGateway(error.to_string()),
    }
}

/// Full pre-connect validation used by the fetcher and exposed for callers
/// that want to vet a URL without fetching: scheme/host syntax plus a DNS
/// resolution where EVERY resolved address must be public.
pub async fn validate_fetch_url(raw: &str, allow_private: bool) -> Result<Url, AppError> {
    validate_untrusted_url(raw, allow_private)
        .await
        .map_err(map_safe_http_error)
}

/// Decide whether a response body should go through HTML→MD conversion.
/// The Content-Type header wins when it is conclusive; otherwise sniff the
/// body prefix for an html document marker.
fn looks_like_html(content_type: Option<&str>, body: &str) -> bool {
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("html") {
            return true;
        }
        if ct.contains("markdown") || ct.contains("text/plain") || ct.contains("json") {
            return false;
        }
    }
    let head: String = body.trim_start().chars().take(256).collect::<String>().to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<html")
}

/// Convert HTML to markdown via `htmd`, falling back to `<title>` + stripped
/// body text when conversion fails. Returns `(title, markdown)`.
pub fn html_to_markdown(html: &str) -> (Option<String>, String) {
    let title = extract_html_title(html);
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "head", "iframe", "noscript"])
        .build();
    let markdown = match converter.convert(html) {
        Ok(md) if !md.trim().is_empty() => md,
        _ => {
            let mut text = strip_tags(html);
            if let Some(t) = &title {
                text = format!("# {t}\n\n{text}");
            }
            text
        }
    };
    (title, markdown)
}

/// First `<title>…</title>` content, whitespace-collapsed.
fn extract_html_title(html: &str) -> Option<String> {
    // ASCII-only lowercasing keeps byte offsets aligned with `html` (full
    // `to_lowercase` can change byte lengths, e.g. 'İ' → "i̇").
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let open_end = lower[open..].find('>').map(|i| open + i + 1)?;
    let close = lower[open_end..].find("</title").map(|i| open_end + i)?;
    let title = html.get(open_end..close)?;
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// Crude tag stripper used only as a conversion fallback: drops `<…>` spans
/// and collapses blank-line runs.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let mut lines: Vec<&str> = Vec::new();
    let mut last_blank = true;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_blank {
                lines.push("");
            }
            last_blank = true;
        } else {
            lines.push(trimmed);
            last_blank = false;
        }
    }
    lines.join("\n").trim().to_owned()
}

/// Derive a snapshot file slug from the URL host+path: lowercase ASCII
/// `[a-z0-9-]`, runs of other chars collapsed to single dashes, capped at
/// [`SLUG_MAX_LEN`]. Never empty.
pub fn slug_for_url(url: &Url) -> String {
    let raw = format!("{}{}", url.host_str().unwrap_or_default(), url.path());
    let mut slug = String::new();
    for c in raw.chars() {
        if slug.len() >= SLUG_MAX_LEN {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() { "page".into() } else { slug }
}

/// Assemble a snapshot document: a readable Markdown metadata quote
/// (`source_url`, `fetched_at`, optional `title`) followed by the page body.
pub fn snapshot_markdown(source_url: &str, fetched_at: &str, title: Option<&str>, body: &str) -> String {
    snapshot_markdown_with_identity(None, source_url, None, fetched_at, title, false, body)
}

/// Assemble a managed snapshot with a durable source-item marker.  The marker
/// survives moves, renames, export/import, and projection rebuilds, so a
/// directory name is never needed to infer ownership.
pub fn managed_snapshot_markdown(
    source_item_id: &KnowledgeSourceItemId,
    source_url: &str,
    final_url: Option<&str>,
    fetched_at: &str,
    title: Option<&str>,
    truncated: bool,
    body: &str,
) -> String {
    snapshot_markdown_with_identity(
        Some(source_item_id),
        source_url,
        final_url,
        fetched_at,
        title,
        truncated,
        body,
    )
}

fn snapshot_markdown_with_identity(
    source_item_id: Option<&KnowledgeSourceItemId>,
    source_url: &str,
    final_url: Option<&str>,
    fetched_at: &str,
    title: Option<&str>,
    truncated: bool,
    body: &str,
) -> String {
    let mut out = format!("> **source_url**: {source_url}\n> **fetched_at**: {fetched_at}\n");
    if let Some(source_item_id) = source_item_id {
        out.push_str(&format!(
            "> **nomifun_source_item_id**: {source_item_id}\n"
        ));
        out.push_str("> **nomifun_source_relationship**: managed\n");
    }
    if let Some(final_url) = final_url.filter(|url| *url != source_url) {
        out.push_str(&format!("> **final_url**: {final_url}\n"));
    }
    if truncated {
        out.push_str("> **truncated**: true\n");
    }
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        // Collapse runs of whitespace (incl. newlines/tabs): a multi-line
        // title must remain on one metadata line.
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str(&format!("> **title**: \"{}\"\n", title.replace('"', "'")));
    }
    out.push_str("---\n\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

/// Extract and validate the durable source-item marker from the leading
/// managed header.  A malformed marker is treated as absent here; callers
/// that reconcile ownership detect duplicate/ambiguous valid markers and fail
/// closed before publishing.
pub fn snapshot_source_item_id(content: &str) -> Option<KnowledgeSourceItemId> {
    let mut lines = content.lines();
    let first = lines.next()?.trim();
    if !first.starts_with("> **source_url**:") && first != "---" {
        return None;
    }

    if first == "---" {
        for line in lines {
            let trimmed = line.trim();
            if trimmed == "---" {
                return None;
            }
            if let Some(value) = trimmed.strip_prefix("nomifun_source_item_id:") {
                return KnowledgeSourceItemId::parse(value.trim()).ok();
            }
        }
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        if let Some(value) = trimmed.strip_prefix("> **nomifun_source_item_id**:") {
            return KnowledgeSourceItemId::parse(value.trim()).ok();
        }
    }
    None
}

/// Relationship marker stored in managed/detached/copied snapshot headers.
/// Legacy snapshots omit it and are treated as `managed` only while they are
/// uniquely matched to an active source URL during migration.
pub fn snapshot_source_relationship(content: &str) -> Option<&str> {
    let mut lines = content.lines();
    let first = lines.next()?.trim();
    if !first.starts_with("> **source_url**:") && first != "---" {
        return None;
    }
    let (prefix, end) = if first == "---" {
        ("nomifun_source_relationship:", "---")
    } else {
        ("> **nomifun_source_relationship**:", "---")
    };
    for line in lines {
        let trimmed = line.trim();
        if trimmed == end {
            return None;
        }
        if let Some(value) = trimmed.strip_prefix(prefix) {
            let value = value.trim();
            return matches!(value, "managed" | "detached" | "copy").then_some(value);
        }
    }
    None
}

/// Extract the `source_url` value from a snapshot header. New snapshots use
/// the Markdown quote written by [`snapshot_markdown`]; legacy YAML
/// frontmatter remains supported so existing knowledge bases keep refreshing.
/// Only a leading header is consulted, so a body-level source label never
/// marks a user-authored file as a managed snapshot.
pub fn snapshot_source_url(content: &str) -> Option<&str> {
    let first_line = content.lines().next()?.trim();
    if let Some(value) = first_line.strip_prefix("> **source_url**:") {
        let value = value.trim();
        return (!value.is_empty()).then_some(value);
    }

    // Backward compatibility for snapshots created with the former YAML
    // frontmatter template.
    let rest = content.strip_prefix("---")?;
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))?;
    for line in rest.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None; // frontmatter ended without the field
        }
        if let Some(value) = trimmed.strip_prefix("source_url:") {
            let value = value.trim();
            return (!value.is_empty()).then_some(value);
        }
    }
    None
}

/// Truncate a string to at most `max_bytes`, never splitting a char.
pub fn truncate_to_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_fetcher() -> HttpFetcher {
        HttpFetcher::new().allow_private_for_tests()
    }

    // ── PageFetcher seam ─────────────────────────────────────────────

    /// A non-HTTP [`PageFetcher`] (returns a canned page without touching the
    /// network) — proves the trait is object-safe and a custom backend can
    /// stand in for `HttpFetcher` behind `dyn PageFetcher` (the K2
    /// `BrowserFetcher` seam).
    struct CannedFetcher(FetchedPage);

    #[async_trait::async_trait]
    impl PageFetcher for CannedFetcher {
        async fn fetch_page(&self, _raw_url: &str) -> Result<FetchedPage, AppError> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn http_fetcher_is_usable_behind_dyn_page_fetcher() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/doc"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><head><title>渲染</title></head><body><h1>X</h1></body></html>",
                "text/html; charset=utf-8",
            ))
            .mount(&server)
            .await;

        // Same code path, reached through the trait object rather than the
        // concrete type.
        let fetcher: std::sync::Arc<dyn PageFetcher> = std::sync::Arc::new(test_fetcher());
        let page = fetcher.fetch_page(&format!("{}/doc", server.uri())).await.unwrap();
        assert_eq!(page.title.as_deref(), Some("渲染"));
        assert!(page.markdown.contains("# X"), "got: {}", page.markdown);
    }

    #[tokio::test]
    async fn custom_page_fetcher_can_replace_http() {
        let fetcher: std::sync::Arc<dyn PageFetcher> = std::sync::Arc::new(CannedFetcher(FetchedPage {
            final_url: "https://spa.example.com/app".into(),
            title: Some("Rendered SPA".into()),
            markdown: "# Rendered\n\ncontent only a browser would see".into(),
            truncated: false,
        }));
        // The injected backend decides the result — no network involved.
        let page = fetcher.fetch_page("https://spa.example.com/app").await.unwrap();
        assert_eq!(page.title.as_deref(), Some("Rendered SPA"));
        assert!(page.markdown.contains("only a browser would see"));
        assert!(!page.truncated);
    }

    // ── SSRF validation ──────────────────────────────────────────────

    #[tokio::test]
    async fn validate_rejects_non_http_schemes() {
        for url in ["ftp://example.com/x", "file:///etc/passwd", "gopher://x", "javascript:alert(1)"] {
            let err = validate_fetch_url(url, false).await.unwrap_err();
            assert!(matches!(err, AppError::BadRequest(_)), "{url} → {err:?}");
        }
        assert!(validate_fetch_url("not a url", false).await.is_err());
    }

    #[tokio::test]
    async fn validate_rejects_loopback_private_and_linklocal() {
        for url in [
            "http://127.0.0.1/x",
            "http://127.8.8.8:9000/",
            "http://localhost/x",
            "http://10.0.0.5/",
            "http://172.16.1.1/",
            "http://192.168.1.1/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://[::1]/x",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            let err = validate_fetch_url(url, false).await.unwrap_err();
            assert!(
                matches!(err, AppError::BadRequest(_)),
                "{url} must be rejected, got {err:?}"
            );
        }
        // The test override admits loopback (mock servers).
        assert!(validate_fetch_url("http://127.0.0.1:1/x", true).await.is_ok());
    }

    /// Obfuscated IPv4 literal notations (decimal, hex, octal): the url crate
    /// normalizes them all to dotted-quad form per the WHATWG URL spec, so
    /// the private-address guard must fire exactly as for `http://127.0.0.1/`.
    #[tokio::test]
    async fn validate_rejects_obfuscated_ipv4_literals() {
        for url in ["http://2130706433/", "http://0x7f000001/", "http://0177.0.0.1/"] {
            let err = validate_fetch_url(url, false).await.unwrap_err();
            assert!(matches!(err, AppError::BadRequest(_)), "{url} must be rejected, got {err:?}");
        }
        // Sanity-check the normalization assumption this test rests on.
        assert_eq!(Url::parse("http://2130706433/").unwrap().host_str(), Some("127.0.0.1"));
        assert_eq!(Url::parse("http://0x7f000001/").unwrap().host_str(), Some("127.0.0.1"));
        assert_eq!(Url::parse("http://0177.0.0.1/").unwrap().host_str(), Some("127.0.0.1"));
    }

    /// Per-hop redirect validation is owned by the shared safe HTTP client.
    /// An end-to-end
    /// wiremock test CANNOT cover the rejection: the mock server itself binds
    /// to loopback, so reaching hop 1 requires `allow_private` — which would
    /// also admit the private hop 2. This test drives the exact same functions
    /// on a redirect Location target instead.
    #[tokio::test]
    async fn redirect_hop_to_private_target_is_rejected() {
        // Absolute Location to the cloud metadata endpoint (classic SSRF pivot).
        let err = validate_fetch_url("http://169.254.169.254/latest/meta-data/", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(err.to_string().contains("forbidden address"), "{err}");

        // A redirect downgrading to a non-http scheme is rejected too.
        assert!(nomifun_net::egress::parse_untrusted_url("ftp://internal/").is_err());
    }

    #[test]
    fn forbidden_ip_policy() {
        let bad = ["127.0.0.1", "10.1.2.3", "172.31.0.1", "192.168.0.1", "169.254.0.1", "0.0.0.0", "100.100.0.1", "192.0.0.5", "224.0.0.1"];
        for ip in bad {
            assert!(nomifun_net::egress::forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        let good = ["1.1.1.1", "8.8.8.8", "93.184.216.34", "100.128.0.1", "172.32.0.1"];
        for ip in good {
            assert!(!nomifun_net::egress::forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(nomifun_net::egress::forbidden_ip("::1".parse().unwrap()));
        assert!(nomifun_net::egress::forbidden_ip("fe80::1".parse().unwrap()));
        assert!(nomifun_net::egress::forbidden_ip("fd12:3456::1".parse().unwrap()));
        assert!(nomifun_net::egress::forbidden_ip("::ffff:192.168.0.1".parse().unwrap()));
        assert!(!nomifun_net::egress::forbidden_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));
    }

    // ── slug / snapshot metadata / conversion ───────────────────────

    #[test]
    fn slug_rules() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(slug_for_url(&u("https://docs.example.com/api/v2/Users")), "docs-example-com-api-v2-users");
        assert_eq!(slug_for_url(&u("https://example.com/")), "example-com");
        assert_eq!(slug_for_url(&u("https://example.com/a//b__c")), "example-com-a-b-c");
        let long = slug_for_url(&u(&format!("https://example.com/{}", "x".repeat(200))));
        assert!(long.len() <= SLUG_MAX_LEN, "{}", long.len());
        assert!(!long.ends_with('-'));
    }

    #[test]
    fn snapshot_metadata_shape() {
        let md = snapshot_markdown(
            "https://example.com/docs",
            "2026-06-12T12:00:00Z",
            Some("My \"Docs\""),
            "# Title\n\nBody",
        );
        assert!(
            md.starts_with(
                "> **source_url**: https://example.com/docs\n> **fetched_at**: 2026-06-12T12:00:00Z\n"
            ),
            "got: {md}"
        );
        assert!(md.contains("> **title**: \"My 'Docs'\""), "got: {md}");
        assert!(md.contains("---\n\n# Title\n\nBody\n"), "got: {md}");
        // No title line when absent.
        let md = snapshot_markdown("https://e.com", "2026-01-01T00:00:00Z", None, "b");
        assert!(!md.contains("**title**:"), "got: {md}");
        // Newlines/tabs in a title collapse to single spaces — a multi-line
        // title must not break out of its metadata line.
        let md = snapshot_markdown("https://e.com", "2026-01-01T00:00:00Z", Some("Line one\nLine\ttwo"), "b");
        assert!(md.contains("> **title**: \"Line one Line two\"\n"), "got: {md}");
    }

    #[test]
    fn snapshot_source_url_reads_only_managed_headers() {
        // Round-trip with the writer.
        let md = snapshot_markdown("https://e.com/docs", "2026-01-01T00:00:00Z", Some("T"), "body");
        assert_eq!(snapshot_source_url(&md), Some("https://e.com/docs"));
        // User-authored files: no managed header, or a body-level label.
        assert_eq!(snapshot_source_url("# notes\nsource_url: https://nope"), None);
        assert_eq!(snapshot_source_url("# notes\n> **source_url**: https://nope"), None);
        assert_eq!(snapshot_source_url("---\ntitle: x\n---\n\nsource_url: https://nope"), None);
        assert_eq!(snapshot_source_url("> **source_url**:\n---\n"), None, "empty value is no value");
        assert_eq!(snapshot_source_url("---\nsource_url:\n---\n"), None, "empty value is no value");
        assert_eq!(snapshot_source_url(""), None);
        // Both the new header and legacy YAML remain compatible with CRLF.
        assert_eq!(
            snapshot_source_url("> **source_url**: https://e.com/new\r\n> **fetched_at**: now\r\n---\r\nbody"),
            Some("https://e.com/new")
        );
        assert_eq!(snapshot_source_url("---\r\nsource_url: https://e.com/x\r\n---\r\nbody"), Some("https://e.com/x"));
    }

    #[test]
    fn managed_snapshot_round_trips_stable_source_item_identity() {
        let source_item_id = KnowledgeSourceItemId::new();
        let md = managed_snapshot_markdown(
            &source_item_id,
            "https://e.com/docs",
            Some("https://www.e.com/docs"),
            "2026-01-01T00:00:00Z",
            Some("Docs"),
            true,
            "body",
        );
        assert_eq!(snapshot_source_item_id(&md).as_ref(), Some(&source_item_id));
        assert!(md.contains("> **final_url**: https://www.e.com/docs"));
        assert!(md.contains("> **truncated**: true"));
        assert_eq!(snapshot_source_item_id("# user note\nbody"), None);
    }

    #[test]
    fn html_conversion_and_fallback() {
        let html = "<html><head><title>Guide  Page</title><script>evil()</script></head>\
                    <body><h1>Guide</h1><p>Hello <b>world</b></p></body></html>";
        let (title, md) = html_to_markdown(html);
        assert_eq!(title.as_deref(), Some("Guide Page"));
        assert!(md.contains("# Guide"), "got: {md}");
        assert!(md.contains("**world**"), "got: {md}");
        assert!(!md.contains("evil()"), "script content must be skipped: {md}");

        // Tag stripping fallback keeps readable text.
        let text = strip_tags("<div><p>第一段</p>\n\n\n<p>第二段</p></div>");
        assert!(text.contains("第一段") && text.contains("第二段"), "got: {text}");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "知识库snapshot";
        let t = truncate_to_bytes(s, 4);
        assert_eq!(t, "知");
        assert_eq!(truncate_to_bytes("abc", 10), "abc");
    }

    // ── fetching (mock HTTP) ─────────────────────────────────────────

    #[tokio::test]
    async fn fetch_converts_html_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/doc"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                "<html><head><title>接口文档</title></head><body><h1>API</h1><p>说明</p></body></html>",
                "text/html; charset=utf-8",
            ))
            .mount(&server)
            .await;

        let page = test_fetcher().fetch_page(&format!("{}/doc", server.uri())).await.unwrap();
        assert_eq!(page.title.as_deref(), Some("接口文档"));
        assert!(page.markdown.contains("# API"), "got: {}", page.markdown);
        assert!(page.markdown.contains("说明"));
        assert!(!page.truncated);
    }

    #[tokio::test]
    async fn fetch_passes_plaintext_through_and_truncates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big.md"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("x".repeat(4096)),
            )
            .mount(&server)
            .await;

        let fetcher = test_fetcher().max_bytes(256);
        let page = fetcher.fetch_page(&format!("{}/big.md", server.uri())).await.unwrap();
        assert!(page.truncated);
        assert!(page.markdown.len() <= 256, "{}", page.markdown.len());
        assert!(page.title.is_none());
    }

    /// A body of exactly `max_bytes` is kept whole and not flagged truncated.
    #[tokio::test]
    async fn fetch_body_exactly_at_cap_is_not_truncated() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/exact"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain")
                    .set_body_string("x".repeat(256)),
            )
            .mount(&server)
            .await;

        let fetcher = test_fetcher().max_bytes(256);
        let page = fetcher.fetch_page(&format!("{}/exact", server.uri())).await.unwrap();
        assert!(!page.truncated, "exactly-at-cap body must not be flagged");
        assert_eq!(page.markdown.len(), 256);
    }

    #[tokio::test]
    async fn fetch_follows_bounded_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/b"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/b"))
            .respond_with(ResponseTemplate::new(200).set_body_string("landed"))
            .mount(&server)
            .await;
        // Self-redirect loop.
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
            .mount(&server)
            .await;

        let page = test_fetcher().fetch_page(&format!("{}/a", server.uri())).await.unwrap();
        assert!(page.markdown.contains("landed"));
        assert!(page.final_url.ends_with("/b"), "{}", page.final_url);

        let err = test_fetcher().fetch_page(&format!("{}/loop", server.uri())).await.unwrap_err();
        assert!(err.to_string().contains("too many redirects"), "{err}");
    }

    #[tokio::test]
    async fn fetch_times_out_and_reports_http_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let fetcher = test_fetcher().timeout(Duration::from_millis(200));
        let err = fetcher.fetch_page(&format!("{}/slow", server.uri())).await.unwrap_err();
        assert!(matches!(err, AppError::Timeout(_)), "{err:?}");

        let err = test_fetcher().fetch_page(&format!("{}/missing", server.uri())).await.unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
    }

    /// Without the test override, fetching the loopback mock server must be
    /// rejected by the pre-connect guard (never reaches the socket).
    #[tokio::test]
    async fn fetch_blocks_private_targets_by_default() {
        let server = MockServer::start().await;
        let err = HttpFetcher::new().fetch_page(&format!("{}/doc", server.uri())).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(err.to_string().contains("forbidden address"), "{err}");
    }
}
