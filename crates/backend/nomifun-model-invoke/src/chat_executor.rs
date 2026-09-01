//! Single-attempt HTTP execution for the v4 chat broker.
//!
//! This module deliberately does not resolve models, select routes, rotate
//! credentials, or retry requests.  It accepts an already-resolved URL/body
//! and an opaque credential lease.  The host supplies a private resolver that
//! turns that lease into [`AuthMaterial`] inside the process boundary.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    self as sigv4_http, PayloadChecksumKind, SignableBody, SignableRequest,
    SignatureLocation, SigningSettings,
};
use crc32fast::Hasher;
use futures_util::{Stream, StreamExt};
use crate::auth::{AuthMaterial, AuthScheme};
use crate::error::InvokeError;
use crate::transport::{error_from_response, net_err};
use serde_json::Value;
use std::time::SystemTime;

/// A process-local credential authority.  The handle is intentionally opaque:
/// it is never serialized into a request body, URL, error, or debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct OpaqueCredentialLease {
    handle: String,
}

impl OpaqueCredentialLease {
    pub fn new(handle: impl Into<String>) -> Result<Self, InvokeError> {
        let handle = handle.into();
        if handle.trim().is_empty() {
            return Err(InvokeError::config(
                "opaque credential lease handle must not be empty",
            ));
        }
        Ok(Self { handle })
    }

    pub fn handle(&self) -> &str {
        &self.handle
    }
}

impl std::fmt::Debug for OpaqueCredentialLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpaqueCredentialLease")
            .field("handle", &"[opaque]")
            .finish()
    }
}

/// Private credential authority used by an application composition root.
///
/// Implementations may decrypt or look up credentials, but the resulting
/// [`AuthMaterial`] remains inside this executor and is never returned to the
/// caller or put into a wire request.
#[async_trait]
pub trait OpaqueCredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        lease: &OpaqueCredentialLease,
    ) -> Result<AuthMaterial, InvokeError>;
}

/// Optional protocol-specific request authentication.
///
/// The default executor applies the single primary HTTP credential without
/// rotation. SDK-backed protocols such as Bedrock can install an adapter here
/// to sign the already-resolved request from the opaque lease. The adapter
/// owns no retry or route selection authority.
pub trait SingleAttemptAuthenticator: Send + Sync {
    fn apply(
        &self,
        builder: reqwest::RequestBuilder,
        request: &SingleAttemptRequest,
        material: &AuthMaterial,
        body: &[u8],
    ) -> Result<reqwest::RequestBuilder, InvokeError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleAttemptFraming {
    Sse,
    AwsEventStream,
}

struct DefaultSingleAttemptAuthenticator;

impl SingleAttemptAuthenticator for DefaultSingleAttemptAuthenticator {
    fn apply(
        &self,
        builder: reqwest::RequestBuilder,
        request: &SingleAttemptRequest,
        material: &AuthMaterial,
        _body: &[u8],
    ) -> Result<reqwest::RequestBuilder, InvokeError> {
        if request.framing == SingleAttemptFraming::AwsEventStream
            || matches!(material.scheme, AuthScheme::Bedrock)
        {
            return sign_bedrock_request(builder, request, material, _body);
        }
        material
            .apply(builder)
            .map_err(|error| error.redacted(&material.secret_redactor()))
    }
}

/// A single already-resolved provider request.
#[derive(Clone)]
pub struct SingleAttemptRequest {
    /// Stable protocol id, for example `openai.chat`, `anthropic`,
    /// `bedrock`, or `vertex`.
    pub protocol: String,
    pub url: String,
    pub model: String,
    pub body: Value,
    pub credential: OpaqueCredentialLease,
    pub timeout: Duration,
    pub framing: SingleAttemptFraming,
    /// Required for AWS SigV4-backed Bedrock requests.
    pub region: Option<String>,
}

impl std::fmt::Debug for SingleAttemptRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SingleAttemptRequest")
            .field("protocol", &self.protocol)
            .field("url", &self.url)
            .field("model", &self.model)
            .field("body", &"[redacted]")
            .field("credential", &self.credential)
            .field("timeout", &self.timeout)
            .field("framing", &self.framing)
            .field("region", &self.region.as_deref().map(|_| "[redacted]"))
            .finish()
    }
}

