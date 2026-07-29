//! Declarative authentication: the connection profile *declares* a scheme,
//! the transport layer *applies* it. Adapters never hand-roll auth headers.
//!
//! The credentials JSON shape is determined by the scheme:
//! - `bearer` / `token` / `header_key:<name>` / `query_key:<param>` use one
//!   secret, read via [`AuthMaterial::primary_secret`];
//! - `volc_voice` (a built-in alias for a [`AuthScheme::MultiHeader`] template)
//!   reads named credential fields per header.

use crate::error::InvokeError;

/// How credentials are attached to an outgoing request.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthScheme {
    /// `Authorization: Bearer <key>` (most providers).
    Bearer,
    /// `Authorization: Token <key>` (Deepgram).
    TokenHeader,
    /// `<header>: <key>` (e.g. `x-goog-api-key`, `xi-api-key`).
    HeaderKey(String),
    /// Multiple headers, each fed from a named credentials field:
    /// `(header_name, credentials_field)` pairs (Volcano voice v3 X-Api-*).
    MultiHeader(Vec<(String, String)>),
    /// `?<param>=<key>` query-string key.
    QueryKey(String),
}

impl AuthScheme {
    /// Parse the persisted scheme vocabulary:
    /// `"bearer" | "token" | "header_key:<name>" | "query_key:<param>" | "volc_voice"`.
    ///
    /// `volc_voice` is a built-in alias expanding to the Volcano voice v3
    /// [`AuthScheme::MultiHeader`] template
    /// (`X-Api-App-Key`/`X-Api-Access-Key`/`X-Api-Resource-Id` fed from the
    /// `app_key`/`access_key`/`resource_id` credential fields).
    pub fn parse(s: &str) -> Result<Self, InvokeError> {
        let s = s.trim();
        if let Some(name) = s.strip_prefix("header_key:") {
            let name = name.trim();
            if name.is_empty() {
                return Err(InvokeError::config("auth scheme \"header_key:\" requires a header name"));
            }
            return Ok(Self::HeaderKey(name.to_string()));
        }
        if let Some(param) = s.strip_prefix("query_key:") {
            let param = param.trim();
            if param.is_empty() {
                return Err(InvokeError::config("auth scheme \"query_key:\" requires a query parameter name"));
            }
            return Ok(Self::QueryKey(param.to_string()));
        }
        match s {
            "bearer" => Ok(Self::Bearer),
            "token" => Ok(Self::TokenHeader),
            // Built-in alias: Volcano voice v3 multi-header template (T9).
            "volc_voice" => Ok(Self::MultiHeader(vec![
                ("X-Api-App-Key".into(), "app_key".into()),
                ("X-Api-Access-Key".into(), "access_key".into()),
                ("X-Api-Resource-Id".into(), "resource_id".into()),
            ])),
            other => Err(InvokeError::config(format!("unknown auth scheme: {other:?}"))),
        }
    }
}

/// A scheme plus the (decrypted) credentials JSON that feeds it.
/// Deliberately not `Debug`: `credentials` holds live secrets.
#[derive(Clone)]
pub struct AuthMaterial {
    pub scheme: AuthScheme,
    pub credentials: serde_json::Value,
}

impl AuthMaterial {
    /// The single secret used by `bearer`/`token`/`header_key`/`query_key`
    /// schemes: `credentials["api_keys"][0]`, falling back to a bare
    /// `{"api_key": "..."}` shape. (`providers` 行 default 连接 stores
    /// `{"api_keys": [first comma/newline-separated key]}`.)
    pub fn primary_secret(&self) -> Result<String, InvokeError> {
        let from_list = self
            .credentials
            .get("api_keys")
            .and_then(|v| v.as_array())
            .and_then(|keys| keys.first())
            .and_then(|v| v.as_str());
        let bare = self.credentials.get("api_key").and_then(|v| v.as_str());
        from_list
            .or(bare)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                InvokeError::config(
                    "connection credentials carry no api key (expected {\"api_keys\": [..]} or {\"api_key\": ..})",
                )
            })
    }

    /// Attach the credentials to an outgoing request per the scheme.
    pub fn apply(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, InvokeError> {
        use reqwest::header::AUTHORIZATION;
        match &self.scheme {
            AuthScheme::Bearer => {
                let value = header_value(&format!("Bearer {}", self.primary_secret()?))?;
                Ok(rb.header(AUTHORIZATION, value))
            }
            AuthScheme::TokenHeader => {
                let value = header_value(&format!("Token {}", self.primary_secret()?))?;
                Ok(rb.header(AUTHORIZATION, value))
            }
            AuthScheme::HeaderKey(name) => {
                let value = header_value(&self.primary_secret()?)?;
                Ok(rb.header(header_name(name)?, value))
            }
            AuthScheme::QueryKey(param) => {
                let secret = self.primary_secret()?;
                Ok(rb.query(&[(param.as_str(), secret.as_str())]))
            }
            AuthScheme::MultiHeader(pairs) => {
                let mut rb = rb;
                for (header, field) in pairs {
                    let value = self
                        .credentials
                        .get(field)
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            InvokeError::config(format!(
                                "connection credentials missing field {field:?} required by auth header {header:?}"
                            ))
                        })?;
                    rb = rb.header(header_name(header)?, header_value(value)?);
                }
                Ok(rb)
            }
        }
    }
}

