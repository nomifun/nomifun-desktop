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

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use futures_util::StreamExt;
use nomifun_common::AppError;
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

/// Backward-compatible alias for the pre-trait name. External callers (e.g.
/// `nomifun-gateway::tools_knowledge`) still refer to `UrlFetcher`; it is now
/// the concrete HTTP implementation of [`PageFetcher`].
pub type UrlFetcher = HttpFetcher;

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
        let mut url = parse_fetch_url(raw_url)?;
        for _hop in 0..=MAX_REDIRECTS {
            let addrs = resolve_validated(&url, self.allow_private).await?;
            let response = self.send(&url, &addrs).await?;
            let status = response.status();

            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| AppError::BadGateway(format!("redirect without Location from {url}")))?;
                let next = url
                    .join(location)
                    .map_err(|e| AppError::BadGateway(format!("invalid redirect target {location}: {e}")))?;
                url = check_scheme(next)?;
                continue;
            }
            if !status.is_success() {
                return Err(AppError::BadGateway(format!("fetch failed: HTTP {status} for {url}")));
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let (body, truncated) = self.read_capped(response).await?;
            let text = String::from_utf8_lossy(&body).into_owned();

            let (title, markdown) = if looks_like_html(content_type.as_deref(), &text) {
                html_to_markdown(&text)
            } else {
                (None, text)
            };
            return Ok(FetchedPage {
                final_url: url.to_string(),
                title,
                markdown,
                truncated,
            });
        }
        Err(AppError::BadGateway(format!("too many redirects fetching {raw_url}")))
    }

    async fn send(&self, url: &Url, addrs: &[SocketAddr]) -> Result<reqwest::Response, AppError> {
        // A fresh Client per hop is deliberate: `resolve_to_addrs` pins one
        // host's pre-validated addresses onto the client, and every redirect
        // hop may land on a different host needing its own pinning.
        let mut builder = nomifun_net::proxy::apply_detected_proxy(reqwest::Client::builder())
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout);
        // Pin the pre-validated addresses so the actual connection cannot be
        // re-resolved to a different (private) host (DNS rebinding).
        if let Some(host) = url.host_str()
            && !addrs.is_empty()
        {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = builder
            .build()
            .map_err(|e| AppError::Internal(format!("failed to build http client: {e}")))?;
        client
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, "NomiFun-Knowledge/1.0")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AppError::Timeout(format!("fetch timed out for {url}"))
                } else {
                    AppError::BadGateway(format!("fetch failed for {url}: {e}"))
                }
            })
    }

    /// Drain the body up to `max_bytes`; longer bodies are truncated. A body
    /// of exactly `max_bytes` is kept whole and NOT flagged as truncated.
    async fn read_capped(&self, response: reqwest::Response) -> Result<(Vec<u8>, bool), AppError> {
        let mut body: Vec<u8> = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) if e.is_timeout() => return Err(AppError::Timeout(format!("fetch body timed out: {e}"))),
                Err(e) => return Err(AppError::BadGateway(format!("fetch body failed: {e}"))),
            };
            if body.len() + chunk.len() > self.max_bytes {
                let take = self.max_bytes - body.len();
                body.extend_from_slice(&chunk[..take]);
                return Ok((body, true));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((body, false))
    }
}

/// Parse + scheme-check a fetch URL (no DNS yet).
fn parse_fetch_url(raw: &str) -> Result<Url, AppError> {
    let url = Url::parse(raw.trim()).map_err(|e| AppError::BadRequest(format!("invalid URL: {e}")))?;
    check_scheme(url)
}

fn check_scheme(url: Url) -> Result<Url, AppError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::BadRequest(format!(
            "only http(s) URLs are supported (got scheme: {})",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(AppError::BadRequest("URL has no host".into()));
    }
    Ok(url)
}

/// Full pre-connect validation used by the fetcher and exposed for callers
/// that want to vet a URL without fetching: scheme/host syntax plus a DNS
/// resolution where EVERY resolved address must be public.
pub async fn validate_fetch_url(raw: &str, allow_private: bool) -> Result<Url, AppError> {
    let url = parse_fetch_url(raw)?;
    resolve_validated(&url, allow_private).await?;
    Ok(url)
}

/// Resolve the URL host and reject private/local addresses. Returns the
/// resolved socket addresses for connection pinning.
async fn resolve_validated(url: &Url, allow_private: bool) -> Result<Vec<SocketAddr>, AppError> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::BadRequest("URL has no host".into()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| AppError::BadGateway(format!("DNS resolution failed for {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(AppError::BadGateway(format!("DNS resolution returned no addresses for {host}")));
    }
    if !allow_private && let Some(bad) = addrs.iter().find(|a| forbidden_ip(&a.ip())) {
        return Err(AppError::BadRequest(format!(
            "URL host {host} resolves to a private or local address ({}); fetching it is blocked",
            bad.ip()
        )));
    }
    Ok(addrs)
}

/// SSRF address policy: anything not unambiguously public is forbidden.
fn forbidden_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || octets[0] == 0
                // CGNAT 100.64.0.0/10
                || (octets[0] == 100 && (64..128).contains(&octets[1]))
                // IETF protocol assignments 192.0.0.0/24
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        }
        IpAddr::V6(v6) => {
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique-local fc00::/7
                || (seg0 & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (seg0 & 0xffc0) == 0xfe80
                // v4-mapped/compatible addresses inherit the v4 verdict.
                || v6.to_ipv4_mapped().is_some_and(|v4| forbidden_ip(&IpAddr::V4(v4)))
        }
    }
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

