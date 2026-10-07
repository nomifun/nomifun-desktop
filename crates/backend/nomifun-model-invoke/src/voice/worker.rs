use super::openai_live_wire::{self, LiveFormat};
use super::stepfun_wire::StepFunDecoder;
use super::transport::{self, Socket};
use super::wire::{WireEvent, required_string, string};
use super::{
    OpenAiLiveVoiceModel, StepFunVoiceModel, invoke_error, media_spec, negotiated_capabilities,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nomifun_voice_contracts::voice::*;
use nomifun_voice_core::{VoiceModelSession, VoiceOpenRequest, VoiceSessionLimits, VoiceWorkerIo};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const MAX_WIRE_BYTES: usize = 256 * 1024;
const MAX_AUDIO_BYTES: usize = 48_000;
const MAX_FACT_BYTES: usize = 128 * 1024;
const MAX_FACTS: usize = 48;
const MAX_LOCAL_BYTES: usize = 64 * 1024;
const MAX_LOCAL_ENTRIES: usize = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const INPUT_ACK_TIMEOUT: Duration = Duration::from_secs(2);
static NEXT_EVENT: AtomicU64 = AtomicU64::new(1);
fn event_id() -> String {
    format!(
        "nomifun_voice_{}",
        NEXT_EVENT.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Clone)]
pub(super) enum Provider {
    StepFun(StepFunVoiceModel),
    Live(OpenAiLiveVoiceModel),
}
impl Provider {
    fn connection(&self) -> &crate::ResolvedConnection {
        match self {
            Self::StepFun(p) => &p.connection,
            Self::Live(p) => &p.connection,
        }
    }
    fn format(&self) -> LiveFormat {
        match self {
            Self::StepFun(_) => LiveFormat::Pcm24k,
            Self::Live(p) => p.config.format,
        }
    }
    fn native(&self) -> bool {
        matches!(self,Self::Live(p) if p.native)
    }
    fn model(&self) -> &str {
        match self {
            Self::StepFun(p) => &p.model,
            Self::Live(p) => &p.model,
        }
    }
    fn url(&self, model: &str) -> Result<reqwest::Url, VoiceError> {
        let mut url = match self {
            Self::StepFun(p) => {
                transport::endpoint(&p.connection, &p.endpoint, true, p.allow_cross_origin)
            }
            Self::Live(p) => {
                transport::endpoint(&p.connection, &p.endpoint, true, p.allow_cross_origin)
            }
        }
        .map_err(invoke_error)?;
        match self {
            Self::StepFun(_) => {
                let query: Vec<_> = url
                    .query_pairs()
                    .filter(|(k, _)| k != "model")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                url.query_pairs_mut()
                    .clear()
                    .extend_pairs(query)
                    .append_pair("model", model);
            }
            Self::Live(_) => {
                if url.query().is_some() {
                    return Err(VoiceError::new(
                        VoiceErrorKind::Configuration,
                        "GPT-Live primary endpoint does not accept query parameters",
                    ));
                }
            }
        }
        Ok(url)
    }
}

pub(super) async fn open(
    mut provider: Provider,
    request: VoiceOpenRequest,
    cancel: CancellationToken,
    deadline: Instant,
) -> Result<VoiceModelSession, VoiceError> {
    if request.model.trim().is_empty() || request.model != provider.model() {
        return Err(VoiceError::new(
            VoiceErrorKind::StaleBinding,
            "voice model does not match frozen credential lease",
        ));
    }
    if matches!(provider, Provider::StepFun(_))
        && request.transport != VoiceTransportPreference::Relay
    {
        return Err(VoiceError::new(
            VoiceErrorKind::Configuration,
            "selected voice adapter does not support native media",
        ));
    }
    if let Provider::Live(p) = &mut provider {
        p.native = request.transport == VoiceTransportPreference::NativeWebrtc;
    }
    if matches!(provider.format(), LiveFormat::Pcmu8k | LiveFormat::Pcma8k) {
        return Err(VoiceError::new(
            VoiceErrorKind::Unsupported,
            "this endpoint requires PCM relay; G.711 needs an encoded-media endpoint",
        ));
    }
    let caps = negotiated_capabilities(&provider, &request.model);
    validate_native_duplex(&caps)
        .map_err(|message| VoiceError::new(VoiceErrorKind::Unsupported, message))?;
    let mut decoder = StepFunDecoder::default();
    let state = State::from_initial(&request)?;
    let opened = tokio::select! {biased;
        _=cancel.cancelled()=>return Err(VoiceError::new(VoiceErrorKind::Closed,"voice opening cancelled")),
        result=tokio::time::timeout_at(deadline,connect_ready(&provider,&request,&mut decoder,state.context_history(&request)))=>result.map_err(|_|VoiceError::new(VoiceErrorKind::Deadline,"voice opening deadline"))??,
    };
    let (socket, upstream_id, attachment) = opened;
    let negotiation = VoiceNegotiation {
        capabilities: caps,
        input_spec: if provider.native() {
            None
        } else {
            Some(media_spec(provider.format(), false))
        },
        output_spec: if provider.native() {
            None
        } else {
            Some(media_spec(provider.format(), true))
        },
        native_attachment: attachment,
    };
    let limits = VoiceSessionLimits::default();
    let (io, worker_io) = VoiceModelSession::channels_with_specs(
        negotiation.input_spec.clone(),
        negotiation.output_spec.clone(),
        limits,
        cancel,
    )?;
    let ready = negotiation.clone();
    let worker = tokio::spawn(async move {
        run(
            provider,
            request,
            socket,
            upstream_id,
            decoder,
            worker_io,
            ready,
            limits,
            state,
        )
        .await;
    });
    Ok(VoiceModelSession::from_parts(negotiation, io, worker))
}

async fn connect_ready(
    provider: &Provider,
    request: &VoiceOpenRequest,
    decoder: &mut StepFunDecoder,
    history: Value,
) -> Result<(Socket, String, Option<VoiceNativeAttachment>), VoiceError> {
    let instructions = startup_instructions(request);
    let endpoint = provider.url(&request.model)?;
    if let Provider::Live(p) = provider {
        if provider.native() {
            let offer = request
                .native_offer
                .as_ref()
                .filter(|offer| !offer.trim().is_empty() && offer.len() <= 64 * 1024)
                .ok_or_else(|| {
                    VoiceError::new(
                        VoiceErrorKind::Configuration,
                        "native voice activation requires a bounded SDP offer",
                    )
                })?;
            let mut http_endpoint = endpoint.clone();
            let http_scheme = if endpoint.scheme() == "wss" {
                "https"
            } else {
                "http"
            };
            http_endpoint.set_scheme(http_scheme).map_err(|_| {
                VoiceError::new(
                    VoiceErrorKind::Configuration,
                    "invalid native voice endpoint",
                )
            })?;
            let (response, selected_connection)=transport::create_native(&p.http,&p.connection,&http_endpoint,&json!({"session":p.config.session(&request.model,&instructions,json!([]),true),"transport":{"type":"webrtc","sdp":offer}}),CONNECT_TIMEOUT,MAX_WIRE_BYTES).await.map_err(invoke_error)?;
            let session = response.get("session").ok_or_else(|| {
                VoiceError::new(
                    VoiceErrorKind::Provider,
                    "native voice response has no session",
                )
            })?;
            let id = required_string(session, "id").map_err(provider_error)?;
            let mut attach = endpoint.clone();
            // Path segments percent-encode the opaque ID; do not interpolate it
            // into a URL or treat an ID prefix as routing authority.
            attach
                .path_segments_mut()
                .map_err(|_| {
                    VoiceError::new(VoiceErrorKind::Configuration, "invalid sideband endpoint")
                })?
                .pop_if_empty()
                .push(&id)
                .push("attach");
            let mut cleanup = NativeStartupCleanup {
                connection: selected_connection.clone(),
                attach: attach.clone(),
                armed: true,
            };
            let sdp = response
                .get("transport")
                .and_then(|transport| string(transport, "sdp"))
                .filter(|sdp| !sdp.trim().is_empty() && sdp.len() <= 64 * 1024)
                .ok_or_else(|| provider_error("native voice response has no bounded SDP answer"))?;
            validate_configuration(provider, &request.model, session)?;
            let mut socket = transport::connect(
                &selected_connection,
                &attach,
                CONNECT_TIMEOUT,
                MAX_WIRE_BYTES,
            )
            .await
            .map_err(invoke_error)?;
            // Sideband attachment does not replay session.started. A real
            // context acknowledgment establishes initialization on this live
            // connection before the product can release its media lease.
            restore_live_history(&mut socket, &history, &selected_connection, true).await?;
            cleanup.armed = false;
            return Ok((
                socket,
                id.clone(),
                Some(VoiceNativeAttachment {
                    attachment_id: id,
                    answer_sdp: sdp,
                    safe_output_boundary: true,
                }),
            ));
        }
    }
    let mut socket = transport::connect(
        provider.connection(),
        &endpoint,
        CONNECT_TIMEOUT,
        MAX_WIRE_BYTES,
    )
    .await
    .map_err(invoke_error)?;
    let initial = match provider {
        Provider::StepFun(p) => {
            p.config
                .session_update(&instructions, stepfun_tools(&request.tools), &event_id())
        }
        Provider::Live(p) => {
            json!({"type":"session.start","event_id":event_id(),"session":p.config.session(&request.model,&instructions,json!([]),false)})
        }
    };
    let text = serde_json::to_string(&initial)
        .map_err(|_| provider_error("voice startup encoding failed"))?;
    if text.len() > MAX_WIRE_BYTES {
        return Err(VoiceError::new(
            VoiceErrorKind::Configuration,
            "voice startup exceeds message budget",
        ));
    }
    tokio::time::timeout(
        Duration::from_secs(3),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| VoiceError::new(VoiceErrorKind::Deadline, "voice startup write timed out"))?
    .map_err(|_| VoiceError::new(VoiceErrorKind::Network, "voice startup write failed"))?;
    let redactor = provider.connection().auth.secret_redactor();
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let raw: Value = serde_json::from_str(&text)
                    .map_err(|_| provider_error("voice startup JSON is invalid"))?;
                for event in decode(provider, decoder, &raw).map_err(provider_error)? {
                    match event {
                        WireEvent::Ready {
                            session_id,
                            configuration,
                        } => {
                            validate_configuration(provider, &request.model, &configuration)?;
                            if matches!(provider, Provider::StepFun(_)) {
                                restore_step_history(&mut socket, &history, provider.connection())
                                    .await?;
                            } else {
                                restore_live_history(
                                    &mut socket,
                                    &history,
                                    provider.connection(),
                                    false,
                                )
                                .await?;
                            }
                            return Ok((socket, session_id, None));
                        }
                        WireEvent::Error { message, .. } => {
                            return Err(provider_error(redactor.redact(&message)));
                        }
                        _ => {}
                    }
                }
            }
            Some(Ok(Message::Ping(payload))) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| provider_error("voice startup pong failed"))?;
            }
            Some(Ok(Message::Pong(_))) => {}
            _ => {
                return Err(VoiceError::new(
                    VoiceErrorKind::Network,
                    "voice closed before configuration acknowledgment",
                ));
            }
        }
    }
}

