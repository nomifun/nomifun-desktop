//! Authenticated upstream connections; secrets never cross the neutral port.

use crate::{AuthScheme, InvokeError, InvokeErrorKind, ResolvedConnection};
use futures_util::{SinkExt, StreamExt};
use nomifun_voice_contracts::voice::{AudioEncoding, MediaSpec};
use nomifun_voice_core::VoiceSessionLimits;
use reqwest::Url;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{
    Message, client::IntoClientRequest, protocol::WebSocketConfig,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_tls_with_config};
use tokio_util::sync::CancellationToken;

pub(super) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const MAX_READ_WIRE_BYTES: usize = 256 * 1024;
const READ_EVENT_SLOTS: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PumpError {
    Network,
    Backlog,
    Deadline,
    Closed,
}
#[derive(Clone, Copy)]
enum WriteLane {
    Urgent,
    Control,
    Media,
}
struct WriteRequest {
    message: Message,
    deadline: Instant,
    media: bool,
    ack: oneshot::Sender<Result<(), PumpError>>,
}
pub(super) struct Writer {
    urgent: mpsc::Sender<WriteRequest>,
    control: mpsc::Sender<WriteRequest>,
    media: mpsc::Sender<WriteRequest>,
    cancel: CancellationToken,
    write_timeout: Duration,
}
struct ReadRequest {
    message: Message,
    received_at: Instant,
    media: bool,
    _duration: Option<OwnedSemaphorePermit>,
    _wire_bytes: Option<OwnedSemaphorePermit>,
}
pub(super) struct Reader {
    rx: mpsc::Receiver<ReadRequest>,
    failure: watch::Receiver<Option<PumpError>>,
    max_media_age: Duration,
}
pub(super) struct PumpGuard {
    cancel: CancellationToken,
    workers: Vec<JoinHandle<()>>,
}
impl Writer {
    async fn enqueue(
        &mut self,
        message: Message,
        lane: WriteLane,
        timeout: Duration,
    ) -> Result<(), PumpError> {
        // A single physical socket write cannot safely be preempted midway
        // through a frame. Bound every write by the media age budget so a
        // stalled ordinary control cannot hold the urgent lane for seconds.
        let timeout = timeout.min(self.write_timeout);
        let (ack, receipt) = oneshot::channel();
        let request = WriteRequest {
            message,
            deadline: Instant::now() + timeout,
            media: matches!(lane, WriteLane::Media),
            ack,
        };
        let tx = match lane {
            WriteLane::Urgent => &self.urgent,
            WriteLane::Control => &self.control,
            WriteLane::Media => &self.media,
        };
        tx.try_send(request).map_err(|error| {
            if matches!(error, mpsc::error::TrySendError::Full(_)) {
                PumpError::Backlog
            } else {
                PumpError::Closed
            }
        })?;
        tokio::select! {biased;_=self.cancel.cancelled()=>Err(PumpError::Closed),
        result=tokio::time::timeout(timeout+Duration::from_millis(20),receipt)=>result.map_err(|_|PumpError::Deadline)?.map_err(|_|PumpError::Closed)?}
    }
    pub async fn send(&mut self, message: Message) -> Result<(), PumpError> {
        let lane = if matches!(message, Message::Close(_) | Message::Pong(_)) {
            WriteLane::Urgent
        } else {
            WriteLane::Control
        };
        self.enqueue(message, lane, self.write_timeout).await
    }
}
impl Reader {
    pub async fn next(&mut self) -> Option<Result<Message, PumpError>> {
        if *self.failure.borrow() == Some(PumpError::Backlog) {
            return Some(Err(PumpError::Backlog));
        }
        let request = match self.rx.try_recv() {
            Ok(request) => request,
            Err(mpsc::error::TryRecvError::Disconnected) => return self.failure.borrow().map(Err),
            Err(mpsc::error::TryRecvError::Empty) => tokio::select! {biased;
                request=self.rx.recv()=>match request{Some(request)=>request,None=>return self.failure.borrow().map(Err)},
                result=self.failure.changed()=>{return match *self.failure.borrow(){Some(error)=>Some(Err(error)),None if result.is_err()=>None,None=>Some(Err(PumpError::Closed))};}
            },
        };
        if request.media && request.received_at.elapsed() > self.max_media_age {
            return Some(Err(PumpError::Backlog));
        }
        Some(Ok(request.message))
    }
}
impl PumpGuard {
    pub async fn shutdown(&mut self) {
        self.cancel.cancel();
        for mut worker in self.workers.drain(..) {
            if tokio::time::timeout(Duration::from_secs(1), &mut worker)
                .await
                .is_err()
            {
                worker.abort();
                let _ = worker.await;
            }
        }
    }
}
impl Drop for PumpGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        let workers = std::mem::take(&mut self.workers);
        for worker in &workers {
            worker.abort();
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                for worker in workers {
                    let _ = worker.await;
                }
            });
        }
    }
}

