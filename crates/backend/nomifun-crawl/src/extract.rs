//! HTML → links + readable markdown.

use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use url::Url;

/// Everything one page contributes: what to ingest and where to go next.
#[derive(Debug, Clone, Default)]
pub struct Extraction {
    pub title: Option<String>,
    pub markdown: String,
    /// sha256 over the extracted markdown, not the raw HTML: ad rotation and
    /// CSRF tokens change the HTML on every fetch, which would defeat the
    /// unchanged-page check.
    pub content_hash: String,
    /// Absolute http(s) links, deduped, in document order.
    pub links: Vec<String>,
    /// `<meta name="robots" content="noindex">` — do not ingest this page.
    pub noindex: bool,
    /// `<meta name="robots" content="nofollow">` — do not follow its links.
    pub nofollow: bool,
}

/// Parse a page. `base_url` is the post-redirect URL, used to resolve relative
/// links (overridden by `<base href>` when present).
pub fn extract(html: &str, base_url: &Url) -> Extraction {
    let doc = Html::parse_document(html);
    let (noindex, nofollow) = meta_robots(&doc);
    let base = base_href(&doc, base_url);
    let links = if nofollow { Vec::new() } else { collect_links(&doc, &base) };
    let (title, markdown) = readable(html, base_url);
    let content_hash = hash(&markdown);

    Extraction { title, markdown, content_hash, links, noindex, nofollow }
}

pub fn hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn base_href(doc: &Html, fallback: &Url) -> Url {
    let selector = Selector::parse("base[href]").expect("static selector");
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("href"))
        .and_then(|href| fallback.join(href).ok())
        .unwrap_or_else(|| fallback.clone())
}

fn meta_robots(doc: &Html) -> (bool, bool) {
    let selector = Selector::parse("meta[name][content]").expect("static selector");
    let mut noindex = false;
    let mut nofollow = false;
    for el in doc.select(&selector) {
        let name = el.value().attr("name").unwrap_or_default();
        // `robots` targets every crawler; the agent-specific form is ours.
        if !name.eq_ignore_ascii_case("robots") && !name.eq_ignore_ascii_case("nomifun") {
            continue;
        }
        let content = el.value().attr("content").unwrap_or_default().to_ascii_lowercase();
        for directive in content.split(',') {
            match directive.trim() {
                "noindex" => noindex = true,
                "nofollow" => nofollow = true,
                "none" => {
                    noindex = true;
                    nofollow = true;
                }
                _ => {}
            }
        }
    }
    (noindex, nofollow)
}

fn collect_links(doc: &Html, base: &Url) -> Vec<String> {
    let selector = Selector::parse("a[href]").expect("static selector");
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for el in doc.select(&selector) {
        // Per-link rel=nofollow is a publisher instruction, same as the meta form.
        if el
            .value()
            .attr("rel")
            .is_some_and(|rel| rel.split_whitespace().any(|t| t.eq_ignore_ascii_case("nofollow")))
        {
            continue;
        }
        let Some(href) = el.value().attr("href") else { continue };
        let Ok(joined) = base.join(href.trim()) else { continue };
        if !matches!(joined.scheme(), "http" | "https") {
            continue;
        }
        let as_str = joined.to_string();
        if seen.insert(as_str.clone()) {
            out.push(as_str);
        }
    }
    out
}

/// Readability + HTML→Markdown. Falls back to converting the whole document
/// when the extractor cannot find an article, which is the right answer for
/// index and listing pages.
fn readable(html: &str, base_url: &Url) -> (Option<String>, String) {
    let article = dom_smoothie::Readability::new(html, Some(base_url.as_str()), None)
        .ok()
        .and_then(|mut r| r.parse().ok());

    match article {
        Some(article) => {
            let title = (!article.title.trim().is_empty()).then(|| article.title.clone());
            let markdown = to_markdown(article.content.as_ref());
            if markdown.trim().is_empty() {
                (title, to_markdown(html))
            } else {
                (title, markdown)
            }
        }
        None => (document_title(html), to_markdown(html)),
    }
}

fn document_title(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let selector = Selector::parse("title").expect("static selector");
    doc.select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
}

