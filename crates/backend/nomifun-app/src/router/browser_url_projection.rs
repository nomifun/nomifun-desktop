//! Safe URL projection for browser data sent to the renderer.
//!
//! Hub snapshots, navigation, and Agent-facing tool results retain their exact
//! URLs. Only renderer-safe Browser inventory metadata should pass through
//! this module; the product no longer exposes a live Viewer surface.

use url::Url;

const REDACTED_URL: &str = "[REDACTED_URL]";

pub(crate) fn project_renderer_url(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw) else {
        // Malformed input can still contain credentials or query secrets, so
        // parsing failure must never fall back to the original string.
        return REDACTED_URL.to_owned();
    };

    // Renderer metadata is only for identifying ordinary web pages. Internal
    // browser, local-file, executable, opaque, and websocket URLs are never
    // useful enough to justify exposing their payloads.
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return REDACTED_URL.to_owned();
    }

    // Browser HTTP discovery endpoints are just as sensitive as a raw
    // `ws://.../devtools/...` URL: they disclose debugging ports and can return
    // attachable websocket endpoints. Debug bridges may be reverse-proxied or
    // exposed on private/remote hosts, so inspect every HTTP(S) URL rather than
    // trusting non-loopback hostnames.
    if looks_like_browser_debug_endpoint(&url) {
        return REDACTED_URL.to_owned();
    }

    if url.set_password(None).is_err() || url.set_username("").is_err() {
        return REDACTED_URL.to_owned();
    }

    // Query strings are intentionally dropped wholesale instead of relying on
    // a denylist. OAuth credentials, cookies, storage snapshots, signed URLs,
    // and nested CDP endpoints routinely use application-specific key names.
    url.set_query(None);
    url.set_fragment(None);

    url.into()
}

fn looks_like_browser_debug_endpoint(url: &Url) -> bool {
    if matches!(url.port(), Some(9222 | 9223 | 9229 | 9333 | 9515)) {
        return true;
    }

    let path = percent_decode_for_inspection(url.path());
    let path = path.to_ascii_lowercase();
    let compact_path = compact_ascii(&path);

    if path == "/devtools"
        || path.starts_with("/devtools/")
        || path.contains("/devtools/")
        || path == "/json"
        || path.starts_with("/json/")
        || path == "/debug"
        || path.starts_with("/debug/")
        || path == "/debugger"
        || path.starts_with("/debugger/")
        || path == "/cdp"
        || path.starts_with("/cdp/")
        || path.contains("/cdp/")
        || compact_path.contains("devtools")
        || compact_path.contains("remotedebugging")
        || compact_path.contains("websocketdebuggerurl")
        || compact_path.contains("debuggeraddress")
        || compact_path.contains("browserwsendpoint")
        || compact_path.contains("cdpendpoint")
        || compact_path.contains("debuggingport")
    {
        return true;
    }

    // A normal page may legitimately carry application-specific query
    // parameters that happen to contain words such as `token`, `endpoint`, or
    // `debug`. The query is removed before projection, so those values cannot
    // leak. Treat query-embedded CDP discovery data as a whole-URL rejection
    // only on local/private hosts, where it is plausibly an attachable debug
    // bridge rather than ordinary page data.
    if !is_local_or_private_host(url.host_str()) {
        return false;
    }

    let Some(query) = url.query() else {
        return false;
    };
    let query = percent_decode_for_inspection(query);
    let query_lower = query.to_ascii_lowercase();
    let compact_query = compact_ascii(&query);

    query_lower.contains("ws://")
        || query_lower.contains("wss://")
        || compact_query.contains("devtools")
        || compact_query.contains("remotedebugging")
        || compact_query.contains("websocketdebuggerurl")
        || compact_query.contains("debuggeraddress")
        || compact_query.contains("browserwsendpoint")
        || compact_query.contains("cdpendpoint")
        || compact_query.contains("debuggingport")
}

fn is_local_or_private_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    let Ok(address) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    match address {
        std::net::IpAddr::V4(address) => {
            address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_unspecified()
        }
        std::net::IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_unspecified()
        }
    }
}

