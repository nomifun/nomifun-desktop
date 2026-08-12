//! `stepfun.realtime_s2s` — StepFun's bidirectional speech-to-speech
//! Realtime API.
//!
//! This is intentionally separate from StepFun streaming ASR
//! (`/realtime/asr/stream`) and streaming TTS (`/realtime/audio`). The normal
//! endpoint is supplied by the selected capability descriptor; StepFun and
//! Step Plan share the same event protocol.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Response;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::{Message, protocol::WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_tls_with_config};

use crate::auth::AuthScheme;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::realtime::{
    RealtimeClientCommand, RealtimeProtocolAdapter, RealtimeServerEvent, RealtimeSession,
    RealtimeSessionConfig, RealtimeSessionLimits, ResolvedRealtimeCall,
};
use super::json_request_body;
#[cfg(test)]
use crate::realtime::RealtimeTurnDetection;

const PROTOCOL_ID: &str = "stepfun.realtime_s2s";
static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

type UpstreamSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// StepFun bidirectional speech-to-speech adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct StepFunRealtimeAdapter {
    limits: RealtimeSessionLimits,
}

impl StepFunRealtimeAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: RealtimeSessionLimits) -> Result<Self, InvokeError> {
        limits.validate()?;
        Ok(Self { limits })
    }

    pub fn limits(&self) -> RealtimeSessionLimits {
        self.limits
    }
}

#[async_trait]
impl RealtimeProtocolAdapter for StepFunRealtimeAdapter {
    fn id(&self) -> &'static str {
        PROTOCOL_ID
    }

    async fn open(
        &self,
        call: &ResolvedRealtimeCall,
        config: RealtimeSessionConfig,
    ) -> Result<RealtimeSession, InvokeError> {
        self.limits.validate()?;
        validate_call(call)?;
        let config = provider_session_config(call, config)?;

        let url = build_realtime_url(call)?;
        let (socket, _response) = connect_with_key_rotation(call, &url, self.limits).await?;

        let (command_tx, command_rx) = mpsc::channel(self.limits.command_channel_capacity);
        let (event_tx, event_rx) = mpsc::channel(self.limits.event_channel_capacity);
        let limits = self.limits;
        let worker = tokio::spawn(async move {
            run_session(socket, config, limits, command_rx, event_tx).await;
        });

        Ok(RealtimeSession::from_parts(command_tx, event_rx, worker, self.limits))
    }
}

fn provider_session_config(
    call: &ResolvedRealtimeCall,
    mut config: RealtimeSessionConfig,
) -> Result<RealtimeSessionConfig, InvokeError> {
    config.extra = json_request_body(&call.model_params, &config.extra, json!({}))?;
    Ok(config)
}

fn validate_call(call: &ResolvedRealtimeCall) -> Result<(), InvokeError> {
    if call.model.trim().is_empty() {
        return Err(InvokeError::config("StepFun realtime model must not be empty"));
    }
    if !matches!(call.connection.auth.scheme, AuthScheme::Bearer) {
        return Err(InvokeError::config(
            "stepfun.realtime_s2s requires bearer authentication",
        ));
    }
    if call.connection.auth.secrets().is_empty() {
        return Err(InvokeError::config(
            "StepFun realtime connection credentials carry no API key",
        ));
    }
    Ok(())
}

/// Resolve the capability's explicit realtime endpoint against its connection
/// base. Provider-specific defaults are materialized by the capability
/// descriptor before the adapter is invoked.
fn build_realtime_url(call: &ResolvedRealtimeCall) -> Result<Url, InvokeError> {
    let base = call.connection.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(InvokeError::config("StepFun realtime base URL must not be empty"));
    }

    let endpoint = params_str(&call.model_params, "realtime_endpoint")
        .ok_or_else(|| InvokeError::config("StepFun realtime capability requires a realtime endpoint"))?;
    let raw = if endpoint.starts_with("http://")
        || endpoint.starts_with("https://")
        || endpoint.starts_with("ws://")
        || endpoint.starts_with("wss://")
    {
        endpoint.to_string()
    } else {
        format!("{base}/{}", endpoint.trim_start_matches('/'))
    };

    let mut url = Url::parse(&raw)
        .map_err(|error| InvokeError::config(format!("invalid StepFun realtime endpoint: {error}")))?;
    let ws_scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" => "ws",
        "wss" => "wss",
        other => {
            return Err(InvokeError::config(format!(
                "StepFun realtime endpoint uses unsupported URL scheme {other:?}"
            )));
        }
    };
    url.set_scheme(ws_scheme)
        .map_err(|_| InvokeError::config("could not set StepFun realtime WebSocket scheme"))?;

    // Placeholder expansion and query ownership belong to the protocol
    // descriptor/resolver. This adapter only canonicalizes the transport
    // scheme because WebSocket handshakes require ws(s).
    Ok(url)
}

