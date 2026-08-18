//! Registrable-domain extraction and URL-ish host normalization for browser policy.

/// Extract and lowercase the host from a bare host, origin, or URL-like string.
pub fn host_of(origin: &str) -> Option<String> {
    let input = origin.trim();
    if input.is_empty() {
        return None;
    }

    let after_scheme = input.split_once("://").map_or(input, |(_scheme, rest)| rest);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or(after_scheme);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_userinfo, host)| host);
    let host = if let Some(ipv6) = host_port.strip_prefix('[') {
        ipv6.split(']').next().unwrap_or(ipv6)
    } else if let Some((host, port)) = host_port.rsplit_once(':') {
        if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) {
            host
        } else {
            host_port
        }
    } else {
        host_port
    };

    let host = host.trim().trim_end_matches('.');
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Compute the eTLD+1 (registrable domain) using the embedded Public Suffix List.
pub fn etld_plus_one(input: &str) -> Option<String> {
    let host = host_of(input)?;
    psl::domain_str(&host).map(|domain| domain.to_ascii_lowercase())
}

/// Return true when both values have the same registrable domain.
pub fn same_etld_plus_one(left: &str, right: &str) -> bool {
    matches!(
        (etld_plus_one(left), etld_plus_one(right)),
        (Some(left), Some(right)) if left == right
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hosts_and_real_public_suffixes() {
        assert_eq!(host_of("https://User:redacted@Sub.X.COM:8443/a"), Some("sub.x.com".into()));
        assert_eq!(etld_plus_one("https://www.a.co.uk"), Some("a.co.uk".into()));
        assert_ne!(etld_plus_one("a.co.uk"), etld_plus_one("b.co.uk"));
        assert!(same_etld_plus_one("login.x.com", "https://x.com"));
    }

    #[test]
    fn invalid_or_non_registrable_values_fail_closed() {
        assert_eq!(etld_plus_one("co.uk"), None);
        assert_eq!(etld_plus_one("localhost"), None);
        assert!(!same_etld_plus_one("", "x.com"));
    }
}
