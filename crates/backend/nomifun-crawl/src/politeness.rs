//! Per-host rate limiting, robots.txt, backoff and circuit breaking.
//!
//! Everything here is keyed by host and driven by an injectable clock, so the
//! tests exercise real elapsed-time logic without sleeping.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use nomifun_common::{TimestampMs, now_ms};
use nomifun_knowledge::source_url::{HttpFetcher, Validators};
use robotxt::{AccessResult, Robots};
use url::Url;

use crate::error::CrawlError;

/// How long a fetched robots.txt stays authoritative.
pub const ROBOTS_TTL_MS: i64 = 30 * 60 * 1000;
/// Consecutive hard failures before a host is cut off.
pub const CIRCUIT_THRESHOLD: u32 = 5;
/// First backoff step; doubles per consecutive throttle up to the ceiling.
pub const BACKOFF_BASE_MS: i64 = 1_000;
pub const BACKOFF_MAX_MS: i64 = 5 * 60 * 1000;
/// Upper bound on a `Retry-After` we are willing to honour inline.
pub const RETRY_AFTER_MAX_MS: i64 = 10 * 60 * 1000;

/// Injectable clock. Politeness is entirely about elapsed time, so the tests
/// would otherwise have to sleep through every backoff step.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> TimestampMs;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> TimestampMs {
        now_ms()
    }
}

/// What a worker may do with a URL right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Proceed after waiting this long (zero when the slot is already free).
    Wait(Duration),
    /// robots.txt forbids this path for our agent.
    RobotsDenied,
    /// The host is cut off; retry after this long.
    CircuitOpen(Duration),
}

/// Where robots.txt content comes from. The HTTP implementation rides the same
/// SSRF-guarded fetcher as page retrieval; tests supply canned bodies.
#[async_trait::async_trait]
pub trait RobotsSource: Send + Sync {
    async fn fetch(&self, robots_url: &str) -> RobotsFetch;
}

/// Robots retrieval result, mapped onto RFC 9309's status classes.
#[derive(Debug, Clone)]
pub enum RobotsFetch {
    Body(Vec<u8>),
    /// 4xx — the file is absent, so everything is permitted.
    Unavailable,
    /// 5xx / network error — the file is undefined, so nothing is permitted.
    Unreachable,
}

pub struct HttpRobotsSource {
    fetcher: HttpFetcher,
}

impl HttpRobotsSource {
    pub fn new(fetcher: HttpFetcher) -> Self {
        Self { fetcher }
    }
}

#[async_trait::async_trait]
impl RobotsSource for HttpRobotsSource {
    async fn fetch(&self, robots_url: &str) -> RobotsFetch {
        match self.fetcher.fetch_raw(robots_url, &Validators::default()).await {
            Ok(page) if page.is_success() => RobotsFetch::Body(page.body),
            // 404 is the common case and means "crawl freely"; conflating it
            // with a 5xx would make the crawler refuse most of the web.
            Ok(page) if (400..500).contains(&page.status) => RobotsFetch::Unavailable,
            Ok(_) => RobotsFetch::Unreachable,
            Err(_) => RobotsFetch::Unreachable,
        }
    }
}

struct HostState {
    /// Earliest time the next request to this host may start.
    next_slot_at: TimestampMs,
    consecutive_failures: u32,
    /// Consecutive throttles (429/503), which drive the exponential backoff.
    throttle_streak: u32,
    circuit_open_until: Option<TimestampMs>,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            next_slot_at: 0,
            consecutive_failures: 0,
            throttle_streak: 0,
            circuit_open_until: None,
        }
    }
}

struct CachedRobots {
    robots: Robots,
    fetched_at: TimestampMs,
}

/// Per-job politeness state.
pub struct Politeness {
    clock: Arc<dyn Clock>,
    source: Arc<dyn RobotsSource>,
    user_agent: String,
    respect_robots: bool,
    /// Floor on the gap between two requests to the same host.
    base_delay: Duration,
    hosts: Mutex<HashMap<String, HostState>>,
    robots: tokio::sync::Mutex<HashMap<String, CachedRobots>>,
}

impl Politeness {
    pub fn new(
        source: Arc<dyn RobotsSource>,
        user_agent: impl Into<String>,
        respect_robots: bool,
        base_delay: Duration,
    ) -> Self {
        Self::with_clock(source, user_agent, respect_robots, base_delay, Arc::new(SystemClock))
    }