impl SingleAttemptRequest {
    pub fn validate(&self) -> Result<(), InvokeError> {
        if self.protocol.trim().is_empty() {
            return Err(InvokeError::config("single-attempt protocol is empty"));
        }
        let url = reqwest::Url::parse(self.url.trim())
            .map_err(|_| InvokeError::config("single-attempt URL is invalid"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(InvokeError::config(
                "single-attempt URL must be an HTTP(S) URL without userinfo or fragment",
            ));
        }
        if self.model.trim().is_empty() {
            return Err(InvokeError::config("single-attempt model is empty"));
        }
        if self.timeout.is_zero() {
            return Err(InvokeError::config(
                "single-attempt timeout must be greater than zero",
            ));
        }
        if self.framing == SingleAttemptFraming::AwsEventStream
            && self.region.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err(InvokeError::config(
                "AWS event-stream requests require a non-empty signing region",
            ));
        }
        Ok(())
    }
}

fn sign_bedrock_request(
    builder: reqwest::RequestBuilder,
    request: &SingleAttemptRequest,
    material: &AuthMaterial,
    body: &[u8],
) -> Result<reqwest::RequestBuilder, InvokeError> {
    let region = request
        .region
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| InvokeError::config("Bedrock signing region is missing"))?;
    let object = material.credentials.as_object().ok_or_else(|| {
        InvokeError::config("Bedrock credentials must be an object for SigV4 signing")
    })?;
    let access_key_id = object
        .get("access_key_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| InvokeError::config("Bedrock access_key_id is missing"))?;
    let secret_access_key = object
        .get("secret_access_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| InvokeError::config("Bedrock secret_access_key is missing"))?;
    let session_token = object
        .get("session_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let credentials = Credentials::new(
        access_key_id,
        secret_access_key,
        session_token,
        None,
        "nomifun-single-attempt",
    );
    let mut settings = SigningSettings::default();
    settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
    settings.signature_location = SignatureLocation::Headers;
    let identity = credentials.into();
    let params = aws_sigv4::sign::v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(SystemTime::now())
        .settings(settings)
        .build()
        .map_err(|_| InvokeError::config("Bedrock SigV4 parameters are invalid"))?;
    let parsed = reqwest::Url::parse(request.url.trim())
        .map_err(|_| InvokeError::config("Bedrock request URL is invalid"))?;
    let signable = SignableRequest::new(
        "POST",
        parsed.as_str(),
        [("content-type", "application/json")].into_iter(),
        SignableBody::Bytes(body),
    )
    .map_err(|_| InvokeError::config("Bedrock request could not be signed"))?;
    let (instructions, _) = sigv4_http::sign(signable, &params.into())
        .map_err(|_| InvokeError::config("Bedrock request could not be signed"))?
        .into_parts();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    for (name, value) in instructions.headers() {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| InvokeError::config("Bedrock signing produced an invalid header"))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| InvokeError::config("Bedrock signing produced an invalid value"))?;
        headers.insert(name, value);
    }
    Ok(builder.headers(headers))
}

/// One provider stream frame.  The chat broker owns protocol-specific semantic
/// decoding; this layer only preserves the provider event name and JSON data.
#[derive(Clone, Debug, PartialEq)]
pub struct SingleAttemptFrame {
    pub event: String,
    pub data: Value,
}

pub type SingleAttemptStream =
    Pin<Box<dyn Stream<Item = Result<SingleAttemptFrame, InvokeError>> + Send>>;

/// Generic HTTP executor with exactly one request send per invocation.
pub struct SingleAttemptHttpExecutor {
    http: reqwest::Client,
    credentials: Arc<dyn OpaqueCredentialResolver>,
    authenticator: Arc<dyn SingleAttemptAuthenticator>,
    max_line_bytes: usize,
}

