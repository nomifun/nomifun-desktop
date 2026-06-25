//! eTLD+1 (registrable-domain) extraction and origin host normalization.
//!
//! Domain binding (DESIGN §4 / §16, ruling ⑦) uses the **eTLD+1** — the
//! registrable domain one label below the public suffix — as the equivalence
//! key. We use the [`psl`] crate, which embeds Mozilla's Public Suffix List at
//! compile time (fully offline) and therefore handles multi-level suffixes
//! correctly: `a.co.uk` and `b.co.uk` have *distinct* eTLD+1s (`a.co.uk` vs
//! `b.co.uk`) because `co.uk` is itself a public suffix — a naive
//! "take the last two labels" would wrongly collapse them to `co.uk`.

/// Extract the host component from an origin or URL-ish string and lowercase it.
///
/// Accepts bare hosts (`x.com`), full origins (`https://sub.x.com:8443`), and
/// values with a path/userinfo. Scheme, port, userinfo, and path are stripped;
/// the host is ASCII-lowercased. Returns `None` if no plausible host remains.
///
/// This is deliberately tolerant rather than a strict URL parser: the only
/// security decision downstream is the eTLD+1 comparison, and a malformed host
/// simply fails to produce an eTLD+1 (fail-closed).
pub fn host_of(origin: &str) -> Option<String> {
    let s = origin.trim();
    if s.is_empty() {
        return None;
    }

    // Strip scheme: everything before "://".
    let after_scheme = match s.split_once("://") {
        Some((_scheme, rest)) => rest,
        None => s,
    };

    // The authority is everything up to the first '/', '?', or '#'.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);

    // Strip userinfo (user:pass@host).
    let host_port = match authority.rsplit_once('@') {
        Some((_userinfo, hp)) => hp,
        None => authority,
    };

    // Strip port. Guard against IPv6 literals like "[::1]:8080".
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        // IPv6: take up to the closing ']'.
        stripped.split(']').next().unwrap_or(stripped)
    } else if let Some((h, port)) = host_port.rsplit_once(':') {
        // Only treat the suffix as a port if it is all digits; otherwise the
        // ':' was not a port separator (defensive — normal hosts have none).
        if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            host_port
        }
    } else {
        host_port
    };

    let host = host.trim().trim_end_matches('.'); // drop trailing root dot
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Compute the eTLD+1 (registrable domain) of a host or origin string.
///
/// `https://sub.login.example.co.uk` → `example.co.uk`.
/// `x.com` → `x.com`. Returns `None` for a bare public suffix (`co.uk`),
/// an IP address, `localhost`, or anything without a registrable domain.
pub fn etld_plus_one(host: &str) -> Option<String> {
    let host = host_of(host)?;
    // psl::domain_str returns the registrable domain (eTLD+1) using the
    // embedded Public Suffix List, or None for bare suffixes / non-domains.
    psl::domain_str(&host).map(|d| d.to_ascii_lowercase())
}

