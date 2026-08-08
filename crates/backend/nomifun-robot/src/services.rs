//! The seam between the robot pipelines and the rest of nomifun.
//!
//! The pipelines only ever see these traits, so every audio/text path is
//! testable without a model provider, a conversation service, or a device. The
//! real implementations live in `crate::wiring` — same crate, separate module,
//! so the dependency direction stays obvious.

use crate::audio::AudioBuffer;
use crate::vad::VadTuning;

/// Who a speech call is for. Both ids are logged and used to pick per-companion
/// model slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechContext {
    pub robot_id: String,
    pub companion_id: String,
}

/// ASR, TTS and one-shot vision.
#[async_trait::async_trait]
pub trait SpeechServices: Send + Sync {
    /// WAV bytes in, transcript out. An empty transcript is a valid answer
    /// (silence or noise) and must not be an error.
    async fn transcribe(&self, ctx: &SpeechContext, wav: Vec<u8>) -> anyhow::Result<String>;
    /// One sentence in, mono PCM out (any sample rate; the caller resamples).
    async fn synthesize(&self, ctx: &SpeechContext, text: &str) -> anyhow::Result<AudioBuffer>;
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
    /// The turn finished normally.
    Done,
    /// The turn failed. `provider_fault` means the error looked like a model or
    /// provider problem, which is what makes a fallback-model retry sensible.
    Failed { message: String, provider_fault: bool },
}

/// Companion conversation access.
#[async_trait::async_trait]
pub trait CompanionTurnDispatcher: Send + Sync {
    /// Find or create the long-lived thread for this `(robot, companion)` pair.
    async fn ensure_thread(&self, robot_id: &str, companion_id: &str) -> anyhow::Result<String>;
    /// Start a turn and stream its events.
    async fn dispatch(
        &self,
        conversation_id: &str,
        text: &str,
        use_fallback_model: bool,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>>;
    /// Stop the in-flight turn (the public `cancel`, never a runtime kill).
    async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()>;
    /// The companion's endpointing tunables.
    async fn vad_tuning(&self, companion_id: &str) -> VadTuning;
    /// The companion's chosen endpointing engine (`voice.vad.engine`). Resolved
    /// by name rather than passed as a built engine because the session builds
    /// one per connection, and [`crate::vad::build_engine`] owns the fallback
    /// when the named engine cannot load.
    async fn vad_engine(&self, companion_id: &str) -> String;
    /// Whether a fallback chat model is configured for this companion.
    async fn has_fallback_model(&self, companion_id: &str) -> bool;
}

#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    //! Programmable doubles for the two seams above.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
        cancelled: Mutex<Vec<String>>,
        fallback_dispatches: AtomicUsize,
        has_fallback: AtomicBool,
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

        pub fn set_has_fallback(&self, value: bool) {
            self.has_fallback.store(value, Ordering::SeqCst);
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

        pub fn cancelled(&self) -> Vec<String> {
            self.cancelled.lock().unwrap().clone()
        }

        pub fn fallback_dispatches(&self) -> usize {
            self.fallback_dispatches.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl CompanionTurnDispatcher for MockDispatcher {
        async fn ensure_thread(
            &self,
            robot_id: &str,
            companion_id: &str,
        ) -> anyhow::Result<String> {
            let key = format!("{robot_id}|{companion_id}");
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
            _conversation_id: &str,
            text: &str,
            use_fallback_model: bool,
        ) -> anyhow::Result<tokio::sync::mpsc::Receiver<TurnEvent>> {
            self.dispatched.lock().unwrap().push(text.to_owned());
            if use_fallback_model {
                self.fallback_dispatches.fetch_add(1, Ordering::SeqCst);
            }
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

        async fn cancel(&self, conversation_id: &str) -> anyhow::Result<()> {
            self.cancelled
                .lock()
                .unwrap()
                .push(conversation_id.to_owned());
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

        async fn has_fallback_model(&self, _companion_id: &str) -> bool {
            self.has_fallback.load(Ordering::SeqCst)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{MockDispatcher, MockSpeech};
    use super::*;
    use std::sync::Arc;

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

        let conversation = dispatcher.ensure_thread("aa:bb", "c1").await.unwrap();
        assert!(!conversation.is_empty());
        assert_eq!(
            dispatcher.ensure_thread("aa:bb", "c1").await.unwrap(),
            conversation,
            "same thread reused"
        );

        let mut rx = dispatcher.dispatch(&conversation, "在吗", false).await.unwrap();
        let mut chunks = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                TurnEvent::Text(t) => chunks.push(t),
                TurnEvent::Done => break,
                TurnEvent::Failed { message, .. } => panic!("unexpected failure: {message}"),
            }
        }
        assert_eq!(chunks, vec!["你好".to_owned(), "呀。".to_owned()]);
        assert_eq!(dispatcher.dispatched_text(), vec!["在吗".to_owned()]);
    }

    #[tokio::test]
    async fn mock_dispatcher_records_fallback_usage_and_cancels() {
        let dispatcher = Arc::new(MockDispatcher::new());
        dispatcher.script_turn(vec![TurnEvent::Done]);
        dispatcher.set_has_fallback(true);
        assert!(dispatcher.has_fallback_model("c1").await);

        let _ = dispatcher.dispatch("conv-1", "hi", true).await.unwrap();
        assert_eq!(dispatcher.fallback_dispatches(), 1);

        dispatcher.cancel("conv-1").await.unwrap();
        assert_eq!(dispatcher.cancelled(), vec!["conv-1".to_owned()]);
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
