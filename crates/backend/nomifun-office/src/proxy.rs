use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HOST, HeaderMap, HeaderName, HeaderValue};

use crate::types::DocType;
use crate::watch_manager::OfficecliWatchManager;

const PROXY_TIMEOUT: Duration = Duration::from_secs(30);

const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "cookie",
    "set-cookie",
    "authorization",
    "x-frame-options",
];

const NAVIGATION_GUARD_TEMPLATE: &str = r#"<script>
(function(b){
function rw(u){if(!u)return u;var s=String(u);var m=/^https?:\/\/(?:localhost|127\.0\.0\.1)(:\d+)?(\/.*)?$/.exec(s);if(m){var p=m[2]||'/';if(!p.startsWith(b))return b+(p==='/'?'/':p);}if(s==='/'||(s[0]==='/'&&s[1]!=='/'&&!s.startsWith(b)))return b+(s==='/'?'/':s);return s;}
var _a=location.assign.bind(location),_r=location.replace.bind(location);
location.assign=function(u){_a(rw(u));};location.replace=function(u){_r(rw(u));};
var _ps=history.pushState.bind(history),_rs=history.replaceState.bind(history);
history.pushState=function(s,t,u){_ps(s,t,u?rw(u):u);};history.replaceState=function(s,t,u){_rs(s,t,u?rw(u):u);};
try{Object.defineProperty(location,'href',{set:function(v){_a(rw(v));},configurable:true});}catch(e){}
document.addEventListener('click',function(e){var t=e.target;while(t&&t.tagName!=='A')t=t.parentElement;if(t&&t.tagName==='A'){var h=t.getAttribute('href');if(h&&(h[0]==='/'&&h[1]!=='/'&&!h.startsWith(b))){e.preventDefault();_a(b+h);}}},true);
})('PROXY_BASE_PLACEHOLDER');
</script>"#;

pub struct ProxyService {
    watch_manager: Arc<OfficecliWatchManager>,
    client: reqwest::Client,
}

#[derive(Debug)]
pub struct ProxyResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ProxyService {
    pub fn new(watch_manager: Arc<OfficecliWatchManager>) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(PROXY_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build proxy HTTP client");

        Self { watch_manager, client }
    }

    pub async fn forward(
        &self,
        capability: &str,
        path: &str,
        doc_type: DocType,
        request_headers: &[(String, String)],
    ) -> Result<ProxyResponse, ProxyError> {
        let target = self
            .watch_manager
            .resolve_capability(capability)
            .ok_or(ProxyError::InvalidCapability)?;
        if target.doc_type != doc_type {
            return Err(ProxyError::InvalidCapability);
        }
        let proxy_base = format!("/api/{}/{}", doc_type.proxy_prefix(), capability);
        self.forward_inner(target.port, path, &proxy_base, request_headers)
            .await
    }

    pub async fn forward_watch(
        &self,
        capability: &str,
        path: &str,
        request_headers: &[(String, String)],
    ) -> Result<ProxyResponse, ProxyError> {
        let target = self
            .watch_manager
            .resolve_capability(capability)
            .ok_or(ProxyError::InvalidCapability)?;
        if !matches!(target.doc_type, DocType::Word | DocType::Excel) {
            return Err(ProxyError::InvalidCapability);
        }
        let proxy_base = format!("/api/office-watch-proxy/{capability}");
        self.forward_inner(target.port, path, &proxy_base, request_headers)
            .await
    }

    async fn forward_inner(
        &self,
        port: u16,
        path: &str,
        proxy_base: &str,
        request_headers: &[(String, String)],
    ) -> Result<ProxyResponse, ProxyError> {
        let target_url = build_target_url(port, path);

        let mut req_headers = HeaderMap::new();
        req_headers.insert(
            HOST,
            HeaderValue::from_str(&format!("127.0.0.1:{port}"))
                .unwrap_or_else(|_| HeaderValue::from_static("127.0.0.1")),
        );

        for (key, value) in request_headers {
            let lower = key.to_lowercase();
            if is_hop_by_hop(&lower) {
                continue;
            }
            if lower == "host" {
                continue;
            }
            if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(lower.as_bytes()), HeaderValue::from_str(value)) {
                req_headers.insert(name, val);
            }
        }

        let resp = self
            .client
            .get(&target_url)
            .headers(req_headers)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProxyError::Timeout
                } else if e.is_connect() {
                    ProxyError::ConnectionFailed(e.to_string())
                } else {
                    ProxyError::RequestFailed(e.to_string())
                }
            })?;

        let status = resp.status().as_u16();
        let resp_headers = resp.headers().clone();
        let body = resp
            .bytes()
            .await
            .map_err(|e| ProxyError::RequestFailed(format!("failed to read response body: {e}")))?;

        let mut out_headers = Vec::new();
        let is_html = is_html_content_type(&resp_headers);
        let mut body_bytes = body.to_vec();

        for (name, value) in resp_headers.iter() {
            let name_str = name.as_str().to_lowercase();

            if is_hop_by_hop(&name_str) {
                continue;
            }

            if name_str == "location"
                && let Ok(loc) = value.to_str()
            {
                let rewritten = rewrite_location(loc, port, proxy_base);
                out_headers.push(("location".to_owned(), rewritten));
                continue;
            }

            if is_html && name_str == "content-length" {
                continue;
            }

            if let Ok(val_str) = value.to_str() {
                out_headers.push((name_str, val_str.to_owned()));
            }
        }

        if is_html {
            body_bytes = inject_navigation_guard(&body_bytes, proxy_base);
        }

        Ok(ProxyResponse {
            status,
            headers: out_headers,
            body: body_bytes,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("invalid or expired preview capability")]
    InvalidCapability,

    #[error("proxy request timed out")]
    Timeout,

    #[error("connection to preview server failed: {0}")]
    ConnectionFailed(String),

    #[error("proxy request failed: {0}")]
    RequestFailed(String),
}

