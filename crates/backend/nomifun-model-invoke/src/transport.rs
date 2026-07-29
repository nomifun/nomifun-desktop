//! Shared HTTP transport helpers (ported from
//! `nomifun-creation/src/adapters/mod.rs`, retyped onto [`InvokeError`]):
//! network-error mapping, non-2xx classification, capped body reads and
//! base64 en/decoding.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::error::{InvokeError, InvokeErrorKind};

/// Map a reqwest transport error onto [`InvokeError`]
/// (timeout → [`InvokeErrorKind::Timeout`], else [`InvokeErrorKind::Network`]).
pub fn net_err(e: reqwest::Error) -> InvokeError {
    InvokeError::network(&e)
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
    let snippet: String = body.chars().take(500).collect();
    InvokeError {
        kind,
        message: format!("provider returned {status}: {snippet}"),
        http_status: Some(code),
        retry_after_ms,
    }
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn respond(template: ResponseTemplate) -> reqwest::Response {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/x")).respond_with(template).mount(&server).await;
        reqwest::Client::new().get(format!("{}/x", server.uri())).send().await.unwrap()
    }

    #[test]
    fn b64_roundtrip() {
        assert_eq!(decode_b64(&encode_b64(b"hello")).unwrap(), b"hello");
        assert_eq!(decode_b64(" aGVsbG8= ").unwrap(), b"hello");
        assert!(decode_b64("!!not base64!!").is_none());
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
