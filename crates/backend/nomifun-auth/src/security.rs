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
/// for both the office preview proxy and the mini-app runner.
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

/// The response policy for a served mini-app document.
///
/// Two directives, both load-bearing:
///
/// * `sandbox` WITHOUT `allow-same-origin` — the document is AI-generated and
///   must never run with the deployment's own origin authority. In WebUI mode it
///   is served from the very origin that holds the session cookie and the API, so
///   a same-origin script could read them; the CSP `sandbox` directive forces an
///   opaque origin even then, which is exactly why it is set here and not left to
///   the embedding iframe's `sandbox` attribute (a document reached directly, or
///   framed by markup we do not control, would otherwise be unsandboxed). The
///   four allowances are what an interactive single-file tool needs: its inline
///   script, forms, `window.open`, and `alert`/`confirm`.
/// * `frame-ancestors` — the same source list the office preview proxy uses
///   ([`OFFICE_FRAME_ANCESTORS`], asserted below), replacing the
///   `X-Frame-Options: DENY` this route must not carry: only the origins our own
///   SPA runs from may embed the runner.
const MINIAPP_SERVE_POLICY: &str = concat!(
    "sandbox allow-scripts allow-forms allow-popups allow-modals; frame-ancestors ",
    app_frame_ancestor_sources!()
);

/// `GET /api/miniapps/{miniapp_id}/serve` — the document channel the preview and
/// runner iframes load. Exactly `/serve`, and nothing below it: the trailing
/// `None` keeps a longer path from inheriting the exemption, so every other
/// mini-app route (the metadata CRUD surface) stays frame-denied.
fn is_miniapp_serve_path(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').split('/');
    match (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) {
        (Some("api"), Some("miniapps"), Some(_miniapp_id), Some("serve")) => {
            segments.next().is_none()
        }
        _ => false,
    }
}

/// Routes whose responses may be framed with no policy of their own:
/// `/api/extensions/{name}/assets/**`, because an extension renders its own
/// settings UI inside an iframe in the app.
///
/// The mini-app serve channel is also framable, but it is NOT listed here — it
/// gets a policy instead of an exemption ([`MINIAPP_SERVE_POLICY`], applied via
/// [`is_miniapp_serve_path`]), because "may be framed" there means "by us only,
/// and sandboxed".
fn allows_embedding(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').split('/');
    matches!(
        (segments.next(), segments.next(), segments.next(), segments.next(),),
        (Some("api"), Some("extensions"), Some(_extension_name), Some("assets"))
    )
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

fn replace_frame_ancestors(policy: &str) -> String {
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
    directives.push(OFFICE_FRAME_ANCESTORS);
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
            HeaderValue::from_static(OFFICE_FRAME_ANCESTORS),
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
            HeaderValue::from_static(OFFICE_FRAME_ANCESTORS),
        );
    }
}

/// Replace whatever the serve handler produced with exactly one policy: the
/// document's isolation must not depend on a second CSP field intersecting the
/// way we hope, and the handler deliberately sets none (this middleware is the
/// single source of truth, as for the office proxy).
fn apply_miniapp_serve_policy(headers: &mut HeaderMap) {
    headers.remove(X_FRAME_OPTIONS);
    headers.remove(&CONTENT_SECURITY_POLICY);
    headers.insert(
        CONTENT_SECURITY_POLICY.clone(),
        HeaderValue::from_static(MINIAPP_SERVE_POLICY),
    );
}