/// Assemble a snapshot document: YAML frontmatter (`source_url`,
/// `fetched_at`, optional `title`) followed by the markdown body.
pub fn snapshot_markdown(source_url: &str, fetched_at: &str, title: Option<&str>, body: &str) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("source_url: {source_url}\n"));
    out.push_str(&format!("fetched_at: {fetched_at}\n"));
    if let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) {
        // Collapse runs of whitespace (incl. newlines/tabs): a multi-line
        // title would break out of its YAML frontmatter line.
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str(&format!("title: \"{}\"\n", title.replace('"', "'")));
    }
    out.push_str("---\n\n");
    out.push_str(body.trim_end());
    out.push('\n');
    out
}

/// Extract the `source_url` value from a snapshot's YAML frontmatter (the
/// shape written by [`snapshot_markdown`]). Returns `None` for documents
/// without a leading frontmatter block, or whose frontmatter has no
/// `source_url` line — i.e. user-authored files that merely live in
/// `snapshots/`. Only the frontmatter block is consulted; a `source_url:`
/// line in the body never matches.
pub fn snapshot_source_url(content: &str) -> Option<&str> {
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

    fn test_fetcher() -> UrlFetcher {
        UrlFetcher::new().allow_private_for_tests()
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

    /// Per-hop redirect validation, exercised at function level: `fetch_page`
    /// follows a redirect by joining the Location value onto the current URL
    /// (`Url::join`), re-checking the scheme (`check_scheme`) and re-resolving
    /// with the private-address guard (`resolve_validated`). An end-to-end
    /// wiremock test CANNOT cover the rejection: the mock server itself binds
    /// to loopback, so reaching hop 1 requires `allow_private` — which would
    /// also admit the private hop 2. This test drives the exact same functions
    /// on a redirect Location target instead.
    #[tokio::test]
    async fn redirect_hop_to_private_target_is_rejected() {
        let origin = Url::parse("https://public.example.com/start").unwrap();
        // Absolute Location to the cloud metadata endpoint (classic SSRF pivot).
        let next = origin.join("http://169.254.169.254/latest/meta-data/").unwrap();
        let next = check_scheme(next).unwrap();
        let err = resolve_validated(&next, false).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(err.to_string().contains("private or local"), "{err}");

        // A redirect downgrading to a non-http scheme dies at check_scheme.
        let bad = origin.join("ftp://internal/").unwrap();
        assert!(check_scheme(bad).is_err());
    }

    #[test]
    fn forbidden_ip_policy() {
        let bad = ["127.0.0.1", "10.1.2.3", "172.31.0.1", "192.168.0.1", "169.254.0.1", "0.0.0.0", "100.100.0.1", "192.0.0.5", "224.0.0.1"];
        for ip in bad {
            assert!(forbidden_ip(&ip.parse().unwrap()), "{ip}");
        }
        let good = ["1.1.1.1", "8.8.8.8", "93.184.216.34", "100.128.0.1", "172.32.0.1"];
        for ip in good {
            assert!(!forbidden_ip(&ip.parse().unwrap()), "{ip}");
        }
        assert!(forbidden_ip(&"::1".parse().unwrap()));
        assert!(forbidden_ip(&"fe80::1".parse().unwrap()));
        assert!(forbidden_ip(&"fd12:3456::1".parse().unwrap()));
        assert!(forbidden_ip(&"::ffff:192.168.0.1".parse().unwrap()));
        assert!(!forbidden_ip(&"2606:4700:4700::1111".parse().unwrap()));
    }

    // ── slug / frontmatter / conversion ──────────────────────────────

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
    fn frontmatter_shape() {
        let md = snapshot_markdown(
            "https://example.com/docs",
            "2026-06-12T12:00:00Z",
            Some("My \"Docs\""),
            "# Title\n\nBody",
        );
        assert!(md.starts_with("---\nsource_url: https://example.com/docs\nfetched_at: 2026-06-12T12:00:00Z\n"), "got: {md}");
        assert!(md.contains("title: \"My 'Docs'\""), "got: {md}");
        assert!(md.contains("---\n\n# Title\n\nBody\n"), "got: {md}");
        // No title line when absent.
        let md = snapshot_markdown("https://e.com", "2026-01-01T00:00:00Z", None, "b");
        assert!(!md.contains("title:"), "got: {md}");
        // Newlines/tabs in a title collapse to single spaces — a multi-line
        // title must not break out of its frontmatter line.
        let md = snapshot_markdown("https://e.com", "2026-01-01T00:00:00Z", Some("Line one\nLine\ttwo"), "b");
        assert!(md.contains("title: \"Line one Line two\"\n"), "got: {md}");
    }

    #[test]
    fn snapshot_source_url_reads_only_frontmatter() {
        // Round-trip with the writer.
        let md = snapshot_markdown("https://e.com/docs", "2026-01-01T00:00:00Z", Some("T"), "body");
        assert_eq!(snapshot_source_url(&md), Some("https://e.com/docs"));
        // User-authored files: no frontmatter at all, or frontmatter without
        // the field — a body-level `source_url:` line never counts.
        assert_eq!(snapshot_source_url("# notes\nsource_url: https://nope"), None);
        assert_eq!(snapshot_source_url("---\ntitle: x\n---\n\nsource_url: https://nope"), None);
        assert_eq!(snapshot_source_url("---\nsource_url:\n---\n"), None, "empty value is no value");
        assert_eq!(snapshot_source_url(""), None);
        // CRLF frontmatter still parses.
        assert_eq!(snapshot_source_url("---\r\nsource_url: https://e.com/x\r\n---\r\nbody"), Some("https://e.com/x"));
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
        let err = UrlFetcher::new().fetch_page(&format!("{}/doc", server.uri())).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(err.to_string().contains("private or local"), "{err}");
    }
}