fn params_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

async fn connect_with_key_rotation(
    call: &ResolvedRealtimeCall,
    url: &Url,
    limits: RealtimeSessionLimits,
) -> Result<(UpstreamSocket, Response), InvokeError> {
    let secrets = call.connection.auth.secrets();
    let mut last_error = None;

    for (index, secret) in secrets.iter().enumerate() {
        let request = websocket_request(url, secret)?;
        let socket_config = WebSocketConfig::default()
            .max_message_size(Some(limits.max_control_message_bytes))
            .max_frame_size(Some(limits.max_control_message_bytes));
        let attempt = tokio::time::timeout(
            limits.connect_timeout,
            connect_async_tls_with_config(request, Some(socket_config), false, None),
        )
        .await;

        match attempt {
            Ok(Ok(connected)) => return Ok(connected),
            Err(_) => {
                last_error = Some(InvokeError::new(
                    InvokeErrorKind::Timeout,
                    "StepFun realtime WebSocket handshake timed out",
                ));
            }
            Ok(Err(error)) => {
                let mapped = websocket_error(error);
                let may_rotate = matches!(
                    mapped.kind,
                    InvokeErrorKind::Auth | InvokeErrorKind::RateLimited | InvokeErrorKind::QuotaExhausted
                );
                last_error = Some(mapped);
                if !may_rotate || index + 1 == secrets.len() {
                    break;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| InvokeError::config("StepFun realtime connection has no usable API key")))
}

fn websocket_request(url: &Url, secret: &str) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, InvokeError> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| InvokeError::config(format!("invalid StepFun WebSocket request: {error}")))?;
    let mut authorization = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&format!(
        "Bearer {secret}"
    ))
    .map_err(|_| InvokeError::config("StepFun API key contains invalid HTTP header characters"))?;
    authorization.set_sensitive(true);
    request.headers_mut().insert(AUTHORIZATION, authorization);
    Ok(request)
}

fn websocket_error(error: tokio_tungstenite::tungstenite::Error) -> InvokeError {
    use tokio_tungstenite::tungstenite::Error;

    match error {
        Error::Http(response) => {
            let status = response.status().as_u16();
            let message = format!("StepFun realtime handshake rejected with HTTP {status}");
            let kind = match status {
                400 | 404 | 422 => InvokeErrorKind::InvalidParams,
                401 | 403 => InvokeErrorKind::Auth,
                408 | 504 => InvokeErrorKind::Timeout,
                429 => InvokeErrorKind::RateLimited,
                _ => InvokeErrorKind::ProviderError,
            };
            InvokeError::new(kind, message).with_http_status(status)
        }
        Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            InvokeError::new(InvokeErrorKind::Timeout, format!("StepFun realtime I/O timed out: {io}"))
        }
        other => InvokeError::new(
            InvokeErrorKind::Network,
            format!("StepFun realtime WebSocket connection failed: {other}"),
        ),
    }
}

