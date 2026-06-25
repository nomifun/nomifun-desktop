use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;

use nomifun_common::AppError;
use nomifun_common::constants::{CSRF_COOKIE_NAME, CSRF_HEADER_NAME};

use crate::cookie::CookieConfig;
use crate::extract::extract_cookie_value;

/// CSRF protection middleware using the Double Submit Cookie pattern.
///
/// Behavior:
/// - Safe methods (GET, HEAD, OPTIONS) bypass validation.
/// - Exempt paths (`/login`, `/api/auth/qr-login`, `/api/auth/setup`) bypass validation.
/// - All other requests must include an `x-csrf-token` header whose value
///   matches the `nomifun-csrf-token` cookie.
/// - Sets the CSRF cookie on responses if the client does not have one.
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
    let is_exempt = path == "/login" || path == "/api/auth/qr-login" || path == "/api/auth/setup";

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
                return Err(AppError::Forbidden("CSRF token validation failed".into()));
            }
        }
    }

    let mut response = next.run(request).await;

    // Set CSRF cookie if the client doesn't have one
    if csrf_cookie.is_none() {
        let token = generate_csrf_token();
        let cookie_str = cookie_config.build_csrf_cookie(&token);
        if let Ok(value) = HeaderValue::from_str(&cookie_str) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }

    Ok(response)
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
}
