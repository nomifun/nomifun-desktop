use axum::http::HeaderMap;

use nomifun_common::AppError;
use nomifun_common::constants::{COOKIE_NAME, CSRF_COOKIE_NAME, SESSION_MAX_AGE_SECONDS};

/// Cookie security configuration derived from the deployment environment.
#[derive(Debug, Clone)]
pub struct CookieConfig {
    /// Whether to set the `Secure` flag on cookies (HTTPS only).
    pub secure: bool,
    /// `SameSite` policy: `"Strict"` for HTTPS, `"Lax"` for HTTP.
    pub same_site: &'static str,
}

impl CookieConfig {
    /// Create cookie config from environment variables.
    ///
    /// - `NOMIFUN_HTTPS=true` → Secure flag, SameSite=Strict
    /// - Otherwise → no Secure flag, SameSite=Lax (for remote HTTP access)
    pub fn from_env() -> Self {
        let https = std::env::var("NOMIFUN_HTTPS")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self {
            secure: https,
            same_site: if https { "Strict" } else { "Lax" },
        }
    }

    /// Build `Set-Cookie` header value for the session token.
    ///
    /// Attributes: HttpOnly, SameSite, Secure (if HTTPS), Max-Age=30d.
    pub fn build_session_cookie(&self, token: &str) -> String {
        let max_age = SESSION_MAX_AGE_SECONDS;
        format!(
            "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite={}{}; Max-Age={max_age}",
            self.same_site,
            if self.secure { "; Secure" } else { "" },
        )
    }

    /// Build `Set-Cookie` header value that clears the session cookie.
    pub fn clear_session_cookie(&self) -> String {
        format!(
            "{COOKIE_NAME}=; Path=/; HttpOnly; SameSite={}{}; Max-Age=0",
            self.same_site,
            if self.secure { "; Secure" } else { "" },
        )
    }

    /// Build `Set-Cookie` header value for the CSRF token.
    ///
    /// NOT HttpOnly — JavaScript must read this value to include it
    /// in the `x-csrf-token` request header (Double Submit Cookie pattern).
    pub fn build_csrf_cookie(&self, token: &str) -> String {
        let max_age = SESSION_MAX_AGE_SECONDS;
        format!(
            "{CSRF_COOKIE_NAME}={token}; Path=/; SameSite={}{}; Max-Age={max_age}",
            self.same_site,
            if self.secure { "; Secure" } else { "" },
        )
    }

    /// Reject a login-family request that arrived over plain HTTP while this
    /// deployment issues `Secure` cookies (`NOMIFUN_HTTPS=true`).
    ///
    /// A browser silently discards a `Secure` cookie set over http://, so the
    /// login would "succeed" and then every request 401s — an unexplained
    /// login loop on exactly the machines using the plain-HTTP access path
    /// (LAN `http://ip:8787` beside a Caddy TLS domain). Fail loudly at the
    /// only moment the problem is diagnosable instead
    /// (audit 2026-07-30, finding D).
    ///
    /// Plaintext detection is header-based: a TLS-terminating proxy sets
    /// `X-Forwarded-Proto: https` (Caddy does by default), and browsers send
    /// an `https://` `Origin` on fetch POSTs. If either says https, the
    /// request is allowed. Non-browser clients (no Origin) that also lack
    /// forwarded headers still pass — they use the response-body token, not
    /// the cookie, so the trap does not apply to them.
    pub fn reject_plaintext_login_when_secure(&self, headers: &HeaderMap) -> Result<(), AppError> {
        if !self.secure {
            return Ok(());
        }

        let forwarded_proto = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            // May be a list when crossing several proxies; the first entry is
            // the client-facing scheme.
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_ascii_lowercase());
        if forwarded_proto.as_deref() == Some("https") {
            return Ok(());
        }

