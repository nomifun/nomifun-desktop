//! Shared HTTP transport helpers (ported from
//! `nomifun-creation/src/adapters/mod.rs`, retyped onto [`InvokeError`]):
//! network-error mapping, non-2xx classification, capped body reads, base64 /
//! hex en/decoding — plus the shared send family ([`post_json`] /
//! [`post_multipart`] / [`post_raw`] / [`get_request`] over
//! [`send_with_rotation`]) that gives every adapter multi-key rotation on
//! 401/403/429 for free.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::auth::AuthMaterial;
use crate::error::{InvokeError, InvokeErrorKind};
use nomifun_net::secret_redaction::SecretRedactor;

/// Map a reqwest transport error onto [`InvokeError`]
/// (timeout → [`InvokeErrorKind::Timeout`], else [`InvokeErrorKind::Network`]).
pub fn net_err(e: reqwest::Error) -> InvokeError {
    InvokeError::network(&e)
}

/// HTTP statuses that mean "this key was refused / throttled" — the rotation
/// trigger set: 401/403 (auth) and 429 (rate limit).
fn is_rotation_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403 | 429)
}

/// Send a request, rotating through the connection's stored key list on
/// 401/403/429.
///
/// `build` constructs a FRESH un-authenticated [`reqwest::RequestBuilder`] per
/// attempt (builders are consumed by `send`, and multipart bodies cannot be
/// cloned). Rotation applies only to array-key schemes
/// ([`crate::auth::AuthScheme::rotates`]) with 2+ stored keys: each key gets
/// exactly one attempt, in stored order, and the FIRST response outside the
/// rotation trigger set (success or any other failure) is returned. When every
/// key is refused, the last response is returned for the caller to classify —
/// adapters keep their existing `error_from_response` handling. Non-rotating
/// schemes (MultiHeader) and single-key credentials are a plain single send.
/// Transport errors are never rotated (they are not key-specific).
pub(crate) async fn send_with_rotation<F>(auth: &AuthMaterial, build: F) -> Result<reqwest::Response, InvokeError>
where
    F: Fn() -> Result<reqwest::RequestBuilder, InvokeError>,
{
    let redactor = auth.secret_redactor();
    let secrets = if auth.scheme.rotates() { auth.secrets() } else { Vec::new() };
    if secrets.len() < 2 {
        // Single-shot path: `apply` also surfaces the canonical Config error
        // for empty credentials.
        let mut response = auth.apply(build()?)?.send().await.map_err(net_err)?;
        response.extensions_mut().insert(redactor);
        return Ok(response);
    }
    let last = secrets.len() - 1;
    for (idx, secret) in secrets.iter().enumerate() {
        let mut resp = auth.apply_with_secret(build()?, secret)?.send().await.map_err(net_err)?;
        resp.extensions_mut().insert(redactor.clone());
        if idx < last && is_rotation_status(resp.status()) {
            continue; // this key was refused/throttled — try the next one
        }
        return Ok(resp);
    }
    unreachable!("rotation loop always returns on the last key")
}

/// `POST url` with a JSON body through key rotation.
pub(crate) async fn post_json(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    auth: &AuthMaterial,
    body: &serde_json::Value,
) -> Result<reqwest::Response, InvokeError> {
    send_with_rotation(auth, || Ok(http.post(url).timeout(timeout).json(body))).await
}

/// `POST url` with a multipart body through key rotation. `make_form` builds
/// a fresh [`reqwest::multipart::Form`] per attempt (forms are not cloneable).
pub(crate) async fn post_multipart<F>(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    auth: &AuthMaterial,
    make_form: F,
) -> Result<reqwest::Response, InvokeError>
where
    F: Fn() -> Result<reqwest::multipart::Form, InvokeError>,
{
    send_with_rotation(auth, || Ok(http.post(url).timeout(timeout).multipart(make_form()?))).await
}

/// `POST url` with a raw binary body (+ `Content-Type` + query string) through
/// key rotation (Deepgram-style upload).
pub(crate) async fn post_raw(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    auth: &AuthMaterial,
    content_type: &str,
    query: &[(&str, String)],
    body: &[u8],
) -> Result<reqwest::Response, InvokeError> {
    send_with_rotation(auth, || {
        Ok(http
            .post(url)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .query(query)
            .body(body.to_vec()))
    })
    .await
}

