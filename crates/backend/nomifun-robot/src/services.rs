//! The seam between the robot pipelines and the rest of nomifun.
//!
//! The pipelines only ever see these traits, so every audio/text path is
//! testable without a model provider, a conversation service, or a device. The
//! real implementations live in `crate::wiring` — same crate, separate module,
//! so the dependency direction stays obvious.

use crate::audio::AudioBuffer;
use crate::vad::VadTuning;
use futures_util::stream::{self, BoxStream};
pub use nomifun_model_invoke::voice::speech::{TranscriptEvent, TranscriptionSession};

/// Actual delivery behavior of the selected speech adapter. A batch result
/// is one complete synthesis, even if the device later encodes it in frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechDeliveryMode {
    Batch,
    Streaming,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechCapabilities {
    pub incremental_asr: bool,
    pub streaming_tts: bool,
}

/// Pulling audio provides backpressure all the way to the provider response.
/// Dropping the stream drops the exact request; no detached producer remains.
pub struct SpeechAudioStream {
    pub mode: SpeechDeliveryMode,
    pub audio: BoxStream<'static, anyhow::Result<AudioBuffer>>,
}

/// Who a speech call is for. Both ids are logged and used to pick per-companion
/// model slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechContext {
    pub robot_id: String,
    pub companion_id: String,
}

#[async_trait::async_trait]
pub trait RobotPlaybackSource: Send + Sync {
    async fn response_source(&self, robot_id: &str, conversation_id: &str) -> Result<RobotSavedResponse, String>;
}
pub struct RobotSavedResponse {
    pub segments:Vec<(nomifun_voice_contracts::VoiceSpeechSourceRef,String)>,
    pub context:crate::registry::RobotPlaybackContext,
}

/// ASR, TTS and one-shot vision.
#[async_trait::async_trait]
pub trait SpeechServices: Send + Sync {
    /// Discovery is local and task scoped. It must never open a provider
    /// connection just because a robot connection was established.
    async fn capabilities(&self, _ctx: &SpeechContext) -> anyhow::Result<SpeechCapabilities> {
        Ok(SpeechCapabilities::default())
    }
    /// Only real incremental-input protocols return a session. Batch/SSE
    /// full-upload protocols return None and use `transcribe` at endpointing.
    async fn open_transcription(
        &self,
        _ctx: &SpeechContext,
        _sample_rate: u32,
    ) -> anyhow::Result<Option<TranscriptionSession>> {
        Ok(None)
    }
    /// WAV bytes in, transcript out. An empty transcript is a valid answer
    /// (silence or noise) and must not be an error.
    async fn transcribe(&self, ctx: &SpeechContext, wav: Vec<u8>) -> anyhow::Result<String>;
    /// One sentence in, mono PCM out (any sample rate; the caller resamples).
    async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer>;
    /// Batch is explicit and yields its complete result once. Streaming
    /// implementations must yield provider audio before response completion.
    async fn synthesize_stream(
        &self,
        ctx: &SpeechContext,
        text: &str,
    ) -> anyhow::Result<SpeechAudioStream> {
        let audio = self.synthesize(ctx, text).await?;
        Ok(SpeechAudioStream {
            mode: SpeechDeliveryMode::Batch,
            audio: Box::pin(stream::once(async move { Ok(audio) })),
        })
    }
    /// A JPEG plus a question in, a natural-language answer out.
    async fn explain_image(
        &self,
        ctx: &SpeechContext,
        jpeg: Vec<u8>,
        question: &str,
    ) -> anyhow::Result<String>;
}

/// What the agent stream told us, reduced to what the downlink needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// An incremental slice of assistant text.
    Text(String),
    /// An irrevocable public prefix backed by its exact canonical source.
    PublicSpeech { source: nomifun_voice_contracts::VoiceSpeechSourceRef, text: String },
    /// Typed canonical interaction; its ID is captured at utterance onset.
    Question { source: nomifun_voice_contracts::VoiceSpeechSourceRef, text: String, presentation_id: String },
    /// The turn finished normally.
    Done,
    /// The turn failed. `provider_fault` means the error looked like a model or
    /// provider problem, which is what makes a fallback-model retry sensible.
    Failed { message: String, provider_fault: bool },
}

