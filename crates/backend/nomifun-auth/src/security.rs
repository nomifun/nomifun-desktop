use axum::extract::Request;
use axum::http::HeaderMap;
use axum::http::header::{
    CACHE_CONTROL, HeaderName, HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
    X_XSS_PROTECTION,
};
use axum::middleware::Next;
use axum::response::Response;
use nomifun_api_types::is_preview_capability;

const CONTENT_SECURITY_POLICY: HeaderName = HeaderName::from_static("content-security-policy");

/// Every origin the NomiFun SPA is itself served from, as a `frame-ancestors`
/// source list. One definition answers "may our own app embed this response?"
/// for the office preview proxy.
///
/// `'self'` covers WebUI, where the SPA and the API share an origin (including
/// behind a reverse proxy); the `tauri:` scheme source and the two
/// `tauri.localhost` origins cover the packaged desktop webview.
///
/// A debug build additionally trusts the pinned Vite dev origin, because
/// `tauri dev` points the webview at `devUrl` — `http://localhost:5173`
/// ([`apps/desktop/tauri.conf.json`], pinned by `server.port` in
/// `ui/vite.config.ts`) — which matches none of the sources above. Omitting it
/// does not fail loudly: the browser fetches the document, gets its 200, then
/// refuses to display it, so the panel is simply blank. Release builds never
/// load the SPA from a dev server, so the extra sources are compiled out
/// instead of shipped.
///
/// A macro rather than a `const` because `concat!` cannot take a constant, and
/// both policies below must be `&'static str` for `HeaderValue::from_static`.
#[cfg(not(debug_assertions))]
macro_rules! app_frame_ancestor_sources {
    () => {
        "'self' tauri: http://tauri.localhost https://tauri.localhost"
    };
}
#[cfg(debug_assertions)]
macro_rules! app_frame_ancestor_sources {
    () => {
        concat!(
            "'self' tauri: http://tauri.localhost https://tauri.localhost",
            " http://localhost:5173 http://127.0.0.1:5173"
        )
    };
}

const OFFICE_FRAME_ANCESTORS: &str = concat!("frame-ancestors ", app_frame_ancestor_sources!());

/// Isolated debug worktrees may use a different Vite port. Only an explicitly
/// configured loopback HTTP origin can extend the debug frame policy.
#[cfg(debug_assertions)]
fn debug_frame_origin(value: &str) -> Option<&str> {
    let uri: axum::http::Uri = value.parse().ok()?;
    (uri.scheme_str() == Some("http")
        && matches!(uri.host(), Some("localhost" | "127.0.0.1"))
        && uri.port_u16().is_some_and(|port| port > 0)
        && uri.path() == "/" && uri.query().is_none())
        .then_some(value.trim_end_matches('/'))
}

fn frame_ancestors() -> String {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("NOMIFUN_DEV_ORIGIN") {
        if let Some(origin) = debug_frame_origin(&value) {
            return format!("{OFFICE_FRAME_ANCESTORS} {origin}");
        }
    }
    OFFICE_FRAME_ANCESTORS.to_owned()
}

fn is_office_preview_capability_path(path: &str) -> bool {
    let mut segments = path.split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ),
        (Some(""), Some("api"), Some("ppt-proxy" | "office-watch-proxy"), Some(capability))
            if is_preview_capability(capability)
    )
}

fn is_miniapp_surface_capability_path(path: &str) -> bool {
    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    matches!(
        segments.as_slice(),
        ["api", "miniapps", _miniapp_id, "surface", "assets", capability, _epoch, _digest, ..]
            if is_preview_capability(capability)
    )
}

fn replace_frame_ancestors(policy: &str) -> String {
    let ancestors = frame_ancestors();
    let mut directives: Vec<&str> = policy
        .split(';')
        .map(str::trim)
        .filter(|directive| !directive.is_empty())
        .filter(|directive| {
            !directive
                .split_ascii_whitespace()
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
        })
        .collect();
    directives.push(&ancestors);
    directives.join("; ")
}

