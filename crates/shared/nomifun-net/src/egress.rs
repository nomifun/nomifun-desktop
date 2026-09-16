//! SSRF-safe HTTP GETs for untrusted URLs.
//!
//! Every redirect hop is parsed and resolved before a socket is opened. All
//! resolved addresses must be public, and the validated addresses are pinned
//! into a fresh, proxy-free reqwest client for that hop. This makes URL
//! validation and the connection use the same DNS answer.
//! A configured proxy may synthesize 198.18/15 DNS answers. Only for those
//! domain answers, public HTTPS DNS can recover real addresses; they undergo
//! the same checks and direct pinning. Reserved ranges never become targets.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::header::{HeaderMap, LOCATION};
use url::{Host, Url};

#[path = "public_dns.rs"]
mod public_dns;

const DNS_TIMEOUT: Duration = Duration::from_secs(15);
const DNS_RESPONSE_LIMIT: usize = 64 * 1024;
const PUBLIC_DNS_ENDPOINT: &str = "https://cloudflare-dns.com/dns-query";

/// Fingerprint the compiled egress implementation used by exact runtime bindings.
pub fn implementation_digest() -> String {
    use sha2::{Digest,Sha256};
    let mut digest=Sha256::new();
    digest.update(include_bytes!("egress.rs"));
    digest.update(include_bytes!("public_dns.rs"));
    format!("{:x}",digest.finalize())
}

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
    public_dns: Option<std::sync::Arc<public_dns::Resolver>>,
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
            public_dns: None,
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

    /// Resolve through the fixed public HTTPS resolver before applying the
    /// same public-address checks and socket pinning. This is an explicit
    /// host policy, never a fallback after a rejected system DNS response.
    /// Only the caller's allowlisted public names leave through DoH, never
    /// arbitrary user URLs, internal names, paths, cookies or query strings.
    pub fn with_public_dns_for_hosts(mut self, hosts: impl IntoIterator<Item=String>) -> Self {
        self.public_dns=Some(std::sync::Arc::new(public_dns::Resolver::new(hosts)));
        self
    }

    /// Permit private targets for loopback mock servers. Production callers
    /// must use the strict default.
    pub fn allow_private_for_tests(mut self) -> Self {
        self.allow_private = true;
        self
    }

    pub async fn get(&self, raw_url: &str) -> Result<SafeHttpResponse, SafeHttpError> {
        let url = parse_untrusted_url(raw_url)?;
        // One budget covers DNS, every redirect and the complete bounded body.
        tokio::time::timeout(self.timeout, self.get_url(url))
            .await
            .map_err(|_| {
                SafeHttpError::new(SafeHttpErrorKind::Timeout, "safe HTTP request timed out")
            })?
    }

    /// Fetch exactly one validated, DNS-pinned hop. Redirects are returned to
    /// the caller, so a browser broker can validate each new origin itself.
    /// Headers belong to the caller's isolated request; no shared cookie jar,
    /// system proxy, credentials, or redirect header forwarding is installed.
    pub async fn get_once(&self, raw_url: &str, headers: HeaderMap) -> Result<SafeHttpResponse, SafeHttpError> {
        let url = parse_untrusted_url(raw_url)?;
        tokio::time::timeout(self.timeout, async {
            let addrs = resolve_validated(&url, self.allow_private, self.public_dns.as_deref()).await?;
            let response = self.send_with_headers(&url, &addrs, headers).await?;
            self.finish_response(url, response).await
        }).await.map_err(|_| SafeHttpError::new(SafeHttpErrorKind::Timeout, "safe HTTP request timed out"))?
    }

    async fn finish_response(&self, url: Url, response: reqwest::Response) -> Result<SafeHttpResponse, SafeHttpError> {
        if self.overflow == BodyOverflowPolicy::Reject
            && response.content_length().is_some_and(|length|length>self.max_body_bytes as u64) {
            return Err(SafeHttpError::new(SafeHttpErrorKind::BodyTooLarge, "safe HTTP response exceeds its body limit"));
        }
        let status = response.status();
        let headers = response.headers().clone();
        let (body,truncated) = self.read_body(response,&url).await?;
        Ok(SafeHttpResponse {final_url:url,status,headers,body,truncated})
    }

    async fn get_url(&self, mut url: Url) -> Result<SafeHttpResponse, SafeHttpError> {
        for hop in 0..=self.max_redirects {
            let addrs = resolve_validated(&url, self.allow_private, self.public_dns.as_deref()).await?;
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
        self.send_with_headers(url,addrs,HeaderMap::new()).await
    }

    async fn send_with_headers(
        &self,
        url: &Url,
        addrs: &[SocketAddr],
        headers: HeaderMap,
    ) -> Result<reqwest::Response, SafeHttpError> {
        // A proxy can resolve the target independently and defeat DNS pinning.
        // Untrusted fetches therefore always connect directly.
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
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
            .headers(headers)
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
        SafeHttpError::new(
            SafeHttpErrorKind::InvalidUrl,
            format!("invalid URL: {error}"),
        )
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
            format!(
                "only http(s) URLs are supported (got scheme: {})",
                url.scheme()
            ),
        ));
    }
    if url.host_str().is_none() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::InvalidUrl,
            "URL has no host",
        ));
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
/// System DNS and optional public DNS recovery are each bounded to 15 seconds.
pub async fn validate_untrusted_url(raw: &str, allow_private: bool) -> Result<Url, SafeHttpError> {
    let url = parse_untrusted_url(raw)?;
    resolve_validated(&url, allow_private, None).await?;
    Ok(url)
}

