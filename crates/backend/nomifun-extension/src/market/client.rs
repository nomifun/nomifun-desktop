//! Allowlist-guarded HTTP client and size-capped body readers for the skill
//! market. All market fetches go through [`build_market_client`], whose
//! custom redirect policy only follows redirects onto known market hosts —
//! a redirect to anything else (cloud metadata endpoints, loopback, RFC1918
//! hosts, arbitrary third parties) is rejected outright.

use std::time::Duration;

use nomifun_common::AppError;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue};

/// Ranking/readme/detail bodies larger than this are rejected.
pub(crate) const MAX_MARKET_BODY_BYTES: u64 = 8 * 1024 * 1024;
/// SkillHub skill zip archives larger than this are rejected.
pub(crate) const MAX_SKILLHUB_SKILL_ZIP_BYTES: u64 = 32 * 1024 * 1024;
/// Per-request timeout. The outer per-source budget
/// ([`super::MARKET_SOURCE_TIMEOUT`]) covers a primary + fallback pair.
const MARKET_REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
/// Redirect hop cap enforced by the custom policy.
const MAX_MARKET_REDIRECT_HOPS: usize = 5;

/// Hosts market requests may land on, including via redirects. Everything
/// else — notably internal addresses like `169.254.169.254`, `127.0.0.1`,
/// or RFC1918 hosts, which can never appear here — is refused.
const MARKET_ALLOWED_HOSTS: &[&str] = &[
    "clawhub.ai",
    "api.skillhub.cn",
    "skillhub.cn",
    "www.skills.sh",
    "skills.sh",
    "api.cocoloop.cn",
    "hub.cocoloop.cn",
    "dl.cocoloop.cn",
    // SkillHub's official download endpoint redirects child-skill archives
    // to this exact Tencent COS acceleration host.
    "skillhub-1388575217.cos.accelerate.myqcloud.com",
    "wry-manatee-359.convex.cloud",
    "www.mcpworld.com",
];

/// SSRF redirect guard predicate: is `host` an exact (case-insensitive)
/// match for an allowlisted market host?
fn is_allowed_market_host(host: &str) -> bool {
    MARKET_ALLOWED_HOSTS.iter().any(|allowed| host.eq_ignore_ascii_case(allowed))
}

/// Build the shared market HTTP client. Redirects are only followed when the
/// target host passes [`is_allowed_market_host`], capped at
/// [`MAX_MARKET_REDIRECT_HOPS`] hops; off-allowlist redirect targets fail the
/// request instead of being fetched.
pub(crate) fn build_market_client() -> Result<reqwest::Client, AppError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8"),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"));

    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_MARKET_REDIRECT_HOPS {
            return attempt.error("too many market redirects");
        }
        if attempt.url().host_str().is_some_and(is_allowed_market_host) {
            attempt.follow()
        } else {
            attempt.error("market redirect target host is not allowlisted")
        }
    });

    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(redirect_policy)
        .timeout(MARKET_REQUEST_TIMEOUT)
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 NomiFun-SkillMarket/1.0",
        )
        .build()
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// GET `url` and return its body as text, capped at [`MAX_MARKET_BODY_BYTES`].
pub(crate) async fn read_market_body(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let mut response = client.get(url).send().await.map_err(map_market_fetch_error)?;
    read_market_response(&mut response).await
}

/// GET `url` with a caller-specific deadline. This is reserved for known
/// large market payloads; ordinary detail and archive requests keep the
/// tighter client-wide timeout.
pub(crate) async fn read_market_body_with_timeout(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<String, AppError> {
    let mut response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(map_market_fetch_error)?;
    read_market_response(&mut response).await
}

/// GET a detail resource while preserving a real upstream 404 as
/// [`AppError::NotFound`]. Ranking pages use [`read_market_body`] because a
/// missing market page is an upstream integration failure, whereas a missing
/// detail resource means the selected entry was removed or became stale.
pub(crate) async fn read_market_detail_body(
    client: &reqwest::Client,
    url: &str,
    label: &str,
) -> Result<String, AppError> {
    let mut response = client.get(url).send().await.map_err(map_market_fetch_error)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!("{label} not found")));
    }
    read_market_response(&mut response).await
}

