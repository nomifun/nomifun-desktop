//! SSRF-safe HTTP GETs for untrusted URLs.
//!
//! Every redirect hop is parsed and resolved before a socket is opened. All
//! resolved addresses must be public, and the validated addresses are pinned
//! into a fresh, proxy-free reqwest client for that hop. This makes URL
//! validation and the connection use the same DNS answer.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::header::{HeaderMap, LOCATION};
use url::{Host, Url};

/// Why an untrusted outbound request was rejected or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeHttpErrorKind {
    InvalidUrl,
    ForbiddenTarget,
    Dns,
    Timeout,
    Network,
    InvalidRedirect,
    TooManyRedirects,
    BodyTooLarge,
    BodyRead,
    ClientBuild,
}

/// Error returned by [`SafeHttpClient`].
#[derive(Debug)]
pub struct SafeHttpError {
    kind: SafeHttpErrorKind,
    message: String,
}

impl SafeHttpError {
    fn new(kind: SafeHttpErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> SafeHttpErrorKind {
        self.kind
    }
}

impl fmt::Display for SafeHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SafeHttpError {}

/// How a body that crosses the configured byte limit is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyOverflowPolicy {
    Reject,
    Truncate,
}

/// A final (non-redirect) response with a bounded body.
#[derive(Debug)]
pub struct SafeHttpResponse {
    pub final_url: Url,
    pub status: reqwest::StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub truncated: bool,
}

/// Policy-enforcing GET client for URLs supplied by providers or users.
#[derive(Debug, Clone)]
pub struct SafeHttpClient {
    timeout: Duration,
    max_body_bytes: usize,
    max_redirects: usize,
    overflow: BodyOverflowPolicy,
    allow_private: bool,
    user_agent: String,
}

impl SafeHttpClient {
    pub fn new(timeout: Duration, max_body_bytes: usize) -> Self {
        Self {
            timeout,
            max_body_bytes,
            max_redirects: 3,
            overflow: BodyOverflowPolicy::Reject,
            allow_private: false,
            user_agent: "NomiFun-SafeHttp/1.0".to_owned(),
        }
    }