/// Reading never waits on a socket write or on a product consumer. PCM wire
/// packets reserve a duration permit before queueing and retain their actual
/// receive clock. Control writes are independent of the single media slot.
pub(super) fn pump(
    socket: Socket,
    output_spec: Option<MediaSpec>,
    limits: VoiceSessionLimits,
) -> (Writer, Reader, PumpGuard) {
    let (mut sink, mut stream) = socket.split();
    let cancel = CancellationToken::new();
    let (failure_tx, failure) = watch::channel(None);
    let (urgent_tx, mut urgent_rx) = mpsc::channel::<WriteRequest>(8);
    let (control_tx, mut control_rx) = mpsc::channel::<WriteRequest>(32);
    let (media_tx, mut media_rx) = mpsc::channel::<WriteRequest>(1);
    // A legitimate context publication has many tiny ACKs. Count and byte
    // limits bound that burst independently of the unchanged audio-duration
    // budget; slots never stand in for seconds of queued sound.
    let (read_tx, read_rx) = mpsc::channel::<ReadRequest>(READ_EVENT_SLOTS);
    let wire_bytes = Arc::new(Semaphore::new(MAX_READ_WIRE_BYTES));
    let age = Duration::from_micros(
        output_spec
            .as_ref()
            .map_or(200_000, |spec| u64::from(spec.max_frame_age_us)),
    );
    let duration = Arc::new(Semaphore::new(
        output_spec
            .as_ref()
            .map_or(1, |spec| spec.max_buffer_duration_us.div_ceil(1000)) as usize,
    ));
    let write_cancel = cancel.clone();
    let write_failure = failure_tx.clone();
    let writer = tokio::spawn(async move {
        loop {
            let request = tokio::select! {biased;_=write_cancel.cancelled()=>break,
            request=urgent_rx.recv()=>request,request=control_rx.recv()=>request,request=media_rx.recv()=>request};
            let Some(request) = request else {
                break;
            };
            let result = tokio::select! {biased;_=write_cancel.cancelled()=>Err(PumpError::Closed),
            result=tokio::time::timeout_at(request.deadline,sink.send(request.message))=>match result{Ok(Ok(()))=>Ok(()),Ok(Err(_))=>Err(PumpError::Network),Err(_)=>Err(if request.media{PumpError::Backlog}else{PumpError::Deadline})}};
            let failed = result.as_ref().err().copied();
            let _ = request.ack.send(result);
            if let Some(error) = failed {
                write_failure.send_replace(Some(error));
                break;
            }
        }
    });
    let read_cancel = cancel.clone();
    let reader = tokio::spawn(async move {
        loop {
            let incoming = tokio::select! {biased;_=read_cancel.cancelled()=>break,message=stream.next()=>message};
            let message = match incoming {
                Some(Ok(message)) => message,
                _ => {
                    failure_tx.send_replace(Some(PumpError::Network));
                    break;
                }
            };
            let received_at = Instant::now();
            let mut media = false;
            let mut permit = None;
            if let Message::Text(text) = &message {
                if let Ok(raw) = serde_json::from_str::<Value>(text) {
                    let kind = raw.get("type").and_then(Value::as_str).unwrap_or("");
                    let audio =
                        kind.ends_with(".audio.delta") || kind.ends_with(".output_audio.delta");
                    if audio {
                        let Some(spec) = &output_spec else {
                            continue;
                        };
                        let encoded = raw.get("delta").and_then(Value::as_str).unwrap_or("");
                        let bytes = encoded.len() / 4 * 3
                            - encoded
                                .as_bytes()
                                .iter()
                                .rev()
                                .take_while(|c| **c == b'=')
                                .count()
                                .min(encoded.len() / 4 * 3);
                        let frame_bytes = spec.format.pcm_frame_bytes().unwrap_or(1);
                        let duration_ms = if spec.format.encoding == AudioEncoding::Pcm {
                            ((bytes / frame_bytes) as u64 * 1000
                                / u64::from(spec.format.sample_rate))
                            .max(1)
                        } else {
                            1
                        };
                        match duration
                            .clone()
                            .try_acquire_many_owned(duration_ms.min(u64::from(u32::MAX)) as u32)
                        {
                            Ok(p) => permit = Some(p),
                            Err(_) => {
                                failure_tx.send_replace(Some(PumpError::Backlog));
                                break;
                            }
                        }
                        media = true;
                    }
                    if output_spec.is_none() && kind == "session.input_audio.append" {
                        continue;
                    }
                }
            }
            let closed = matches!(message, Message::Close(_));
            let wire_permit = match wire_bytes
                .clone()
                .try_acquire_many_owned(message.len().max(1).min(u32::MAX as usize) as u32)
            {
                Ok(permit) => permit,
                Err(_) => {
                    failure_tx.send_replace(Some(PumpError::Backlog));
                    break;
                }
            };
            if read_tx
                .try_send(ReadRequest {
                    message,
                    received_at,
                    media,
                    _duration: permit,
                    _wire_bytes: Some(wire_permit),
                })
                .is_err()
            {
                failure_tx.send_replace(Some(PumpError::Backlog));
                break;
            }
            if closed {
                break;
            }
            // Give the model actor a turn even when many protocol/control
            // messages arrive in one TCP read; the duration permits still
            // bound audio and queue overflow remains an explicit failure.
            tokio::task::yield_now().await;
        }
    });
    (
        Writer {
            urgent: urgent_tx,
            control: control_tx,
            media: media_tx,
            cancel: cancel.clone(),
            write_timeout: limits.write_timeout.min(age),
        },
        Reader {
            rx: read_rx,
            failure,
            max_media_age: age,
        },
        PumpGuard {
            cancel,
            workers: vec![reader, writer],
        },
    )
}

