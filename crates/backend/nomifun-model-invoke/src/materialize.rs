//! Bounded materialization of normalized provider assets.
//!
//! Adapters may return inline bytes or a short-lived URL.  Product callers
//! should not each grow their own downloader: this module applies the same
//! scheme, status, timeout, MIME and memory limits to both forms and only
//! returns after the complete batch has been materialized successfully.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::{StatusCode, Url};
use nomifun_api_types::ModelTask;

use crate::{
    InvokeError, InvokeErrorKind, ModelInvokeService, ProducedAsset, ProducedData,
    error_from_response, read_body_capped,
};

/// Conservative defaults for one materialization batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializeLimits {
    /// Maximum number of provider assets accepted in one batch.
    pub max_assets: usize,
    /// Maximum bytes accepted for any single inline/downloaded asset.
    pub max_bytes_per_asset: u64,
    /// Maximum aggregate bytes accepted across the complete batch.
    pub max_total_bytes: u64,
    /// Wall-clock limit for each URL download, including response-body reads.
    pub download_timeout: Duration,
    /// Wall-clock limit for materializing the complete batch.
    pub total_timeout: Duration,
}

impl Default for MaterializeLimits {
    fn default() -> Self {
        Self {
            max_assets: 8,
            max_bytes_per_asset: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            download_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(2 * 60),
        }
    }
}

/// One fully materialized provider asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedAsset {
    pub bytes: Vec<u8>,
    /// Normalized adapter/HTTP MIME declaration, when either supplied one.
    pub mime: Option<String>,
}

impl ModelInvokeService {
    /// Materialize against the sanitized dispatch origin captured by the exact
    /// submit call. This is the preferred post-invocation path: it remains
    /// valid if the provider/model catalog changes while generation is in
    /// flight and exposes no decrypted authentication material.
    pub async fn materialize_assets_for_invocation(
        &self,
        context: &crate::InvocationContext,
        assets: Vec<ProducedAsset>,
        limits: MaterializeLimits,
    ) -> Result<Vec<MaterializedAsset>, InvokeError> {
        self.materialize_assets(&context.artifact_origin, assets, limits)
            .await
    }

    /// Resolve the selected model locally and use its endpoint origin as the
    /// trust boundary for private-address downloads.  Provider-hosted local
    /// endpoints may therefore return same-origin localhost assets, while a
    /// public provider cannot redirect the application into localhost,
    /// link-local metadata services, or another private network origin.
    pub async fn materialize_assets_for_model(
        &self,
        model: &crate::ModelRef,
        task: ModelTask,
        assets: Vec<ProducedAsset>,
        limits: MaterializeLimits,
    ) -> Result<Vec<MaterializedAsset>, InvokeError> {
        let call = self.resolve_validated_call(model, task).await?;
        let endpoint = Url::parse(&call.endpoint_url()?)
            .map_err(|error| InvokeError::config(format!("resolved endpoint is not a valid URL: {error}")))?;
        self.materialize_assets(&endpoint, assets, limits).await
    }

