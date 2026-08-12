//! Session-oriented model invocation primitives.
//!
//! Unlike [`crate::adapter::ProtocolAdapter`], which represents one request
//! followed by either a terminal result or a persisted polling handle, a
//! realtime adapter owns a live, process-local, bidirectional session.  The
//! bounded command/event channels in this module are deliberately not
//! serializable: a socket must never be disguised as a [`crate::types::JobHandle`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::call::ResolvedConnection;
use crate::error::InvokeError;

/// Conservative local limit for one raw PCM command or decoded audio delta.
///
/// At mono PCM16/24 kHz this is more than one second of audio. Callers should
/// normally submit 20-30 ms chunks; rejecting unexpectedly large frames keeps
/// the browser/backend bridge from turning a realtime connection into an
/// unbounded file-upload channel.
pub const DEFAULT_MAX_AUDIO_FRAME_BYTES: usize = 64 * 1024;

/// Maximum JSON control/event frame accepted from either side.
pub const DEFAULT_MAX_CONTROL_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

/// Default bounded-channel sizes. Backpressure is intentional: callers may
/// await [`RealtimeSession::send`] or handle [`RealtimeSendError::Backpressure`]
/// from [`RealtimeSession::try_send`].
pub const DEFAULT_COMMAND_CHANNEL_CAPACITY: usize = 32;
pub const DEFAULT_EVENT_CHANNEL_CAPACITY: usize = 64;

/// Upper bounds and queue sizes for one realtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeSessionLimits {
    pub max_audio_frame_bytes: usize,
    pub max_control_message_bytes: usize,
    pub command_channel_capacity: usize,
    pub event_channel_capacity: usize,
    pub connect_timeout: Duration,
    pub close_timeout: Duration,
}

impl Default for RealtimeSessionLimits {
    fn default() -> Self {
        Self {
            max_audio_frame_bytes: DEFAULT_MAX_AUDIO_FRAME_BYTES,
            max_control_message_bytes: DEFAULT_MAX_CONTROL_MESSAGE_BYTES,
            command_channel_capacity: DEFAULT_COMMAND_CHANNEL_CAPACITY,
            event_channel_capacity: DEFAULT_EVENT_CHANNEL_CAPACITY,
            connect_timeout: Duration::from_secs(20),
            close_timeout: Duration::from_secs(5),
        }
    }
}

impl RealtimeSessionLimits {
    /// Validate limits before allocating channels or configuring tungstenite.
    pub fn validate(&self) -> Result<(), InvokeError> {
        if self.max_audio_frame_bytes == 0 {
            return Err(InvokeError::config("realtime max_audio_frame_bytes must be positive"));
        }
        if self.max_control_message_bytes == 0 {
            return Err(InvokeError::config(
                "realtime max_control_message_bytes must be positive",
            ));
        }
        if self.command_channel_capacity == 0 || self.event_channel_capacity == 0 {
            return Err(InvokeError::config(
                "realtime command/event channel capacities must be positive",
            ));
        }
        if self.connect_timeout.is_zero() || self.close_timeout.is_zero() {
            return Err(InvokeError::config("realtime connect/close timeouts must be positive"));
        }
        Ok(())
    }
}

/// Provider/model/connection data needed to establish a realtime call.
///
/// This intentionally does not embed [`crate::types::TaskRequest`]. The
/// catalog resolver can construct it after enforcing the dedicated realtime
/// model task, while the live session stays independent of the one-shot invoke
/// union.
#[derive(Clone)]
pub struct ResolvedRealtimeCall {
    pub provider_id: String,
    pub platform: String,
    pub model: String,
    pub protocol: String,
    pub connection: ResolvedConnection,
    /// Ephemeral adapter view built from provider parameters plus typed
    /// capability transport fields.
    pub model_params: Value,
}

/// StepFun-compatible server VAD settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeTurnDetection {
    #[serde(default = "server_vad_kind")]
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_padding_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_duration_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_awakeness_threshold: Option<u32>,
}

fn server_vad_kind() -> String {
    "server_vad".to_string()
}

impl Default for RealtimeTurnDetection {
    fn default() -> Self {
        Self {
            r#type: server_vad_kind(),
            prefix_padding_ms: None,
            silence_duration_ms: None,
            energy_awakeness_threshold: None,
        }
    }
}