pub(super) async fn connect(
    connection: &ResolvedConnection,
    endpoint: &Url,
    timeout: Duration,
    max_message_bytes: usize,
) -> Result<Socket, InvokeError> {
    if connection.auth.scheme != AuthScheme::Bearer {
        return Err(InvokeError::config(
            "voice protocol requires bearer authentication",
        ));
    }
    connection.auth.validate()?;
    let redactor = connection.auth.secret_redactor();
    let secrets = connection.auth.secrets();
    let mut last_error = None;
    for (index, secret) in secrets.iter().enumerate() {
        let mut request = endpoint
            .as_str()
            .into_client_request()
            .map_err(|_| InvokeError::config("invalid voice WebSocket endpoint"))?;
        let mut authorization = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(
            &format!("Bearer {secret}"),
        )
        .map_err(|_| InvokeError::config("invalid voice credential header"))?;
        authorization.set_sensitive(true);
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
            authorization,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(max_message_bytes))
            .max_frame_size(Some(max_message_bytes));
        let attempt = tokio::time::timeout(
            timeout,
            connect_async_tls_with_config(request, Some(config), false, None),
        )
        .await;
        match attempt {
            Ok(Ok((socket, _))) => return Ok(socket),
            Err(_) => {
                last_error = Some(InvokeError::new(
                    InvokeErrorKind::Timeout,
                    "voice WebSocket handshake timed out",
                ));
                break;
            }
            Ok(Err(error)) => {
                let mapped = socket_error(error).redacted(&redactor);
                let rotate = matches!(
                    mapped.kind,
                    InvokeErrorKind::Auth
                        | InvokeErrorKind::RateLimited
                        | InvokeErrorKind::QuotaExhausted
                );
                last_error = Some(mapped);
                if !rotate || index + 1 == secrets.len() {
                    break;
                }
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| InvokeError::config("voice connection has no usable credentials")))
}

fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> InvokeError {
    if let tokio_tungstenite::tungstenite::Error::Http(response) = error {
        let status = response.status().as_u16();
        let kind = match status {
            401 | 403 => InvokeErrorKind::Auth,
            429 => InvokeErrorKind::RateLimited,
            408 | 504 => InvokeErrorKind::Timeout,
            400 | 404 | 422 => InvokeErrorKind::InvalidParams,
            _ => InvokeErrorKind::ProviderError,
        };
        // HTTP body and endpoint are untrusted and may contain secrets.
        InvokeError::new(kind, format!("voice handshake rejected with HTTP {status}"))
            .with_http_status(status)
    } else {
        InvokeError::new(
            InvokeErrorKind::Network,
            "voice WebSocket connection failed",
        )
    }
}