impl SingleAttemptHttpExecutor {
    pub const DEFAULT_MAX_LINE_BYTES: usize = 1024 * 1024;

    pub fn new(
        http: reqwest::Client,
        credentials: Arc<dyn OpaqueCredentialResolver>,
    ) -> Self {
        Self {
            http,
            credentials,
            authenticator: Arc::new(DefaultSingleAttemptAuthenticator),
            max_line_bytes: Self::DEFAULT_MAX_LINE_BYTES,
        }
    }

    pub fn with_authenticator(
        mut self,
        authenticator: Arc<dyn SingleAttemptAuthenticator>,
    ) -> Self {
        self.authenticator = authenticator;
        self
    }

    pub fn with_max_line_bytes(mut self, max_line_bytes: usize) -> Result<Self, InvokeError> {
        if max_line_bytes == 0 {
            return Err(InvokeError::config(
                "single-attempt stream line limit must be greater than zero",
            ));
        }
        self.max_line_bytes = max_line_bytes;
        Ok(self)
    }

    /// Execute one request.  This method intentionally has no retry loop,
    /// backoff, key rotation, or alternate credential lookup.
    pub async fn open_stream(
        &self,
        request: SingleAttemptRequest,
    ) -> Result<SingleAttemptStream, InvokeError> {
        request.validate()?;
        let material = self.credentials.resolve(&request.credential).await?;
        material.validate_credentials()?;

        let response = self.send_once(&request, &material).await?;
        if !response.status().is_success() {
            return Err(error_from_response(response)
                .await
                .redacted(&material.secret_redactor()));
        }
        if let Some(content_type) =
            nomifun_net::api_response::is_non_api_content_type(response.headers())
        {
            let status = response.status().as_u16();
            return Err(InvokeError::non_api_response(status, &content_type));
        }

        let json_response = request.framing == SingleAttemptFraming::Sse
            && json_chat_protocol(&request.protocol)
            && response_is_json(&response);
        let bytes = response.bytes_stream().map(|chunk| {
            chunk.map_err(net_err).map(|bytes| bytes.to_vec())
        });
        if json_response {
            return Ok(Box::pin(JsonFrameStream::new(
                bytes,
                self.max_line_bytes,
            )));
        }
        Ok(stream_response(
            bytes,
            request.framing,
            self.max_line_bytes,
        ))
    }

    async fn send_once(
        &self,
        request: &SingleAttemptRequest,
        material: &AuthMaterial,
    ) -> Result<reqwest::Response, InvokeError> {
        let body = serde_json::to_vec(&request.body)
            .map_err(|_| InvokeError::parse("provider request body is not serializable"))?;
        let builder = self
            .http
            .post(request.url.trim())
            .timeout(request.timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone());
        self.authenticator
            .apply(builder, request, material, &body)?
            .send()
            .await
            .map_err(|error| net_err(error).redacted(&material.secret_redactor()))
    }
}

fn stream_response<S>(
    source: S,
    framing: SingleAttemptFraming,
    max_line_bytes: usize,
) -> SingleAttemptStream
where
    S: Stream<Item = Result<Vec<u8>, InvokeError>> + Unpin + Send + 'static,
{
    match framing {
        SingleAttemptFraming::Sse => Box::pin(SseFrameStream::new(source, max_line_bytes)),
        SingleAttemptFraming::AwsEventStream => Box::pin(BedrockFrameStream::new(source)),
    }
}

fn response_is_json(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("application/json")
                || value
                    .to_ascii_lowercase()
                    .strip_suffix("+json")
                    .is_some_and(|prefix| prefix.starts_with("application/"))
        })
}

fn json_chat_protocol(protocol: &str) -> bool {
    matches!(
        protocol.trim().to_ascii_lowercase().as_str(),
        "openai.chat"
            | "openai.chat_text"
            | "gemini"
            | "google.gemini"
            | "gemini.generate_text"
    )
}