fn startup_instructions(request: &VoiceOpenRequest) -> String {
    request.instructions.clone()
}

async fn restore_step_history(
    socket: &mut Socket,
    history: &Value,
    connection: &crate::ResolvedConnection,
) -> Result<(), VoiceError> {
    for message in history.as_array().into_iter().flatten() {
        let mut item = message.clone();
        // Step's conversation schema uses system facts; the neutral stored
        // history uses developer for verified application facts.
        if item.get("role").and_then(Value::as_str) == Some("developer") {
            item["role"] = json!("system");
        }
        let text =
            json!({"type":"conversation.item.create","event_id":event_id(),"item":item.clone()})
                .to_string();
        if text.len() > MAX_WIRE_BYTES {
            return Err(provider_error(
                "voice restored context exceeds message budget",
            ));
        }
        tokio::time::timeout(
            Duration::from_millis(200),
            socket.send(Message::Text(text.into())),
        )
        .await
        .map_err(|_| {
            VoiceError::new(
                VoiceErrorKind::Deadline,
                "voice context restore write timed out",
            )
        })?
        .map_err(|_| {
            VoiceError::new(
                VoiceErrorKind::Network,
                "voice context restore write failed",
            )
        })?;
        loop {
            let raw = initialization_message(socket, connection).await?;
            if string(&raw, "type").as_deref() == Some("conversation.item.created")
                && raw.get("item").is_some_and(|accepted| {
                    accepted.get("type") == item.get("type")
                        && accepted.get("role") == item.get("role")
                        && accepted.get("content") == item.get("content")
                })
            {
                break;
            }
        }
    }
    Ok(())
}

async fn initialization_message(
    socket: &mut Socket,
    connection: &crate::ResolvedConnection,
) -> Result<Value, VoiceError> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let raw: Value = serde_json::from_str(&text)
                    .map_err(|_| provider_error("invalid context initialization acknowledgment"))?;
                if string(&raw, "type").as_deref() == Some("error") {
                    let message = raw
                        .get("error")
                        .and_then(|error| string(error, "message"))
                        .unwrap_or_else(|| "context initialization rejected".into());
                    return Err(provider_error(
                        connection.auth.secret_redactor().redact(&message),
                    ));
                }
                return Ok(raw);
            }
            Some(Ok(Message::Ping(payload))) => {
                tokio::time::timeout(
                    Duration::from_millis(200),
                    socket.send(Message::Pong(payload)),
                )
                .await
                .map_err(|_| {
                    VoiceError::new(
                        VoiceErrorKind::Deadline,
                        "context initialization pong timed out",
                    )
                })?
                .map_err(|_| {
                    VoiceError::new(
                        VoiceErrorKind::Network,
                        "context initialization pong failed",
                    )
                })?;
            }
            Some(Ok(Message::Pong(_))) => {}
            _ => {
                return Err(VoiceError::new(
                    VoiceErrorKind::Network,
                    "voice ended before context initialization acknowledgment",
                ));
            }
        }
    }
}
async fn restore_live_history(
    socket: &mut Socket,
    history: &Value,
    connection: &crate::ResolvedConnection,
    barrier: bool,
) -> Result<(), VoiceError> {
    let mut contents = Vec::new();
    for message in history.as_array().into_iter().flatten() {
        let text = message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
        let label = if message.get("role").and_then(Value::as_str) == Some("user") {
            "Committed historical input; do not resubmit its work"
        } else {
            "Application context record"
        };
        contents.extend(bounded_chunks(&format!("{label}: {text}")));
    }
    if barrier || !contents.is_empty() {
        contents.push("Application context initialized. Await current user input; do not replay historical work.".into());
    }
    for content in contents {
        let id = event_id();
        let command = openai_live_wire::context_append(&content, None, false, &id).to_string();
        tokio::time::timeout(
            Duration::from_millis(200),
            socket.send(Message::Text(command.into())),
        )
        .await
        .map_err(|_| {
            VoiceError::new(
                VoiceErrorKind::Deadline,
                "context initialization write timed out",
            )
        })?
        .map_err(|_| {
            VoiceError::new(
                VoiceErrorKind::Network,
                "context initialization write failed",
            )
        })?;
        loop {
            let raw = initialization_message(socket, connection).await?;
            if string(&raw, "type").as_deref() == Some("session.thinking.appended")
                && string(&raw, "client_event_id").as_deref() == Some(id.as_str())
            {
                break;
            }
        }
    }
    Ok(())
}

/// A native HTTP create can succeed before a cancelled/failed sideband attach.
/// Reattach once only to request the documented session.close; Live's SIP-only
/// hangup endpoint is not assumed to control a WebRTC session.
struct NativeStartupCleanup {
    connection: crate::ResolvedConnection,
    attach: reqwest::Url,
    armed: bool,
}
impl Drop for NativeStartupCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let connection = self.connection.clone();
        let attach = self.attach.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let cleanup = async {
                    let Ok(socket) = transport::connect(
                        &connection,
                        &attach,
                        Duration::from_secs(3),
                        MAX_WIRE_BYTES,
                    )
                    .await
                    else {
                        return;
                    };
                    let (mut writer, mut reader, mut guard) =
                        transport::pump(socket, None, VoiceSessionLimits::default());
                    let _ = transport::send_json(
                        &mut writer,
                        json!({"type":"session.close","event_id":event_id()}),
                        Duration::from_secs(1),
                        MAX_WIRE_BYTES,
                    )
                    .await;
                    while let Some(Ok(Message::Text(text))) = reader.next().await {
                        if serde_json::from_str::<Value>(&text)
                            .ok()
                            .and_then(|value| string(&value, "type"))
                            .as_deref()
                            == Some("session.closed")
                        {
                            break;
                        }
                    }
                    let _ = writer.send(Message::Close(None)).await;
                    guard.shutdown().await;
                };
                let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
            });
        }
    }
}

fn validate_configuration(
    provider: &Provider,
    model: &str,
    configuration: &Value,
) -> Result<(), VoiceError> {
    if configuration.get("model").and_then(Value::as_str) != Some(model) {
        return Err(VoiceError::new(
            VoiceErrorKind::StaleBinding,
            "upstream negotiated a different voice model",
        ));
    }
    match provider {
        Provider::StepFun(p) => {
            for field in ["input_audio_format", "output_audio_format"] {
                if configuration.get(field).and_then(Value::as_str) != Some("pcm16") {
                    return Err(provider_error("StepFun did not confirm PCM16 media"));
                }
            }
            if let Some(voice) = &p.config.voice {
                if configuration.get("voice").and_then(Value::as_str) != Some(voice.as_str()) {
                    return Err(provider_error("StepFun negotiated a different voice"));
                }
            }
        }
        Provider::Live(p) => {
            if !provider.native()
                && configuration
                    .get("audio")
                    .and_then(|audio| audio.get("format"))
                    != Some(&p.config.format.wire())
            {
                return Err(provider_error(
                    "GPT-Live negotiated a different audio format",
                ));
            }
            if configuration
                .get("delegation")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                != Some("client")
            {
                return Err(provider_error(
                    "GPT-Live did not negotiate client delegation",
                ));
            }
        }
    }
    Ok(())
}

fn decode(
    provider: &Provider,
    decoder: &mut StepFunDecoder,
    raw: &Value,
) -> Result<Vec<WireEvent>, String> {
    match provider {
        Provider::StepFun(_) => decoder.decode(raw, MAX_AUDIO_BYTES),
        Provider::Live(_) => {
            openai_live_wire::decode(raw, MAX_AUDIO_BYTES, provider.format(), provider.native())
        }
    }
}
fn stepfun_tools(tools: &[VoiceToolDefinition]) -> Value {
    json!(tools.iter().map(|tool|json!({"type":"function","function":{"name":tool.name,"description":tool.description,"parameters":tool.parameters}})).collect::<Vec<_>>())
}
fn provider_error(message: impl Into<String>) -> VoiceError {
    VoiceError::new(VoiceErrorKind::Provider, message)
}