async fn resolve_validated(
    url: &Url,
    allow_private: bool,
    public_dns: Option<&public_dns::Resolver>,
) -> Result<Vec<SocketAddr>, SafeHttpError> {
    let allow_private=allow_private && public_dns.is_none();
    let host = url
        .host_str()
        .ok_or_else(|| SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, "URL has no host"))?;
    let port = url.port_or_known_default().ok_or_else(|| {
        SafeHttpError::new(SafeHttpErrorKind::InvalidUrl, "URL has no usable port")
    })?;

    if let Some(literal) = url.host().and_then(host_ip) {
        if !allow_private && forbidden_ip(literal) {
            return Err(forbidden_target(host, literal));
        }
        // IP literals need neither DNS nor platform-specific handling of the
        // brackets returned by Url::host_str for IPv6.
        return Ok(vec![SocketAddr::new(literal, port)]);
    }

    let mut addrs: Vec<SocketAddr> = if let Some(resolver)=public_dns {
        resolver.resolve(host,port).await?
    } else { tokio::time::timeout(
        DNS_TIMEOUT, tokio::net::lookup_host((host, port)),
    )
        .await
        .map_err(|_| SafeHttpError::new(SafeHttpErrorKind::Timeout, "DNS resolution timed out"))?
        .map_err(|error| {
            SafeHttpError::new(
                SafeHttpErrorKind::Dns,
                format!("DNS resolution failed for {host}: {error}"),
            )
        })?
        .collect() };
    addrs.sort_unstable();
    addrs.dedup();
    if addrs.is_empty() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            format!("DNS resolution returned no addresses for {host}"),
        ));
    }
    validate_dns_with_recovery(
        host,
        addrs,
        allow_private,
        public_dns.is_none() && !allow_private && crate::proxy::domain_uses_detected_proxy(url),
        || recover_public_dns(host, port),
    )
    .await
}

fn fake_ip(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(ip) if ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19))
}