/// Companion conversation access.
#[derive(Debug,Clone,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotExecutionAttemptTarget {
    pub step_id:String,pub attempt_id:String,pub expected_step_version:i64,pub expected_attempt_version:i64,
    pub agent_session_id:String,pub turn_operation_id:String,pub binding_version:u64,pub execution_generation:u64,
}
#[derive(Debug,Clone,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotExecutionTarget {
    pub execution_id:String,pub expected_version:i64,
    pub source_turn_operation_id:String,pub source_tool_operation_id:String,pub source_effect_id:String,
    pub attempt:Option<RobotExecutionAttemptTarget>,pub attempt_ambiguous:bool,
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct RobotInputContext {
    pub binding_version:u64,pub context_floor:u64,pub presented_question_id:Option<String>,
    pub work_operation_id:Option<String>,pub execution_generation:Option<u64>,
    pub background_work:Option<RobotExecutionTarget>,pub background_work_ambiguous:bool,
}
#[derive(Debug, Clone)]
pub struct RobotTurnRequest {
    pub robot_id: String,
    pub companion_id: String,
    pub conversation_id: String,
    pub connection_id: String,
    pub request_id: String,
    pub text: String,
    /// The exact question already presented when this physical utterance
    /// began. Preserved through ASR and queuing; never inferred at admission.
    /// This is internal host context, not an addition to the device wire.
    pub presented_question_id: Option<String>,
    pub captured_context:Option<RobotInputContext>,
}

/// Opaque short host authority held only across synchronous writer acceptance.
/// A media consumer never sees the connection or canonical owner locks inside.
pub struct RobotOutputLease { _hold:Box<dyn std::any::Any+Send> }
impl RobotOutputLease {
    pub fn new<T:Send+'static>(hold:T)->Self {Self {_hold:Box::new(hold)}}
}

/// Exact authority for a queued background question. Its source notice and
/// presentation must still name the same pending canonical interaction at
/// each writer acceptance, including after synthesis has started.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct RobotQuestionPlaybackAuthority {
    pub source:nomifun_voice_contracts::VoiceSpeechSourceRef,
    pub presentation_id:String,
}

#[derive(Clone)]
pub struct RobotOutputGuard {
    dispatcher:std::sync::Arc<dyn CompanionTurnDispatcher>,
    request:std::sync::Arc<RobotTurnRequest>,
    expected_proactive_revision:Option<u64>,
    question:Option<RobotQuestionPlaybackAuthority>,
}
impl RobotOutputGuard {
    pub fn new(dispatcher:std::sync::Arc<dyn CompanionTurnDispatcher>,request:RobotTurnRequest)->Self {Self {dispatcher,request:std::sync::Arc::new(request),expected_proactive_revision:None,question:None}}
    pub fn new_proactive(dispatcher:std::sync::Arc<dyn CompanionTurnDispatcher>,request:RobotTurnRequest,revision:u64)->Self {
        Self {dispatcher,request:std::sync::Arc::new(request),expected_proactive_revision:Some(revision),question:None}
    }
    pub fn with_question(mut self,question:RobotQuestionPlaybackAuthority)->Self {self.question=Some(question);self}
    pub async fn acquire(&self)->anyhow::Result<Option<RobotOutputLease>> {
        self.dispatcher.acquire_output_context(&self.request,self.expected_proactive_revision,self.question.as_ref()).await
    }
}