struct PendingContext {
    context: VoiceWorkContextFact,
    receipt_id: String,
    pending: BTreeSet<String>,
    range: Option<MediaRange>,
    source: Option<VoiceSpeechSourceRef>,
}
/// Cache metadata stays private. Only `message` is restored on the real wire.
struct RememberedFact {
    message: Value,
    source: Option<VoiceSpeechSourceRef>,
}
struct RememberedUser {
    fragment_key: String,
    revision: u64,
    message: Value,
}
struct RequestedSpeech {
    correlation_id: String,
    source: Option<VoiceSpeechSourceRef>,
}
struct PendingInputControl {
    request_id: String,
    wire_id: String,
    muted: bool,
    deadline: Instant,
}
struct PendingInputClear {
    wire_id: String,
    product: bool,
}
struct PendingInputPresentation {
    pending: BTreeSet<String>,
    step_item: Option<Value>,
    deadline: Instant,
}
enum DataIngress {
    Frame(Option<AudioFrame>),
    Wire(Option<Result<Message, transport::PumpError>>),
}
struct State {
    generation: u64,
    sequence: u64,
    cursor_us: u64,
    muted: bool,
    pending_input: Option<PendingInputControl>,
    input_clears: VecDeque<PendingInputClear>,
    input_failure: Option<VoiceError>,
    input_presentations: BTreeMap<String, PendingInputPresentation>,
    blocked: bool,
    transcripts: BTreeMap<String, (u64, String)>,
    segments: BTreeSet<String>,
    active_responses: BTreeSet<String>,
    known_calls: BTreeSet<String>,
    returned_calls: BTreeSet<String>,
    requested_speech: VecDeque<RequestedSpeech>,
    pending_response_creates: usize,
    pending_context: BTreeMap<String, PendingContext>,
    revoked: BTreeSet<String>,
    delegations: BTreeSet<String>,
    user_history: Vec<RememberedUser>,
    facts: Vec<RememberedFact>,
    played: Vec<Value>,
    revoked_speech: BTreeMap<String, VoiceSpeechSourceRef>,
    force_rebuild: bool,
}
impl State {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            sequence: 0,
            cursor_us: 0,
            muted: false,
            pending_input: None,
            input_clears: VecDeque::new(),
            input_failure: None,
            input_presentations: BTreeMap::new(),
            blocked: false,
            transcripts: BTreeMap::new(),
            segments: BTreeSet::new(),
            active_responses: BTreeSet::new(),
            known_calls: BTreeSet::new(),
            returned_calls: BTreeSet::new(),
            requested_speech: VecDeque::new(),
            pending_response_creates: 0,
            pending_context: BTreeMap::new(),
            revoked: BTreeSet::new(),
            delegations: BTreeSet::new(),
            user_history: Vec::new(),
            facts: Vec::new(),
            played: Vec::new(),
            revoked_speech: BTreeMap::new(),
            force_rebuild: false,
        }
    }
    fn from_initial(request: &VoiceOpenRequest) -> Result<Self, VoiceError> {
        request
            .replay_scope
            .validate()
            .map_err(|message| VoiceError::new(VoiceErrorKind::Configuration, message))?;
        if request.initial_facts.len() > MAX_FACTS {
            return Err(VoiceError::new(
                VoiceErrorKind::Configuration,
                "initial voice facts exceed context count budget",
            ));
        }
        let mut state = Self::new(request.output_generation);
        let mut total = 0usize;
        for fact in &request.initial_facts {
            if fact.content.trim().is_empty()
                || fact.canonical_receipt_id.trim().is_empty()
                || fact
                    .output_generation
                    .is_some_and(|generation| generation != request.output_generation)
            {
                return Err(VoiceError::new(
                    VoiceErrorKind::Configuration,
                    "initial voice facts require current generation and canonical source identity",
                ));
            }
            if let Some(source) = &fact.speech_source {
                source
                    .validate()
                    .map_err(|message| VoiceError::new(VoiceErrorKind::Configuration, message))?;
            }
            let message = json!({"type":"message","role":"developer","content":[{"type":"input_text","text":fact.content}]});
            total = total.saturating_add(message.to_string().len());
            if total > MAX_FACT_BYTES {
                return Err(VoiceError::new(
                    VoiceErrorKind::Configuration,
                    "initial voice facts exceed context byte budget",
                ));
            }
            state.remember_fact(message, fact.speech_source.clone());
        }
        if let Some(replay) = &request.initial_local_replay {
            replay
                .validate_for(&request.replay_scope)
                .map_err(|message| VoiceError::new(VoiceErrorKind::StaleBinding, message))?;
            let mut bytes = 0usize;
            for entry in &replay.entries {
                let (message, user) = match entry {
                    VoiceLocalReplayEntry::UserCommitted { origin, fragment } => (
                        json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("Historical committed voice-local input (already received; never resubmit work from this record) [{}]: {}",serde_json::to_string(origin).unwrap_or_default(),fragment.text)}]}),
                        Some((
                            format!(
                                "replay:{}",
                                serde_json::to_string(&(
                                    &origin.voice_session_id,
                                    origin.activation_epoch,
                                    &fragment.fragment_id
                                ))
                                .unwrap_or_default()
                            ),
                            fragment.revision,
                        )),
                    ),
                    VoiceLocalReplayEntry::Playback { origin, receipt } => (
                        json!({"type":"message","role":"developer","content":[{"type":"input_text","text":format!("Voice-local media-consumption checkpoint [{}] (reported timing only; heard words and work completion are not inferred): {}",serde_json::to_string(origin).unwrap_or_default(),serde_json::to_string(receipt).unwrap_or_default())}]}),
                        None,
                    ),
                };
                bytes = bytes.saturating_add(message.to_string().len());
                if bytes > MAX_LOCAL_BYTES {
                    return Err(VoiceError::new(
                        VoiceErrorKind::Configuration,
                        "voice-local replay exceeds encoded context budget",
                    ));
                }
                if let Some((key, revision)) = user {
                    state.remember_user(key, revision, message);
                } else {
                    state.played.push(message);
                }
            }
        }
        Ok(state)
    }
    fn history(&self) -> Value {
        json!(
            self.user_history
                .iter()
                .map(|user| user.message.clone())
                .chain(
                    self.facts
                        .iter()
                        .filter(|fact| !self.source_revoked(fact.source.as_ref()))
                        .map(|fact| fact.message.clone())
                )
                .chain(self.played.iter().cloned())
                .collect::<Vec<_>>()
        )
    }
    fn context_history(&self, request: &VoiceOpenRequest) -> Value {
        let mut history = self.history().as_array().cloned().unwrap_or_default();
        if let Some(context) = &request.work_context {
            history.push(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":format!("Current application work context (supersedes earlier observed targets): {}",serde_json::to_string(context).unwrap_or_default())}]}));
        }
        Value::Array(history)
    }
    fn remember(list: &mut Vec<Value>, message: Value) {
        list.push(message);
        while list.len() > MAX_LOCAL_ENTRIES
            || list.iter().map(|m| m.to_string().len()).sum::<usize>() > MAX_LOCAL_BYTES
        {
            list.remove(0);
        }
    }
    fn remember_user(&mut self, fragment_key: String, revision: u64, message: Value) {
        if self
            .user_history
            .iter()
            .any(|user| user.fragment_key == fragment_key && user.revision >= revision)
        {
            return;
        }
        self.user_history
            .retain(|user| user.fragment_key != fragment_key);
        self.user_history.push(RememberedUser {
            fragment_key,
            revision,
            message,
        });
        while self.user_history.len() > MAX_LOCAL_ENTRIES
            || self
                .user_history
                .iter()
                .map(|user| user.message.to_string().len())
                .sum::<usize>()
                > MAX_LOCAL_BYTES
        {
            self.user_history.remove(0);
        }
    }
    fn source_revoked(&self, source: Option<&VoiceSpeechSourceRef>) -> bool {
        source.is_some_and(|source| {
            self.revoked_speech
                .get(&source.message_id)
                .is_some_and(|fence| source.revision <= fence.revision)
        })
    }
    fn remember_fact(&mut self, message: Value, source: Option<VoiceSpeechSourceRef>) {
        if self.source_revoked(source.as_ref()) {
            return;
        }
        self.facts.push(RememberedFact { message, source });
        while self.facts.len() > MAX_FACTS
            || self
                .facts
                .iter()
                .map(|fact| fact.message.to_string().len())
                .sum::<usize>()
                > MAX_FACT_BYTES
        {
            self.facts.remove(0);
        }
    }
    fn revoke_speech(&mut self, source: VoiceSpeechSourceRef) -> bool {
        let changed = self
            .revoked_speech
            .get(&source.message_id)
            .is_none_or(|prior| {
                source.revision > prior.revision || source.through_seq > prior.through_seq
            });
        let fence = self
            .revoked_speech
            .entry(source.message_id.clone())
            .or_insert(source.clone());
        fence.revision = fence.revision.max(source.revision);
        fence.through_seq = fence.through_seq.max(source.through_seq);
        let fences = &self.revoked_speech;
        let revoked = |source: Option<&VoiceSpeechSourceRef>| {
            source.is_some_and(|source| {
                fences
                    .get(&source.message_id)
                    .is_some_and(|fence| source.revision <= fence.revision)
            })
        };
        self.facts.retain(|fact| !revoked(fact.source.as_ref()));
        self.requested_speech
            .retain(|speech| !revoked(speech.source.as_ref()));
        self.pending_context
            .retain(|_, context| !revoked(context.source.as_ref()));
        self.force_rebuild |= changed;
        changed
    }
    fn remember_playback(&mut self, receipt: &PlaybackReceipt) {
        if receipt.consumed_us == 0
            || !matches!(
                receipt.state,
                DeliveryState::Played | DeliveryState::Interrupted
            )
        {
            return;
        }
        // Consumption timing is an endpoint fact; it does not establish which
        // words were heard or revive the old generated speech source body.
        Self::remember(
            &mut self.played,
            json!({"type":"message","role":"developer","content":[{"type":"input_text","text":format!("Application reported a media-consumption checkpoint (timing only; wording and work completion remain unknown): {}",serde_json::to_string(receipt).unwrap_or_default())}]}),
        );
    }
    fn input_ack(&mut self, wire_id: &str, muted: bool, io: &mut VoiceWorkerIo) -> bool {
        let Some(pending) = self
            .pending_input
            .as_ref()
            .filter(|pending| pending.wire_id == wire_id)
        else {
            return true;
        };
        if pending.muted != muted {
            let error =
                provider_error("provider input acknowledgment contradicts its exact request");
            self.input_failure = Some(error.clone());
            return emit(io, VoiceModelEvent::ControlRejected { error });
        }
        let pending = self
            .pending_input
            .take()
            .expect("matched input acknowledgment");
        io.media_rx.clear();
        self.muted = pending.muted;
        emit(
            io,
            VoiceModelEvent::InputMuteApplied {
                muted: pending.muted,
                request_id: pending.request_id,
            },
        )
    }
}

