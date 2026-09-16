//! Fixed, versioned organic-result adapter. No model selectors or engine fallback.
use super::{LocalSearchError, LocalSearchResult};
use base64::Engine;
use serde::Deserialize;
use std::collections::BTreeSet;
use url::{Host, Url};

pub const ADAPTER_ID: &str = "bing-organic-v1";
pub const EXTRACTION: &str = r#"(()=>{
    const captcha=document.querySelector('#b_captcha, #captcha, iframe[src*="captcha"], form[action*="challenge"]');
    const dialog=document.querySelector('[role="dialog"][aria-modal="true"]');
    if (captcha || (dialog && /consent|cookies|隐私|同意/i.test(dialog.textContent))) return {state:'challenge'};
    const root=document.querySelector('#b_results');
    if (!root) return {state:document.readyState==='complete'?'invalid':'waiting'};
    const nodes=Array.from(root.querySelectorAll('li.b_algo')).slice(0,30);
    const results=nodes.map(node=>{
        const link=node.querySelector('h2 a[href]');
        return link ? {title:link.textContent.trim().slice(0,1024),url:link.href,
            snippet:(node.querySelector('.b_caption p')?.textContent||'').trim().slice(0,4096)} : null;
    }).filter(Boolean);
    if (results.length) return {state:'ready',results};
    if (root.querySelector('.b_no')) return {state:'empty',results:[]};
    return {state:document.readyState==='complete'?'invalid':'waiting'};
})()"#;

pub fn origins() -> BTreeSet<String> {
    [
        "https://www.bing.com",
        "https://cn.bing.com",
        "https://r.bing.com",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

pub fn query_url(query: &str, locale: &str) -> Url {
    let mut url = Url::parse("https://www.bing.com/search").expect("fixed search URL");
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("count", "10")
        .append_pair("mkt", locale);
    url
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawResult {
    title: String,
    url: String,
    snippet: String,
}

pub fn normalize(
    rows: Vec<RawResult>,
    limit: usize,
) -> Result<Vec<LocalSearchResult>, LocalSearchError> {
    if rows.len() > 30 {
        return Err(LocalSearchError::InvalidResult);
    }
    let mut urls = BTreeSet::new();
    let mut results = Vec::new();
    for row in rows {
        let Some(url) = canonical_url(&row.url) else {
            continue;
        };
        let title = bounded_text(&row.title, 512);
        if title.is_empty() || !urls.insert(url.clone()) {
            continue;
        }
        results.push(LocalSearchResult {
            citation_id: String::new(),
            rank: results.len() + 1,
            title,
            url,
            snippet: bounded_text(&row.snippet, 2048),
        });
        if results.len() == limit {
            break;
        }
    }
    if results.is_empty() {
        return Err(LocalSearchError::InvalidResult);
    }
    Ok(results)
}

fn bounded_text(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn canonical_url(raw: &str) -> Option<String> {
    if raw.len() > 8192 {
        return None;
    }
    let mut url = Url::parse(raw).ok()?;
    if matches!(url.host_str(), Some("www.bing.com" | "cn.bing.com")) && url.path() == "/ck/a" {
        let encoded = url
            .query_pairs()
            .find(|(key, _)| key == "u")?
            .1
            .into_owned();
        let encoded = encoded.strip_prefix("a1")?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded))
            .ok()?;
        url = Url::parse(std::str::from_utf8(&decoded).ok()?).ok()?;
    }
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    match url.host()? {
        Host::Domain(host)
            if !host.contains('.')
                || host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local") =>
        {
            return None;
        }
        Host::Ipv4(ip) if nomifun_net::egress::forbidden_ip(std::net::IpAddr::V4(ip)) => {
            return None;
        }
        Host::Ipv6(ip) if nomifun_net::egress::forbidden_ip(std::net::IpAddr::V6(ip)) => {
            return None;
        }
        _ => {}
    }
    url.set_fragment(None);
    let query: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && !matches!(key.as_str(), "gclid" | "fbclid" | "msclkid")
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_builder_keeps_query_in_one_parameter() {
        let url = query_url("x&url=http://127.0.0.1/#中文", "zh-CN");
        assert_eq!(url.origin().ascii_serialization(), "https://www.bing.com");
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "q").unwrap().1,
            "x&url=http://127.0.0.1/#中文"
        );
    }
    #[test]
    fn canonicalization_unwraps_tracking_and_deduplicates() {
        let target = "https://example.com/docs?utm_source=bing&lang=zh#fragment";
        let wrapped = format!(
            "https://www.bing.com/ck/a?u=a1{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(target)
        );
        let rows = vec![
            RawResult {
                title: " Doc ".into(),
                url: wrapped,
                snippet: " text\n value ".into(),
            },
            RawResult {
                title: "duplicate".into(),
                url: target.into(),
                snippet: String::new(),
            },
        ];
        let results = normalize(rows, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://example.com/docs?lang=zh");
        assert_eq!(results[0].snippet, "text value");
        for url in [
            "file:///secret",
            "http://127.0.0.1/",
            "http://[::1]/",
            "https://user:secret@example.com/",
            "http://router.local/",
            "https://www.bing.com/ck/a?u=bad",
        ] {
            assert!(canonical_url(url).is_none(), "{url}");
        }
    }
}