/// Session defaults sent through the provider's `session.update` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RealtimeSessionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// `None` leaves server VAD disabled; callers then explicitly commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_detection: Option<RealtimeTurnDetection>,
    /// Provider-native tool declarations. Tool execution remains the caller's
    /// responsibility and is not silently bridged to MCP.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    /// Forward-compatible session fields. Protocol-owned fields such as
    /// `modalities` and audio formats override conflicting entries.
    #[serde(default)]
    pub extra: Value,
}

/// Commands accepted by a live realtime model session.
#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeClientCommand {
    /// Raw little-endian mono PCM16 audio. The provider adapter performs any
    /// required base64 envelope encoding.
    AppendAudio(Vec<u8>),
    CommitAudio,
    ClearAudio,
    /// Add a text message to the provider-managed conversation.
    AddText { text: String, previous_item_id: Option<String> },
    DeleteItem { item_id: String },
    /// Update mutable session fields. Providers may reject a voice change once
    /// the initial session has been created.
    UpdateSession(RealtimeSessionConfig),
    CreateResponse { extra: Value },
    CancelResponse,
    Close,
}

/// Provider-independent events emitted by a realtime session.
#[derive(Debug, Clone, PartialEq)]
pub enum RealtimeServerEvent {
    SessionCreated {
        event_id: Option<String>,
        session_id: Option<String>,
        model: Option<String>,
        voice: Option<String>,
    },
    SessionUpdated {
        event_id: Option<String>,
        session_id: Option<String>,
        voice: Option<String>,
    },
    SpeechStarted {
        event_id: Option<String>,
        item_id: Option<String>,
        audio_start_ms: Option<u64>,
    },
    SpeechStopped {
        event_id: Option<String>,
        item_id: Option<String>,
        audio_end_ms: Option<u64>,
    },
    InputAudioCommitted { event_id: Option<String>, item_id: Option<String> },
    InputAudioCleared { event_id: Option<String> },
    ConversationItemCreated { event_id: Option<String>, item: Value },
    ConversationItemDeleted { event_id: Option<String>, item_id: Option<String> },
    ResponseCreated { event_id: Option<String>, response_id: Option<String> },
    AudioDelta {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        audio: Vec<u8>,
    },
    AudioDone {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
    },
    AssistantTranscriptDelta {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        delta: String,
    },
    AssistantTranscriptDone {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        transcript: String,
    },
    InputTranscriptDone {
        event_id: Option<String>,
        item_id: Option<String>,
        transcript: String,
    },
    ThinkingDelta {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        delta: String,
    },
    ThinkingDone {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        thinking: String,
    },
    TextDelta {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        delta: String,
    },
    TextDone {
        event_id: Option<String>,
        response_id: Option<String>,
        item_id: Option<String>,
        text: String,
    },
    ResponseDone {
        event_id: Option<String>,
        response_id: Option<String>,
        status: Option<String>,
        response: Value,
    },
    ResponseCancelled { event_id: Option<String>, response_id: Option<String> },
    ProviderError {
        event_id: Option<String>,
        error_type: Option<String>,
        code: Option<String>,
        message: String,
        caused_by_event_id: Option<String>,
    },
    /// A malformed/oversized provider event or socket failure. The provider API
    /// key is never included in this message.
    TransportError { message: String },
    /// Preserve new provider event kinds without teaching callers that their
    /// shape is stable.
    Unknown { event_type: String, raw: Value },
    Closed { code: Option<u16>, reason: String },
}

/// Local queue/lifecycle error when driving a session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RealtimeSendError {
    #[error("realtime session is closed")]
    Closed,
    #[error("realtime command queue is full")]
    Backpressure,
    #[error("audio frame is {actual} bytes; maximum is {maximum}")]
    AudioFrameTooLarge { actual: usize, maximum: usize },
    #[error("realtime worker failed: {0}")]
    Worker(String),
}

/// Process-local handle for one live provider socket.
pub struct RealtimeSession {
    command_tx: mpsc::Sender<RealtimeClientCommand>,
    event_rx: mpsc::Receiver<RealtimeServerEvent>,
    worker: Option<JoinHandle<()>>,
    limits: RealtimeSessionLimits,
}