fn percent_decode_for_inspection(value: &str) -> String {
    let mut current = value.as_bytes().to_vec();

    // A small bounded number of passes catches both raw and nested
    // percent-encoding without allowing attacker-controlled expansion.
    for _ in 0..3 {
        let mut decoded = Vec::with_capacity(current.len());
        let mut changed = false;
        let mut index = 0;

        while index < current.len() {
            if current[index] == b'%'
                && index + 2 < current.len()
                && let (Some(high), Some(low)) =
                    (hex_value(current[index + 1]), hex_value(current[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                changed = true;
                continue;
            }

            decoded.push(current[index]);
            index += 1;
        }

        current = decoded;
        if !changed {
            break;
        }
    }

    String::from_utf8_lossy(&current).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn compact_ascii(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric())
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_normal_http_https_host_and_path_only() {
        assert_eq!(
            project_renderer_url("https://example.test/catalog/items"),
            "https://example.test/catalog/items"
        );
        assert_eq!(
            project_renderer_url("http://example.test:8080/docs/page%20one?lang=zh#section"),
            "http://example.test:8080/docs/page%20one"
        );
    }

    #[test]
    fn removes_userinfo_query_and_fragment() {
        let projected = project_renderer_url(
            "https://alice:p%40ssword@example.test/callback?safe=value&api_key=secret#private-fragment",
        );
        assert_eq!(projected, "https://example.test/callback");
        for secret in ["alice", "p%40ssword", "safe=value", "secret", "private-fragment"] {
            assert!(!projected.contains(secret), "projection leaked {secret}");
        }
    }

    #[test]
    fn removes_all_raw_sensitive_query_keys_and_values() {
        let projected = project_renderer_url(
            "https://example.test/page?cdp_endpoint=ws://127.0.0.1:9222/devtools/browser/secret&debugging_port=9222&profile_path=C:%5CUsers%5Calice%5CChrome&api_key=api-secret&client_secret=client-secret&password=password-secret&authorization=Bearer%20auth-secret&jwt=jwt-secret&sig=sig-secret&cookie=session%3Dcookie-secret&storage=storage-secret&ordinary=also-private",
        );
        assert_eq!(projected, "https://example.test/page");
        for secret in [
            "cdp_endpoint",
            "debugging_port",
            "profile_path",
            "api_key",
            "client_secret",
            "password",
            "authorization",
            "jwt",
            "sig",
            "cookie",
            "storage",
            "ordinary",
            "secret",
            "9222",
        ] {
            assert!(!projected.contains(secret), "projection leaked {secret}");
        }
    }

    #[test]
    fn removes_percent_encoded_sensitive_query_keys_and_nested_endpoints() {
        let projected = project_renderer_url(
            "https://example.test/page?%63%64%70_%65%6e%64%70%6f%69%6e%74=ws%253A%252F%252F127.0.0.1%253A9222%252Fdevtools%252Fbrowser%252Fsecret&%64%65%62%75%67%67%69%6e%67_%70%6f%72%74=9222&%70%72%6f%66%69%6c%65_%70%61%74%68=C%253A%255Csecret&%61%70%69_%6b%65%79=secret&%63%6c%69%65%6e%74_%73%65%63%72%65%74=secret&%70%61%73%73%77%6f%72%64=secret&%61%75%74%68%6f%72%69%7a%61%74%69%6f%6e=secret&%6a%77%74=secret&%73%69%67=secret&%63%6f%6f%6b%69%65=secret&%73%74%6f%72%61%67%65=secret",
        );
        assert_eq!(projected, "https://example.test/page");
        assert!(!projected.contains('%'));
        assert!(!projected.contains("secret"));
        assert!(!projected.contains("9222"));
    }

    #[test]
    fn rejects_non_page_schemes() {
        for raw in [
            "file:///C:/Users/alice/Chrome/User%20Data/Profile%201",
            "ws://127.0.0.1:9222/devtools/browser/secret",
            "wss://example.test/devtools/page/secret",
            "devtools://devtools/bundled/inspector.html",
            "chrome://version",
            "chrome-extension://extension-id/page.html",
            "data:text/html,<script>alert(1)</script>",
            "javascript:alert(document.cookie)",
            "blob:https://example.test/secret",
            "about:blank",
            "ftp://example.test/private",
        ] {
            assert_eq!(project_renderer_url(raw), REDACTED_URL, "{raw}");
        }
    }

    #[test]
    fn rejects_loopback_browser_debug_endpoints() {
        for raw in [
            "http://127.0.0.1:9222/",
            "http://127.0.0.2:40123/devtools/browser/secret",
            "http://[::1]:40123/json/version",
            "https://localhost:40123/devtools/page/secret",
            "http://browser.localhost:40123/json/list",
            "http://localhost:40123/%64%65%76%74%6f%6f%6c%73/%62%72%6f%77%73%65%72/secret",
            "http://localhost:40123/%252fdevtools%252fbrowser%252fsecret",
            "http://localhost:40123/internal/devtools",
            "http://localhost:40123/cdp/browser/secret",
            "http://localhost:40123/page?cdp_endpoint=ws%3A%2F%2F127.0.0.1%3A9222%2Fdevtools%2Fbrowser%2Fsecret",
            "http://localhost:40123/page?endpoint=ws%253A%252F%252F127.0.0.1%253A9222",
            "http://localhost:40123/page?%64%65%62%75%67%67%69%6e%67_%70%6f%72%74=9222",
        ] {
            assert_eq!(project_renderer_url(raw), REDACTED_URL, "{raw}");
        }
    }

    #[test]
    fn rejects_remote_and_private_browser_debug_endpoints() {
        for raw in [
            "https://debug.example.test/devtools/browser/secret",
            "https://debug.example.test/json/version",
            "https://debug.example.test:9222/",
            "http://10.20.30.40:40123/cdp/browser/secret",
            "https://browser.internal/%64%65%76%74%6f%6f%6c%73/page/secret",
        ] {
            assert_eq!(project_renderer_url(raw), REDACTED_URL, "{raw}");
        }
    }

    #[test]
    fn preserves_non_debug_loopback_pages_without_metadata() {
        assert_eq!(
            project_renderer_url(
                "http://localhost:3000/application/dashboard?cookie=secret#private"
            ),
            "http://localhost:3000/application/dashboard"
        );
        assert_eq!(
            project_renderer_url("http://127.0.0.1:8080/docs/api-key"),
            "http://127.0.0.1:8080/docs/api-key"
        );
    }

    #[test]
    fn malformed_urls_fail_closed() {
        assert_eq!(project_renderer_url("not a url?token=secret"), REDACTED_URL);
        assert!(!project_renderer_url("https://[invalid?token=secret").contains("secret"));
    }
}