struct SseFrameStream<S> {
    source: S,
    buffer: Vec<u8>,
    event: String,
    data: String,
    max_line_bytes: usize,
    finished: bool,
}

struct JsonFrameStream<S> {
    source: S,
    buffer: Vec<u8>,
    max_bytes: usize,
    finished: bool,
    emitted: bool,
}

impl<S> JsonFrameStream<S> {
    fn new(source: S, max_bytes: usize) -> Self {
        Self {
            source,
            buffer: Vec::new(),
            max_bytes,
            finished: false,
            emitted: false,
        }
    }
}

impl<S> Stream for JsonFrameStream<S>
where
    S: Stream<Item = Result<Vec<u8>, InvokeError>> + Unpin,
{
    type Item = Result<SingleAttemptFrame, InvokeError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.emitted {
            return std::task::Poll::Ready(None);
        }

        while !self.finished {
            match Pin::new(&mut self.source).poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => {
                    self.buffer.extend_from_slice(&bytes);
                    if self.buffer.len() > self.max_bytes {
                        self.finished = true;
                        self.emitted = true;
                        return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                            "provider JSON response exceeds the configured limit",
                        ))));
                    }
                }
                std::task::Poll::Ready(Some(Err(error))) => {
                    self.finished = true;
                    self.emitted = true;
                    return std::task::Poll::Ready(Some(Err(error)));
                }
                std::task::Poll::Ready(None) => {
                    self.finished = true;
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }

        self.emitted = true;
        if self.buffer.is_empty() {
            return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                "provider JSON response is empty",
            ))));
        }
        let data = match serde_json::from_slice(&self.buffer) {
            Ok(data) => data,
            Err(_) => {
                return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                    "provider JSON response is not valid JSON",
                ))));
            }
        };
        std::task::Poll::Ready(Some(Ok(SingleAttemptFrame {
            event: "json".to_owned(),
            data,
        })))
    }
}

impl<S> SseFrameStream<S> {
    fn new(source: S, max_line_bytes: usize) -> Self {
        Self {
            source,
            buffer: Vec::new(),
            event: String::new(),
            data: String::new(),
            max_line_bytes,
            finished: false,
        }
    }
}