/// `GET url` through key rotation (status polls / content downloads).
pub(crate) async fn get_request(
    http: &reqwest::Client,
    url: &str,
    timeout: Duration,
    auth: &AuthMaterial,
) -> Result<reqwest::Response, InvokeError> {
    send_with_rotation(auth, || Ok(http.get(url).timeout(timeout))).await
}

/// Longest Retry-After we are willing to honor (seconds).
const MAX_RETRY_AFTER_SECS: u64 = 120;

/// Parse a `Retry-After` header value in the delta-seconds form, clamped to
/// [`MAX_RETRY_AFTER_SECS`], as milliseconds. The HTTP-date form yields `None`.
fn parse_retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
    let secs: u64 = value?.to_str().ok()?.trim().parse().ok()?;
    Some(secs.min(MAX_RETRY_AFTER_SECS) * 1000)
}

/// Classify a non-2xx response into a typed [`InvokeError`], folding the
/// status + a 500-char body snippet into the message:
/// 429 → [`InvokeErrorKind::RateLimited`] (+ `Retry-After` seconds, clamped
/// to 120 s, as `retry_after_ms`); 401/403 → [`InvokeErrorKind::Auth`];
/// 400/422 → [`InvokeErrorKind::InvalidParams`]; 5xx and everything else →
/// [`InvokeErrorKind::ProviderError`]. `http_status` is always set.
pub async fn error_from_response(resp: reqwest::Response) -> InvokeError {
    let redactor = response_secret_redactor(&resp);
    let status = resp.status();
    let code = status.as_u16();
    let kind = match code {
        429 => InvokeErrorKind::RateLimited,
        401 | 403 => InvokeErrorKind::Auth,
        400 | 422 => InvokeErrorKind::InvalidParams,
        _ => InvokeErrorKind::ProviderError, // 5xx and everything unclassified
    };
    // Read the header before `text()` consumes the response.
    let retry_after_ms = (code == 429)
        .then(|| parse_retry_after(resp.headers().get(reqwest::header::RETRY_AFTER)))
        .flatten();
    let body = resp.text().await.unwrap_or_default();
    let snippet: String = redactor.redact(&body).chars().take(500).collect();
    InvokeError {
        kind,
        message: format!("provider returned {status}: {snippet}"),
        http_status: Some(code),
        retry_after_ms,
    }
}

/// Obtain the exact runtime credential redactor attached by the authenticated
/// send path. Protocols that surface provider-specific failure headers/bodies
/// use this before consuming the response.
pub(crate) fn response_secret_redactor(resp: &reqwest::Response) -> SecretRedactor {
    resp.extensions()
        .get::<SecretRedactor>()
        .cloned()
        .unwrap_or_default()
}

/// Hard ceiling on a single downloaded artifact / video-content body. Streams
/// are aborted once this is exceeded so a large or hostile provider response
/// cannot exhaust process memory.
pub const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

