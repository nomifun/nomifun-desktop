use axum::http::{HeaderMap, Request, header};

use nomifun_common::constants::COOKIE_NAME;

/// Extract the client IP address from request headers.
///
/// Priority: `X-Forwarded-For` (first IP) > `X-Real-IP` > `"unknown"`.
pub fn extract_client_ip<B>(request: &Request<B>) -> String {
    extract_client_ip_from_headers(request.headers())
}

/// Extract client IP from a `HeaderMap` directly.
pub fn extract_client_ip_from_headers(headers: &HeaderMap) -> String {
    // X-Forwarded-For: client, proxy1, proxy2
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first_ip) = forwarded.split(',').next()
    {
        let ip = first_ip.trim();
        if !ip.is_empty() {
            return ip.to_owned();
        }
    }

    // X-Real-IP
    if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = real_ip.trim();
        if !ip.is_empty() {
            return ip.to_owned();
        }
    }

    "unknown".to_owned()
}

/// Extract bearer token from HTTP request headers.
///
/// Priority: `Authorization: Bearer <token>` > `nomifun-session` cookie.
pub fn extract_token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = extract_bearer_token(headers) {
        return Some(token);
    }
    extract_cookie_value(headers, COOKIE_NAME)
}

/// Extract bearer token from WebSocket upgrade request headers.
///
/// Priority: `Authorization` > `Cookie` > `Sec-WebSocket-Protocol` (first value).
pub fn extract_token_from_ws_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = extract_bearer_token(headers) {
        return Some(token);
    }

    if let Some(token) = extract_cookie_value(headers, COOKIE_NAME) {
        return Some(token);
    }

    // Sec-WebSocket-Protocol: <token>, ...
    headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .and_then(|protocols| {
            let first = protocols.split(',').next()?.trim();
            if first.is_empty() { None } else { Some(first.to_owned()) }
        })
}

/// Extract a named cookie value from the `Cookie` header.
pub fn extract_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        if key.trim() == name {
            let v = value.trim();
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    None
}