impl<S> Stream for SseFrameStream<S>
where
    S: Stream<Item = Result<Vec<u8>, InvokeError>> + Unpin,
{
    type Item = Result<SingleAttemptFrame, InvokeError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            let finished = self.finished;
            match self.take_frame(finished) {
                Ok(Some(frame)) => return std::task::Poll::Ready(Some(Ok(frame))),
                Ok(None) => {}
                Err(error) => return std::task::Poll::Ready(Some(Err(error))),
            }
            if self.finished {
                return std::task::Poll::Ready(None);
            }
            match Pin::new(&mut self.source).poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => {
                    self.buffer.extend_from_slice(&bytes);
                    let line_too_long = self
                        .buffer
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .is_none()
                        && self.buffer.len() > self.max_line_bytes;
                    if line_too_long {
                        return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                            "provider SSE line exceeds the configured limit",
                        ))));
                    }
                }
                std::task::Poll::Ready(Some(Err(error))) => {
                    return std::task::Poll::Ready(Some(Err(error)));
                }
                std::task::Poll::Ready(None) => {
                    self.finished = true;
                    if self.data.is_empty() && self.event.is_empty() {
                        return std::task::Poll::Ready(None);
                    }
                    match self.take_frame(true) {
                        Ok(Some(frame)) => return std::task::Poll::Ready(Some(Ok(frame))),
                        Ok(None) => {}
                        Err(error) => return std::task::Poll::Ready(Some(Err(error))),
                    }
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

impl<S> SseFrameStream<S> {
    fn take_frame(
        &mut self,
        eof: bool,
    ) -> Result<Option<SingleAttemptFrame>, InvokeError> {
        loop {
            let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                if eof {
                    let line = std::mem::take(&mut self.buffer);
                    if !line.is_empty() {
                        self.consume_line(&line)?;
                    }
                    return self.finish_event();
                }
                return Ok(None);
            };
            let line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            self.consume_line(&line)?;
            if line.iter().all(|byte| matches!(byte, b'\n' | b'\r')) {
                return self.finish_event();
            }
        }
    }

    fn consume_line(&mut self, line: &[u8]) -> Result<(), InvokeError> {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.starts_with(b":") {
            return Ok(());
        }
        let (field, value): (&[u8], &[u8]) = match line.iter().position(|byte| *byte == b':') {
            Some(index) => {
                let value = &line[index + 1..];
                (&line[..index], value.strip_prefix(b" ").unwrap_or(value))
            }
            None => (line, &[] as &[u8]),
        };
        let value = std::str::from_utf8(value)
            .map_err(|_| InvokeError::parse("provider SSE line is not UTF-8"))?;
        match field {
            b"event" => self.event = value.to_owned(),
            b"data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
        Ok(())
    }

    fn finish_event(&mut self) -> Result<Option<SingleAttemptFrame>, InvokeError> {
        if self.event.is_empty() && self.data.is_empty() {
            return Ok(None);
        }
        let event = if self.event.is_empty() {
            "message"
        } else {
            self.event.as_str()
        }
        .to_owned();
        let data = std::mem::take(&mut self.data);
        self.event.clear();
        if data.trim() == "[DONE]" {
            return Ok(Some(SingleAttemptFrame {
                event: "done".to_owned(),
                data: Value::Object(Default::default()),
            }));
        }
        let data = serde_json::from_str(&data)
            .map_err(|_| InvokeError::parse("provider SSE data is not valid JSON"))?;
        Ok(Some(SingleAttemptFrame { event, data }))
    }
}

struct BedrockFrameStream<S> {
    source: S,
    buffer: Vec<u8>,
    finished: bool,
}

impl<S> BedrockFrameStream<S> {
    fn new(source: S) -> Self {
        Self {
            source,
            buffer: Vec::new(),
            finished: false,
        }
    }
}

impl<S> Stream for BedrockFrameStream<S>
where
    S: Stream<Item = Result<Vec<u8>, InvokeError>> + Unpin,
{
    type Item = Result<SingleAttemptFrame, InvokeError>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            if self.buffer.len() >= 12 {
                let total = u32::from_be_bytes(self.buffer[0..4].try_into().unwrap()) as usize;
                let headers = u32::from_be_bytes(self.buffer[4..8].try_into().unwrap()) as usize;
                if !(16..=16 * 1024 * 1024).contains(&total)
                    || headers > total.saturating_sub(16)
                {
                    return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                        "invalid Bedrock event-stream frame length",
                    ))));
                }
                if self.buffer.len() < total {
                    // Need more bytes from the network.
                } else {
                    let frame = self.buffer.drain(..total).collect::<Vec<_>>();
                    if let Err(error) = validate_event_stream_crc(&frame, total, headers) {
                        return std::task::Poll::Ready(Some(Err(error)));
                    }
                    let payload_start = 12 + headers;
                    let payload_end = total - 4;
                    let payload = &frame[payload_start..payload_end];
                    return std::task::Poll::Ready(Some(decode_bedrock_payload(payload)));
                }
            }
            if self.finished {
                if self.buffer.is_empty() {
                    return std::task::Poll::Ready(None);
                }
                return std::task::Poll::Ready(Some(Err(InvokeError::parse(
                    "Bedrock event stream ended mid-frame",
                ))));
            }
            match Pin::new(&mut self.source).poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => self.buffer.extend_from_slice(&bytes),
                std::task::Poll::Ready(Some(Err(error))) => {
                    return std::task::Poll::Ready(Some(Err(error)));
                }
                std::task::Poll::Ready(None) => self.finished = true,
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

