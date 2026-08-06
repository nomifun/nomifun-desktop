//! One actor per connected robot.
//!
//! This task owns the read loop, the handshake, the keepalive ping, and both
//! audio pipelines. It never touches a socket directly — only [`AcceptedLink`]
//! halves — so the same actor serves a LAN WebSocket today and a relay tunnel
//! later.
//!
//! Model access is reached through the [`crate::services`] seam, so the whole
//! conversation loop is exercised in tests with mocks and no provider.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

use crate::audio::OpusStreamEncoder;
use crate::link::{AcceptedLink, Frame, RobotLinkSink};
use crate::pipeline::{
    DownlinkPacer, SentenceSplitter, UplinkOutcome, UplinkPipeline, encode_for_downlink,
    strip_emotion,
};
use crate::protocol::{
    DeviceMessage, ListenState, ListeningMode, ServerMessage, parse_device_message,
    serialize_server_message,
};
use crate::registry::RobotRegistry;
use crate::services::{CompanionTurnDispatcher, SpeechContext, SpeechServices, TurnEvent};
use crate::status::{RobotPhase, RobotStatusRegistry};
use crate::vad::build_engine;

/// The firmware declares a link dead after 120 s of silence; ping at half that.
pub const PING_INTERVAL_SECS: u64 = 60;

/// Consecutive TTS failures tolerated before the reply is abandoned. One bad
/// sentence is survivable (it is still on screen); a run of them means the
/// provider is down and the device should be released.
pub const MAX_CONSECUTIVE_TTS_FAILURES: usize = 2;

/// Everything a session actor needs from the host.
#[derive(Clone)]
pub struct SessionDeps {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub speech: Arc<dyn SpeechServices>,
    pub dispatcher: Arc<dyn CompanionTurnDispatcher>,
    /// HTTP base the device can reach us on, e.g. `http://192.168.1.20:25808`.
    /// The MCP `initialize` handshake is the only channel that can configure the
    /// firmware's photo-explain endpoint, so with no reachable base we simply do
    /// not advertise vision.
    pub vision_base: Option<String>,
    /// Bearer token the device presents on `/robot/vision/explain`.
    pub device_token: String,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Outbound frames are funnelled through one writer task so the ping timer and
/// the pipelines never contend for the sink.
#[derive(Clone)]
struct Writer {
    pub(crate) tx: mpsc::Sender<Frame>,
}

impl Writer {
    fn spawn(mut sink: Box<dyn RobotLinkSink>) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<Frame>(64);
        let handle = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if sink.send(frame).await.is_err() {
                    break;
                }
            }
            sink.close().await;
        });
        (Self { tx }, handle)
    }

    async fn send_json(&self, msg: &ServerMessage) {
        let _ = self
            .tx
            .send(Frame::Text(serialize_server_message(msg)))
            .await;
    }
}

/// Why a turn ended badly.
#[derive(Debug, Clone)]
pub struct TurnFailure {
    pub message: String,
    pub provider_fault: bool,
}

/// How a turn ended.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub failed: Option<TurnFailure>,
    pub used_fallback: bool,
}

/// Speak one sentence: face, screen, then audio. Returns a failure only when the
/// turn should be abandoned (a run of TTS errors), never for a single bad
/// sentence — that one is still on the device's screen.
#[allow(clippy::too_many_arguments)]
async fn speak_sentence(
    writer: &Writer,
    speech: &Arc<dyn SpeechServices>,
    ctx: &SpeechContext,
    pacer: &Arc<DownlinkPacer>,
    encoder: &mut OpusStreamEncoder,
    speaking: &mut bool,
    tts_failures: &mut usize,
    session_id: &str,
    generation: u64,
    sentence: &str,
) -> Option<TurnFailure> {
    let (emotion, text) = strip_emotion(sentence);
    if text.trim().is_empty() {
        // A marker-only chunk still sets the face; there is nothing to say.
        if let Some(emotion) = emotion {
            writer
                .send_json(&ServerMessage::Llm {
                    session_id: session_id.to_owned(),
                    emotion: emotion.to_owned(),
                })
                .await;
        }
        return None;
    }
    if !*speaking {
        writer
            .send_json(&ServerMessage::TtsStart {
                session_id: session_id.to_owned(),
            })
            .await;
        *speaking = true;
    }
    if let Some(emotion) = emotion {
        writer
            .send_json(&ServerMessage::Llm {
                session_id: session_id.to_owned(),
                emotion: emotion.to_owned(),
            })
            .await;
    }
    writer
        .send_json(&ServerMessage::TtsSentence {
            session_id: session_id.to_owned(),
            text: text.clone(),
        })
        .await;
    match speech.synthesize(ctx, &text).await {
        Ok(audio) => {
            *tts_failures = 0;
            match encode_for_downlink(encoder, &audio) {
                Ok(frames) => pacer.enqueue(generation, frames).await,
                Err(error) => {
                    tracing::warn!(%error, "robot: opus encode failed for a sentence");
                }
            }
            None
        }
        Err(error) => {
            *tts_failures += 1;
            tracing::warn!(
                %error,
                tts_failures = *tts_failures,
                "robot: TTS failed, sentence stays on screen only"
            );
            (*tts_failures >= MAX_CONSECUTIVE_TTS_FAILURES).then(|| TurnFailure {
                message: format!("TTS failed {tts_failures} times: {error}", tts_failures = *tts_failures),
                provider_fault: true,
            })
        }
    }
}

