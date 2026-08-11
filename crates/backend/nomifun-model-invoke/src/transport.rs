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
use serde::de::DeserializeOwned;

use crate::auth::AuthMaterial;
use crate::error::{InvokeError, InvokeErrorKind};

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
    let secrets = if auth.scheme.rotates() { auth.secrets() } else { Vec::new() };
    if secrets.len() < 2 {
        // Single-shot path: `apply` also surfaces the canonical Config error
        // for empty credentials.
        return auth.apply(build()?)?.send().await.map_err(net_err);
    }
    let last = secrets.len() - 1;
    for (idx, secret) in secrets.iter().enumerate() {
        let resp = auth.apply_with_secret(build()?, secret)?.send().await.map_err(net_err)?;
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
/// Error bodies are diagnostics, never artifacts. Bound their transport read
/// separately so non-2xx submit/download responses cannot allocate an
/// arbitrarily large String before the existing 500-character presentation
/// truncation runs.
const MAX_ERROR_RESPONSE_BODY_BYTES: usize = 64 * 1024;
const MAX_ERROR_RESPONSE_SNIPPET_CHARS: usize = 500;

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
    let status = resp.status();
    let code = status.as_u16();
    let kind = match code {
        429 => InvokeErrorKind::RateLimited,
        401 | 403 => InvokeErrorKind::Auth,
        400 | 422 => InvokeErrorKind::InvalidParams,
        _ => InvokeErrorKind::ProviderError, // 5xx and everything unclassified
    };
    // Read the header before the bounded body reader consumes the response.
    let retry_after_ms = (code == 429)
        .then(|| parse_retry_after(resp.headers().get(reqwest::header::RETRY_AFTER)))
        .flatten();
    let snippet = read_error_body_snippet(resp).await;
    InvokeError {
        kind,
        message: format!("provider returned {status}: {snippet}"),
        http_status: Some(code),
        retry_after_ms,
        catalog_failure: false,
    }
}

async fn read_error_body_snippet(mut resp: reqwest::Response) -> String {
    if let Some(declared) = resp.content_length()
        && declared > MAX_ERROR_RESPONSE_BODY_BYTES as u64
    {
        return format!(
            "<provider error body omitted: declared {declared} bytes exceeds {}-byte cap>",
            MAX_ERROR_RESPONSE_BODY_BYTES
        );
    }

    let mut body = Vec::new();
    let mut exceeded_cap = false;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = MAX_ERROR_RESPONSE_BODY_BYTES.saturating_sub(body.len());
                if chunk.len() > remaining {
                    body.extend_from_slice(&chunk[..remaining]);
                    exceeded_cap = true;
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                if body.is_empty() {
                    return format!("<provider error body read failed: {error}>");
                }
                break;
            }
        }
    }

    let mut snippet: String = String::from_utf8_lossy(&body)
        .chars()
        .take(MAX_ERROR_RESPONSE_SNIPPET_CHARS)
        .collect();
    if exceeded_cap {
        snippet.push_str(&format!(
            "… <truncated at {}-byte cap>",
            MAX_ERROR_RESPONSE_BODY_BYTES
        ));
    }
    snippet
}

/// Hard ceiling on a single downloaded artifact / video-content body. Streams
/// are aborted once this is exceeded so a large or hostile provider response
/// cannot exhaust process memory.
pub const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

/// Native image-result contract shared by every image adapter. These limits
/// are enforced while the provider response is still at the invocation
/// boundary, before an untrusted base64 string is decoded into another large
/// allocation. Product persistence repeats the per-image check as defense in
/// depth.
pub(crate) const MAX_IMAGE_RESPONSE_IMAGES: usize = 8;
pub(crate) const MAX_IMAGE_RESPONSE_BYTES_PER_IMAGE: usize = 20 * 1024 * 1024;
/// Aggregate decoded budget shared with the native chat image product. This
/// must be enforced before JSON parsing/base64 decode, not only later during
/// materialization, otherwise an ultimately rejected 8-image response can
/// transiently occupy hundreds of MiB.
pub(crate) const MAX_IMAGE_RESPONSE_TOTAL_BYTES: usize = 40 * 1024 * 1024;

