use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use dashmap::DashMap;

use nomifun_common::AppError;

use crate::extract::extract_client_ip;
use crate::middleware::CurrentUser;

/// Rate limit entry tracking request count within a fixed time window.
struct RateLimitEntry {
    count: u32,
    reset_time_ms: u64,
}

/// Fixed-window rate limiter backed by a concurrent `DashMap`.
///
/// Thread-safe for use across multiple request handlers.
pub struct RateLimiter {
    entries: DashMap<String, RateLimitEntry>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    /// Create a rate limiter with the given capacity and window duration.
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            max_requests,
            window,
        }
    }

    /// Auth rate limiter: 5 failed attempts per 15-minute window.
    pub fn auth() -> Self {
        Self::new(5, Duration::from_secs(15 * 60))
    }

    /// API rate limiter: 60 requests per 1-minute window.
    pub fn api() -> Self {
        Self::new(60, Duration::from_secs(60))
    }

    /// Authenticated action limiter: 20 requests per 1-minute window.
    pub fn authenticated_action() -> Self {
        Self::new(20, Duration::from_secs(60))
    }

    /// Check if the key is rate limited without modifying state.
    ///
    /// For the auth rate limiter: check first, record failure later
    /// via [`record_attempt`](Self::record_attempt).
    pub fn check(&self, key: &str) -> Result<(), AppError> {
        let now = now_ms();
        if let Some(entry) = self.entries.get(key)
            && now < entry.reset_time_ms
            && entry.count >= self.max_requests
        {
            return Err(AppError::RateLimited);
        }
        Ok(())
    }

    /// Check rate limit and increment the counter atomically.
    ///
    /// For API and authenticated-action rate limiters.
    pub fn check_and_increment(&self, key: &str) -> Result<(), AppError> {
        let now = now_ms();
        let window_ms = self.window.as_millis() as u64;

        let mut entry = self.entries.entry(key.to_owned()).or_insert(RateLimitEntry {
            count: 0,
            reset_time_ms: now + window_ms,
        });

        if now >= entry.reset_time_ms {
            entry.count = 0;
            entry.reset_time_ms = now + window_ms;
        }

        if entry.count >= self.max_requests {
            return Err(AppError::RateLimited);
        }

        entry.count += 1;
        Ok(())
    }

    /// Record a single failed attempt without checking the limit.
    ///
    /// Used by the auth rate limiter after a failed login response.
    pub fn record_attempt(&self, key: &str) {
        let now = now_ms();
        let window_ms = self.window.as_millis() as u64;

        let mut entry = self.entries.entry(key.to_owned()).or_insert(RateLimitEntry {
            count: 0,
            reset_time_ms: now + window_ms,
        });

        if now >= entry.reset_time_ms {
            entry.count = 0;
            entry.reset_time_ms = now + window_ms;
        }

        entry.count += 1;
    }

    /// Remove expired entries to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = now_ms();
        self.entries.retain(|_, entry| now < entry.reset_time_ms);
    }

    /// Start a background task that cleans up expired entries periodically.
    pub fn start_cleanup_task(self: &Arc<Self>, interval: Duration) {
        let limiter = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                limiter.cleanup();
            }
        });
    }

    /// Number of tracked keys (for monitoring/testing).
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Client identity for rate limiting.
///
/// - Public TCP peer address (`ConnectInfo`) wins outright: it cannot be
///   spoofed, and a public peer talking to us directly has no legitimate
///   proxy in between.
/// - A private/loopback peer is some local hop — the docker bridge, a
///   reverse proxy (Caddy), or a LAN gateway. Those deployments carry the
///   real client in `X-Forwarded-For`/`X-Real-IP`, so prefer the header and
///   fall back to the peer address when no header is present.
/// - Without `ConnectInfo` (tests, in-process routers) fall back to the
///   headers, matching the pre-ConnectInfo behavior.
///
/// The web host installs `ConnectInfo` (see `apps/web/src/main.rs`); before
/// it did, every docker/WebUI client collapsed into one shared `"unknown"`
/// bucket and locked each other out (audit 2026-07-30, finding A).
fn rate_limit_ip(request: &Request) -> String {
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|axum::extract::ConnectInfo(addr)| addr.ip());
    match peer {
        Some(ip) if !is_private_or_loopback(ip) => ip.to_string(),
        Some(ip) => {
            let forwarded = extract_client_ip(request);
            if forwarded == "unknown" {
                ip.to_string()
            } else {
                forwarded
            }
        }
        None => extract_client_ip(request),
    }
}