/// Stream a reply to the device: split into sentences, drive the face, speak.
#[allow(clippy::too_many_arguments)]
async fn drive_turn(
    mut events: mpsc::Receiver<TurnEvent>,
    ctx: SpeechContext,
    speech: Arc<dyn SpeechServices>,
    pacer: Arc<DownlinkPacer>,
    writer: Writer,
    session_id: String,
    generation: u64,
    used_fallback: bool,
) -> TurnOutcome {
    let mut splitter = SentenceSplitter::default();
    let mut encoder = match OpusStreamEncoder::new_downlink() {
        Ok(e) => e,
        Err(error) => {
            return TurnOutcome {
                failed: Some(TurnFailure {
                    message: error.to_string(),
                    provider_fault: false,
                }),
                used_fallback,
            };
        }
    };
    let mut speaking = false;
    let mut tts_failures = 0usize;
    let mut failure: Option<TurnFailure> = None;

    while let Some(event) = events.recv().await {
        match event {
            TurnEvent::Text(chunk) => {
                for sentence in splitter.push(&chunk) {
                    failure = speak_sentence(
                        &writer,
                        &speech,
                        &ctx,
                        &pacer,
                        &mut encoder,
                        &mut speaking,
                        &mut tts_failures,
                        &session_id,
                        generation,
                        &sentence,
                    )
                    .await;
                    if failure.is_some() {
                        break;
                    }
                }
            }
            TurnEvent::Done => {
                if failure.is_none()
                    && let Some(tail) = splitter.flush()
                {
                    failure = speak_sentence(
                        &writer,
                        &speech,
                        &ctx,
                        &pacer,
                        &mut encoder,
                        &mut speaking,
                        &mut tts_failures,
                        &session_id,
                        generation,
                        &tail,
                    )
                    .await;
                }
                break;
            }
            TurnEvent::Failed {
                message,
                provider_fault,
            } => {
                failure = Some(TurnFailure {
                    message,
                    provider_fault,
                });
                break;
            }
        }
        if failure.is_some() {
            break;
        }
    }

    if speaking {
        // The stop rides the pacer queue rather than jumping it: the device
        // drops downlink audio the moment it leaves the speaking state, so a
        // stop sent while frames are still queued would truncate the reply.
        let stop = serialize_server_message(&ServerMessage::TtsStop {
            session_id: session_id.clone(),
        });
        if !pacer.enqueue_text(generation, stop).await && pacer.generation() == generation {
            // The pacer could not take it and this turn was not cancelled; the
            // device must never be left believing it is still speaking.
            writer
                .send_json(&ServerMessage::TtsStop {
                    session_id: session_id.clone(),
                })
                .await;
        }
    }
    TurnOutcome {
        failed: failure,
        used_fallback,
    }
}