async fn run_session(
    socket: UpstreamSocket,
    initial_config: RealtimeSessionConfig,
    limits: RealtimeSessionLimits,
    mut commands: mpsc::Receiver<RealtimeClientCommand>,
    events: mpsc::Sender<RealtimeServerEvent>,
) {
    let (mut writer, mut reader) = socket.split();
    let locked_voice = initial_config.voice.clone();
    let persistent_extra = initial_config.extra.clone();

    if let Err(message) = send_json(&mut writer, session_update(&initial_config), limits).await {
        let _ = emit(&events, RealtimeServerEvent::TransportError { message });
        return;
    }

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    let _ = writer.send(Message::Close(None)).await;
                    let _ = emit(&events, RealtimeServerEvent::Closed {
                        code: Some(1000),
                        reason: "local session handle dropped".to_string(),
                    });
                    return;
                };

                if matches!(command, RealtimeClientCommand::Close) {
                    let _ = writer.send(Message::Close(None)).await;
                    let _ = emit(&events, RealtimeServerEvent::Closed {
                        code: Some(1000),
                        reason: "client closed session".to_string(),
                    });
                    return;
                }

                let command = match command {
                    RealtimeClientCommand::UpdateSession(mut updated) => {
                        if updated.voice != locked_voice {
                            if !emit(&events, RealtimeServerEvent::TransportError {
                                message: "StepFun voice cannot change after session creation".to_string(),
                            }) {
                                let _ = writer.send(Message::Close(None)).await;
                                return;
                            }
                            continue;
                        }
                        if let Err(message) = merge_realtime_update_defaults(
                            &persistent_extra,
                            &mut updated,
                        ) {
                            if !emit(&events, RealtimeServerEvent::TransportError { message }) {
                                let _ = writer.send(Message::Close(None)).await;
                                return;
                            }
                            continue;
                        }
                        RealtimeClientCommand::UpdateSession(updated)
                    }
                    command => command,
                };

                match command_json(command, limits) {
                    Ok(value) => {
                        if let Err(message) = send_json(&mut writer, value, limits).await {
                            let _ = emit(&events, RealtimeServerEvent::TransportError { message });
                            let _ = writer.send(Message::Close(None)).await;
                            return;
                        }
                    }
                    Err(message) => {
                        if !emit(&events, RealtimeServerEvent::TransportError { message }) {
                            let _ = writer.send(Message::Close(None)).await;
                            return;
                        }
                    }
                }
            }
            incoming = reader.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if text.len() > limits.max_control_message_bytes {
                            let _ = emit(&events, RealtimeServerEvent::TransportError {
                                message: format!(
                                    "StepFun realtime event is {} bytes; maximum is {}",
                                    text.len(), limits.max_control_message_bytes
                                ),
                            });
                            let _ = writer.send(Message::Close(None)).await;
                            return;
                        }
                        match parse_server_event(text.as_ref(), limits.max_audio_frame_bytes) {
                            Ok(event) => {
                                if !emit(&events, event) {
                                    let _ = writer.send(Message::Close(None)).await;
                                    return;
                                }
                            }
                            Err(message) => {
                                if !emit(&events, RealtimeServerEvent::TransportError { message }) {
                                    let _ = writer.send(Message::Close(None)).await;
                                    return;
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if writer.send(Message::Pong(payload)).await.is_err() {
                            let _ = emit(&events, RealtimeServerEvent::TransportError {
                                message: "StepFun realtime pong write failed".to_string(),
                            });
                            return;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        let (code, reason) = frame
                            .map(|frame| (Some(u16::from(frame.code)), frame.reason.to_string()))
                            .unwrap_or((None, "provider closed session".to_string()));
                        let _ = emit(&events, RealtimeServerEvent::Closed { code, reason });
                        return;
                    }
                    Some(Ok(Message::Binary(_))) => {
                        if !emit(&events, RealtimeServerEvent::TransportError {
                            message: "StepFun realtime sent an unexpected binary frame".to_string(),
                        }) {
                            let _ = writer.send(Message::Close(None)).await;
                            return;
                        }
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        let _ = emit(&events, RealtimeServerEvent::TransportError {
                            message: format!("StepFun realtime socket read failed: {error}"),
                        });
                        return;
                    }
                    None => {
                        let _ = emit(&events, RealtimeServerEvent::Closed {
                            code: None,
                            reason: "StepFun realtime stream ended".to_string(),
                        });
                        return;
                    }
                }
            }
        }
    }
}