/// True for peers that are plausibly a local proxy hop rather than the real
/// client: loopback, RFC 1918 private ranges, and link-local/ULA.
fn is_private_or_loopback(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                // fc00::/7 unique-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Auth rate limit middleware: 5 failed attempts per 15 minutes per IP.
///
/// Pre-checks the limit; records failures only for definitive credential
/// rejections (401/403). Other non-success statuses must not consume the
/// budget: 400 (malformed request), 409 (setup already claimed), 429 (the
/// limiter itself), and 5xx (server fault) are not evidence of a
/// credential-guessing client — counting them let unrelated errors lock a
/// whole deployment out of login (audit 2026-07-30, finding A).
pub async fn auth_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let ip = rate_limit_ip(&request);
    limiter.check(&ip)?;

    let response = next.run(request).await;

    let status = response.status();
    if status == axum::http::StatusCode::UNAUTHORIZED || status == axum::http::StatusCode::FORBIDDEN {
        limiter.record_attempt(&ip);
    }

    Ok(response)
}

/// API rate limit middleware: 60 requests per minute per IP.
pub async fn api_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let ip = rate_limit_ip(&request);
    limiter.check_and_increment(&ip)?;
    Ok(next.run(request).await)
}

/// Authenticated action rate limit middleware: 20 requests per minute.
///
/// Key priority: session/bearer token (hashed) → user id → client IP.
///
/// Keying by token gives every browser session its own bucket. Keying by
/// user id alone made the quota deployment-wide in practice: WebUI installs
/// typically share one admin account, so every tab and device drained the
/// same 20/min budget (audit 2026-07-30). The token hash is truncated —
/// it identifies a bucket, it cannot be reversed into the token.
pub async fn authenticated_action_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let key = if let Some(token) = crate::extract::extract_token_from_headers(request.headers()) {
        format!("session:{}", crate::jwt::token_bucket_key(&token))
    } else if let Some(user) = request.extensions().get::<CurrentUser>() {
        // Locally-trusted callers (desktop webview) carry no session token.
        format!("user:{}", user.id)
    } else {
        format!("ip:{}", rate_limit_ip(&request))
    };
    limiter.check_and_increment(&key)?;
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_limiter_allows_requests() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        assert!(limiter.check("key").is_ok());
    }

    #[test]
    fn check_and_increment_enforces_limit() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check_and_increment("key").is_ok());
        assert!(limiter.check_and_increment("key").is_ok());
        assert!(limiter.check_and_increment("key").is_err());
    }

    #[test]
    fn different_keys_have_independent_limits() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        assert!(limiter.check_and_increment("key_a").is_ok());
        assert!(limiter.check_and_increment("key_b").is_ok());
        assert!(limiter.check_and_increment("key_a").is_err());
    }

    #[test]
    fn check_does_not_increment() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        // check() alone never increments
        assert!(limiter.check("key").is_ok());
        assert!(limiter.check("key").is_ok());
        // One recorded attempt fills the quota
        limiter.record_attempt("key");
        assert!(limiter.check("key").is_err());
    }

    #[test]
    fn record_attempt_increments_counter() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        limiter.record_attempt("key");
        limiter.record_attempt("key");
        assert!(limiter.check("key").is_err());
    }

    #[test]
    fn expired_window_resets_count() {
        let limiter = RateLimiter::new(1, Duration::from_millis(50));
        assert!(limiter.check_and_increment("key").is_ok());
        std::thread::sleep(Duration::from_millis(100));
        // Window expired → counter reset
        assert!(limiter.check_and_increment("key").is_ok());
    }

    #[test]
    fn expired_window_allows_check() {
        let limiter = RateLimiter::new(1, Duration::from_millis(50));
        limiter.record_attempt("key");
        assert!(limiter.check("key").is_err());
        std::thread::sleep(Duration::from_millis(100));
        // Window expired → check passes
        assert!(limiter.check("key").is_ok());
    }

    #[test]
    fn cleanup_removes_expired_entries() {
        let limiter = RateLimiter::new(10, Duration::from_millis(50));
        limiter.check_and_increment("key").unwrap();
        assert_eq!(limiter.entry_count(), 1);
        std::thread::sleep(Duration::from_millis(100));
        limiter.cleanup();
        assert_eq!(limiter.entry_count(), 0);
    }

    #[test]
    fn cleanup_keeps_active_entries() {
        let limiter = RateLimiter::new(10, Duration::from_secs(60));
        limiter.check_and_increment("key").unwrap();
        limiter.cleanup();
        assert_eq!(limiter.entry_count(), 1);
    }

    #[test]
    fn factory_auth_limit_is_five() {
        let limiter = RateLimiter::auth();
        for _ in 0..5 {
            assert!(limiter.check_and_increment("ip").is_ok());
        }
        assert!(limiter.check_and_increment("ip").is_err());
    }

    #[test]
    fn factory_api_limit_is_sixty() {
        let limiter = RateLimiter::api();
        for _ in 0..60 {
            assert!(limiter.check_and_increment("ip").is_ok());
        }
        assert!(limiter.check_and_increment("ip").is_err());
    }

    #[test]
    fn factory_authenticated_action_limit_is_twenty() {
        let limiter = RateLimiter::authenticated_action();
        for _ in 0..20 {
            assert!(limiter.check_and_increment("user:1").is_ok());
        }
        assert!(limiter.check_and_increment("user:1").is_err());
    }

    // -- rate_limit_ip key selection -----------------------------------------

    fn request_with(
        peer: Option<std::net::SocketAddr>,
        forwarded_for: Option<&str>,
    ) -> Request {
        let mut builder = axum::http::Request::builder().uri("/login");
        if let Some(xff) = forwarded_for {
            builder = builder.header("x-forwarded-for", xff);
        }
        let mut request = builder.body(axum::body::Body::empty()).unwrap();
        if let Some(addr) = peer {
            request.extensions_mut().insert(axum::extract::ConnectInfo(addr));
        }
        request
    }

    #[test]
    fn public_peer_address_wins_over_spoofable_headers() {
        let request = request_with(Some("203.0.113.9:44444".parse().unwrap()), Some("9.9.9.9"));
        assert_eq!(rate_limit_ip(&request), "203.0.113.9");
    }

    #[test]
    fn private_peer_defers_to_forwarded_header() {
        // Docker bridge / reverse proxy: the TCP peer is the proxy, the real
        // client rides in X-Forwarded-For.
        let request = request_with(Some("172.18.0.2:33000".parse().unwrap()), Some("198.51.100.7"));
        assert_eq!(rate_limit_ip(&request), "198.51.100.7");
    }

    #[test]
    fn private_peer_without_header_keys_by_peer_not_unknown() {
        // Direct LAN exposure (compose default, no proxy): each browser must
        // get its own bucket instead of the shared "unknown" key that let one
        // user's failures lock the whole deployment out.
        let request = request_with(Some("192.168.1.50:9000".parse().unwrap()), None);
        assert_eq!(rate_limit_ip(&request), "192.168.1.50");
    }

    #[test]
    fn no_connect_info_falls_back_to_headers() {
        let request = request_with(None, Some("198.51.100.7"));
        assert_eq!(rate_limit_ip(&request), "198.51.100.7");
        let request = request_with(None, None);
        assert_eq!(rate_limit_ip(&request), "unknown");
    }

    // -- auth middleware failure accounting ----------------------------------

    async fn run_auth_middleware(status: axum::http::StatusCode) -> Arc<RateLimiter> {
        use axum::routing::post;
        use tower::ServiceExt;

        let limiter = Arc::new(RateLimiter::auth());
        let app = axum::Router::new()
            .route("/login", post(move || async move { status }))
            .layer(axum::middleware::from_fn_with_state(
                Arc::clone(&limiter),
                auth_rate_limit_middleware,
            ));
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/login")
            .header("x-forwarded-for", "198.51.100.7")
            .body(axum::body::Body::empty())
            .unwrap();
        let _ = app.oneshot(request).await.unwrap();
        limiter
    }

    #[tokio::test]
    async fn auth_middleware_records_definitive_credential_rejections() {
        assert_eq!(run_auth_middleware(axum::http::StatusCode::UNAUTHORIZED).await.entry_count(), 1);
        assert_eq!(run_auth_middleware(axum::http::StatusCode::FORBIDDEN).await.entry_count(), 1);
    }

    #[tokio::test]
    async fn auth_middleware_ignores_non_credential_failures() {
        // 400/409/429/5xx are not evidence of credential guessing; counting
        // them let unrelated errors consume the shared login budget.
        for status in [
            axum::http::StatusCode::BAD_REQUEST,
            axum::http::StatusCode::CONFLICT,
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::http::StatusCode::OK,
        ] {
            assert_eq!(
                run_auth_middleware(status).await.entry_count(),
                0,
                "status {status} must not consume the login budget"
            );
        }
    }

    // -- authenticated_action bucket selection --------------------------------

    async fn run_action(app: &axum::Router, cookie: Option<&str>) -> axum::http::StatusCode {
        use tower::ServiceExt;
        let mut builder = axum::http::Request::builder().method("POST").uri("/action");
        if let Some(cookie) = cookie {
            builder = builder.header(axum::http::header::COOKIE, cookie);
        }
        app.clone()
            .oneshot(builder.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    fn action_app(limiter: Arc<RateLimiter>) -> axum::Router {
        use axum::routing::post;
        axum::Router::new()
            .route("/action", post(|| async { "done" }))
            .layer(axum::middleware::from_fn_with_state(
                limiter,
                authenticated_action_rate_limit_middleware,
            ))
    }

    #[tokio::test]
    async fn action_limiter_buckets_by_session_token_not_shared_account() {
        // WebUI deployments share one admin account across every browser; the
        // quota must therefore follow the session, not the user.
        let app = action_app(Arc::new(RateLimiter::new(1, Duration::from_secs(60))));

        assert_eq!(
            run_action(&app, Some("nomifun-session=session-a")).await,
            axum::http::StatusCode::OK
        );
        // A DIFFERENT session of the same account still has its own budget.
        assert_eq!(
            run_action(&app, Some("nomifun-session=session-b")).await,
            axum::http::StatusCode::OK
        );
        // The same session is limited.
        assert_eq!(
            run_action(&app, Some("nomifun-session=session-a")).await,
            axum::http::StatusCode::TOO_MANY_REQUESTS
        );
    }
}