impl RealtimeSession {
    /// Constructed by realtime adapters after the upstream handshake succeeds.
    pub(crate) fn from_parts(
        command_tx: mpsc::Sender<RealtimeClientCommand>,
        event_rx: mpsc::Receiver<RealtimeServerEvent>,
        worker: JoinHandle<()>,
        limits: RealtimeSessionLimits,
    ) -> Self {
        Self { command_tx, event_rx, worker: Some(worker), limits }
    }

    /// Await command-queue capacity. This is the normal audio streaming path.
    pub async fn send(&self, command: RealtimeClientCommand) -> Result<(), RealtimeSendError> {
        self.validate_command(&command)?;
        self.command_tx.send(command).await.map_err(|_| RealtimeSendError::Closed)
    }

    /// Non-blocking command submission for UI bridges that implement their own
    /// backpressure policy.
    pub fn try_send(&self, command: RealtimeClientCommand) -> Result<(), RealtimeSendError> {
        self.validate_command(&command)?;
        self.command_tx.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => RealtimeSendError::Backpressure,
            mpsc::error::TrySendError::Closed(_) => RealtimeSendError::Closed,
        })
    }

    /// Receive the next normalized provider event. `None` means the worker and
    /// event channel are both gone.
    pub async fn recv(&mut self) -> Option<RealtimeServerEvent> {
        self.event_rx.recv().await
    }

    pub fn limits(&self) -> RealtimeSessionLimits {
        self.limits
    }

    /// Gracefully request a WebSocket close and wait a bounded amount of time
    /// for the worker. Dropping a session also queues `Close`, but cannot await
    /// the peer's close handshake.
    pub async fn close(mut self) -> Result<(), RealtimeSendError> {
        // Do not await queue capacity here: if the worker is wedged in an
        // upstream write and the command queue is full, close itself must still
        // enter the bounded worker timeout below.
        let _ = self.command_tx.try_send(RealtimeClientCommand::Close);
        let Some(mut worker) = self.worker.take() else {
            return Ok(());
        };
        match tokio::time::timeout(self.limits.close_timeout, &mut worker).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(RealtimeSendError::Worker(error.to_string())),
            Err(_) => {
                worker.abort();
                Err(RealtimeSendError::Worker("close handshake timed out".to_string()))
            }
        }
    }

    fn validate_command(&self, command: &RealtimeClientCommand) -> Result<(), RealtimeSendError> {
        if let RealtimeClientCommand::AppendAudio(audio) = command
            && audio.len() > self.limits.max_audio_frame_bytes
        {
            return Err(RealtimeSendError::AudioFrameTooLarge {
                actual: audio.len(),
                maximum: self.limits.max_audio_frame_bytes,
            });
        }
        Ok(())
    }
}

impl Drop for RealtimeSession {
    fn drop(&mut self) {
        // Best effort only. Once this handle (the sole sender) is dropped, a
        // full queue eventually drains and then closes, which also makes the
        // adapter worker close its socket.
        let _ = self.command_tx.try_send(RealtimeClientCommand::Close);
    }
}

/// Parallel adapter seam for live model sessions.
#[async_trait]
pub trait RealtimeProtocolAdapter: Send + Sync {
    fn id(&self) -> &'static str;

    async fn open(
        &self,
        call: &ResolvedRealtimeCall,
        config: RealtimeSessionConfig,
    ) -> Result<RealtimeSession, InvokeError>;
}

/// Immutable protocol-to-session-adapter table.
///
/// Realtime transports intentionally use their own registry: accepting a
/// protocol here never makes it eligible for the one-shot HTTP dispatcher,
/// and vice versa.
pub struct RealtimeAdapterRegistry {
    map: BTreeMap<&'static str, Arc<dyn RealtimeProtocolAdapter>>,
}

impl RealtimeAdapterRegistry {
    pub fn new(adapters: Vec<Arc<dyn RealtimeProtocolAdapter>>) -> Self {
        Self::try_new(adapters)
            .unwrap_or_else(|error| panic!("invalid realtime adapter registry: {error}"))
    }

    pub fn try_new(adapters: Vec<Arc<dyn RealtimeProtocolAdapter>>) -> Result<Self, InvokeError> {
        let mut map = BTreeMap::new();
        for adapter in adapters {
            let id = adapter.id();
            if map.insert(id, adapter).is_some() {
                return Err(InvokeError::config(format!(
                    "duplicate realtime protocol adapter id {id:?}"
                )));
            }
        }
        Ok(Self { map })
    }