impl From<ProxyError> for nomifun_common::AppError {
    fn from(err: ProxyError) -> Self {
        match err {
            ProxyError::InvalidCapability => nomifun_common::AppError::Forbidden(err.to_string()),
            ProxyError::Timeout => nomifun_common::AppError::Timeout(err.to_string()),
            ProxyError::ConnectionFailed(msg) => nomifun_common::AppError::BadGateway(msg),
            ProxyError::RequestFailed(msg) => nomifun_common::AppError::BadGateway(msg),
        }
    }
}

fn build_target_url(port: u16, path: &str) -> String {
    let normalized = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    format!("http://127.0.0.1:{port}{normalized}")
}

fn is_hop_by_hop(header: &str) -> bool {
    HOP_BY_HOP_HEADERS.contains(&header)
}

fn is_html_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/html"))
}

fn rewrite_location(location: &str, port: u16, proxy_base: &str) -> String {
    let pattern = format!("http://localhost:{port}");
    let pattern_ip = format!("http://127.0.0.1:{port}");

    let rewritten = if location.starts_with(&pattern) {
        format!("{proxy_base}{}", &location[pattern.len()..])
    } else if location.starts_with(&pattern_ip) {
        format!("{proxy_base}{}", &location[pattern_ip.len()..])
    } else {
        location.to_owned()
    };

    if rewritten == "/"
        || (rewritten.starts_with('/') && !rewritten.starts_with("//") && !rewritten.starts_with(proxy_base))
    {
        if rewritten == "/" {
            format!("{proxy_base}/")
        } else {
            format!("{proxy_base}{rewritten}")
        }
    } else {
        rewritten
    }
}

fn inject_navigation_guard(body: &[u8], proxy_base: &str) -> Vec<u8> {
    let html = String::from_utf8_lossy(body);
    let guard_script = NAVIGATION_GUARD_TEMPLATE.replace("PROXY_BASE_PLACEHOLDER", proxy_base);

    let result = if let Some(pos) = find_head_tag_end(&html) {
        format!("{}{}{}", &html[..pos], guard_script, &html[pos..])
    } else {
        format!("{guard_script}{html}")
    };

    result.into_bytes()
}