async fn run(
    mut provider: Provider,
    mut request: VoiceOpenRequest,
    socket: Socket,
    mut upstream_id: String,
    mut decoder: StepFunDecoder,
    mut io: VoiceWorkerIo,
    negotiation: VoiceNegotiation,
    limits: VoiceSessionLimits,
    mut state: State,
) {
    let redactor = provider.connection().auth.secret_redactor();
    let output_spec = negotiation.output_spec.clone();
    let (mut writer, mut reader, mut pump) = transport::pump(socket, output_spec.clone(), limits);
    let _ = io.event_tx.try_send(VoiceModelEvent::Ready { negotiation });
    let _ = io.event_tx.try_send(VoiceModelEvent::ContextOpened {
        context_window_ref: upstream_id.clone(),
        work_context: request.work_context.clone(),
    });
    let termination = loop {
        if state.transcripts.len() > 4096
            || state
                .transcripts
                .values()
                .map(|(_, text)| text.len())
                .sum::<usize>()
                > MAX_WIRE_BYTES * 4
            || state.revoked.len() > 4096
            || state.segments.len() > 4096
            || state.active_responses.len() > 4096
            || state.delegations.len() > 4096
            || state.known_calls.len() > 4096
            || state.returned_calls.len() > 4096
            || state.requested_speech.len() > 64
            || state.pending_response_creates > 64
            || state.pending_context.len() > 64
            || state.revoked_speech.len() > 4096
            || state.input_clears.len() > 64
            || state.input_presentations.len() > 64
            || state
                .input_presentations
                .values()
                .filter_map(|pending| pending.step_item.as_ref())
                .map(|item| item.to_string().len())
                .sum::<usize>()
                > MAX_WIRE_BYTES
        {
            break terminal(
                VoiceCloseReason::ModelLimit,
                false,
                "voice event identity retention limit reached",
            );
        }
        let input_deadline = state.pending_input.as_ref().map_or_else(
            || Instant::now() + Duration::from_secs(86400),
            |pending| pending.deadline,
        );
        let presentation_deadline = state
            .input_presentations
            .values()
            .map(|pending| pending.deadline)
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(86400));
        tokio::select! {biased;
            _=io.cancel.cancelled()=>{
                let prior=io.termination_tx.borrow().clone();
                let confirmed=close_upstream(&provider,&mut writer,&mut reader,limits).await;
                break VoiceTermination{reason:prior.as_ref().map_or(io.requested_close_reason(),|t|t.reason),finalization_confirmed:confirmed,message:prior.and_then(|t|t.message).or_else(||if confirmed{None}else{Some("provider finalization unconfirmed".into())})};
            }
            _=tokio::time::sleep_until(input_deadline),if state.pending_input.is_some()=>{
                let error=VoiceError::new(VoiceErrorKind::Deadline,"provider input control was not acknowledged before its deadline");
                let _=emit(&io,VoiceModelEvent::ControlRejected{error});
                break terminal(VoiceCloseReason::ProviderFailed,false,"voice input control acknowledgment deadline");
            }
            _=tokio::time::sleep_until(presentation_deadline),if !state.input_presentations.is_empty()=>{
                let _=emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::Deadline,"committed input source was not acknowledged by the voice provider")});
                break terminal(VoiceCloseReason::ProviderFailed,false,"voice input source acknowledgment deadline");
            }
            control=async{tokio::select!{biased;control=io.urgent_rx.recv()=>control,control=io.control_rx.recv()=>control}}=>{
                let Some(control)=control else{break terminal(VoiceCloseReason::AppShutdown,false,"voice control owner released");};
                match control{
                    VoiceControl::InterruptOutput{output_generation,played}=>{
                        if output_generation<=state.generation{continue;}
                        if let Some(receipt)=played{if receipt.activation_epoch==request.activation_epoch&&receipt.output_generation<=state.generation{state.remember_playback(&receipt);}}
                        let requires_rebuild=state.force_rebuild||matches!(provider,Provider::Live(_))||state.active_responses.is_empty()||state.pending_response_creates>0;
                        state.generation=output_generation;state.blocked=true;
                        state.revoked.extend(state.segments.iter().cloned());state.segments.clear();
                        state.requested_speech.clear();
                        io.media_rx.clear();
                        if provider.native(){
                            let instruction=json!({"type":"session.instructions.append","event_id":event_id(),"delegation_id":null,"content":"The application stopped playback. Stop the current speech; await a fresh media attachment."});
                            let _=transport::send_urgent_json(&mut writer,instruction,limits.write_timeout,MAX_WIRE_BYTES).await;
                            if !emit(&io,VoiceModelEvent::NativeAttachmentRequired{output_generation}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}
                        }else if requires_rebuild{
                            state.muted=true;
                            if !emit(&io,VoiceModelEvent::RelayRecovering{output_generation}){break terminal(VoiceCloseReason::Backlog,false,"voice recovery event queue exceeded budget");}
                            if state.pending_input.take().is_some(){
                                let _=emit(&io,VoiceModelEvent::ControlRejected{error:provider_error("output recovery invalidated an unconfirmed input control; start a fresh voice activation")});
                                break terminal(VoiceCloseReason::ProviderFailed,false,"input acknowledgment belongs to the superseded connection");
                            }
                            state.input_clears.clear();
                            state.input_presentations.clear();
                            // Live has no response fence. Step also cannot
                            // prove a boundary before response.created arrives.
                            // Rebuild those cases instead of accepting late
                            // old audio under the new output generation.
                            let _=close_upstream(&provider,&mut writer,&mut reader,limits).await;
                            pump.shutdown().await;
                            request.output_generation=output_generation;
                            decoder=StepFunDecoder::default();
                            let reopen=tokio::select!{biased;
                                _=io.cancel.cancelled()=>break terminal(io.requested_close_reason(),false,"voice recovery cancelled"),
                                result=tokio::time::timeout(CONNECT_TIMEOUT,connect_ready(&provider,&request,&mut decoder,state.context_history(&request)))=>result,
                            };
                            match reopen{
                                Ok(Ok((socket,id,_)))=>{let parts=transport::pump(socket,output_spec.clone(),limits);writer=parts.0;reader=parts.1;pump=parts.2;upstream_id=id;
                                    state.blocked=false;state.force_rebuild=false;state.cursor_us=0;state.delegations.clear();state.active_responses.clear();state.revoked.clear();state.known_calls.clear();state.returned_calls.clear();state.pending_response_creates=0;state.pending_context.clear();
                                    if !emit(&io,VoiceModelEvent::ContextOpened{context_window_ref:upstream_id.clone(),work_context:request.work_context.clone()}){break terminal(VoiceCloseReason::Backlog,false,"voice context event exceeded budget");}
                                    // Fresh transports default to accepting
                                    // input upstream. The local gate remains
                                    // closed until an explicit product unmute
                                    // receives its real new-connection ACK.
                                    io.media_rx.clear();
                                    if !emit(&io,VoiceModelEvent::RelayRecovered{output_generation}){break terminal(VoiceCloseReason::Backlog,false,"voice recovery event queue exceeded budget");}
                                    if !emit(&io,VoiceModelEvent::OutputBoundary{output_generation,attachment_id:None}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}
                                }
                                _=>break terminal(VoiceCloseReason::NetworkLost,false,"voice relay could not rebuild a safe output boundary"),
                            }
                        }else if transport::send_urgent_json(&mut writer,json!({"type":"response.cancel","event_id":event_id()}),limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){break terminal(VoiceCloseReason::NetworkLost,false,"voice interruption write failed");}
                    }
                    VoiceControl::MuteInput{muted}=>{
                        if state.pending_input.is_some(){let _=emit(&io,VoiceModelEvent::ControlRejected{error:provider_error("input control acknowledgment is still pending")});continue;}
                        state.muted=muted;io.media_rx.clear();
                        let id=event_id();let command=if matches!(provider,Provider::Live(_)){json!({"type":if muted{"session.input_audio.mute"}else{"session.input_audio.unmute"},"event_id":id})}else{
                            state.input_clears.push_back(PendingInputClear{wire_id:id.clone(),product:false});json!({"type":"input_audio_buffer.clear","event_id":id})};
                        if transport::send_urgent_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){break terminal(VoiceCloseReason::NetworkLost,false,"voice input control write failed");}
                    }
                    VoiceControl::SetInputMuted{muted,request_id}=>{
                        if request_id.trim().is_empty()||request_id.len()>256||state.pending_input.is_some(){
                            let _=emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::Configuration,"input control requires one complete bounded request identity")});continue;
                        }
                        // Hold capture admission closed even for unmute until
                        // the documented acknowledgment proves acceptance.
                        state.muted=true;io.media_rx.clear();let id=event_id();
                        state.pending_input=Some(PendingInputControl{request_id,wire_id:id.clone(),muted,deadline:Instant::now()+INPUT_ACK_TIMEOUT});
                        let command=if matches!(provider,Provider::Live(_)){json!({"type":if muted{"session.input_audio.mute"}else{"session.input_audio.unmute"},"event_id":id})}else{
                            // Step has a local relay input gate, not an
                            // upstream mute RPC. Its clear ACK proves the old
                            // input buffer is cleared; no mute wire is invented.
                            state.input_clears.push_back(PendingInputClear{wire_id:id.clone(),product:true});json!({"type":"input_audio_buffer.clear","event_id":id})};
                        if transport::send_urgent_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){break terminal(VoiceCloseReason::NetworkLost,false,"voice input control write failed");}
                    }
                    VoiceControl::PresentInputSource{fragment}=>{
                        let current=state.transcripts.get(&fragment.fragment_id);
                        if fragment.speaker!=VoiceSpeaker::User||fragment.commit!=TranscriptCommit::Committed||fragment.fragment_id.len()>1024||fragment.text.trim().is_empty()||fragment.text.len()>32*1024
                            ||!fragment.fragment_id.starts_with(&format!("{upstream_id}:user:"))||!current.is_some_and(|(revision,text)|*revision==fragment.revision&&text==&fragment.text){
                            let _=emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::StaleEpoch,"input source does not match a current committed provider fragment")});continue;
                        }
                        let reference=json!({"fragment_id":fragment.fragment_id,"revision":fragment.revision});
                        let guidance=if matches!(provider,Provider::StepFun(_)){"Every mutating nomi_work call must cite this exact source_ref. Never substitute the newest fragment for a different utterance."}
                            else{"This identifies a committed local transcript fragment for application client delegation; it is not a typed tool call or a complete conversational turn."};
                        let content=format!("Application input source (silent reference data, not a new user message, canonical receipt or permission): {}. {} If work context changed and the input time domain is unknown, clarify the exact task; ASR delivery time does not identify the user's target. Do not speak the source identifiers. Transcript data: {}",
                            reference,guidance,serde_json::to_string(&fragment).unwrap_or_default());
                        let commands=match &provider{
                            Provider::StepFun(_)=>{let id=event_id();let item=json!({"type":"message","role":"system","content":[{"type":"input_text","text":content}]});
                                state.input_presentations.insert(id.clone(),PendingInputPresentation{pending:BTreeSet::from([id.clone()]),step_item:Some(item.clone()),deadline:Instant::now()+INPUT_ACK_TIMEOUT});
                                vec![json!({"type":"conversation.item.create","event_id":id,"item":item})]},
                            Provider::Live(_)=>{let commands=bounded_chunks(&content).into_iter().map(|chunk|openai_live_wire::context_append(&chunk,None,false,&event_id())).collect::<Vec<_>>();
                                let ids=commands.iter().filter_map(|command|command.get("event_id").and_then(Value::as_str).map(str::to_owned)).collect::<BTreeSet<_>>();
                                if let Some(last)=commands.last().and_then(|command|command.get("event_id")).and_then(Value::as_str){state.input_presentations.insert(last.into(),PendingInputPresentation{pending:ids,step_item:None,deadline:Instant::now()+INPUT_ACK_TIMEOUT});}commands}
                        };
                        // Never restore an old session's ephemeral source ID
                        // as fresh input or as a canonical authorization.
                        let mut failed=false;for command in commands{if transport::send_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){failed=true;break;}}
                        if failed{break terminal(VoiceCloseReason::NetworkLost,false,"voice input source presentation write failed");}
                    }
                    VoiceControl::RejectWorkTrigger{upstream_trigger_id,reason}=>{
                        let reason=redactor.redact(&reason);
                        let content=format!("The application did not admit the requested work: {reason}. Clarify the user's complete intent before requesting new work.");
                        let commands=match &provider{
                            Provider::StepFun(_)=>{
                                let item=if state.known_calls.contains(&upstream_trigger_id)&&state.returned_calls.insert(upstream_trigger_id.clone()){
                                    json!({"type":"function_call_output","call_id":upstream_trigger_id,"output":json!({"status":"rejected","reason":reason}).to_string()})
                                }else{json!({"type":"message","role":"system","content":[{"type":"input_text","text":content}]})};
                                vec![json!({"type":"conversation.item.create","event_id":event_id(),"item":item}),json!({"type":"response.create","event_id":event_id()})]
                            },
                            Provider::Live(_)=>{
                                let delegation=if state.delegations.contains(&upstream_trigger_id){Some(upstream_trigger_id.as_str())}else{None};
                                bounded_chunks(&content).into_iter().map(|chunk|openai_live_wire::context_append(&chunk,delegation,true,&event_id())).collect()
                            }
                        };
                        let mut failed=false;for command in commands{if command["type"]=="response.create"{state.pending_response_creates+=1;}if transport::send_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){failed=true;break;}}
                        if failed{break terminal(VoiceCloseReason::NetworkLost,false,"voice rejection delivery failed");}
                    }
                    VoiceControl::InjectFact{fact}=>{
                        if fact.output_generation.is_some_and(|generation|generation!=state.generation){
                            if !emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::StaleEpoch,"speech fact was revoked by a newer output generation")}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}
                            continue;
                        }
                        if let Some(source)=&fact.speech_source{
                            let error=source.validate().err().map(|message|VoiceError::new(VoiceErrorKind::Configuration,message))
                                .or_else(||state.source_revoked(Some(source)).then(||VoiceError::new(VoiceErrorKind::StaleEpoch,"canonical speech source was revoked; await a verified newer revision")));
                            if let Some(error)=error{if !emit(&io,VoiceModelEvent::ControlRejected{error}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}continue;}
                        }
                        let mut fact=fact;
                        if let Some(context)=&fact.work_context{
                            request.work_context=Some(context.clone());
                            fact.content.push_str(&format!("\nApplication exact work context: {}",serde_json::to_string(context).unwrap_or_default()));
                        }
                        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":fact.content}]}),fact.speech_source.clone());
                        let commands=match &provider{
                            Provider::StepFun(_)=>{
                                if fact.speak{state.requested_speech.push_back(RequestedSpeech{correlation_id:fact.correlation_id.clone(),source:fact.speech_source.clone()});}
                                let pending=fact.upstream_trigger_id.as_ref().filter(|id|state.known_calls.contains(*id)&&!state.returned_calls.contains(*id));
                                let item=if let Some(id)=pending{state.returned_calls.insert(id.clone());json!({"type":"function_call_output","call_id":id,"output":fact.content})}else{json!({"type":"message","role":"system","content":[{"type":"input_text","text":fact.content}]})};
                                let mut commands=vec![json!({"type":"conversation.item.create","event_id":event_id(),"item":item})];
                                if fact.speak{commands.push(json!({"type":"response.create","event_id":event_id()}));}commands
                            }
                            Provider::Live(_)=>{
                                let delegation=fact.upstream_trigger_id.as_deref().filter(|id|state.delegations.contains(*id));
                                let commands=bounded_chunks(&fact.content).into_iter().map(|content|openai_live_wire::context_append(&content,delegation,fact.speak,&event_id())).collect::<Vec<_>>();
                                if let (Some(context),Some(last))=(&fact.work_context,commands.last()){
                                    if let Some(id)=last.get("event_id").and_then(Value::as_str){
                                        let pending=commands.iter().filter_map(|command|command.get("event_id").and_then(Value::as_str).map(str::to_owned)).collect();
                                        state.pending_context.insert(id.into(),PendingContext{context:context.clone(),receipt_id:fact.canonical_receipt_id.clone(),pending,range:None,source:fact.speech_source.clone()});
                                    }
                                }commands
                            }
                        };
                        let mut failed=false;for command in commands{if command["type"]=="response.create"{state.pending_response_creates+=1;}if transport::send_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){failed=true;break;}}
                        if failed{break terminal(VoiceCloseReason::NetworkLost,false,"voice fact injection write failed");}
                    }
                    VoiceControl::RevokeSpeech{source,output_generation}=>{
                        if let Err(message)=source.validate(){if !emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::Configuration,message)}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}continue;}
                        let changed=state.revoke_speech(source.clone());
                        // Keep the following Interrupt at the same generation
                        // actionable. A history-only same-generation revocation
                        // does not block unrelated, still-valid output.
                        if output_generation>state.generation{state.blocked=true;}
                        if !changed{continue;}
                        let content=format!("The application revoked previously provided speech from message {} through revision {} and canonical sequence {}. Do not replay or rely on that revoked wording. Await verified replacement facts; media checkpoints do not establish heard words or work completion.",source.message_id,source.revision,source.through_seq);
                        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":content}]}),None);
                        let commands=match &provider{
                            Provider::StepFun(_)=>vec![json!({"type":"conversation.item.create","event_id":event_id(),"item":{"type":"message","role":"system","content":[{"type":"input_text","text":content}]}})],
                            Provider::Live(_)=>bounded_chunks(&content).into_iter().map(|chunk|json!({"type":"session.instructions.append","event_id":event_id(),"delegation_id":null,"content":chunk})).collect(),
                        };
                        let mut failed=false;for command in commands{if transport::send_urgent_json(&mut writer,command,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){failed=true;break;}}
                        if failed{break terminal(VoiceCloseReason::NetworkLost,false,"speech revocation context delivery failed");}
                    }
                    VoiceControl::UpdateConfiguration{patch}=>{
                        if let Provider::StepFun(p)=&mut provider{
                            match patch.instructions{VoicePatch::Keep=>{},VoicePatch::Clear=>request.instructions.clear(),VoicePatch::Set(v)=>request.instructions=v}
                            match patch.tools{VoicePatch::Keep=>{},VoicePatch::Clear=>request.tools.clear(),VoicePatch::Set(v)=>request.tools=v}
                            let update=p.config.session_update(&request.instructions,stepfun_tools(&request.tools),&event_id());
                            if transport::send_json(&mut writer,update,limits.write_timeout,MAX_WIRE_BYTES).await.is_err(){break terminal(VoiceCloseReason::NetworkLost,false,"voice configuration write failed");}
                        }else if !matches!(patch.instructions,VoicePatch::Keep)||!matches!(patch.tools,VoicePatch::Keep){
                            if !emit(&io,VoiceModelEvent::ControlRejected{error:VoiceError::new(VoiceErrorKind::Unsupported,"GPT-Live startup instructions and client delegation tools require a new session")}){break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}
                        }
                    }
                    VoiceControl::Playback{receipt}=>{
                        if receipt.activation_epoch==request.activation_epoch&&receipt.output_generation<=state.generation{state.remember_playback(&receipt);}
                    },
                    VoiceControl::Close{reason}=>{let confirmed=close_upstream(&provider,&mut writer,&mut reader,limits).await;break VoiceTermination{reason,finalization_confirmed:confirmed,message:None};}
                }
            }
            incoming=async{tokio::select!{
                frame=io.media_rx.recv(),if !provider.native()=>DataIngress::Frame(frame),
                incoming=reader.next()=>DataIngress::Wire(incoming),
            }}=>{match incoming{
            DataIngress::Frame(frame)=>{
                let Some(frame)=frame else{continue;};
                if state.muted||frame.activation_epoch!=request.activation_epoch{continue;}
                let command=json!({"type":if matches!(provider,Provider::StepFun(_)){"input_audio_buffer.append"}else{"session.input_audio.append"},"event_id":event_id(),"audio":base64::engine::general_purpose::STANDARD.encode(frame.payload)});
                let timeout=limits.write_timeout.min(Duration::from_micros(u64::from(media_spec(provider.format(),false).max_frame_age_us)));
                if let Err(error)=transport::send_media_json(&mut writer,command,timeout,MAX_WIRE_BYTES).await{break terminal(if error==transport::PumpError::Backlog{VoiceCloseReason::Backlog}else{VoiceCloseReason::NetworkLost},false,"voice audio write exceeded its media deadline");}
            }
            DataIngress::Wire(incoming)=>{
                let raw=match incoming{
                    Some(Ok(Message::Text(text)))=>match serde_json::from_str::<Value>(&text){Ok(value)=>value,Err(_)=>break terminal(VoiceCloseReason::ProviderFailed,false,"voice provider sent invalid JSON")},
                    Some(Ok(Message::Ping(payload)))=>{if writer.send(Message::Pong(payload)).await.is_err(){break terminal(VoiceCloseReason::NetworkLost,false,"voice pong failed");}continue;}
                    Some(Ok(Message::Pong(_)|Message::Frame(_)))=>continue,
                    Some(Ok(Message::Close(_)))|None=>break terminal(VoiceCloseReason::NetworkLost,false,"voice transport ended before confirmed finalization"),
                    Some(Err(transport::PumpError::Backlog))=>break terminal(VoiceCloseReason::Backlog,false,"voice upstream receive exceeded media duration/age budget"),
                    _=>break terminal(VoiceCloseReason::NetworkLost,false,"voice transport failed"),
                };
                let events=match decode(&provider,&mut decoder,&raw){Ok(events)=>events,Err(message)=>{
                    let _=emit(&io,VoiceModelEvent::ControlRejected{error:provider_error(message)});continue;
                }};
                let mut failed=false;
                for event in events{
                    if let WireEvent::Closed{reason}=event{let reason=match reason.as_str(){"expired"=>VoiceCloseReason::ModelLimit,"content"=>VoiceCloseReason::ProviderFailed,"connection_lost"=>VoiceCloseReason::NetworkLost,_=>VoiceCloseReason::UserEnded};pump.shutdown().await;finish(&io,terminal(reason,true,"provider finalized session"));return;}
                    if !project(&provider,&request,&upstream_id,&mut state,event,&mut io,&redactor){failed=true;break;}
                }
                if let Some(error)=state.input_failure.take(){break terminal(VoiceCloseReason::ProviderFailed,false,&error.message);}
                if failed{break terminal(VoiceCloseReason::Backlog,false,"voice event queue exceeded budget");}
            }
            }}
        }
    };
    let _ = tokio::time::timeout(limits.write_timeout, writer.send(Message::Close(None))).await;
    pump.shutdown().await;
    finish(&io, termination);
}