/// Middleware that adds security response headers to every response.
///
/// Headers set:
/// - `X-Frame-Options: DENY` — prevent clickjacking on non-embeddable routes
/// - Office capability proxy routes replace XFO with a narrow frame-ancestors
///   policy that permits same-origin WebUI and the Tauri application origins
/// - The mini-app serve route replaces XFO with the same narrow frame-ancestors
///   list plus a `sandbox` directive, so the AI-generated document runs on an
///   opaque origin ([`MINIAPP_SERVE_POLICY`])
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

    if is_office_preview_capability_path(&path) {
        apply_office_frame_policy(headers);
    } else if is_miniapp_serve_path(&path) {
        apply_miniapp_serve_policy(headers);
    } else if !allows_embedding(&path) {
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
    async fn extension_asset_routes_omit_frame_deny_header() {
        let app = Router::new()
            .route(
                "/api/extensions/hello/assets/settings/index.html",
                get(|| async { "ok" }),
            )
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/extensions/hello/assets/settings/index.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(response.headers().get("x-frame-options").is_none());
        assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
    }

    #[tokio::test]
    async fn miniapp_serve_route_is_sandboxed_and_framable_only_by_us() {
        const MINI_APP_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
        let app = Router::new()
            .route("/api/miniapps/{miniapp_id}/serve", get(|| async { "<h1/>" }))
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/api/miniapps/{MINI_APP_ID}/serve"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.headers().get(X_FRAME_OPTIONS).is_none(),
            "the runner/preview iframe cannot load a frame-denied document"
        );
        let policies: Vec<&str> = response
            .headers()
            .get_all(&CONTENT_SECURITY_POLICY)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(
            policies.len(),
            1,
            "exactly one policy field, so isolation never depends on an intersection: {policies:?}"
        );
        let policy = policies[0];
        // No allow-same-origin: the generated document must not run with the
        // deployment's origin authority, which in WebUI mode is the session's.
        assert!(policy.contains("sandbox allow-scripts allow-forms allow-popups allow-modals"));
        assert!(!policy.contains("allow-same-origin"));
        assert!(policy.contains(OFFICE_FRAME_ANCESTORS));
        assert!(!policy.contains('*'));
        assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
    }

    /// Both policies must keep deriving their ancestor list from the one shared
    /// source list, so an origin added for one surface is never missing on the other.
    #[test]
    fn miniapp_serve_policy_reuses_the_office_ancestor_list() {
        assert!(
            MINIAPP_SERVE_POLICY.ends_with(OFFICE_FRAME_ANCESTORS),
            "{MINIAPP_SERVE_POLICY}"
        );
    }

    /// Regression: the runner iframe loads the serve route cross-origin from the
    /// Vite dev server under `tauri dev`. When that origin is absent from
    /// `frame-ancestors` the request still succeeds with 200 and the browser then
    /// refuses to render the frame, so the failure looks like an empty panel with
    /// a healthy server log. Debug builds must therefore trust `devUrl`.
    #[test]
    #[cfg(debug_assertions)]
    fn a_debug_build_lets_the_vite_dev_origin_frame_a_miniapp() {
        for origin in ["http://localhost:5173", "http://127.0.0.1:5173"] {
            assert!(MINIAPP_SERVE_POLICY.contains(origin), "{origin} missing: {MINIAPP_SERVE_POLICY}");
            assert!(OFFICE_FRAME_ANCESTORS.contains(origin), "{origin} missing: {OFFICE_FRAME_ANCESTORS}");
        }
    }

    #[tokio::test]
    async fn miniapp_serve_route_policy_replaces_any_handler_policy() {
        let app = Router::new()
            .route("/api/miniapps/{miniapp_id}/serve", get(upstream_csp_response))
            .layer(middleware::from_fn(security_headers_middleware));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/miniapps/0190f5fe-7c00-7a00-8000-000000000001/serve")
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
        assert_eq!(policies, vec![MINIAPP_SERVE_POLICY]);
    }

    #[tokio::test]
    async fn miniapp_management_routes_remain_frame_denied() {
        const MINI_APP_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
        for uri in [
            "/api/miniapps".to_string(),
            format!("/api/miniapps/{MINI_APP_ID}"),
            // Nothing below `/serve` inherits the exemption.
            format!("/api/miniapps/{MINI_APP_ID}/serve/index.html"),
            format!("/api/miniapps/{MINI_APP_ID}/serve/"),
        ] {
            let app = Router::new()
                .fallback(get(|| async { "ok" }))
                .layer(middleware::from_fn(security_headers_middleware));
            let response = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri(&uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                response.headers().get(X_FRAME_OPTIONS).unwrap(),
                "DENY",
                "{uri} must stay frame-denied"
            );
            assert!(
                response.headers().get(&CONTENT_SECURITY_POLICY).is_none(),
                "{uri} is not a document channel and must carry no sandbox policy"
            );
        }
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