    pub fn max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    pub fn overflow_policy(mut self, overflow: BodyOverflowPolicy) -> Self {
        self.overflow = overflow;
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Permit private targets for loopback mock servers. Production callers
    /// must use the strict default.
    pub fn allow_private_for_tests(mut self) -> Self {
        self.allow_private = true;
        self
    }

    pub async fn get(&self, raw_url: &str) -> Result<SafeHttpResponse, SafeHttpError> {
        let mut url = parse_untrusted_url(raw_url)?;
        for hop in 0..=self.max_redirects {
            let addrs = resolve_validated(&url, self.allow_private).await?;
            let response = self.send(&url, &addrs).await?;
            let status = response.status();

            if status.is_redirection() {
                if hop == self.max_redirects {
                    return Err(SafeHttpError::new(
                        SafeHttpErrorKind::TooManyRedirects,
                        format!("too many redirects fetching {}", redacted_url(&url)),
                    ));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| {
                        SafeHttpError::new(
                            SafeHttpErrorKind::InvalidRedirect,
                            format!(
                                "redirect without a valid Location from {}",
                                redacted_url(&url)
                            ),
                        )
                    })?;
                url = url.join(location).map_err(|error| {
                    SafeHttpError::new(
                        SafeHttpErrorKind::InvalidRedirect,
                        format!(
                            "invalid redirect target from {}: {error}",
                            redacted_url(&url)
                        ),
                    )
                })?;
                url = validate_url(url)?;
                continue;
            }

            if self.overflow == BodyOverflowPolicy::Reject
                && response
                    .content_length()
                    .is_some_and(|length| length > self.max_body_bytes as u64)
            {
                return Err(SafeHttpError::new(
                    SafeHttpErrorKind::BodyTooLarge,
                    format!(
                        "response body exceeds the {} byte limit for {url}",
                        self.max_body_bytes,
                        url = redacted_url(&url)
                    ),
                ));
            }

            let headers = response.headers().clone();
            let (body, truncated) = self.read_body(response, &url).await?;
            return Ok(SafeHttpResponse {
                final_url: url,
                status,
                headers,
                body,
                truncated,
            });
        }
        unreachable!("redirect loop always returns or advances within the bounded range")
    }

    async fn send(
        &self,
        url: &Url,
        addrs: &[SocketAddr],
    ) -> Result<reqwest::Response, SafeHttpError> {
        // A proxy can resolve the target independently and defeat DNS pinning.
        // Untrusted fetches therefore always connect directly.
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout);
        if let Some(host) = url.host_str() {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = builder.build().map_err(|error| {
            SafeHttpError::new(
                SafeHttpErrorKind::ClientBuild,
                format!("failed to build safe HTTP client: {error}"),
            )
        })?;
        client
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .send()
            .await
            .map_err(|error| {
                let error = error.without_url();
                if error.is_timeout() {
                    SafeHttpError::new(
                        SafeHttpErrorKind::Timeout,
                        format!("request timed out for {}", redacted_url(url)),
                    )
                } else {
                    SafeHttpError::new(
                        SafeHttpErrorKind::Network,
                        format!("request failed for {}: {error}", redacted_url(url)),
                    )
                }
            })
    }

    async fn read_body(
        &self,
        mut response: reqwest::Response,
        url: &Url,
    ) -> Result<(Vec<u8>, bool), SafeHttpError> {
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            let error = error.without_url();
            let kind = if error.is_timeout() {
                SafeHttpErrorKind::Timeout
            } else {
                SafeHttpErrorKind::BodyRead
            };
            SafeHttpError::new(
                kind,
                format!(
                    "failed reading response body for {}: {error}",
                    redacted_url(url)
                ),
            )
        })? {
            let Some(next_len) = body.len().checked_add(chunk.len()) else {
                return Err(SafeHttpError::new(
                    SafeHttpErrorKind::BodyTooLarge,
                    format!("response body size overflow for {}", redacted_url(url)),
                ));
            };
            if next_len > self.max_body_bytes {
                if self.overflow == BodyOverflowPolicy::Reject {
                    return Err(SafeHttpError::new(
                        SafeHttpErrorKind::BodyTooLarge,
                        format!(
                            "response body exceeds the {} byte limit for {url}",
                            self.max_body_bytes,
                            url = redacted_url(url)
                        ),
                    ));
                }
                let remaining = self.max_body_bytes.saturating_sub(body.len());
                body.extend_from_slice(&chunk[..remaining]);
                return Ok((body, true));
            }
            body.extend_from_slice(&chunk);
        }
        Ok((body, false))
    }
}

/// Parse an untrusted URL before any DNS or network operation.
pub fn parse_untrusted_url(raw: &str) -> Result<Url, SafeHttpError> {
    let url = Url::parse(raw.trim()).map_err(|error| {
        SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, format!("invalid URL: {error}"))
    })?;
    validate_url(url)
}

/// Display an outbound URL without its query credentials. Fragments are
/// rejected by policy, but are cleared here as defense in depth.
pub fn redacted_url(url: &Url) -> String {
    let mut redacted = url.clone();
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

fn validate_url(url: Url) -> Result<Url, SafeHttpError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::InvalidUrl,
            format!("only http(s) URLs are supported (got scheme: {})", url.scheme()),
        ));
    }
    if url.host_str().is_none() {
        return Err(SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, "URL has no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::InvalidUrl,
            "URL user information is not allowed",
        ));
    }
    if url.fragment().is_some() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::InvalidUrl,
            "URL fragments are not allowed",
        ));
    }
    Ok(url)
}

/// Validate syntax and resolve all addresses without opening a connection.
pub async fn validate_untrusted_url(
    raw: &str,
    allow_private: bool,
) -> Result<Url, SafeHttpError> {
    let url = parse_untrusted_url(raw)?;
    resolve_validated(&url, allow_private).await?;
    Ok(url)
}