pub(super) async fn send_json(
    writer: &mut Writer,
    value: Value,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(), String> {
    let text =
        serde_json::to_string(&value).map_err(|_| "voice command encoding failed".to_string())?;
    if text.len() > max_bytes {
        return Err("voice command exceeds control-message limit".into());
    }
    writer
        .enqueue(Message::Text(text.into()), WriteLane::Control, timeout)
        .await
        .map_err(|_| "voice control write failed".to_string())
}
pub(super) async fn send_urgent_json(
    writer: &mut Writer,
    value: Value,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(), String> {
    send_lane(writer, value, timeout, max_bytes, WriteLane::Urgent).await
}
pub(super) async fn send_media_json(
    writer: &mut Writer,
    value: Value,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(), PumpError> {
    let text = serde_json::to_string(&value).map_err(|_| PumpError::Network)?;
    if text.len() > max_bytes {
        return Err(PumpError::Backlog);
    }
    writer
        .enqueue(Message::Text(text.into()), WriteLane::Media, timeout)
        .await
}
async fn send_lane(
    writer: &mut Writer,
    value: Value,
    timeout: Duration,
    max_bytes: usize,
    lane: WriteLane,
) -> Result<(), String> {
    let text =
        serde_json::to_string(&value).map_err(|_| "voice command encoding failed".to_string())?;
    if text.len() > max_bytes {
        return Err("voice command exceeds message limit".into());
    }
    writer
        .enqueue(Message::Text(text.into()), lane, timeout)
        .await
        .map_err(|_| "voice socket write failed".into())
}

pub(super) async fn create_native(
    http: &reqwest::Client,
    connection: &ResolvedConnection,
    endpoint: &Url,
    body: &Value,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(Value, ResolvedConnection), InvokeError> {
    let redactor = connection.auth.secret_redactor();
    connection.auth.validate()?;
    let secrets = connection.auth.secrets();
    for (index, secret) in secrets.iter().enumerate() {
        // A native session belongs to the project selected by the successful
        // POST credential. Pin that lease for sideband attach and cleanup;
        // restarting the original ring could attach with a different project.
        let mut selected = connection.clone();
        selected.auth.credentials = serde_json::json!({"api_keys":[secret]});
        let response =
            crate::transport::post_json(http, endpoint.as_str(), timeout, &selected.auth, body)
                .await
                .map_err(|error| error.redacted(&redactor))?;
        if matches!(response.status().as_u16(), 401 | 403 | 429) && index + 1 < secrets.len() {
            continue;
        }
        if !response.status().is_success() {
            return Err(crate::transport::error_from_response(response)
                .await
                .redacted(&redactor));
        }
        let bytes = crate::transport::read_body_capped(response, max_bytes as u64)
            .await
            .map_err(|error| error.redacted(&redactor))?;
        let payload = serde_json::from_slice(&bytes).map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::ProviderError,
                "voice native creation returned invalid JSON",
            )
        })?;
        return Ok((payload, selected));
    }
    Err(InvokeError::config(
        "voice connection has no usable credentials",
    ))
}

pub(super) fn endpoint(
    connection: &ResolvedConnection,
    raw: &str,
    websocket: bool,
    allow_cross_origin: bool,
) -> Result<Url, InvokeError> {
    let credential_origin = connection
        .extra
        .get("nomifun_voice_credential_origin")
        .and_then(Value::as_str)
        .unwrap_or(&connection.base_url);
    if credential_origin != connection.base_url {
        crate::call::validate_credentialed_target_url(
            credential_origin,
            allow_cross_origin,
            &connection.base_url,
            "voice_base_url_override",
            if websocket {
                crate::manifest::ProtocolTransportKind::Websocket
            } else {
                crate::manifest::ProtocolTransportKind::Http
            },
            false,
        )?;
    }
    crate::call::validate_credentialed_target_url(
        &connection.base_url,
        allow_cross_origin,
        raw,
        "voice_endpoint",
        if websocket {
            crate::manifest::ProtocolTransportKind::Websocket
        } else {
            crate::manifest::ProtocolTransportKind::Http
        },
        true,
    )?;
    let resolved = if Url::parse(raw).is_ok() {
        raw.to_string()
    } else {
        format!(
            "{}/{}",
            connection.base_url.trim_end_matches('/'),
            raw.trim_start_matches('/')
        )
    };
    let mut url =
        Url::parse(&resolved).map_err(|_| InvokeError::config("invalid voice endpoint"))?;
    let scheme = if websocket {
        match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => other,
        }
    } else {
        match url.scheme() {
            "ws" => "http",
            "wss" => "https",
            other => other,
        }
    }
    .to_owned();
    url.set_scheme(&scheme)
        .map_err(|_| InvokeError::config("invalid voice endpoint scheme"))?;
    Ok(url)
}

