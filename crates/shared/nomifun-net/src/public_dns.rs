//! Public-only DNS for explicitly selected isolated consumers. No system-DNS
//! fallback, proxy, cookies, redirects, or caller-supplied resolver endpoints.
use super::{SafeHttpError, SafeHttpErrorKind, forbidden_ip};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

const MAX_DNS_BODY: usize = 64 * 1024;
fn invalid() -> SafeHttpError {
    SafeHttpError::new(
        SafeHttpErrorKind::Dns,
        "Public DNS response was invalid or unavailable",
    )
}
fn name(value: &str) -> Option<String> {
    let value = value.strip_suffix('.').unwrap_or(value);
    if value.is_empty()
        || value.len() > 253
        || !value.is_ascii()
        || value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}
fn answers(host: &str, kind: u16, body: &[u8]) -> Result<(Vec<IpAddr>, u64), SafeHttpError> {
    if body.len() > MAX_DNS_BODY {
        return Err(invalid());
    }
    let data: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    if data["Status"] != 0 || data["TC"] != false || data["CD"] != false {
        return Err(invalid());
    }
    let question = data["Question"]
        .as_array()
        .filter(|q| q.len() == 1)
        .ok_or_else(invalid)?;
    if question[0]["type"] != kind
        || question[0]["name"].as_str().and_then(name).as_deref() != Some(host)
    {
        return Err(invalid());
    }
    let Some(records) = data.get("Answer") else {
        return Ok((vec![], 60));
    };
    let records = records
        .as_array()
        .filter(|r| r.len() <= 64)
        .ok_or_else(invalid)?;
    let mut aliases = BTreeMap::new();
    let mut addresses = vec![];
    let mut ttl = 60;
    for record in records {
        let owner = record["name"].as_str().and_then(name).ok_or_else(invalid)?;
        let record_kind = record["type"].as_u64().ok_or_else(invalid)?;
        let value = record["data"].as_str().ok_or_else(invalid)?;
        if matches!(record_kind, 1 | 5 | 28) {
            ttl = ttl.min(record["TTL"].as_u64().ok_or_else(invalid)?);
        }
        match record_kind {
            5 => {
                let next = name(value).ok_or_else(invalid)?;
                if aliases.insert(owner, next).is_some() {
                    return Err(invalid());
                }
            }
            1 | 28 => {
                let ip: IpAddr = value.parse().map_err(|_| invalid())?;
                if record_kind != u64::from(kind) || (record_kind == 1) != ip.is_ipv4() {
                    return Err(invalid());
                }
                // Never silently discard a forbidden record from a mixed set.
                if forbidden_ip(ip) {
                    return Err(SafeHttpError::new(
                        SafeHttpErrorKind::ForbiddenTarget,
                        "Public DNS returned a non-public address",
                    ));
                }
                addresses.push((owner, ip));
            }
            _ => {} // DNSSEC records may be present even with DO=false.
        }
    }
    let mut reachable = BTreeSet::from([host.to_owned()]);
    let mut current = host;
    while let Some(next) = aliases.get(current) {
        if reachable.len() >= 16 || !reachable.insert(next.clone()) {
            return Err(invalid());
        }
        current = next;
    }
    if addresses
        .iter()
        .any(|(owner, _)| !reachable.contains(owner))
    {
        return Err(invalid());
    }
    Ok((addresses.into_iter().map(|(_, ip)| ip).collect(), ttl))
}
async fn query(
    client: &reqwest::Client,
    host: &str,
    kind: u16,
) -> Result<(Vec<IpAddr>, u64), SafeHttpError> {
    let mut url = url::Url::parse("https://dns.google/resolve").expect("fixed public resolver");
    url.query_pairs_mut()
        .append_pair("name", host)
        .append_pair("type", &kind.to_string())
        .append_pair("edns_client_subnet", "0.0.0.0/0");
    let mut response = client.get(url).send().await.map_err(|_| invalid())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|n| n > MAX_DNS_BODY as u64)
    {
        return Err(invalid());
    }
    let age=response.headers().get(reqwest::header::AGE).map(|value|value.to_str().ok().and_then(|value|value.parse::<u64>().ok()).unwrap_or(u64::MAX)).unwrap_or(0);
    let mut body = vec![];
    while let Some(chunk) = response.chunk().await.map_err(|_| invalid())? {
        if body.len() + chunk.len() > MAX_DNS_BODY {
            return Err(invalid());
        }
        body.extend_from_slice(&chunk);
    }
    let (addresses,ttl)=answers(host, kind, &body)?;
    Ok((addresses,ttl.saturating_sub(age)))
}
#[derive(Debug)]
struct Cached {
    addresses: Vec<IpAddr>,
    expires: Instant,
}
#[derive(Debug, Default)]
pub(super) struct Resolver {
    hosts: BTreeSet<String>,
    client: tokio::sync::OnceCell<reqwest::Client>,
    cache: tokio::sync::Mutex<BTreeMap<String, Cached>>,
}
impl Resolver {
    pub(super) fn new(hosts: impl IntoIterator<Item=String>)->Self {
        let mut hosts:BTreeSet<_>=hosts.into_iter().take(9).filter_map(|host|name(&host)).collect();
        if hosts.len()>8 {hosts.clear();}
        Self {hosts,..Default::default()}
    }
    pub(super) async fn resolve(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Vec<SocketAddr>, SafeHttpError> {
        let host = name(host).ok_or_else(invalid)?;
        if !self.hosts.contains(&host) {
            return Err(SafeHttpError::new(SafeHttpErrorKind::ForbiddenTarget,"Public DNS is not admitted for this hostname"));
        }
        // One resolver belongs to one explicit SafeHttpClient family. Search owns
        // it for one request, never globally or across user/session boundaries.
        let mut cache = self.cache.lock().await;
        cache.retain(|_, entry| entry.expires > Instant::now());
        if let Some(entry) = cache.get(&host) {
            return Ok(entry
                .addresses
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect());
        }
        // Bootstrap IPs belong to the fixed DNS service, not the requested target.
        // HTTPS still validates dns.google's certificate and SNI. ECS=0 avoids
        // forwarding the client's subnet to authoritative DNS servers.
        let client = self
            .client
            .get_or_try_init(|| async {
                let bootstrap = [
                    SocketAddr::from(([8, 8, 8, 8], 443)),
                    SocketAddr::from(([8, 8, 4, 4], 443)),
                ];
                reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(5))
                    .resolve_to_addrs("dns.google", &bootstrap)
                    .build()
                    .map_err(|_| invalid())
            })
            .await?;
        let started = Instant::now();
        let ((v4, ttl4), (v6, ttl6)) =
            tokio::try_join!(query(client, &host, 1), query(client, &host, 28))?;
        let mut addresses: Vec<_> = v4.into_iter().chain(v6).collect();
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(invalid());
        }
        let result = addresses
            .iter()
            .map(|ip| SocketAddr::new(*ip, port))
            .collect();
        let expires = started + Duration::from_secs(ttl4.min(ttl6));
        if cache.len() < 32 && expires > Instant::now() {
            cache.insert(host, Cached { addresses, expires });
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn unlisted_names_never_initialize_the_external_dns_client() {
        let resolver=Resolver::new(["www.bing.com".to_owned()]);
        assert_eq!(resolver.resolve("internal.example.invalid",443).await.unwrap_err().kind(),SafeHttpErrorKind::ForbiddenTarget);
        assert!(resolver.client.get().is_none());
    }
    fn response(mut records: Value) -> Vec<u8> {
        for record in records.as_array_mut().unwrap() {
            if record.get("TTL").is_none() {
                record["TTL"] = json!(30);
            }
        }
        serde_json::to_vec(&json!({"Status":0,"TC":false,"CD":false,"Question":[{"name":"search.test.","type":1}],"Answer":records})).unwrap()
    }
    #[test]
    fn follows_bounded_alias_chain_to_public_addresses() {
        let body = response(
            json!([{"name":"search.test.","type":5,"data":"edge.test."},{"name":"edge.test.","type":1,"data":"8.8.8.8"}]),
        );
        assert_eq!(
            answers("search.test", 1, &body).unwrap(),
            (vec!["8.8.8.8".parse::<IpAddr>().unwrap()], 30)
        );
    }
    #[test]
    fn rejects_private_and_mixed_answers_including_fake_ip() {
        for address in ["127.0.0.1", "198.18.0.21", "10.0.0.1", "169.254.169.254"] {
            let body = response(
                json!([{"name":"search.test.","type":1,"data":"8.8.8.8"},{"name":"search.test.","type":1,"data":address}]),
            );
            assert_eq!(
                answers("search.test", 1, &body).unwrap_err().kind(),
                SafeHttpErrorKind::ForbiddenTarget
            );
        }
    }
    #[test]
    fn rejects_unrelated_records_cycles_and_wrong_questions() {
        for records in [
            json!([{"name":"other.test.","type":1,"data":"8.8.8.8"}]),
            json!([{"name":"search.test.","type":5,"data":"search.test."}]),
        ] {
            assert!(answers("search.test", 1, &response(records)).is_err());
        }
        assert!(answers("different.test", 1, &response(json!([]))).is_err());
        assert!(answers("search.test", 28, &response(json!([]))).is_err());
    }
    #[test]
    fn rejects_truncated_or_failed_dns_and_bounds_names() {
        for field in ["Status", "TC", "CD"] {
            let mut data: Value = serde_json::from_slice(&response(json!([]))).unwrap();
            data[field] = if field == "Status" {
                json!(2)
            } else {
                json!(true)
            };
            assert!(answers("search.test", 1, &serde_json::to_vec(&data).unwrap()).is_err());
        }
        assert!(name("search.test?token=private").is_none());
        assert!(name("http://search.test").is_none());
        assert!(name(&"a".repeat(254)).is_none());
        assert!(name("search.test..").is_none());
    }
    #[test]
    fn cache_lifetime_uses_the_shortest_alias_or_address_ttl() {
        let body = response(
            json!([{"name":"search.test.","type":5,"data":"edge.test.","TTL":3},{"name":"edge.test.","type":1,"data":"8.8.8.8","TTL":60}]),
        );
        assert_eq!(answers("search.test", 1, &body).unwrap().1, 3);
        let body = response(json!([{"name":"search.test.","type":1,"data":"8.8.8.8","TTL":0}]));
        assert_eq!(answers("search.test", 1, &body).unwrap().1, 0);
    }
}
