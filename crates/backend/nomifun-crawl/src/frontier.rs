//! URL normalization, fingerprinting, and scope evaluation.
//!
//! Normalization decides the dedup identity, so it runs before the fingerprint
//! and before any scope check. Two URLs that differ only in case, default port,
//! query order, fragment, or a known tracking parameter are the same page.

use regex::Regex;
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::CrawlError;
use crate::model::{CrawlScope, DiscoveredUrl};

/// Query parameters that never change what a page returns. Dropping them keeps
/// one shared link from minting thousands of frontier rows.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "gclid",
    "fbclid",
    "msclkid",
    "mc_cid",
    "mc_eid",
    "ref",
    "ref_src",
    "spm",
    "from",
    "_ga",
    "yclid",
    "igshid",
];

/// Schemes a crawler may enqueue. `mailto:`, `javascript:`, `data:` and friends
/// are dropped silently during link extraction.
fn is_crawlable_scheme(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Canonical form used for dedup. Fails for non-http(s) or host-less URLs.
pub fn normalize(raw: &str) -> Result<Url, CrawlError> {
    let mut url =
        Url::parse(raw.trim()).map_err(|e| CrawlError::UrlRejected(format!("invalid URL {raw}: {e}")))?;
    if !is_crawlable_scheme(&url) {
        return Err(CrawlError::UrlRejected(format!(
            "scheme {} is not crawlable",
            url.scheme()
        )));
    }
    if url.host_str().is_none() {
        return Err(CrawlError::UrlRejected(format!("URL {raw} has no host")));
    }

    url.set_fragment(None);
    // `Url` already lowercases the host and drops the default port, but an
    // explicit non-default port must survive.
    if url.username() != "" || url.password().is_some() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }

    let filtered: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if filtered.is_empty() {
        url.set_query(None);
    } else {
        let mut sorted = filtered;
        sorted.sort();
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        for (k, v) in &sorted {
            pairs.append_pair(k, v);
        }
        drop(pairs);
    }

    // A bare host must still have a path, so `example.com` and `example.com/`
    // do not become two frontier rows.
    if url.path().is_empty() {
        url.set_path("/");
    }
    Ok(url)
}

/// sha256 of the normalized URL. Matches the 64-char lowercase-hex CHECK on
/// `crawl_tasks.url_fingerprint`.
pub fn fingerprint(normalized: &Url) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_str().as_bytes());
    hex::encode(hasher.finalize())
}

/// Registrable domain (`docs.rs` and `blog.docs.rs` share `docs.rs`). Falls
/// back to the full host for anything the public suffix list cannot split.
pub fn registrable_domain(host: &str) -> String {
    match psl::domain_str(host) {
        Some(domain) => domain.to_ascii_lowercase(),
        None => host.to_ascii_lowercase(),
    }
}