/// Read a response body fully into memory under a hard byte cap. Rejects early
/// on an oversized `Content-Length`, then streams chunk-by-chunk (Content-Length
/// may be absent or spoofed) aborting the moment the running total would exceed
/// `max_bytes`. Replaces the unbounded `resp.bytes()` used for artifact/video
/// downloads.
pub async fn read_body_capped(mut resp: reqwest::Response, max: u64) -> Result<Vec<u8>, InvokeError> {
    if let Some(len) = resp.content_length()
        && len > max
    {
        return Err(InvokeError::new(
            InvokeErrorKind::ProviderError,
            format!("artifact too large: declared {len} bytes exceeds cap of {max}"),
        ));
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(net_err)? {
        if buf.len() as u64 + chunk.len() as u64 > max {
            return Err(InvokeError::new(
                InvokeErrorKind::ProviderError,
                format!("artifact exceeded size cap of {max} bytes"),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Decode a base64 payload (adapters share this for inline results).
pub fn decode_b64(s: &str) -> Option<Vec<u8>> {
    BASE64.decode(s.trim()).ok()
}

/// Encode input bytes to base64 (e.g. Gemini `inline_data`).
pub fn encode_b64(b: &[u8]) -> String {
    BASE64.encode(b)
}

/// Decode a HEX-encoded payload (MiniMax t2a returns audio as a hex string,
/// not base64). Tolerates surrounding whitespace; rejects odd length and
/// non-hex digits.
pub fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};

    async fn respond(template: ResponseTemplate) -> reqwest::Response {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/x")).respond_with(template).mount(&server).await;
        reqwest::Client::new().get(format!("{}/x", server.uri())).send().await.unwrap()
    }

    fn assert_query_secret_redacted(error: &InvokeError, secret: &str, request_root: &str) {
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("api_key", secret)
            .finish();
        let invoke_rendered = error.to_string();
        let app_error: nomifun_common::AppError = error.clone().into();
        let app_rendered = app_error.to_string();
        for rendered in [&invoke_rendered, &app_rendered] {
            assert!(!rendered.contains(secret), "raw secret leaked: {rendered}");
            assert!(!rendered.contains(&encoded), "encoded secret leaked: {rendered}");
            assert!(!rendered.contains("api_key"), "query parameter leaked: {rendered}");
            assert!(!rendered.contains(request_root), "request URL leaked: {rendered}");
        }
    }

    #[test]
    fn b64_roundtrip() {
        assert_eq!(decode_b64(&encode_b64(b"hello")).unwrap(), b"hello");
        assert_eq!(decode_b64(" aGVsbG8= ").unwrap(), b"hello");
        assert!(decode_b64("!!not base64!!").is_none());
    }

    #[test]
    fn hex_decodes_valid_and_rejects_malformed() {
        assert_eq!(decode_hex("68656c6c6f").unwrap(), b"hello");
        // Uppercase digits and surrounding whitespace tolerated.
        assert_eq!(decode_hex(" 48490A ").unwrap(), b"HI\n");
        assert_eq!(decode_hex("").unwrap(), Vec::<u8>::new());
        for bad in ["abc", "zz", "0g", "6 8"] {
            assert!(decode_hex(bad).is_none(), "input {bad:?}");
        }
    }

    // -- multi-key rotation -----------------------------------------------------

    fn bearer(keys: &[&str]) -> AuthMaterial {
        AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({ "api_keys": keys }) }
    }

    /// Mount a 401 for `Bearer <bad>` and a 200 for `Bearer <good>` on POST /r.
    async fn rotation_server(bad: &str, good: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .and(header("authorization", format!("Bearer {bad}")))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .and(header("authorization", format!("Bearer {good}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn rotation_first_key_401_second_key_succeeds() {
        let server = rotation_server("sk-bad", "sk-good").await;
        let auth = bearer(&["sk-bad", "sk-good"]);
        let url = format!("{}/r", server.uri());

        let resp = post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, &json!({"p": 1}))
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        // Two requests, each carrying a DIFFERENT Authorization header.
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let auth_of = |i: usize| requests[i].headers.get("authorization").unwrap().to_str().unwrap().to_string();
        assert_eq!(auth_of(0), "Bearer sk-bad");
        assert_eq!(auth_of(1), "Bearer sk-good");
    }

    #[tokio::test]
    async fn rotation_403_and_429_also_trigger_next_key() {
        for status in [403u16, 429] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/r"))
                .and(header("authorization", "Bearer sk-1"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/r"))
                .and(header("authorization", "Bearer sk-2"))
                .respond_with(ResponseTemplate::new(200))
                .expect(1)
                .mount(&server)
                .await;
            let auth = bearer(&["sk-1", "sk-2"]);
            let url = format!("{}/r", server.uri());
            let resp =
                post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, &json!({})).await.unwrap();
            assert_eq!(resp.status().as_u16(), 200, "trigger status {status}");
        }
    }

    #[tokio::test]
    async fn rotation_all_keys_fail_returns_last_response_classified_as_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(401).set_body_string("every key is bad"))
            .expect(3)
            .mount(&server)
            .await;

        let auth = bearer(&["sk-1", "sk-2", "sk-3"]);
        let url = format!("{}/r", server.uri());
        let resp =
            post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, &json!({})).await.unwrap();
        assert_eq!(resp.status().as_u16(), 401, "last refusal is surfaced");
        // The caller-side classification (every adapter's error path) → Auth.
        let err = error_from_response(resp).await;
        assert_eq!(err.kind, InvokeErrorKind::Auth);
        assert!(err.message.contains("every key is bad"), "message: {}", err.message);
        assert_eq!(server.received_requests().await.unwrap().len(), 3, "one attempt per key");
    }

    #[tokio::test]
    async fn rotation_non_trigger_failure_does_not_rotate() {
        // A 500 is not key-specific: no second attempt.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .expect(1)
            .mount(&server)
            .await;
        let auth = bearer(&["sk-1", "sk-2"]);
        let url = format!("{}/r", server.uri());
        let resp =
            post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, &json!({})).await.unwrap();
        assert_eq!(resp.status().as_u16(), 500);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rotation_multi_header_scheme_is_single_shot() {
        // MultiHeader credentials are one named-field object — a 401 must NOT
        // trigger any second attempt.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(401).set_body_string("denied"))
            .expect(1)
            .mount(&server)
            .await;
        let auth = AuthMaterial {
            scheme: AuthScheme::parse("volc_voice").unwrap(),
            credentials: json!({
                "app_key": "a", "access_key": "b", "resource_id": "r",
                // Even a stray api_keys array must not induce rotation here.
                "api_keys": ["k1", "k2"],
            }),
        };
        let url = format!("{}/r", server.uri());
        let resp =
            post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, &json!({})).await.unwrap();
        assert_eq!(resp.status().as_u16(), 401);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "MultiHeader must not rotate");
        assert_eq!(requests[0].headers.get("X-Api-App-Key").unwrap(), "a");
    }

    #[tokio::test]
    async fn rotation_single_key_is_single_shot_and_empty_keys_is_config_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/r"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}/r", server.uri());
        let resp = post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &bearer(&["sk-only"]), &json!({}))
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);

        // No keys at all: the canonical Config error, no request sent.
        let none = AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({}) };
        let err = post_json(&reqwest::Client::new(), &url, Duration::from_secs(5), &none, &json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
    }

    #[tokio::test]
    async fn rotation_query_key_rotates_query_param() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/q"))
            .and(wiremock::matchers::query_param("key", "q-1"))
            .respond_with(ResponseTemplate::new(429))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/q"))
            .and(wiremock::matchers::query_param("key", "q-2"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let auth = AuthMaterial {
            scheme: AuthScheme::QueryKey("key".into()),
            credentials: json!({"api_keys": ["q-1", "q-2"]}),
        };
        let url = format!("{}/q", server.uri());
        let resp = get_request(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth).await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn rotation_rebuilds_multipart_form_per_attempt() {
        let server = rotation_server("mk-bad", "mk-good").await;
        let auth = bearer(&["mk-bad", "mk-good"]);
        let url = format!("{}/r", server.uri());
        let resp = post_multipart(&reqwest::Client::new(), &url, Duration::from_secs(5), &auth, || {
            Ok(reqwest::multipart::Form::new().text("field", "value"))
        })
        .await
        .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for req in &requests {
            let body = String::from_utf8_lossy(&req.body);
            assert!(body.contains("name=\"field\""), "each attempt carries a full form");
        }
    }

    #[tokio::test]
    async fn net_err_classifies_timeout_vs_network() {
        // Timeout: server answers slower than the client timeout.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let err = client.get(format!("{}/slow", server.uri())).send().await.unwrap_err();
        assert_eq!(net_err(err).kind, InvokeErrorKind::Timeout);

        // Connection refused: bind a port, drop the listener, then dial it.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let err = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/x"))
            .send()
            .await
            .unwrap_err();
        assert_eq!(net_err(err).kind, InvokeErrorKind::Network);
    }

    #[tokio::test]
    async fn query_key_transport_error_never_discloses_raw_or_encoded_secret() {
        let server = MockServer::start().await;
        let secret = "query secret/+?&=TOP_SECRET";
        Mock::given(method("GET"))
            .and(path("/slow-secret"))
            .and(query_param("api_key", secret))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(250)))
            .expect(1)
            .mount(&server)
            .await;
        let auth = AuthMaterial {
            scheme: AuthScheme::QueryKey("api_key".into()),
            credentials: json!({"api_keys": [secret]}),
        };
        let url = format!("{}/slow-secret", server.uri());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let error = get_request(&client, &url, Duration::from_millis(20), &auth)
            .await
            .unwrap_err();

        assert_eq!(error.kind, InvokeErrorKind::Timeout);
        assert_query_secret_redacted(&error, secret, &server.uri());
        server.verify().await;
    }

    #[tokio::test]
    async fn query_key_json_decode_error_never_discloses_raw_or_encoded_secret() {
        let server = MockServer::start().await;
        let secret = "json secret/+?&=TOP_SECRET";
        Mock::given(method("GET"))
            .and(path("/invalid-json-secret"))
            .and(query_param("api_key", secret))
            .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
            .expect(1)
            .mount(&server)
            .await;
        let auth = AuthMaterial {
            scheme: AuthScheme::QueryKey("api_key".into()),
            credentials: json!({"api_keys": [secret]}),
        };
        let url = format!("{}/invalid-json-secret", server.uri());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = get_request(&client, &url, Duration::from_secs(1), &auth)
            .await
            .unwrap();

        let source = response.json::<serde_json::Value>().await.unwrap_err();
        let error = InvokeError::response_json("invalid test JSON", &source);

        assert_eq!(error.kind, InvokeErrorKind::ParseError);
        assert_query_secret_redacted(&error, secret, &server.uri());
        server.verify().await;
    }

    #[tokio::test]
    async fn error_from_response_maps_status_codes() {
        for (status, kind) in [
            (401, InvokeErrorKind::Auth),
            (403, InvokeErrorKind::Auth),
            (400, InvokeErrorKind::InvalidParams),
            (422, InvokeErrorKind::InvalidParams),
            (500, InvokeErrorKind::ProviderError),
            (503, InvokeErrorKind::ProviderError),
            (418, InvokeErrorKind::ProviderError),
        ] {
            let resp = respond(ResponseTemplate::new(status).set_body_string("nope")).await;
            let err = error_from_response(resp).await;
            assert_eq!(err.kind, kind, "status {status}");
            assert_eq!(err.http_status, Some(status), "status {status}");
            assert_eq!(err.retry_after_ms, None, "status {status}");
            assert!(err.message.contains("nope"), "status {status}: {}", err.message);
        }
    }

    #[tokio::test]
    async fn error_from_response_redacts_every_runtime_key_and_encoded_form() {
        let first = "sk first/+?=";
        let second = "sk-second-secret";
        let encoded_first = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("key", first)
            .finish()
            .strip_prefix("key=")
            .unwrap()
            .to_owned();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/redact"))
            .respond_with(ResponseTemplate::new(401).set_body_string(format!(
                "Authorization: Bearer {first}; x-api-key={second}; query={encoded_first}"
            )))
            .expect(2)
            .mount(&server)
            .await;

        let response = post_json(
            &reqwest::Client::new(),
            &format!("{}/redact", server.uri()),
            Duration::from_secs(5),
            &bearer(&[first, second]),
            &json!({}),
        )
        .await
        .unwrap();
        let error = error_from_response(response).await;
        assert_eq!(error.kind, InvokeErrorKind::Auth);
        for secret in [first, second, encoded_first.as_str()] {
            assert!(!error.message.contains(secret), "secret leaked: {}", error.message);
        }
        assert!(error.message.contains("[REDACTED]"));
        server.verify().await;
    }

    #[tokio::test]
    async fn error_from_response_429_parses_and_clamps_retry_after() {
        // Plain seconds.
        let resp =
            respond(ResponseTemplate::new(429).insert_header("Retry-After", "7").set_body_string("slow")).await;
        let err = error_from_response(resp).await;
        assert_eq!(err.kind, InvokeErrorKind::RateLimited);
        assert_eq!(err.http_status, Some(429));
        assert_eq!(err.retry_after_ms, Some(7_000));

        // Clamped to 120 s.
        let resp = respond(ResponseTemplate::new(429).insert_header("Retry-After", "9999")).await;
        assert_eq!(error_from_response(resp).await.retry_after_ms, Some(120_000));

        // Missing / unparseable (HTTP-date) header → None, still RateLimited.
        let resp = respond(ResponseTemplate::new(429)).await;
        let err = error_from_response(resp).await;
        assert_eq!(err.kind, InvokeErrorKind::RateLimited);
        assert_eq!(err.retry_after_ms, None);
        let resp =
            respond(ResponseTemplate::new(429).insert_header("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT")).await;
        assert_eq!(error_from_response(resp).await.retry_after_ms, None);
    }

    #[tokio::test]
    async fn error_from_response_truncates_body_to_500_chars() {
        let resp = respond(ResponseTemplate::new(500).set_body_string("x".repeat(600))).await;
        let err = error_from_response(resp).await;
        assert_eq!(err.message.chars().filter(|c| *c == 'x').count(), 500);
    }

    #[tokio::test]
    async fn read_body_capped_enforces_cap() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/artifact"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 100]))
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let url = format!("{}/artifact", server.uri());

        // Over the cap → error (rejected on the declared Content-Length).
        let resp = client.get(&url).send().await.unwrap();
        assert!(read_body_capped(resp, 10).await.is_err(), "oversized body must be rejected");

        // Within the cap → full body returned (streaming accumulation path).
        let resp2 = client.get(&url).send().await.unwrap();
        let body = read_body_capped(resp2, 1024).await.unwrap();
        assert_eq!(body.len(), 100);
    }
}