const IMAGE_RESPONSE_JSON_OVERHEAD_BYTES: u64 = 1024 * 1024;

/// URL-only image submit/poll envelopes never contain image bytes. Keeping a
/// separate small cap prevents a hostile async provider from consuming the much
/// larger allowance needed for legal inline base64 results.
pub(crate) const MAX_IMAGE_METADATA_RESPONSE_BYTES: u64 = 1024 * 1024;

/// Maximum JSON body size for an inline response expected to contain at most
/// `max_images` legal images. The fixed allowance covers JSON structure, MIME
/// strings, text parts and provider metadata without making that metadata
/// unbounded.
pub(crate) fn inline_image_response_body_limit(max_images: usize) -> u64 {
    debug_assert!((1..=MAX_IMAGE_RESPONSE_IMAGES).contains(&max_images));
    let decoded_budget = MAX_IMAGE_RESPONSE_TOTAL_BYTES
        .min(MAX_IMAGE_RESPONSE_BYTES_PER_IMAGE.saturating_mul(max_images));
    decoded_budget.div_ceil(3) as u64 * 4 + IMAGE_RESPONSE_JSON_OVERHEAD_BYTES
}

/// Validate the caller-requested image count before issuing network requests.
/// Besides avoiding excessive Gemini loops, this makes the response-body cap
/// calculable from a trusted value.
pub(crate) fn validate_image_request_count(count: u32) -> Result<usize, InvokeError> {
    let count = usize::try_from(count).map_err(|_| {
        InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("image count must be between 1 and {MAX_IMAGE_RESPONSE_IMAGES}"),
        )
    })?;
    if !(1..=MAX_IMAGE_RESPONSE_IMAGES).contains(&count) {
        return Err(InvokeError::new(
            InvokeErrorKind::InvalidParams,
            format!("image count must be between 1 and {MAX_IMAGE_RESPONSE_IMAGES}"),
        ));
    }
    Ok(count)
}

/// Per-response/batch image budget. Adapters first preflight the number of
/// image-bearing items with [`Self::ensure_additional_count`], then record URL
/// results or decode inline base64 through this value. The decoder checks the
/// encoded length *before* allocating the decoded buffer, and repeats the
/// decoded per-image and aggregate checks afterwards.
pub(crate) struct ImageResponseBudget {
    max_images: usize,
    max_bytes_per_image: usize,
    max_total_bytes: usize,
    images: usize,
    decoded_bytes: usize,
}