fn project(
    provider: &Provider,
    request: &VoiceOpenRequest,
    upstream_id: &str,
    state: &mut State,
    event: WireEvent,
    io: &mut VoiceWorkerIo,
    redactor: &nomifun_net::secret_redaction::SecretRedactor,
) -> bool {
    match event {
        WireEvent::Transcript {
            user,
            response_id,
            fragment_id,
            text,
            complete,
            append,
            start_ms,
            end_ms,
        } => {
            if !user
                && (state.blocked
                    || response_id
                        .as_ref()
                        .is_some_and(|id| state.revoked.contains(id)))
            {
                return true;
            }
            let key = format!(
                "{}:{}:{}",
                upstream_id,
                if user { "user" } else { "assistant" },
                fragment_id
            );
            let entry = state
                .transcripts
                .entry(key.clone())
                .or_insert((0, String::new()));
            entry.0 += 1;
            if append {
                entry.1.push_str(&text);
            } else {
                entry.1 = text;
            }
            if entry.1.len() > MAX_WIRE_BYTES {
                return false;
            }
            let (revision, committed_text) = (entry.0, entry.1.clone());
            if user && complete {
                state.remember_user(key.clone(),revision,
                    json!({"type":"message","role":"user","content":[{"type":"input_text","text":committed_text}]}));
            }
            emit(
                io,
                VoiceModelEvent::Transcript {
                    fragment: TranscriptFragment {
                        speaker: if user {
                            VoiceSpeaker::User
                        } else {
                            VoiceSpeaker::Assistant
                        },
                        fragment_id: key,
                        revision,
                        commit: if complete {
                            TranscriptCommit::Committed
                        } else {
                            TranscriptCommit::Tentative
                        },
                        text: committed_text,
                        media_range: start_ms.zip(end_ms).map(|(s, e)| MediaRange {
                            start_us: s * 1000,
                            end_us: e * 1000,
                        }),
                    },
                },
            )
        }
        WireEvent::OutputStarted { response_id } => {
            state.pending_response_creates = state.pending_response_creates.saturating_sub(1);
            state.active_responses.insert(response_id.clone());
            if state.blocked {
                state.revoked.insert(response_id);
                return true;
            }
            let correlation = if matches!(provider, Provider::StepFun(_)) {
                state
                    .requested_speech
                    .pop_front()
                    .map(|speech| speech.correlation_id)
            } else {
                None
            };
            ensure_segment(state, &response_id, io, correlation)
        }
        WireEvent::Audio {
            response_id,
            item_id,
            bytes,
            ..
        } => {
            let segment = response_id
                .or(item_id)
                .unwrap_or_else(|| format!("{upstream_id}:{}", state.generation));
            if state.blocked || state.revoked.contains(&segment) {
                return true;
            }
            if !ensure_segment(state, &segment, io, None) {
                return false;
            }
            let spec = media_spec(provider.format(), true);
            let chunk_bytes = spec.max_frame_bytes as usize;
            for chunk in bytes.chunks(chunk_bytes) {
                let duration = match spec.format.duration_us(chunk.len()) {
                    Ok(duration) => duration as u32,
                    Err(_) => return false,
                };
                if duration == 0 {
                    continue;
                }
                state.sequence += 1;
                let frame = AudioFrame {
                    activation_epoch: request.activation_epoch,
                    output_generation: state.generation,
                    sequence: state.sequence,
                    timestamp: state.cursor_us,
                    duration_us: duration,
                    format: spec.format.clone(),
                    payload: chunk.to_vec(),
                };
                state.cursor_us += u64::from(duration);
                if !emit(
                    io,
                    VoiceModelEvent::Audio {
                        segment_id: segment.clone(),
                        frame,
                    },
                ) {
                    return false;
                }
            }
            true
        }
        WireEvent::AudioDone { response_id, .. } => response_id.is_none_or(|id| {
            if !state.active_responses.remove(&id) {
                return true;
            }
            let finished = state.revoked.contains(&id)
                || emit(io, VoiceModelEvent::OutputFinished { response_id: id });
            if state.blocked && state.active_responses.is_empty() {
                state.blocked = false;
                return finished
                    && emit(
                        io,
                        VoiceModelEvent::OutputBoundary {
                            output_generation: state.generation,
                            attachment_id: None,
                        },
                    );
            }
            finished
        }),
        WireEvent::OutputInterrupted { response_id } => {
            if let Some(id) = &response_id {
                state.revoked.insert(id.clone());
                state.active_responses.remove(id);
            }
            let interrupted = emit(io, VoiceModelEvent::OutputInterrupted { response_id });
            if !state.active_responses.is_empty() {
                return interrupted;
            }
            state.blocked = false;
            interrupted
                && emit(
                    io,
                    VoiceModelEvent::OutputBoundary {
                        output_generation: state.generation,
                        attachment_id: None,
                    },
                )
        }
        WireEvent::ToolCall {
            call_id,
            response_id,
            name,
            arguments,
        } => {
            if state.blocked
                || response_id
                    .as_ref()
                    .is_some_and(|id| state.revoked.contains(id))
                || (state.generation != request.output_generation && response_id.is_none())
            {
                return true;
            }
            state.known_calls.insert(call_id.clone());
            emit(
                io,
                VoiceModelEvent::WorkTrigger {
                    trigger: WorkTrigger::TypedToolCall {
                        upstream_trigger_id: call_id,
                        name,
                        arguments,
                        // Step's function response ID does not establish a
                        // source transcript item; never substitute the latest
                        // committed user fragment as proof of that relation.
                        transcript_window_ref: None,
                    },
                },
            )
        }
        WireEvent::Delegation { id, offset_ms } => {
            if !state.delegations.insert(id.clone()) {
                return true;
            }
            emit(
                io,
                VoiceModelEvent::WorkTrigger {
                    trigger: WorkTrigger::DelegationTrigger {
                        upstream_trigger_id: id,
                        target: "client".into(),
                        offset_ms,
                        context_window_ref: Some(upstream_id.into()),
                    },
                },
            )
        }
        WireEvent::Ready { .. } | WireEvent::ConfigurationUpdated => {
            emit(io, VoiceModelEvent::ConfigurationApplied)
        }
        WireEvent::Error { code, message } => {
            // An upstream error cannot certify that every piece of a context
            // batch was accepted. Keep the previously proven target until a
            // fresh fully acknowledged fact or real context rebuild.
            state.pending_context.clear();
            let kind = match code.as_deref() {
                Some("rate_limit_exceeded" | "insufficient_quota") => VoiceErrorKind::Quota,
                Some("invalid_api_key" | "authentication_error") => VoiceErrorKind::Authentication,
                _ => VoiceErrorKind::Provider,
            };
            let error = VoiceError::new(kind, redactor.redact(&message));
            if state.pending_input.is_some() || !state.input_presentations.is_empty() {
                state.input_failure = Some(error.clone());
            }
            emit(
                io,
                if matches!(kind, VoiceErrorKind::Authentication | VoiceErrorKind::Quota) {
                    VoiceModelEvent::Error { error }
                } else {
                    VoiceModelEvent::ControlRejected { error }
                },
            )
        }
        WireEvent::InputMuted {
            client_event_id,
            muted,
        } => state.input_ack(&client_event_id, muted, io),
        WireEvent::InputCleared { client_event_id } => {
            let Some(expected) = state.input_clears.front() else {
                return true;
            };
            if client_event_id
                .as_ref()
                .is_some_and(|id| id != &expected.wire_id)
            {
                return true;
            }
            let expected = state
                .input_clears
                .pop_front()
                .expect("queued clear acknowledgment");
            if expected.product {
                let muted = state
                    .pending_input
                    .as_ref()
                    .filter(|pending| pending.wire_id == expected.wire_id)
                    .map(|pending| pending.muted);
                muted.is_none_or(|muted| state.input_ack(&expected.wire_id, muted, io))
            } else {
                true
            }
        }
        WireEvent::Usage { .. } | WireEvent::Speech { .. } | WireEvent::Closed { .. } => true,
        WireEvent::MessageAccepted { item } => {
            state.input_presentations.retain(|_, pending| {
                !pending.step_item.as_ref().is_some_and(|expected| {
                    item.get("type") == expected.get("type")
                        && item.get("role") == expected.get("role")
                        && item.get("content") == expected.get("content")
                })
            });
            true
        }
        WireEvent::ContextAccepted {
            client_event_id,
            start_ms,
            end_ms,
        } => {
            let Some(client_id) = client_event_id else {
                return true;
            };
            state.input_presentations.retain(|_, pending| {
                pending.pending.remove(&client_id);
                !pending.pending.is_empty()
            });
            let Some(key) = state
                .pending_context
                .iter()
                .find(|(_, group)| group.pending.contains(&client_id))
                .map(|(key, _)| key.clone())
            else {
                return true;
            };
            if let Some((start_us, end_us)) = start_ms
                .and_then(|n| n.checked_mul(1000))
                .zip(end_ms.and_then(|n| n.checked_mul(1000)))
                .filter(|(start, end)| end >= start)
            {
                let group = state
                    .pending_context
                    .get_mut(&key)
                    .expect("matched pending context");
                group.pending.remove(&client_id);
                group.range = Some(match &group.range {
                    Some(prior) => MediaRange {
                        start_us: prior.start_us.min(start_us),
                        end_us: prior.end_us.max(end_us),
                    },
                    None => MediaRange { start_us, end_us },
                });
                if !group.pending.is_empty() {
                    return true;
                }
                let group = state
                    .pending_context
                    .remove(&key)
                    .expect("fully acknowledged context");
                emit(
                    io,
                    VoiceModelEvent::WorkContextCheckpoint {
                        context_window_ref: upstream_id.into(),
                        media_range: group.range.expect("acknowledged timeline"),
                        target: group.context.target,
                        canonical_receipt_id: group.receipt_id,
                    },
                )
            } else {
                state.pending_context.remove(&key);
                emit(
                    io,
                    VoiceModelEvent::ControlRejected {
                        error: VoiceError::new(
                            VoiceErrorKind::Unsupported,
                            "context acknowledgment did not prove a session timeline checkpoint; clarify the work target",
                        ),
                    },
                )
            }
        }
    }
}