/// Tell the user something went wrong without stranding the device in `speaking`.
async fn report_turn_failure(writer: &Writer, session_id: &str) {
    writer
        .send_json(&ServerMessage::Llm {
            session_id: session_id.to_owned(),
            emotion: "sad".to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::TtsStart {
            session_id: session_id.to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::TtsSentence {
            session_id: session_id.to_owned(),
            text: "我这边出了点问题，稍后再试试。".to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::TtsStop {
            session_id: session_id.to_owned(),
        })
        .await;
}

/// Kick off one turn and remember its task so `abort` can kill it.
#[allow(clippy::too_many_arguments)]
async fn start_turn(
    deps: &SessionDeps,
    pacer: &Arc<DownlinkPacer>,
    writer: &Writer,
    turn_tx: &mpsc::Sender<TurnOutcome>,
    turn_task: &mut Option<tokio::task::JoinHandle<()>>,
    robot_id: &str,
    companion_id: &str,
    conversation_id: &str,
    session_id: &str,
    text: &str,
    use_fallback: bool,
) {
    let events = match deps
        .dispatcher
        .dispatch(conversation_id, text, use_fallback)
        .await
    {
        Ok(rx) => rx,
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: dispatch failed");
            let _ = turn_tx
                .send(TurnOutcome {
                    failed: Some(TurnFailure {
                        message: error.to_string(),
                        provider_fault: true,
                    }),
                    used_fallback: use_fallback,
                })
                .await;
            return;
        }
    };
    let ctx = SpeechContext {
        robot_id: robot_id.to_owned(),
        companion_id: companion_id.to_owned(),
    };
    let generation = pacer.generation();
    let (speech, pacer, writer, session_id, turn_tx) = (
        deps.speech.clone(),
        pacer.clone(),
        writer.clone(),
        session_id.to_owned(),
        turn_tx.clone(),
    );
    *turn_task = Some(tokio::spawn(async move {
        let outcome = drive_turn(
            events,
            ctx,
            speech,
            pacer,
            writer,
            session_id,
            generation,
            use_fallback,
        )
        .await;
        let _ = turn_tx.send(outcome).await;
    }));
}

/// Run one robot session to completion. Returns when the inbound stream ends.
pub async fn run_session(link: AcceptedLink, deps: SessionDeps) {
    let AcceptedLink {
        identity,
        sink,
        mut stream,
    } = link;
    let robot_id = identity.robot_id.clone();
    let (writer, writer_task) = Writer::spawn(sink);

    let mut session_id: Option<String> = None;
    let mut companion_id: Option<String> = None;
    let mut uplink: Option<UplinkPipeline> = None;
    let mut mcp: Option<Arc<crate::mcp_bridge::RobotMcpClient>> = None;
    let mut discovery_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut conversation_id: Option<String> = None;
    let (pacer, pacer_task) = DownlinkPacer::spawn(writer.tx.clone());
    let pacer = Arc::new(pacer);
    let (turn_tx, mut turn_rx) = mpsc::channel::<TurnOutcome>(4);
    let mut turn_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut pending_text: Option<String> = None;
    let mut ping = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.tick().await; // the first tick is immediate; skip it

    loop {
        // Set by whichever branch decided the user stopped talking; handled at
        // the end of the iteration so the read loop stays one flat match.
        let mut utterance: Option<Vec<u8>> = None;

        tokio::select! {
            _ = ping.tick() => {
                if let Some(sid) = &session_id {
                    writer.send_json(&ServerMessage::Ping { session_id: sid.clone() }).await;
                }
            }
            outcome = turn_rx.recv() => {
                let Some(outcome) = outcome else { continue };
                turn_task = None;
                let Some(failure) = outcome.failed else {
                    if let Some(bound) = &companion_id {
                        deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                    }
                    pending_text = None;
                    continue;
                };
                tracing::warn!(%robot_id, message = %failure.message, "robot: turn failed");
                let retryable = failure.provider_fault && !outcome.used_fallback;
                let has_fallback = match &companion_id {
                    Some(bound) => deps.dispatcher.has_fallback_model(bound).await,
                    None => false,
                };
                if retryable
                    && has_fallback
                    && let (Some(text), Some(conversation), Some(sid), Some(bound)) = (
                        pending_text.clone(),
                        conversation_id.clone(),
                        session_id.clone(),
                        companion_id.clone(),
                    )
                {
                    tracing::info!(%robot_id, "robot: retrying the turn on the fallback model");
                    start_turn(
                        &deps, &pacer, &writer, &turn_tx, &mut turn_task,
                        &robot_id, &bound, &conversation, &sid, &text, true,
                    )
                    .await;
                    continue;
                }
                if let Some(sid) = &session_id {
                    report_turn_failure(&writer, sid).await;
                }
                pending_text = None;
                if let Some(bound) = &companion_id {
                    deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let Ok(frame) = frame else { break };
                match frame {
                    Frame::Binary(_) if session_id.is_none() => {
                        // Wake-word audio can arrive before `listen start`; before
                        // the handshake it is simply noise.
                        continue;
                    }
                    Frame::Binary(bytes) => {
                        let Some(pipeline) = uplink.as_mut() else { continue };
                        if let UplinkOutcome::Utterance(wav) = pipeline.push_packet(&bytes) {
                            utterance = Some(wav);
                        }
                    }
                    Frame::Text(raw) => {
                        let message = match parse_device_message(&raw) {
                            Ok(m) => m,
                            Err(error) => {
                                tracing::warn!(%robot_id, %error, "robot: unparseable text frame");
                                continue;
                            }
                        };
                        match message {
                            DeviceMessage::Hello(hello) => {
                                let record = deps.registry.list().await.into_iter().find(|r| r.robot_id == robot_id);
                                let Some(bound) = record.as_ref().and_then(|r| r.companion_id.clone()) else {
                                    tracing::warn!(%robot_id, "robot: refusing session, not bound to a companion");
                                    break;
                                };
                                let sid = uuid::Uuid::new_v4().to_string();
                                tracing::info!(
                                    %robot_id,
                                    companion_id = %bound,
                                    session_id = %sid,
                                    protocol_version = hello.version,
                                    mcp = hello.mcp,
                                    "robot: session established"
                                );
                                writer.send_json(&ServerMessage::Hello { session_id: sid.clone() }).await;
                                deps.status
                                    .publish(&robot_id, Some(&bound), RobotPhase::Idle, now_ms())
                                    .await;
                                let tuning = deps.dispatcher.vad_tuning(&bound).await;
                                let engine = build_engine("silero", tuning);
                                tracing::info!(%robot_id, vad = engine.name(), "robot: endpointer ready");
                                uplink = UplinkPipeline::new(engine).ok();
                                conversation_id = deps
                                    .dispatcher
                                    .ensure_thread(&robot_id, &bound)
                                    .await
                                    .inspect_err(|error| {
                                        tracing::error!(%robot_id, %error, "robot: could not open a companion thread");
                                    })
                                    .ok();
                                let client = Arc::new(crate::mcp_bridge::RobotMcpClient::new(
                                    writer.tx.clone(),
                                    sid.clone(),
                                ));
                                mcp = Some(client.clone());
                                if hello.mcp {
                                    let vision_base = deps.vision_base.clone();
                                    let device_token = deps.device_token.clone();
                                    let discovering = robot_id.clone();
                                    discovery_task = Some(tokio::spawn(async move {
                                        let url = vision_base
                                            .map(|base| format!("{base}{}", crate::endpoint::VISION_PATH));
                                        if let Err(error) =
                                            client.initialize(url.as_deref(), &device_token).await
                                        {
                                            tracing::warn!(robot_id = %discovering, %error, "robot: MCP initialize failed");
                                            return;
                                        }
                                        match client.list_tools().await {
                                            Ok(tools) => tracing::info!(
                                                robot_id = %discovering,
                                                count = tools.len(),
                                                names = ?tools.iter().map(|t| &t.exposed_name).collect::<Vec<_>>(),
                                                "robot: device tools discovered"
                                            ),
                                            Err(error) => tracing::warn!(robot_id = %discovering, %error, "robot: tools/list failed"),
                                        }
                                    }));
                                }
                                session_id = Some(sid);
                                companion_id = Some(bound);
                            }
                            DeviceMessage::Listen { state, mode, .. } => {
                                let Some(pipeline) = uplink.as_mut() else { continue };
                                match state {
                                    ListenState::Start => {
                                        pipeline.begin(mode.unwrap_or(ListeningMode::Auto));
                                        if let Some(bound) = &companion_id {
                                            deps.status
                                                .publish(&robot_id, Some(bound), RobotPhase::Listening, now_ms())
                                                .await;
                                        }
                                    }
                                    ListenState::Stop => {
                                        if let Some(wav) = pipeline.finish() {
                                            utterance = Some(wav);
                                        }
                                    }
                                    // The wake word itself is not part of the turn.
                                    ListenState::Detect => {}
                                }
                            }
                            DeviceMessage::Abort { reason } => {
                                tracing::info!(%robot_id, ?reason, "robot: abort");
                                // Order matters: stop our own queue first, because
                                // the device will play anything we hand over.
                                pacer.flush();
                                if let Some(task) = turn_task.take() {
                                    task.abort();
                                }
                                if let Some(pipeline) = uplink.as_mut() {
                                    pipeline.abort();
                                }
                                if let Some(conversation) = &conversation_id
                                    && let Err(error) = deps.dispatcher.cancel(conversation).await
                                {
                                    tracing::warn!(%robot_id, %error, "robot: turn cancel failed");
                                }
                                if let Some(sid) = &session_id {
                                    writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                                }
                                if let Some(bound) = &companion_id {
                                    deps.status.publish(&robot_id, Some(bound), RobotPhase::Idle, now_ms()).await;
                                }
                            }
                            DeviceMessage::Goodbye => {
                                tracing::info!(%robot_id, "robot: device said goodbye");
                                break;
                            }
                            DeviceMessage::Unknown { raw_type } => {
                                tracing::debug!(%robot_id, %raw_type, "robot: unknown message type");
                            }
                            DeviceMessage::Mcp { payload } => {
                                if let Some(client) = &mcp {
                                    client.handle_incoming(payload).await;
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(wav) = utterance.take() {
            let (Some(sid), Some(bound), Some(conversation)) = (
                session_id.clone(),
                companion_id.clone(),
                conversation_id.clone(),
            ) else {
                continue;
            };
            let ctx = SpeechContext {
                robot_id: robot_id.clone(),
                companion_id: bound.clone(),
            };
            let transcript = match deps.speech.transcribe(&ctx, wav).await {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%robot_id, %error, "robot: ASR failed");
                    String::new()
                }
            };
            if transcript.trim().is_empty() {
                // An empty round: hand the device straight back to listening
                // without spending a model turn on noise.
                writer
                    .send_json(&ServerMessage::TtsStart {
                        session_id: sid.clone(),
                    })
                    .await;
                writer
                    .send_json(&ServerMessage::TtsStop {
                        session_id: sid.clone(),
                    })
                    .await;
                deps.status
                    .publish(&robot_id, Some(&bound), RobotPhase::Idle, now_ms())
                    .await;
                continue;
            }
            writer
                .send_json(&ServerMessage::Stt {
                    session_id: sid.clone(),
                    text: transcript.clone(),
                })
                .await;
            deps.status
                .publish(&robot_id, Some(&bound), RobotPhase::Speaking, now_ms())
                .await;
            pending_text = Some(transcript.clone());
            start_turn(
                &deps,
                &pacer,
                &writer,
                &turn_tx,
                &mut turn_task,
                &robot_id,
                &bound,
                &conversation,
                &sid,
                &transcript,
                false,
            )
            .await;
        }
    }

    // The turn task, the tool-discovery task, the pacer and the MCP client all
    // hold clones of the writer's sender, so every one of them has to be gone —
    // not merely asked to stop — before the writer task can see its channel
    // close and finish.
    if let Some(task) = turn_task.take() {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = discovery_task.take() {
        task.abort();
        let _ = task.await;
    }
    drop(mcp);
    pacer.flush();
    pacer_task.abort();
    let _ = pacer_task.await;
    drop(pacer);
    deps.status.mark_offline(&robot_id, now_ms()).await;
    drop(writer);
    let _ = writer_task.await;
    tracing::info!(%robot_id, "robot: session ended");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RobotEventEmitter;
    use crate::link::{
        AcceptedLink, Frame, LinkError, RobotIdentity, RobotLinkSink, RobotLinkStream,
    };
    use crate::registry::{RobotRegistry, RobotReport};
    use crate::services::TurnEvent;
    use crate::services::mock::{MockDispatcher, MockSpeech};
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    struct NullSink;
    impl UserEventSink for NullSink {
        fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<serde_json::Value>) {}
    }

    /// A sink that records everything written, and a stream driven by a channel.
    struct RecordingSink(Arc<Mutex<Vec<Frame>>>);
    #[async_trait::async_trait]
    impl RobotLinkSink for RecordingSink {
        async fn send(&mut self, frame: Frame) -> Result<(), LinkError> {
            self.0.lock().unwrap().push(frame);
            Ok(())
        }
        async fn close(&mut self) {}
    }

    struct ChannelStream(mpsc::Receiver<Frame>);
    #[async_trait::async_trait]
    impl RobotLinkStream for ChannelStream {
        async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
            self.0.recv().await.map(Ok)
        }
    }

    /// The deps plus handles on the mocks behind them.
    struct Harness {
        deps: SessionDeps,
        speech: Arc<MockSpeech>,
        dispatcher: Arc<MockDispatcher>,
    }

    impl Harness {
        fn speech_mock(&self) -> Arc<MockSpeech> {
            self.speech.clone()
        }
        fn dispatcher_mock(&self) -> Arc<MockDispatcher> {
            self.dispatcher.clone()
        }
    }

    async fn harness(
        bound: bool,
    ) -> (
        Harness,
        AcceptedLink,
        mpsc::Sender<Frame>,
        Arc<Mutex<Vec<Frame>>>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, token) = registry
            .upsert_on_report(
                RobotReport {
                    robot_id: "aa:bb:cc:dd:ee:ff".into(),
                    client_id: "cid".into(),
                    board: "esp32-s3n16r8-emoji".into(),
                    firmware_version: "1.9.0".into(),
                },
                1,
            )
            .await
            .unwrap();
        if bound {
            registry
                .claim(
                    record.activation_code.as_deref().unwrap(),
                    "0190f5fe-7c00-7a00-8000-0000000000aa",
                )
                .await
                .unwrap();
        }
        let _ = token;
        let status = Arc::new(crate::status::RobotStatusRegistry::new(
            RobotEventEmitter::new(Arc::new(NullSink)),
            "owner-1".to_owned(),
        ));
        let speech = Arc::new(MockSpeech::new());
        let dispatcher = Arc::new(MockDispatcher::new());
        let written = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel(64);
        let link = AcceptedLink {
            identity: RobotIdentity {
                robot_id: "aa:bb:cc:dd:ee:ff".into(),
                client_id: "cid".into(),
                peer: "192.168.1.9".into(),
            },
            sink: Box::new(RecordingSink(written.clone())),
            stream: Box::new(ChannelStream(rx)),
        };
        let deps = SessionDeps {
            registry,
            status,
            speech: speech.clone(),
            dispatcher: dispatcher.clone(),
            vision_base: None,
            device_token: "tok".to_owned(),
        };
        (
            Harness {
                deps,
                speech,
                dispatcher,
            },
            link,
            tx,
            written,
            dir,
        )
    }

    fn texts(frames: &Arc<Mutex<Vec<Frame>>>) -> Vec<serde_json::Value> {
        frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|f| match f {
                Frame::Text(t) => serde_json::from_str(t).ok(),
                Frame::Binary(_) => None,
            })
            .collect()
    }

    /// Encode `ms` of 16 kHz audio into 60 ms uplink packets, as the device
    /// would. `loud` audio has to read as speech to the **real** Silero VAD the
    /// session builds, so it is a glottal pulse train under two sweeping
    /// formants; a static tone mix scores ~1% and would never open a turn.
    fn uplink_packets(ms: u32, loud: bool) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        let tau = std::f32::consts::TAU;
        let pcm: Vec<i16> = (0..n)
            .map(|i| {
                if !loud {
                    return 0;
                }
                let t = i as f32 / 16_000.0;
                let f0 = 120.0 + 30.0 * (t * 4.0 * tau).sin();
                let f1 = 500.0 + 300.0 * (t * 3.0 * tau).sin();
                let f2 = 1500.0 + 500.0 * (t * 2.0 * tau).cos();
                let mut v = 0.0;
                for harmonic in 1..=20 {
                    let f = f0 * harmonic as f32;
                    let a = (-((f - f1) / 250.0).powi(2)).exp()
                        + 0.6 * (-((f - f2) / 350.0).powi(2)).exp();
                    v += a * (t * f * tau).sin();
                }
                let enveloped = v * 0.5 * (0.6 + 0.4 * (t * 3.5 * tau).sin());
                (enveloped.clamp(-1.0, 1.0) * 9000.0) as i16
            })
            .collect();
        crate::audio::OpusStreamEncoder::new_uplink_for_test()
            .unwrap()
            .encode_frames(&pcm)
            .unwrap()
    }

    async fn send_audio(tx: &mpsc::Sender<Frame>, ms: u32, loud: bool) {
        for packet in uplink_packets(ms, loud) {
            tx.send(Frame::Binary(bytes::Bytes::from(packet)))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn bound_device_gets_a_server_hello_after_its_hello() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps.deps.clone()));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#
                .into(),
        ))
        .await
        .unwrap();
        // Closing the stream ends the session loop.
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        assert_eq!(sent.len(), 1, "exactly one server hello");
        assert_eq!(sent[0]["type"], "hello");
        assert_eq!(sent[0]["audio_params"]["sample_rate"], 24000);
        assert!(sent[0]["session_id"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[tokio::test]
    async fn unbound_device_is_refused_after_hello_with_no_server_hello() {
        let (deps, link, tx, written, _dir) = harness(false).await;
        let task = tokio::spawn(run_session(link, deps.deps.clone()));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        task.await.unwrap();

        let sent = texts(&written);
        assert!(
            sent.iter().all(|m| m["type"] != "hello"),
            "an unbound robot must never get a session"
        );
    }

    #[tokio::test]
    async fn audio_before_hello_is_ignored_not_fatal() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps.deps.clone()));

        tx.send(Frame::Binary(bytes::Bytes::from_static(&[0xfc, 0x01])))
            .await
            .unwrap();
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            texts(&written).len(),
            1,
            "session still established after stray audio"
        );
    }

    #[tokio::test]
    async fn unknown_message_type_does_not_end_the_session() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let task = tokio::spawn(run_session(link, deps.deps.clone()));

        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(r#"{"type":"brand_new_thing","x":1}"#.into()))
            .await
            .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"stop"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        assert_eq!(texts(&written)[0]["type"], "hello");
    }

    #[tokio::test]
    async fn session_marks_offline_when_the_link_drops() {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let status = deps.deps.status.clone();
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        drop(tx);
        task.await.unwrap();

        let snap = status.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].phase, "offline");
    }

    #[tokio::test]
    async fn a_full_turn_produces_stt_emotion_sentence_audio_and_stop() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("今天天气怎么样");
        dispatcher.script_turn(vec![
            TurnEvent::Text("[emotion:happy] 晴朗得很。".into()),
            TurnEvent::Done,
        ]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await; // trailing silence ends the utterance

        // Give the turn time to run, then close the link.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        let types: Vec<String> = sent
            .iter()
            .map(|m| {
                let t = m["type"].as_str().unwrap_or_default();
                match (t, m["state"].as_str()) {
                    ("tts", Some(state)) => format!("tts:{state}"),
                    _ => t.to_owned(),
                }
            })
            .collect();

        assert!(
            types.contains(&"stt".to_owned()),
            "the transcript is shown on screen: {types:?}"
        );
        assert!(
            types.contains(&"llm".to_owned()),
            "the emotion marker drives the face: {types:?}"
        );
        assert!(types.contains(&"tts:start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:sentence_start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:stop".to_owned()), "{types:?}");

        let stt = sent.iter().find(|m| m["type"] == "stt").unwrap();
        assert_eq!(stt["text"], "今天天气怎么样");
        let llm = sent.iter().find(|m| m["type"] == "llm").unwrap();
        assert_eq!(llm["emotion"], "happy");
        let sentence = sent
            .iter()
            .find(|m| m["type"] == "tts" && m["state"] == "sentence_start")
            .unwrap();
        assert_eq!(
            sentence["text"], "晴朗得很。",
            "the emotion marker is stripped before display"
        );

        let audio_frames = written
            .lock()
            .unwrap()
            .iter()
            .filter(|f| matches!(f, Frame::Binary(_)))
            .count();
        assert!(audio_frames > 0, "synthesised audio reached the device");
        assert_eq!(dispatcher.dispatched_text(), vec!["今天天气怎么样".to_owned()]);
    }

    #[tokio::test]
    async fn the_stop_never_overtakes_the_audio_it_ends() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("说点什么");
        dispatcher.script_turn(vec![
            TurnEvent::Text("一二三四五六七八九十。".into()),
            TurnEvent::Done,
        ]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        drop(tx);
        task.await.unwrap();

        // The device drops downlink audio as soon as it stops speaking, so every
        // audio frame of a reply must be written before that reply's stop.
        let frames = written.lock().unwrap().clone();
        let stop_at = frames.iter().position(|f| match f {
            Frame::Text(t) => t.contains(r#""state":"stop""#),
            Frame::Binary(_) => false,
        });
        let last_audio = frames
            .iter()
            .rposition(|f| matches!(f, Frame::Binary(_)))
            .expect("the reply was spoken");
        assert!(
            stop_at.is_some_and(|stop| stop > last_audio),
            "stop at {stop_at:?} must follow the last audio frame at {last_audio}"
        );
    }

    #[tokio::test]
    async fn empty_transcript_idles_the_device_without_bothering_the_model() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let dispatcher = deps.dispatcher_mock();
        // MockSpeech returns "" by default.

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"manual"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 200, true).await;
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"stop"}"#.into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        drop(tx);
        task.await.unwrap();

        assert!(
            dispatcher.dispatched_text().is_empty(),
            "no model turn for silence"
        );
        let sent = texts(&written);
        let states: Vec<&str> = sent
            .iter()
            .filter(|m| m["type"] == "tts")
            .filter_map(|m| m["state"].as_str())
            .collect();
        assert_eq!(
            states,
            vec!["start", "stop"],
            "an empty round returns the device to listening"
        );
    }

    #[tokio::test]
    async fn abort_cancels_the_turn_and_sends_tts_stop() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("讲个很长的故事");
        // A long reply so there is something in flight to cancel.
        dispatcher.script_turn(
            (0..40)
                .map(|i| TurnEvent::Text(format!("第{i}句话。")))
                .chain(std::iter::once(TurnEvent::Done))
                .collect(),
        );

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#.into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let frames_at_abort = written.lock().unwrap().len();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let frames_later = written.lock().unwrap().len();
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            frames_at_abort, frames_later,
            "not one frame may be written after abort — the device does not flush its own queue"
        );
        assert_eq!(
            dispatcher.cancelled().len(),
            1,
            "the platform turn is cancelled too"
        );
        let sent = texts(&written);
        assert!(
            sent.iter()
                .any(|m| m["type"] == "tts" && m["state"] == "stop"),
            "abort must be acknowledged with tts stop"
        );
    }

    #[tokio::test]
    async fn a_provider_failure_retries_once_on_the_fallback_model() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("你好");
        dispatcher.set_has_fallback(true);
        dispatcher.script_turn(vec![TurnEvent::Failed {
            message: "upstream 503".into(),
            provider_fault: true,
        }]);
        dispatcher.script_turn(vec![TurnEvent::Text("我在。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        drop(tx);
        task.await.unwrap();

        assert_eq!(
            dispatcher.fallback_dispatches(),
            1,
            "exactly one fallback retry"
        );
        assert_eq!(
            dispatcher.dispatched_text().len(),
            2,
            "the same text, twice"
        );
        let sent = texts(&written);
        assert!(
            sent.iter().any(
                |m| m["type"] == "tts" && m["state"] == "sentence_start" && m["text"] == "我在。"
            ),
            "the fallback reply reaches the device"
        );
    }

    #[tokio::test]
    async fn a_failure_with_no_fallback_reports_sadly_and_stops() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("你好");
        dispatcher.set_has_fallback(false);
        dispatcher.script_turn(vec![TurnEvent::Failed {
            message: "upstream 503".into(),
            provider_fault: true,
        }]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(tx);
        task.await.unwrap();

        assert_eq!(dispatcher.fallback_dispatches(), 0);
        let sent = texts(&written);
        assert!(
            sent.iter().any(|m| m["type"] == "llm" && m["emotion"] == "sad"),
            "the robot looks sad about it"
        );
        assert!(
            sent.iter()
                .any(|m| m["type"] == "tts" && m["state"] == "stop"),
            "the device must not be left stuck in speaking"
        );
    }

    #[tokio::test]
    async fn status_walks_idle_listening_speaking_then_offline() {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let status = deps.deps.status.clone();
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("嗨");
        dispatcher.script_turn(vec![TurnEvent::Text("嗨。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(status.snapshot().await[0].phase, "idle");

        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#.into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(status.snapshot().await[0].phase, "listening");

        send_audio(&tx, 300, true).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(tx);
        task.await.unwrap();
        assert_eq!(status.snapshot().await[0].phase, "offline");
    }
}