/// Validate a header name up front so a bad scheme surfaces as a typed
/// config error instead of a deferred reqwest build failure.
fn header_name(name: &str) -> Result<reqwest::header::HeaderName, InvokeError> {
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map_err(|e| InvokeError::config(format!("invalid auth header name {name:?}: {e}")))
}

/// Validate a header value without ever folding the secret into the error.
fn header_value(value: &str) -> Result<reqwest::header::HeaderValue, InvokeError> {
    let mut hv = reqwest::header::HeaderValue::from_str(value)
        .map_err(|_| InvokeError::config("auth credential contains characters not valid in an HTTP header"))?;
    hv.set_sensitive(true);
    Ok(hv)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::InvokeErrorKind;

    fn material(scheme: AuthScheme, credentials: serde_json::Value) -> AuthMaterial {
        AuthMaterial { scheme, credentials }
    }

    fn build(m: &AuthMaterial) -> reqwest::Request {
        let rb = reqwest::Client::new().get("https://example.test/x");
        m.apply(rb).expect("apply").build().expect("build")
    }

    #[test]
    fn parse_covers_full_vocabulary() {
        assert_eq!(AuthScheme::parse("bearer").unwrap(), AuthScheme::Bearer);
        assert_eq!(AuthScheme::parse("token").unwrap(), AuthScheme::TokenHeader);
        assert_eq!(
            AuthScheme::parse("header_key:x-goog-api-key").unwrap(),
            AuthScheme::HeaderKey("x-goog-api-key".into())
        );
        assert_eq!(AuthScheme::parse("query_key:key").unwrap(), AuthScheme::QueryKey("key".into()));
        assert_eq!(
            AuthScheme::parse("volc_voice").unwrap(),
            AuthScheme::MultiHeader(vec![
                ("X-Api-App-Key".into(), "app_key".into()),
                ("X-Api-Access-Key".into(), "access_key".into()),
                ("X-Api-Resource-Id".into(), "resource_id".into()),
            ])
        );
    }

    #[test]
    fn parse_rejects_unknown_and_empty_arguments() {
        for bad in ["", "basic", "header_key:", "query_key:", "bearer "] {
            // "bearer " (trailing space) must still parse after trim; treat separately.
            if bad == "bearer " {
                assert_eq!(AuthScheme::parse(bad).unwrap(), AuthScheme::Bearer);
                continue;
            }
            let err = AuthScheme::parse(bad).unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::Config, "input {bad:?}");
        }
    }

    #[test]
    fn primary_secret_prefers_api_keys_then_bare_api_key() {
        let m = material(AuthScheme::Bearer, json!({"api_keys": ["sk-1", "sk-2"]}));
        assert_eq!(m.primary_secret().unwrap(), "sk-1");

        let bare = material(AuthScheme::Bearer, json!({"api_key": "sk-raw"}));
        assert_eq!(bare.primary_secret().unwrap(), "sk-raw");

        for empty in [json!({}), json!({"api_keys": []}), json!({"api_keys": [""]})] {
            let m = material(AuthScheme::Bearer, empty.clone());
            let err = m.primary_secret().unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::Config, "credentials {empty}");
        }
    }

    #[test]
    fn apply_bearer_sets_authorization_header() {
        let req = build(&material(AuthScheme::Bearer, json!({"api_keys": ["sk-1"]})));
        assert_eq!(req.headers().get("authorization").unwrap(), "Bearer sk-1");
    }

    #[test]
    fn apply_token_sets_authorization_header() {
        let req = build(&material(AuthScheme::TokenHeader, json!({"api_key": "dg-1"})));
        assert_eq!(req.headers().get("authorization").unwrap(), "Token dg-1");
    }

    #[test]
    fn apply_header_key_sets_named_header() {
        let req = build(&material(AuthScheme::HeaderKey("x-goog-api-key".into()), json!({"api_keys": ["g-1"]})));
        assert_eq!(req.headers().get("x-goog-api-key").unwrap(), "g-1");
        assert!(req.headers().get("authorization").is_none());
    }

    #[test]
    fn apply_query_key_appends_query_param() {
        let req = build(&material(AuthScheme::QueryKey("key".into()), json!({"api_keys": ["q-1"]})));
        assert_eq!(req.url().query(), Some("key=q-1"));
    }

    #[test]
    fn apply_multi_header_injects_each_pair() {
        let scheme = AuthScheme::parse("volc_voice").unwrap();
        let creds = json!({"app_key": "a", "access_key": "b", "resource_id": "volc.bigasr.auc"});
        let req = build(&material(scheme, creds));
        assert_eq!(req.headers().get("X-Api-App-Key").unwrap(), "a");
        assert_eq!(req.headers().get("X-Api-Access-Key").unwrap(), "b");
        assert_eq!(req.headers().get("X-Api-Resource-Id").unwrap(), "volc.bigasr.auc");
    }

    #[test]
    fn apply_multi_header_missing_field_is_config_error() {
        let scheme = AuthScheme::parse("volc_voice").unwrap();
        let m = material(scheme, json!({"app_key": "a"}));
        let rb = reqwest::Client::new().get("https://example.test/x");
        let err = m.apply(rb).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("access_key"), "message: {}", err.message);
    }

    #[test]
    fn apply_without_secret_is_config_error() {
        let m = material(AuthScheme::Bearer, json!({}));
        let rb = reqwest::Client::new().get("https://example.test/x");
        assert_eq!(m.apply(rb).unwrap_err().kind, InvokeErrorKind::Config);
    }
}