fn merge_realtime_update_defaults(
    persistent_extra: &Value,
    updated: &mut RealtimeSessionConfig,
) -> Result<(), String> {
    updated.extra = json_request_body(persistent_extra, &updated.extra, json!({}))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Never await an unbounded event consumer: a full bounded event queue closes
/// the provider socket, forcing the UI bridge to make an explicit recovery
/// decision rather than silently dropping transcript/audio ordering.
fn emit(events: &mpsc::Sender<RealtimeServerEvent>, event: RealtimeServerEvent) -> bool {
    events.try_send(event).is_ok()
}

async fn send_json(
    writer: &mut futures_util::stream::SplitSink<UpstreamSocket, Message>,
    value: Value,
    limits: RealtimeSessionLimits,
) -> Result<(), String> {
    let text = serde_json::to_string(&value)
        .map_err(|error| format!("could not encode StepFun realtime event: {error}"))?;
    if text.len() > limits.max_control_message_bytes {
        return Err(format!(
            "StepFun realtime control event is {} bytes; maximum is {}",
            text.len(), limits.max_control_message_bytes
        ));
    }
    writer
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| format!("StepFun realtime socket write failed: {error}"))
}

fn event_id() -> String {
    format!("nomifun_rt_{}", NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed))
}

fn base_event(kind: &str) -> Map<String, Value> {
    Map::from_iter([
        ("event_id".to_string(), Value::String(event_id())),
        ("type".to_string(), Value::String(kind.to_string())),
    ])
}

fn session_update(config: &RealtimeSessionConfig) -> Value {
    let mut session = config.extra.as_object().cloned().unwrap_or_default();
    session.insert("modalities".to_string(), json!(["text", "audio"]));
    session.insert("input_audio_format".to_string(), Value::String("pcm16".to_string()));
    session.insert("output_audio_format".to_string(), Value::String("pcm16".to_string()));
    insert_nonblank(&mut session, "instructions", config.instructions.as_deref());
    insert_nonblank(&mut session, "voice", config.voice.as_deref());
    if let Some(turn_detection) = &config.turn_detection {
        session.insert(
            "turn_detection".to_string(),
            serde_json::to_value(turn_detection).expect("serializable realtime turn detection"),
        );
    } else {
        session.remove("turn_detection");
    }
    if config.tools.is_empty() {
        session.remove("tools");
    } else {
        session.insert("tools".to_string(), Value::Array(config.tools.clone()));
    }

    let mut event = base_event("session.update");
    event.insert("session".to_string(), Value::Object(session));
    Value::Object(event)
}

fn command_json(command: RealtimeClientCommand, limits: RealtimeSessionLimits) -> Result<Value, String> {
    let kind = match &command {
        RealtimeClientCommand::AppendAudio(_) => "input_audio_buffer.append",
        RealtimeClientCommand::CommitAudio => "input_audio_buffer.commit",
        RealtimeClientCommand::ClearAudio => "input_audio_buffer.clear",
        RealtimeClientCommand::AddText { .. } => "conversation.item.create",
        RealtimeClientCommand::DeleteItem { .. } => "conversation.item.delete",
        RealtimeClientCommand::UpdateSession(_) => "session.update",
        RealtimeClientCommand::CreateResponse { .. } => "response.create",
        RealtimeClientCommand::CancelResponse => "response.cancel",
        RealtimeClientCommand::Close => return Err("Close is not a JSON command".to_string()),
    };

    match command {
        RealtimeClientCommand::AppendAudio(audio) => {
            if audio.len() > limits.max_audio_frame_bytes {
                return Err(format!(
                    "audio frame is {} bytes; maximum is {}",
                    audio.len(), limits.max_audio_frame_bytes
                ));
            }
            let mut event = base_event(kind);
            event.insert(
                "audio".to_string(),
                Value::String(base64::engine::general_purpose::STANDARD.encode(audio)),
            );
            Ok(Value::Object(event))
        }
        RealtimeClientCommand::CommitAudio
        | RealtimeClientCommand::ClearAudio
        | RealtimeClientCommand::CancelResponse => Ok(Value::Object(base_event(kind))),
        RealtimeClientCommand::AddText { text, previous_item_id } => {
            if text.trim().is_empty() {
                return Err("StepFun realtime text item must not be empty".to_string());
            }
            let mut event = base_event(kind);
            if let Some(previous_item_id) = previous_item_id.filter(|value| !value.trim().is_empty()) {
                event.insert("previous_item_id".to_string(), Value::String(previous_item_id));
            }
            event.insert(
                "item".to_string(),
                json!({
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}],
                }),
            );
            Ok(Value::Object(event))
        }
        RealtimeClientCommand::DeleteItem { item_id } => {
            if item_id.trim().is_empty() {
                return Err("StepFun realtime item_id must not be empty".to_string());
            }
            let mut event = base_event(kind);
            event.insert("item_id".to_string(), Value::String(item_id));
            Ok(Value::Object(event))
        }
        RealtimeClientCommand::UpdateSession(config) => Ok(session_update(&config)),
        RealtimeClientCommand::CreateResponse { extra } => {
            let mut event = extra.as_object().cloned().unwrap_or_default();
            event.insert("event_id".to_string(), Value::String(event_id()));
            event.insert("type".to_string(), Value::String(kind.to_string()));
            Ok(Value::Object(event))
        }
        RealtimeClientCommand::Close => unreachable!("handled above"),
    }
}

