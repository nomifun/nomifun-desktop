use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

const DEFAULT_TIMEOUT_MS: u64 = 1000;
const CACHE_HIT_TTL: Duration = Duration::from_secs(20);
const CACHE_MISS_TTL: Duration = Duration::from_millis(1500);
const MAX_CONCURRENT_WORKERS: usize = 6;
const SCAN_RADIUS: u16 = 24;
const KNOWN_PORTS: [u16; 2] = [19000, 18791];

const STATUS_MARKERS: [&str; 6] = ["idle", "writing", "researching", "executing", "syncing", "error"];
const FEATURE_KEYWORDS: [&str; 3] = ["star office", "decorate room", "asset sidebar"];
const EXCLUDE_KEYWORDS: [&str; 1] = ["openclaw control"];

struct DetectCache {
    url: Option<String>,
    cached_at: Instant,
    was_hit: bool,
}

pub struct StarOfficeDetector {
    cache: Mutex<Option<DetectCache>>,
    client: reqwest::Client,
}

impl StarOfficeDetector {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            cache: Mutex::new(None),
            client,
        }
    }

    pub async fn detect(&self, preferred_url: Option<&str>, force: bool, timeout_ms: Option<u64>) -> Option<String> {
        self.detect_inner(preferred_url, force, timeout_ms, true).await
    }

    /// Probe only `preferred_url` without expanding to `KNOWN_PORTS` or the
    /// `±SCAN_RADIUS` neighborhood. Exists for deterministic tests that need
    /// to pin detection to a specific mock server; production callers should
    /// use [`detect`].
    pub async fn detect_exact(
        &self,
        preferred_url: Option<&str>,
        force: bool,
        timeout_ms: Option<u64>,
    ) -> Option<String> {
        self.detect_inner(preferred_url, force, timeout_ms, false).await
    }

    async fn detect_inner(
        &self,
        preferred_url: Option<&str>,
        force: bool,
        timeout_ms: Option<u64>,
        scan_neighbors: bool,
    ) -> Option<String> {
        if !force {
            let cache = self.cache.lock().await;
            if let Some(ref c) = *cache {
                let ttl = if c.was_hit { CACHE_HIT_TTL } else { CACHE_MISS_TTL };
                if c.cached_at.elapsed() < ttl {
                    tracing::debug!(cached_url = ?c.url, "returning cached star-office result");
                    return c.url.clone();
                }
            }
        }

        let candidates = build_candidate_urls(preferred_url, scan_neighbors);
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));

        tracing::debug!(count = candidates.len(), "scanning star-office candidate URLs");
        let result = self.scan_candidates(&candidates, timeout).await;

        let mut cache = self.cache.lock().await;
        *cache = Some(DetectCache {
            url: result.clone(),
            cached_at: Instant::now(),
            was_hit: result.is_some(),
        });

        result
    }

    async fn scan_candidates(&self, candidates: &[String], timeout: Duration) -> Option<String> {
        for chunk in candidates.chunks(MAX_CONCURRENT_WORKERS) {
            let mut set = tokio::task::JoinSet::new();
            for url in chunk {
                let client = self.client.clone();
                let url = url.clone();
                set.spawn(async move {
                    if check_health(&client, &url, timeout).await {
                        Some(url)
                    } else {
                        None
                    }
                });
            }
            while let Some(result) = set.join_next().await {
                if let Ok(Some(url)) = result {
                    return Some(url);
                }
            }
        }
        None
    }
}

fn build_candidate_urls(preferred_url: Option<&str>, scan_neighbors: bool) -> Vec<String> {
    let mut seed_ports: Vec<u16> = Vec::new();

    if let Some(url) = preferred_url
        && let Some(port) = extract_port(url)
    {
        seed_ports.push(port);
    }

    if !scan_neighbors {
        // Exact mode: skip KNOWN_PORTS and the ±SCAN_RADIUS expansion so the
        // detector talks to the caller-supplied URL only.
        return seed_ports.iter().map(|p| format!("http://localhost:{p}")).collect();
    }

    for p in KNOWN_PORTS {
        if !seed_ports.contains(&p) {
            seed_ports.push(p);
        }
    }

    let mut expanded: Vec<u16> = Vec::new();
    for &base in &seed_ports {
        let start = base.saturating_sub(SCAN_RADIUS);
        let end = base.saturating_add(SCAN_RADIUS);
        for p in start..=end {
            if p > 0 && !expanded.contains(&p) {
                expanded.push(p);
            }
        }
    }

    let mut urls = Vec::with_capacity(expanded.len());
    for &p in &seed_ports {
        urls.push(format!("http://localhost:{p}"));
    }
    for &p in &expanded {
        if !seed_ports.contains(&p) {
            urls.push(format!("http://localhost:{p}"));
        }
    }

    urls
}

fn extract_port(url: &str) -> Option<u16> {
    let without_scheme = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let host_part = without_scheme.split('/').next()?;
    let port_str = host_part.rsplit(':').next()?;
    port_str.parse().ok()
}