#[async_trait::async_trait]
pub trait CompanionTurnDispatcher: Send + Sync {
    async fn capture_input_context(&self,_companion:&str,_robot:&str,_session:&str,_connection:&str,_question:Option<String>)->anyhow::Result<Option<RobotInputContext>> {Ok(None)}
    async fn input_context_matches(&self,_request:&RobotTurnRequest)->anyhow::Result<bool> {Ok(true)}
    async fn acquire_output_context(&self,request:&RobotTurnRequest,_expected_proactive_revision:Option<u64>,question:Option<&RobotQuestionPlaybackAuthority>)->anyhow::Result<Option<RobotOutputLease>> {
        if question.is_some() {return Ok(None);}
        Ok(self.input_context_matches(request).await?.then(||RobotOutputLease::new(())))
    }
    /// Actor acknowledgement that this exact canonical question has entered
    /// its original connection's presentation, before accepting an answer.
    async fn question_presented(&self,_request:&RobotTurnRequest,_source:&nomifun_voice_contracts::VoiceSpeechSourceRef,_presentation_id:&str)->anyhow::Result<()> {Ok(())}
    /// Accepted supplements keep their exact original work target. A newer
    /// utterance never selects the latest running task after delayed ASR.
    async fn supplement(&self,_request:&RobotTurnRequest,_text:&str,_correction:bool,_input_id:&str)->anyhow::Result<()> {anyhow::bail!("This dispatcher does not accept work supplements")}
    async fn cancel_background_work(&self,_request:&RobotTurnRequest)->anyhow::Result<()> {anyhow::bail!("No exact background work owner is available")}
    async fn supplement_background_work(&self,_request:&RobotTurnRequest,_text:&str,_correction:bool,_input_id:&str)->anyhow::Result<()> {anyhow::bail!("No exact background attempt owner is available")}
    /// Resolve the independent ingress Session for this exact paired device.
    async fn ensure_companion_session(&self, companion_id: &str, robot_id: &str) -> anyhow::Result<String>;
    /// Enqueue one utterance without blocking the physical connection loop.
    async fn dispatch(&self, request: RobotTurnRequest) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>>;
    /// Cancel this exact utterance, never an unrelated turn of its Conversation.
    async fn cancel(&self, request: &RobotTurnRequest) -> anyhow::Result<()>;
    /// The companion's endpointing tunables.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// The companion's chosen endpointing engine (`voice.vad.engine`). Resolved
    /// by name rather than passed as a built engine because the session builds
    /// one per connection, and [`crate::vad::build_engine`] owns the fallback
    /// when the named engine cannot load.
    async fn vad_engine(&self, companion_id: &str) -> String;
}