impl ImageResponseBudget {
    pub(crate) fn new(max_images: usize) -> Result<Self, InvokeError> {
        if !(1..=MAX_IMAGE_RESPONSE_IMAGES).contains(&max_images) {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!("image count must be between 1 and {MAX_IMAGE_RESPONSE_IMAGES}"),
            ));
        }
        Ok(Self {
            max_images,
            max_bytes_per_image: MAX_IMAGE_RESPONSE_BYTES_PER_IMAGE,
            max_total_bytes: MAX_IMAGE_RESPONSE_TOTAL_BYTES
                .min(MAX_IMAGE_RESPONSE_BYTES_PER_IMAGE * max_images),
            images: 0,
            decoded_bytes: 0,
        })
    }

    #[cfg(test)]
    fn with_limits(max_images: usize, max_bytes_per_image: usize, max_total_bytes: usize) -> Self {
        Self {
            max_images,
            max_bytes_per_image,
            max_total_bytes,
            images: 0,
            decoded_bytes: 0,
        }
    }

    pub(crate) fn ensure_additional_count(
        &self,
        additional: usize,
        context: &str,
    ) -> Result<(), InvokeError> {
        let total = self.images.checked_add(additional).ok_or_else(|| {
            image_response_limit_error(format!("{context} image count overflowed"))
        })?;
        if total > self.max_images {
            return Err(image_response_limit_error(format!(
                "{context} returned {total} images, exceeding the limit of {}",
                self.max_images
            )));
        }
        Ok(())
    }

    pub(crate) fn accept_url(&mut self, context: &str) -> Result<(), InvokeError> {
        self.ensure_additional_count(1, context)?;
        self.images += 1;
        Ok(())
    }

    pub(crate) fn decode_base64(
        &mut self,
        encoded: &str,
        context: &str,
    ) -> Result<Vec<u8>, InvokeError> {
        self.ensure_additional_count(1, context)?;
        let encoded = encoded.trim();
        let encoded_cap = self.max_bytes_per_image.div_ceil(3) * 4;
        if encoded.len() > encoded_cap {
            return Err(image_response_limit_error(format!(
                "{context} base64 length {} exceeds the encoded limit of {encoded_cap}",
                encoded.len()
            )));
        }
        let bytes = BASE64
            .decode(encoded)
            .map_err(|_| InvokeError::parse(format!("{context} is not valid base64")))?;
        if bytes.len() > self.max_bytes_per_image {
            return Err(image_response_limit_error(format!(
                "{context} decoded to {} bytes, exceeding the per-image limit of {}",
                bytes.len(),
                self.max_bytes_per_image
            )));
        }
        let total = self.decoded_bytes.checked_add(bytes.len()).ok_or_else(|| {
            image_response_limit_error(format!("{context} aggregate size overflowed"))
        })?;
        if total > self.max_total_bytes {
            return Err(image_response_limit_error(format!(
                "{context} would raise decoded image bytes to {total}, exceeding the aggregate limit of {}",
                self.max_total_bytes
            )));
        }
        self.images += 1;
        self.decoded_bytes = total;
        Ok(bytes)
    }
}

fn image_response_limit_error(message: impl Into<String>) -> InvokeError {
    InvokeError::new(InvokeErrorKind::ProviderError, message)
}

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