#[cfg(test)]
mod pump_tests {
    use super::*;
    #[tokio::test]
    async fn control_wire_byte_permit_is_held_until_the_actor_consumes_it() {
        let (tx, rx) = mpsc::channel(1);
        let (_failure_tx, failure) = watch::channel(None);
        let bytes = Arc::new(Semaphore::new(8));
        assert!(
            tx.try_send(ReadRequest {
                message: Message::Text("12345678".into()),
                received_at: Instant::now(),
                media: false,
                _duration: None,
                _wire_bytes: Some(bytes.clone().try_acquire_many_owned(8).unwrap()),
            })
            .is_ok()
        );
        assert_eq!(bytes.available_permits(), 0);
        let mut reader = Reader {
            rx,
            failure,
            max_media_age: Duration::from_millis(200),
        };
        assert_eq!(
            reader.next().await.unwrap().unwrap(),
            Message::Text("12345678".into())
        );
        assert_eq!(bytes.available_permits(), 8);
    }
    #[tokio::test]
    async fn real_wire_control_burst_exceeding_256_kib_fails_before_actor_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sent_tx, sent_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for _ in 0..2 {
                socket.send(Message::Text(serde_json::json!({"type":"control_byte_limit_fixture","data":"x".repeat(160*1024)}).to_string().into())).await.unwrap();
            }
            let _ = sent_tx.send(());
            while let Some(message) = socket.next().await {
                if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                    break;
                }
            }
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
            .await
            .unwrap();
        let (_writer, mut reader, mut guard) = pump(socket, None, VoiceSessionLimits::default());
        sent_rx.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if *reader.failure.borrow() == Some(PumpError::Backlog) {
                    break;
                }
                reader.failure.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            reader.next().await.unwrap().unwrap_err(),
            PumpError::Backlog,
            "queued earlier success cannot hide a later byte-budget failure"
        );
        guard.shutdown().await;
        drop(reader);
        drop(_writer);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reader_preserves_backlog_when_failed_pump_drops_its_sender() {
        let (tx, rx) = mpsc::channel(1);
        let (failure_tx, failure) = watch::channel(None);
        let mut reader = Reader {
            rx,
            failure,
            max_media_age: Duration::from_millis(200),
        };
        let fail = tokio::spawn(async move {
            tokio::task::yield_now().await;
            failure_tx.send_replace(Some(PumpError::Backlog));
            drop(tx);
        });
        assert_eq!(
            reader.next().await.unwrap().unwrap_err(),
            PumpError::Backlog
        );
        fail.await.unwrap();
    }

    #[tokio::test]
    async fn reader_fences_real_media_age_and_releases_duration_budget() {
        let (tx, rx) = mpsc::channel(1);
        let (_failure_tx, failure) = watch::channel(None);
        let duration = Arc::new(Semaphore::new(120));
        let permit = duration.clone().acquire_many_owned(120).await.unwrap();
        assert!(
            tx.try_send(ReadRequest {
                message: Message::Text("old audio".into()),
                received_at: Instant::now() - Duration::from_millis(201),
                media: true,
                _duration: Some(permit),
                _wire_bytes: None,
            })
            .is_ok()
        );
        let mut reader = Reader {
            rx,
            failure,
            max_media_age: Duration::from_millis(200),
        };
        assert_eq!(duration.available_permits(), 0);
        assert_eq!(
            reader.next().await.unwrap().unwrap_err(),
            PumpError::Backlog
        );
        assert_eq!(duration.available_permits(), 120);
    }

    #[tokio::test]
    async fn reader_does_not_apply_media_clock_to_control() {
        let (tx, rx) = mpsc::channel(1);
        let (_failure_tx, failure) = watch::channel(None);
        assert!(
            tx.try_send(ReadRequest {
                message: Message::Text("closed receipt".into()),
                received_at: Instant::now() - Duration::from_secs(1),
                media: false,
                _duration: None,
                _wire_bytes: None,
            })
            .is_ok()
        );
        let mut reader = Reader {
            rx,
            failure,
            max_media_age: Duration::from_millis(200),
        };
        assert_eq!(
            reader.next().await.unwrap().unwrap(),
            Message::Text("closed receipt".into())
        );
    }
}
