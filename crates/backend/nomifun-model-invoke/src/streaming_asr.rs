//! Dedicated, tool-free live transcription. Audio leaves while recording;
//! neither file upload nor a conversational voice session is used here.
//!
//! Wire contracts checked 2026-10-19:
//! https://developers.deepgram.com/reference/speech-to-text/listen-streaming
//! https://developers.deepgram.com/docs/close-stream
//! https://developers.openai.com/api/docs/guides/realtime-transcription
//! https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream.md

mod audio;
#[cfg(test)]
mod tests;

use crate::{
    AuthScheme, InvokeError, InvokeErrorKind, ModelInvokeService, ModelRef, ResolvedTaskConfig,
};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::ModelTask;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::{
    Message, client::IntoClientRequest, protocol::WebSocketConfig,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_tls_with_config};
use tokio_util::sync::CancellationToken;

const MAX_TEXT: usize = 1024 * 1024;
const MAX_WIRE: usize = 2 * 1024 * 1024;
const AUDIO_QUEUE: usize = 4; // Four 60 ms PCM chunks, never a recording-sized backlog.
const MAX_AUDIO_AGE: Duration = Duration::from_millis(250);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const FINAL_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RECORDING: Duration = Duration::from_secs(600);

pub fn supports_protocol(protocol: &str) -> bool {
    matches!(
        protocol,
        "deepgram.listen_stream" | "openai.realtime_transcription" | "stepfun.asr_stream"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wire {
    Deepgram,
    OpenAi,
    StepFun,
}
fn wire(protocol: &str) -> Wire {
    match protocol {
        "deepgram.listen_stream" => Wire::Deepgram,
        "stepfun.asr_stream" => Wire::StepFun,
        _ => Wire::OpenAi,
    }
}
fn event_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "nomifun-asr-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn audio_message(bytes: &[u8], stepfun: bool) -> Message {
    let mut value = json!({"type":"input_audio_buffer.append","audio":base64::engine::general_purpose::STANDARD.encode(bytes)});
    if stepfun {
        value["event_id"] = json!(event_id());
    }
    Message::Text(value.to_string().into())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamingAsrConfig {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub dictionary: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingAsrEvent {
    /// Full provisional text for this recording; replace the previous partial.
    Partial {
        text: String,
    },
    /// Stable provider segment. Repeated identical finals are suppressed.
    Final {
        segment_id: String,
        text: String,
    },
    /// All final segments in source order. Append this once to the document.
    Finished {
        text: String,
    },
    Error {
        message: String,
    },
    Cancelled,
}

enum Command {
    Pcm16 { samples: Vec<i16>, at: Instant },
}

#[derive(Default)]
struct InputState {
    finishing: bool,
    closed: bool,
    failure: Option<&'static str>,
}

/// Cloneable producer so native audio and provider events can be driven in
/// independent select branches. Queue overflow cancels the entire recording;
/// it never silently drops audio or pretends a discontinuity is complete.
#[derive(Clone)]
pub struct StreamingAsrInput {
    tx: mpsc::Sender<Command>,
    state: Arc<Mutex<InputState>>,
    finish_requested: CancellationToken,
    cancel: CancellationToken,
}

impl StreamingAsrInput {
    fn new(tx: mpsc::Sender<Command>, cancel: CancellationToken) -> Self {
        Self {
            tx,
            state: Arc::new(Mutex::new(InputState::default())),
            finish_requested: CancellationToken::new(),
            cancel,
        }
    }
    /// A prepared handle is usable only until its first terminal intent or
    /// worker completion. Consumers must open a fresh session after sealing.
    pub fn is_accepting(&self) -> bool {
        !self.cancel.is_cancelled()
            && !self.tx.is_closed()
            && self
                .state
                .lock()
                .is_ok_and(|state| !state.finishing && !state.closed)
    }
    /// Incremental native OS audio, mono 16 kHz, at most 60 ms per chunk.
    pub fn append_pcm16_16k(&self, samples: &[i16]) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "ASR admission state failed".to_string())?;
        if state.finishing || state.closed || self.cancel.is_cancelled() {
            return Err("ASR audio input is closed".into());
        }
        if samples.is_empty() || samples.len() > 960 {
            state.failure = Some("ASR requires 1–960 mono PCM16 samples at 16 kHz per chunk");
            self.cancel.cancel();
            return Err("ASR requires 1–960 mono PCM16 samples at 16 kHz per chunk".into());
        }
        self.tx
            .try_send(Command::Pcm16 {
                samples: samples.to_vec(),
                at: Instant::now(),
            })
            .map_err(|_| {
                state.failure = Some("ASR audio queue exceeded 240 ms or closed");
                self.cancel.cancel();
                "ASR audio queue exceeded 240 ms or closed".into()
            })
    }

    /// Called only after the native OS stream stopped and all PCM was drained.
    pub async fn finish(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err("ASR cancelled".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "ASR admission state failed".to_string())?;
        if state.closed {
            return Err("ASR input closed".into());
        }
        // A terminal intent is not audio and must not wait for room in the
        // audio queue. Publish it while holding the producer admission lock;
        // cancellation of this future cannot lose EOS or reopen admission.
        state.finishing = true;
        self.finish_requested.cancel();
        Ok(())
    }
    pub fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.finishing = true;
        }
        self.cancel.cancel();
    }
}

