use axum::extract::Request;
use axum::http::header::{HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS, X_XSS_PROTECTION};
use axum::middleware::Next;
use axum::response::Response;

fn allows_embedding(path: &str) -> bool {
    let mut segments = path.trim_start_matches('/').split('/');
    matches!(
        (segments.next(), segments.next(), segments.next(), segments.next(),),
        (Some("api"), Some("extensions"), Some(_extension_name), Some("assets"))
    )
}

/// Middleware that adds security response headers to every response.
///
/// Headers set:
/// - `X-Frame-Options: DENY` — prevent clickjacking on non-embeddable routes
/// - `X-Content-Type-Options: nosniff` — prevent MIME sniffing
/// - `X-XSS-Protection: 1; mode=block` — enable XSS filter
/// - `Referrer-Policy: strict-origin-when-cross-origin` — limit referrer leakage
pub async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    if !allows_embedding(&path) {
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
}