fn to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/docs/page").unwrap()
    }

    #[test]
    fn resolves_relative_and_absolute_links() {
        let html = r#"<a href="/a">A</a><a href="b">B</a><a href="https://other.com/c">C</a>"#;
        let out = extract(html, &base());
        assert_eq!(
            out.links,
            vec![
                "https://example.com/a",
                "https://example.com/docs/b",
                "https://other.com/c",
            ]
        );
    }

    #[test]
    fn base_href_overrides_the_document_url() {
        let html = r#"<head><base href="https://cdn.example.com/root/"></head><a href="x">X</a>"#;
        let out = extract(html, &base());
        assert_eq!(out.links, vec!["https://cdn.example.com/root/x"]);
    }

    #[test]
    fn drops_non_http_schemes() {
        let html = r#"<a href="mailto:a@b.com">M</a><a href="javascript:void(0)">J</a><a href="/ok">O</a>"#;
        let out = extract(html, &base());
        assert_eq!(out.links, vec!["https://example.com/ok"]);
    }

    #[test]
    fn dedups_repeated_links() {
        let html = r#"<a href="/a">1</a><a href="/a">2</a><a href="/a#frag">3</a>"#;
        let out = extract(html, &base());
        // `#frag` is a distinct URL here; the frontier strips fragments later.
        assert_eq!(out.links, vec!["https://example.com/a", "https://example.com/a#frag"]);
    }

    #[test]
    fn honours_per_link_nofollow() {
        let html = r#"<a href="/a" rel="nofollow">A</a><a href="/b">B</a>"#;
        let out = extract(html, &base());
        assert_eq!(out.links, vec!["https://example.com/b"]);
    }

    #[test]
    fn meta_noindex_is_reported_without_dropping_links() {
        let html = r#"<meta name="robots" content="noindex"><a href="/a">A</a>"#;
        let out = extract(html, &base());
        assert!(out.noindex);
        assert!(!out.nofollow);
        assert_eq!(out.links.len(), 1);
    }

    #[test]
    fn meta_nofollow_suppresses_every_link() {
        let html = r#"<meta name="robots" content="nofollow"><a href="/a">A</a>"#;
        let out = extract(html, &base());
        assert!(out.nofollow);
        assert!(out.links.is_empty());
    }

    #[test]
    fn meta_none_sets_both_flags() {
        let html = r#"<meta name="robots" content="none"><a href="/a">A</a>"#;
        let out = extract(html, &base());
        assert!(out.noindex && out.nofollow);
    }

    #[test]
    fn extracts_the_article_and_drops_the_chrome() {
        let html = r#"<html><head><title>Doc</title></head><body>
            <nav><a href="/nav">navigation menu here</a></nav>
            <article><h1>Real Heading</h1>
            <p>This is the actual body copy of the article and it is deliberately
            long enough that the readability scorer prefers it over the navigation
            chrome that surrounds it on the page.</p>
            <p>A second substantial paragraph keeps the candidate score high so the
            extractor settles on this container rather than the body element.</p>
            </article></body></html>"#;
        let out = extract(html, &base());
        assert!(out.markdown.contains("Real Heading"), "markdown: {}", out.markdown);
        assert!(!out.markdown.contains("navigation menu here"), "markdown: {}", out.markdown);
    }

    #[test]
    fn falls_back_to_the_whole_document_when_there_is_no_article() {
        let html = r#"<html><head><title>Index</title></head><body><ul>
            <li><a href="/a">A</a></li><li><a href="/b">B</a></li></ul></body></html>"#;
        let out = extract(html, &base());
        assert!(!out.markdown.trim().is_empty());
        assert_eq!(out.links.len(), 2);
    }

    #[test]
    fn hash_tracks_content_not_markup_noise() {
        let a = extract(
            r#"<article><p>Same words in the body of this page.</p></article><span id="csrf">aaa</span>"#,
            &base(),
        );
        let b = extract(
            r#"<article><p>Same words in the body of this page.</p></article><span id="csrf">bbb</span>"#,
            &base(),
        );
        assert_eq!(a.content_hash, b.content_hash);
    }

    #[test]
    fn different_body_text_changes_the_hash() {
        let a = extract("<article><p>First body text.</p></article>", &base());
        let b = extract("<article><p>Second body text.</p></article>", &base());
        assert_ne!(a.content_hash, b.content_hash);
    }
}