/// Compiled scope. Building it up front turns every per-URL check into a
/// regex match instead of a recompile.
#[derive(Debug)]
pub struct ScopeMatcher {
    same_site: bool,
    seed_domains: Vec<String>,
    path_prefixes: Vec<String>,
    allow: Vec<Regex>,
    deny: Vec<Regex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeVerdict {
    InScope,
    Rejected(&'static str),
}

impl ScopeMatcher {
    pub fn build(scope: &CrawlScope, seeds: &[String]) -> Result<Self, CrawlError> {
        let mut seed_domains: Vec<String> = seeds
            .iter()
            .filter_map(|seed| normalize(seed).ok())
            .filter_map(|url| url.host_str().map(registrable_domain))
            .collect();
        seed_domains.sort();
        seed_domains.dedup();

        let compile = |patterns: &[String], label: &str| -> Result<Vec<Regex>, CrawlError> {
            patterns
                .iter()
                .map(|p| {
                    Regex::new(p).map_err(|e| {
                        CrawlError::UrlRejected(format!("invalid {label} pattern {p:?}: {e}"))
                    })
                })
                .collect()
        };

        Ok(Self {
            same_site: scope.same_site,
            seed_domains,
            path_prefixes: scope.path_prefixes.clone(),
            allow: compile(&scope.allow, "allow")?,
            deny: compile(&scope.deny, "deny")?,
        })
    }

    pub fn evaluate(&self, url: &Url) -> ScopeVerdict {
        let as_str = url.as_str();

        if self.same_site {
            let host = url.host_str().unwrap_or_default();
            let domain = registrable_domain(host);
            // No seed domains means nothing can ever be in scope; that is a
            // misconfiguration, not an invitation to crawl the whole web.
            if !self.seed_domains.iter().any(|d| d == &domain) {
                return ScopeVerdict::Rejected("off-site");
            }
        }

        if !self.path_prefixes.is_empty()
            && !self.path_prefixes.iter().any(|p| url.path().starts_with(p.as_str()))
        {
            return ScopeVerdict::Rejected("path-prefix");
        }

        if !self.allow.is_empty() && !self.allow.iter().any(|re| re.is_match(as_str)) {
            return ScopeVerdict::Rejected("allow-miss");
        }

        if self.deny.iter().any(|re| re.is_match(as_str)) {
            return ScopeVerdict::Rejected("deny-match");
        }

        ScopeVerdict::InScope
    }
}

/// Normalize, scope-check, and dedup one page's outbound links.
pub fn plan_discoveries(
    links: &[String],
    matcher: &ScopeMatcher,
    child_depth: u32,
    max_depth: u32,
) -> Vec<DiscoveredUrl> {
    if child_depth > max_depth {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for link in links {
        let Ok(url) = normalize(link) else { continue };
        if matcher.evaluate(&url) != ScopeVerdict::InScope {
            continue;
        }
        let fp = fingerprint(&url);
        if !seen.insert(fp.clone()) {
            continue;
        }
        let Some(host) = url.host_str() else { continue };
        out.push(DiscoveredUrl {
            url: url.to_string(),
            fingerprint: fp,
            host: host.to_ascii_lowercase(),
            depth: child_depth,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(raw: &str) -> String {
        normalize(raw).unwrap().to_string()
    }

    #[test]
    fn drops_fragment_and_default_port() {
        assert_eq!(norm("https://Example.COM:443/a#frag"), "https://example.com/a");
        assert_eq!(norm("http://example.com:80/a"), "http://example.com/a");
    }

    #[test]
    fn keeps_explicit_non_default_port() {
        assert_eq!(norm("https://example.com:8443/a"), "https://example.com:8443/a");
    }

    #[test]
    fn sorts_query_and_strips_tracking_params() {
        assert_eq!(
            norm("https://example.com/p?b=2&utm_source=x&a=1&gclid=y"),
            "https://example.com/p?a=1&b=2"
        );
    }

    #[test]
    fn drops_query_entirely_when_only_tracking_params() {
        assert_eq!(norm("https://example.com/p?utm_source=x"), "https://example.com/p");
    }

    #[test]
    fn bare_host_gets_root_path() {
        assert_eq!(norm("https://example.com"), "https://example.com/");
    }

    #[test]
    fn strips_credentials() {
        assert_eq!(norm("https://user:pw@example.com/a"), "https://example.com/a");
    }

    #[test]
    fn rejects_non_http_schemes() {
        for raw in ["mailto:a@b.com", "javascript:alert(1)", "data:text/html,x", "ftp://example.com/f"] {
            assert!(normalize(raw).is_err(), "{raw} should be rejected");
        }
    }

    #[test]
    fn equivalent_urls_share_a_fingerprint() {
        let a = fingerprint(&normalize("https://Example.com/p?b=2&a=1#x").unwrap());
        let b = fingerprint(&normalize("https://example.com:443/p?a=1&b=2").unwrap());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn subdomains_share_a_registrable_domain() {
        assert_eq!(registrable_domain("blog.example.com"), "example.com");
        assert_eq!(registrable_domain("example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("a.b.example.co.uk"), "example.co.uk");
    }

    fn matcher(scope: CrawlScope) -> ScopeMatcher {
        ScopeMatcher::build(&scope, &["https://example.com/docs/".to_string()]).unwrap()
    }

    #[test]
    fn same_site_admits_subdomains_and_rejects_others() {
        let m = matcher(CrawlScope { same_site: true, ..Default::default() });
        assert_eq!(m.evaluate(&normalize("https://blog.example.com/x").unwrap()), ScopeVerdict::InScope);
        assert_eq!(
            m.evaluate(&normalize("https://evil.com/x").unwrap()),
            ScopeVerdict::Rejected("off-site")
        );
    }

    #[test]
    fn path_prefix_narrows_within_the_site() {
        let m = matcher(CrawlScope {
            same_site: true,
            path_prefixes: vec!["/docs/".into()],
            ..Default::default()
        });
        assert_eq!(m.evaluate(&normalize("https://example.com/docs/a").unwrap()), ScopeVerdict::InScope);
        assert_eq!(
            m.evaluate(&normalize("https://example.com/blog/a").unwrap()),
            ScopeVerdict::Rejected("path-prefix")
        );
    }

    #[test]
    fn deny_beats_allow() {
        let m = matcher(CrawlScope {
            same_site: true,
            allow: vec![r"/docs/".into()],
            deny: vec![r"\.pdf$".into()],
            ..Default::default()
        });
        assert_eq!(m.evaluate(&normalize("https://example.com/docs/a").unwrap()), ScopeVerdict::InScope);
        assert_eq!(
            m.evaluate(&normalize("https://example.com/docs/a.pdf").unwrap()),
            ScopeVerdict::Rejected("deny-match")
        );
        assert_eq!(
            m.evaluate(&normalize("https://example.com/other").unwrap()),
            ScopeVerdict::Rejected("allow-miss")
        );
    }

    #[test]
    fn same_site_with_no_resolvable_seed_admits_nothing() {
        let m = ScopeMatcher::build(&CrawlScope { same_site: true, ..Default::default() }, &[]).unwrap();
        assert_eq!(
            m.evaluate(&normalize("https://example.com/x").unwrap()),
            ScopeVerdict::Rejected("off-site")
        );
    }

    #[test]
    fn invalid_regex_is_a_build_error_not_a_silent_pass() {
        let scope = CrawlScope { deny: vec!["(unclosed".into()], ..Default::default() };
        assert!(ScopeMatcher::build(&scope, &[]).is_err());
    }

    #[test]
    fn discoveries_are_deduped_and_depth_capped() {
        let m = matcher(CrawlScope { same_site: true, ..Default::default() });
        let links = vec![
            "https://example.com/a".to_string(),
            "https://example.com/a#dup".to_string(),
            "https://evil.com/a".to_string(),
            "mailto:x@example.com".to_string(),
            "https://example.com/b".to_string(),
        ];
        let found = plan_discoveries(&links, &m, 1, 3);
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|d| d.depth == 1));

        assert!(plan_discoveries(&links, &m, 4, 3).is_empty());
    }
}