    /// Resolve every inline/URL asset into memory under explicit limits.
    ///
    /// The output is all-or-nothing: an invalid member returns an error and no
    /// materialized prefix is exposed to the caller.  This function performs no
    /// persistence; the consuming product remains responsible for validating
    /// the media payload and committing it atomically.
    pub async fn materialize_assets(
        &self,
        trusted_origin: &Url,
        assets: Vec<ProducedAsset>,
        limits: MaterializeLimits,
    ) -> Result<Vec<MaterializedAsset>, InvokeError> {
        validate_limits(limits)?;
        if assets.is_empty() {
            return Err(InvokeError::parse("provider returned no assets to materialize"));
        }
        if assets.len() > limits.max_assets {
            return Err(InvokeError::new(
                InvokeErrorKind::ProviderError,
                format!(
                    "provider returned {} assets, exceeding the batch limit of {}",
                    assets.len(),
                    limits.max_assets
                ),
            ));
        }

        let deadline = tokio::time::Instant::now() + limits.total_timeout;
        let mut total = 0_u64;
        let mut materialized = Vec::with_capacity(assets.len());
        for asset in assets {
            let remaining_time = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining_time.is_zero() {
                return Err(materialize_timeout_error(limits.total_timeout));
            }
            let remaining = limits.max_total_bytes.saturating_sub(total);
            if remaining == 0 {
                return Err(total_limit_error(limits.max_total_bytes));
            }
            let member_cap = remaining.min(limits.max_bytes_per_asset);
            let item = match asset.data {
                ProducedData::Bytes(bytes) => {
                    check_size(bytes.len() as u64, member_cap, limits.max_bytes_per_asset)?;
                    if bytes.is_empty() {
                        return Err(InvokeError::parse("provider returned an empty inline asset"));
                    }
                    MaterializedAsset {
                        bytes,
                        mime: normalize_mime(asset.mime.as_deref()),
                    }
                }
                ProducedData::Url(url) => {
                    self.download_asset(
                        trusted_origin,
                        &url,
                        asset.mime.as_deref(),
                        member_cap,
                        limits.download_timeout.min(remaining_time),
                    )
                    .await?
                }
            };
            total = total
                .checked_add(item.bytes.len() as u64)
                .ok_or_else(|| total_limit_error(limits.max_total_bytes))?;
            if total > limits.max_total_bytes {
                return Err(total_limit_error(limits.max_total_bytes));
            }
            materialized.push(item);
        }
        Ok(materialized)
    }

    async fn download_asset(
        &self,
        trusted_origin: &Url,
        raw_url: &str,
        mime_hint: Option<&str>,
        max_bytes: u64,
        timeout: Duration,
    ) -> Result<MaterializedAsset, InvokeError> {
        let url = parse_artifact_url(raw_url)?;
        let download = async {
            let mut current = url;
            let mut redirects = 0_usize;
            let response = loop {
                let resolved = validate_download_target(&current, trusted_origin).await?;
                let client = pinned_client(&current, &resolved)?;
                let response = client
                    .get(current.clone())
                    .timeout(timeout)
                    .send()
                    .await
                    .map_err(|error| InvokeError::network(&error))?;
                if !is_followable_redirect(response.status()) {
                    break response;
                }
                if redirects >= MAX_ARTIFACT_REDIRECTS {
                    return Err(InvokeError::new(
                        InvokeErrorKind::ProviderError,
                        format!("artifact download exceeded {MAX_ARTIFACT_REDIRECTS} redirects"),
                    ));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| InvokeError::parse("artifact redirect is missing a valid Location header"))?;
                current = current.join(location).map_err(|error| {
                    InvokeError::parse(format!("artifact redirect has an invalid Location: {error}"))
                })?;
                current = parse_artifact_url(current.as_str())?;
                redirects += 1;
            };
            if !response.status().is_success() {
                return Err(error_from_response(response).await);
            }
            let response_mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let mime = reconcile_mime(mime_hint, response_mime)?;
            let bytes = read_body_capped(response, max_bytes).await?;
            if bytes.is_empty() {
                return Err(InvokeError::parse("provider returned an empty artifact body"));
            }
            Ok(MaterializedAsset { bytes, mime })
        };
        tokio::time::timeout(timeout, download).await.map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::Timeout,
                format!("artifact download timed out after {} ms", timeout.as_millis()),
            )
        })?
    }
}

const MAX_ARTIFACT_REDIRECTS: usize = 5;

fn validate_limits(limits: MaterializeLimits) -> Result<(), InvokeError> {
    if limits.max_assets == 0
        || limits.max_bytes_per_asset == 0
        || limits.max_total_bytes == 0
        || limits.download_timeout.is_zero()
        || limits.total_timeout.is_zero()
    {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            "materialize limits must all be greater than zero",
        ));
    }
    Ok(())
}