/// Read and deserialize a JSON response under a hard body cap. This must be
/// used instead of `Response::json()` for provider responses that may carry
/// inline media: the latter buffers without an application-level ceiling.
pub(crate) async fn read_json_capped<T: DeserializeOwned>(
    resp: reqwest::Response,
    max_bytes: u64,
    context: &str,
) -> Result<T, InvokeError> {
    let body = read_body_capped(resp, max_bytes).await?;
    serde_json::from_slice(&body)
        .map_err(|error| InvokeError::parse(format!("invalid {context} JSON: {error}")))
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};

    async fn respond(template: ResponseTemplate) -> reqwest::Response {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/x")).respond_with(template).mount(&server).await;
        reqwest::Client::new().get(format!("{}/x", server.uri())).send().await.unwrap()
    }

    async fn respond_chunked(status: u16, chunks: Vec<Vec<u8>>) -> reqwest::Response {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            for chunk in chunks {
                stream
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .unwrap();
                stream.write_all(&chunk).await.unwrap();
                stream.write_all(b"\r\n").await.unwrap();
            }
            stream.write_all(b"0\r\n\r\n").await.unwrap();
        });
        reqwest::Client::new()
            .get(format!("http://{address}/chunked"))
            .send()
            .await
            .unwrap()
    }

    #[test]
    fn b64_roundtrip() {
        assert_eq!(decode_b64(&encode_b64(b"hello")).unwrap(), b"hello");
        assert_eq!(decode_b64(" aGVsbG8= ").unwrap(), b"hello");
        assert!(decode_b64("!!not base64!!").is_none());
    }

    #[test]
    fn image_contract_response_limits_reject_encoded_per_image_aggregate_and_count_overflow() {
        assert_eq!(MAX_IMAGE_RESPONSE_IMAGES, 8);
        assert_eq!(MAX_IMAGE_RESPONSE_BYTES_PER_IMAGE, 20 * 1024 * 1024);
        assert_eq!(MAX_IMAGE_RESPONSE_TOTAL_BYTES, 40 * 1024 * 1024);
        assert_eq!(
            inline_image_response_body_limit(8),
            (40 * 1024 * 1024usize).div_ceil(3) as u64 * 4
                + IMAGE_RESPONSE_JSON_OVERHEAD_BYTES
        );
        assert!(validate_image_request_count(1).is_ok());
        assert!(validate_image_request_count(8).is_ok());
        assert_eq!(validate_image_request_count(0).unwrap_err().kind, InvokeErrorKind::InvalidParams);
        assert_eq!(validate_image_request_count(9).unwrap_err().kind, InvokeErrorKind::InvalidParams);

        // Four decoded bytes would require eight base64 characters, but a
        // three-byte member budget permits only four. This fails before decode.
        let mut encoded = ImageResponseBudget::with_limits(2, 3, 6);
        let error = encoded.decode_base64("AQIDBA==", "test image").unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("base64 length"));

        let mut aggregate = ImageResponseBudget::with_limits(3, 3, 5);
        assert_eq!(aggregate.decode_base64("YWJj", "first").unwrap(), b"abc");
        let error = aggregate.decode_base64("ZGVm", "second").unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("aggregate limit"));

        let mut count = ImageResponseBudget::with_limits(1, 3, 3);
        count.accept_url("first").unwrap();
        let error = count.accept_url("second").unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("exceeding the limit of 1"));
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
        let client = reqwest::Client::builder().timeout(Duration::from_millis(50)).build().unwrap();
        let err = client.get(format!("{}/slow", server.uri())).send().await.unwrap_err();
        assert_eq!(net_err(err).kind, InvokeErrorKind::Timeout);

        // Connection refused: bind a port, drop the listener, then dial it.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let err = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/x"))
            .send()
            .await
            .unwrap_err();
        assert_eq!(net_err(err).kind, InvokeErrorKind::Network);
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
    async fn image_contract_response_limits_bound_declared_and_chunked_error_bodies() {
        let declared = respond(
            ResponseTemplate::new(500)
                .set_body_string("x".repeat(MAX_ERROR_RESPONSE_BODY_BYTES + 1)),
        )
        .await;
        let declared_error = error_from_response(declared).await;
        assert_eq!(declared_error.http_status, Some(500));
        assert!(
            declared_error.message.contains("provider error body omitted"),
            "{}",
            declared_error.message
        );

        let chunked = respond_chunked(
            500,
            vec![
                vec![b'x'; MAX_ERROR_RESPONSE_BODY_BYTES / 2],
                vec![b'y'; MAX_ERROR_RESPONSE_BODY_BYTES / 2 + 1],
            ],
        )
        .await;
        assert_eq!(chunked.content_length(), None);
        let chunked_error = error_from_response(chunked).await;
        assert_eq!(chunked_error.http_status, Some(500));
        assert!(
            chunked_error.message.contains("truncated at 65536-byte cap"),
            "{}",
            chunked_error.message
        );
        assert!(chunked_error.message.len() < 1024);
    }

    #[tokio::test]
    async fn image_contract_response_limits_reject_declared_body_before_json_parse() {
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
        let error = read_json_capped::<serde_json::Value>(resp, 10, "test")
            .await
            .unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("declared"));

        // Within the cap → full body returned (streaming accumulation path).
        let resp2 = client.get(&url).send().await.unwrap();
        let body = read_body_capped(resp2, 1024).await.unwrap();
        assert_eq!(body.len(), 100);
    }

    #[tokio::test]
    async fn image_contract_response_limits_reject_chunked_body_without_content_length() {
        let resp = respond_chunked(200, vec![b"1234".to_vec(), b"5678".to_vec()]).await;
        assert_eq!(resp.content_length(), None);
        let error = read_body_capped(resp, 5).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert!(error.message.contains("exceeded size cap"));
    }
}