    pub fn with_clock(
        source: Arc<dyn RobotsSource>,
        user_agent: impl Into<String>,
        respect_robots: bool,
        base_delay: Duration,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            clock,
            source,
            user_agent: user_agent.into(),
            respect_robots,
            base_delay,
            hosts: Mutex::new(HashMap::new()),
            robots: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Decide whether `url` may be fetched now, and reserve the host slot when
    /// it may. Reserving inside the check is what keeps two workers on the same
    /// host from both concluding the slot is free.
    pub async fn acquire(&self, url: &Url) -> Result<Verdict, CrawlError> {
        let host = url
            .host_str()
            .ok_or_else(|| CrawlError::UrlRejected(format!("no host in {url}")))?
            .to_ascii_lowercase();

        if let Some(open) = self.circuit_state(&host) {
            return Ok(Verdict::CircuitOpen(open));
        }

        let crawl_delay = if self.respect_robots {
            let robots = self.robots_for(url).await;
            if !robots.allowed(url) {
                return Ok(Verdict::RobotsDenied);
            }
            robots.crawl_delay
        } else {
            None
        };

        // robots' crawl-delay outranks our own floor when it is stricter.
        let delay = crawl_delay.unwrap_or(self.base_delay).max(self.base_delay);
        Ok(Verdict::Wait(self.reserve_slot(&host, delay)))
    }

    fn circuit_state(&self, host: &str) -> Option<Duration> {
        let now = self.clock.now_ms();
        let mut hosts = self.hosts.lock().expect("politeness host state poisoned");
        let state = hosts.entry(host.to_string()).or_default();
        match state.circuit_open_until {
            Some(until) if until > now => Some(Duration::from_millis((until - now).max(0) as u64)),
            Some(_) => {
                // Cool-off elapsed: let one request through to probe the host.
                state.circuit_open_until = None;
                state.consecutive_failures = 0;
                None
            }
            None => None,
        }
    }

    fn reserve_slot(&self, host: &str, delay: Duration) -> Duration {
        let now = self.clock.now_ms();
        let mut hosts = self.hosts.lock().expect("politeness host state poisoned");
        let state = hosts.entry(host.to_string()).or_default();
        let start_at = state.next_slot_at.max(now);
        state.next_slot_at = start_at + delay.as_millis() as i64;
        Duration::from_millis((start_at - now).max(0) as u64)
    }

    /// Record a completed request so backoff and the breaker can react.
    /// `retry_after_ms` comes from the `Retry-After` header when present.
    pub fn note_response(&self, host: &str, status: u16, retry_after_ms: Option<i64>) {
        let now = self.clock.now_ms();
        let mut hosts = self.hosts.lock().expect("politeness host state poisoned");
        let state = hosts.entry(host.to_ascii_lowercase()).or_default();

        if status == 429 || status == 503 {
            state.throttle_streak = state.throttle_streak.saturating_add(1);
            let backoff = retry_after_ms
                .map(|ms| ms.clamp(0, RETRY_AFTER_MAX_MS))
                .unwrap_or_else(|| exponential_backoff(state.throttle_streak));
            state.next_slot_at = state.next_slot_at.max(now + backoff);
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        } else if (500..600).contains(&status) {
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        } else {
            state.consecutive_failures = 0;
            state.throttle_streak = 0;
        }

        if state.consecutive_failures >= CIRCUIT_THRESHOLD {
            state.circuit_open_until = Some(now + BACKOFF_MAX_MS);
        }
    }

    /// Record a transport-level failure (DNS, TLS, timeout), which carries no
    /// status but still counts toward the breaker.
    pub fn note_transport_failure(&self, host: &str) {
        let now = self.clock.now_ms();
        let mut hosts = self.hosts.lock().expect("politeness host state poisoned");
        let state = hosts.entry(host.to_ascii_lowercase()).or_default();
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.consecutive_failures >= CIRCUIT_THRESHOLD {
            state.circuit_open_until = Some(now + BACKOFF_MAX_MS);
        }
    }

    async fn robots_for(&self, url: &Url) -> RobotsDecision {
        let key = format!(
            "{}://{}",
            url.scheme(),
            url.host_str().unwrap_or_default()
        );
        let now = self.clock.now_ms();

        let mut cache = self.robots.lock().await;
        if let Some(entry) = cache.get(&key)
            && now - entry.fetched_at < ROBOTS_TTL_MS
        {
            return RobotsDecision::from(&entry.robots, url);
        }

        let robots_url = match url.join("/robots.txt") {
            Ok(u) => u.to_string(),
            // A URL that cannot host a /robots.txt is not fetchable anyway.
            Err(_) => return RobotsDecision { allowed: false, crawl_delay: None },
        };
        let fetched = self.source.fetch(&robots_url).await;
        let access = match &fetched {
            RobotsFetch::Body(body) => AccessResult::Successful(body),
            RobotsFetch::Unavailable => AccessResult::Unavailable,
            RobotsFetch::Unreachable => AccessResult::Unreachable,
        };
        let parsed = Robots::from_access(access, &self.user_agent);
        let decision = RobotsDecision::from(&parsed, url);
        cache.insert(key, CachedRobots { robots: parsed, fetched_at: now });
        decision
    }
}

struct RobotsDecision {
    allowed: bool,
    crawl_delay: Option<Duration>,
}

impl RobotsDecision {
    fn from(robots: &Robots, url: &Url) -> Self {
        Self {
            allowed: robots.is_absolute_allowed(url),
            crawl_delay: robots.crawl_delay(),
        }
    }