fn materialize_timeout_error(timeout: Duration) -> InvokeError {
    InvokeError::new(
        InvokeErrorKind::Timeout,
        format!(
            "artifact batch materialization timed out after {} ms",
            timeout.as_millis()
        ),
    )
}

fn check_size(actual: u64, effective_cap: u64, per_asset_cap: u64) -> Result<(), InvokeError> {
    if actual > effective_cap {
        let message = if effective_cap < per_asset_cap {
            format!("artifact batch exceeds the remaining total byte limit ({effective_cap} bytes)")
        } else {
            format!("artifact is {actual} bytes, exceeding the per-asset limit of {per_asset_cap}")
        };
        return Err(InvokeError::new(InvokeErrorKind::ProviderError, message));
    }
    Ok(())
}

fn total_limit_error(limit: u64) -> InvokeError {
    InvokeError::new(
        InvokeErrorKind::ProviderError,
        format!("artifact batch exceeds the total byte limit of {limit}"),
    )
}

fn parse_artifact_url(raw: &str) -> Result<reqwest::Url, InvokeError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(InvokeError::parse("provider returned an empty artifact URL"));
    }
    let url = reqwest::Url::parse(raw)
        .map_err(|error| InvokeError::parse(format!("provider returned an invalid artifact URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(InvokeError::parse(format!(
            "artifact URL uses unsupported scheme {:?}",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(InvokeError::parse("artifact URL must not contain embedded credentials"));
    }
    Ok(url)
}

fn is_followable_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

async fn validate_download_target(
    url: &Url,
    trusted_origin: &Url,
) -> Result<Vec<SocketAddr>, InvokeError> {
    let host = url
        .host_str()
        .ok_or_else(|| InvokeError::parse("artifact URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| InvokeError::parse("artifact URL has no usable port"))?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| {
            InvokeError::new(
                InvokeErrorKind::Network,
                format!("failed to resolve artifact host {host:?}: {error}"),
            )
        })?
        .collect();
    if addresses.is_empty() {
        return Err(InvokeError::new(
            InvokeErrorKind::Network,
            format!("artifact host {host:?} resolved to no addresses"),
        ));
    }
    // String equality alone is not trust: a public provider hostname can DNS
    // rebind to loopback/metadata while remaining textually same-origin.  Only
    // an endpoint explicitly configured as localhost/private IP grants the
    // same-origin private-network exception.
    let private_allowed = same_origin(url, trusted_origin)
        && trusted_origin_allows_private(trusted_origin);
    if !private_allowed
        && let Some(address) = addresses.iter().find(|address| is_forbidden_ip(address.ip()))
    {
        return Err(InvokeError::new(
            InvokeErrorKind::ProviderError,
            format!(
                "artifact URL resolves to a private, loopback, link-local, metadata, or reserved address ({})",
                address.ip()
            ),
        ));
    }
    Ok(addresses)
}

fn pinned_client(url: &Url, addresses: &[SocketAddr]) -> Result<reqwest::Client, InvokeError> {
    let host = url
        .host_str()
        .ok_or_else(|| InvokeError::parse("artifact URL has no host"))?;
    // Redirects are processed manually and every hop is DNS-checked.  Pinning
    // the validated address set also closes the DNS-rebinding gap between the
    // policy lookup above and the actual connection.  `no_proxy` is deliberate:
    // a proxy would resolve the hostname again outside this process and defeat
    // the pin/private-address decision.
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, addresses)
        .build()
        .map_err(|error| InvokeError::config(format!("failed to build artifact download client: {error}")))
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme().eq_ignore_ascii_case(right.scheme())
        && left
            .host_str()
            .zip(right.host_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        && left.port_or_known_default() == right.port_or_known_default()
}

fn trusted_origin_allows_private(origin: &Url) -> bool {
    let Some(host) = origin.host_str() else {
        return false;
    };
    let unbracketed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return is_trusted_local_origin_ip(ip);
    }
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    normalized == "localhost" || normalized.ends_with(".localhost")
}