/// True when two origins/hosts share the same registrable domain (eTLD+1).
///
/// Fail-closed: if *either* side has no derivable eTLD+1, returns `false`.
pub fn same_etld_plus_one(a: &str, b: &str) -> bool {
    match (etld_plus_one(a), etld_plus_one(b)) {
        (Some(a1), Some(b1)) => a1 == b1,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- host_of: scheme / port / case / userinfo / path normalization ----

    #[test]
    fn host_of_strips_scheme_and_lowercases() {
        assert_eq!(host_of("https://Sub.X.COM").as_deref(), Some("sub.x.com"));
        assert_eq!(host_of("HTTP://EXAMPLE.com").as_deref(), Some("example.com"));
    }

    #[test]
    fn host_of_strips_port() {
        assert_eq!(host_of("https://login.x.com:8443").as_deref(), Some("login.x.com"));
        assert_eq!(host_of("x.com:443").as_deref(), Some("x.com"));
    }

    #[test]
    fn host_of_strips_path_query_fragment() {
        assert_eq!(host_of("https://x.com/login?next=1#frag").as_deref(), Some("x.com"));
    }

    #[test]
    fn host_of_strips_userinfo() {
        assert_eq!(host_of("https://user:redacted@example.invalid/path").as_deref(), Some("x.com"));
    }

    #[test]
    fn host_of_handles_bare_host() {
        assert_eq!(host_of("x.com").as_deref(), Some("x.com"));
    }

    #[test]
    fn host_of_strips_trailing_dot() {
        assert_eq!(host_of("https://x.com.").as_deref(), Some("x.com"));
    }

    #[test]
    fn host_of_ipv6_literal_with_port() {
        assert_eq!(host_of("http://[::1]:8080").as_deref(), Some("::1"));
        // IPv6 has no eTLD+1, so domain binding fail-closes downstream.
        assert_eq!(etld_plus_one("http://[::1]:8080"), None);
    }

    #[test]
    fn host_of_empty_is_none() {
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("   "), None);
        assert_eq!(host_of("https://"), None);
    }

    // ---- etld_plus_one: real PSL behavior (the load-bearing correctness) ----

    #[test]
    fn etld_simple_two_label() {
        assert_eq!(etld_plus_one("x.com").as_deref(), Some("x.com"));
    }

    #[test]
    fn etld_collapses_subdomains() {
        assert_eq!(etld_plus_one("sub.x.com").as_deref(), Some("x.com"));
        assert_eq!(etld_plus_one("https://sub.login.x.com").as_deref(), Some("x.com"));
    }

    #[test]
    fn etld_multilevel_suffix_co_uk_is_not_collapsed() {
        // The critical PSL test: co.uk IS a public suffix, so the eTLD+1 of
        // a.co.uk is a.co.uk — NOT co.uk. A naive last-two-labels heuristic
        // would wrongly make a.co.uk and b.co.uk share an eTLD+1.
        assert_eq!(etld_plus_one("a.co.uk").as_deref(), Some("a.co.uk"));
        assert_eq!(etld_plus_one("b.co.uk").as_deref(), Some("b.co.uk"));
        assert_ne!(etld_plus_one("a.co.uk"), etld_plus_one("b.co.uk"));
        // Subdomains below the registrable domain still collapse correctly.
        assert_eq!(etld_plus_one("www.a.co.uk").as_deref(), Some("a.co.uk"));
    }

    #[test]
    fn etld_multilevel_suffix_com_cn() {
        assert_eq!(etld_plus_one("shop.example.com.cn").as_deref(), Some("example.com.cn"));
        assert_ne!(etld_plus_one("a.com.cn"), etld_plus_one("b.com.cn"));
    }

    #[test]
    fn etld_bare_public_suffix_is_none() {
        assert_eq!(etld_plus_one("co.uk"), None);
        assert_eq!(etld_plus_one("com"), None);
    }

    #[test]
    fn etld_is_lowercased() {
        assert_eq!(etld_plus_one("SUB.X.COM").as_deref(), Some("x.com"));
    }

    // ---- same_etld_plus_one ----

    #[test]
    fn same_etld_subdomain_matches_parent() {
        assert!(same_etld_plus_one("https://login.x.com", "x.com"));
        assert!(same_etld_plus_one("https://sub.x.com:8443/path", "https://other.x.com"));
    }

    #[test]
    fn same_etld_cross_domain_does_not_match() {
        assert!(!same_etld_plus_one("x.com", "y.com"));
        assert!(!same_etld_plus_one("https://evil.com", "x.com"));
    }

    #[test]
    fn same_etld_distinct_co_uk_registrables_do_not_match() {
        // Proves binding is on real eTLD+1, not the shared public suffix co.uk.
        assert!(!same_etld_plus_one("a.co.uk", "b.co.uk"));
        assert!(same_etld_plus_one("www.a.co.uk", "mail.a.co.uk"));
    }

    #[test]
    fn same_etld_fail_closed_on_unparseable() {
        assert!(!same_etld_plus_one("co.uk", "co.uk")); // bare suffix → None → false
        assert!(!same_etld_plus_one("", "x.com"));
    }
}