pub struct StreamingAsrSession {
    input: StreamingAsrInput,
    events: mpsc::Receiver<StreamingAsrEvent>,
    worker: tokio::task::JoinHandle<()>,
}
impl StreamingAsrSession {
    pub fn input(&self) -> StreamingAsrInput {
        self.input.clone()
    }
    pub async fn recv(&mut self) -> Option<StreamingAsrEvent> {
        self.events.recv().await
    }
    pub fn cancel(&self) {
        self.input.cancel();
    }
    pub async fn close(mut self) {
        self.cancel();
        if tokio::time::timeout(Duration::from_secs(1), &mut self.worker)
            .await
            .is_err()
        {
            self.worker.abort();
        }
    }
}
impl Drop for StreamingAsrSession {
    fn drop(&mut self) {
        self.input.cancel();
        self.worker.abort();
    }
}

impl ModelInvokeService {
    pub async fn available_streaming_asr_models(&self) -> Result<Vec<ModelRef>, InvokeError> {
        let rows = self
            .provider_model_capability_repo
            .list()
            .await
            .map_err(|e| InvokeError::config(format!("cannot read ASR catalog: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            if row.task != "speech_recognition" || !supports_protocol(&row.protocol) {
                continue;
            }
            let selected = ModelRef {
                provider_id: row.provider_id,
                model: row.model,
            };
            match self
                .resolve_task_config(&selected, ModelTask::SpeechRecognition)
                .await
            {
                Ok(config) if validate_model(&config).is_ok() => out.push(selected),
                Err(e) if e.is_catalog_failure() => return Err(e),
                _ => {}
            }
        }
        Ok(out)
    }

    pub async fn open_streaming_asr(
        &self,
        model: &ModelRef,
        config: StreamingAsrConfig,
    ) -> Result<StreamingAsrSession, InvokeError> {
        let (socket, wire) = prepare_session(
            self.resolve_task_config(model, ModelTask::SpeechRecognition),
            &config,
        )
        .await?;
        let (tx, commands) = mpsc::channel(AUDIO_QUEUE);
        let (event_tx, events) = mpsc::channel(64);
        let cancel = CancellationToken::new();
        let input = StreamingAsrInput::new(tx, cancel.clone());
        let finish_requested = input.finish_requested.clone();
        let admission = input.state.clone();
        let worker = tokio::spawn(async move {
            let result = run(
                socket,
                wire,
                commands,
                &event_tx,
                cancel.clone(),
                finish_requested,
            )
            .await;
            let event = terminal_event(result, &admission, &cancel);
            // A stalled UI cannot hold a socket/credential lease indefinitely.
            let _ = tokio::time::timeout(Duration::from_secs(1), event_tx.send(event)).await;
        });
        Ok(StreamingAsrSession {
            input,
            events,
            worker,
        })
    }
}

fn terminal_event(
    result: Result<String, String>,
    admission: &Mutex<InputState>,
    cancel: &CancellationToken,
) -> StreamingAsrEvent {
    let failure = match admission.lock() {
        Ok(mut state) => {
            state.closed = true;
            state.failure
        }
        Err(_) => {
            return StreamingAsrEvent::Error {
                message: "ASR admission state failed".into(),
            };
        }
    };
    if let Some(message) = failure {
        return StreamingAsrEvent::Error {
            message: message.into(),
        };
    }
    match result {
        _ if cancel.is_cancelled() => StreamingAsrEvent::Cancelled,
        Ok(text) => StreamingAsrEvent::Finished { text },
        Err(message) => StreamingAsrEvent::Error { message },
    }
}

/// One readiness deadline includes catalog/credential resolution, connection,
/// and the provider's configuration receipt. Dropping this future on timeout
/// also drops any partially opened socket; it cannot become ready later.
async fn prepare_session(
    resolve: impl std::future::Future<Output = Result<ResolvedTaskConfig, InvokeError>>,
    config: &StreamingAsrConfig,
) -> Result<(Socket, Wire), InvokeError> {
    tokio::time::timeout(OPEN_TIMEOUT, async {
        let call = resolve.await?;
        validate_model(&call)?;
        validate_config(config)?;
        let wire = wire(&call.protocol);
        let url = session_url(&call, config)?;
        let mut socket = connect(&call, &url).await?;
        if wire != Wire::Deepgram {
            configure_session(&mut socket, &call, config).await?;
        }
        Ok((socket, wire))
    })
    .await
    .map_err(|_| {
        InvokeError::new(
            InvokeErrorKind::Timeout,
            "streaming ASR preparation timed out after 20 seconds",
        )
    })?
}

pub(crate) fn validate_model(call: &ResolvedTaskConfig) -> Result<(), InvokeError> {
    if !supports_protocol(&call.protocol) {
        return Err(InvokeError::config(
            "Select a live transcription capability; file-upload ASR cannot stream keyboard audio",
        ));
    }
    if call.protocol == "openai.realtime_transcription" && call.model != "gpt-live-transcribe" {
        return Err(InvokeError::config(
            "Live partial transcription requires gpt-live-transcribe; commit-only transcription models are not supported",
        ));
    }
    if call.protocol == "stepfun.asr_stream" {
        if call.model != "stepaudio-2.5-asr-stream" {
            return Err(InvokeError::config(
                "StepFun bidirectional ASR requires the explicit stepaudio-2.5-asr-stream model",
            ));
        }
        if call.platform == "stepfun-plan" || call.connection.base_url.contains("/step_plan/") {
            return Err(InvokeError::config(
                "Step Plan currently supports file SSE ASR only; configure the ordinary StepFun streaming model with its existing ordinary API connection",
            ));
        }
    }
    call.connection.auth.validate()
}

pub fn validate_config(config: &StreamingAsrConfig) -> Result<(), InvokeError> {
    if config
        .language
        .as_ref()
        .is_some_and(|v| v.len() > 64 || v.trim().is_empty())
    {
        return Err(InvokeError::config(
            "ASR language must be a nonempty language code of at most 64 bytes",
        ));
    }
    if config.dictionary.len() > 256
        || config.dictionary.iter().any(|term| {
            term.trim().is_empty() || term.len() > 256 || term.contains(['<', '>', '\r', '\n'])
        })
    {
        return Err(InvokeError::config(
            "ASR dictionary allows up to 256 nonempty, single-line terms of 256 bytes",
        ));
    }
    Ok(())
}

fn session_url(
    call: &ResolvedTaskConfig,
    config: &StreamingAsrConfig,
) -> Result<reqwest::Url, InvokeError> {
    let endpoint =
        call.transport.realtime_endpoint.as_deref().ok_or_else(|| {
            InvokeError::config("streaming ASR capability has no session endpoint")
        })?;
    crate::call::validate_credentialed_url(
        &call.connection,
        call.transport.allow_cross_origin_credentials,
        endpoint,
        "realtime_endpoint",
        crate::call::CredentialedUrlKind::WebSocket,
        true,
    )?;
    let raw = if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        endpoint.to_owned()
    } else {
        crate::url_algebra::join_endpoint(&call.connection.base_url, endpoint)
    };
    let mut url =
        reqwest::Url::parse(&raw).map_err(|_| InvokeError::config("invalid ASR endpoint"))?;
    match url.scheme() {
        "https" => {
            let _ = url.set_scheme("wss");
        }
        "http" => {
            let _ = url.set_scheme("ws");
        }
        "ws" | "wss" => {}
        _ => return Err(InvokeError::config("invalid ASR endpoint scheme")),
    }
    if call.protocol == "deepgram.listen_stream" {
        let mut fields: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        fields.extend(crate::adapters::scalar_request_fields(
            &call.provider_params,
            &json!({}),
        )?);
        for (key, value) in [
            ("model", call.model.as_str()),
            ("encoding", "linear16"),
            ("sample_rate", "16000"),
            ("channels", "1"),
            ("interim_results", "true"),
            ("endpointing", "false"),
        ] {
            fields.insert(key.into(), value.into());
        }
        fields.entry("punctuate".into()).or_insert("true".into());
        if let Some(language) = &config.language {
            fields.insert("language".into(), language.clone());
        }
        let mut query = url.query_pairs_mut();
        query.clear();
        for (key, value) in fields {
            query.append_pair(&key, &value);
        }
        let term_key = if call.model == "nova-3" || call.model.starts_with("nova-3-") {
            "keyterm"
        } else {
            "keywords"
        };
        for term in &config.dictionary {
            query.append_pair(term_key, term);
        }
    }
    Ok(url)
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn connect(call: &ResolvedTaskConfig, url: &reqwest::Url) -> Result<Socket, InvokeError> {
    let prefix = match call.connection.auth.scheme {
        AuthScheme::Bearer => "Bearer",
        AuthScheme::TokenHeader => "Token",
        _ => {
            return Err(InvokeError::config(
                "streaming ASR needs token or bearer auth",
            ));
        }
    };
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let secrets = call.connection.auth.secrets();
    if secrets.is_empty() {
        return Err(InvokeError::config(
            "streaming ASR connection has no usable credentials",
        ));
    }
    for (index, secret) in secrets.iter().enumerate() {
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| InvokeError::config("invalid ASR WebSocket request"))?;
        let mut header = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&format!(
            "{prefix} {secret}"
        ))
        .map_err(|_| InvokeError::config("invalid ASR authentication"))?;
        header.set_sensitive(true);
        request.headers_mut().insert("Authorization", header);
        let limits = WebSocketConfig::default()
            .max_message_size(Some(MAX_WIRE))
            .max_frame_size(Some(MAX_WIRE));
        match tokio::time::timeout_at(
            deadline,
            connect_async_tls_with_config(request, Some(limits), false, None),
        )
        .await
        {
            Ok(Ok((socket, _))) => return Ok(socket),
            Err(_) => {
                return Err(InvokeError::new(
                    InvokeErrorKind::Timeout,
                    "streaming ASR connection timed out",
                ));
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
                let status = response.status().as_u16();
                if matches!(status, 401 | 403 | 429) && index + 1 < secrets.len() {
                    continue;
                }
                let kind = match status {
                    401 | 403 => InvokeErrorKind::Auth,
                    429 => InvokeErrorKind::RateLimited,
                    _ => InvokeErrorKind::ProviderError,
                };
                return Err(InvokeError::new(
                    kind,
                    format!("streaming ASR handshake rejected with HTTP {status}"),
                ));
            }
            Ok(Err(_)) => {
                return Err(InvokeError::new(
                    InvokeErrorKind::Network,
                    "streaming ASR WebSocket connection failed",
                ));
            }
        }
    }
    Err(InvokeError::config(
        "streaming ASR connection has no usable credentials",
    ))
}