    pub fn get(&self, protocol: &str) -> Result<Arc<dyn RealtimeProtocolAdapter>, InvokeError> {
        self.map.get(protocol).cloned().ok_or_else(|| {
            InvokeError::new(
                crate::error::InvokeErrorKind::NoAdapter,
                format!("no realtime adapter registered for protocol {protocol:?}"),
            )
        })
    }

    pub fn contains(&self, protocol: &str) -> bool {
        self.map.contains_key(protocol)
    }

    pub fn protocol_ids(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.map.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRealtimeAdapter;

    #[async_trait]
    impl RealtimeProtocolAdapter for FakeRealtimeAdapter {
        fn id(&self) -> &'static str {
            "fake.realtime"
        }

        async fn open(
            &self,
            _call: &ResolvedRealtimeCall,
            _config: RealtimeSessionConfig,
        ) -> Result<RealtimeSession, InvokeError> {
            Err(InvokeError::config("not used by registry test"))
        }
    }

    #[test]
    fn realtime_registry_is_strict_and_separate_from_http_adapters() {
        let registry = RealtimeAdapterRegistry::new(vec![Arc::new(FakeRealtimeAdapter)]);
        assert_eq!(registry.get("fake.realtime").unwrap().id(), "fake.realtime");
        assert!(registry.contains("fake.realtime"));
        let error = match registry.get("openai.chat_text") {
            Ok(_) => panic!("unexpected realtime adapter"),
            Err(error) => error,
        };
        assert_eq!(error.kind, crate::error::InvokeErrorKind::NoAdapter);
    }

    #[test]
    fn realtime_registry_rejects_duplicate_ids_and_is_enumerable() {
        let error = RealtimeAdapterRegistry::try_new(vec![
            Arc::new(FakeRealtimeAdapter),
            Arc::new(FakeRealtimeAdapter),
        ])
        .err()
        .expect("duplicate must fail registry construction");
        assert_eq!(error.kind, crate::error::InvokeErrorKind::Config);
        assert!(error.message.contains("fake.realtime"));

        let registry = RealtimeAdapterRegistry::new(vec![Arc::new(FakeRealtimeAdapter)]);
        assert_eq!(registry.protocol_ids().collect::<Vec<_>>(), vec!["fake.realtime"]);
        assert_eq!(registry.len(), 1);
    }

    fn detached_session(
        limits: RealtimeSessionLimits,
    ) -> (RealtimeSession, mpsc::Receiver<RealtimeClientCommand>) {
        let (command_tx, command_rx) = mpsc::channel(limits.command_channel_capacity);
        let (_event_tx, event_rx) = mpsc::channel(limits.event_channel_capacity);
        let worker = tokio::spawn(async {});
        (RealtimeSession::from_parts(command_tx, event_rx, worker, limits), command_rx)
    }

    #[tokio::test]
    async fn bounded_command_queue_reports_backpressure() {
        let limits = RealtimeSessionLimits { command_channel_capacity: 1, ..Default::default() };
        let (session, _commands) = detached_session(limits);

        session.try_send(RealtimeClientCommand::CommitAudio).unwrap();
        assert_eq!(
            session.try_send(RealtimeClientCommand::ClearAudio),
            Err(RealtimeSendError::Backpressure)
        );
    }

    #[tokio::test]
    async fn oversized_audio_is_rejected_before_entering_queue() {
        let limits = RealtimeSessionLimits { max_audio_frame_bytes: 3, ..Default::default() };
        let (session, mut commands) = detached_session(limits);

        assert_eq!(
            session.try_send(RealtimeClientCommand::AppendAudio(vec![0; 4])),
            Err(RealtimeSendError::AudioFrameTooLarge { actual: 4, maximum: 3 })
        );
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn drop_requests_a_graceful_close() {
        let limits = RealtimeSessionLimits::default();
        let (session, mut commands) = detached_session(limits);
        drop(session);
        assert_eq!(commands.recv().await, Some(RealtimeClientCommand::Close));
    }

    #[test]
    fn invalid_limits_fail_closed() {
        let limits = RealtimeSessionLimits { event_channel_capacity: 0, ..Default::default() };
        assert!(limits.validate().is_err());
    }
}