fn apply_office_frame_policy(headers: &mut HeaderMap) {
    headers.remove(X_FRAME_OPTIONS);

    // Multiple CSP response fields are enforced as an intersection. Replace
    // frame-ancestors in every field (rather than appending another policy), so
    // an upstream localhost policy cannot silently keep blocking the Tauri
    // ancestor while all unrelated upstream restrictions remain intact.
    let upstream_policies: Vec<String> = headers
        .get_all(&CONTENT_SECURITY_POLICY)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    headers.remove(&CONTENT_SECURITY_POLICY);

    if upstream_policies.is_empty() {
        headers.insert(
            CONTENT_SECURITY_POLICY.clone(),
            HeaderValue::from_str(&frame_ancestors()).expect("validated frame origins"),
        );
        return;
    }

    for policy in upstream_policies {
        if let Ok(value) = HeaderValue::from_str(&replace_frame_ancestors(&policy)) {
            headers.append(CONTENT_SECURITY_POLICY.clone(), value);
        }
    }

    if !headers.contains_key(&CONTENT_SECURITY_POLICY) {
        headers.insert(
            CONTENT_SECURITY_POLICY.clone(),
            HeaderValue::from_str(&frame_ancestors()).expect("validated frame origins"),
        );
    }
}

/// Middleware that adds security response headers to every response.
///
/// Headers set:
/// - `X-Frame-Options: DENY` — prevent clickjacking on non-embeddable routes
/// - Office capability proxy routes replace XFO with a narrow frame-ancestors
///   policy that permits same-origin WebUI and the Tauri application origins
/// - `X-Content-Type-Options: nosniff` — prevent MIME sniffing
/// - `X-XSS-Protection: 1; mode=block` — enable XSS filter
/// - `Referrer-Policy: strict-origin-when-cross-origin` — limit referrer leakage
pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    // API responses represent mutable application state.  They must never be
    // replayed from a WebView/browser cache when a route is revisited after a
    // mutation (most visibly: an initially-empty conversation after sending a
    // message).  Preserve an explicit route policy so immutable logo/assets and
    // other deliberately cacheable binaries keep their ETag/max-age behavior.
    if path.starts_with("/api/") && !headers.contains_key(CACHE_CONTROL) {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }

    if is_office_preview_capability_path(&path)
        || is_miniapp_surface_capability_path(&path)
    {
        apply_office_frame_policy(headers);
    } else {
        headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_XSS_PROTECTION, HeaderValue::from_static("1; mode=block"));
    headers.insert(
        REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(debug_assertions)]
    fn isolated_debug_origin_requires_an_explicit_loopback_http_port() {
        assert_eq!(debug_frame_origin("http://127.0.0.1:5197"), Some("http://127.0.0.1:5197"));
        assert_eq!(debug_frame_origin("http://localhost:5197/"), Some("http://localhost:5197"));
        for origin in ["https://evil.example", "http://127.0.0.1.evil.example:5197", "http://localhost", "http://localhost:0", "http://localhost:5197/path", "http://localhost:5197/?x=1", "http://localhost:5197\r\nX-Test: injected"] {
            assert!(debug_frame_origin(origin).is_none(), "{origin:?}");
        }
    }
    use axum::body::Body;
    use axum::routing::get;
    use axum::{Router, middleware};
    use tower::ServiceExt;

    const CAPABILITY: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    async fn upstream_csp_response() -> Response {
        let mut response = Response::new(Body::from("ok"));
        response.headers_mut().append(
            CONTENT_SECURITY_POLICY.clone(),
            HeaderValue::from_static("default-src 'none'; frame-ancestors https://evil.example"),
        );
        response.headers_mut().append(
            CONTENT_SECURITY_POLICY.clone(),
            HeaderValue::from_static("img-src 'self'; FRAME-ANCESTORS 'none'"),
        );
        response
    }

    #[tokio::test]
    async fn all_security_headers_present() {
        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(axum::http::Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(response.headers().get("x-xss-protection").unwrap(), "1; mode=block");
        assert_eq!(
            response.headers().get("referrer-policy").unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert!(response.headers().get(CACHE_CONTROL).is_none());
    }

    #[tokio::test]
    async fn mutable_api_responses_are_not_cacheable() {
        let app = Router::new()
            .route("/api/conversations/{id}/messages", get(|| async { "[]" }))
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/conversations/conv-1/messages")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    }

    #[tokio::test]
    async fn explicit_asset_cache_policy_is_preserved() {
        let app = Router::new()
            .route(
                "/api/assets/logo.svg",
                get(|| async {
                    let mut response = Response::new(Body::from("svg"));
                    response.headers_mut().insert(
                        CACHE_CONTROL,
                        HeaderValue::from_static("public, max-age=31536000, immutable"),
                    );
                    response
                }),
            )
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/assets/logo.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.headers().get(CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[tokio::test]
    async fn miniapp_surface_capability_can_be_framed_by_the_app() {
        let capability =
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let digest =
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let path = format!(
            "/api/miniapps/miniapp-1/surface/assets/{capability}/3/{digest}/ui/index.html"
        );
        let app = Router::new()
            .route(&path, get(|| async { "ok" }))
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.headers().get(X_FRAME_OPTIONS).is_none());
        let policy = response
            .headers()
            .get(&CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(policy.contains("frame-ancestors 'self'"));
    }

    #[tokio::test]
    async fn security_headers_on_error_responses() {
        let app = Router::new()
            .route(
                "/error",
                get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/error")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        // Security headers still present even on error responses
        assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
    }

    #[tokio::test]
    async fn office_capability_routes_allow_only_webui_and_tauri_ancestors() {
        for prefix in ["ppt-proxy", "office-watch-proxy"] {
            let uri = format!("/api/{prefix}/{CAPABILITY}/assets/index.html");
            let app = Router::new()
                .route(
                    "/api/{prefix}/{capability}/{*path}",
                    get(|| async { "ok" }),
                )
                .layer(middleware::from_fn(security_headers_middleware));

            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert!(response.headers().get(X_FRAME_OPTIONS).is_none());
            let policy = response
                .headers()
                .get(&CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap();
            assert!(policy.contains("frame-ancestors 'self'"));
            assert!(policy.contains("tauri:"));
            assert!(policy.contains("http://tauri.localhost"));
            assert!(policy.contains("https://tauri.localhost"));
            assert!(!policy.contains('*'));
            assert!(!policy.contains("evil.example"));
        }
    }

    #[tokio::test]
    async fn office_capability_routes_replace_frame_ancestors_in_every_upstream_policy() {
        let app = Router::new()
            .route(
                "/api/ppt-proxy/{capability}/{*path}",
                get(upstream_csp_response),
            )
            .layer(middleware::from_fn(security_headers_middleware));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/ppt-proxy/{CAPABILITY}/index.html"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let policies: Vec<&str> = response
            .headers()
            .get_all(&CONTENT_SECURITY_POLICY)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(policies.len(), 2);
        assert!(policies[0].contains("default-src 'none'"));
        assert!(policies[1].contains("img-src 'self'"));
        assert!(policies.iter().all(|policy| policy.contains(OFFICE_FRAME_ANCESTORS)));
        assert!(policies.iter().all(|policy| {
            !policy.contains("evil.example") && !policy.to_ascii_lowercase().contains("frame-ancestors 'none'")
        }));
    }

    #[tokio::test]
    async fn malformed_or_similar_office_paths_remain_frame_denied() {
        for uri in [
            "/api/ppt-proxy/43210/",
            "/api/office-watch-proxy/not-a-capability/",
            "/api/ppt-proxy-extra/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/",
        ] {
            let app = Router::new()
                .fallback(get(|| async { "ok" }))
                .layer(middleware::from_fn(security_headers_middleware));
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(response.headers().get(X_FRAME_OPTIONS).unwrap(), "DENY");
            assert!(response.headers().get(&CONTENT_SECURITY_POLICY).is_none());
        }
    }
}