fn insert_nonblank(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        map.insert(key.to_string(), Value::String(value.to_string()));
    } else {
        map.remove(key);
    }
}

fn parse_server_event(text: &str, max_audio_frame_bytes: usize) -> Result<RealtimeServerEvent, String> {
    let raw: Value = serde_json::from_str(text)
        .map_err(|error| format!("invalid StepFun realtime JSON event: {error}"))?;
    let kind = raw
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "StepFun realtime event is missing string type".to_string())?;
    let event_id = string_at(&raw, &["event_id"]);
    let response_id = string_at(&raw, &["response_id"])
        .or_else(|| string_at(&raw, &["response", "id"]));
    let item_id = string_at(&raw, &["item_id"])
        .or_else(|| string_at(&raw, &["item", "id"]));

    let event = match kind {
        "session.created" => RealtimeServerEvent::SessionCreated {
            event_id,
            session_id: string_at(&raw, &["session", "id"]),
            model: string_at(&raw, &["session", "model"]),
            voice: string_at(&raw, &["session", "voice"]),
        },
        "session.updated" => RealtimeServerEvent::SessionUpdated {
            event_id,
            session_id: string_at(&raw, &["session", "id"]),
            voice: string_at(&raw, &["session", "voice"]),
        },
        "input_audio_buffer.speech_started" => RealtimeServerEvent::SpeechStarted {
            event_id,
            item_id,
            audio_start_ms: u64_at(&raw, &["audio_start_ms"]),
        },
        "input_audio_buffer.speech_stopped" => RealtimeServerEvent::SpeechStopped {
            event_id,
            item_id,
            audio_end_ms: u64_at(&raw, &["audio_end_ms"]),
        },
        "input_audio_buffer.committed" => {
            RealtimeServerEvent::InputAudioCommitted { event_id, item_id }
        }
        "input_audio_buffer.cleared" => RealtimeServerEvent::InputAudioCleared { event_id },
        "conversation.item.created" => RealtimeServerEvent::ConversationItemCreated {
            event_id,
            item: raw.get("item").cloned().unwrap_or(Value::Null),
        },
        "conversation.item.deleted" => {
            RealtimeServerEvent::ConversationItemDeleted { event_id, item_id }
        }
        "response.created" => RealtimeServerEvent::ResponseCreated { event_id, response_id },
        "response.audio.delta" => {
            let encoded = raw
                .get("delta")
                .and_then(Value::as_str)
                .ok_or_else(|| "StepFun response.audio.delta is missing delta".to_string())?;
            let audio = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| format!("invalid StepFun response.audio.delta base64: {error}"))?;
            if audio.len() > max_audio_frame_bytes {
                return Err(format!(
                    "StepFun decoded audio delta is {} bytes; maximum is {}",
                    audio.len(), max_audio_frame_bytes
                ));
            }
            RealtimeServerEvent::AudioDelta { event_id, response_id, item_id, audio }
        }
        "response.audio.done" => RealtimeServerEvent::AudioDone { event_id, response_id, item_id },
        "response.audio_transcript.delta" => RealtimeServerEvent::AssistantTranscriptDelta {
            event_id,
            response_id,
            item_id,
            delta: string_at(&raw, &["delta"]).unwrap_or_default(),
        },
        "response.audio_transcript.done" => RealtimeServerEvent::AssistantTranscriptDone {
            event_id,
            response_id,
            item_id,
            transcript: string_at(&raw, &["transcript"]).unwrap_or_default(),
        },
        "conversation.item.input_audio_transcription.completed" => {
            RealtimeServerEvent::InputTranscriptDone {
                event_id,
                item_id,
                transcript: string_at(&raw, &["transcript"]).unwrap_or_default(),
            }
        }
        "response.thinking.delta" => RealtimeServerEvent::ThinkingDelta {
            event_id,
            response_id,
            item_id,
            delta: string_at(&raw, &["delta"]).unwrap_or_default(),
        },
        "response.thinking.done" => RealtimeServerEvent::ThinkingDone {
            event_id,
            response_id,
            item_id,
            thinking: string_at(&raw, &["thinking"]).unwrap_or_default(),
        },
        "response.text.delta" => RealtimeServerEvent::TextDelta {
            event_id,
            response_id,
            item_id,
            delta: string_at(&raw, &["delta"]).unwrap_or_default(),
        },
        "response.text.done" => RealtimeServerEvent::TextDone {
            event_id,
            response_id,
            item_id,
            text: string_at(&raw, &["text"]).unwrap_or_default(),
        },
        "response.done" => RealtimeServerEvent::ResponseDone {
            event_id,
            response_id,
            status: string_at(&raw, &["response", "status"])
                .or_else(|| string_at(&raw, &["status"])),
            response: raw.get("response").cloned().unwrap_or(Value::Null),
        },
        "response.cancelled" => {
            RealtimeServerEvent::ResponseCancelled { event_id, response_id }
        }
        "error" => RealtimeServerEvent::ProviderError {
            event_id,
            error_type: string_at(&raw, &["error", "type"]),
            code: string_at(&raw, &["error", "code"]),
            message: string_at(&raw, &["error", "message"])
                .unwrap_or_else(|| "StepFun realtime provider error".to_string()),
            caused_by_event_id: string_at(&raw, &["error", "event_id"]),
        },
        _ => RealtimeServerEvent::Unknown { event_type: kind.to_string(), raw },
    };
    Ok(event)
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(*key))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    value_at(value, path).and_then(Value::as_str).map(str::to_string)
}

