//! Declarative authentication: the connection profile *declares* a scheme,
//! the transport layer *applies* it. Adapters never hand-roll auth headers.
//!
//! The credentials JSON shape is determined by the scheme:
//! - `bearer` / `token` / `header_key:<name>` / `query_key:<param>` use the
//!   ordered `api_keys` ring and rotate only on retryable auth/rate failures;
//! - `volc_voice` (a built-in alias for a [`AuthScheme::MultiHeader`] template)
//!   reads named credential fields per header;
//! - `bedrock` reads either an empty SDK-chain/profile object or an exact AWS
//!   access-key object and is never attached by generic HTTP transport.

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
    /// AWS SDK/SigV4 credentials from the default connection's typed encrypted
    /// credential object. Non-secret region/profile selection remains in
    /// `providers.bedrock_config`; this scheme is never attached by generic
    /// HTTP transport.
    Bedrock,
}

impl AuthScheme {
    /// Whether the scheme draws on the `api_keys` ARRAY and is therefore
    /// eligible for multi-key rotation
    /// ([`crate::transport::send_with_rotation`]). [`AuthScheme::MultiHeader`]
    /// credentials are one named-field object (not a key list) — single-shot;
    /// the same will hold for any future body-embedded scheme.
    pub fn rotates(&self) -> bool {
        match self {
            Self::Bearer | Self::TokenHeader | Self::HeaderKey(_) | Self::QueryKey(_) => true,
            Self::MultiHeader(_) | Self::Bedrock => false,
        }
    }

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
            "bedrock" => Ok(Self::Bedrock),
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
    /// Exact redaction authority for every string in the decrypted credential
    /// payload. This includes all rotation keys, multi-header fields, and AWS
    /// access/secret/session values.
    pub(crate) fn secret_redactor(&self) -> nomifun_net::secret_redaction::SecretRedactor {
        fn collect(value: &serde_json::Value, secrets: &mut Vec<String>) {
            match value {
                serde_json::Value::String(value) => secrets.push(value.clone()),
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect(value, secrets);
                    }
                }
                serde_json::Value::Object(values) => {
                    for value in values.values() {
                        collect(value, secrets);
                    }
                }
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
            }
        }

        let mut secrets = Vec::new();
        collect(&self.credentials, &mut secrets);
        nomifun_net::secret_redaction::SecretRedactor::new(secrets)
    }

    /// Validate the persisted credentials JSON against the parsed scheme
    /// without sending a request.
    ///
    /// Connection-management code calls this before encrypting a named
    /// connection profile. Keeping the schema checks next to the transport
    /// implementation prevents the management API and invocation path from
    /// drifting apart (the previous API accepted any non-empty object and
    /// deferred errors until the first real model call).
    ///
    /// Single-key schemes use exactly `{"api_keys":[...]}`. Multi-header
    /// schemes require every named field declared by the scheme.
    pub fn validate_credentials(&self) -> Result<(), InvokeError> {
        match &self.scheme {
            AuthScheme::Bedrock => {
                let object = self.credentials.as_object().ok_or_else(|| {
                    InvokeError::config("bedrock credentials must be a JSON object")
                })?;
                if let Some(field) = object.keys().find(|field| {
                    !matches!(
                        field.as_str(),
                        "access_key_id" | "secret_access_key" | "session_token"
                    )
                }) {
                    return Err(InvokeError::config(format!(
                        "bedrock credentials contain unsupported field {field:?}"
                    )));
                }
                if object.is_empty() {
                    return Ok(());
                }
                for field in ["access_key_id", "secret_access_key"] {
                    object
                        .get(field)
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            InvokeError::config(format!(
                                "bedrock credentials missing non-empty string field {field:?}"
                            ))
                        })?;
                }
                if object.contains_key("session_token") {
                    object
                        .get("session_token")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            InvokeError::config(
                                "bedrock credentials session_token must be a non-empty string",
                            )
                        })?;
                }
                Ok(())
            }
            AuthScheme::MultiHeader(pairs) => {
                let object = self.credentials.as_object().ok_or_else(|| {
                    InvokeError::config("connection credentials must be a JSON object")
                })?;
                if object.is_empty() {
                    return Err(InvokeError::config(
                        "connection credentials must be a non-empty JSON object",
                    ));
                }
                if let Some(field) = object
                    .keys()
                    .find(|field| !pairs.iter().any(|(_, required)| required == *field))
                {
                    return Err(InvokeError::config(format!(
                        "connection credentials contain unsupported field {field:?}"
                    )));
                }
                for (header, field) in pairs {
                    let value = object
                        .get(field)
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| {
                            InvokeError::config(format!(
                                "connection credentials missing non-empty string field {field:?} required by auth header {header:?}"
                            ))
                        })?;
                    header_name(header)?;
                    header_value(value)?;
                }
                Ok(())
            }
            AuthScheme::Bearer
            | AuthScheme::TokenHeader
            | AuthScheme::HeaderKey(_)
            | AuthScheme::QueryKey(_) => {
                let secrets = validated_single_key_credentials(&self.credentials)?;
                match &self.scheme {
                    AuthScheme::Bearer => {
                        for secret in secrets {
                            header_value(&format!("Bearer {secret}"))?;
                        }
                    }
                    AuthScheme::TokenHeader => {
                        for secret in secrets {
                            header_value(&format!("Token {secret}"))?;
                        }
                    }
                    AuthScheme::HeaderKey(name) => {
                        header_name(name)?;
                        for secret in secrets {
                            header_value(secret)?;
                        }
                    }
                    AuthScheme::QueryKey(param) => {
                        if param.trim().is_empty() {
                            return Err(InvokeError::config(
                                "query-key auth requires a non-empty query parameter name",
                            ));
                        }
                    }
                    AuthScheme::MultiHeader(_) => unreachable!("covered by outer match"),
                    AuthScheme::Bedrock => unreachable!("covered by outer match"),
                }
                Ok(())
            }
        }
    }

    /// Validate that this auth declaration can produce a request without
    /// sending one. Capability discovery and artifact materialization share
    /// the strict credential schema used by connection management.
    pub fn validate(&self) -> Result<(), InvokeError> {
        self.validate_credentials()
    }

    /// Every rotation-eligible secret, in stored order, from the sole
    /// `credentials["api_keys"]` array.
    pub fn secrets(&self) -> Vec<String> {
        let Some(object) = self.credentials.as_object() else {
            return Vec::new();
        };
        if object.keys().any(|key| key != "api_keys") {
            return Vec::new();
        }
        let from_list: Vec<String> = self
            .credentials
            .get("api_keys")
            .and_then(|v| v.as_array())
            .map(|keys| {
                keys.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        from_list
    }

    /// The primary secret used by single-attempt helpers. Runtime HTTP
    /// dispatch normally consumes the complete [`AuthMaterial::secrets`] ring
    /// through the rotation helper instead.
    pub fn primary_secret(&self) -> Result<String, InvokeError> {
        self.secrets().into_iter().next().ok_or_else(|| {
            InvokeError::config(
                "connection credentials carry no api key (expected {\"api_keys\": [..]})",
            )
        })
    }

    /// Attach ONE specific secret per the (single-key) scheme. The rotation
    /// helper ([`crate::transport::send_with_rotation`]) drives this with each
    /// entry of [`AuthMaterial::secrets`] in turn; [`AuthMaterial::apply`]
    /// drives it with the primary secret. [`AuthScheme::MultiHeader`] does not
    /// take a single secret — calling this with it is a config error.
    pub(crate) fn apply_with_secret(
        &self,
        rb: reqwest::RequestBuilder,
        secret: &str,
    ) -> Result<reqwest::RequestBuilder, InvokeError> {
        use reqwest::header::AUTHORIZATION;
        match &self.scheme {
            AuthScheme::Bearer => Ok(rb.header(AUTHORIZATION, header_value(&format!("Bearer {secret}"))?)),
            AuthScheme::TokenHeader => Ok(rb.header(AUTHORIZATION, header_value(&format!("Token {secret}"))?)),
            AuthScheme::HeaderKey(name) => Ok(rb.header(header_name(name)?, header_value(secret)?)),
            AuthScheme::QueryKey(param) => Ok(rb.query(&[(param.as_str(), secret)])),
            AuthScheme::MultiHeader(_) => {
                Err(InvokeError::config("multi-header auth does not take a single secret"))
            }
            AuthScheme::Bedrock => Err(InvokeError::config(
                "bedrock auth is available only through the SDK-backed Chat protocol",
            )),
        }
    }

    /// Attach the credentials to an outgoing request per the scheme
    /// (single-key schemes use the primary secret).
    pub fn apply(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, InvokeError> {
        match &self.scheme {
            AuthScheme::Bedrock => Err(InvokeError::config(
                "bedrock auth is available only through the SDK-backed Chat protocol",
            )),
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
            _ => {
                let secret = self.primary_secret()?;
                self.apply_with_secret(rb, &secret)
            }
        }
    }
}

/// Strict parser for the one persisted single-key credential shape.
fn validated_single_key_credentials(
    credentials: &serde_json::Value,
) -> Result<Vec<&str>, InvokeError> {
    let object = credentials
        .as_object()
        .ok_or_else(|| InvokeError::config("connection credentials must be a JSON object"))?;

    if object.keys().any(|key| key != "api_keys") {
        return Err(InvokeError::config(
            "single-key connection credentials accept only the \"api_keys\" field",
        ));
    }
    let values = object
        .get("api_keys")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            InvokeError::config(
                "connection credentials field \"api_keys\" must be an array of strings",
            )
        })?;
    let mut secrets = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let secret = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                InvokeError::config(format!(
                    "connection credentials field \"api_keys\" entry {index} must be a non-empty string"
                ))
            })?;
        secrets.push(secret);
    }
    if secrets.is_empty() {
        return Err(InvokeError::config(
            "connection credentials carry no api key (expected {\"api_keys\": [..]})",
        ));
    }
    Ok(secrets)
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
    fn primary_secret_uses_first_api_keys_entry() {
        let m = material(AuthScheme::Bearer, json!({"api_keys": ["sk-1", "sk-2"]}));
        assert_eq!(m.primary_secret().unwrap(), "sk-1");

        for empty in [
            json!({}),
            json!({"api_keys": []}),
            json!({"api_keys": [""]}),
            json!({"api_key": "sk-raw"}),
        ] {
            let m = material(AuthScheme::Bearer, empty.clone());
            let err = m.primary_secret().unwrap_err();
            assert_eq!(err.kind, InvokeErrorKind::Config, "credentials {empty}");
        }
    }

    #[test]
    fn secrets_returns_only_the_api_keys_array() {
        let m = material(AuthScheme::Bearer, json!({"api_keys": [" sk-1 ", "", "sk-2"]}));
        assert_eq!(m.secrets(), vec!["sk-1".to_string(), "sk-2".to_string()]);

        for empty in [
            json!({}),
            json!({"api_keys": []}),
            json!({"api_keys": ["", "  "]}),
            json!({"api_key": "sk-raw"}),
        ] {
            let m = material(AuthScheme::Bearer, empty.clone());
            assert!(m.secrets().is_empty(), "credentials {empty}");
        }
    }

    #[test]
    fn validate_distinguishes_complete_and_incomplete_auth_material() {
        assert!(
            material(AuthScheme::Bearer, json!({"api_keys": ["sk-ready"]}))
                .validate()
                .is_ok()
        );
        assert_eq!(
            material(AuthScheme::Bearer, json!({})).validate().unwrap_err().kind,
            InvokeErrorKind::Config
        );

        let multi = AuthScheme::parse("volc_voice").unwrap();
        let incomplete = material(
            multi.clone(),
            json!({"app_key": "app", "access_key": "access"}),
        );
        assert!(incomplete.validate().unwrap_err().message.contains("resource_id"));
        assert!(
            material(
                multi,
                json!({
                    "app_key": "app",
                    "access_key": "access",
                    "resource_id": "resource"
                }),
            )
            .validate()
            .is_ok()
        );
    }


    #[test]
    fn rotation_eligibility_covers_array_key_schemes_only() {
        assert!(AuthScheme::Bearer.rotates());
        assert!(AuthScheme::TokenHeader.rotates());
        assert!(AuthScheme::HeaderKey("x-goog-api-key".into()).rotates());
        assert!(AuthScheme::QueryKey("key".into()).rotates());
        assert!(!AuthScheme::parse("volc_voice").unwrap().rotates());
    }

    #[test]
    fn apply_with_secret_attaches_the_given_key_per_scheme() {
        let creds = json!({"api_keys": ["sk-1", "sk-2"]});
        let build = |m: &AuthMaterial, secret: &str| {
            let rb = reqwest::Client::new().get("https://example.test/x");
            m.apply_with_secret(rb, secret).expect("apply_with_secret").build().expect("build")
        };
        let bearer = material(AuthScheme::Bearer, creds.clone());
        assert_eq!(build(&bearer, "sk-2").headers().get("authorization").unwrap(), "Bearer sk-2");
        let token = material(AuthScheme::TokenHeader, creds.clone());
        assert_eq!(build(&token, "sk-2").headers().get("authorization").unwrap(), "Token sk-2");
        let header = material(AuthScheme::HeaderKey("xi-api-key".into()), creds.clone());
        assert_eq!(build(&header, "sk-2").headers().get("xi-api-key").unwrap(), "sk-2");
        let query = material(AuthScheme::QueryKey("key".into()), creds.clone());
        assert_eq!(build(&query, "sk-2").url().query(), Some("key=sk-2"));

        // MultiHeader has no single-secret form.
        let multi = material(AuthScheme::parse("volc_voice").unwrap(), json!({"app_key": "a"}));
        let rb = reqwest::Client::new().get("https://example.test/x");
        assert_eq!(multi.apply_with_secret(rb, "sk-2").unwrap_err().kind, InvokeErrorKind::Config);
    }

    #[test]
    fn apply_bearer_sets_authorization_header() {
        let req = build(&material(AuthScheme::Bearer, json!({"api_keys": ["sk-1"]})));
        assert_eq!(req.headers().get("authorization").unwrap(), "Bearer sk-1");
    }

    #[test]
    fn apply_token_sets_authorization_header() {
        let req = build(&material(AuthScheme::TokenHeader, json!({"api_keys": ["dg-1"]})));
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

    #[test]
    fn save_time_validation_accepts_only_the_api_keys_shape() {
        for scheme in [
            AuthScheme::Bearer,
            AuthScheme::TokenHeader,
            AuthScheme::HeaderKey("x-api-key".into()),
            AuthScheme::QueryKey("key".into()),
        ] {
            material(scheme.clone(), json!({"api_keys": [" key-1 ", "key-2"]}))
                .validate_credentials()
                .unwrap();
            assert!(
                material(scheme, json!({"api_key": "old-key"}))
                    .validate_credentials()
                    .is_err()
            );
        }
    }

    #[test]
    fn save_time_validation_rejects_malformed_single_key_shapes() {
        for credentials in [
            json!("raw-key"),
            json!({}),
            json!({"api_keys": "key"}),
            json!({"api_keys": ["key", 2]}),
            json!({"api_keys": [" "]}),
            json!({"api_key": 2}),
            json!({"api_key": " "}),
        ] {
            let error = material(AuthScheme::Bearer, credentials.clone())
                .validate_credentials()
                .unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::Config, "credentials {credentials}");
        }
    }

    #[test]
    fn save_time_validation_checks_every_multi_header_field_and_header_value() {
        let scheme = AuthScheme::parse("volc_voice").unwrap();
        material(
            scheme.clone(),
            json!({
                "app_key": "app",
                "access_key": "access",
                "resource_id": "volc.bigasr.auc"
            }),
        )
        .validate_credentials()
        .unwrap();

        for credentials in [
            json!({"app_key": "app", "access_key": "access"}),
            json!({"app_key": "app", "access_key": 1, "resource_id": "resource"}),
            json!({"app_key": "app\ninvalid", "access_key": "access", "resource_id": "resource"}),
        ] {
            let error = material(scheme.clone(), credentials)
                .validate_credentials()
                .unwrap_err();
            assert_eq!(error.kind, InvokeErrorKind::Config);
        }
    }

    #[test]
    fn bedrock_credentials_allow_profile_marker_or_sts_payload_only() {
        material(AuthScheme::Bedrock, json!({}))
            .validate_credentials()
            .unwrap();
        material(
            AuthScheme::Bedrock,
            json!({
                "access_key_id": "AKIA_TEST",
                "secret_access_key": "secret",
                "session_token": "sts-token"
            }),
        )
        .validate_credentials()
        .unwrap();

        for credentials in [
            json!({"access_key_id": "AKIA_TEST"}),
            json!({"secret_access_key": "secret"}),
            json!({"access_key_id": "AKIA_TEST", "secret_access_key": "secret", "session_token": ""}),
            json!({"access_key_id": "AKIA_TEST", "secret_access_key": "secret", "profile": "forbidden"}),
        ] {
            assert!(
                material(AuthScheme::Bedrock, credentials)
                    .validate_credentials()
                    .is_err()
            );
        }
    }
}