/// POST a JSON `body` to `url` and return the response body as text, capped
/// at [`MAX_MARKET_BODY_BYTES`].
pub(crate) async fn read_market_json_post(
    client: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> Result<String, AppError> {
    let mut response = client.post(url).json(&body).send().await.map_err(map_market_fetch_error)?;
    read_market_response(&mut response).await
}

async fn read_market_response(response: &mut reqwest::Response) -> Result<String, AppError> {
    if !response.status().is_success() {
        return Err(AppError::BadGateway(format!("market page returned {}", response.status())));
    }
    if response.content_length().unwrap_or(0) > MAX_MARKET_BODY_BYTES {
        return Err(AppError::BadGateway("market response is too large".into()));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_market_fetch_error)? {
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_MARKET_BODY_BYTES {
            return Err(AppError::BadGateway("market response is too large".into()));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Drain a binary response (e.g. a skill zip) up to `max_bytes`, mapping 404
/// to [`AppError::NotFound`] so callers can fall back to a search.
pub(crate) async fn read_market_bytes(
    response: &mut reqwest::Response,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(format!("{label} not found")));
    }
    if !status.is_success() {
        return Err(AppError::BadGateway(format!("{label} returned {status}")));
    }
    if response.content_length().unwrap_or(0) > max_bytes {
        return Err(AppError::BadGateway(format!("{label} is too large")));
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_market_fetch_error)? {
        if bytes.len().saturating_add(chunk.len()) as u64 > max_bytes {
            return Err(AppError::BadGateway(format!("{label} is too large")));
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(bytes)
}

pub(crate) fn map_market_fetch_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::Timeout(format!("skill market fetch timed out: {error}"))
    } else {
        AppError::BadGateway(format!("skill market fetch failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn spawn_test_response(
        status: &'static str,
        body: &'static str,
        body_delay: Duration,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(headers.as_bytes()).await;
            tokio::time::sleep(body_delay).await;
            let _ = socket.write_all(body.as_bytes()).await;
        });
        (format!("http://{address}/"), server)
    }

    #[test]
    fn market_host_allowlist_accepts_known_hosts() {
        for host in MARKET_ALLOWED_HOSTS {
            assert!(is_allowed_market_host(host), "{host}");
        }
        // Case-insensitive.
        assert!(is_allowed_market_host("ClawHub.AI"));
    }

    /// SSRF redirect guard: internal addresses and off-allowlist hosts can
    /// never satisfy the redirect policy's host predicate.
    #[test]
    fn market_host_allowlist_rejects_internal_and_foreign_redirect_targets() {
        // Redirect-to-internal pivots (cloud metadata, loopback, RFC1918).
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8080/",
            "http://10.0.0.5/",
            "http://192.168.1.1/admin",
            "http://[::1]/",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap();
            let host = parsed.host_str().expect("test URL has a host");
            assert!(!is_allowed_market_host(host), "{url} must be rejected");
        }

        // Off-allowlist public hosts are rejected too.
        for host in [
            "evil.example.com",
            "clawhub.ai.evil.com",
            "sub.skillhub.cn",
            "other-bucket.cos.accelerate.myqcloud.com",
            "example.com",
        ] {
            assert!(!is_allowed_market_host(host), "{host} must be rejected");
        }
    }

    #[test]
    fn build_market_client_succeeds() {
        assert!(build_market_client().is_ok());
    }

    #[tokio::test]
    async fn request_timeout_override_covers_delayed_response_body() {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();

        let (default_url, default_server) =
            spawn_test_response("200 OK", "{}", Duration::from_millis(150)).await;
        let error = read_market_body(&client, &default_url).await.unwrap_err();
        assert!(matches!(&error, AppError::Timeout(_)), "{error}");
        default_server.await.unwrap();

        let (override_url, override_server) =
            spawn_test_response("200 OK", "{}", Duration::from_millis(150)).await;
        let body = read_market_body_with_timeout(&client, &override_url, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(body, "{}");
        override_server.await.unwrap();
    }

    #[tokio::test]
    async fn detail_reader_preserves_not_found_and_rejects_other_errors() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let (not_found_url, not_found_server) =
            spawn_test_response("404 Not Found", "missing", Duration::ZERO).await;
        let error = read_market_detail_body(&client, &not_found_url, "test package")
            .await
            .unwrap_err();
        assert!(matches!(&error, AppError::NotFound(_)), "{error}");
        not_found_server.await.unwrap();

        let (server_error_url, server_error) =
            spawn_test_response("500 Internal Server Error", "failed", Duration::ZERO).await;
        let error = read_market_detail_body(&client, &server_error_url, "test package")
            .await
            .unwrap_err();
        assert!(matches!(&error, AppError::BadGateway(_)), "{error}");
        server_error.await.unwrap();
    }
}
