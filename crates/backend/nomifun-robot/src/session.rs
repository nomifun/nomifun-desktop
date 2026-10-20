//! One actor per connected robot.
//!
//! This task owns the read loop, the handshake, the keepalive ping, and both
//! audio pipelines. It never touches a socket directly — only [`AcceptedLink`]
//! halves — so the same actor serves a LAN WebSocket today and a relay tunnel
//! later.
//!
//! Model access is reached through the [`crate::services`] seam, so the whole
//! conversation loop is exercised in tests with mocks and no provider.

use std::{collections::{BTreeMap, VecDeque}, sync::Arc};
use sha2::{Digest, Sha256};
use futures_util::StreamExt;

use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, interval};

use crate::audio::OpusStreamEncoder;
use crate::link::{AcceptedLink, Frame, RobotLinkSink};
use crate::lifecycle::{OwnedTask, OwnedTasks, TaskScope};
use crate::pipeline::{
    DownlinkPacer, SentenceSplitter, UplinkOutcome, UplinkPipeline,
    sanitize_for_display, sanitize_for_speech,
};
use crate::protocol::{
    DeviceMessage, ListenState, ListeningMode, ServerMessage, parse_device_message,
    serialize_server_message,
};
use crate::registry::{RobotRegistry, RobotPlaybackError};
use crate::services::{CompanionTurnDispatcher, RobotTurnRequest, SpeechContext, SpeechServices, TurnEvent};
use crate::status::{RobotPhase, RobotStatusRegistry};
use crate::vad::build_engine;

/// The firmware declares a link dead after 120 s of silence; ping at half that.
pub const PING_INTERVAL_SECS: u64 = 60;

/// Consecutive TTS failures tolerated before the reply is abandoned. One bad
/// sentence is survivable (it is still on screen); a run of them means the
/// provider is down and the device should be released.
pub const MAX_CONSECUTIVE_TTS_FAILURES: usize = 2;

/// Transport acceptance is scoped to this exact live connection. Reaching the
/// bound rejects new sources instead of evicting one and authorizing a replay.
const MAX_CANONICAL_PLAYBACK_SOURCES: usize = 256;
#[derive(Default)]
struct CanonicalPlaybackSources {
    accepted: BTreeMap<String, (nomifun_voice_contracts::VoiceSpeechSourceRef, [u8; 32], Option<String>)>,
}

#[cfg(test)]
mod canonical_playback_tests {
    use super::*;
    fn source(id: usize) -> nomifun_voice_contracts::VoiceSpeechSourceRef {
        nomifun_voice_contracts::VoiceSpeechSourceRef { message_id:format!("answer-{id}"),revision:10,through_seq:10 }
    }
    #[test]
    fn accepted_source_replays_acknowledgement_without_another_playback() {
        let mut window = CanonicalPlaybackSources::default();
        let original = source(1);
        assert!(!window.already_accepted(&original,"finished",None).unwrap());
        window.record(original.clone(),"finished",None);
        assert!(window.already_accepted(&original,"finished",None).unwrap());
        assert!(window.already_accepted(&original,"different words",None).is_err());
        assert!(window.already_accepted(&original,"finished",Some("other-question")).is_err());
        let mut changed = original; changed.revision += 1; changed.through_seq += 1;
        assert!(window.already_accepted(&changed,"finished",None).is_err());
    }
    #[test]
    fn capacity_never_evicts_a_previously_accepted_source() {
        let mut window = CanonicalPlaybackSources::default();
        for id in 0..MAX_CANONICAL_PLAYBACK_SOURCES { window.record(source(id),"result",None); }
        assert!(window.already_accepted(&source(MAX_CANONICAL_PLAYBACK_SOURCES),"result",None).is_err());
        assert!(window.already_accepted(&source(0),"result",None).unwrap());
    }
}
impl CanonicalPlaybackSources {
    fn already_accepted(&self, source: &nomifun_voice_contracts::VoiceSpeechSourceRef, text: &str, question_id:Option<&str>) -> Result<bool, RobotPlaybackError> {
        source.validate().map_err(RobotPlaybackError::InvalidSource)?;
        let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        if let Some((prior, prior_digest, prior_question)) = self.accepted.get(&source.message_id) {
            return if prior == source && *prior_digest == digest && prior_question.as_deref()==question_id { Ok(true) }
                else { Err(RobotPlaybackError::SourceChanged) };
        }
        if self.accepted.len() >= MAX_CANONICAL_PLAYBACK_SOURCES {
            return Err(RobotPlaybackError::SourceCapacity);
        }
        Ok(false)
    }
    fn record(&mut self, source: nomifun_voice_contracts::VoiceSpeechSourceRef, text: &str, question_id:Option<String>) {
        self.accepted.insert(source.message_id.clone(), (source, Sha256::digest(text.as_bytes()).into(), question_id));
    }
}

/// One ASR request runs at a time; two complete utterances may wait behind it.
/// Each WAV is already bounded by the uplink pipeline's 60-second ceiling.
const MAX_PENDING_TRANSCRIPTIONS: usize = 2;

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
enum SpokenIntent {NewRequest,StopPlayback,CancelWork,Supplement,Correction}
fn spoken_intent(text:&str)->SpokenIntent {
    let text=text.trim().trim_end_matches(['。','！','!','.']).trim();
    match text {
        "停播"|"停止播音"|"停止播放"|"别说了"|"stop speaking"=>SpokenIntent::StopPlayback,
        "取消任务"|"取消工作"|"停止这个任务"|"别做了"|"cancel task"|"取消后台任务"|"停止后台任务"=>SpokenIntent::CancelWork,
        _ if ["补充：","补充:","补充，","补充,","补充一下","再补充","补充后台任务：","补充后台任务:"].iter().any(|prefix|text.starts_with(prefix))=>SpokenIntent::Supplement,
        _ if ["更正：","更正:","更正为","更正一下","改口：","改口:","改口为","更正后台任务：","更正后台任务:"].iter().any(|prefix|text.starts_with(prefix))=>SpokenIntent::Correction,
        _=>SpokenIntent::NewRequest,
    }
}
fn explicit_new_request(text:&str)->bool {
    ["新任务：","新任务:","新请求：","新请求:","另外一个问题"].iter().any(|prefix|text.trim().starts_with(prefix))
}
fn explicit_background_control(text:&str)->bool {
    ["取消后台任务","停止后台任务","补充后台任务","更正后台任务"].iter().any(|prefix|text.trim().starts_with(prefix))
}

enum CapturedTranscript { Batch, Transcript(String), Cancelled }
struct CapturedPcm { samples:Vec<i16>, captured_at:tokio::time::Instant }
struct PreparedTranscription {
    input: Option<mpsc::Sender<CapturedPcm>>,
    cancellation: tokio::sync::watch::Sender<bool>,
    task: OwnedTask,
    result: oneshot::Receiver<anyhow::Result<CapturedTranscript>>,
    failure: Arc<std::sync::Mutex<Option<String>>>,
}
impl PreparedTranscription {
    fn start(deps:&SessionDeps, robot_id:&str, session_id:&str, companion_id:&str,
        tasks:&TaskScope, stopped:&tokio::sync::watch::Receiver<bool>) -> Self {
        let (input, frames)=mpsc::channel(8);
        let (cancellation, cancel)=tokio::sync::watch::channel(false);
        let (outcome, result)=oneshot::channel();
        let (speech, registry)=(deps.speech.clone(),deps.registry.clone());
        let context=SpeechContext { robot_id:robot_id.into(),companion_id:companion_id.into() };
        let session_id=session_id.to_owned();
        let stopped=stopped.clone();
        let failure=Arc::new(std::sync::Mutex::new(None));
        let capture_failure=failure.clone();
        let task=tasks.spawn(async move {
            let result=capture_transcript(speech,registry,context,session_id,frames,cancel,stopped,capture_failure).await;
            let _=outcome.send(result);
        });
        Self { input:Some(input), cancellation, task, result, failure }
    }
    fn push(&mut self, samples:Vec<i16>) {
        if self.failure.lock().unwrap_or_else(|error|error.into_inner()).is_some() { return; }
        let Some(input)=self.input.as_ref() else { return; };
        // At most one decoded packet or one 300 ms VAD lead-in is cloned.
        // The wire/provider ports both receive no more than 60 ms per frame.
        for frame in samples.chunks(crate::audio::UPLINK_FRAME_SAMPLES) {
            match input.try_send(CapturedPcm { samples:frame.to_vec(),captured_at:tokio::time::Instant::now() }) {
                Ok(()) => {},
                Err(mpsc::error::TrySendError::Closed(_)) => break, // Batch or a retained provider failure.
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // The configured protocol may still be resolving to
                    // batch. Keep its complete WAV authoritative; unused PCM
                    // tap saturation must not cancel that discovery. A real
                    // incremental session observes this same failure before
                    // admitting another frame and cancels without fallback.
                    *self.failure.lock().unwrap_or_else(|error|error.into_inner())=Some("incremental ASR capture exceeded its media queue budget".into());
                    break;
                }
            }
        }
    }
    fn endpoint(&mut self) { self.input.take(); }
    async fn cancel(self, errors:&mut Vec<String>) {
        self.cancellation.send_replace(true);
        if let Err(error)=self.task.join().await { errors.push(error); }
    }
}