/// Extract the bearer token from the `Authorization` header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let auth = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = auth.strip_prefix("Bearer ")?;
    if token.is_empty() { None } else { Some(token.to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for &(name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    // --- extract_client_ip ---

    #[test]
    fn ip_from_x_forwarded_for() {
        let headers = headers_with(&[("x-forwarded-for", "1.2.3.4, 5.6.7.8")]);
        assert_eq!(extract_client_ip_from_headers(&headers), "1.2.3.4");
    }

    #[test]
    fn ip_from_x_real_ip() {
        let headers = headers_with(&[("x-real-ip", "10.0.0.1")]);
        assert_eq!(extract_client_ip_from_headers(&headers), "10.0.0.1");
    }

    #[test]
    fn ip_forwarded_for_takes_priority() {
        let headers = headers_with(&[("x-forwarded-for", "1.2.3.4"), ("x-real-ip", "10.0.0.1")]);
        assert_eq!(extract_client_ip_from_headers(&headers), "1.2.3.4");
    }

    #[test]
    fn ip_fallback_to_unknown() {
        let headers = HeaderMap::new();
        assert_eq!(extract_client_ip_from_headers(&headers), "unknown");
    }

    // --- extract_token_from_headers ---

    #[test]
    fn token_from_authorization_header() {
        let headers = headers_with(&[("authorization", "Bearer my_jwt_token")]);
        assert_eq!(extract_token_from_headers(&headers), Some("my_jwt_token".into()));
    }

    #[test]
    fn token_from_cookie() {
        let headers = headers_with(&[("cookie", "nomifun-session=cookie_token; other=val")]);
        assert_eq!(extract_token_from_headers(&headers), Some("cookie_token".into()));
    }

    #[test]
    fn token_header_takes_priority_over_cookie() {
        let headers = headers_with(&[
            ("authorization", "Bearer header_token"),
            ("cookie", "nomifun-session=cookie_token"),
        ]);
        assert_eq!(extract_token_from_headers(&headers), Some("header_token".into()));
    }

    #[test]
    fn token_none_when_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_token_from_headers(&headers), None);
    }

    #[test]
    fn token_none_for_empty_bearer() {
        let headers = headers_with(&[("authorization", "Bearer ")]);
        assert_eq!(extract_token_from_headers(&headers), None);
    }

    #[test]
    fn token_none_for_non_bearer_auth() {
        let headers = headers_with(&[("authorization", "Basic dXNlcjpwYXNz")]);
        assert_eq!(extract_token_from_headers(&headers), None);
    }

    // --- extract_token_from_ws_headers ---

    #[test]
    fn ws_token_from_authorization() {
        let headers = headers_with(&[("authorization", "Bearer ws_token")]);
        assert_eq!(extract_token_from_ws_headers(&headers), Some("ws_token".into()));
    }

    #[test]
    fn ws_token_from_cookie() {
        let headers = headers_with(&[("cookie", "nomifun-session=ws_cookie")]);
        assert_eq!(extract_token_from_ws_headers(&headers), Some("ws_cookie".into()));
    }

    #[test]
    fn ws_token_from_sec_websocket_protocol() {
        let headers = headers_with(&[("sec-websocket-protocol", "my_ws_token, graphql-ws")]);
        assert_eq!(extract_token_from_ws_headers(&headers), Some("my_ws_token".into()));
    }

    #[test]
    fn ws_token_priority_order() {
        let headers = headers_with(&[
            ("authorization", "Bearer auth_token"),
            ("cookie", "nomifun-session=cookie_token"),
            ("sec-websocket-protocol", "proto_token"),
        ]);
        assert_eq!(extract_token_from_ws_headers(&headers), Some("auth_token".into()));
    }

    #[test]
    fn ws_token_fallback_through_sources() {
        // Only cookie and protocol, no authorization
        let headers = headers_with(&[
            ("cookie", "nomifun-session=cookie_token"),
            ("sec-websocket-protocol", "proto_token"),
        ]);
        assert_eq!(extract_token_from_ws_headers(&headers), Some("cookie_token".into()));
    }

    // --- extract_cookie_value ---

    #[test]
    fn cookie_value_extracted() {
        let headers = headers_with(&[("cookie", "a=1; target=hello; b=2")]);
        assert_eq!(extract_cookie_value(&headers, "target"), Some("hello".into()));
    }

    #[test]
    fn cookie_value_not_found() {
        let headers = headers_with(&[("cookie", "a=1; b=2")]);
        assert_eq!(extract_cookie_value(&headers, "missing"), None);
    }

    #[test]
    fn cookie_value_no_cookie_header() {
        let headers = HeaderMap::new();
        assert_eq!(extract_cookie_value(&headers, "any"), None);
    }

    #[test]
    fn cookie_value_skips_malformed_entries() {
        // Entry without '=' should be skipped, not abort the entire search
        let headers = headers_with(&[("cookie", "malformed; target=found; also_bad")]);
        assert_eq!(extract_cookie_value(&headers, "target"), Some("found".into()));
    }

    #[test]
    fn cookie_value_all_malformed_returns_none() {
        let headers = headers_with(&[("cookie", "no_equals; also_none")]);
        assert_eq!(extract_cookie_value(&headers, "target"), None);
    }

    #[test]
    fn cookie_value_malformed_before_target() {
        // Malformed entry appears before the target cookie
        let headers = headers_with(&[("cookie", "bad_entry; nomifun-session=tok123")]);
        assert_eq!(extract_cookie_value(&headers, "nomifun-session"), Some("tok123".into()));
    }

    #[test]
    fn token_from_cookie_with_malformed_entries() {
        // End-to-end: extract_token_from_headers should still find the
        // session cookie even when other entries lack '='
        let headers = headers_with(&[("cookie", "garbage; nomifun-session=abc; nope")]);
        assert_eq!(extract_token_from_headers(&headers), Some("abc".into()));
    }
}