    fn allowed(&self, _url: &Url) -> bool {
        self.allowed
    }
}

fn exponential_backoff(streak: u32) -> i64 {
    let shift = streak.saturating_sub(1).min(16);
    BACKOFF_BASE_MS.saturating_mul(1_i64 << shift).min(BACKOFF_MAX_MS)
}

/// Parse `Retry-After` in its delta-seconds form. The HTTP-date form is
/// deliberately ignored: without a trusted clock offset it is easy to turn a
/// skewed server date into a multi-hour stall.
pub fn parse_retry_after(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok().map(|secs| secs.max(0) * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);

    impl TestClock {
        fn new() -> Arc<Self> {
            Arc::new(Self(AtomicI64::new(1_000_000)))
        }
        fn advance(&self, ms: i64) {
            self.0.fetch_add(ms, Ordering::SeqCst);
        }
    }

    impl Clock for TestClock {
        fn now_ms(&self) -> TimestampMs {
            self.0.load(Ordering::SeqCst)
        }
    }

    struct CannedRobots(RobotsFetch);

    #[async_trait::async_trait]
    impl RobotsSource for CannedRobots {
        async fn fetch(&self, _robots_url: &str) -> RobotsFetch {
            self.0.clone()
        }
    }

    fn politeness(fetch: RobotsFetch, respect: bool, delay_ms: u64) -> (Politeness, Arc<TestClock>) {
        let clock = TestClock::new();
        let p = Politeness::with_clock(
            Arc::new(CannedRobots(fetch)),
            "NomiFun-Crawler/1.0",
            respect,
            Duration::from_millis(delay_ms),
            clock.clone(),
        );
        (p, clock)
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[tokio::test]
    async fn first_request_to_a_host_is_immediate() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 500);
        assert_eq!(
            p.acquire(&url("https://a.com/x")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn consecutive_requests_are_spaced_by_the_delay() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 500);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        assert_eq!(
            p.acquire(&url("https://a.com/2")).await.unwrap(),
            Verdict::Wait(Duration::from_millis(500))
        );
        assert_eq!(
            p.acquire(&url("https://a.com/3")).await.unwrap(),
            Verdict::Wait(Duration::from_millis(1000))
        );
    }

    #[tokio::test]
    async fn spacing_is_per_host_not_global() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 500);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        assert_eq!(
            p.acquire(&url("https://b.com/1")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn waiting_out_the_delay_frees_the_slot() {
        let (p, clock) = politeness(RobotsFetch::Unavailable, true, 500);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        clock.advance(600);
        assert_eq!(
            p.acquire(&url("https://a.com/2")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn missing_robots_allows_everything() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 0);
        assert_eq!(
            p.acquire(&url("https://a.com/anything")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn unreachable_robots_denies_everything() {
        let (p, _c) = politeness(RobotsFetch::Unreachable, true, 0);
        assert_eq!(
            p.acquire(&url("https://a.com/anything")).await.unwrap(),
            Verdict::RobotsDenied
        );
    }

    #[tokio::test]
    async fn disallowed_path_is_denied_and_sibling_is_allowed() {
        let body = b"User-agent: *\nDisallow: /private\n".to_vec();
        let (p, _c) = politeness(RobotsFetch::Body(body), true, 0);
        assert_eq!(
            p.acquire(&url("https://a.com/private/x")).await.unwrap(),
            Verdict::RobotsDenied
        );
        assert_eq!(
            p.acquire(&url("https://a.com/public/x")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn robots_can_be_switched_off() {
        let body = b"User-agent: *\nDisallow: /\n".to_vec();
        let (p, _c) = politeness(RobotsFetch::Body(body), false, 0);
        assert_eq!(
            p.acquire(&url("https://a.com/private")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn crawl_delay_overrides_a_smaller_base_delay() {
        let body = b"User-agent: *\nCrawl-delay: 2\n".to_vec();
        let (p, _c) = politeness(RobotsFetch::Body(body), true, 100);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        assert_eq!(
            p.acquire(&url("https://a.com/2")).await.unwrap(),
            Verdict::Wait(Duration::from_secs(2))
        );
    }

    #[tokio::test]
    async fn base_delay_wins_when_it_is_the_stricter_of_the_two() {
        let body = b"User-agent: *\nCrawl-delay: 1\n".to_vec();
        let (p, _c) = politeness(RobotsFetch::Body(body), true, 5_000);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        assert_eq!(
            p.acquire(&url("https://a.com/2")).await.unwrap(),
            Verdict::Wait(Duration::from_millis(5_000))
        );
    }

    #[tokio::test]
    async fn retry_after_pushes_the_next_slot_out() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 0);
        p.acquire(&url("https://a.com/1")).await.unwrap();
        p.note_response("a.com", 429, Some(30_000));
        assert_eq!(
            p.acquire(&url("https://a.com/2")).await.unwrap(),
            Verdict::Wait(Duration::from_millis(30_000))
        );
    }

    #[tokio::test]
    async fn throttling_without_retry_after_backs_off_exponentially() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 0);
        p.note_response("a.com", 429, None);
        let first = p.acquire(&url("https://a.com/1")).await.unwrap();
        p.note_response("a.com", 429, None);
        let second = p.acquire(&url("https://a.com/2")).await.unwrap();
        assert_eq!(first, Verdict::Wait(Duration::from_millis(1_000)));
        assert_eq!(second, Verdict::Wait(Duration::from_millis(2_000)));
    }

    #[tokio::test]
    async fn a_success_clears_the_throttle_streak() {
        let (p, clock) = politeness(RobotsFetch::Unavailable, true, 0);
        p.note_response("a.com", 429, None);
        p.note_response("a.com", 200, None);
        clock.advance(BACKOFF_MAX_MS);
        p.note_response("a.com", 429, None);
        assert_eq!(
            p.acquire(&url("https://a.com/1")).await.unwrap(),
            Verdict::Wait(Duration::from_millis(BACKOFF_BASE_MS as u64))
        );
    }

    #[tokio::test]
    async fn repeated_failures_open_the_circuit() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 0);
        for _ in 0..CIRCUIT_THRESHOLD {
            p.note_response("a.com", 500, None);
        }
        assert!(matches!(
            p.acquire(&url("https://a.com/1")).await.unwrap(),
            Verdict::CircuitOpen(_)
        ));
    }

    #[tokio::test]
    async fn an_open_circuit_does_not_block_other_hosts() {
        let (p, _c) = politeness(RobotsFetch::Unavailable, true, 0);
        for _ in 0..CIRCUIT_THRESHOLD {
            p.note_response("a.com", 500, None);
        }
        assert_eq!(
            p.acquire(&url("https://b.com/1")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[tokio::test]
    async fn the_circuit_closes_after_the_cool_off() {
        let (p, clock) = politeness(RobotsFetch::Unavailable, true, 0);
        for _ in 0..CIRCUIT_THRESHOLD {
            p.note_transport_failure("a.com");
        }
        assert!(matches!(
            p.acquire(&url("https://a.com/1")).await.unwrap(),
            Verdict::CircuitOpen(_)
        ));
        clock.advance(BACKOFF_MAX_MS + 1);
        assert_eq!(
            p.acquire(&url("https://a.com/1")).await.unwrap(),
            Verdict::Wait(Duration::ZERO)
        );
    }

    #[test]
    fn retry_after_seconds_parse_and_garbage_does_not() {
        assert_eq!(parse_retry_after("120"), Some(120_000));
        assert_eq!(parse_retry_after(" 5 "), Some(5_000));
        assert_eq!(parse_retry_after("-3"), Some(0));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }
}