async fn validate_dns_with_recovery<F, Fut>(
    host: &str,
    mut addresses: Vec<SocketAddr>,
    allow_private: bool,
    proxy_selected: bool,
    recover: F,
) -> Result<Vec<SocketAddr>, SafeHttpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<SocketAddr>, SafeHttpError>>,
{
    let forbidden: Vec<_> = addresses
        .iter()
        .filter(|address| forbidden_ip(address.ip()))
        .collect();
    // Never reinterpret real private/metadata answers, or mixed private and
    // Fake-IP answers, as a proxy artefact. NO_PROXY also keeps fail-closed DNS.
    if !allow_private
        && proxy_selected
        && !forbidden.is_empty()
        && forbidden.iter().all(|address| fake_ip(address.ip()))
    {
        addresses = recover().await?;
    }
    if addresses.is_empty() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            format!("DNS resolution returned no addresses for {host}"),
        ));
    }
    if !allow_private
        && let Some(address) = addresses.iter().find(|address| forbidden_ip(address.ip()))
    {
        return Err(forbidden_target(host, address.ip()));
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

async fn recover_public_dns(host: &str, port: u16) -> Result<Vec<SocketAddr>, SafeHttpError> {
    tokio::time::timeout(DNS_TIMEOUT, async {
        // This fixed HTTPS resolver is reached through the existing configured
        // proxy client, with normal TLS verification and redirects disabled.
        // It receives only the hostname, never the artifact URL or credentials.
        let client = crate::http_client_no_redirect().map_err(|_| {
            SafeHttpError::new(
                SafeHttpErrorKind::ClientBuild,
                "cannot initialize public DNS recovery client",
            )
        })?;
        let (ipv4, ipv6) = tokio::try_join!(
            public_dns_answer(&client, host, port, "A"),
            public_dns_answer(&client, host, port, "AAAA"),
        )?;
        Ok(ipv4.into_iter().chain(ipv6).collect())
    })
    .await
    .map_err(|_| SafeHttpError::new(SafeHttpErrorKind::Timeout, "public DNS recovery timed out"))?
}

async fn public_dns_answer(
    client: &reqwest::Client,
    host: &str,
    port: u16,
    record_type: &str,
) -> Result<Vec<SocketAddr>, SafeHttpError> {
    let mut response = client
        .get(PUBLIC_DNS_ENDPOINT)
        .query(&[("name", host), ("type", record_type)])
        .header(reqwest::header::ACCEPT, "application/dns-json")
        .send()
        .await
        .map_err(|_| {
            SafeHttpError::new(SafeHttpErrorKind::Dns, "public DNS recovery request failed")
        })?;
    if !response.status().is_success() {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            "public DNS recovery returned a non-success response",
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            "cannot read public DNS recovery response",
        )
    })? {
        if body.len().saturating_add(chunk.len()) > DNS_RESPONSE_LIMIT {
            return Err(SafeHttpError::new(
                SafeHttpErrorKind::Dns,
                "public DNS recovery response exceeds size limit",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    parse_public_dns_answer(&body, port)
}

fn parse_public_dns_answer(body: &[u8], port: u16) -> Result<Vec<SocketAddr>, SafeHttpError> {
    let data: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
        SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            "invalid public DNS recovery response",
        )
    })?;
    if data.get("Status").and_then(|status| status.as_u64()) != Some(0)
        || data.get("TC").and_then(|truncated| truncated.as_bool()) == Some(true)
    {
        return Err(SafeHttpError::new(
            SafeHttpErrorKind::Dns,
            "public DNS recovery did not return a complete successful answer",
        ));
    }
    let mut addresses = Vec::new();
    for answer in data
        .get("Answer")
        .and_then(|answers| answers.as_array())
        .into_iter()
        .flatten()
    {
        let kind = answer.get("type").and_then(|kind| kind.as_u64());
        if !matches!(kind, Some(1 | 28)) {
            continue;
        }
        let ip = answer
            .get("data")
            .and_then(|ip| ip.as_str())
            .and_then(|ip| ip.parse::<IpAddr>().ok())
            .filter(|ip| {
                matches!(
                    (kind, ip),
                    (Some(1), IpAddr::V4(_)) | (Some(28), IpAddr::V6(_))
                )
            })
            .ok_or_else(|| {
                SafeHttpError::new(SafeHttpErrorKind::Dns, "invalid public DNS address record")
            })?;
        addresses.push(SocketAddr::new(ip, port));
    }
    Ok(addresses)
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
    if let Some(ipv4) = ip.to_ipv4_mapped() {
        return forbidden_ipv4(ipv4);
    }
    let segments = ip.segments();
    // Global unicast is 2000::/3. Everything outside it is special/reserved,
    // including local, multicast, NAT64 and deprecated IPv4-compatible forms.
    (segments[0] & 0xe000) != 0x2000
        || (segments[0] == 0x2001 && segments[1] < 0x0200) // IETF special-purpose /23 (including Teredo)
        || segments[0] == 0x2002 // deprecated 6to4 can embed non-public IPv4
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || (segments[0] == 0x3fff && (segments[1] & 0xf000) == 0) // documentation 3fff::/20
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn fake_dns_recovery_requires_proxy_and_preserves_public_pins() {
        let initial = vec!["198.18.0.12:443".parse().unwrap()];
        let expected = vec![
            "8.8.8.8:443".parse().unwrap(),
            "1.1.1.1:443".parse().unwrap(),
        ];
        let recovered =
            validate_dns_with_recovery("cdn.example", initial.clone(), false, true, || async {
                Ok(expected.clone())
            })
            .await
            .unwrap();
        let mut sorted = expected;
        sorted.sort_unstable();
        assert_eq!(recovered, sorted);
        let error = validate_dns_with_recovery("cdn.example", initial, false, false, || async {
            panic!("NO_PROXY or absent proxy must never invoke public DNS");
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind(), SafeHttpErrorKind::ForbiddenTarget);
        let public = vec!["8.8.8.8:443".parse().unwrap()];
        assert_eq!(
            validate_dns_with_recovery("cdn.example", public.clone(), false, true, || async {
                panic!("ordinary public DNS must not be replaced");
            })
            .await
            .unwrap(),
            public
        );
    }

    #[tokio::test]
    async fn fake_dns_recovery_never_reinterprets_or_accepts_private_answers() {
        for address in ["127.0.0.1:443", "10.0.0.1:443", "169.254.169.254:443"] {
            let error = validate_dns_with_recovery(
                "cdn.example",
                vec!["198.18.0.12:443".parse().unwrap(), address.parse().unwrap()],
                false,
                true,
                || async {
                    panic!("mixed private answers must remain blocked");
                },
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), SafeHttpErrorKind::ForbiddenTarget);
        }
        for address in [
            "198.18.0.12:443",
            "127.0.0.1:443",
            "169.254.169.254:443",
            "[::1]:443",
        ] {
            let error = validate_dns_with_recovery(
                "cdn.example",
                vec!["198.19.0.2:443".parse().unwrap()],
                false,
                true,
                || async {
                    Ok(vec![
                        "8.8.8.8:443".parse().unwrap(),
                        address.parse().unwrap(),
                    ])
                },
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), SafeHttpErrorKind::ForbiddenTarget);
        }
        for raw in [
            "http://198.18.0.12/file",
            "https://198.19.0.2/file",
            "http://127.0.0.1/file",
        ] {
            assert_eq!(
                resolve_validated(&Url::parse(raw).unwrap(), false, None)
                    .await
                    .unwrap_err()
                    .kind(),
                SafeHttpErrorKind::ForbiddenTarget
            );
        }
    }

    #[test]
    fn public_dns_recovery_extracts_only_typed_addresses_and_rejects_bad_responses() {
        let bytes = br#"{"Status":0,"Answer":[{"type":5,"data":"alias.example"},{"type":16,"data":"127.0.0.1"},{"type":1,"data":"8.8.8.8"},{"type":28,"data":"2606:4700:4700::1111"}]}"#;
        assert_eq!(
            parse_public_dns_answer(bytes, 443).unwrap(),
            vec![
                "8.8.8.8:443".parse().unwrap(),
                "[2606:4700:4700::1111]:443".parse().unwrap()
            ]
        );
        for bytes in [
            br#"{"Status":3}"#.as_slice(),
            br#"{"Status":0,"TC":true}"#.as_slice(),
            br#"{"Status":0,"Answer":[{"type":1,"data":"::1"}]}"#.as_slice(),
            br#"{"Status":0,"Answer":[{"type":28,"data":"not an address"}]}"#.as_slice(),
        ] {
            assert_eq!(
                parse_public_dns_answer(bytes, 443).unwrap_err().kind(),
                SafeHttpErrorKind::Dns
            );
        }
    }

    #[tokio::test]
    async fn pinned_transport_uses_validated_address_and_original_host() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 2048];
            let count = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
            assert!(request.contains("host: pinned-artifact.invalid:"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let url = Url::parse(&format!(
            "http://pinned-artifact.invalid:{}/artifact",
            address.port()
        ))
        .unwrap();
        let response = SafeHttpClient::new(Duration::from_secs(2), 32)
            .send(&url, &[address])
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn literal_addresses_are_resolved_without_domain_syntax() {
        for (raw, expected) in [
            ("https://8.8.8.8/", "8.8.8.8:443"),
            (
                "https://[2606:4700:4700::1111]/",
                "[2606:4700:4700::1111]:443",
            ),
            ("http://[::1]:8080/", "[::1]:8080"),
        ] {
            let url = Url::parse(raw).unwrap();
            assert_eq!(resolve_validated(&url, true, None).await.unwrap(), vec![expected.parse::<SocketAddr>().unwrap()]);
        }
    }

    #[test]
    fn reserved_and_transition_ipv6_ranges_are_not_public_egress() {
        let allowed: Vec<_> = [
            "4000::1",
            "2001::1",
            "2001:20::1",
            "2002:7f00:1::",
            "3fff::1",
        ]
        .into_iter()
        .filter(|raw| !forbidden_ip(raw.parse().unwrap()))
        .collect();
        assert!(
            allowed.is_empty(),
            "special IPv6 ranges were permitted: {allowed:?}"
        );
    }

    #[tokio::test]
    async fn redirects_share_one_request_timeout_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".as_slice(),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 2048];
                stream.read(&mut buffer).await.unwrap();
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = stream.write_all(response).await;
            }
        });
        let result = SafeHttpClient::new(Duration::from_millis(400), 32)
            .allow_private_for_tests()
            .get(&format!("http://{address}/start?token=fixture-secret"))
            .await;
        server.abort();
        let _ = server.await;
        let error = result.expect_err("redirects must not reset the total timeout");
        assert_eq!(error.kind(), SafeHttpErrorKind::Timeout);
        assert!(!error.to_string().contains("fixture-secret"));
    }

    async fn one_response(response: &[u8]) -> (String, tokio::task::JoinHandle<usize>) {
        let response = response.to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).await.unwrap();
            stream.write_all(&response).await.unwrap();
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
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn one_hop_returns_redirect_without_connecting_to_its_target() {
        let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = destination.local_addr().unwrap();
        let response = format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/not-followed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let (url, server) = one_response(response.as_bytes()).await;
        let result = SafeHttpClient::new(Duration::from_secs(1),32)
            .allow_private_for_tests().get_once(&url, HeaderMap::new()).await.unwrap();
        assert_eq!(result.status,StatusCode::FOUND);
        assert_eq!(result.final_url.as_str(),url);
        assert!(result.headers.contains_key(LOCATION));
        assert!(server.await.unwrap()>0);
        assert!(tokio::time::timeout(Duration::from_millis(50), destination.accept()).await.is_err());
    }

    #[tokio::test]
    async fn one_hop_keeps_private_network_denied() {
        let error = SafeHttpClient::new(Duration::from_secs(1),32)
            .get_once("http://127.0.0.1:12345/", HeaderMap::new()).await.unwrap_err();
        assert_eq!(error.kind(),SafeHttpErrorKind::ForbiddenTarget);
    }

    #[tokio::test]
    async fn public_resolution_never_enables_private_test_targets() {
        for url in ["http://127.0.0.1/","https://198.18.0.21/","http://[::1]/"] {
            let error=SafeHttpClient::new(Duration::from_secs(1),32).allow_private_for_tests().with_public_dns_for_hosts(["www.bing.com".to_owned()])
                .get_once(url,HeaderMap::new()).await.unwrap_err();
            assert_eq!(error.kind(),SafeHttpErrorKind::ForbiddenTarget);
        }
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