#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    //! Programmable doubles for the two seams above.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Scriptable [`SpeechServices`].
    #[derive(Default)]
    pub struct MockSpeech {
        transcripts: Mutex<std::collections::VecDeque<String>>,
        transcribe_failure: Mutex<Option<String>>,
        transcribe_calls: AtomicUsize,
        synthesized: Mutex<Vec<String>>,
        synthesize_failure: Mutex<Option<String>>,
        tts_rate: Mutex<u32>,
        vision_answer: Mutex<String>,
        vision_failure: Mutex<Option<String>>,
    }

    impl MockSpeech {
        pub fn new() -> Self {
            Self {
                tts_rate: Mutex::new(24_000),
                ..Default::default()
            }
        }

        /// Queue one transcript to return.
        pub fn push_transcript(&self, text: &str) {
            self.transcripts.lock().unwrap().push_back(text.to_owned());
        }

        /// Make the next `transcribe` fail.
        pub fn fail_next_transcribe(&self, message: &str) {
            *self.transcribe_failure.lock().unwrap() = Some(message.to_owned());
        }

        /// Make the next `synthesize` fail.
        pub fn fail_next_synthesize(&self, message: &str) {
            *self.synthesize_failure.lock().unwrap() = Some(message.to_owned());
        }

        pub fn set_tts_rate(&self, rate: u32) {
            *self.tts_rate.lock().unwrap() = rate;
        }

        pub fn set_vision_answer(&self, text: &str) {
            *self.vision_answer.lock().unwrap() = text.to_owned();
        }

        /// Make the next `explain_image` fail.
        pub fn fail_next_vision(&self, message: &str) {
            *self.vision_failure.lock().unwrap() = Some(message.to_owned());
        }

        pub fn transcribe_calls(&self) -> usize {
            self.transcribe_calls.load(Ordering::SeqCst)
        }

        pub fn synthesized_text(&self) -> Vec<String> {
            self.synthesized.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl SpeechServices for MockSpeech {
        async fn transcribe(&self, _ctx: &SpeechContext, _wav: Vec<u8>) -> anyhow::Result<String> {
            self.transcribe_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(message) = self.transcribe_failure.lock().unwrap().take() {
                anyhow::bail!(message);
            }
            Ok(self
                .transcripts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }

        async fn synthesize(
            &self,
            _ctx: &SpeechContext,
            text: &str,
        ) -> anyhow::Result<AudioBuffer> {
            self.synthesized.lock().unwrap().push(text.to_owned());
            if let Some(message) = self.synthesize_failure.lock().unwrap().take() {
                anyhow::bail!(message);
            }
            let rate = *self.tts_rate.lock().unwrap();
            // ~80 ms of silence per character keeps frame counts realistic.
            let samples = (rate as usize / 1000) * 80 * text.chars().count().max(1);
            Ok(AudioBuffer {
                pcm: vec![0i16; samples],
                sample_rate: rate,
            })
        }

        async fn explain_image(
            &self,
            _ctx: &SpeechContext,
            _jpeg: Vec<u8>,
            _question: &str,
        ) -> anyhow::Result<String> {
            if let Some(message) = self.vision_failure.lock().unwrap().take() {
                anyhow::bail!(message);
            }
            Ok(self.vision_answer.lock().unwrap().clone())
        }
    }

    /// Scriptable [`CompanionTurnDispatcher`].
    #[derive(Default)]
    pub struct MockDispatcher {
        threads: Mutex<std::collections::BTreeMap<String, String>>,
        scripted: Mutex<std::collections::VecDeque<Vec<TurnEvent>>>,
        dispatched: Mutex<Vec<String>>,
        requests: Mutex<Vec<RobotTurnRequest>>,
        cancelled: Mutex<Vec<String>>,
        presented_questions: Mutex<Vec<String>>,
        tuning: Mutex<Option<VadTuning>>,
        engine: Mutex<Option<String>>,
    }

    impl MockDispatcher {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue the events one `dispatch` call will emit.
        pub fn script_turn(&self, events: Vec<TurnEvent>) {
            self.scripted.lock().unwrap().push_back(events);
        }


        pub fn set_vad_tuning(&self, tuning: VadTuning) {
            *self.tuning.lock().unwrap() = Some(tuning);
        }

        /// Pretend the companion profile names this endpointing engine.
        pub fn set_vad_engine(&self, engine: &str) {
            *self.engine.lock().unwrap() = Some(engine.to_owned());
        }

        pub fn dispatched_text(&self) -> Vec<String> {
            self.dispatched.lock().unwrap().clone()
        }

        pub fn dispatched_requests(&self) -> Vec<RobotTurnRequest> {
            self.requests.lock().unwrap().clone()
        }

        pub fn cancelled(&self) -> Vec<String> {
            self.cancelled.lock().unwrap().clone()
        }

        pub fn presented_questions(&self)->Vec<String> {self.presented_questions.lock().unwrap().clone()}

    }

    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for MockDispatcher {
        async fn question_presented(&self,_request:&RobotTurnRequest,_source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation_id:&str)->anyhow::Result<()> {
            self.presented_questions.lock().unwrap().push(presentation_id.to_owned());Ok(())
        }
        async fn ensure_companion_session(
            &self,
            companion_id: &str,
            robot_id: &str,
        ) -> anyhow::Result<String> {
            let key = format!("{companion_id}:{robot_id}");
            let mut threads = self.threads.lock().unwrap();
            let next = threads.len() + 1;
            let id = threads
                .entry(key)
                .or_insert_with(|| format!("conv-{next}"))
                .clone();
            Ok(id)
        }

        async fn dispatch(
            &self,
            request: RobotTurnRequest,
        ) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>> {
            self.requests.lock().unwrap().push(request.clone());
            self.dispatched.lock().unwrap().push(request.text);
            let events = self
                .scripted
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| vec![TurnEvent::Done]);
            let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
            tokio::spawn(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }

        async fn cancel(&self, request: &RobotTurnRequest) -> anyhow::Result<()> {
            self.cancelled
                .lock()
                .unwrap()
                .push(request.request_id.to_owned());
            Ok(())
        }

        async fn vad_tuning(&self, _companion_id: &str) -> VadTuning {
            self.tuning.lock().unwrap().unwrap_or_default()
        }

        async fn vad_engine(&self, _companion_id: &str) -> String {
            self.engine
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| crate::vad::DEFAULT_VAD_ENGINE.to_owned())
        }

    }
}

#[cfg(test)]
mod tests {
    use super::mock::{MockDispatcher, MockSpeech};
    use super::*;
    use std::sync::Arc;

    fn request(conversation: &str, text: &str) -> RobotTurnRequest {
        RobotTurnRequest {
            robot_id: "aa:bb".into(), companion_id: "c1".into(),
            conversation_id: conversation.into(), connection_id: "socket-1".into(),
            request_id: "utterance-1".into(), text: text.into(), presented_question_id:None,captured_context:None,
        }
    }

    fn ctx() -> SpeechContext {
        SpeechContext {
            robot_id: "aa:bb".into(),
            companion_id: "c1".into(),
        }
    }

    #[tokio::test]
    async fn mock_speech_returns_scripted_values_and_records_calls() {
        let speech = Arc::new(MockSpeech::new());
        speech.push_transcript("你好小智");
        speech.set_tts_rate(24_000);

        let text = speech.transcribe(&ctx(), vec![1, 2, 3]).await.unwrap();
        assert_eq!(text, "你好小智");
        assert_eq!(speech.transcribe_calls(), 1);

        let audio = speech.synthesize(&ctx(), "在呢").await.unwrap();
        assert_eq!(audio.sample_rate, 24_000);
        assert!(
            !audio.pcm.is_empty(),
            "mock synthesises silence of a plausible length"
        );
        assert_eq!(speech.synthesized_text(), vec!["在呢".to_owned()]);
    }

    #[tokio::test]
    async fn mock_speech_can_be_scripted_to_fail() {
        let speech = Arc::new(MockSpeech::new());
        speech.fail_next_transcribe("network down");
        let error = speech.transcribe(&ctx(), vec![]).await.unwrap_err();
        assert!(error.to_string().contains("network down"));
        // The failure is consumed; the next call succeeds with the default.
        assert_eq!(speech.transcribe(&ctx(), vec![]).await.unwrap(), "");
    }

    #[tokio::test]
    async fn mock_dispatcher_streams_scripted_turn_events() {
        let dispatcher = Arc::new(MockDispatcher::new());
        dispatcher.script_turn(vec![
            TurnEvent::Text("你好".into()),
            TurnEvent::Text("呀。".into()),
            TurnEvent::Done,
        ]);

        let conversation = dispatcher.ensure_companion_session("c1", "r1").await.unwrap();
        assert!(!conversation.is_empty());
        assert_eq!(
            dispatcher.ensure_companion_session("c1", "r1").await.unwrap(),
            conversation,
            "same thread reused"
        );

        let mut rx = dispatcher.dispatch(request(&conversation, "在吗")).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                TurnEvent::Text(t) => chunks.push(t),
                TurnEvent::PublicSpeech {text,..}|TurnEvent::Question {text,..} => chunks.push(text),
                TurnEvent::Done => break,
                TurnEvent::Failed { message, .. } => panic!("unexpected failure: {message}"),
            }
        }
        assert_eq!(chunks, vec!["你好".to_owned(), "呀。".to_owned()]);
        assert_eq!(dispatcher.dispatched_text(), vec!["在吗".to_owned()]);
    }

    #[tokio::test]
    async fn mock_dispatcher_cancels_the_exact_utterance() {
        let dispatcher = Arc::new(MockDispatcher::new());
        dispatcher.script_turn(vec![TurnEvent::Done]);

        let request = request("conv-1", "hi");
        let _ = dispatcher.dispatch(request.clone()).await.unwrap();

        dispatcher.cancel(&request).await.unwrap();
        assert_eq!(dispatcher.cancelled(), vec!["utterance-1".to_owned()]);
    }

    #[tokio::test]
    async fn mock_dispatcher_serves_vad_tuning() {
        let dispatcher = Arc::new(MockDispatcher::new());
        assert_eq!(
            dispatcher.vad_tuning("c1").await,
            crate::vad::VadTuning::default()
        );
        dispatcher.set_vad_tuning(crate::vad::VadTuning {
            sensitivity: 0.9,
            min_silence_ms: 400,
        });
        let tuning = dispatcher.vad_tuning("c1").await;
        assert_eq!(tuning.sensitivity, 0.9);
        assert_eq!(tuning.min_silence_ms, 400);
    }
}