fn validate_event_stream_crc(
    frame: &[u8],
    total: usize,
    headers: usize,
) -> Result<(), InvokeError> {
    if frame.len() != total || total < 16 || headers > total - 16 {
        return Err(InvokeError::parse(
            "invalid Bedrock event-stream frame bounds",
        ));
    }

    // AWS event-stream layout:
    // total length (4), headers length (4), prelude CRC (4), headers,
    // payload, message CRC (4). Both CRCs are CRC32/IEEE.
    let mut prelude = Hasher::new();
    prelude.update(&frame[..8]);
    let expected_prelude = u32::from_be_bytes(frame[8..12].try_into().unwrap());
    if prelude.finalize() != expected_prelude {
        return Err(InvokeError::parse(
            "Bedrock event-stream prelude CRC mismatch",
        ));
    }

    let mut message = Hasher::new();
    message.update(&frame[..total - 4]);
    let expected_message = u32::from_be_bytes(frame[total - 4..].try_into().unwrap());
    if message.finalize() != expected_message {
        return Err(InvokeError::parse(
            "Bedrock event-stream message CRC mismatch",
        ));
    }
    Ok(())
}

fn decode_bedrock_payload(payload: &[u8]) -> Result<SingleAttemptFrame, InvokeError> {
    let wrapper: Value = serde_json::from_slice(payload)
        .map_err(|_| InvokeError::parse("Bedrock event payload is not JSON"))?;
    // Bedrock model streams commonly carry the model event JSON directly.
    // Some gateway fixtures wrap that JSON in a base64 `bytes` field; accept
    // both shapes while keeping the payload bounded by the frame limit.
    let data: Value = match wrapper.get("bytes").and_then(Value::as_str) {
        Some(encoded) => {
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                encoded,
            )
            .map_err(|_| InvokeError::parse("Bedrock event payload bytes are not base64"))?;
            serde_json::from_slice(&bytes)
                .map_err(|_| InvokeError::parse("Bedrock model event is not JSON"))?
        }
        None => wrapper,
    };
    let event = data
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message")
        .to_owned();
    Ok(SingleAttemptFrame { event, data })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::stream;
    use serde_json::json;

    use super::*;
    use crate::error::InvokeErrorKind;

    struct FixtureCredentials {
        material: AuthMaterial,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl OpaqueCredentialResolver for FixtureCredentials {
        async fn resolve(
            &self,
            lease: &OpaqueCredentialLease,
        ) -> Result<AuthMaterial, InvokeError> {
            assert_eq!(lease.handle(), "fixture-lease");
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(self.material.clone())
        }
    }

    fn executor() -> (SingleAttemptHttpExecutor, Arc<FixtureCredentials>) {
        let credentials = Arc::new(FixtureCredentials {
            material: AuthMaterial {
                scheme: AuthScheme::Bearer,
                credentials: json!({"api_keys": ["fixture-secret"]}),
            },
            calls: AtomicUsize::new(0),
        });
        (
            SingleAttemptHttpExecutor::new(
                reqwest::Client::builder().no_proxy().build().unwrap(),
                credentials.clone(),
            ),
            credentials,
        )
    }

    #[tokio::test]
    async fn single_attempt_success_preserves_sse_frames_and_does_not_retry() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "event: response.created\ndata: {\"id\":\"r1\"}\n\n\
                 event: text.delta\ndata: {\"text\":\"hello\"}\n\n\
                 data: [DONE]\n\n",
            ))
            .mount(&server)
            .await;
        let (executor, credentials) = executor();
        let stream = executor
            .open_stream(SingleAttemptRequest {
                protocol: "openai.chat".into(),
                url: format!("{}/chat", server.uri()),
                model: "fixture-model".into(),
                body: json!({"stream": true}),
                credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
                timeout: Duration::from_secs(5),
                framing: SingleAttemptFraming::Sse,
                region: None,
            })
            .await
            .unwrap();
        let frames = stream.collect::<Vec<_>>().await;
        assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
        assert_eq!(frames[0].as_ref().unwrap().event, "response.created");
        assert_eq!(frames[1].as_ref().unwrap().data["text"], "hello");
        assert_eq!(frames[2].as_ref().unwrap().event, "done");
    }

    #[tokio::test]
    async fn chat_executor_preserves_raw_openai_sse_and_gemini_json_shapes() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        "data: {\"id\":\"chatcmpl_raw_executor\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n\
                         data: [DONE]\n\n",
                        "text/event-stream",
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/gemini"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(
                        r#"{"candidates":[{"content":{"parts":[{"text":"gemini"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":2}}"#,
                        "application/json; charset=utf-8",
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/gemini-sse"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(
                    "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"streamed\"}]}}],\"usageMetadata\":{\"promptTokenCount\":9,\"candidatesTokenCount\":3}}\n\n\
                     data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
            )
            .mount(&server)
            .await;

        let (executor, credentials) = executor();
        let openai_stream = executor
            .open_stream(SingleAttemptRequest {
                protocol: "openai.chat".into(),
                url: format!("{}/openai", server.uri()),
                model: "fixture-model".into(),
                body: json!({"stream": true}),
                credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
                timeout: Duration::from_secs(5),
                framing: SingleAttemptFraming::Sse,
                region: None,
            })
            .await
            .unwrap();
        let openai_frames = openai_stream.collect::<Vec<_>>().await;
        assert_eq!(openai_frames.len(), 2);
        assert_eq!(
            openai_frames[0].as_ref().unwrap().data["choices"][0]["delta"]["content"],
            "hello"
        );
        assert_eq!(openai_frames[1].as_ref().unwrap().event, "done");

        let gemini_stream = executor
            .open_stream(SingleAttemptRequest {
                protocol: "gemini.generate_text".into(),
                url: format!("{}/gemini", server.uri()),
                model: "fixture-model".into(),
                body: json!({"contents": []}),
                credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
                timeout: Duration::from_secs(5),
                framing: SingleAttemptFraming::Sse,
                region: None,
            })
            .await
            .unwrap();
        let gemini_frames = gemini_stream.collect::<Vec<_>>().await;
        assert_eq!(gemini_frames.len(), 1);
        let gemini_frame = gemini_frames[0].as_ref().unwrap();
        assert_eq!(gemini_frame.event, "json");
        assert_eq!(
            gemini_frame.data["candidates"][0]["content"]["parts"][0]["text"],
            "gemini"
        );
        assert_eq!(
            gemini_frame.data["candidates"][0]["finishReason"],
            "STOP"
        );
        assert_eq!(
            gemini_frame.data["usageMetadata"]["promptTokenCount"],
            7
        );

        let gemini_sse_stream = executor
            .open_stream(SingleAttemptRequest {
                protocol: "gemini.generate_text".into(),
                url: format!("{}/gemini-sse", server.uri()),
                model: "fixture-model".into(),
                body: json!({"contents": []}),
                credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
                timeout: Duration::from_secs(5),
                framing: SingleAttemptFraming::Sse,
                region: None,
            })
            .await
            .unwrap();
        let gemini_sse_frames = gemini_sse_stream.collect::<Vec<_>>().await;
        assert_eq!(gemini_sse_frames.len(), 3);
        assert_eq!(
            gemini_sse_frames[0]
                .as_ref()
                .unwrap()
                .data["candidates"][0]["content"]["parts"][0]["text"],
            "streamed"
        );
        assert_eq!(
            gemini_sse_frames[0].as_ref().unwrap().data["usageMetadata"]["candidatesTokenCount"],
            3
        );
        assert_eq!(
            gemini_sse_frames[1].as_ref().unwrap().data["candidates"][0]["finishReason"],
            "STOP"
        );
        assert_eq!(gemini_sse_frames[2].as_ref().unwrap().event, "done");
        assert_eq!(credentials.calls.load(Ordering::Acquire), 3);
    }

    #[tokio::test]
    async fn provider_unavailable_is_typed_and_single_shot() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let server = wiremock::MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat"))
            .respond_with(
                ResponseTemplate::new(503)
                    .set_body_string(r#"{"error":{"message":"upstream unavailable"}}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (executor, credentials) = executor();
        let result = executor
            .open_stream(SingleAttemptRequest {
                protocol: "anthropic".into(),
                url: format!("{}/chat", server.uri()),
                model: "fixture-model".into(),
                body: json!({"stream": true}),
                credential: OpaqueCredentialLease::new("fixture-lease").unwrap(),
                timeout: Duration::from_secs(5),
                framing: SingleAttemptFraming::Sse,
                region: None,
            })
            .await;
        let error = match result {
            Ok(_) => panic!("provider-unavailable response must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind, InvokeErrorKind::ProviderError);
        assert_eq!(error.http_status, Some(503));
        assert_eq!(credentials.calls.load(Ordering::Acquire), 1);
    }

    #[test]
    fn opaque_lease_debug_never_exposes_handle() {
        let lease = OpaqueCredentialLease::new("secret-runtime-handle").unwrap();
        let debug = format!("{lease:?}");
        assert!(!debug.contains("secret-runtime-handle"));
        assert!(debug.contains("opaque"));
    }

    #[test]
    fn bedrock_payload_conversion_preserves_native_event_type() {
        let inner = br#"{"type":"content_block_delta","delta":{"text":"hi"}}"#;
        let wrapped = json!({
            "bytes": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                inner,
            )
        });
        let frame = decode_bedrock_payload(wrapped.to_string().as_bytes()).unwrap();
        assert_eq!(frame.event, "content_block_delta");
        assert_eq!(frame.data["delta"]["text"], "hi");
    }

    fn event_stream_frame(headers: &[u8], payload: &[u8]) -> Vec<u8> {
        let total = 16usize
            .saturating_add(headers.len())
            .saturating_add(payload.len());
        let total_u32 = u32::try_from(total).unwrap();
        let headers_u32 = u32::try_from(headers.len()).unwrap();
        let mut frame = Vec::with_capacity(total);
        frame.extend_from_slice(&total_u32.to_be_bytes());
        frame.extend_from_slice(&headers_u32.to_be_bytes());
        let mut prelude = Hasher::new();
        prelude.update(&frame);
        frame.extend_from_slice(&prelude.finalize().to_be_bytes());
        frame.extend_from_slice(headers);
        frame.extend_from_slice(payload);
        let mut message = Hasher::new();
        message.update(&frame);
        frame.extend_from_slice(&message.finalize().to_be_bytes());
        frame
    }

    #[test]
    fn event_stream_crc_accepts_valid_direct_json_payload() {
        let frame = event_stream_frame(
            &[],
            br#"{"type":"content_block_delta","delta":{"text":"hi"}}"#,
        );
        validate_event_stream_crc(&frame, frame.len(), 0).unwrap();
        let decoded = decode_bedrock_payload(&frame[12..frame.len() - 4]).unwrap();
        assert_eq!(decoded.event, "content_block_delta");
        assert_eq!(decoded.data["delta"]["text"], "hi");
    }

    #[test]
    fn event_stream_crc_rejects_corrupted_prelude_and_message() {
        let valid = event_stream_frame(&[], br#"{"type":"message_stop"}"#);

        let mut bad_prelude = valid.clone();
        bad_prelude[0] ^= 1;
        let error = validate_event_stream_crc(&bad_prelude, bad_prelude.len(), 0).unwrap_err();
        assert!(error.message.contains("prelude CRC"));

        let mut bad_message = valid;
        let last = bad_message.len() - 1;
        bad_message[last] ^= 1;
        let error = validate_event_stream_crc(&bad_message, bad_message.len(), 0).unwrap_err();
        assert!(error.message.contains("message CRC"));
    }

    #[tokio::test]
    async fn sse_parser_emits_frames_from_one_chunk() {
        let source = stream::iter([Ok::<_, InvokeError>(
            b"event: response.created\ndata: {\"id\":\"r1\"}\n\n\
              event: text.delta\ndata: {\"text\":\"hello\"}\n\n\
              data: [DONE]\n\n"
                .to_vec(),
        )]);
        let frames = stream_response(source, SingleAttemptFraming::Sse, 1024)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(frames.len(), 3, "frames: {frames:?}");
    }

}