fn u64_at(value: &Value, path: &[&str]) -> Option<u64> {
    let value = value_at(value, path)?;
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

    use super::*;
    use crate::auth::AuthMaterial;
    use crate::call::ResolvedConnection;

    fn call(base_url: &str, model: &str) -> ResolvedRealtimeCall {
        let encoded_model = url::form_urlencoded::byte_serialize(model.as_bytes()).collect::<String>();
        ResolvedRealtimeCall {
            provider_id: "018f0000-0000-7000-8000-0000000000ee".to_string(),
            platform: "stepfun".to_string(),
            model: model.to_string(),
            protocol: PROTOCOL_ID.to_string(),
            connection: ResolvedConnection {
                role: "default".to_string(),
                base_url: base_url.to_string(),
                auth: AuthMaterial {
                    scheme: AuthScheme::Bearer,
                    credentials: json!({"api_keys": ["step-secret"]}),
                },
                extra: json!({}),
            },
            model_params: json!({"realtime_endpoint": format!("/realtime?model={encoded_model}")}),
        }
    }

    #[test]
    fn normal_and_plan_urls_preserve_their_version_roots() {
        let normal = call("https://api.stepfun.com/v1", "stepaudio-2.5-realtime");
        assert_eq!(
            build_realtime_url(&normal).unwrap().as_str(),
            "wss://api.stepfun.com/v1/realtime?model=stepaudio-2.5-realtime"
        );

        let plan = call("https://api.stepfun.com/step_plan/v1", "stepaudio-2.5-realtime");
        assert_eq!(
            build_realtime_url(&plan).unwrap().as_str(),
            "wss://api.stepfun.com/step_plan/v1/realtime?model=stepaudio-2.5-realtime"
        );
    }

    #[test]
    fn missing_capability_endpoint_is_rejected() {
        let mut call = call("https://api.stepfun.com/v1", "m");
        call.model_params = json!({});
        let error = build_realtime_url(&call).unwrap_err();
        assert!(error.to_string().contains("requires a realtime endpoint"));
    }

    #[test]
    fn endpoint_override_preserves_resolver_expanded_query() {
        let mut call = call("https://gateway.example/root", "new model");
        call.model_params = json!({"realtime_endpoint": "https://gateway.example/socket?tenant=a&model=new%20model"});
        let url = build_realtime_url(&call).unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/socket");
        assert_eq!(url.query_pairs().filter(|(key, _)| key == "model").count(), 1);
        assert!(url.query_pairs().any(|(key, value)| key == "tenant" && value == "a"));
        assert!(url.query_pairs().any(|(key, value)| key == "model" && value == "new model"));
    }

    #[test]
    fn capability_provider_params_seed_session_and_typed_config_wins() {
        let mut call = call("https://api.stepfun.com/v1", "stepaudio-2.5-realtime");
        call.model_params["temperature"] = json!(0.35);
        call.model_params["provider_options"] = json!({
            "threshold": 0.2,
            "provider_future_field": true
        });
        call.model_params["api_key"] = json!("must-not-leak");

        let config = provider_session_config(
            &call,
            RealtimeSessionConfig {
                voice: Some("cixingnansheng".into()),
                extra: json!({
                    "temperature": 0.6,
                    "provider_options": {"threshold": 0.5}
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let event = session_update(&config);
        assert_eq!(event["session"]["temperature"], 0.6);
        assert_eq!(event["session"]["voice"], "cixingnansheng");
        assert_eq!(event["session"]["provider_options"]["threshold"], 0.5);
        assert_eq!(event["session"]["provider_options"]["provider_future_field"], true);
        assert!(event["session"].get("api_key").is_none());
        assert_eq!(event["session"]["modalities"], json!(["text", "audio"]));

        let mut updated = RealtimeSessionConfig {
            voice: Some("cixingnansheng".into()),
            extra: json!({
                "temperature": 0.8,
                "provider_options": {"new_option": "enabled"}
            }),
            ..Default::default()
        };
        merge_realtime_update_defaults(&config.extra, &mut updated).unwrap();
        let event = session_update(&updated);
        assert_eq!(event["session"]["temperature"], 0.8);
        assert_eq!(event["session"]["provider_options"]["threshold"], 0.5);
        assert_eq!(event["session"]["provider_options"]["provider_future_field"], true);
        assert_eq!(event["session"]["provider_options"]["new_option"], "enabled");
    }

    #[test]
    fn event_parser_decodes_audio_and_maps_lifecycle() {
        let event = parse_server_event(
            &json!({
                "event_id": "e1",
                "type": "response.audio.delta",
                "response_id": "r1",
                "item_id": "i1",
                "delta": base64::engine::general_purpose::STANDARD.encode([1, 2, 3]),
            })
            .to_string(),
            10,
        )
        .unwrap();
        assert_eq!(
            event,
            RealtimeServerEvent::AudioDelta {
                event_id: Some("e1".to_string()),
                response_id: Some("r1".to_string()),
                item_id: Some("i1".to_string()),
                audio: vec![1, 2, 3],
            }
        );

        let done = parse_server_event(
            r#"{"type":"response.done","response":{"id":"r2","status":"completed"}}"#,
            10,
        )
        .unwrap();
        assert!(matches!(
            done,
            RealtimeServerEvent::ResponseDone {
                response_id: Some(ref id),
                status: Some(ref status),
                ..
            } if id == "r2" && status == "completed"
        ));
    }

    #[test]
    fn event_parser_enforces_decoded_audio_limit() {
        let encoded = base64::engine::general_purpose::STANDARD.encode([0; 5]);
        let error = parse_server_event(
            &json!({"type": "response.audio.delta", "delta": encoded}).to_string(),
            4,
        )
        .unwrap_err();
        assert!(error.contains("maximum is 4"));
    }

    #[tokio::test]
    async fn mock_websocket_covers_auth_commands_and_streamed_events() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handshake = Arc::new(Mutex::new(None::<(String, String)>));
        let captured = Arc::clone(&handshake);

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_hdr_async(stream, move |request: &Request, response: Response| {
                let auth = request
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                *captured.lock().unwrap() = Some((request.uri().to_string(), auth));
                Ok(response)
            })
            .await
            .unwrap();

            socket
                .send(Message::Text(
                    json!({
                        "event_id": "created-1",
                        "type": "session.created",
                        "session": {"id": "sess-1", "model": "stepaudio-2.5-realtime", "voice": "voice-a"}
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            let mut client_events = Vec::new();
            while client_events.len() < 7 {
                match socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => {
                        client_events.push(serde_json::from_str::<Value>(text.as_ref()).unwrap());
                    }
                    other => panic!("unexpected client frame: {other:?}"),
                }
            }

            for event in [
                json!({"type": "session.updated", "session": {"id": "sess-1", "voice": "voice-a"}}),
                json!({
                    "type": "response.audio.delta",
                    "response_id": "resp-1",
                    "item_id": "item-1",
                    "delta": base64::engine::general_purpose::STANDARD.encode([7, 8])
                }),
                json!({
                    "type": "response.audio_transcript.delta",
                    "response_id": "resp-1",
                    "item_id": "item-1",
                    "delta": "你"
                }),
                json!({
                    "type": "response.thinking.delta",
                    "response_id": "resp-1",
                    "item_id": "item-1",
                    "delta": "思考"
                }),
                json!({
                    "type": "error",
                    "error": {"type": "invalid_request_error", "code": "bad", "message": "example"}
                }),
                json!({"type": "response.done", "response": {"id": "resp-1", "status": "completed"}}),
            ] {
                socket.send(Message::Text(event.to_string().into())).await.unwrap();
            }

            let close = socket.next().await;
            (client_events, close)
        });

        let call = call(&format!("http://{address}/v1"), "stepaudio-2.5-realtime");
        let adapter = StepFunRealtimeAdapter::new();
        let mut session = adapter
            .open(
                &call,
                RealtimeSessionConfig {
                    instructions: Some("be concise".to_string()),
                    voice: Some("voice-a".to_string()),
                    turn_detection: Some(RealtimeTurnDetection::default()),
                    tools: vec![],
                    extra: json!({}),
                },
            )
            .await
            .unwrap();

        assert!(matches!(
            session.recv().await,
            Some(RealtimeServerEvent::SessionCreated { session_id: Some(ref id), .. }) if id == "sess-1"
        ));

        session.send(RealtimeClientCommand::AppendAudio(vec![1, 2, 3])).await.unwrap();
        session.send(RealtimeClientCommand::CommitAudio).await.unwrap();
        session.send(RealtimeClientCommand::ClearAudio).await.unwrap();
        session
            .send(RealtimeClientCommand::AddText {
                text: "hello".to_string(),
                previous_item_id: None,
            })
            .await
            .unwrap();
        session
            .send(RealtimeClientCommand::CreateResponse { extra: json!({}) })
            .await
            .unwrap();
        session.send(RealtimeClientCommand::CancelResponse).await.unwrap();

        let mut saw_audio = false;
        let mut saw_transcript = false;
        let mut saw_thinking = false;
        let mut saw_error = false;
        loop {
            match session.recv().await.unwrap() {
                RealtimeServerEvent::AudioDelta { audio, .. } => saw_audio = audio == [7, 8],
                RealtimeServerEvent::AssistantTranscriptDelta { delta, .. } => {
                    saw_transcript = delta == "你"
                }
                RealtimeServerEvent::ThinkingDelta { delta, .. } => saw_thinking = delta == "思考",
                RealtimeServerEvent::ProviderError { code, .. } => {
                    saw_error = code.as_deref() == Some("bad")
                }
                RealtimeServerEvent::ResponseDone { .. } => break,
                _ => {}
            }
        }
        assert!(saw_audio && saw_transcript && saw_thinking && saw_error);
        session.close().await.unwrap();

        let (events, close) = server.await.unwrap();
        let kinds: Vec<&str> = events.iter().filter_map(|event| event["type"].as_str()).collect();
        assert_eq!(
            kinds,
            [
                "session.update",
                "input_audio_buffer.append",
                "input_audio_buffer.commit",
                "input_audio_buffer.clear",
                "conversation.item.create",
                "response.create",
                "response.cancel",
            ]
        );
        assert_eq!(events[0]["session"]["input_audio_format"], "pcm16");
        assert_eq!(events[0]["session"]["output_audio_format"], "pcm16");
        assert_eq!(
            events[1]["audio"],
            base64::engine::general_purpose::STANDARD.encode([1, 2, 3])
        );
        assert!(matches!(close, Some(Ok(Message::Close(_))) | None));

        let (uri, auth) = handshake.lock().unwrap().clone().unwrap();
        assert_eq!(uri, "/v1/realtime?model=stepaudio-2.5-realtime");
        assert_eq!(auth, "Bearer step-secret");
    }
}