fn find_head_tag_end(html: &str) -> Option<usize> {
    let lower = html.to_lowercase();
    let head_start = lower.find("<head")?;
    let tag_end = lower[head_start..].find('>')?;
    Some(head_start + tag_end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_target_url_with_leading_slash() {
        assert_eq!(
            build_target_url(8080, "/index.html"),
            "http://127.0.0.1:8080/index.html"
        );
    }

    #[test]
    fn build_target_url_without_leading_slash() {
        assert_eq!(build_target_url(8080, "index.html"), "http://127.0.0.1:8080/index.html");
    }

    #[test]
    fn build_target_url_root() {
        assert_eq!(build_target_url(3000, "/"), "http://127.0.0.1:3000/");
    }

    #[test]
    fn build_target_url_empty_path() {
        assert_eq!(build_target_url(3000, ""), "http://127.0.0.1:3000/");
    }

    #[test]
    fn is_hop_by_hop_recognizes_all_headers() {
        for h in HOP_BY_HOP_HEADERS {
            assert!(is_hop_by_hop(h), "expected {h} to be hop-by-hop");
        }
    }

    #[test]
    fn is_hop_by_hop_rejects_normal_headers() {
        assert!(!is_hop_by_hop("content-type"));
        assert!(!is_hop_by_hop("accept"));
        assert!(!is_hop_by_hop("x-custom-header"));
    }

    #[test]
    fn is_html_content_type_detects_html() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
        assert!(is_html_content_type(&headers));
    }

    #[test]
    fn is_html_content_type_rejects_json() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        assert!(!is_html_content_type(&headers));
    }

    #[test]
    fn is_html_content_type_empty_headers() {
        let headers = HeaderMap::new();
        assert!(!is_html_content_type(&headers));
    }

    #[test]
    fn rewrite_location_localhost_absolute() {
        let result = rewrite_location("http://localhost:8080/new/path", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "/api/ppt-proxy/8080/new/path");
    }

    #[test]
    fn rewrite_location_ip_absolute() {
        let result = rewrite_location("http://127.0.0.1:8080/new/path", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "/api/ppt-proxy/8080/new/path");
    }

    #[test]
    fn rewrite_location_root_relative() {
        let result = rewrite_location("/foo/bar", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "/api/ppt-proxy/8080/foo/bar");
    }

    #[test]
    fn rewrite_location_root_path() {
        let result = rewrite_location("/", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "/api/ppt-proxy/8080/");
    }

    #[test]
    fn rewrite_location_already_proxied() {
        let result = rewrite_location("/api/ppt-proxy/8080/existing", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "/api/ppt-proxy/8080/existing");
    }

    #[test]
    fn rewrite_location_external_url_unchanged() {
        let result = rewrite_location("https://example.com/path", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "https://example.com/path");
    }

    #[test]
    fn rewrite_location_different_port_unchanged() {
        let result = rewrite_location("http://localhost:9999/path", 8080, "/api/ppt-proxy/8080");
        assert_eq!(result, "http://localhost:9999/path");
    }

    #[test]
    fn rewrite_location_localhost_root() {
        let result = rewrite_location("http://localhost:3000", 3000, "/api/office-watch-proxy/3000");
        assert_eq!(result, "/api/office-watch-proxy/3000");
    }

    #[test]
    fn inject_guard_with_head_tag() {
        let html = b"<!DOCTYPE html><html><head><title>Test</title></head><body></body></html>";
        let result = inject_navigation_guard(html, "/api/ppt-proxy/8080");
        let result_str = String::from_utf8(result).unwrap();

        assert!(result_str.contains("<head><script>"));
        assert!(result_str.contains("'/api/ppt-proxy/8080'"));
        assert!(result_str.contains("<title>Test</title>"));
    }

    #[test]
    fn inject_guard_with_head_attributes() {
        let html = b"<html><head lang=\"en\"><title>Test</title></head></html>";
        let result = inject_navigation_guard(html, "/api/ppt-proxy/8080");
        let result_str = String::from_utf8(result).unwrap();

        assert!(result_str.contains("lang=\"en\"><script>"));
    }

    #[test]
    fn inject_guard_no_head_tag() {
        let html = b"<html><body>content</body></html>";
        let result = inject_navigation_guard(html, "/api/ppt-proxy/8080");
        let result_str = String::from_utf8(result).unwrap();

        assert!(result_str.starts_with("<script>"));
        assert!(result_str.contains("content</body>"));
    }

    #[test]
    fn inject_guard_uppercase_head() {
        let html = b"<HTML><HEAD><TITLE>Test</TITLE></HEAD></HTML>";
        let result = inject_navigation_guard(html, "/api/ppt-proxy/8080");
        let result_str = String::from_utf8(result).unwrap();

        assert!(result_str.contains("<HEAD><script>"));
    }

    #[test]
    fn find_head_tag_end_normal() {
        assert_eq!(find_head_tag_end("<html><head><title>"), Some(12));
    }

    #[test]
    fn find_head_tag_end_with_attrs() {
        let html = "<html><head lang=\"en\">";
        assert_eq!(find_head_tag_end(html), Some(html.len()));
    }

    #[test]
    fn find_head_tag_end_uppercase() {
        assert_eq!(find_head_tag_end("<html><HEAD>"), Some(12));
    }

    #[test]
    fn find_head_tag_end_missing() {
        assert_eq!(find_head_tag_end("<html><body>"), None);
    }

    #[test]
    fn proxy_error_invalid_capability_to_forbidden() {
        let err: nomifun_common::AppError = ProxyError::InvalidCapability.into();
        assert!(matches!(err, nomifun_common::AppError::Forbidden(_)));
    }

    #[test]
    fn proxy_error_timeout_to_timeout() {
        let err: nomifun_common::AppError = ProxyError::Timeout.into();
        assert!(matches!(err, nomifun_common::AppError::Timeout(_)));
    }

    #[test]
    fn proxy_error_connection_failed_to_bad_gateway() {
        let err: nomifun_common::AppError = ProxyError::ConnectionFailed("refused".into()).into();
        assert!(matches!(err, nomifun_common::AppError::BadGateway(_)));
    }

    #[test]
    fn proxy_error_request_failed_to_bad_gateway() {
        let err: nomifun_common::AppError = ProxyError::RequestFailed("network error".into()).into();
        assert!(matches!(err, nomifun_common::AppError::BadGateway(_)));
    }

    #[test]
    fn proxy_error_display() {
        assert_eq!(
            ProxyError::InvalidCapability.to_string(),
            "invalid or expired preview capability"
        );
        assert_eq!(ProxyError::Timeout.to_string(), "proxy request timed out");
        assert_eq!(
            ProxyError::ConnectionFailed("refused".into()).to_string(),
            "connection to preview server failed: refused"
        );
    }

    #[test]
    fn navigation_guard_contains_all_intercepts() {
        let guard = NAVIGATION_GUARD_TEMPLATE;
        assert!(guard.contains("location.assign"));
        assert!(guard.contains("location.replace"));
        assert!(guard.contains("history.pushState"));
        assert!(guard.contains("history.replaceState"));
        assert!(guard.contains("location,'href'"));
        assert!(guard.contains("addEventListener('click'"));
    }
}
