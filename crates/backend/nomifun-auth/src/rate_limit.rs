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

/// Client identity for rate limiting. Prefers the real TCP peer address from
/// `ConnectInfo` (set on the desktop LAN listener via
/// `into_make_service_with_connect_info`) over the spoofable
/// `X-Forwarded-For` / `X-Real-IP` headers. Falls back to the header value when
/// connect-info is absent (loopback / standalone-web / tests — byte-identical).
fn rate_limit_ip(request: &Request) -> String {
    if let Some(axum::extract::ConnectInfo(addr)) =
        request.extensions().get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        return addr.ip().to_string();
    }
    extract_client_ip(request)
}

/// Auth rate limit middleware: 5 failed attempts per 15 minutes per IP.
///
/// Pre-checks the limit; records failures only for non-success responses
/// (skips successful requests per API spec).
pub async fn auth_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let ip = rate_limit_ip(&request);
    limiter.check(&ip)?;

    let response = next.run(request).await;

    if !response.status().is_success() {
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
/// Prefers user ID from [`CurrentUser`] extension (set by auth middleware),
/// falls back to client IP.
pub async fn authenticated_action_rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let key = request
        .extensions()
        .get::<CurrentUser>()
        .map(|u| format!("user:{}", u.id))
        .unwrap_or_else(|| format!("ip:{}", rate_limit_ip(&request)));
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
}