fn ensure_segment(
    state: &mut State,
    id: &str,
    io: &VoiceWorkerIo,
    correlation_id: Option<String>,
) -> bool {
    if !state.segments.insert(id.into()) {
        return true;
    }
    emit(
        io,
        VoiceModelEvent::OutputStarted {
            segment: OutputSegment {
                response_id: id.into(),
                segment_id: id.into(),
                revision: 1,
                output_generation: state.generation,
                text: None,
                // Associates the application's requested speech. This does
                // not prove the model's wording, playback or work completion.
                correlation_id,
            },
        },
    )
}
fn emit(io: &VoiceWorkerIo, event: VoiceModelEvent) -> bool {
    io.event_tx.try_send(event).is_ok()
}
fn terminal(reason: VoiceCloseReason, confirmed: bool, message: &str) -> VoiceTermination {
    VoiceTermination {
        reason,
        finalization_confirmed: confirmed,
        message: Some(message.into()),
    }
}
fn finish(io: &VoiceWorkerIo, termination: VoiceTermination) {
    io.terminate(termination.clone());
    let _ = emit(io, VoiceModelEvent::Closed { termination });
}

async fn close_upstream(
    provider: &Provider,
    writer: &mut transport::Writer,
    reader: &mut transport::Reader,
    limits: VoiceSessionLimits,
) -> bool {
    let close = async {
        if matches!(provider, Provider::Live(_)) {
            transport::send_urgent_json(
                writer,
                json!({"type":"session.close","event_id":event_id()}),
                limits.write_timeout,
                MAX_WIRE_BYTES,
            )
            .await
            .ok()?;
        } else {
            writer.send(Message::Close(None)).await.ok()?;
        }
        loop {
            match reader.next().await {
                Some(Ok(Message::Text(text))) if matches!(provider, Provider::Live(_)) => {
                    let value: Value = serde_json::from_str(&text).ok()?;
                    if string(&value, "type").as_deref() == Some("session.closed") {
                        return Some(());
                    }
                }
                Some(Ok(Message::Close(_))) if matches!(provider, Provider::StepFun(_)) => {
                    return Some(());
                }
                Some(Ok(Message::Ping(payload))) => {
                    writer.send(Message::Pong(payload)).await.ok()?;
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Text(_))) if matches!(provider, Provider::StepFun(_)) => {}
                _ => return None,
            }
        }
    };
    tokio::time::timeout(
        limits
            .close_timeout
            .saturating_sub(Duration::from_millis(100)),
        close,
    )
    .await
    .ok()
    .flatten()
    .is_some()
}