async fn resolve_validated(
    url: &Url,
    allow_private: bool,
) -> Result<Vec<SocketAddr>, SafeHttpError> {
    let host = url
        .host_str()
        .ok_or_else(|| SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, "URL has no host"))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, "URL has no usable port")
    })?;

    if !allow_private
        && let Some(literal) = url.host().and_then(host_ip)
        && forbidden_ip(literal)
    {
        return Err(forbidden_target(host, literal));
    }

    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            SafeHttpError::new(
                SafeHttpErrorKind::Dns,
                format!("DNS resolution failed for {host}: {error}"),
            )
        })?
        .collect();
    addrs.sort_unstable();
    addrs.dedup();
    if addrs.is_empty() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            format!("DNS resolution returned no addresses for {host}"),
        ));
    }
    if !allow_private
        && let Some(address) = addrs.iter().find(|address| forbidden_ip(address.ip()))
    {
        return Err(forbidden_target(host, address.ip()));
    }
    Ok(addrs)
}

fn forbidden_target(host: &str, ip: IpAddr) -> SafeHttpError {
    SafeHttpError::new(
        SafeHttpErrorKind::ForbiddenTarget,
        format!("URL host {host} resolves to forbidden address {ip}"),
    )
}

fn host_ip(host: Host<&str>) -> Option<IpAddr> {
    match host {
        Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        Host::Domain(_) => None,
    }
}

/// Conservative outbound-address policy: reject every non-public or special
/// range, including CGNAT, benchmarking and documentation networks.
pub fn forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => forbidden_ipv4(ip),
        IpAddr::V6(ip) => forbidden_ipv6(ip),
    }
}

fn forbidden_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn forbidden_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    let ipv4_compatible = segments[..6].iter().all(|segment| *segment == 0);
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
        || (segments[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (segments[0] & 0xffc0) == 0xfec0 // deprecated site-local fec0::/10
        || ipv4_compatible // deprecated ::a.b.c.d form, including private IPv4
        || (segments[0] == 0x0064 && segments[1] == 0xff9b) // NAT64 well-known/local-use
        || (segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0)
        || (segments[0] == 0x2001 && segments[1] == 0x0002) // benchmarking
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || ip
            .to_ipv4_mapped()
            .is_some_and(forbidden_ipv4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn one_response(response: &'static [u8]) -> (String, tokio::task::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).await.unwrap();
            stream.write_all(response).await.unwrap();
            read
        });
        (format!("http://{address}/artifact"), task)
    }

    #[test]
    fn rejects_special_address_ranges() {
        for ip in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.0.1",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fd00::1",
            "2001:db8::1",
            "::ffff:192.168.1.1",
            "::192.168.1.1",
            "fec0::1",
        ] {
            assert!(forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(!forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn rejects_credentials_fragments_and_non_http_schemes() {
        for url in [
            "file:///etc/passwd",
            "http://user@example.com/a",
            "http://user:pass@example.com/a",
            "http://example.com/a#fragment",
        ] {
            assert!(parse_untrusted_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn error_display_url_strips_signed_query_values() {
        let url = Url::parse("https://cdn.example.com/a.png?token=secret&expires=1").unwrap();
        let displayed = redacted_url(&url);
        assert_eq!(displayed, "https://cdn.example.com/a.png");
        assert!(!displayed.contains("secret"));
    }

    #[tokio::test]
    async fn strict_policy_rejects_loopback_before_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let error = SafeHttpClient::new(Duration::from_secs(1), 32)
            .get(&format!("http://{address}/secret"))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), SafeHttpErrorKind::ForbiddenTarget);
        assert!(tokio::time::timeout(Duration::from_millis(50), listener.accept()).await.is_err());
    }

    #[tokio::test]
    async fn permitted_mock_download_is_bounded_and_returns_metadata() {
        let (url, server) = one_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Type: image/png\r\nConnection: close\r\n\r\ntest",
        )
        .await;
        let response = SafeHttpClient::new(Duration::from_secs(1), 4)
            .allow_private_for_tests()
            .get(&url)
            .await
            .unwrap();
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, b"test");
        assert!(!response.truncated);
        assert!(server.await.unwrap() > 0);
    }

    #[tokio::test]
    async fn content_length_over_limit_is_rejected_before_body_read() {
        let (url, server) = one_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\noversized",
        )
        .await;
        let error = SafeHttpClient::new(Duration::from_secs(1), 4)
            .allow_private_for_tests()
            .get(&url)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), SafeHttpErrorKind::BodyTooLarge);
        assert!(server.await.unwrap() > 0);
    }
}
