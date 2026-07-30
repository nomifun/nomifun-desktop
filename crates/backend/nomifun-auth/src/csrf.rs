use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use nomifun_common::AppError;
use nomifun_common::constants::{CSRF_COOKIE_NAME, CSRF_HEADER_NAME};

use crate::cookie::CookieConfig;
use crate::extract::extract_cookie_value;

/// CSRF protection middleware using the Double Submit Cookie pattern.
///
/// Behavior:
/// - Safe methods (GET, HEAD, OPTIONS) bypass validation.
/// - Exempt paths bypass validation: `/login`, `/api/auth/qr-login`,
///   `/api/auth/setup` (no session exists yet to protect), and `/logout`
///   (idempotent and self-deauthorizing — a forged logout can only end the
///   victim's own session, while requiring a token here meant a stale cookie
///   made logout permanently fail and the server session survive).
/// - All other requests must include an `x-csrf-token` header whose value
///   matches the `nomifun-csrf-token` cookie.
/// - Every response re-issues the CSRF cookie (same token, fresh `Max-Age`):
///   sliding expiry. The cookie used to be set only when absent with a fixed
///   30-day `Max-Age`, so a long-lived login outlived its CSRF cookie and
///   every mutation started failing with an opaque 403 until a lucky
///   navigation re-seeded it (audit 2026-07-30, finding B).
/// - A validation failure (403) also re-seeds the cookie so the client can
///   self-heal by retrying once, instead of staying wedged.
pub async fn csrf_middleware(
    State(cookie_config): State<Arc<CookieConfig>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    // Extract CSRF cookie before consuming the request
    let csrf_cookie = extract_cookie_value(request.headers(), CSRF_COOKIE_NAME);

    // Validate CSRF for state-changing requests
    let needs_validation = matches!(method, Method::POST | Method::PUT | Method::DELETE | Method::PATCH);
    let is_exempt =
        path == "/login" || path == "/api/auth/qr-login" || path == "/api/auth/setup" || path == "/logout";

    // Locally-trusted requests authenticate via the `X-Nomi-Local-Trust` header,
    // not an ambient cookie, so they are not a CSRF target — skip validation.
    let local_trusted = request.extensions().get::<crate::trust::LocalTrusted>().is_some();

    if needs_validation && !is_exempt && !local_trusted {
        let header_token = request
            .headers()
            .get(CSRF_HEADER_NAME)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());

        match (&csrf_cookie, header_token) {
            (Some(cookie), Some(ref hdr)) if !cookie.is_empty() && cookie == hdr => {
                // Valid: cookie and header match
            }
            _ => {
                // Reject, but re-seed the cookie on the failure response so the
                // client's very next attempt can succeed (self-healing).
                let mut response = AppError::Forbidden("CSRF token validation failed".into()).into_response();
                append_csrf_cookie(&mut response, &cookie_config, &csrf_cookie);
                return Ok(response);
            }
        }
    }

    let mut response = next.run(request).await;
    append_csrf_cookie(&mut response, &cookie_config, &csrf_cookie);
    Ok(response)
}

/// Append a `Set-Cookie` header carrying the CSRF token: the existing token
/// with a fresh `Max-Age` (sliding renewal), or a newly generated one when the
/// client has none. Cookie values cannot contain `;` (the `Cookie` header
/// separator), so reflecting the client's value cannot inject attributes;
/// `HeaderValue::from_str` rejects control characters as defense in depth.
fn append_csrf_cookie(response: &mut Response, cookie_config: &CookieConfig, existing: &Option<String>) {
    let token = match existing {
        Some(value) if !value.is_empty() => value.clone(),
        _ => generate_csrf_token(),
    };
    let cookie_str = cookie_config.build_csrf_cookie(&token);
    if let Ok(value) = HeaderValue::from_str(&cookie_str) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// Generate a cryptographically random 32-byte CSRF token as a hex string.
fn generate_csrf_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("OS entropy source unavailable");
    let mut hex = String::with_capacity(64);
    for byte in buf {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::post;
    use tower::ServiceExt;

    #[test]
    fn csrf_token_is_64_hex_chars() {
        let token = generate_csrf_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn csrf_tokens_are_unique() {
        let t1 = generate_csrf_token();
        let t2 = generate_csrf_token();
        assert_ne!(t1, t2);
    }

    // -- Middleware-level lifecycle tests ------------------------------------

    fn test_router() -> Router {
        let config = Arc::new(CookieConfig {
            secure: false,
            same_site: "Lax",
        });
        Router::new()
            .route("/api/thing", post(|| async { "ok" }).get(|| async { "ok" }))
            .route("/logout", post(|| async { "bye" }))
            .layer(axum::middleware::from_fn_with_state(config, csrf_middleware))
    }

    fn set_cookie_values(response: &Response) -> Vec<String> {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_owned))
            .collect()
    }

    #[tokio::test]
    async fn get_seeds_csrf_cookie_when_absent() {
        let resp = test_router()
            .oneshot(HttpRequest::builder().uri("/api/thing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookies = set_cookie_values(&resp);
        assert!(
            cookies.iter().any(|c| c.starts_with("nomifun-csrf-token=")),
            "GET must seed the csrf cookie: {cookies:?}"
        );
    }

    #[tokio::test]
    async fn every_response_slides_existing_cookie_max_age() {
        // Sliding renewal: a client that already has the cookie gets it
        // re-issued (same token, fresh Max-Age) instead of nothing.
        let resp = test_router()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/thing")
                    .header(header::COOKIE, "nomifun-csrf-token=tok123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let cookies = set_cookie_values(&resp);
        assert!(
            cookies.iter().any(|c| c.starts_with("nomifun-csrf-token=tok123")),
            "existing token must be re-issued with a fresh Max-Age: {cookies:?}"
        );
    }

    #[tokio::test]
    async fn mutation_with_matching_header_passes() {
        let resp = test_router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/thing")
                    .header(header::COOKIE, "nomifun-csrf-token=tok123")
                    .header("x-csrf-token", "tok123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejected_mutation_reseeds_cookie_for_self_healing() {
        // Missing header → 403, but the failure response must carry a usable
        // csrf cookie so the client's single retry can succeed.
        let resp = test_router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/thing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let cookies = set_cookie_values(&resp);
        assert!(
            cookies.iter().any(|c| c.starts_with("nomifun-csrf-token=")),
            "403 must re-seed the csrf cookie: {cookies:?}"
        );
    }

    #[tokio::test]
    async fn logout_is_exempt_from_csrf() {
        // Logout is idempotent and self-deauthorizing; requiring a token here
        // let a stale cookie make logout fail forever (session survived).
        let resp = test_router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/logout")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