async fn send(
    socket: &mut Socket,
    message: Message,
    cancel: &CancellationToken,
) -> Result<(), String> {
    tokio::select! { biased; _ = cancel.cancelled() => Err("ASR cancelled".into()),
        result = tokio::time::timeout(WRITE_TIMEOUT, socket.send(message)) => {
            result.map_err(|_| "ASR socket write timed out".to_string())?
                .map_err(|_| "ASR socket write failed".to_string())
        }
    }
}

async fn configure_session(
    socket: &mut Socket,
    call: &ResolvedTaskConfig,
    config: &StreamingAsrConfig,
) -> Result<(), InvokeError> {
    let mut transcription = crate::adapters::provider_body_fields(&call.provider_params)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    let stepfun = call.protocol == "stepfun.asr_stream";
    transcription.remove("language");
    transcription.insert("model".into(), json!(call.model));
    if !stepfun {
        transcription.insert("keywords".into(), json!(config.dictionary));
    }
    if let Some(language) = &config.language {
        transcription.insert(
            if stepfun { "language" } else { "languages" }.into(),
            if stepfun {
                json!(language)
            } else {
                json!([language])
            },
        );
    }
    let mut update = json!({"type":"session.update","session":{"type":"transcription","audio":{"input":{
        "format":{"type":"audio/pcm","rate":24000}, "transcription":transcription,"turn_detection":null}}}});
    if stepfun {
        update["event_id"] = json!(event_id());
        update["session"].as_object_mut().unwrap().remove("type");
        update["session"]["audio"]["input"]["format"] =
            json!({"type":"pcm","codec":"pcm_s16le","rate":16000,"bits":16,"channel":1});
        update["session"]["audio"]["input"]["turn_detection"] = json!({});
        let transcription = &mut update["session"]["audio"]["input"]["transcription"];
        // StepFun's second-pass correction is optional and defaults to false.
        // Preserve the catalog's explicit choice; a held-key transcript still
        // waits for the matching committed/completed receipts either way.
        if !config.dictionary.is_empty() {
            let prompt = transcription["prompt"]
                .as_str()
                .unwrap_or("请准确转写所听到的语音。");
            transcription["prompt"] =
                json!(format!("{prompt}\n术语：{}", config.dictionary.join("、")));
        }
    }
    let handshake = async {
        send(
            socket,
            Message::Text(update.to_string().into()),
            &CancellationToken::new(),
        )
        .await?;
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let raw: Value = serde_json::from_str(&text)
                        .map_err(|_| "invalid ASR configuration acknowledgement".to_string())?;
                    match raw["type"].as_str() {
                        Some("session.updated")
                            if (stepfun
                                && raw["session"]["audio"]["input"]["format"]["rate"] == 16000)
                                || (!stepfun && raw["session"]["type"] == "transcription") =>
                        {
                            if stepfun
                                && raw["session"]["audio"]["input"]["turn_detection"]["type"]
                                    == "server_vad"
                            {
                                return Err(
                                    "StepFun did not disable server VAD for held-key recording"
                                        .into(),
                                );
                            }
                            return Ok::<(), String>(());
                        }
                        Some("session.created") => {}
                        _ => {
                            return Err(
                                "ASR rejected the transcription session configuration".into()
                            );
                        }
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    send(socket, Message::Pong(payload), &CancellationToken::new()).await?
                }
                _ => return Err("ASR disconnected during session configuration".into()),
            }
        }
    };
    tokio::time::timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| {
            InvokeError::new(
                InvokeErrorKind::Timeout,
                "ASR session configuration timed out",
            )
        })?
        .map_err(InvokeError::config)
}