async fn capture_transcript(speech:Arc<dyn SpeechServices>,registry:Arc<RobotRegistry>,context:SpeechContext,
    session_id:String,mut frames:mpsc::Receiver<CapturedPcm>,cancel:tokio::sync::watch::Receiver<bool>,
    stopped:tokio::sync::watch::Receiver<bool>,failure:Arc<std::sync::Mutex<Option<String>>>) -> anyhow::Result<CapturedTranscript> {
    let invalidated=async {
        loop {
            let cancelled=*cancel.borrow() || *stopped.borrow();
            if cancelled
                || !registry.connection_matches(&context.robot_id,&session_id).await
                || !registry.get(&context.robot_id).await.is_some_and(|record|
                    record.companion_id.as_deref()==Some(context.companion_id.as_str())) { return; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    tokio::pin!(invalidated);
    let started=tokio::time::Instant::now();
    let provider=tokio::select! { biased;
        _=&mut invalidated=>return Ok(CapturedTranscript::Cancelled),
        provider=speech.open_transcription(&context,crate::protocol::UPLINK_SAMPLE_RATE)=>provider?,
    };
    let Some(mut provider)=provider else { return Ok(CapturedTranscript::Batch); };
    tracing::info!(robot_id=%context.robot_id,%session_id,startup_ms=started.elapsed().as_millis() as u64,
        "robot: incremental ASR opened during capture");
    let result=loop {
        let capture_failure=failure.lock().unwrap_or_else(|error|error.into_inner()).clone();
        if let Some(failure)=capture_failure {break Err(anyhow::anyhow!(failure));}
        tokio::select! { biased;
            _=&mut invalidated=>break Ok(CapturedTranscript::Cancelled),
            frame=frames.recv()=>match frame {
                Some(frame)=>{
                    if frame.captured_at.elapsed()>Duration::from_millis(750) {
                        break Err(anyhow::anyhow!("incremental ASR capture exceeded its media age budget"));
                    }
                    if let Err(error)=provider.try_send(frame.samples) { break Err(error.into()); }
                }
                None=>{
                    break tokio::select! { biased;
                        _=&mut invalidated=>Ok(CapturedTranscript::Cancelled),
                        result=provider.finish_result()=>result.map(CapturedTranscript::Transcript).map_err(anyhow::Error::from),
                    };
                }
            },
            event=provider.recv()=>match event {
                Some(Ok(crate::services::TranscriptEvent::Interim(_) | crate::services::TranscriptEvent::FinalSegment(_)))=>{},
                Some(Ok(crate::services::TranscriptEvent::Finished(_)))=>break Err(anyhow::anyhow!("ASR finalized before physical endpointing")),
                Some(Err(error))=>break Err(error.into()),
                None=>break Err(anyhow::anyhow!("ASR stream ended during audio capture")),
            }
        }
    };
    provider.cancel_and_wait().await;
    tracing::info!(robot_id=%context.robot_id,%session_id,elapsed_ms=started.elapsed().as_millis() as u64,
        success=result.is_ok(),"robot: incremental ASR capture finished");
    result
}

struct PendingTranscription {
    request_id: String,
    session_id: String,
    companion_id: String,
    wav: Vec<u8>,
    presented_question_id: Option<String>,
    work_target: Option<RobotTurnRequest>,
    media_generation:u64,
    captured_context:Option<crate::services::RobotInputContext>,
    prepared: Option<PreparedTranscription>,
}

struct ActiveTranscription {
    request_id: String,
    session_id: String,
    companion_id: String,
    presented_question_id: Option<String>,
    work_target: Option<RobotTurnRequest>,
    media_generation:u64,
    captured_context:Option<crate::services::RobotInputContext>,
    capture_task: Option<OwnedTask>,
    capture_cancellation: Option<tokio::sync::watch::Sender<bool>>,
    task: OwnedTask,
    result: oneshot::Receiver<anyhow::Result<Option<String>>>,
}

/// ASR owns no socket or speech projection. The actor remains the sole owner
/// of frame ordering, connection checks, transcript delivery and cancellation.
#[derive(Default)]
struct Transcriptions {
    pending: VecDeque<PendingTranscription>,
    active: Option<ActiveTranscription>,
}

impl Transcriptions {
    fn enqueue(&mut self, session_id: String, companion_id: String, wav: Vec<u8>, presented_question_id:Option<String>, work_target:Option<RobotTurnRequest>, prepared:Option<PreparedTranscription>,media_generation:u64,captured_context:Option<crate::services::RobotInputContext>) -> bool {
        if self.pending.len() >= MAX_PENDING_TRANSCRIPTIONS { return false; }
        self.pending.push_back(PendingTranscription {
            request_id: uuid::Uuid::now_v7().to_string(), session_id, companion_id, wav, presented_question_id, work_target, prepared,media_generation,captured_context,
        });
        true
    }

    fn start_next(&mut self, deps: &SessionDeps, robot_id: &str, tasks: &TaskScope,
        stopped: &tokio::sync::watch::Receiver<bool>) {
        if self.active.is_some() { return; }
        let Some(request) = self.pending.pop_front() else { return; };
        let (tx, result) = oneshot::channel();
        let context = SpeechContext {
            robot_id: robot_id.to_owned(), companion_id: request.companion_id.clone(),
        };
        let speech = deps.speech.clone();
        let registry = deps.registry.clone();
        let session_id = request.session_id.clone();
        let request_id = request.request_id.clone();
        let mut stopped = stopped.clone();
        let capture_task=request.prepared.as_ref().map(|prepared|prepared.task.clone());
        let capture_cancellation=request.prepared.as_ref().map(|prepared|prepared.cancellation.clone());
        tracing::info!(%robot_id, %session_id, %request_id, wav_bytes = request.wav.len(),
            queued = self.pending.len(), "robot: ASR delivery started");
        let task = tasks.spawn(async move {
            // Replacement of this connection or its Companion must also drop a
            // held provider future, without waiting for another device frame.
            let invalidated = async {
                loop {
                    if !registry.connection_matches(&context.robot_id, &session_id).await
                        || !registry.get(&context.robot_id).await.is_some_and(|record|
                            record.companion_id.as_deref() == Some(context.companion_id.as_str())) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            };
            let started = tokio::time::Instant::now();
            let outcome = tokio::select! {
                biased;
                _ = invalidated => Ok(None),
                result = async {
                    if let Some(prepared)=request.prepared {
                        let result=prepared.result.await.map_err(|_|anyhow::anyhow!("Robot capture task ended without a result"))?;
                        prepared.task.join().await.map_err(anyhow::Error::msg)?;
                        match result? {
                            CapturedTranscript::Transcript(text)=>return Ok(Some(text)),
                            CapturedTranscript::Cancelled=>return Ok(None),
                            CapturedTranscript::Batch=>{},
                        }
                    }
                    transcribe_until_stopped(speech.as_ref(), &context, request.wav, &mut stopped).await
                } => result,
            };
            tracing::info!(robot_id = %context.robot_id, %session_id, %request_id,
                elapsed_ms = started.elapsed().as_millis() as u64,
                success = outcome.is_ok(), cancelled = matches!(&outcome, Ok(None)),
                "robot: ASR delivery finished");
            let _ = tx.send(outcome);
        });
        self.active = Some(ActiveTranscription {
            request_id: request.request_id, session_id: request.session_id,
            companion_id: request.companion_id, presented_question_id:request.presented_question_id, work_target:request.work_target,
            media_generation:request.media_generation,
            captured_context:request.captured_context,
            capture_task,capture_cancellation,task, result,
        });
    }

    async fn receive(&mut self) -> anyhow::Result<Option<String>> {
        let active = self.active.as_mut().expect("ASR completion requires an active request");
        // Receiving a oneshot is cancellation-safe. Taking the task or joining
        // it belongs in the selected branch, so another ready device frame
        // cannot discard a consumed result while that join is still pending.
        (&mut active.result).await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Robot ASR task ended without a result")))
    }

    async fn cancel(&mut self, cleanup_errors: &mut Vec<String>) {
        for request in self.pending.drain(..) {
            if let Some(prepared)=request.prepared { prepared.cancel(cleanup_errors).await; }
        }
        if let Some(active) = self.active.take() {
            if let Some(cancel)=active.capture_cancellation { cancel.send_replace(true); }
            active.task.abort();
            if let Err(error) = active.task.join().await { cleanup_errors.push(error); }
            if let Some(task)=active.capture_task {
                if let Err(error)=task.join().await { cleanup_errors.push(error); }
            }
        }
    }
}

/// Everything a session actor needs from the host.
#[derive(Clone)]
pub struct SessionDeps {
    pub registry: Arc<RobotRegistry>,
    pub status: Arc<RobotStatusRegistry>,
    pub speech: Arc<dyn SpeechServices>,
    pub dispatcher: Arc<dyn CompanionTurnDispatcher>,
    /// Where discovered device tools are published for the canonical Robot
    /// Action owner.
    pub tools: Arc<crate::tool_registry::RobotToolRegistry>,
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
    async fn send_json_guarded(&self,msg:&ServerMessage,pacer:&DownlinkPacer,generation:u64,
        guard:Option<&crate::services::RobotOutputGuard>)->bool {
        let permit=tokio::select! {biased;
            _=pacer.generation_cancelled(generation)=>return false,
            permit=self.tx.reserve()=>match permit {Ok(permit)=>permit,Err(_)=>return false},
        };
        let lease=if let Some(guard)=guard {
            tokio::select! {biased;
                _=pacer.generation_cancelled(generation)=>return false,
                acquired=guard.acquire()=>match acquired {Ok(Some(lease))=>Some(lease),_=>{
                    pacer.flush_if_current(generation);return false;
                }},
            }
        }else{None};
        if pacer.generation()!=generation{return false;}
        permit.send(Frame::Text(serialize_server_message(msg)));
        drop(lease);true
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
}

/// A short line the DEVICE can display when the voice link cannot do its job,
/// so a missing or broken model reads as a problem on screen instead of as a
/// mute robot. It uses only the frames the firmware already renders — it shows
/// `sentence_start` text on the OLED — and plays no audio, so it still works
/// when TTS itself is the thing that is unconfigured.
///
/// The `sad` face here is STATE-driven: the gateway knows the voice link is
/// broken, so it is a fact, not a syntax parsed out of model text. That is the
/// difference between this send and the deleted per-sentence one.
async fn show_voice_notice(writer: &Writer, session_id: &str, text: &str) {
    writer
        .send_json(&ServerMessage::TtsStart {
            session_id: session_id.to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::Llm {
            session_id: session_id.to_owned(),
            emotion: "sad".to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::TtsSentence {
            session_id: session_id.to_owned(),
            text: text.to_owned(),
        })
        .await;
    writer
        .send_json(&ServerMessage::TtsStop {
            session_id: session_id.to_owned(),
        })
        .await;
}

/// A device-friendly notice for an ASR failure. The "not configured" case is the
/// overwhelmingly common one on a fresh install and gets an actionable line.
fn asr_notice_for(error: &anyhow::Error) -> String {
    if error.to_string().contains("no speech-recognition model configured") {
        "未配置语音识别模型，请在伙伴设置里选择 ASR 模型".to_owned()
    } else {
        "语音识别失败，请稍后再试".to_owned()
    }
}

/// A device-friendly notice for a TTS failure. The reply text is already on the
/// screen; this one extra line explains why there is no voice.
fn tts_notice_for(error: &anyhow::Error) -> String {
    if error.to_string().contains("no speech-synthesis model configured") {
        "（未配置语音合成模型，暂时只显示文字）".to_owned()
    } else {
        "（语音合成失败，暂时只显示文字）".to_owned()
    }
}

/// Speak one sentence: screen, then audio. Returns a failure only when the turn
/// should be abandoned (a run of TTS errors), never for a single bad sentence —
/// that one is still on the device's screen.
///
/// Spoken text drives no facial expression. Conversational expressions travel
/// through the Session's granted device MCP tools. `ServerMessage::Llm` remains
/// a device/turn state notice (`show_voice_notice`, `report_turn_failure`), not
/// an alternate control channel parsed from speech text.
struct QuestionPublication {
    callback: Option<(Arc<dyn CompanionTurnDispatcher>,RobotTurnRequest)>,
    source:nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:String,
    presented:Arc<std::sync::Mutex<Option<String>>>,
}
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
    question: Option<&QuestionPublication>,
    output_guard:Option<&crate::services::RobotOutputGuard>,
) -> Option<TurnFailure> {
    if pacer.generation()!=generation {return None;}
    // Screen copy and voice copy get the same cleaning: bracketed stage
    // directions removed, plus emoji / non-speech symbols dropped. Sending emoji
    // or a bracketed annotation to the TTS engine is what 400s and cuts the
    // voice, and the device's 14px CJK font has no glyph for them either — an
    // emoji left in the screen copy draws a hollow box. The gateway owns the wire
    // format, so it cleans both rather than letting the firmware carry a second
    // sanitiser.
    let display = sanitize_for_display(sentence);
    let voice_text = sanitize_for_speech(sentence);
    if display.trim().is_empty() && voice_text.is_empty() {
        // Nothing survived the guard (an emoji- or annotation-only chunk). There
        // is nothing to show and nothing to say, so there is nothing to do.
        return None;
    }
    if !*speaking {
        if !writer
            .send_json_guarded(&ServerMessage::TtsStart {
                session_id: session_id.to_owned(),
            },pacer,generation,output_guard)
            .await {return None;}
        *speaking = true;
    }
    if !display.trim().is_empty() {
        if !writer
            .send_json_guarded(&ServerMessage::TtsSentence {
                session_id: session_id.to_owned(),
                text: display.clone(),
            },pacer,generation,output_guard)
            .await {return None;}
    }
    if let Some(question)=question {
        // The displayed sentence entered this generation's writer before a
        // canonical presentation is acknowledged. A stop that won either
        // side of the awaited owner callback cannot publish an answer target.
        if pacer.generation()!=generation {return None;}
        if let Some((dispatcher,request))=&question.callback {
            if let Err(error)=dispatcher.question_presented(request,&question.source,&question.presentation_id).await {
                tracing::debug!(%error,"robot: original question presentation authority was not retained");
                pacer.flush_if_current(generation);
                return None;
            }
        }
        if pacer.generation()!=generation {return None;}
        *question.presented.lock().unwrap_or_else(|error|error.into_inner())=Some(question.presentation_id.clone());
    }
    // Nothing speakable remained (e.g. the sentence was only an emoji): it is on
    // screen, but calling TTS with empty/symbol-only text is exactly what fails
    // and breaks the audio stream. Skip synthesis — this is not a failure, so the
    // consecutive-failure streak resets and real audio keeps flowing.
    if voice_text.is_empty() {
        *tts_failures = 0;
        return None;
    }
    let synthesis=async {
        let started=tokio::time::Instant::now();
        let mut audio=speech.synthesize_stream(ctx,&voice_text).await?;
        let mode=audio.mode;let mut first_audio=true;
        let mut pending=Vec::new();
        while let Some(chunk)=audio.audio.next().await {
            if pacer.generation()!=generation {return Ok::<_,anyhow::Error>(());}
            let chunk=chunk?;
            // Preserve frame boundaries across provider fragments. Padding
            // every HTTP chunk would insert silence throughout the sentence.
            pending.extend(crate::audio::resample_linear(&chunk.pcm,chunk.sample_rate,crate::protocol::DOWNLINK_SAMPLE_RATE));
            let complete=pending.len()/crate::audio::DOWNLINK_FRAME_SAMPLES*crate::audio::DOWNLINK_FRAME_SAMPLES;
            if complete>0 {
                let frames=encoder.encode_frames(&pending[..complete])?;
                pending.drain(..complete);
                if first_audio {
                    first_audio=false;
                    tracing::info!(robot_id=%ctx.robot_id,%session_id,generation,delivery_mode=?mode,
                        elapsed_ms=started.elapsed().as_millis() as u64,"robot: first TTS PCM ready for paced delivery");
                }
                pacer.enqueue_guarded(generation,frames,output_guard.cloned()).await;
            }
        }
        if !pending.is_empty() {pacer.enqueue_guarded(generation,encoder.encode_frames(&pending)?,output_guard.cloned()).await;}
        Ok(())
    };
    let result=tokio::select! {biased;
        _=pacer.generation_cancelled(generation)=>return None,
        result=synthesis=>result,
    };
    match result {
        Ok(()) => { *tts_failures = 0; None }
        Err(error) => {
            *tts_failures += 1;
            tracing::warn!(
                %error,
                tts_failures = *tts_failures,
                "robot: TTS failed, sentence stays on screen only"
            );
            // First failure this run: the reply text is already on the OLED, so
            // add one line explaining why there is no voice. Once per run, so a
            // long fully-muted reply does not spam the screen.
            if *tts_failures == 1 {
                writer
                    .send_json_guarded(&ServerMessage::TtsSentence {
                        session_id: session_id.to_owned(),
                        text: tts_notice_for(&error),
                    },pacer,generation,output_guard)
                    .await;
            }
            (*tts_failures >= MAX_CONSECUTIVE_TTS_FAILURES).then(|| TurnFailure {
                message: format!("TTS failed {tts_failures} times: {error}", tts_failures = *tts_failures),
                provider_fault: true,
            })
        }
    }
}

/// Stream a reply to the device: split into sentences, then speak.
#[allow(clippy::too_many_arguments)]
async fn drive_turn(
    mut events: mpsc::Receiver<TurnEvent>,
    ctx: SpeechContext,
    speech: Arc<dyn SpeechServices>,
    pacer: Arc<DownlinkPacer>,
    writer: Writer,
    session_id: String,
    generation: u64,
    presented_question: Arc<std::sync::Mutex<Option<String>>>,
    question_delivery: Option<(Arc<dyn CompanionTurnDispatcher>,RobotTurnRequest)>,
    output_guard:Option<crate::services::RobotOutputGuard>,
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
            };
        }
    };
    let mut speaking = false;
    let mut tts_failures = 0usize;
    let mut failure: Option<TurnFailure> = None;

    while let Some(event) = events.recv().await {
        match event {
            TurnEvent::Text(_) | TurnEvent::PublicSpeech {..} | TurnEvent::Question {..} => {
                let (chunk,committed,question)=match event {
                    TurnEvent::Text(chunk)=>(chunk,false,None),
                    TurnEvent::PublicSpeech {source,text}=>{
                        if let Err(error)=source.validate(){failure=Some(TurnFailure {message:error,provider_fault:false});break;}
                        (text,true,None)
                    },
                    TurnEvent::Question {source,text,presentation_id}=>{
                        if let Err(error)=source.validate(){failure=Some(TurnFailure {message:error,provider_fault:false});break;}
                        if pacer.generation()!=generation {continue;}
                        let question=QuestionPublication {callback:question_delivery.clone(),source,presentation_id,presented:presented_question.clone()};
                        (text,true,Some(question))
                    },_=>unreachable!(),
                };
                let mut sentences=splitter.push(&chunk);
                if committed && let Some(tail)=splitter.flush(){sentences.push(tail);}
                for (index,sentence) in sentences.into_iter().enumerate() {
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
                        if index==0 {question.as_ref()}else{None},
                        output_guard.as_ref(),
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
                        None,
                        output_guard.as_ref(),
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
        if !pacer.enqueue_text_and_wait(generation, stop).await {
            // A source lease can revoke this generation during the final
            // barrier. This actor still owns the media until drive_turn
            // returns, so its gateway stop cannot target a later owner.
            writer
                .send_json(&ServerMessage::TtsStop {
                    session_id: session_id.clone(),
                })
                .await;
        }
    }
    TurnOutcome {
        failed: failure,
    }
}

/// Tell the user something went wrong without stranding the device in `speaking`.
///
/// The `sad` face is STATE-driven, like `show_voice_notice`'s: the turn failed,
/// which the gateway knows for a fact. Model text drives no expression.
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
    turn_tx: &mpsc::Sender<(String, TurnOutcome)>,
    turn_task: &mut Option<OwnedTask>,
    active_request: &mut Option<RobotTurnRequest>,
    robot_id: &str,
    companion_id: &str,
    conversation_id: &str,
    session_id: &str,
    text: &str,
    presented_question_id:Option<String>,
    captured_context:Option<crate::services::RobotInputContext>,
    presented_question: Arc<std::sync::Mutex<Option<String>>>,
    tasks: &TaskScope,
    stopped: &tokio::sync::watch::Receiver<bool>,
    cleanup_errors: &mut Vec<String>,
) {
    if *stopped.borrow() { return; }
    let request = RobotTurnRequest {
        robot_id: robot_id.to_owned(), companion_id: companion_id.to_owned(),
        conversation_id: conversation_id.to_owned(), connection_id: session_id.to_owned(),
        request_id: uuid::Uuid::now_v7().to_string(), text: text.to_owned(), presented_question_id,captured_context,
    };
    if request.presented_question_id.is_some() {
        let mut current=presented_question.lock().unwrap_or_else(|error|error.into_inner());
        if *current==request.presented_question_id {*current=None;}
    }
    let request_id = request.request_id.clone();
    let source_request=request.clone();
    *active_request = Some(request.clone());
    let events = match deps
        .dispatcher
        .dispatch(request)
        .await
    {
        Ok(rx) => rx,
        Err(error) => {
            tracing::error!(%robot_id, %error, "robot: dispatch failed");
            let _ = turn_tx
                .send((request_id, TurnOutcome {
                    failed: Some(TurnFailure {
                        message: error.to_string(),
                        provider_fault: false,
                    }),
                }))
                .await;
            return;
        }
    };
    // Dispatch may have admitted a canonical Turn before stop. Keep that
    // future's result, then cancel the exact admitted request; dropping it or
    // starting the speech projection would lose its ownership boundary.
    if *stopped.borrow() {
        if let Some(request) = active_request.take()
            && let Err(error) = deps.dispatcher.cancel(&request).await
        {
            cleanup_errors.push(format!("Robot admitted turn cancellation failed: {error}"));
        }
        return;
    }
    let ctx = SpeechContext {
        robot_id: robot_id.to_owned(),
        companion_id: companion_id.to_owned(),
    };
    let generation = pacer.flush();
    let (speech, pacer, writer, session_id, turn_tx) = (
        deps.speech.clone(),
        pacer.clone(),
        writer.clone(),
        session_id.to_owned(),
        turn_tx.clone(),
    );
    let registry = deps.registry.clone();
    let dispatcher=deps.dispatcher.clone();
    let output_request=source_request.clone();
    let output_guard=crate::services::RobotOutputGuard::new(dispatcher.clone(),source_request.clone());
    *turn_task = Some(tasks.spawn(async move {
        let revoked_question=presented_question.clone();
        let watch = async {
            loop {
                if !registry.connection_matches(&ctx.robot_id, &session_id).await
                    || !dispatcher.input_context_matches(&output_request).await.unwrap_or(false)
                    || !registry.get(&ctx.robot_id).await.is_some_and(|record| record.companion_id.as_deref() == Some(ctx.companion_id.as_str())) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        let outcome = tokio::select! {
            outcome = drive_turn(events, ctx.clone(), speech, pacer.clone(), writer.clone(), session_id.clone(), generation, presented_question,Some((dispatcher.clone(),source_request)),Some(output_guard)) => outcome,
            _ = watch => {
                pacer.flush();
                // No successor media owner can start until this task returns
                // its outcome, so this is still the original source's cut.
                *revoked_question.lock().unwrap_or_else(|error|error.into_inner())=None;
                writer.send_json(&ServerMessage::TtsStop { session_id: session_id.clone() }).await;
                TurnOutcome { failed: None }
            }
        };
        let _ = turn_tx.send((request_id, outcome)).await;
    }));
}

/// Run one robot session to completion. Returns when the inbound stream ends.
pub async fn run_session(link: AcceptedLink, deps: SessionDeps) {
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let tasks = Arc::new(OwnedTasks::default());
    if let Err(error) = run_session_owned(link, deps, stopped, tasks.clone()).await {
        tasks.record_error(error);
    }
    if let Err(error) = tasks.abort_and_join().await { tracing::error!(%error, "robot session stopped uncleanly"); }
}

pub(crate) async fn run_session_owned(
    link: AcceptedLink,
    deps: SessionDeps,
    mut stopped: tokio::sync::watch::Receiver<bool>,
    owned: Arc<OwnedTasks>,
) -> Result<(), String> {
    let AcceptedLink {
        identity,
        sink,
        mut stream,
    } = link;
    let robot_id = identity.robot_id.clone();    let (writer, writer_task) = Writer::spawn(sink);
    let writer_task = owned.track(writer_task);
    let tasks = TaskScope::new(owned);
    let mut cleanup_errors = Vec::new();

    let mut session_id: Option<String> = None;
    let mut companion_id: Option<String> = None;
    let mut uplink: Option<UplinkPipeline> = None;
    let mut mcp: Option<Arc<crate::mcp_bridge::RobotMcpClient>> = None;
    let mut discovery_task: Option<OwnedTask> = None;
    let mut conversation_id: Option<String> = None;
    let (pacer, pacer_task) = DownlinkPacer::spawn(writer.tx.clone());
    let pacer_task = tasks.track(pacer_task);
    let pacer = Arc::new(pacer);
    let (turn_tx, mut turn_rx) = mpsc::channel::<(String, TurnOutcome)>(4);
    let mut turn_task: Option<OwnedTask> = None;
    let mut active_request: Option<RobotTurnRequest> = None;
    let mut active_playback: Option<String> = None;
    let mut playback: Option<mpsc::Receiver<crate::registry::RobotPlaybackCommand>> = None;
    let mut playback_sources = CanonicalPlaybackSources::default();
    let mut utterance_queue = std::collections::VecDeque::<(String,Option<String>,Option<crate::services::RobotInputContext>)>::new();
    // A question is attached when physical recording starts, before ASR or
    // queueing can delay an utterance past a later question presentation.
    let presented_question=Arc::new(std::sync::Mutex::new(None::<String>));
    let mut recording_question_id:Option<String>=None;
    let mut recording_work_target:Option<RobotTurnRequest>=None;
    let mut recording_media_generation=pacer.generation();
    let mut recording_context=None;
    let mut recording_capture:Option<PreparedTranscription>=None;
    // Auto/realtime VAD cuts another utterance within the same device listen
    // window. Freeze its question at the next speech onset, not idle silence or
    // Turn completion, and never let a late Done reopen a stopped microphone.
    let mut awaiting_next_utterance = false;
    let mut transcriptions = Transcriptions::default();
    let mut ping = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.tick().await; // the first tick is immediate; skip it

    loop {
        if *stopped.borrow_and_update() { break; }
        if let Some(sid) = session_id.as_deref()
            && !deps.registry.connection_matches(&robot_id, sid).await { break; }
        transcriptions.start_next(&deps, &robot_id, &tasks, &stopped);
        if active_request.is_none() && turn_task.is_none()
            && let (Some(conversation), Some(sid), Some(bound)) = (
                conversation_id.as_deref(), session_id.as_deref(), companion_id.as_deref(),
            )
            && let Some((text,question_id,captured_context)) = utterance_queue.pop_front()
        {
            start_turn(&deps, &pacer, &writer, &turn_tx, &mut turn_task, &mut active_request,
                &robot_id, bound, conversation, sid, &text, question_id,captured_context, presented_question.clone(), &tasks, &stopped, &mut cleanup_errors).await;
        }
        // Set by whichever branch decided the user stopped talking; handled at
        // the end of the iteration so the read loop stays one flat match.
        let mut utterance: Option<Vec<u8>> = None;
        let asr_active = transcriptions.active.is_some();

        tokio::select! {
            biased;
            changed = stopped.changed() => {
                if changed.is_err() { cleanup_errors.push("Robot session stop channel was lost".into()); }
                break;
            }
            command = async {
                match playback.as_mut() { Some(receiver) => receiver.recv().await, None => std::future::pending().await }
            } => {
                let Some(command) = command else { playback = None; continue; };
                if command.accepted.is_closed() { continue; }
                let (Some(sid), Some(bound)) = (session_id.clone(), companion_id.clone()) else {
                    let _ = command.accepted.send(Err(RobotPlaybackError::Unavailable)); continue;
                };
                let record = deps.registry.get(&robot_id).await;
                let requested_response=command.requested_response;
                let Some(record) = record.filter(|record| (requested_response||record.permissions.proactive_speech)
                    && record.companion_id.as_deref() == Some(bound.as_str())) else {
                    let _ = command.accepted.send(Err(RobotPlaybackError::PermissionDenied)); continue;
                };
                if let Some(source) = &command.source {
                    match playback_sources.already_accepted(source, &command.text,command.question_presentation_id.as_deref()) {
                        Ok(true) => { let _ = command.accepted.send(Ok(())); continue; }
                        Err(error) => { let _ = command.accepted.send(Err(error)); continue; }
                        Ok(false) => {}
                    }
                }
                if turn_task.is_some() || active_request.is_some() {
                    let _ = command.accepted.send(Err(RobotPlaybackError::Busy)); continue;
                }
                if !deps.registry.connection_matches(&robot_id, &sid).await {
                    let _ = command.accepted.send(Err(RobotPlaybackError::Unavailable)); continue;
                }
                let playback_context=command.context.map(|context|RobotTurnRequest {robot_id:robot_id.clone(),companion_id:bound.clone(),
                    conversation_id:context.session_id,connection_id:sid.clone(),request_id:"canonical-playback".into(),text:String::new(),
                    presented_question_id:None,captured_context:Some(context.input_context)});
                if let Some(context)=&playback_context
                    && !deps.dispatcher.input_context_matches(context).await.unwrap_or(false) {
                    let _=command.accepted.send(Err(RobotPlaybackError::Unavailable));continue;
                }
                if let Some(source) = &command.source { playback_sources.record(source.clone(), &command.text,command.question_presentation_id.clone()); }
                let request_id = uuid::Uuid::now_v7().to_string();
                active_playback = Some(request_id.clone());
                let question_authority=match (&command.source,&command.question_presentation_id) {
                    (Some(source),Some(presentation_id))=>Some(crate::services::RobotQuestionPlaybackAuthority {
                        source:source.clone(),presentation_id:presentation_id.clone(),
                    }),_=>None,
                };
                let events_to_play=if command.replay_segments.is_empty() {
                    vec![match (command.source,command.question_presentation_id) {
                        (Some(source),Some(presentation_id))=>TurnEvent::Question {source,text:command.text,presentation_id},
                        (Some(source),None)=>TurnEvent::PublicSpeech {source,text:command.text},
                        (None,_)=>TurnEvent::Text(command.text),
                    }]
                }else {
                    command.replay_segments.into_iter().map(|(source,text)|TurnEvent::PublicSpeech {source,text}).collect::<Vec<_>>()
                };
                let (events_tx, events) = mpsc::channel(events_to_play.len()+1);
                for event in events_to_play {let _=events_tx.try_send(event);}
                let _ = events_tx.try_send(TurnEvent::Done);
                drop(events_tx);
                let (speech, pacer, writer, registry, robot_id, turn_tx) = (
                    deps.speech.clone(), pacer.clone(), writer.clone(), deps.registry.clone(), robot_id.clone(), turn_tx.clone(),
                );
                let presented_question=presented_question.clone();
                let dispatcher=deps.dispatcher.clone();
                let question_delivery=playback_context.as_ref().filter(|_|question_authority.is_some())
                    .map(|request|(dispatcher.clone(),request.clone()));
                let playback_question=question_authority.as_ref().map(|authority|authority.presentation_id.clone());
                let output_guard=playback_context.as_ref().map(|request|if requested_response {
                    crate::services::RobotOutputGuard::new(dispatcher.clone(),request.clone())
                }else {crate::services::RobotOutputGuard::new_proactive(dispatcher.clone(),request.clone(),record.authorization_revision)})
                    .map(|guard|match question_authority {Some(authority)=>guard.with_question(authority),None=>guard});
                let generation=pacer.flush();
                deps.status.publish_for_connection(&deps.registry, &robot_id, session_id.as_deref(), Some(&bound), RobotPhase::Speaking, now_ms()).await;
                let _ = command.accepted.send(Ok(()));
                turn_task = Some(tasks.spawn(async move {
                    let revoked_question=presented_question.clone();
                    let watched_guard=output_guard.clone();
                    let watch = async {
                        loop {
                            let valid = registry.get(&robot_id).await.is_some_and(|current| (requested_response||current.permissions.proactive_speech)
                                && (requested_response||current.authorization_revision == record.authorization_revision)
                                && current.companion_id.as_deref() == Some(bound.as_str()))
                                && registry.connection_matches(&robot_id, &sid).await;
                            let valid=valid&&match playback_context.as_ref() {Some(context)=>dispatcher.input_context_matches(context).await.unwrap_or(false),None=>true};
                            let valid=valid&&match watched_guard.as_ref() {
                                Some(guard)=>guard.acquire().await.is_ok_and(|lease|lease.is_some()),None=>true,
                            };
                            if !valid { break; }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    };
                    let outcome = tokio::select! {
                        outcome = drive_turn(events, SpeechContext { robot_id: robot_id.clone(), companion_id: bound.clone() },
                            speech, pacer.clone(), writer.clone(), sid.clone(), generation, presented_question,question_delivery,output_guard) => outcome,
                        _ = watch => {
                            pacer.flush();
                            if let Some(question)=playback_question {
                                let mut presented=revoked_question.lock().unwrap_or_else(|error|error.into_inner());
                                if presented.as_deref()==Some(question.as_str()) {*presented=None;}
                            }
                            writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                            // The original source was intentionally revoked.
                            // End this media owner quietly; a generic provider
                            // failure notice would itself outlive its scope.
                            TurnOutcome { failed: None }
                        }
                    };
                    let _ = turn_tx.send((request_id, outcome)).await;
                }));
            }
            _ = ping.tick() => {
                if let Some(sid) = &session_id {
                    writer.send_json(&ServerMessage::Ping { session_id: sid.clone() }).await;
                }
            }
            outcome = turn_rx.recv() => {
                let Some((request_id, outcome)) = outcome else { continue };
                if let Some(sid) = session_id.as_deref()
                    && !deps.registry.connection_matches(&robot_id, sid).await { break; }
                if !active_request.as_ref().is_some_and(|request| request.request_id == request_id)
                    && active_playback.as_deref() != Some(request_id.as_str()) {
                    continue;
                }
                active_request = None;
                active_playback = None;
                if let Some(task) = turn_task.take() {
                    if let Err(error) = task.join().await { cleanup_errors.push(error); }
                }
                let Some(failure) = outcome.failed else {
                    if let Some(bound) = &companion_id {
                        deps.status.publish_for_connection(&deps.registry, &robot_id, session_id.as_deref(), Some(bound), RobotPhase::Idle, now_ms()).await;
                    }
                    continue;
                };
                tracing::warn!(%robot_id, message = %failure.message, "robot: turn failed");
                if let Some(sid) = &session_id {
                    report_turn_failure(&writer, sid).await;
                }
                if let Some(bound) = &companion_id {
                    deps.status.publish_for_connection(&deps.registry, &robot_id, session_id.as_deref(), Some(bound), RobotPhase::Idle, now_ms()).await;
                }
            }
            outcome = transcriptions.receive(), if asr_active => {
                let request = transcriptions.active.take().expect("completed ASR remains owned");
                if let Err(error) = request.task.join().await { cleanup_errors.push(error); }
                if *stopped.borrow() { break; }
                if session_id.as_deref() != Some(request.session_id.as_str())
                    || companion_id.as_deref() != Some(request.companion_id.as_str())
                    || !deps.registry.connection_matches(&robot_id, &request.session_id).await {
                    continue;
                }
                let sid = request.session_id;
                let bound = request.companion_id;
                let transcript = match outcome {
                    Ok(Some(text)) => text,
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(%robot_id, session_id = %sid, request_id = %request.request_id,
                            %error, "robot: ASR failed");
                        show_voice_notice(&writer, &sid, &asr_notice_for(&error)).await;
                        deps.status.publish_for_connection(&deps.registry, &robot_id,
                            Some(&sid), Some(&bound), RobotPhase::Idle, now_ms()).await;
                        continue;
                    }
                };
                if transcript.trim().is_empty() {
                    // Preserve the existing quiet recovery: no Agent or TTS
                    // synthesis for an empty transcript, only reset listening.
                    tracing::debug!(%robot_id, session_id = %sid, request_id = %request.request_id,
                        "robot: ASR returned an empty transcript");
                    writer.send_json(&ServerMessage::TtsStart { session_id: sid.clone() }).await;
                    writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                    deps.status.publish_for_connection(&deps.registry, &robot_id,
                        Some(&sid), Some(&bound), RobotPhase::Idle, now_ms()).await;
                    continue;
                }
                writer.send_json(&ServerMessage::Stt {
                    session_id: sid.clone(), text: transcript.clone(),
                }).await;
                match spoken_intent(&transcript) {
                    SpokenIntent::StopPlayback=>{
                        // Media cancellation only. Continue observing the
                        // original work; do not turn barge-in into tool cancel.
                        if request.media_generation==pacer.generation() {
                            pacer.flush();writer.send_json(&ServerMessage::TtsStop {session_id:sid.clone()}).await;
                        }
                        continue;
                    },
                    SpokenIntent::CancelWork=>{
                        let explicit_background=explicit_background_control(&transcript);
                        if !explicit_background && let Some(target)=request.work_target.as_ref() {
                            if let Err(error)=deps.dispatcher.cancel(target).await {tracing::warn!(%error,"robot: exact spoken work cancellation failed");}
                            if active_request.as_ref().is_some_and(|active|active.request_id==target.request_id) {
                                pacer.flush();writer.send_json(&ServerMessage::TtsStop {session_id:sid.clone()}).await;
                            }
                        }else if !explicit_background && request.captured_context.as_ref().is_some_and(|context|context.work_operation_id.is_some()&&context.background_work.is_some()) {
                            show_voice_notice(&writer,&sid,"当前入口和后台都有运行任务，请明确说出要取消的任务").await;
                        }else if request.captured_context.as_ref().is_some_and(|context|context.background_work.is_some()&&!context.background_work_ambiguous) {
                            if let (Some(conversation),Some(context))=(conversation_id.as_ref(),request.captured_context.clone()) {
                                let control=RobotTurnRequest {robot_id:robot_id.clone(),companion_id:bound.clone(),conversation_id:conversation.clone(),connection_id:sid.clone(),
                                    request_id:request.request_id.clone(),text:transcript.clone(),presented_question_id:request.presented_question_id.clone(),captured_context:Some(context)};
                                if let Err(error)=deps.dispatcher.cancel_background_work(&control).await {
                                    tracing::warn!(%error,"robot: recorded background cancellation was not applied");
                                    show_voice_notice(&writer,&sid,"原后台任务已发生变化，本次没有取消其他任务").await;
                                }
                            }
                        }else if request.captured_context.as_ref().is_some_and(|context|context.background_work_ambiguous) {
                            show_voice_notice(&writer,&sid,"当前有多个后台任务，请说出任务名称后再取消").await;
                        }else {show_voice_notice(&writer,&sid,"录音开始时没有可取消的当前任务").await;}
                        continue;
                    },
                    intent @ (SpokenIntent::Supplement|SpokenIntent::Correction)=>{
                        let explicit_background=explicit_background_control(&transcript);
                        if request.presented_question_id.is_some()&&!explicit_background {
                            // Information supplied to the exact presented
                            // question follows canonical reply handling.
                        }else if !explicit_background && let Some(target)=request.work_target.as_ref() {
                            if let Err(error)=deps.dispatcher.supplement(target,&transcript,intent==SpokenIntent::Correction,&request.request_id).await {
                                tracing::warn!(%error,"robot: exact work supplement was not applied");
                                show_voice_notice(&writer,&sid,"原任务已结束或发生变化，请重新发起请求").await;
                            }
                            continue;
                        }else if !explicit_background && request.captured_context.as_ref().is_some_and(|context|context.work_operation_id.is_some()&&context.background_work.is_some()) {
                            show_voice_notice(&writer,&sid,"当前入口和后台都有运行任务，请明确说出要修改的任务").await;continue;
                        }else if request.captured_context.as_ref().is_some_and(|context|context.background_work.is_some()&&!context.background_work_ambiguous) {
                            let exact=request.captured_context.as_ref().and_then(|context|context.background_work.as_ref())
                                .is_some_and(|target|target.attempt.is_some()&&!target.attempt_ambiguous);
                            if exact {
                                if let (Some(conversation),Some(context))=(conversation_id.as_ref(),request.captured_context.clone()) {
                                    let control=RobotTurnRequest {robot_id:robot_id.clone(),companion_id:bound.clone(),conversation_id:conversation.clone(),connection_id:sid.clone(),
                                        request_id:request.request_id.clone(),text:transcript.clone(),presented_question_id:None,captured_context:Some(context)};
                                    if let Err(error)=deps.dispatcher.supplement_background_work(&control,&transcript,intent==SpokenIntent::Correction,&request.request_id).await {
                                        tracing::warn!(%error,"robot: recorded background supplement was not applied");
                                        show_voice_notice(&writer,&sid,"原后台步骤已发生变化，本次没有修改其他任务").await;
                                    }
                                }
                            }else {show_voice_notice(&writer,&sid,"当前后台任务没有唯一可修改的运行步骤，请说明要修改的任务").await;}
                            continue;
                        }else if request.captured_context.as_ref().is_some_and(|context|context.background_work_ambiguous) {
                            show_voice_notice(&writer,&sid,"当前有多个后台任务，请说出要补充或更正的任务名称").await;continue;
                        }
                    },SpokenIntent::NewRequest=>{},
                }
                deps.status.publish_for_connection(&deps.registry, &robot_id,
                    Some(&sid), Some(&bound), RobotPhase::Speaking, now_ms()).await;
                if utterance_queue.len() < 16 {
                    let question=if explicit_new_request(&transcript){None}else{request.presented_question_id};
                    utterance_queue.push_back((transcript,question,request.captured_context));
                } else {
                    show_voice_notice(&writer, &sid, "等待处理的消息过多，请稍后再说").await;
                }
            }
            frame = stream.next() => {
                let Some(frame) = frame else { break };
                let Ok(frame) = frame else { break };
                if let Some(sid) = session_id.as_deref()
                    && !deps.registry.connection_matches(&robot_id, sid).await { break; }
                match frame {
                    Frame::Binary(_) if session_id.is_none() => {
                        // Wake-word audio can arrive before `listen start`; before
                        // the handshake it is simply noise.
                        continue;
                    }
                    Frame::Binary(bytes) => {
                        let Some(pipeline) = uplink.as_mut() else { continue };
                        let question_at_packet_start=presented_question.lock().unwrap_or_else(|error|error.into_inner()).clone();
                        let outcome=pipeline.push_packet(&bytes);
                        if awaiting_next_utterance && pipeline.is_speech_started() {
                            recording_question_id=question_at_packet_start;
                            recording_work_target=active_request.clone();
                            recording_media_generation=pacer.generation();
                            recording_context=None;
                            if let (Some(bound),Some(conversation),Some(sid))=(companion_id.as_deref(),conversation_id.as_deref(),session_id.as_deref()) {
                                recording_context=match deps.dispatcher.capture_input_context(bound,&robot_id,conversation,sid,recording_question_id.clone()).await {
                                    Ok(context)=>context,Err(error)=>{tracing::warn!(%error,"Robot input context could not be captured");pipeline.abort();continue;}
                                };
                                if let Some(context)=&recording_context {
                                    recording_question_id=context.presented_question_id.clone();
                                    if let Some(target)=recording_work_target.as_mut(){target.captured_context=Some(context.clone());}
                                }
                            }
                            awaiting_next_utterance=false;
                            if let (Some(sid),Some(bound))=(session_id.as_deref(),companion_id.as_deref()) {
                                let mut capture=PreparedTranscription::start(&deps,&robot_id,sid,bound,&tasks,&stopped);
                                capture.push(pipeline.buffered_pcm().to_vec());
                                recording_capture=Some(capture);
                                pipeline.take_decoded_frame(); // The onset is already included in the retained lead-in.
                            }
                        }
                        if let (Some(capture),Some(frame))=(recording_capture.as_mut(),pipeline.take_decoded_frame()) { capture.push(frame); }
                        if let UplinkOutcome::Utterance(wav) = outcome {
                            utterance = Some(wav);
                            awaiting_next_utterance=pipeline.is_active();
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
                                // A repeated hello creates a new connection
                                // identity; old audio/results cannot cross it.
                                transcriptions.cancel(&mut cleanup_errors).await;
                                if let Some(capture)=recording_capture.take() { capture.cancel(&mut cleanup_errors).await; }
                                utterance_queue.clear();
                                *presented_question.lock().unwrap_or_else(|error|error.into_inner())=None;
                                recording_question_id=None;
                                recording_work_target=None;
                                awaiting_next_utterance=false;
                                playback_sources=CanonicalPlaybackSources::default();
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
                                if deps.registry.connect(&robot_id, &bound, &sid).await.is_err() {
                                    break;
                                }
                                playback = deps.registry.open_playback(&robot_id, &sid).await.ok();
                                writer.send_json(&ServerMessage::Hello { session_id: sid.clone() }).await;
                                deps.status
                                    .publish_for_connection(&deps.registry, &robot_id, Some(&sid), Some(&bound), RobotPhase::Idle, now_ms())
                                    .await;
                                let tuning = deps.dispatcher.vad_tuning(&bound).await;
                                // The profile picks the engine; `build_engine`
                                // owns the fallback when the named one cannot
                                // load, so an unavailable engine costs accuracy,
                                // never the voice link.
                                let named = deps.dispatcher.vad_engine(&bound).await;
                                let engine = build_engine(&named, tuning);
                                tracing::info!(
                                    %robot_id,
                                    requested = %named,
                                    vad = engine.name(),
                                    "robot: endpointer ready"
                                );
                                uplink = UplinkPipeline::new(engine).ok();
                                conversation_id = deps
                                    .dispatcher
                                    .ensure_companion_session(&bound, &robot_id)
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
                                    let vision_base = identity.vision_base.clone();
                                    let device_token = identity.device_token.clone();
                                    let discovering = robot_id.clone();
                                    let tool_registry = deps.tools.clone();
                                    discovery_task = Some(tasks.spawn(async move {
                                        let url = vision_base
                                            .map(|base| format!("{base}{}", crate::endpoint::VISION_PATH));
                                        if let Err(error) =
                                            client.initialize(url.as_deref(), &device_token).await
                                        {
                                            tracing::warn!(robot_id = %discovering, %error, "robot: MCP initialize failed");
                                            return;
                                        }
                                        match client.list_tools().await {
                                            Ok(tools) => {
                                                tracing::info!(
                                                    robot_id = %discovering,
                                                    count = tools.len(),
                                                    names = ?tools.iter().map(|t| &t.exposed_name).collect::<Vec<_>>(),
                                                    "robot: device tools discovered"
                                                );
                                                // Publishing here is what makes the
                                                // tools reachable from the model, via
                                                // the canonical Robot Action owner.
                                                tool_registry
                                                    .attach(&discovering, client.clone(), tools)
                                                    .await;
                                            }
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
                                        if let Some(capture)=recording_capture.take() {capture.cancel(&mut cleanup_errors).await;}
                                        let manual=matches!(mode,Some(ListeningMode::Manual));
                                        recording_question_id=presented_question.lock().unwrap_or_else(|error|error.into_inner()).clone();
                                        recording_work_target=active_request.clone();
                                        recording_media_generation=pacer.generation();
                                        recording_context=None;
                                        if manual && let (Some(bound),Some(conversation),Some(sid))=(companion_id.as_deref(),conversation_id.as_deref(),session_id.as_deref()) {
                                            recording_context=match deps.dispatcher.capture_input_context(bound,&robot_id,conversation,sid,recording_question_id.clone()).await {
                                                Ok(context)=>context,Err(error)=>{tracing::warn!(%error,"Robot input context could not be captured");pipeline.abort();continue;}
                                            };
                                            if let Some(context)=&recording_context {
                                                recording_question_id=context.presented_question_id.clone();
                                                if let Some(target)=recording_work_target.as_mut(){target.captured_context=Some(context.clone());}
                                            }
                                        }
                                        if let Some(capture)=recording_capture.take() { capture.cancel(&mut cleanup_errors).await; }
                                        if manual && let (Some(sid),Some(bound))=(session_id.as_deref(),companion_id.as_deref()) {
                                            recording_capture=Some(PreparedTranscription::start(&deps,&robot_id,sid,bound,&tasks,&stopped));
                                        }
                                        // In continuous modes a listen window
                                        // can be idle for minutes. Authority,
                                        // question and ASR ownership start at
                                        // real VAD speech onset, not silence.
                                        awaiting_next_utterance=!manual;
                                        pipeline.begin(mode.unwrap_or(ListeningMode::Auto));
                                        if let Some(bound) = &companion_id {
                                            deps.status
                                                .publish_for_connection(&deps.registry, &robot_id, session_id.as_deref(), Some(bound), RobotPhase::Listening, now_ms())
                                                .await;
                                        }
                                    }
                                    ListenState::Stop => {
                                        awaiting_next_utterance=false;
                                        if let Some(wav) = pipeline.finish() {
                                            utterance = Some(wav);
                                        } else if let Some(capture)=recording_capture.take() {
                                            capture.cancel(&mut cleanup_errors).await;
                                        }
                                    }
                                    // The wake word itself is not part of the turn.
                                    ListenState::Detect => {}
                                }
                            }
                            DeviceMessage::Abort { reason } => {
                                tracing::info!(%robot_id, ?reason, "robot: abort");
                                if matches!(reason.as_deref(),Some("wake_word_detected"|"barge_in"|"stop_playback")) {
                                    pacer.flush();
                                    if let Some(sid)=&session_id {writer.send_json(&ServerMessage::TtsStop {session_id:sid.clone()}).await;}
                                    continue;
                                }
                                // Fence and join ASR before clearing input and
                                // acknowledging stop, including queued audio.
                                transcriptions.cancel(&mut cleanup_errors).await;
                                if let Some(capture)=recording_capture.take() { capture.cancel(&mut cleanup_errors).await; }
                                // Order matters: stop our own queue first, because
                                // the device will play anything we hand over.
                                pacer.flush();
                                active_playback = None;
                                if let Some(task) = turn_task.take() {
                                    task.abort();
                                    if let Err(error) = task.join().await { cleanup_errors.push(error); }
                                }
                                if let Some(pipeline) = uplink.as_mut() {
                                    pipeline.abort();
                                }
                                utterance_queue.clear();
                                recording_question_id=None;
                                recording_work_target=None;
                                awaiting_next_utterance=false;
                                if let Some(request) = active_request.take()
                                    && let Err(error) = deps.dispatcher.cancel(&request).await
                                {
                                    tracing::warn!(%robot_id, %error, "robot: turn cancel failed");
                                    cleanup_errors.push(format!("Robot turn cancellation failed: {error}"));
                                }
                                if let Some(sid) = &session_id {
                                    writer.send_json(&ServerMessage::TtsStop { session_id: sid.clone() }).await;
                                }
                                if let Some(bound) = &companion_id {
                                    deps.status.publish_for_connection(&deps.registry, &robot_id, session_id.as_deref(), Some(bound), RobotPhase::Idle, now_ms()).await;
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
            if conversation_id.is_none() && let Some(bound) = companion_id.as_deref() {
                conversation_id = deps.dispatcher.ensure_companion_session(bound, &robot_id).await.ok();
            }
            if conversation_id.is_none() {
                if let Some(capture)=recording_capture.take() { capture.cancel(&mut cleanup_errors).await; }
                if let Some(sid) = session_id.as_deref() {
                    if deps.registry.connection_matches(&robot_id, sid).await {
                        show_voice_notice(&writer, sid, "请先在伙伴设置中配置对话模型").await;
                    }
                }
                continue;
            }
            let (Some(sid), Some(bound), Some(_conversation)) = (
                session_id.clone(),
                companion_id.clone(),
                conversation_id.clone(),
            ) else {
                continue;
            };
            let mut prepared=recording_capture.take();
            if let Some(capture)=prepared.as_mut() { capture.endpoint(); }
            if transcriptions.pending.len()>=MAX_PENDING_TRANSCRIPTIONS {
                if let Some(capture)=prepared.take() { capture.cancel(&mut cleanup_errors).await; }
            }
            if !transcriptions.enqueue(sid.clone(), bound, wav,recording_question_id.take(),recording_work_target.take(),prepared,recording_media_generation,recording_context.take()) {
                tracing::warn!(%robot_id, session_id = %sid, limit = MAX_PENDING_TRANSCRIPTIONS,
                    "robot: pending transcription queue is full");
                show_voice_notice(&writer, &sid, "等待识别的语音过多，请稍后再说").await;
            }
        }
    }

    // The turn task, the tool-discovery task, the pacer and the MCP client all
    // hold clones of the writer's sender, so every one of them has to be gone —
    // not merely asked to stop — before the writer task can see its channel
    // close and finish.
    transcriptions.cancel(&mut cleanup_errors).await;
    if let Some(capture)=recording_capture.take() { capture.cancel(&mut cleanup_errors).await; }
    if let Some(task) = turn_task.take() {
        task.abort();
        if let Err(error) = task.join().await { cleanup_errors.push(error); }
    }
    if *stopped.borrow() {
        if let Some(request) = active_request.take()
            && let Err(error) = deps.dispatcher.cancel(&request).await
        {
            cleanup_errors.push(format!("Robot active turn cancellation failed: {error}"));
        }
    }
    if let Some(task) = discovery_task.take() {
        task.abort();
        if let Err(error) = task.join().await { cleanup_errors.push(error); }
    }
    if let Some(sid) = session_id.as_deref() {
        deps.registry.disconnect(&robot_id, sid).await;
        deps.tools.detach_connection(&robot_id, sid).await;
    }
    drop(mcp);
    pacer.flush();
    pacer_task.abort();
    if let Err(error) = pacer_task.join().await { cleanup_errors.push(error); }
    // Includes old/replaced turns and discovery tasks, not only the latest.
    if let Err(error) = tasks.abort_and_join().await { cleanup_errors.push(error); }
    drop(pacer);
    deps.status.mark_offline_if_disconnected(&deps.registry, &robot_id, now_ms()).await;
    drop(writer);
    if let Err(error) = writer_task.join().await { cleanup_errors.push(error); }
    tracing::info!(%robot_id, "robot: session ended");
    if cleanup_errors.is_empty() { Ok(()) } else { Err(cleanup_errors.join("; ")) }
}

async fn transcribe_until_stopped(
    speech: &dyn SpeechServices,
    context: &SpeechContext,
    wav: Vec<u8>,
    stopped: &mut tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<Option<String>> {
    if *stopped.borrow() { return Ok(None); }
    tokio::select! {
        biased;
        result = async {
            loop {
                if *stopped.borrow_and_update() { return Ok::<(), anyhow::Error>(()); }
                stopped.changed().await.map_err(|_| anyhow::anyhow!("Robot ASR stop channel was lost"))?;
            }
        } => { result?; Ok(None) }
        result = speech.transcribe(context, wav) => result.map(Some),
    }
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

    struct GatedSpeechDiscovery {
        inner:Arc<dyn SpeechServices>,
        entered:tokio::sync::Notify,
        release:tokio::sync::Semaphore,
        batch_calls:std::sync::atomic::AtomicUsize,
        wav_samples:std::sync::atomic::AtomicUsize,
    }
    impl GatedSpeechDiscovery {
        fn new(inner:Arc<dyn SpeechServices>)->Self {Self {inner,entered:tokio::sync::Notify::new(),
            release:tokio::sync::Semaphore::new(0),batch_calls:Default::default(),wav_samples:Default::default()}}
    }
    #[async_trait::async_trait]
    impl SpeechServices for GatedSpeechDiscovery {
        async fn open_transcription(&self,context:&SpeechContext,sample_rate:u32)->anyhow::Result<Option<crate::services::TranscriptionSession>> {
            self.entered.notify_one();self.release.acquire().await.unwrap().forget();
            self.inner.open_transcription(context,sample_rate).await
        }
        async fn transcribe(&self,context:&SpeechContext,wav:Vec<u8>)->anyhow::Result<String> {
            self.batch_calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            self.wav_samples.store(wav.len().saturating_sub(44)/2,std::sync::atomic::Ordering::SeqCst);
            self.inner.transcribe(context,wav).await
        }
        async fn synthesize(&self,context:&SpeechContext,text:&str)->anyhow::Result<crate::audio::AudioBuffer> {self.inner.synthesize(context,text).await}
        async fn explain_image(&self,context:&SpeechContext,jpeg:Vec<u8>,question:&str)->anyhow::Result<String> {self.inner.explain_image(context,jpeg,question).await}
    }

    #[tokio::test]
    async fn batch_protocol_discovery_preserves_the_complete_wav_when_its_unused_pcm_tap_overflows() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let batch=Arc::new(MockSpeech::new());batch.push_transcript("完整批处理语音");
        let gated=Arc::new(GatedSpeechDiscovery::new(batch));
        deps.deps.speech=gated.clone();
        let dispatcher=deps.dispatcher_mock();dispatcher.script_turn(vec![TurnEvent::Done]);
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
        send_audio(&tx,1200,true).await; // Twenty packets cannot fit the eight-frame incremental tap.
        tx.send(Frame::Text(r#"{"type":"listen","state":"stop"}"#.into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),gated.entered.notified()).await.unwrap();
        gated.release.add_permits(1);
        await_frame(&written,|frame|frame["type"]=="stt"&&frame["text"]=="完整批处理语音").await;
        assert_eq!(gated.batch_calls.load(std::sync::atomic::Ordering::SeqCst),1);
        assert_eq!(gated.wav_samples.load(std::sync::atomic::Ordering::SeqCst),16_000*1200/1000,
            "unused streaming backpressure cannot truncate a batch upload");
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn incremental_protocol_discovery_fails_closed_on_capture_overflow_without_batch_fallback() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let (wire,_)=listener.accept().await.unwrap();
            let mut wire=tokio_tungstenite::accept_async(wire).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3),async {
                while let Some(Ok(message))=wire.next().await {
                    assert!(!matches!(&message,Message::Binary(_)),"a known incomplete capture must never admit partial PCM");
                    if matches!(message,Message::Close(_)) {break;}
                }
            }).await.expect("overflow must cancel and join the selected incremental provider");
        });
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let gated=Arc::new(GatedSpeechDiscovery::new(Arc::new(WireStreamingSpeech {
            endpoint,batch_calls:Arc::new(std::sync::atomic::AtomicUsize::new(0)),tts:MockSpeech::new(),
        })));
        deps.deps.speech=gated.clone();
        let dispatcher=deps.dispatcher_mock();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
        send_audio(&tx,1200,true).await;
        tx.send(Frame::Text(r#"{"type":"listen","state":"stop"}"#.into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),gated.entered.notified()).await.unwrap();
        gated.release.add_permits(1);
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="sentence_start").await;
        server.await.unwrap();
        assert_eq!(gated.batch_calls.load(std::sync::atomic::Ordering::SeqCst),0);
        assert!(dispatcher.dispatched_requests().is_empty());
        assert!(texts(&written).iter().all(|frame|frame["type"]!="stt"));
        drop(tx);task.await.unwrap();
    }

    struct WireStreamingSpeech {
        endpoint:String,
        batch_calls:Arc<std::sync::atomic::AtomicUsize>,
        tts:MockSpeech,
    }
    #[async_trait::async_trait]
    impl SpeechServices for WireStreamingSpeech {
        async fn open_transcription(&self,_:&SpeechContext,sample_rate:u32)->anyhow::Result<Option<crate::services::TranscriptionSession>> {
            use nomifun_model_invoke::{AuthMaterial,AuthScheme,ResolvedConnection,ResolvedTaskConfig,ResolvedTaskTransport};
            Ok(Some(nomifun_model_invoke::voice::speech::create_transcription_session(ResolvedTaskConfig {
                provider_id:"018f0000-0000-7000-8000-000000000001".into(),config_revision:1,
                platform:"deepgram".into(),model:"nova-3".into(),task:nomifun_api_types::ModelTask::SpeechRecognition,
                traits:vec![], input_constraints: None,protocol:nomifun_model_invoke::voice::speech::DEEPGRAM_INCREMENTAL_ASR_PROTOCOL.into(),
                connection:ResolvedConnection {role:"default".into(),base_url:self.endpoint.clone(),
                    auth:AuthMaterial {scheme:AuthScheme::TokenHeader,credentials:serde_json::json!({"api_keys":["fixture"]})},extra:serde_json::json!({})},
                transport:ResolvedTaskTransport {realtime_endpoint:Some("/v1/listen".into()),..Default::default()},
                provider_params:serde_json::json!({}),context_limit:None,output_limit:None,bedrock_config:None,
            },sample_rate,None).await?))
        }
        async fn transcribe(&self,_:&SpeechContext,_:Vec<u8>)->anyhow::Result<String> {
            self.batch_calls.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("streaming fixture must not invoke batch transcription")
        }
        async fn synthesize(&self,context:&SpeechContext,text:&str)->anyhow::Result<crate::audio::AudioBuffer> {self.tts.synthesize(context,text).await}
        async fn explain_image(&self,_:&SpeechContext,_:Vec<u8>,_:&str)->anyhow::Result<String> {anyhow::bail!("unused fixture")}
    }

    #[tokio::test]
    async fn streaming_asr_consumes_real_capture_before_endpoint_and_only_dispatches_final_text() {
        use futures_util::{SinkExt,StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        let (first_audio,audio_received)=oneshot::channel();
        let server=tokio::spawn(async move {
            let (wire,_)=listener.accept().await.unwrap();
            let mut wire=tokio_tungstenite::accept_async(wire).await.unwrap();
            let first=wire.next().await.unwrap().unwrap();
            assert!(matches!(first,Message::Binary(ref bytes) if bytes.len()==1920));
            wire.send(Message::Text(serde_json::json!({"type":"Results","start":0.0,"is_final":false,
                "channel":{"alternatives":[{"transcript":"可撤回的错误草稿"}]}}).to_string().into())).await.unwrap();
            first_audio.send(()).unwrap();
            loop {
                if let Message::Text(text)=wire.next().await.unwrap().unwrap() {
                    assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["type"],"CloseStream");
                    break;
                }
            }
            wire.send(Message::Text(serde_json::json!({"type":"Results","start":0.0,"is_final":true,
                "channel":{"alternatives":[{"transcript":"真实最终转写"}]}}).to_string().into())).await.unwrap();
            wire.send(Message::Text(serde_json::json!({"type":"Metadata"}).to_string().into())).await.unwrap();
            wire.close(None).await.unwrap();
        });
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let batch_calls=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        deps.deps.speech=Arc::new(WireStreamingSpeech {endpoint,batch_calls:batch_calls.clone(),tts:MockSpeech::new()});
        let dispatcher=deps.dispatcher_mock();
        dispatcher.script_turn(vec![TurnEvent::Done]);
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
        send_audio(&tx,120,true).await;
        tokio::time::timeout(Duration::from_secs(3),audio_received).await.unwrap().unwrap();
        assert!(dispatcher.dispatched_requests().is_empty(),"interim ASR is not canonical work input");
        assert!(texts(&written).iter().all(|frame|frame["type"]!="stt"));
        tx.send(Frame::Text(r#"{"type":"listen","state":"stop"}"#.into())).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="stt"&&frame["text"]=="真实最终转写").await;
        tokio::time::timeout(Duration::from_secs(3),async {
            while dispatcher.dispatched_requests().is_empty() {tokio::task::yield_now().await;}
        }).await.unwrap();
        assert_eq!(dispatcher.dispatched_requests()[0].text,"真实最终转写");
        assert_eq!(batch_calls.load(std::sync::atomic::Ordering::SeqCst),0);
        drop(tx);task.await.unwrap();server.await.unwrap();
    }

    #[tokio::test]
    async fn abort_during_streaming_capture_joins_provider_and_never_dispatches_a_transcript() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}",listener.local_addr().unwrap());
        let (first_audio,audio_received)=oneshot::channel();
        let server=tokio::spawn(async move {
            let (wire,_)=listener.accept().await.unwrap();
            let mut wire=tokio_tungstenite::accept_async(wire).await.unwrap();
            assert!(matches!(wire.next().await.unwrap().unwrap(),Message::Binary(_)));
            first_audio.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(3),async {
                while let Some(Ok(message))=wire.next().await {
                    if matches!(message,Message::Close(_)) {break;}
                }
            }).await.expect("aborted capture must release the exact upstream connection");
        });
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let batch_calls=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        deps.deps.speech=Arc::new(WireStreamingSpeech {endpoint,batch_calls:batch_calls.clone(),tts:MockSpeech::new()});
        let dispatcher=deps.dispatcher_mock();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
        send_audio(&tx,120,true).await;
        tokio::time::timeout(Duration::from_secs(3),audio_received).await.unwrap().unwrap();
        tx.send(Frame::Text(r#"{"type":"abort","reason":"cancel_work"}"#.into())).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        server.await.unwrap();
        assert!(dispatcher.dispatched_requests().is_empty());
        assert_eq!(batch_calls.load(std::sync::atomic::Ordering::SeqCst),0);
        drop(tx);task.await.unwrap();
        assert!(texts(&written).iter().all(|frame|frame["type"]!="stt"));
    }

    #[tokio::test]
    async fn stopped_generation_rejects_late_question_before_publishing_its_answer_target() {
        let written=Arc::new(Mutex::new(Vec::new()));
        let (writer,writer_task)=Writer::spawn(Box::new(RecordingSink(written.clone())));
        let (pacer,pacer_task)=DownlinkPacer::spawn(writer.tx.clone());
        let pacer=Arc::new(pacer);
        let generation=pacer.generation();
        let (events,receiver)=mpsc::channel(2);
        let speech=Arc::new(MockSpeech::new());
        let dispatcher=Arc::new(MockDispatcher::new());
        let presented=Arc::new(std::sync::Mutex::new(None));
        let request=RobotTurnRequest {robot_id:"robot".into(),companion_id:"companion".into(),conversation_id:"conversation".into(),
            connection_id:"connection".into(),request_id:"request".into(),text:"question".into(),presented_question_id:None,captured_context:None};
        let turn=tokio::spawn(drive_turn(receiver,SpeechContext {robot_id:"robot".into(),companion_id:"companion".into()},
            speech.clone(),pacer.clone(),writer.clone(),"connection".into(),generation,presented.clone(),Some((dispatcher.clone(),request)),None));
        pacer.flush();
        events.send(TurnEvent::Question {source:nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:"question-source".into(),revision:1,through_seq:1},
            text:"你几点出发？".into(),presentation_id:"late-question".into()}).await.unwrap();
        events.send(TurnEvent::Done).await.unwrap();drop(events);
        assert!(turn.await.unwrap().failed.is_none());
        assert!(presented.lock().unwrap().is_none());
        assert!(dispatcher.presented_questions().is_empty(),"stopped media cannot acknowledge a question presentation");
        assert!(speech.synthesized_text().is_empty());
        drop(pacer);pacer_task.abort();let _=pacer_task.await;
        drop(writer);writer_task.await.unwrap();
        assert!(texts(&written).is_empty(),"late questions cannot write into the successor media generation");
    }

    #[tokio::test]
    async fn queued_second_turn_waits_until_every_first_audio_frame_and_stop_reach_the_writer() {
        let (deps,link,tx,written,_dir)=harness(true).await;
        let first=format!("{}。","第一段".repeat(10));
        deps.speech.push_transcript("第一个任务");deps.speech.push_transcript("第二个任务");
        deps.dispatcher.script_turn(vec![TurnEvent::Text(first.clone()),TurnEvent::Done]);
        deps.dispatcher.script_turn(vec![TurnEvent::Text("第二段。".into()),TurnEvent::Done]);
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["text"]==first).await;
        manual_utterance(&tx).await;
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                if texts(&written).iter().filter(|frame|frame["type"]=="tts"&&frame["state"]=="stop").count()>=2 {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let frames=written.lock().unwrap().clone();
        let marker=|frame:&Frame,state:&str|match frame {Frame::Text(text)=>serde_json::from_str::<serde_json::Value>(text)
            .is_ok_and(|value|value["type"]=="tts"&&value["state"]==state),_=>false};
        let starts:Vec<_>=frames.iter().enumerate().filter(|(_,frame)|marker(frame,"start")).map(|(index,_)|index).collect();
        let stops:Vec<_>=frames.iter().enumerate().filter(|(_,frame)|marker(frame,"stop")).map(|(index,_)|index).collect();
        assert_eq!(starts.len(),2);assert_eq!(stops.len(),2);
        assert!(starts[0]<stops[0]&&stops[0]<starts[1]&&starts[1]<stops[1]);
        let expected_samples=24_000/1000*80*first.chars().count();
        let expected_frames=expected_samples.div_ceil(crate::audio::DOWNLINK_FRAME_SAMPLES);
        assert_eq!(frames[starts[0]..stops[0]].iter().filter(|frame|matches!(frame,Frame::Binary(_))).count(),expected_frames,
            "a successor must not flush the first turn's queued audio tail");
        assert!(frames[stops[0]..starts[1]].iter().all(|frame|!matches!(frame,Frame::Binary(_))));
        drop(tx);task.await.unwrap();
    }

    struct HeldTranscribe {
        entered: Arc<tokio::sync::Notify>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl SpeechServices for HeldTranscribe {
        async fn transcribe(&self, _: &SpeechContext, _: Vec<u8>) -> anyhow::Result<String> {
            struct Guard(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Guard { fn drop(&mut self) { self.0.store(true, std::sync::atomic::Ordering::Release); } }
            let _guard = Guard(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        }
        async fn synthesize(&self, _: &SpeechContext, _: &str) -> anyhow::Result<crate::audio::AudioBuffer> { anyhow::bail!("unused fixture") }
        async fn explain_image(&self, _: &SpeechContext, _: Vec<u8>, _: &str) -> anyhow::Result<String> { anyhow::bail!("unused fixture") }
    }

    #[tokio::test]
    async fn host_stop_cancels_held_asr_without_late_transcript() {
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let speech = HeldTranscribe { entered: entered.clone(), dropped: dropped.clone() };
        let task = tokio::spawn(async move {
            transcribe_until_stopped(&speech, &SpeechContext { robot_id: "fixture".into(), companion_id: "fixture".into() }, vec![], &mut stopped).await
        });
        entered.notified().await;
        stop.send_replace(true);
        assert_eq!(task.await.unwrap().unwrap(), None);
        assert!(dropped.load(std::sync::atomic::Ordering::Acquire));
    }

    struct ControlledAsrCall {
        reply: oneshot::Sender<anyhow::Result<String>>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }

    struct ControlledAsr {
        calls: mpsc::UnboundedSender<ControlledAsrCall>,
        tts: MockSpeech,
    }

    #[async_trait::async_trait]
    impl SpeechServices for ControlledAsr {
        async fn transcribe(&self, _: &SpeechContext, _: Vec<u8>) -> anyhow::Result<String> {
            struct Guard(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Guard {
                fn drop(&mut self) { self.0.store(true, std::sync::atomic::Ordering::Release); }
            }
            let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let _guard = Guard(dropped.clone());
            let (reply, result) = oneshot::channel();
            self.calls.send(ControlledAsrCall { reply, dropped })
                .map_err(|_| anyhow::anyhow!("test ASR controller closed"))?;
            result.await.map_err(|_| anyhow::anyhow!("test ASR response dropped"))?
        }
        async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<crate::audio::AudioBuffer> {
            self.tts.synthesize(ctx, text).await
        }
        async fn explain_image(&self, _: &SpeechContext, _: Vec<u8>, _: &str) -> anyhow::Result<String> {
            anyhow::bail!("unused fixture")
        }
    }

    fn controlled_asr(deps: &mut Harness) -> mpsc::UnboundedReceiver<ControlledAsrCall> {
        let (calls, rx) = mpsc::unbounded_channel();
        deps.deps.speech = Arc::new(ControlledAsr { calls, tts: MockSpeech::new() });
        deps.dispatcher.set_vad_engine("energy");
        rx
    }

    async fn next_asr(calls: &mut mpsc::UnboundedReceiver<ControlledAsrCall>) -> ControlledAsrCall {
        tokio::time::timeout(Duration::from_secs(3), calls.recv()).await
            .expect("ASR must start without waiting for another device frame")
            .expect("ASR controller must remain open")
    }

    struct BackgroundControlDispatcher {
        inner:MockDispatcher,
        current:Mutex<crate::services::RobotExecutionTarget>,
        cancelled:Mutex<Vec<RobotTurnRequest>>,
        supplements:Mutex<Vec<(RobotTurnRequest,String,bool,String)>>,
        foreground_active:std::sync::atomic::AtomicBool,
    }
    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for BackgroundControlDispatcher {
        async fn capture_input_context(&self,_:&str,_:&str,_:&str,_:&str,question:Option<String>)->anyhow::Result<Option<crate::services::RobotInputContext>> {
            Ok(Some(crate::services::RobotInputContext {
                binding_version:7,context_floor:11,presented_question_id:question,
                work_operation_id:self.foreground_active.load(std::sync::atomic::Ordering::SeqCst).then(||"other-window-turn".into()),execution_generation:None,
                background_work:Some(self.current.lock().unwrap().clone()),background_work_ambiguous:false,
            }))
        }
        // This fixture records the port input. Exact current-version CAS is
        // exercised by the application/execution owner, not simulated here.
        async fn cancel_background_work(&self,request:&RobotTurnRequest)->anyhow::Result<()> {
            self.cancelled.lock().unwrap().push(request.clone());Ok(())
        }
        async fn supplement_background_work(&self,request:&RobotTurnRequest,text:&str,correction:bool,input_id:&str)->anyhow::Result<()> {
            self.supplements.lock().unwrap().push((request.clone(),text.to_owned(),correction,input_id.to_owned()));Ok(())
        }
        async fn ensure_companion_session(&self,companion:&str,robot:&str)->anyhow::Result<String> {self.inner.ensure_companion_session(companion,robot).await}
        async fn dispatch(&self,request:RobotTurnRequest)->anyhow::Result<mpsc::Receiver<TurnEvent>> {self.inner.dispatch(request).await}
        async fn cancel(&self,request:&RobotTurnRequest)->anyhow::Result<()> {self.inner.cancel(request).await}
        async fn vad_tuning(&self,_:&str)->crate::vad::VadTuning {Default::default()}
        async fn vad_engine(&self,_:&str)->String {"energy".into()}
    }
    fn background_target(label:&str)->crate::services::RobotExecutionTarget {
        crate::services::RobotExecutionTarget {
            execution_id:format!("execution-{label}"),expected_version:5,source_turn_operation_id:format!("source-turn-{label}"),
            source_tool_operation_id:format!("source-tool-{label}"),source_effect_id:format!("source-effect-{label}"),attempt_ambiguous:false,
            attempt:Some(crate::services::RobotExecutionAttemptTarget {
                step_id:format!("step-{label}"),attempt_id:format!("attempt-{label}"),expected_step_version:6,expected_attempt_version:7,
                agent_session_id:format!("child-session-{label}"),turn_operation_id:format!("child-turn-{label}"),binding_version:8,execution_generation:9,
            }),
        }
    }
    fn background_dispatcher(target:crate::services::RobotExecutionTarget)->Arc<BackgroundControlDispatcher> {
        Arc::new(BackgroundControlDispatcher {inner:MockDispatcher::new(),current:Mutex::new(target),cancelled:Default::default(),supplements:Default::default(),foreground_active:Default::default()})
    }

    #[tokio::test]
    async fn delayed_asr_background_cancel_keeps_the_target_frozen_at_recording_onset() {
        let (mut deps,link,tx,_written,_dir)=harness(true).await;
        let mut calls=controlled_asr(&mut deps);
        let target_a=background_target("a");let target_b=background_target("b");
        let dispatcher=background_dispatcher(target_a.clone());deps.deps.dispatcher=dispatcher.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let recorded=next_asr(&mut calls).await;
        *dispatcher.current.lock().unwrap()=target_b.clone();
        recorded.reply.send(Ok("取消任务".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while dispatcher.cancelled.lock().unwrap().is_empty() {tokio::task::yield_now().await;}
        }).await.unwrap();
        let received=dispatcher.cancelled.lock().unwrap().clone();
        assert_eq!(received.len(),1);
        assert_eq!(received[0].captured_context.as_ref().unwrap().background_work.as_ref(),Some(&target_a),
            "delayed recognition cannot replace the recorded execution/attempt/version/source identity with the latest target");
        assert_eq!(*dispatcher.current.lock().unwrap(),target_b);
        assert!(dispatcher.inner.dispatched_requests().is_empty());
        assert!(dispatcher.inner.cancelled().is_empty(),"background control is not a foreground Agent turn cancellation");
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn identical_background_supplements_keep_distinct_recorded_input_ids_and_attempt_targets() {
        let (mut deps,link,tx,_written,_dir)=harness(true).await;
        let mut calls=controlled_asr(&mut deps);
        let target_a=background_target("a");let target_b=background_target("b");
        let dispatcher=background_dispatcher(target_a.clone());deps.deps.dispatcher=dispatcher.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        for count in 1..=2 {
            *dispatcher.current.lock().unwrap()=target_a.clone();
            manual_utterance(&tx).await;
            let recorded=next_asr(&mut calls).await;
            *dispatcher.current.lock().unwrap()=target_b.clone();
            recorded.reply.send(Ok("补充：保留完整图表".into())).unwrap();
            tokio::time::timeout(Duration::from_secs(3),async {
                while dispatcher.supplements.lock().unwrap().len()<count {tokio::task::yield_now().await;}
            }).await.unwrap();
        }
        let received=dispatcher.supplements.lock().unwrap().clone();
        assert_eq!(received.len(),2);
        for (request,text,correction,input_id) in &received {
            assert_eq!(request.captured_context.as_ref().unwrap().background_work.as_ref(),Some(&target_a));
            assert_eq!(text,"补充：保留完整图表");assert!(!correction);
            assert_eq!(input_id,&request.request_id,"the ASR input operation identity must survive into the control port");
        }
        assert_ne!(received[0].3,received[1].3,"two utterances with identical text are distinct user inputs");
        assert!(dispatcher.inner.dispatched_requests().is_empty());
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn foreground_and_background_scope_requires_an_explicit_control_target() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let mut calls=controlled_asr(&mut deps);
        let dispatcher=background_dispatcher(background_target("original"));
        dispatcher.foreground_active.store(true,std::sync::atomic::Ordering::SeqCst);
        deps.deps.dispatcher=dispatcher.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        for text in ["取消任务","补充：保留图表"] {
            manual_utterance(&tx).await;next_asr(&mut calls).await.reply.send(Ok(text.into())).unwrap();
            await_frame(&written,|frame|frame["type"]=="tts"&&frame["text"].as_str().is_some_and(|text|text.contains("当前入口和后台"))).await;
        }
        assert!(dispatcher.cancelled.lock().unwrap().is_empty());assert!(dispatcher.supplements.lock().unwrap().is_empty());
        for (text,cancel) in [("取消后台任务",true),("补充后台任务：保留图表",false)] {
            manual_utterance(&tx).await;next_asr(&mut calls).await.reply.send(Ok(text.into())).unwrap();
            tokio::time::timeout(Duration::from_secs(3),async {
                while if cancel {dispatcher.cancelled.lock().unwrap().is_empty()}else{dispatcher.supplements.lock().unwrap().is_empty()} {tokio::task::yield_now().await;}
            }).await.unwrap();
        }
        assert_eq!(dispatcher.cancelled.lock().unwrap().len(),1);assert_eq!(dispatcher.supplements.lock().unwrap().len(),1);
        assert!(dispatcher.inner.dispatched_requests().is_empty());assert!(dispatcher.inner.cancelled().is_empty());
        drop(tx);task.await.unwrap();
    }

    async fn manual_utterance(tx: &mpsc::Sender<Frame>) {
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
        send_audio(tx, 120, true).await;
        tx.send(Frame::Text(r#"{"type":"listen","state":"stop"}"#.into())).await.unwrap();
    }

    async fn reply_mcp(tx: &mpsc::Sender<Frame>, request: &serde_json::Value, result: serde_json::Value) {
        tx.send(Frame::Text(serde_json::json!({
            "type":"mcp", "payload":{"jsonrpc":"2.0", "id":request["payload"]["id"], "result":result}
        }).to_string())).await.unwrap();
    }

    #[tokio::test]
    async fn held_asr_keeps_mcp_responses_and_keepalive_flowing() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let tools = deps.deps.tools.clone();
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#.into())).await.unwrap();
        let initialize = await_frame(&written, |f| f["payload"]["method"] == "initialize").await;
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        reply_mcp(&tx, &initialize, serde_json::json!({})).await;
        let list = await_frame(&written, |f| f["payload"]["method"] == "tools/list").await;
        reply_mcp(&tx, &list, serde_json::json!({"tools":[{
            "name":"self.get_device_status", "description":"status",
            "inputSchema":{"type":"object","properties":{}}
        }]})).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while !tools.is_attached("aa:bb:cc:dd:ee:ff").await { tokio::task::yield_now().await; }
        }).await.expect("discovery replies must be read while ASR is held");
        let call = tokio::spawn(async move {
            tools.call_for_action("aa:bb:cc:dd:ee:ff", crate::capability::RobotAction::Device,
                "robot_get_device_status", serde_json::json!({})).await
        });
        let request = await_frame(&written, |f| f["payload"]["method"] == "tools/call").await;
        reply_mcp(&tx, &request, serde_json::json!({"content":[{"type":"text","text":"ready"}]})).await;
        assert_eq!(tokio::time::timeout(Duration::from_secs(3), call).await.unwrap().unwrap().unwrap(), "ready");
        assert!(!held.dropped.load(std::sync::atomic::Ordering::Acquire), "ASR is still waiting independently");
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(PING_INTERVAL_SECS + 1)).await;
        await_frame(&written, |f| f["type"] == "ping").await;
        drop(tx);
        task.await.unwrap();
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(held.reply.send(Ok("late".into())).is_err());
    }

    #[tokio::test]
    async fn delayed_asr_keeps_the_question_presented_when_recording_started() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let mut calls=controlled_asr(&mut deps);
        let registry=deps.deps.registry.clone();
        let robot="aa:bb:cc:dd:ee:ff";
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=tokio::time::timeout(Duration::from_secs(3),async {
            loop {if let Some(connection)=registry.current_connection(robot).await{break connection;}
                tokio::task::yield_now().await;}
        }).await.unwrap();
        let q1=nomifun_voice_contracts::VoiceSpeechSourceRef{message_id:"question-1".into(),revision:11,through_seq:11};
        let q2=nomifun_voice_contracts::VoiceSpeechSourceRef{message_id:"question-2".into(),revision:21,through_seq:21};

        // This WAV predates every question. Presenting Q1 while the provider
        // is still held must not bind the old answer at eventual admission.
        manual_utterance(&tx).await;
        let old=next_asr(&mut calls).await;
        registry.speak_requested_question_source(robot,&connection,"请问你的名字？".into(),q1.clone(),"presentation-1".into()).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        old.reply.send(Ok("旧的话语".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while deps.dispatcher.dispatched_requests().is_empty(){tokio::task::yield_now().await;}
        }).await.unwrap();
        assert_eq!(deps.dispatcher.dispatched_requests()[0].presented_question_id,None);

        // A Q1 utterance is held across Q2 presentation. Its frozen cut must
        // remain Q1, so the Host's exact-current-question check rejects it.
        manual_utterance(&tx).await;
        let first=next_asr(&mut calls).await;
        tokio::time::timeout(Duration::from_secs(3),async {
            loop{match registry.speak_requested_question_source(robot,&connection,"请问几点出发？".into(),q2.clone(),"presentation-2".into()).await {
                Ok(())=>break,Err(RobotPlaybackError::Busy)=>tokio::task::yield_now().await,Err(error)=>panic!("question playback: {error}")
            }}
        }).await.unwrap();
        // Re-acknowledging an older exact source never changes the cut back.
        registry.speak_requested_question_source(robot,&connection,"请问你的名字？".into(),q1.clone(),"presentation-1".into()).await.unwrap();
        assert!(matches!(registry.speak_requested_question_source(robot,&connection,"请问你的名字？".into(),q1,"presentation-changed".into()).await,Err(RobotPlaybackError::SourceChanged)));
        first.reply.send(Ok("小明".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while deps.dispatcher.dispatched_requests().len()<2{tokio::task::yield_now().await;}
        }).await.unwrap();
        assert_eq!(deps.dispatcher.dispatched_requests()[1].presented_question_id.as_deref(),Some("presentation-1"));
        manual_utterance(&tx).await;
        next_asr(&mut calls).await.reply.send(Ok("九点".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while deps.dispatcher.dispatched_requests().len()<3{tokio::task::yield_now().await;}
        }).await.unwrap();
        let requests=deps.dispatcher.dispatched_requests();
        assert_eq!(requests[2].presented_question_id.as_deref(),Some("presentation-2"));
        assert!(requests.iter().all(|request|request.connection_id==connection));
        drop(tx);task.await.unwrap();
    }

    async fn continuous_realtime_question_cut_tracks_speech_onset(question_before_speech: bool) {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let registry = deps.deps.registry.clone();
        let status = deps.deps.status.clone();
        let dispatcher = deps.dispatcher_mock();
        let robot = "aa:bb:cc:dd:ee:ff";
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Done]);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#.into()))
            .await.unwrap();
        let initialize = await_frame(&written, |frame| frame["payload"]["method"] == "initialize").await;
        let connection = registry.current_connection(robot).await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"realtime"}"#.into()))
            .await.unwrap();

        // Establish the continuation path after a silent action. No subsequent
        // listen.start may establish or overwrite the next question cut.
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        next_asr(&mut calls).await.reply.send(Ok("坐下".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if dispatcher.dispatched_requests().len() == 1
                    && status.snapshot().await.first().is_some_and(|snapshot| snapshot.phase == "idle")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("initial silent action must settle before presenting a question");

        send_audio(&tx, 1200, false).await;
        if !question_before_speech {
            // This user started speaking before Q1 existed. Presenting Q1
            // halfway through the utterance must never claim that old speech.
            send_tone(&tx, 300).await;
        }
        // The actor emits tools/list only after consuming this response, so
        // every earlier silence/speech packet has passed the onset boundary.
        reply_mcp(&tx, &initialize, serde_json::json!({})).await;
        await_frame(&written, |frame| frame["payload"]["method"] == "tools/list").await;
        let q1 = nomifun_voice_contracts::VoiceSpeechSourceRef {
            message_id: "continuous-question-1".into(), revision: 11, through_seq: 11,
        };
        let q2 = nomifun_voice_contracts::VoiceSpeechSourceRef {
            message_id: "continuous-question-2".into(), revision: 21, through_seq: 21,
        };
        registry.speak_requested_question_source(robot, &connection, "你的名字？".into(), q1,
            "continuous-presentation-1".into()).await.unwrap();
        await_frame(&written, |frame| frame["type"] == "tts" && frame["state"] == "stop").await;

        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        let held = next_asr(&mut calls).await;
        registry.speak_requested_question_source(robot, &connection, "几点？".into(), q2,
            "continuous-presentation-2".into()).await.unwrap();
        // A later presented question cannot rebind an already recorded WAV,
        // even while the provider keeps its transcription result in flight.
        held.reply.send(Ok("小明".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while dispatcher.dispatched_requests().len() < 2 {
                tokio::task::yield_now().await;
            }
        }).await.expect("held transcription must still become a request");
        let requests = dispatcher.dispatched_requests();
        assert_eq!(requests[0].presented_question_id, None);
        assert_eq!(requests[1].text, "小明");
        assert_eq!(requests[1].presented_question_id.as_deref(),
            question_before_speech.then_some("continuous-presentation-1"),
            "only speech beginning after Q1 presentation may carry Q1; Q2 cannot rewrite the frozen cut");
        assert!(requests.iter().all(|request| request.connection_id == connection));
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn continuous_realtime_question_after_leading_silence_is_frozen_at_speech_onset() {
        continuous_realtime_question_cut_tracks_speech_onset(true).await;
    }

    #[tokio::test]
    async fn continuous_realtime_speech_started_before_question_is_never_rebound() {
        continuous_realtime_question_cut_tracks_speech_onset(false).await;
    }

    struct FloodingStream {
        inner: Box<dyn RobotLinkStream>,
        enabled: Arc<std::sync::atomic::AtomicBool>,
        remaining: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl RobotLinkStream for FloodingStream {
        async fn next(&mut self) -> Option<Result<Frame, LinkError>> {
            if self.enabled.load(std::sync::atomic::Ordering::Acquire)
                && self.remaining.fetch_update(std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire, |left| left.checked_sub(1)).is_ok() {
                // The previous listen window is closed. These ready packets
                // must not starve the result that can open the next window.
                Some(Ok(Frame::Binary(bytes::Bytes::new())))
            } else {
                self.inner.next().await
            }
        }
    }

    #[tokio::test]
    async fn ready_asr_result_progresses_under_continuously_ready_device_audio() {
        let (mut deps, mut link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let flooding = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let remaining = Arc::new(std::sync::atomic::AtomicUsize::new(100_000));
        link.stream = Box::new(FloodingStream {
            inner: link.stream, enabled: flooding.clone(), remaining: remaining.clone(),
        });
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        flooding.store(true, std::sync::atomic::Ordering::Release);
        held.reply.send(Ok("not starved".into())).unwrap();
        await_frame(&written, |f| f["type"] == "stt" && f["text"] == "not starved").await;
        assert!(remaining.load(std::sync::atomic::Ordering::Acquire) > 0,
            "ASR must complete before a continuously ready device backlog drains");
        flooding.store(false, std::sync::atomic::Ordering::Release);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn device_abort_joins_asr_and_discards_queued_and_late_transcripts() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        manual_utterance(&tx).await;
        tx.send(Frame::Text(r#"{"type":"abort","reason":"cancel_work"}"#.into())).await.unwrap();
        await_frame(&written, |f| f["type"] == "tts" && f["state"] == "stop").await;
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire), "abort acknowledgement follows ASR join");
        assert!(held.reply.send(Ok("obsolete".into())).is_err());
        assert!(calls.try_recv().is_err(), "the queued utterance was cancelled too");
        manual_utterance(&tx).await;
        next_asr(&mut calls).await.reply.send(Ok("new input".into())).unwrap();
        await_frame(&written, |f| f["type"] == "stt" && f["text"] == "new input").await;
        drop(tx);
        task.await.unwrap();
        let transcripts: Vec<_> = texts(&written).into_iter().filter(|f| f["type"] == "stt")
            .map(|f| f["text"].as_str().unwrap().to_owned()).collect();
        assert_eq!(transcripts, ["new input"]);
    }

    #[tokio::test]
    async fn wake_word_barge_in_stops_only_media_and_preserves_active_work_and_asr() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let mut calls=controlled_asr(&mut deps);
        let dispatcher=deps.dispatcher_mock();
        let answer=format!("{}。","工作仍在继续".repeat(10));
        dispatcher.script_turn(vec![TurnEvent::Text(answer.clone()),TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Done]);
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        next_asr(&mut calls).await.reply.send(Ok("原工作".into())).unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["text"]==answer).await;
        manual_utterance(&tx).await;
        let captured=next_asr(&mut calls).await;
        let prior_stops=texts(&written).iter().filter(|frame|frame["type"]=="tts"&&frame["state"]=="stop").count();
        tx.send(Frame::Text(r#"{"type":"abort","reason":"wake_word_detected"}"#.into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while texts(&written).iter().filter(|frame|frame["type"]=="tts"&&frame["state"]=="stop").count()==prior_stops {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert!(dispatcher.cancelled().is_empty(),"wake-word barge-in has no work cancellation authority");
        assert!(!captured.dropped.load(std::sync::atomic::Ordering::Acquire),"stopping output must not cancel the current input recognition");
        captured.reply.send(Ok("继续交谈".into())).unwrap();
        await_frame(&written,|frame|frame["type"]=="stt"&&frame["text"]=="继续交谈").await;
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn session_shutdown_joins_held_asr_before_returning() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let owned = Arc::new(OwnedTasks::default());
        let task = tokio::spawn(run_session_owned(link, deps.deps.clone(), stopped, owned.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap().unwrap();
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(held.reply.send(Ok("late".into())).is_err());
        assert!(texts(&written).iter().all(|f| f["type"] != "stt"));
        owned.abort_and_join().await.unwrap();
        assert_eq!(owned.task_count(), 0);
    }

    #[tokio::test]
    async fn host_stop_wins_over_a_ready_asr_response_without_late_projection() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let owned = Arc::new(OwnedTasks::default());
        let task = tokio::spawn(run_session_owned(link, deps.deps.clone(), stopped, owned.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        held.reply.send(Ok("cancelled response".into())).unwrap();
        // Neither operation yields, so stop and the provider response become
        // ready together before the actor can publish either result.
        stop.send_replace(true);
        tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap().unwrap();
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(texts(&written).iter().all(|f| f["type"] != "stt"));
        owned.abort_and_join().await.unwrap();
    }

    #[tokio::test]
    async fn repeated_hello_discards_old_asr_and_pending_audio() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        let hello = r#"{"type":"hello","version":1,"transport":"websocket"}"#;
        tx.send(Frame::Text(hello.into())).await.unwrap();
        let first = await_frame(&written, |f| f["type"] == "hello").await;
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        manual_utterance(&tx).await;
        tx.send(Frame::Text(hello.into())).await.unwrap();
        let replacement = await_frame(&written, |f| f["type"] == "hello" && f["session_id"] != first["session_id"]).await;
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(held.reply.send(Ok("old hello".into())).is_err());
        assert!(calls.try_recv().is_err());
        manual_utterance(&tx).await;
        next_asr(&mut calls).await.reply.send(Ok("new hello".into())).unwrap();
        let stt = await_frame(&written, |f| f["type"] == "stt").await;
        assert_eq!(stt["text"], "new hello");
        assert_eq!(stt["session_id"], replacement["session_id"]);
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn replacing_connection_cancels_asr_without_publishing_old_input() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let registry = deps.deps.registry.clone();
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let held = next_asr(&mut calls).await;
        registry.connect("aa:bb:cc:dd:ee:ff", "0190f5fe-7c00-7a00-8000-0000000000aa", "replacement").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
        assert!(held.dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(held.reply.send(Ok("old connection".into())).is_err());
        assert!(texts(&written).iter().all(|f| f["type"] != "stt"));
        assert_eq!(registry.current_connection("aa:bb:cc:dd:ee:ff").await.as_deref(), Some("replacement"));
    }

    #[tokio::test]
    async fn asr_is_serial_bounded_and_preserves_utterance_order() {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        let first = next_asr(&mut calls).await;
        for _ in 0..MAX_PENDING_TRANSCRIPTIONS + 1 { manual_utterance(&tx).await; }
        await_frame(&written, |f| f["type"] == "tts" && f["text"] == "等待识别的语音过多，请稍后再说").await;
        assert!(calls.try_recv().is_err(), "there is only one in-flight ASR");
        first.reply.send(Ok("first".into())).unwrap();
        next_asr(&mut calls).await.reply.send(Ok("second".into())).unwrap();
        next_asr(&mut calls).await.reply.send(Ok("third".into())).unwrap();
        await_frame(&written, |f| f["type"] == "stt" && f["text"] == "third").await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while deps.dispatcher.dispatched_text().len() < 3 { tokio::task::yield_now().await; }
        }).await.expect("recognized utterances must reach the Agent in the same order");
        assert_eq!(deps.dispatcher.dispatched_text(), ["first", "second", "third"]);
        drop(tx);
        task.await.unwrap();
        let transcripts: Vec<_> = texts(&written).into_iter().filter(|f| f["type"] == "stt")
            .map(|f| f["text"].as_str().unwrap().to_owned()).collect();
        assert_eq!(transcripts, ["first", "second", "third"]);
        assert!(calls.try_recv().is_err(), "overflow never started another provider request");
    }

    struct HeldDispatch {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Semaphore>,
        accepted: Mutex<Vec<RobotTurnRequest>>,
        cancelled: Mutex<Vec<RobotTurnRequest>>,
    }
    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for HeldDispatch {
        async fn ensure_companion_session(&self, _: &str, _: &str) -> anyhow::Result<String> { Ok("conversation".into()) }
        async fn dispatch(&self, request: RobotTurnRequest) -> anyhow::Result<mpsc::Receiver<TurnEvent>> {
            self.accepted.lock().unwrap().push(request);
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
            Ok(mpsc::channel(1).1)
        }
        async fn cancel(&self, request: &RobotTurnRequest) -> anyhow::Result<()> { self.cancelled.lock().unwrap().push(request.clone()); Ok(()) }
        async fn vad_tuning(&self, _: &str) -> crate::vad::VadTuning { Default::default() }
        async fn vad_engine(&self, _: &str) -> String { crate::vad::DEFAULT_VAD_ENGINE.into() }
    }

    #[tokio::test]
    async fn host_stop_joins_dispatch_result_then_cancels_exact_request_without_projection() {
        let (mut harness, _link, _tx, written, _directory) = harness(true).await;
        let dispatcher = Arc::new(HeldDispatch {
            entered: Arc::new(tokio::sync::Notify::new()), release: Arc::new(tokio::sync::Semaphore::new(0)),
            accepted: Default::default(), cancelled: Default::default(),
        });
        harness.deps.dispatcher = dispatcher.clone();
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let owned = Arc::new(OwnedTasks::default());
        let cleanup = owned.clone();
        let task = tokio::spawn(async move {
            let tasks = TaskScope::new(owned.clone());
            let (writer, writer_task) = Writer::spawn(Box::new(RecordingSink(written)));
            owned.track(writer_task);
            let (pacer, pacer_task) = DownlinkPacer::spawn(writer.tx.clone());
            tasks.track(pacer_task);
            let (turn_tx, _turn_rx) = mpsc::channel(1);
            let mut active = None; let mut turn = None; let mut errors = Vec::new();
            start_turn(&harness.deps, &Arc::new(pacer), &writer, &turn_tx, &mut turn, &mut active,
                "robot", "companion", "conversation", "connection", "utterance", None,None,Arc::new(std::sync::Mutex::new(None)), &tasks, &stopped, &mut errors).await;
            assert!(turn.is_none() && active.is_none());
            assert!(errors.is_empty());
        });
        dispatcher.entered.notified().await;
        stop.send_replace(true);
        assert!(!task.is_finished(), "submitted dispatch is still owned until it returns");
        dispatcher.release.add_permits(1);
        task.await.unwrap();
        let accepted = dispatcher.accepted.lock().unwrap();
        let cancelled = dispatcher.cancelled.lock().unwrap();
        assert_eq!(accepted.len(), 1); assert_eq!(cancelled.len(), 1);
        assert_eq!(accepted[0].request_id, cancelled[0].request_id);
        assert_eq!(accepted[0].connection_id, cancelled[0].connection_id);
        drop(accepted); drop(cancelled);
        cleanup.abort_and_join().await.unwrap();
    }
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
                vision_base: None,
                device_token: "tok".to_owned(),
            },
            sink: Box::new(RecordingSink(written.clone())),
            stream: Box::new(ChannelStream(rx)),
        };
        let deps = SessionDeps {
            registry,
            status,
            speech: speech.clone(),
            dispatcher: dispatcher.clone(),
            tools: Arc::new(crate::tool_registry::RobotToolRegistry::default()),
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

    struct PendingQuestionDispatcher {
        inner:MockDispatcher,
        authority:crate::services::RobotQuestionPlaybackAuthority,
        pending:std::sync::atomic::AtomicBool,
        observed:Mutex<Vec<Option<crate::services::RobotQuestionPlaybackAuthority>>>,
        captured_questions:Mutex<Vec<Option<String>>>,
        entered:tokio::sync::Notify,
        before_writer:Option<Arc<tokio::sync::Semaphore>>,
        denied:tokio::sync::Notify,
        presented:std::sync::atomic::AtomicUsize,
    }
    impl PendingQuestionDispatcher {
        fn new(before_writer:Option<Arc<tokio::sync::Semaphore>>)->Arc<Self> {
            Arc::new(Self {inner:MockDispatcher::new(),authority:crate::services::RobotQuestionPlaybackAuthority {
                source:nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:"exact-execution-question-notice".into(),revision:41,through_seq:41},
                presentation_id:"execution:original-task:decision:17".into(),
            },pending:std::sync::atomic::AtomicBool::new(true),observed:Default::default(),captured_questions:Default::default(),
                entered:tokio::sync::Notify::new(),before_writer,denied:tokio::sync::Notify::new(),presented:Default::default()})
        }
        fn assert_exact_authority(&self) {
            let observed=self.observed.lock().unwrap();
            assert!(!observed.is_empty(),"the actual actor/writer must consult the pending-question owner");
            for authority in observed.iter() {
                assert_eq!(authority.as_ref(),Some(&self.authority),"neither the canonical notice revision nor its presentation may be replaced or omitted");
            }
        }
    }
    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for PendingQuestionDispatcher {
        async fn capture_input_context(&self,_:&str,_:&str,_:&str,_:&str,question:Option<String>)->anyhow::Result<Option<crate::services::RobotInputContext>> {
            self.captured_questions.lock().unwrap().push(question);Ok(None)
        }
        // The root Session binding/floor deliberately remains valid when this
        // mock pending decision is cancelled. App tests own real Execution CAS.
        async fn input_context_matches(&self,_:&RobotTurnRequest)->anyhow::Result<bool> {Ok(true)}
        async fn acquire_output_context(&self,_:&RobotTurnRequest,proactive_revision:Option<u64>,question:Option<&crate::services::RobotQuestionPlaybackAuthority>)
            ->anyhow::Result<Option<crate::services::RobotOutputLease>> {
            self.observed.lock().unwrap().push(question.cloned());self.entered.notify_one();
            if let Some(gate)=&self.before_writer {gate.acquire().await.unwrap().forget();}
            if proactive_revision.is_some()||question!=Some(&self.authority) {anyhow::bail!("the exact requested-question authority was replaced");}
            if !self.pending.load(std::sync::atomic::Ordering::SeqCst) {self.denied.notify_one();return Ok(None);}
            Ok(Some(crate::services::RobotOutputLease::new(())))
        }
        async fn question_presented(&self,_:&RobotTurnRequest,source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:&str)->anyhow::Result<()> {
            if source!=&self.authority.source||presentation_id!=self.authority.presentation_id
                ||!self.pending.load(std::sync::atomic::Ordering::SeqCst) {anyhow::bail!("question presentation no longer has its original pending authority");}
            self.presented.fetch_add(1,std::sync::atomic::Ordering::SeqCst);Ok(())
        }
        async fn ensure_companion_session(&self,companion:&str,robot:&str)->anyhow::Result<String> {self.inner.ensure_companion_session(companion,robot).await}
        async fn dispatch(&self,request:RobotTurnRequest)->anyhow::Result<mpsc::Receiver<TurnEvent>> {self.inner.dispatch(request).await}
        async fn cancel(&self,request:&RobotTurnRequest)->anyhow::Result<()> {self.inner.cancel(request).await}
        async fn vad_tuning(&self,_:&str)->crate::vad::VadTuning {Default::default()}
        async fn vad_engine(&self,_:&str)->String {"energy".into()}
    }
    fn pending_question_context()->crate::registry::RobotPlaybackContext {
        crate::registry::RobotPlaybackContext {session_id:"original-question-owner-session".into(),input_context:crate::services::RobotInputContext {
            binding_version:1,context_floor:0,presented_question_id:None,work_operation_id:None,execution_generation:None,
            background_work:None,background_work_ambiguous:false,
        }}
    }
    async fn pending_question_connection(registry:&RobotRegistry)->String {
        tokio::time::timeout(Duration::from_secs(3),async {
            loop {if let Some(connection)=registry.current_connection("aa:bb:cc:dd:ee:ff").await {break connection;}
                tokio::task::yield_now().await;}
        }).await.unwrap()
    }

    #[tokio::test]
    async fn requested_background_question_retains_exact_notice_and_presentation_at_each_writer_acceptance() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let dispatcher=PendingQuestionDispatcher::new(None);deps.deps.dispatcher=dispatcher.clone();
        let registry=deps.deps.registry.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=pending_question_connection(&registry).await;
        registry.speak_requested_context_source("aa:bb:cc:dd:ee:ff",&connection,"请问使用哪个名字？".into(),
            dispatcher.authority.source.clone(),Some(dispatcher.authority.presentation_id.clone()),pending_question_context()).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        dispatcher.assert_exact_authority();
        assert!(dispatcher.observed.lock().unwrap().len()>=3,"text and actual audio both require the same exact question authority");
        assert!(written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))));
        assert_eq!(dispatcher.presented.load(std::sync::atomic::Ordering::SeqCst),1);
        assert!(dispatcher.inner.dispatched_requests().is_empty(),"question delivery is not a new Agent Turn");
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_queued_question_before_writer_acceptance_publishes_no_sentence_audio_or_local_target() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let gate=Arc::new(tokio::sync::Semaphore::new(0));
        let dispatcher=PendingQuestionDispatcher::new(Some(gate.clone()));deps.deps.dispatcher=dispatcher.clone();
        let registry=deps.deps.registry.clone();
        deps.speech.push_transcript("后续普通输入");
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=pending_question_connection(&registry).await;
        registry.speak_requested_context_source("aa:bb:cc:dd:ee:ff",&connection,"这个过期问题不能播出？".into(),
            dispatcher.authority.source.clone(),Some(dispatcher.authority.presentation_id.clone()),pending_question_context()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),dispatcher.entered.notified()).await.unwrap();
        dispatcher.pending.store(false,std::sync::atomic::Ordering::SeqCst);gate.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3),dispatcher.denied.notified()).await.unwrap();
        // A later recording observes the actor's local question cut directly;
        // an obsolete queued question must never become its answer target.
        manual_utterance(&tx).await;
        tokio::time::timeout(Duration::from_secs(3),async {
            while dispatcher.inner.dispatched_requests().is_empty() {tokio::task::yield_now().await;}
        }).await.unwrap();
        dispatcher.assert_exact_authority();
        assert_eq!(*dispatcher.captured_questions.lock().unwrap(),vec![None]);
        assert!(dispatcher.inner.dispatched_requests()[0].presented_question_id.is_none());
        assert!(deps.speech.synthesized_text().is_empty());
        assert_eq!(dispatcher.presented.load(std::sync::atomic::Ordering::SeqCst),0);
        assert!(!texts(&written).iter().any(|frame|frame["type"]=="tts"&&frame["state"]=="sentence_start"));
        assert!(!written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))));
        drop(tx);task.await.unwrap();
    }

    struct HeldFirstQuestionChunk {
        entered:tokio::sync::Notify,release:tokio::sync::Semaphore,produced:std::sync::atomic::AtomicBool,
    }
    struct HeldQuestionSpeech {inner:MockSpeech,first:Arc<HeldFirstQuestionChunk>}
    #[async_trait::async_trait]
    impl SpeechServices for HeldQuestionSpeech {
        async fn transcribe(&self,context:&SpeechContext,wav:Vec<u8>)->anyhow::Result<String> {self.inner.transcribe(context,wav).await}
        async fn synthesize(&self,context:&SpeechContext,text:&str)->anyhow::Result<crate::audio::AudioBuffer> {self.inner.synthesize(context,text).await}
        async fn synthesize_stream(&self,_:&SpeechContext,_:&str)->anyhow::Result<crate::services::SpeechAudioStream> {
            let first=self.first.clone();
            Ok(crate::services::SpeechAudioStream {mode:crate::services::SpeechDeliveryMode::Streaming,
                audio:Box::pin(futures_util::stream::once(async move {
                    first.entered.notify_one();first.release.acquire().await.unwrap().forget();
                    first.produced.store(true,std::sync::atomic::Ordering::SeqCst);
                    Ok(crate::audio::AudioBuffer {pcm:vec![0;crate::audio::DOWNLINK_FRAME_SAMPLES*3],sample_rate:24_000})
                }))})
        }
        async fn explain_image(&self,_:&SpeechContext,_:Vec<u8>,_:&str)->anyhow::Result<String> {anyhow::bail!("unused fixture")}
    }

    #[tokio::test]
    async fn cancelling_a_question_while_first_tts_chunk_is_held_blocks_every_later_audio_frame() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let dispatcher=PendingQuestionDispatcher::new(None);deps.deps.dispatcher=dispatcher.clone();
        let first=Arc::new(HeldFirstQuestionChunk {entered:tokio::sync::Notify::new(),release:tokio::sync::Semaphore::new(0),produced:Default::default()});
        deps.deps.speech=Arc::new(HeldQuestionSpeech {inner:MockSpeech::new(),first:first.clone()});
        let registry=deps.deps.registry.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=pending_question_connection(&registry).await;
        registry.speak_requested_context_source("aa:bb:cc:dd:ee:ff",&connection,"哪一天出发？".into(),
            dispatcher.authority.source.clone(),Some(dispatcher.authority.presentation_id.clone()),pending_question_context()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),first.entered.notified()).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["text"]=="哪一天出发？").await;
        assert!(!written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))));
        dispatcher.pending.store(false,std::sync::atomic::Ordering::SeqCst);first.release.add_permits(1);
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        assert!(first.produced.load(std::sync::atomic::Ordering::SeqCst),"the late provider chunk must actually reach the paced authority check");
        dispatcher.assert_exact_authority();
        assert!(dispatcher.observed.lock().unwrap().len()>=3,"the frame path cannot reuse only the earlier text acceptance");
        assert!(!written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))),"cancelled question audio must never enter the writer");
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_pending_question_releases_a_held_tts_provider_without_waiting_for_a_chunk() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let dispatcher=PendingQuestionDispatcher::new(None);deps.deps.dispatcher=dispatcher.clone();
        let first=Arc::new(HeldFirstQuestionChunk {entered:tokio::sync::Notify::new(),release:tokio::sync::Semaphore::new(0),produced:Default::default()});
        let speech=MockSpeech::new();speech.push_transcript("下一条普通请求");
        deps.deps.speech=Arc::new(HeldQuestionSpeech {inner:speech,first:first.clone()});
        let registry=deps.deps.registry.clone();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=pending_question_connection(&registry).await;
        registry.speak_requested_context_source("aa:bb:cc:dd:ee:ff",&connection,"哪一天出发？".into(),
            dispatcher.authority.source.clone(),Some(dispatcher.authority.presentation_id.clone()),pending_question_context()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3),first.entered.notified()).await.unwrap();
        dispatcher.pending.store(false,std::sync::atomic::Ordering::SeqCst);
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        assert!(!first.produced.load(std::sync::atomic::Ordering::SeqCst),"revocation must release the held provider without waiting for its first chunk");
        assert!(!written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))));
        // The connection loop remains available after revocation of this one
        // question, rather than retaining an unending Speaking media owner.
        manual_utterance(&tx).await;
        tokio::time::timeout(Duration::from_secs(3),async {
            while dispatcher.inner.dispatched_requests().is_empty() {tokio::task::yield_now().await;}
        }).await.unwrap();
        assert!(dispatcher.inner.dispatched_requests()[0].presented_question_id.is_none(),
            "revocation clears only this original question's local answer target");
        drop(tx);task.await.unwrap();
    }

    struct RevocableDirectDispatcher {inner:MockDispatcher,valid:std::sync::atomic::AtomicBool}
    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for RevocableDirectDispatcher {
        async fn input_context_matches(&self,_:&RobotTurnRequest)->anyhow::Result<bool> {Ok(self.valid.load(std::sync::atomic::Ordering::SeqCst))}
        async fn question_presented(&self,request:&RobotTurnRequest,source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation:&str)->anyhow::Result<()> {
            self.inner.question_presented(request,source,presentation).await
        }
        async fn ensure_companion_session(&self,companion:&str,robot:&str)->anyhow::Result<String> {self.inner.ensure_companion_session(companion,robot).await}
        async fn dispatch(&self,request:RobotTurnRequest)->anyhow::Result<mpsc::Receiver<TurnEvent>> {self.inner.dispatch(request).await}
        async fn cancel(&self,request:&RobotTurnRequest)->anyhow::Result<()> {self.inner.cancel(request).await}
        async fn vad_tuning(&self,_:&str)->crate::vad::VadTuning {Default::default()}
        async fn vad_engine(&self,_:&str)->String {"energy".into()}
    }

    #[tokio::test]
    async fn direct_scope_revocation_clears_its_question_without_emitting_a_late_failure_sentence() {
        let (mut deps,link,tx,written,_dir)=harness(true).await;
        let dispatcher=Arc::new(RevocableDirectDispatcher {inner:MockDispatcher::new(),valid:std::sync::atomic::AtomicBool::new(true)});
        dispatcher.inner.script_turn(vec![TurnEvent::Question {
            source:nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:"direct-original-question".into(),revision:17,through_seq:17},
            text:"请问几点出发？".into(),presentation_id:"robot-question:original:17".into(),
        },TurnEvent::Done]);
        dispatcher.inner.script_turn(vec![TurnEvent::Done]);deps.deps.dispatcher=dispatcher.clone();
        let first=Arc::new(HeldFirstQuestionChunk {entered:tokio::sync::Notify::new(),release:tokio::sync::Semaphore::new(0),produced:Default::default()});
        let speech=MockSpeech::new();speech.push_transcript("第一个问题请求");speech.push_transcript("下一条普通请求");
        deps.deps.speech=Arc::new(HeldQuestionSpeech {inner:speech,first:first.clone()});
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        manual_utterance(&tx).await;
        tokio::time::timeout(Duration::from_secs(3),first.entered.notified()).await.unwrap();
        dispatcher.valid.store(false,std::sync::atomic::Ordering::SeqCst);
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        assert!(!first.produced.load(std::sync::atomic::Ordering::SeqCst));
        assert!(texts(&written).iter().filter(|frame|frame["type"]=="tts"&&frame["state"]=="sentence_start")
            .all(|frame|frame["text"]=="请问几点出发？"),"revoked context cannot produce an unguarded generic failure sentence");
        dispatcher.valid.store(true,std::sync::atomic::Ordering::SeqCst);
        manual_utterance(&tx).await;
        tokio::time::timeout(Duration::from_secs(3),async {
            while dispatcher.inner.dispatched_requests().len()<2 {tokio::task::yield_now().await;}
        }).await.unwrap();
        assert!(dispatcher.inner.dispatched_requests()[1].presented_question_id.is_none());
        assert!(!written.lock().unwrap().iter().any(|frame|matches!(frame,Frame::Binary(_))));
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn desktop_playback_uses_existing_text_and_stops_when_permission_is_revoked() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let registry = deps.deps.registry.clone();
        let robot_id = "aa:bb:cc:dd:ee:ff";
        let mut permissions = crate::registry::RobotPermissions::default();
        permissions.proactive_speech = true;
        registry.set_permissions(robot_id, permissions).await.unwrap();
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.to_owned())).await.unwrap();
        let connection = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(connection) = registry.current_connection(robot_id).await { break connection; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        registry.speak(robot_id, &connection, "已有回复。".repeat(100)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while deps.speech.synthesized_text().is_empty() { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
        registry.set_permissions(robot_id, crate::registry::RobotPermissions::default()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !texts(&written).iter().any(|event| event["type"] == "tts" && event["state"] == "stop") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert!(deps.dispatcher.dispatched_text().is_empty(), "playback must never start a model turn");
        assert!(deps.dispatcher.cancelled().is_empty(), "stopping playback must not cancel conversation work");
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn canonical_result_playback_deduplicates_on_the_original_connection() {
        let (deps,link,tx,written,_dir)=harness(true).await;
        let registry=deps.deps.registry.clone();
        let robot_id="aa:bb:cc:dd:ee:ff";
        assert!(!registry.get(robot_id).await.unwrap().permissions.proactive_speech,
            "requested replies must work without enabling unrelated proactive announcements");
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=tokio::time::timeout(Duration::from_secs(2),async {
            loop { if let Some(connection)=registry.current_connection(robot_id).await {break connection;}
                tokio::time::sleep(Duration::from_millis(5)).await; }
        }).await.unwrap();
        let source=nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:"canonical-answer".into(),revision:42,through_seq:42};
        assert!(registry.speak(robot_id,&connection,"无请求的公告".into()).await.is_err());
        registry.speak_requested_source(robot_id,&connection,"任务完成。".into(),source.clone()).await.unwrap();
        registry.speak_requested_source(robot_id,&connection,"任务完成。".into(),source.clone()).await.unwrap();
        await_frame(&written,|frame|frame["type"]=="tts"&&frame["state"]=="stop").await;
        assert_eq!(deps.speech.synthesized_text(),vec!["任务完成。"]);
        assert!(matches!(registry.speak_requested_source(robot_id,&connection,"改写的结果".into(),source.clone()).await,
            Err(RobotPlaybackError::SourceChanged)));
        assert!(matches!(registry.speak_requested_source(robot_id,"another-connection","任务完成。".into(),source).await,
            Err(RobotPlaybackError::Unavailable)));
        assert!(deps.dispatcher.dispatched_text().is_empty());
        assert!(deps.dispatcher.cancelled().is_empty());
        drop(tx);task.await.unwrap();
    }

    #[tokio::test]
    async fn explicit_saved_replay_preserves_segment_order_and_replays_again_only_on_a_new_owner_request() {
        let (deps,link,tx,written,_dir)=harness(true).await;
        let registry=deps.deps.registry.clone();let robot="aa:bb:cc:dd:ee:ff";
        let mut permissions=registry.get(robot).await.unwrap().permissions;permissions.proactive_speech=true;
        registry.set_permissions(robot,permissions).await.unwrap();
        let task=tokio::spawn(run_session(link,deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        let connection=tokio::time::timeout(Duration::from_secs(3),async {
            loop {if let Some(connection)=registry.current_connection(robot).await{break connection;}tokio::task::yield_now().await;}
        }).await.unwrap();
        let response=||crate::services::RobotSavedResponse {
            segments:vec![(nomifun_voice_contracts::VoiceSpeechSourceRef{message_id:"final-seal-a".into(),revision:10,through_seq:12},"第一段。".into()),
                (nomifun_voice_contracts::VoiceSpeechSourceRef{message_id:"final-seal-b".into(),revision:11,through_seq:12},"第二段。".into())],
            context:crate::registry::RobotPlaybackContext {session_id:"saved-source-session".into(),input_context:crate::services::RobotInputContext {
                binding_version:1,context_floor:0,presented_question_id:None,work_operation_id:None,execution_generation:None,background_work:None,background_work_ambiguous:false}},
        };
        for replay in 1..=2 {
            tokio::time::timeout(Duration::from_secs(3),async {
                loop {match registry.replay_saved_response(robot,&connection,response()).await {
                    Ok(())=>break,Err(RobotPlaybackError::Busy)=>tokio::task::yield_now().await,Err(error)=>panic!("saved replay: {error}"),
                }}
            }).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5),async {
                while texts(&written).iter().filter(|frame|frame["type"]=="tts"&&frame["state"]=="stop").count()<replay {tokio::task::yield_now().await;}
            }).await.unwrap();
        }
        assert_eq!(deps.speech.synthesized_text(),vec!["第一段。","第二段。","第一段。","第二段。"]);
        assert!(deps.dispatcher.dispatched_requests().is_empty(),"saved source replay cannot open a model Turn");
        drop(tx);task.await.unwrap();
    }

    /// Poll until a written frame satisfies `predicate`, or fail loudly.
    ///
    /// Preferred over sleeping a fixed duration: the wait then scales with how
    /// busy the machine is instead of assuming a bound that a loaded CI box or a
    /// parallel workspace run will exceed.
    async fn await_frame(
        frames: &Arc<Mutex<Vec<Frame>>>,
        predicate: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        const LIMIT: std::time::Duration = std::time::Duration::from_secs(10);
        let deadline = tokio::time::Instant::now() + LIMIT;
        loop {
            if let Some(found) = texts(frames).into_iter().find(&predicate) {
                return found;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no frame matched within {LIMIT:?}; frames so far: {:?}",
                texts(frames)
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
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

    /// A plain 300 Hz tone: loud enough for the energy engine, and scored at
    /// about one percent by Silero. That difference is what makes the engine the
    /// profile asked for observable from outside.
    async fn send_tone(tx: &mpsc::Sender<Frame>, ms: u32) {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        let pcm: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect();
        let packets = crate::audio::OpusStreamEncoder::new_uplink_for_test()
            .unwrap()
            .encode_frames(&pcm)
            .unwrap();
        for packet in packets {
            tx.send(Frame::Binary(bytes::Bytes::from(packet)))
                .await
                .unwrap();
        }
    }

    /// Run one tone-then-silence round with `engine` named by the profile and
    /// report whether the session decided a turn had happened.
    async fn tone_round_transcribes(engine: &str) -> bool {
        let (deps, link, tx, _written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        deps.dispatcher_mock().set_vad_engine(engine);
        speech.push_transcript("嗯");

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
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        drop(tx);
        task.await.unwrap();
        speech.transcribe_calls() > 0
    }

    #[tokio::test]
    async fn the_profile_chooses_the_endpointing_engine() {
        assert!(
            tone_round_transcribes("energy").await,
            "the energy engine hears a plain tone, so the profile's choice reached the session"
        );
        assert!(
            tone_round_transcribes("no-such-engine").await,
            "an engine this build does not know falls back to energy rather than going deaf"
        );
    }

    #[tokio::test]
    async fn silero_from_the_profile_ignores_a_plain_tone() {
        if crate::vad::silero::SileroVad::new(crate::vad::VadTuning::default()).is_err() {
            eprintln!("skipping: ONNX runtime unavailable, silero degrades to energy here");
            return;
        }
        assert!(
            !tone_round_transcribes("silero").await,
            "silero must not mistake a plain tone for speech"
        );
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

    /// A full turn, with the bracketed annotation the model actually emits. It
    /// must reach neither the screen nor the face: the text channel drives no
    /// expression at all any more, and the guard deletes the annotation.
    #[tokio::test]
    async fn a_full_turn_produces_stt_sentence_audio_and_stop() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("今天天气怎么样");
        dispatcher.script_turn(vec![
            TurnEvent::Text("[winking] 晴朗得很。".into()),
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

        // Wait for the turn's terminal frame rather than for a fixed duration.
        // A wall-clock sleep raced the pipeline: under a saturated CPU (the full
        // workspace test run) 600ms elapsed before `tts:stop` was written and
        // the assertions below saw a half-finished turn.
        await_frame(&written, |frame| {
            frame["type"] == "tts" && frame["state"] == "stop"
        })
        .await;
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
            !types.contains(&"llm".to_owned()),
            "a healthy turn sets no face: model text is not an expression channel: {types:?}"
        );
        assert!(types.contains(&"tts:start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:sentence_start".to_owned()), "{types:?}");
        assert!(types.contains(&"tts:stop".to_owned()), "{types:?}");

        let stt = sent.iter().find(|m| m["type"] == "stt").unwrap();
        assert_eq!(stt["text"], "今天天气怎么样");
        let sentence = sent
            .iter()
            .find(|m| m["type"] == "tts" && m["state"] == "sentence_start")
            .unwrap();
        assert_eq!(
            sentence["text"], "晴朗得很。",
            "the bracketed annotation is stripped before display"
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
        // Wait for the reply's terminal frame, not a fixed duration: a
        // wall-clock bound races the pipeline whenever the machine is busy.
        await_frame(&written, |frame| frame["state"] == "stop").await;
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
    async fn asr_failure_shows_a_notice_instead_of_silence() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        // A configured-but-unusable ASR (the fresh-install "no model" case).
        speech.fail_next_transcribe("no speech-recognition model configured");

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
            "a failed transcription must not spend a model turn"
        );
        let sent = texts(&written);
        let notices: Vec<&str> = sent
            .iter()
            .filter(|m| m["type"] == "tts" && m["state"] == "sentence_start")
            .filter_map(|m| m["text"].as_str())
            .collect();
        assert!(
            notices.iter().any(|t| t.contains("语音识别")),
            "the ASR failure must be visible on the device screen, got {notices:?}"
        );
        assert!(
            sent.iter().any(|m| m["type"] == "llm" && m["emotion"] == "sad"),
            "the notice still sets a face — STATE-driven expression is the part that survived"
        );
    }

    #[tokio::test]
    async fn silent_action_turns_return_idle_without_tts_and_accept_the_next_question() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        let status = deps.deps.status.clone();
        speech.push_transcript("坐下");
        speech.push_transcript("站起来");
        speech.push_transcript("现在天气如何");
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Text("有小雨。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into())).await.unwrap();
        for expected in 1..=3 {
            tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
            send_audio(&tx, 200, true).await;
            tx.send(Frame::Text(r#"{"session_id":"s","type":"listen","state":"stop"}"#.into())).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if dispatcher.dispatched_text().len() == expected
                        && status.snapshot().await.first().is_some_and(|snapshot| snapshot.phase == "idle")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.expect("each admitted turn must settle and release the next utterance");
            if expected <= 2 {
                assert!(speech.synthesized_text().is_empty(), "silent actions must not invoke speech synthesis");
                assert!(!texts(&written).iter().any(|message| message["type"] == "tts"),
                    "silent actions must not start speaking, display a sentence or report a false failure");
                assert!(!written.lock().unwrap().iter().any(|frame| matches!(frame, Frame::Binary(_))),
                    "silent actions have no downlink audio");
            }
        }
        assert_eq!(dispatcher.dispatched_text(), vec!["坐下", "站起来", "现在天气如何"]);
        assert_eq!(speech.synthesized_text(), vec!["有小雨。".to_owned()], "the following question still gets its answer");
        drop(tx);
        task.await.unwrap();
        assert_eq!(status.snapshot().await[0].phase, "offline");
    }

    async fn silent_actions_keep_one_automatic_listen_window_usable(mode: &str) {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        let status = deps.deps.status.clone();
        // Deterministic protocol/endpointing coverage; loading a provider or
        // Silero is unrelated to whether the next utterance is still admitted.
        dispatcher.set_vad_engine("energy");
        speech.push_transcript("坐下");
        speech.push_transcript("站起来");
        speech.push_transcript("现在天气如何");
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Text("有小雨。".into()), TurnEvent::Done]);

        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await.unwrap();
        tx.send(Frame::Text(serde_json::json!({
            "session_id": "s", "type": "listen", "state": "start", "mode": mode,
        }).to_string())).await.unwrap();

        for expected in 1..=3 {
            // One device listening window, with server-side VAD endpoints.
            // In particular, do not manufacture a new listen.start/stop after
            // Done: a silent physical robot has no speech transition that
            // would make the firmware send that otherwise missing frame.
            send_tone(&tx, 300).await;
            send_audio(&tx, 900, false).await;
            let settled = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if dispatcher.dispatched_text().len() == expected
                        && status.snapshot().await.first().is_some_and(|snapshot| snapshot.phase == "idle")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await;
            if settled.is_err() {
                let dispatched = dispatcher.dispatched_text();
                let calls = speech.transcribe_calls();
                drop(tx);
                task.await.unwrap();
                panic!("{mode} utterance {expected} was not admitted/settled in the existing listen window; dispatched={dispatched:?}, ASR calls={calls}");
            }
            if expected <= 2 {
                assert!(speech.synthesized_text().is_empty(), "silent actions must not synthesize an acknowledgement");
                assert!(!texts(&written).iter().any(|message|
                    message["type"] == "tts" && message["state"] == "sentence_start"),
                    "silent actions must not display an acknowledgement or false failure");
                assert!(!written.lock().unwrap().iter().any(|frame| matches!(frame, Frame::Binary(_))),
                    "silent actions must not send downlink speech audio");
            }
        }
        assert_eq!(dispatcher.dispatched_text(), vec!["坐下", "站起来", "现在天气如何"]);
        assert_eq!(speech.transcribe_calls(), 3, "each distinct utterance is transcribed exactly once");
        assert_eq!(speech.synthesized_text(), vec!["有小雨。".to_owned()], "a later question still gets its spoken answer");
        drop(tx);
        task.await.unwrap();
        assert_eq!(status.snapshot().await[0].phase, "offline");
    }

    #[tokio::test]
    async fn silent_actions_auto_window_accepts_following_voice_without_another_listen_start() {
        silent_actions_keep_one_automatic_listen_window_usable("auto").await;
    }

    #[tokio::test]
    async fn silent_actions_realtime_window_accepts_following_voice_without_another_listen_start() {
        silent_actions_keep_one_automatic_listen_window_usable("realtime").await;
    }

    async fn automatic_listen_window_stays_closed_until_the_next_start(mode: &str, close: &str) {
        let (mut deps, link, tx, written, _dir) = harness(true).await;
        let mut calls = controlled_asr(&mut deps);
        let dispatcher = deps.dispatcher_mock();
        let status = deps.deps.status.clone();
        dispatcher.script_turn(vec![TurnEvent::Done]);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true}}"#.into()))
            .await.unwrap();
        let initialize = await_frame(&written, |frame| frame["payload"]["method"] == "initialize").await;
        tx.send(Frame::Text(serde_json::json!({
            "type": "listen", "state": "start", "mode": mode,
        }).to_string())).await.unwrap();
        let close_message = match close {
            "stop" => serde_json::json!({"type": "listen", "state": "stop"}),
            "abort" => serde_json::json!({"type": "abort", "reason": "cancel_work"}),
            other => panic!("unsupported fixture control: {other}"),
        };
        tx.send(Frame::Text(close_message.to_string())).await.unwrap();
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;

        // tools/list is emitted only after the actor has consumed this MCP
        // response, so it fences every earlier audio frame in the read loop.
        // The closed-window assertion is not based on an arbitrary sleep while
        // the frames could still be waiting to be read.
        reply_mcp(&tx, &initialize, serde_json::json!({})).await;
        await_frame(&written, |frame| frame["payload"]["method"] == "tools/list").await;
        assert!(tokio::time::timeout(Duration::from_millis(100), calls.recv()).await.is_err(),
            "{mode} must discard voice after explicit {close}, not restart a microphone window");
        assert!(dispatcher.dispatched_text().is_empty());

        tx.send(Frame::Text(serde_json::json!({
            "type": "listen", "state": "start", "mode": mode,
        }).to_string())).await.unwrap();
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        next_asr(&mut calls).await.reply.send(Ok("重新开始后坐下".into())).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if dispatcher.dispatched_text() == vec!["重新开始后坐下"]
                    && status.snapshot().await.first().is_some_and(|snapshot| snapshot.phase == "idle")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("only a new explicit listen.start may reopen the microphone window");
        assert!(calls.try_recv().is_err(), "there is no extra closed-window ASR request");
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn automatic_listen_stop_rejects_voice_until_a_new_listen_start() {
        for mode in ["auto", "realtime"] {
            automatic_listen_window_stays_closed_until_the_next_start(mode, "stop").await;
        }
    }

    #[tokio::test]
    async fn automatic_listen_abort_rejects_voice_until_a_new_listen_start() {
        for mode in ["auto", "realtime"] {
            automatic_listen_window_stays_closed_until_the_next_start(mode, "abort").await;
        }
    }

    #[tokio::test]
    async fn realtime_voice_after_spoken_reply_needs_no_new_listen_start() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        let status = deps.deps.status.clone();
        dispatcher.set_vad_engine("energy");
        speech.push_transcript("在吗");
        speech.push_transcript("坐下");
        dispatcher.script_turn(vec![TurnEvent::Text("在的。".into()), TurnEvent::Done]);
        dispatcher.script_turn(vec![TurnEvent::Done]);
        let task = tokio::spawn(run_session(link, deps.deps.clone()));
        tx.send(Frame::Text(r#"{"type":"hello","version":1,"transport":"websocket"}"#.into()))
            .await.unwrap();
        tx.send(Frame::Text(r#"{"type":"listen","state":"start","mode":"realtime"}"#.into()))
            .await.unwrap();
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        await_frame(&written, |frame| frame["type"] == "tts" && frame["state"] == "stop").await;
        // Realtime firmware stays in its existing microphone window across
        // playback. Do not fake the auto-mode firmware's next listen.start.
        send_tone(&tx, 300).await;
        send_audio(&tx, 900, false).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if dispatcher.dispatched_text().len() == 2
                    && status.snapshot().await.first().is_some_and(|snapshot| snapshot.phase == "idle")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("realtime speech after TTS must still reach ASR and the next Agent turn");
        assert_eq!(dispatcher.dispatched_text(), vec!["在吗", "坐下"]);
        assert_eq!(speech.transcribe_calls(), 2);
        assert_eq!(speech.synthesized_text(), vec!["在的。".to_owned()]);
        assert_eq!(texts(&written).iter().filter(|frame|
            frame["type"] == "tts" && frame["state"] == "sentence_start").count(), 1,
            "the following action has no spoken/displayed acknowledgement");
        drop(tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn tts_failure_shows_a_one_time_notice_beside_the_reply() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        speech.push_transcript("你好");
        speech.fail_next_synthesize("no speech-synthesis model configured");
        deps.dispatcher_mock()
            .script_turn(vec![TurnEvent::Text("在的。".into()), TurnEvent::Done]);

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
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(tx);
        task.await.unwrap();

        let sent = texts(&written);
        let sentences: Vec<&str> = sent
            .iter()
            .filter(|m| m["type"] == "tts" && m["state"] == "sentence_start")
            .filter_map(|m| m["text"].as_str())
            .collect();
        assert!(
            sentences.iter().any(|t| t.contains("在的")),
            "the reply text still reaches the screen, got {sentences:?}"
        );
        assert!(
            sentences.iter().any(|t| t.contains("语音合成")),
            "the missing-voice reason must be visible once, got {sentences:?}"
        );
    }

    #[tokio::test]
    async fn emoji_faces_and_stickers_are_filtered_before_synthesis() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        speech.push_transcript("天气如何");
        deps.dispatcher_mock().script_turn(vec![
            // Faces and image syntax can cross arbitrary provider chunks. The
            // splitter must keep them intact until the consumer guard runs.
            TurnEvent::Text("北京现在是 🌤️ (•̀".into()),
            TurnEvent::Text("ᴗ•́)و 挺舒服的天气呢~。".into()),
            TurnEvent::Text("(^".into()),
            TurnEvent::Text("_^)！\nQAQ!\n[笑哭]！\n👍。\n[表情包：开心]\n!".into()),
            TurnEvent::Text("[smile](https://example.test/".into()),
            TurnEvent::Text("sticker.png)\n1️⃣🏴\u{E0067}\u{E0062}\u{E007F}👍".into()),
            TurnEvent::Done,
        ]);

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
        await_frame(&written, |frame| frame["type"] == "tts" && frame["state"] == "stop").await;
        drop(tx);
        task.await.unwrap();

        let spoken = speech.synthesized_text();
        assert!(
            spoken.iter().any(|t| t.contains("挺舒服的天气")),
            "the words around the emoji are still spoken: {spoken:?}"
        );
        assert_eq!(spoken, vec!["北京现在是 挺舒服的天气呢~。"],
            "decoration-only fragments never invoke synthesis, while the natural reply survives");
        let displayed: Vec<String> = texts(&written).into_iter()
            .filter(|frame| frame["type"] == "tts" && frame["state"] == "sentence_start")
            .filter_map(|frame| frame["text"].as_str().map(str::to_owned)).collect();
        assert_eq!(displayed, spoken, "the device display uses the same plain reply");
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
        tokio::time::timeout(Duration::from_secs(3), async {
            while dispatcher.dispatched_text().is_empty() || speech.synthesized_text().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("the turn must actually start before testing its cancellation");

        tx.send(Frame::Text(
            r#"{"session_id":"s","type":"abort","reason":"cancel_work"}"#.into(),
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
    async fn a_failure_with_no_fallback_reports_sadly_and_stops() {
        let (deps, link, tx, written, _dir) = harness(true).await;
        let speech = deps.speech_mock();
        let dispatcher = deps.dispatcher_mock();
        speech.push_transcript("你好");
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

        assert_eq!(dispatcher.dispatched_text().len(), 1, "device failures must not replay user input");
        let sent = texts(&written);
        assert!(
            sent.iter().any(|m| m["type"] == "llm" && m["emotion"] == "sad"),
            "the robot looks sad about it — the surviving, STATE-driven face"
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