fn bounded_chunks(value: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for character in value.chars() {
        if chunk.len() + character.len_utf8() > 500 {
            chunks.push(std::mem::take(&mut chunk));
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_replay_is_scoped_silent_data_and_user_revision_replaces_rebuild_cache() {
        let mut request =
            super::super::contract_tests::request("gpt-live-1", VoiceTransportPreference::Relay);
        request.instructions = "static voice policy".into();
        request.initial_local_replay = Some(super::super::local_replay_contract_tests::replay(
            request.replay_scope.clone(),
        ));
        let mut state = State::from_initial(&request).unwrap();
        let restored = state.context_history(&request).to_string();
        assert!(restored.contains("PRIOR_COMMITTED_USER_INPUT"));
        assert!(restored.contains("prior-segment") && restored.contains("unknown"));
        assert!(!restored.contains("canonical_receipt_id"));
        assert_eq!(startup_instructions(&request), "static voice policy");
        assert!(
            state.requested_speech.is_empty()
                && state.transcripts.is_empty()
                && state.delegations.is_empty()
        );
        state.remember_user("current-fragment".into(),1,json!({"type":"message","role":"user","content":[{"type":"input_text","text":"STALE_COMMITTED_TEXT"}]}));
        state.remember_user("current-fragment".into(),2,json!({"type":"message","role":"user","content":[{"type":"input_text","text":"CORRECTED_COMMITTED_TEXT"}]}));
        state.remember_user("current-fragment".into(),1,json!({"type":"message","role":"user","content":[{"type":"input_text","text":"STALE_LATE_TEXT"}]}));
        state.revoke_speech(VoiceSpeechSourceRef {
            message_id: "withdrawn-canonical-message".into(),
            revision: 2,
            through_seq: 10,
        });
        let rebuilt = state.context_history(&request).to_string();
        assert!(!rebuilt.contains("STALE_COMMITTED_TEXT") && !rebuilt.contains("STALE_LATE_TEXT"));
        assert!(
            rebuilt.contains("CORRECTED_COMMITTED_TEXT") && rebuilt.contains("prior-segment"),
            "canonical source withdrawal cannot erase observed consumption timing or local user intent"
        );
        request
            .initial_local_replay
            .as_mut()
            .unwrap()
            .scope
            .context_floor += 1;
        assert_eq!(
            State::from_initial(&request).err().unwrap().kind,
            VoiceErrorKind::StaleBinding
        );
    }
    #[test]
    fn initial_facts_are_source_tagged_not_in_instructions_and_revoke_before_rebuild() {
        let mut request =
            super::super::contract_tests::request("gpt-live-1", VoiceTransportPreference::Relay);
        request.instructions = "static agent and voice policy".into();
        request.initial_facts = vec![VerifiedVoiceFact {
            correlation_id: "initial".into(),
            upstream_trigger_id: None,
            canonical_receipt_id: "canonical-event-10".into(),
            content: "INITIAL_ASSISTANT_BODY".into(),
            speak: true,
            output_generation: Some(1),
            work_context: None,
            speech_source: Some(VoiceSpeechSourceRef {
                message_id: "initial-message".into(),
                revision: 1,
                through_seq: 10,
            }),
        }];
        let mut state = State::from_initial(&request).unwrap();
        assert!(!startup_instructions(&request).contains("INITIAL_ASSISTANT_BODY"));
        assert!(
            state
                .context_history(&request)
                .to_string()
                .contains("INITIAL_ASSISTANT_BODY")
        );
        assert!(
            state.requested_speech.is_empty(),
            "initial data is silent context, never queued speech"
        );
        state.revoke_speech(VoiceSpeechSourceRef {
            message_id: "initial-message".into(),
            revision: 2,
            through_seq: 20,
        });
        assert!(
            !state
                .context_history(&request)
                .to_string()
                .contains("INITIAL_ASSISTANT_BODY")
        );
        assert_eq!(
            startup_instructions(&request),
            "static agent and voice policy"
        );
        request.initial_facts[0].content = "x".repeat(MAX_FACT_BYTES);
        assert!(
            State::from_initial(&request).is_err(),
            "oversize initialization cannot silently trim admitted data"
        );
    }
    #[test]
    fn revoked_source_filters_rebuild_body_correlations_and_pending_context_but_keeps_playback_timing()
     {
        let source = |id: &str, revision, through_seq| VoiceSpeechSourceRef {
            message_id: id.into(),
            revision,
            through_seq,
        };
        let old = source("assistant-a", 1, 10);
        let current = source("assistant-a", 3, 30);
        let other = source("assistant-b", 1, 12);
        let mut state = State::new(1);
        state.remember_user("committed-fragment".into(),1,
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":"committed user input"}]}),
        );
        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"REVOKED_ASSISTANT_BODY"}]}),Some(old.clone()));
        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"CURRENT_ASSISTANT_BODY"}]}),Some(current.clone()));
        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"canonical effect remains executed"}]}),None);
        state.requested_speech.push_back(RequestedSpeech {
            correlation_id: "same-operation".into(),
            source: Some(old.clone()),
        });
        state.requested_speech.push_back(RequestedSpeech {
            correlation_id: "same-operation".into(),
            source: Some(current.clone()),
        });
        state.requested_speech.push_back(RequestedSpeech {
            correlation_id: "other-operation".into(),
            source: Some(other.clone()),
        });
        for (id, source) in [
            ("old-context", old.clone()),
            ("current-context", current.clone()),
        ] {
            state.pending_context.insert(
                id.into(),
                PendingContext {
                    context: VoiceWorkContextFact { target: None },
                    receipt_id: id.into(),
                    pending: BTreeSet::from([id.into()]),
                    range: None,
                    source: Some(source),
                },
            );
        }
        state.remember_playback(&PlaybackReceipt {
            activation_epoch: 1,
            output_generation: 1,
            segment_id: "old-output".into(),
            revision: 1,
            state: DeliveryState::Played,
            consumed_us: 20_000,
            uncertain_tail_us: 10_000,
            precision: PlaybackPrecision::Unknown,
        });
        assert!(state.revoke_speech(source("assistant-a", 2, 20)));
        state.remember_fact(json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"LATE_REVOKED_ASSISTANT_BODY"}]}),Some(old.clone()));
        let history = state.history().to_string();
        assert!(!history.contains("REVOKED_ASSISTANT_BODY"));
        assert!(
            history.contains("CURRENT_ASSISTANT_BODY")
                && history.contains("committed user input")
                && history.contains("canonical effect remains executed")
        );
        assert!(history.contains("media-consumption checkpoint") && history.contains("unknown"));
        assert!(
            !history.contains("speech_source") && !history.contains("correlation_id"),
            "private cache metadata is not a vendor message"
        );
        assert_eq!(state.requested_speech.len(), 2);
        assert_eq!(
            state.requested_speech[0].source,
            Some(current.clone()),
            "identical work correlation must retain the valid newer source only"
        );
        assert!(
            !state.pending_context.contains_key("old-context")
                && state.pending_context.contains_key("current-context")
        );
        assert!(state.source_revoked(Some(&old)));
        assert!(!state.source_revoked(Some(&current)) && !state.source_revoked(Some(&other)));
        state.revoke_speech(source("assistant-a", 1, 40));
        assert!(
            !state.source_revoked(Some(&current)),
            "a newer revision is not revoked by a later observation of an older source"
        );
        assert!(state.force_rebuild);
        assert_eq!(
            state.generation, 1,
            "source fence must not swallow the following same-target-generation Interrupt"
        );
    }
    #[tokio::test]
    async fn context_checkpoint_waits_for_every_piece_and_provider_error_revokes_pending_proof() {
        let connection = super::super::contract_tests::connection(1);
        let redactor = connection.auth.secret_redactor();
        let provider = Provider::Live(
            OpenAiLiveVoiceModel::new(
                connection,
                "/live/sessions".into(),
                json!({}),
                false,
                reqwest::Client::new(),
                "gpt-live-1".into(),
            )
            .unwrap(),
        );
        let request =
            super::super::contract_tests::request("gpt-live-1", VoiceTransportPreference::Relay);
        let (mut product, mut worker) = VoiceModelSession::channels(
            None,
            VoiceSessionLimits::default(),
            CancellationToken::new(),
        )
        .unwrap();
        let mut state = State::new(1);
        state.pending_context.insert(
            "batch".into(),
            PendingContext {
                context: VoiceWorkContextFact { target: None },
                receipt_id: "canonical-fact".into(),
                pending: BTreeSet::from(["first".into(), "last".into()]),
                range: None,
                source: None,
            },
        );
        assert!(project(
            &provider,
            &request,
            "live-session",
            &mut state,
            WireEvent::ContextAccepted {
                client_event_id: Some("last".into()),
                start_ms: Some(1300),
                end_ms: Some(1400)
            },
            &mut worker,
            &redactor
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), product.event_rx.recv())
                .await
                .is_err(),
            "the last chunk alone proves no full context update"
        );
        assert!(project(
            &provider,
            &request,
            "live-session",
            &mut state,
            WireEvent::ContextAccepted {
                client_event_id: Some("first".into()),
                start_ms: Some(1200),
                end_ms: Some(1300)
            },
            &mut worker,
            &redactor
        ));
        assert!(
            matches!(product.event_rx.recv().await,Some(VoiceModelEvent::WorkContextCheckpoint{media_range,canonical_receipt_id,..}) if media_range==(MediaRange{start_us:1_200_000,end_us:1_400_000})&&canonical_receipt_id=="canonical-fact")
        );
        state.pending_context.insert(
            "failed".into(),
            PendingContext {
                context: VoiceWorkContextFact { target: None },
                receipt_id: "unaccepted-fact".into(),
                pending: BTreeSet::from(["unaccepted".into()]),
                range: None,
                source: None,
            },
        );
        assert!(project(
            &provider,
            &request,
            "live-session",
            &mut state,
            WireEvent::Error {
                code: Some("invalid_event".into()),
                message: "rejected context".into()
            },
            &mut worker,
            &redactor
        ));
        assert!(matches!(
            product.event_rx.recv().await,
            Some(VoiceModelEvent::ControlRejected { .. })
        ));
        assert!(project(
            &provider,
            &request,
            "live-session",
            &mut state,
            WireEvent::ContextAccepted {
                client_event_id: Some("unaccepted".into()),
                start_ms: Some(1500),
                end_ms: Some(1600)
            },
            &mut worker,
            &redactor
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), product.event_rx.recv())
                .await
                .is_err()
        );
    }
    #[test]
    fn live_context_chunks_preserve_unicode_and_provider_limit() {
        let value = "已核实：任务等待审批。".repeat(200);
        let chunks = bounded_chunks(&value);
        assert!(chunks.iter().all(|s| s.len() <= 500));
        assert_eq!(chunks.concat(), value);
    }
}