#[derive(Default)]
struct Transcript {
    segments: BTreeMap<String, (String, bool)>,
    order: Vec<String>,
}
impl Transcript {
    fn put(&mut self, id: String, text: String, final_: bool) -> Result<bool, String> {
        if text.len() > MAX_TEXT || self.segments.len() >= 4096 {
            return Err("ASR transcript exceeded local limit".into());
        }
        if text.is_empty() && !final_ && !self.segments.contains_key(&id) {
            return Ok(false);
        }
        if let Some((old, true)) = self.segments.get(&id) {
            return if old == &text {
                Ok(false)
            } else {
                Err("ASR changed a finalized segment".into())
            };
        }
        if !self.segments.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.segments.insert(id, (text, final_));
        if self.text(false).len() > MAX_TEXT {
            return Err("ASR transcript exceeded local limit".into());
        }
        Ok(true)
    }
    fn text(&self, final_only: bool) -> String {
        self.order
            .iter()
            .filter_map(|id| self.segments.get(id))
            .filter(|(_, done)| !final_only || *done)
            .map(|(text, _)| text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("")
    }
    fn completed(&self) -> Result<String, String> {
        if self.segments.values().any(|(_, done)| !done) {
            return Err("ASR stream ended with an unfinished transcript".into());
        }
        Ok(self.text(true))
    }
    fn final_ready(&self, committed_item: Option<&str>) -> bool {
        committed_item.is_some_and(|id| self.segments.get(id).is_some_and(|(_, done)| *done))
    }
}

/// StepFun creates a buffer item and a distinct transcription source item.
/// Both must be announced by this socket before their receipts are accepted.
/// A fresh session has one manual commit and one source; an arbitrary sole
/// final is not proof that it belongs to the submitted recording.
#[derive(Default)]
struct StepFunItems {
    created: BTreeSet<String>,
    source: Option<String>,
}
impl StepFunItems {
    fn created(&mut self, raw: &Value) -> Result<(), String> {
        if self.created.len() >= 4 {
            return Err("ASR created unexpected conversation items".into());
        }
        self.created.insert(required(&raw["item"], "id")?);
        Ok(())
    }
    fn source(&mut self, id: &str) -> Result<(), String> {
        if !self.created.contains(id) {
            return Err("ASR transcript has no matching source item".into());
        }
        if self.source.as_deref().is_some_and(|source| source != id) {
            return Err("ASR returned multiple sources for one held recording".into());
        }
        self.source = Some(id.to_owned());
        Ok(())
    }
    fn committed_source(&self, committed: Option<&str>) -> Option<&str> {
        committed.filter(|id| self.created.contains(*id))?;
        self.source.as_deref()
    }
}

async fn run(
    mut socket: Socket,
    wire: Wire,
    mut commands: mpsc::Receiver<Command>,
    events: &mpsc::Sender<StreamingAsrEvent>,
    cancel: CancellationToken,
    finish_requested: CancellationToken,
) -> Result<String, String> {
    let deepgram = wire == Wire::Deepgram;
    let stepfun = wire == Wire::StepFun;
    let mut transcript = Transcript::default();
    let mut resampler = audio::Resampler16To24::default();
    let mut finishing = false;
    let mut draining = false;
    let mut committed_item: Option<String> = None;
    let mut stepfun_items = StepFunItems::default();
    // The application preconnects before it grants capture. Idle time does
    // not consume the actual recording limit; provider expiry is an explicit
    // connection failure and the application may reconnect while idle.
    let mut final_deadline = Instant::now() + Duration::from_secs(24 * 3600);
    let mut heartbeat = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(4),
        Duration::from_secs(4),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut samples = 0usize;
    let mut signal = false;
    loop {
        // Fair socket/audio polling prevents repeated provider deltas or
        // heartbeats from starving the producer's drain and commit. send()
        // independently gives cancellation priority before every write.
        tokio::select! {
            _ = cancel.cancelled() => return Err("ASR cancelled".into()),
            _ = finish_requested.cancelled(), if !draining && !finishing => {
                commands.close();
                draining = true;
            },
            _ = tokio::time::sleep_until(final_deadline) => return Err(if finishing {
                if !deepgram && committed_item.is_some(){"ASR 已确认音频提交，但没有返回完整转写结果；请重试或改用普通 ASR"}
                else{"ASR finalization timed out"}
            } else { "ASR recording exceeded ten minutes" }.into()),
            _ = heartbeat.tick(), if !finishing => {
                let message = if deepgram { Message::Text(json!({"type":"KeepAlive"}).to_string().into()) }
                    else { Message::Ping(Vec::new().into()) };
                send(&mut socket, message, &cancel).await?;
            },
            next = socket.next() => {
                let message = next.ok_or_else(|| "ASR disconnected before finalization".to_string())?
                    .map_err(|_| "ASR transport failed".to_string())?;
                let raw = match message {
                    Message::Text(text) => serde_json::from_str::<Value>(&text).map_err(|_| "ASR sent malformed JSON".to_string())?,
                    Message::Ping(payload) => { send(&mut socket, Message::Pong(payload), &cancel).await?; continue; },
                    Message::Pong(_) => continue,
                    Message::Close(_) => return Err("ASR closed before its final acknowledgement".into()),
                    _ => return Err("ASR sent unexpected binary data".into()),
                };
                let kind = raw["type"].as_str().ok_or_else(|| "ASR event has no type".to_string())?;
                if kind == "error" || kind == "Error" || kind.ends_with(".failed") { return Err("ASR provider rejected the transcription; check model, language, dictionary and credentials".into()); }
                if stepfun && kind == "conversation.item.created" {
                    stepfun_items.created(&raw)?;
                }
                if deepgram && kind == "Metadata" && finishing { return transcript.completed(); }
                if !deepgram && kind == "input_audio_buffer.committed" {
                    if !finishing { return Err("ASR committed input before native capture stopped".into()); }
                    committed_item = Some(required(&raw, "item_id")?);
                    if stepfun && !stepfun_items.created.contains(committed_item.as_deref().unwrap()) {
                        return Err("ASR commit has no matching buffer item".into());
                    }
                    let source = if stepfun { stepfun_items.committed_source(committed_item.as_deref()) } else { committed_item.as_deref() };
                    if transcript.final_ready(source) {
                        return transcript.completed();
                    }
                }
                let parsed = if deepgram && kind == "Results" {
                    let start = raw["start"].as_f64().filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| "ASR result has invalid start".to_string())?;
                    let text = raw["channel"]["alternatives"][0]["transcript"].as_str().ok_or_else(|| "ASR result has no transcript".to_string())?.to_owned();
                    Some((format!("{start:.6}"), text, raw["is_final"].as_bool().ok_or_else(|| "ASR result has no final flag".to_string())?))
                } else if !deepgram && kind == "conversation.item.input_audio_transcription.delta" {
                    let id = required(&raw, "item_id")?;
                    let text=if stepfun{
                        let mut text=required(&raw,"text")?;
                        let stash=raw["stash"].as_str().unwrap_or("");
                        // Stable prefix and revisable suffix are separate
                        // fields; equal syllables still represent repetition.
                        text.push_str(stash);
                        text
                    }else{
                        let previous = transcript.segments.get(&id).map_or("", |(text, _)| text.as_str());
                        format!("{previous}{}", required(&raw, "delta")?)
                    };
                    Some((id, text, false))
                } else if !deepgram && kind == "conversation.item.input_audio_transcription.completed" {
                    Some((required(&raw, "item_id")?, required(&raw, "transcript")?, true))
                } else { None };
                if let Some((id, text, final_)) = parsed {
                    if stepfun { stepfun_items.source(&id)?; }
                    let changed = transcript.put(id.clone(), text.clone(), final_)?;
                    if changed {
                        events.try_send(if final_ { StreamingAsrEvent::Final { segment_id: id.clone(), text } }
                            else { StreamingAsrEvent::Partial { text: transcript.text(false) } })
                            .map_err(|_| "ASR transcript consumer exceeded backlog limit".to_string())?;
                    }
                    let source = if stepfun { stepfun_items.committed_source(committed_item.as_deref()) } else { committed_item.as_deref() };
                    if !deepgram && finishing && final_ && transcript.final_ready(source) { return transcript.completed(); }
                }
            },
            command = commands.recv(), if !finishing => {
                match command {
                    Some(Command::Pcm16 { samples: pcm, at }) => {
                        if at.elapsed() > MAX_AUDIO_AGE { return Err("ASR audio backlog exceeded 250 ms".into()); }
                        if samples == 0 { final_deadline = Instant::now() + MAX_RECORDING; }
                        signal |= pcm.iter().any(|sample| *sample != 0);
                        samples += pcm.len();
                        let pcm = if wire!=Wire::OpenAi { pcm } else { resampler.push(&pcm) };
                        let bytes: Vec<u8> = pcm.iter().flat_map(|value| value.to_le_bytes()).collect();
                        let message = if deepgram { Message::Binary(bytes.into()) }
                            else { audio_message(&bytes,stepfun) };
                        send(&mut socket, message, &cancel).await?;
                    },
                    None if draining || finish_requested.is_cancelled() => {
                        finishing = true;
                        final_deadline = Instant::now() + FINAL_TIMEOUT;
                        if samples == 0 { return Ok(String::new()); }
                        if !signal { return Err("未检测到麦克风输入信号，请检查系统输入设备后重试".into()); }
                        if wire==Wire::OpenAi {
                            let tail = resampler.finish();
                            if !tail.is_empty() {
                                let bytes: Vec<u8> = tail.iter().flat_map(|value| value.to_le_bytes()).collect();
                                send(&mut socket, Message::Text(json!({"type":"input_audio_buffer.append","audio":base64::engine::general_purpose::STANDARD.encode(bytes)}).to_string().into()), &cancel).await?;
                            }
                        }
                        let mut end = if deepgram { json!({"type":"CloseStream"}) } else { json!({"type":"input_audio_buffer.commit"}) };
                        if stepfun{end["event_id"]=json!(event_id());}
                        send(&mut socket, Message::Text(end.to_string().into()), &cancel).await?;
                    },
                    None => return Err("ASR input ended before native finalization".into()),
                }
            }
        }
    }
}

fn required(raw: &Value, key: &str) -> Result<String, String> {
    raw[key]
        .as_str()
        .filter(|value| value.len() <= MAX_TEXT)
        .map(str::to_owned)
        .ok_or_else(|| format!("ASR event has invalid {key}"))
}