async fn check_health(client: &reqwest::Client, base_url: &str, timeout: Duration) -> bool {
    let health_url = format!("{base_url}/health");
    let resp = match client.get(&health_url).timeout(timeout).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };
    if !resp.status().is_success() {
        return false;
    }

    let status_url = format!("{base_url}/status");
    let resp = match client.get(&status_url).timeout(timeout).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };
    let body = match resp.text().await {
        Ok(t) => t.to_lowercase(),
        Err(_) => return false,
    };
    if !STATUS_MARKERS.iter().any(|m| body.contains(m)) {
        return false;
    }

    let resp = match client.get(base_url).timeout(timeout).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };
    let body = match resp.text().await {
        Ok(t) => t.to_lowercase(),
        Err(_) => return false,
    };

    FEATURE_KEYWORDS.iter().any(|k| body.contains(k)) && !EXCLUDE_KEYWORDS.iter().any(|k| body.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_port_http() {
        assert_eq!(extract_port("http://localhost:19000"), Some(19000));
    }

    #[test]
    fn extract_port_https() {
        assert_eq!(extract_port("https://localhost:8443"), Some(8443));
    }

    #[test]
    fn extract_port_with_path() {
        assert_eq!(extract_port("http://localhost:19000/star"), Some(19000));
    }

    #[test]
    fn extract_port_no_scheme() {
        assert_eq!(extract_port("localhost:19000"), None);
    }

    #[test]
    fn extract_port_no_port() {
        assert_eq!(extract_port("http://localhost"), None);
    }

    #[test]
    fn extract_port_invalid_port() {
        assert_eq!(extract_port("http://localhost:abc"), None);
    }

    #[test]
    fn extract_port_ipv4() {
        assert_eq!(extract_port("http://127.0.0.1:18791"), Some(18791));
    }

    #[test]
    fn build_candidates_default_ports() {
        let urls = build_candidate_urls(None, true);
        assert!(urls[0] == "http://localhost:19000");
        assert!(urls[1] == "http://localhost:18791");
        let expected_count = count_unique_expanded_ports(&[19000, 18791]);
        assert_eq!(urls.len(), expected_count);
    }

    #[test]
    fn build_candidates_preferred_first() {
        let urls = build_candidate_urls(Some("http://localhost:15000"), true);
        assert_eq!(urls[0], "http://localhost:15000");
        assert_eq!(urls[1], "http://localhost:19000");
        assert_eq!(urls[2], "http://localhost:18791");
    }

    #[test]
    fn build_candidates_preferred_overlaps_known() {
        let urls = build_candidate_urls(Some("http://localhost:19000"), true);
        assert_eq!(urls[0], "http://localhost:19000");
        assert_eq!(urls[1], "http://localhost:18791");
    }

    #[test]
    fn build_candidates_no_duplicates() {
        let urls = build_candidate_urls(Some("http://localhost:18800"), true);
        let unique: std::collections::HashSet<_> = urls.iter().collect();
        assert_eq!(urls.len(), unique.len());
    }

    #[test]
    fn build_candidates_scan_radius_coverage() {
        let urls = build_candidate_urls(None, true);
        assert!(urls.contains(&"http://localhost:18976".to_string()));
        assert!(urls.contains(&"http://localhost:19024".to_string()));
        assert!(urls.contains(&"http://localhost:18767".to_string()));
        assert!(urls.contains(&"http://localhost:18815".to_string()));
    }

    #[test]
    fn build_candidates_preferred_without_port_ignored() {
        let urls = build_candidate_urls(Some("http://localhost"), true);
        assert_eq!(urls[0], "http://localhost:19000");
    }

    #[test]
    fn build_candidates_low_port_no_underflow() {
        let urls = build_candidate_urls(Some("http://localhost:10"), true);
        assert!(urls.iter().all(|u| {
            let p = extract_port(u).unwrap();
            p > 0
        }));
    }

    #[test]
    fn build_candidates_exact_mode_only_preferred() {
        let urls = build_candidate_urls(Some("http://localhost:55555"), false);
        assert_eq!(urls, vec!["http://localhost:55555"]);
    }

    #[test]
    fn build_candidates_exact_mode_empty_when_no_preferred() {
        assert!(build_candidate_urls(None, false).is_empty());
    }

    fn count_unique_expanded_ports(seeds: &[u16]) -> usize {
        let mut all = std::collections::HashSet::new();
        for &base in seeds {
            let start = base.saturating_sub(SCAN_RADIUS);
            let end = base.saturating_add(SCAN_RADIUS);
            for p in start..=end {
                if p > 0 {
                    all.insert(p);
                }
            }
        }
        all.len()
    }

    #[test]
    fn cache_ttl_constants() {
        assert_eq!(CACHE_HIT_TTL, Duration::from_secs(20));
        assert_eq!(CACHE_MISS_TTL, Duration::from_millis(1500));
    }

    #[test]
    fn default_timeout_constant() {
        assert_eq!(DEFAULT_TIMEOUT_MS, 1000);
    }

    #[test]
    fn max_concurrent_workers_constant() {
        assert_eq!(MAX_CONCURRENT_WORKERS, 6);
    }

    #[test]
    fn scan_radius_constant() {
        assert_eq!(SCAN_RADIUS, 24);
    }

    #[test]
    fn status_markers_all_present() {
        let expected = ["idle", "writing", "researching", "executing", "syncing", "error"];
        assert_eq!(STATUS_MARKERS, expected);
    }

    #[test]
    fn feature_keywords_all_present() {
        let expected = ["star office", "decorate room", "asset sidebar"];
        assert_eq!(FEATURE_KEYWORDS, expected);
    }

    #[test]
    fn exclude_keywords_contains_openclaw() {
        assert_eq!(EXCLUDE_KEYWORDS, ["openclaw control"]);
    }

    #[tokio::test]
    async fn detect_no_service_returns_none() {
        let detector = StarOfficeDetector::new(reqwest::Client::new());
        let result = detector
            .detect_exact(Some("http://localhost:59999"), false, Some(50))
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_cache_miss_stored() {
        let detector = StarOfficeDetector::new(reqwest::Client::new());
        let _ = detector
            .detect_exact(Some("http://localhost:59998"), false, Some(50))
            .await;
        let cache = detector.cache.lock().await;
        let c = cache.as_ref().unwrap();
        assert!(c.url.is_none());
        assert!(!c.was_hit);
    }
}