fn is_trusted_local_origin_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            a == 127
                || a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xfe00) == 0xfc00
        }
    }
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_forbidden_v4(ip),
        IpAddr::V6(ip) => is_forbidden_v6(ip),
    }
}

fn is_forbidden_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn is_forbidden_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4() {
        return is_forbidden_v4(v4);
    }
    let segments = ip.segments();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
        || (segments[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (segments[0] & 0xffc0) == 0xfec0 // deprecated site-local fec0::/10
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
}

fn normalize_mime(value: Option<&str>) -> Option<String> {
    let normalized = value
        .and_then(|mime| mime.split(';').next())
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .map(str::to_ascii_lowercase)?;
    Some(if normalized == "image/jpg" {
        "image/jpeg".to_owned()
    } else {
        normalized
    })
}

fn is_generic_binary_mime(mime: &str) -> bool {
    matches!(mime, "application/octet-stream" | "binary/octet-stream")
}

fn reconcile_mime(hint: Option<&str>, response: Option<&str>) -> Result<Option<String>, InvokeError> {
    let hint = normalize_mime(hint);
    let response = normalize_mime(response);
    match (hint, response) {
        (Some(hint), Some(response)) if is_generic_binary_mime(&response) => Ok(Some(hint)),
        (Some(hint), Some(response)) if is_generic_binary_mime(&hint) => Ok(Some(response)),
        (Some(hint), Some(response)) if hint != response => Err(InvokeError::parse(format!(
            "artifact MIME mismatch: adapter declared {hint:?}, HTTP response declared {response:?}"
        ))),
        (Some(mime), Some(_)) | (Some(mime), None) | (None, Some(mime)) => Ok(Some(mime)),
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_db::{
        SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
        SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
    };
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::{AdapterRegistry, ProducedAsset, ProducedData};

    async fn service() -> ModelInvokeService {
        let database = init_database_memory().await.expect("database");
        let pool = database.pool().clone();
        ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool)),
            [0x42; 32],
            reqwest::Client::new(),
            AdapterRegistry::new(Vec::new()),
        )
    }

    fn limits() -> MaterializeLimits {
        MaterializeLimits {
            max_assets: 4,
            max_bytes_per_asset: 16,
            max_total_bytes: 24,
            download_timeout: Duration::from_millis(500),
            total_timeout: Duration::from_secs(2),
        }
    }

    #[tokio::test]
    async fn materializes_inline_and_url_in_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "image/png; charset=binary")
                    .set_body_bytes(b"download"),
            )
            .mount(&server)
            .await;
        let assets = vec![
            ProducedAsset {
                data: ProducedData::Bytes(b"inline".to_vec()),
                mime: Some("IMAGE/PNG".into()),
            },
            ProducedAsset {
                data: ProducedData::Url(format!("{}/asset", server.uri())),
                mime: Some("application/octet-stream".into()),
            },
        ];

        let origin = Url::parse(&server.uri()).unwrap();
        let result = service()
            .await
            .materialize_assets(&origin, assets, limits())
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0], MaterializedAsset { bytes: b"inline".to_vec(), mime: Some("image/png".into()) });
        assert_eq!(result[1], MaterializedAsset { bytes: b"download".to_vec(), mime: Some("image/png".into()) });
    }

    #[tokio::test]
    async fn rejects_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_string("gone"))
            .mount(&server)
            .await;
        let error = service()
            .await
            .materialize_assets(
                &Url::parse(&server.uri()).unwrap(),
                vec![ProducedAsset {
                    data: ProducedData::Url(format!("{}/missing", server.uri())),
                    mime: None,
                }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.http_status, Some(404));
    }

    #[tokio::test]
    async fn rejects_oversized_inline_and_aggregate_batches() {
        let svc = service().await;
        let origin = Url::parse("https://example.com").unwrap();
        let single = svc
            .materialize_assets(
                &origin,
                vec![ProducedAsset {
                    data: ProducedData::Bytes(vec![0; 17]),
                    mime: None,
                }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(single.kind, InvokeErrorKind::ProviderError);

        let aggregate = svc
            .materialize_assets(
                &origin,
                vec![
                    ProducedAsset { data: ProducedData::Bytes(vec![1; 13]), mime: None },
                    ProducedAsset { data: ProducedData::Bytes(vec![2; 12]), mime: None },
                ],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(aggregate.kind, InvokeErrorKind::ProviderError);
    }

    #[tokio::test]
    async fn rejects_mime_conflict_and_invalid_scheme() {
        assert_eq!(
            reconcile_mime(Some("IMAGE/JPG"), Some("image/jpeg; charset=binary")).unwrap(),
            Some("image/jpeg".to_owned())
        );
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string("<html></html>"),
            )
            .mount(&server)
            .await;
        let svc = service().await;
        let mismatch = svc
            .materialize_assets(
                &Url::parse(&server.uri()).unwrap(),
                vec![ProducedAsset {
                    data: ProducedData::Url(format!("{}/html", server.uri())),
                    mime: Some("image/png".into()),
                }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(mismatch.kind, InvokeErrorKind::ParseError);

        let scheme = svc
            .materialize_assets(
                &Url::parse("https://example.com").unwrap(),
                vec![ProducedAsset { data: ProducedData::Url("file:///tmp/x".into()), mime: None }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(scheme.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn materialization_batch_has_a_hard_wall_clock_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .mount(&server)
            .await;
        let mut constrained = limits();
        constrained.download_timeout = Duration::from_millis(500);
        constrained.total_timeout = Duration::from_millis(10);
        let error = service()
            .await
            .materialize_assets(
                &Url::parse(&server.uri()).unwrap(),
                vec![ProducedAsset {
                    data: ProducedData::Url(format!("{}/slow", server.uri())),
                    mime: None,
                }],
                constrained,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::Timeout);
    }

    #[tokio::test]
    async fn blocks_metadata_and_cross_origin_private_redirects() {
        let svc = service().await;
        let origin = Url::parse("https://provider.example/v1/images").unwrap();
        let metadata = svc
            .materialize_assets(
                &origin,
                vec![ProducedAsset {
                    data: ProducedData::Url("http://169.254.169.254/latest/meta-data".into()),
                    mime: None,
                }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(metadata.kind, InvokeErrorKind::ProviderError);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://169.254.169.254/latest/meta-data"),
            )
            .mount(&server)
            .await;
        let redirected = svc
            .materialize_assets(
                &Url::parse(&server.uri()).unwrap(),
                vec![ProducedAsset {
                    data: ProducedData::Url(format!("{}/redirect", server.uri())),
                    mime: None,
                }],
                limits(),
            )
            .await
            .unwrap_err();
        assert_eq!(redirected.kind, InvokeErrorKind::ProviderError);
    }

    #[test]
    fn same_origin_public_hostname_never_grants_private_network_access() {
        let trusted = Url::parse("https://images.example/v1/images").unwrap();
        let same_origin = Url::parse("https://images.example/download/1").unwrap();
        assert!(super::same_origin(&same_origin, &trusted));
        assert!(!super::trusted_origin_allows_private(&trusted));

        for local in [
            "http://localhost:8080/v1/images",
            "http://localhost.:8080/v1/images",
            "http://worker.localhost:8080/v1/images",
            "http://127.0.0.1:8080/v1/images",
            "http://10.0.0.7:8080/v1/images",
            "http://192.168.1.7:8080/v1/images",
            "http://[::1]:8080/v1/images",
            "http://[fd00::7]:8080/v1/images",
        ] {
            assert!(
                super::trusted_origin_allows_private(&Url::parse(local).unwrap()),
                "origin={local}"
            );
        }
        for forbidden in [
            "http://169.254.169.254/latest/meta-data",
            "http://0.0.0.0:8080/v1/images",
            "http://224.0.0.1:8080/v1/images",
        ] {
            assert!(
                !super::trusted_origin_allows_private(&Url::parse(forbidden).unwrap()),
                "origin={forbidden}"
            );
        }
    }
}