        let origin = headers
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase);
        match origin.as_deref() {
            // Browser page served over TLS → cookie will stick.
            Some(origin) if origin.starts_with("https://") => Ok(()),
            // Browser page served over plain HTTP → the Secure cookie is
            // guaranteed to be dropped. Refuse with an actionable message.
            Some(_) => Err(AppError::BadRequest(
                "This server is configured for HTTPS (NOMIFUN_HTTPS=true), but the page was loaded over \
                 plain HTTP, so the browser would silently discard the session cookie and sign-in would \
                 loop forever. Open the site via its https:// address, or unset NOMIFUN_HTTPS if this \
                 deployment is intentionally HTTP-only."
                    .into(),
            )),
            // No Origin: not a browser form/fetch (curl, native client, same
            // -origin GET). The cookie trap does not apply; let it through.
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_config() -> CookieConfig {
        CookieConfig {
            secure: false,
            same_site: "Lax",
        }
    }

    fn https_config() -> CookieConfig {
        CookieConfig {
            secure: true,
            same_site: "Strict",
        }
    }

    #[test]
    fn session_cookie_http() {
        let cookie = http_config().build_session_cookie("my_token");
        assert!(cookie.contains("nomifun-session=my_token"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age="));
        assert!(!cookie.contains("Secure"));
    }

    #[test]
    fn session_cookie_https() {
        let cookie = https_config().build_session_cookie("my_token");
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("; Secure"));
    }

    #[test]
    fn clear_session_cookie_sets_max_age_zero() {
        let cookie = http_config().clear_session_cookie();
        assert!(cookie.contains("nomifun-session="));
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[test]
    fn csrf_cookie_not_http_only() {
        let cookie = http_config().build_csrf_cookie("csrf_abc");
        assert!(cookie.contains("nomifun-csrf-token=csrf_abc"));
        assert!(!cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Max-Age="));
    }

    #[test]
    fn csrf_cookie_https_has_secure() {
        let cookie = https_config().build_csrf_cookie("csrf_abc");
        assert!(cookie.contains("; Secure"));
        assert!(cookie.contains("SameSite=Strict"));
    }

    #[test]
    fn session_cookie_max_age_30_days() {
        let cookie = http_config().build_session_cookie("t");
        let expected = 30 * 24 * 60 * 60;
        assert!(cookie.contains(&format!("Max-Age={expected}")));
    }

    // -- reject_plaintext_login_when_secure ----------------------------------

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for &(name, value) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn http_deployment_never_rejects() {
        let config = http_config();
        assert!(config
            .reject_plaintext_login_when_secure(&headers_with(&[("origin", "http://192.168.1.5:8787")]))
            .is_ok());
        assert!(config.reject_plaintext_login_when_secure(&HeaderMap::new()).is_ok());
    }

    #[test]
    fn https_page_passes_via_origin() {
        let config = https_config();
        assert!(config
            .reject_plaintext_login_when_secure(&headers_with(&[("origin", "https://nomi.example.com")]))
            .is_ok());
    }

    #[test]
    fn tls_terminating_proxy_passes_via_forwarded_proto() {
        let config = https_config();
        // Caddy in front: page Origin may even be missing on same-origin
        // requests in some browsers; X-Forwarded-Proto is authoritative.
        assert!(config
            .reject_plaintext_login_when_secure(&headers_with(&[
                ("x-forwarded-proto", "https"),
                ("origin", "https://nomi.example.com"),
            ]))
            .is_ok());
        assert!(config
            .reject_plaintext_login_when_secure(&headers_with(&[("x-forwarded-proto", "https, http")]))
            .is_ok());
    }

    #[test]
    fn plain_http_browser_login_is_rejected_with_actionable_error() {
        let config = https_config();
        let err = config
            .reject_plaintext_login_when_secure(&headers_with(&[("origin", "http://192.168.1.5:8787")]))
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("NOMIFUN_HTTPS"), "got: {message}");
        assert!(message.contains("https://"), "got: {message}");
    }

    #[test]
    fn non_browser_clients_without_origin_pass() {
        // curl / native clients use the response-body token, not the cookie;
        // the Secure-cookie trap does not apply to them.
        let config = https_config();
        assert!(config.reject_plaintext_login_when_secure(&HeaderMap::new()).is_ok());
    }
}
