//! Silero VAD via ONNX Runtime.
//!
//! The model is a streaming RNN: it takes 512-sample chunks at 16 kHz plus a
//! carried state tensor and returns P(speech) for that chunk. We buffer the
//! device's 60 ms (960-sample) frames into 512-sample chunks and take the
//! maximum probability across the chunks belonging to one frame, so a frame that
//! starts silent but ends in speech still counts as speech.
//!
//! Weights are embedded at compile time — no runtime download, no user setup.

use ort::session::Session;
use ort::value::Tensor;

use crate::audio::UPLINK_SAMPLE_RATE_HINT;

use super::{VadDecision, VadEngine, VadTuning, frame_ms};

/// Chunk size the model expects at 16 kHz.
const CHUNK: usize = 512;
/// Samples of the previous chunk the model expects to be prepended.
const CONTEXT: usize = 64;
/// Hidden state shape: 2 layers × 1 batch × 128 features.
const STATE_LEN: usize = 2 * 1 * 128;
/// Probability threshold at `sensitivity = 0.0` and `1.0`.
const P_AT_MIN_SENSITIVITY: f32 = 0.20;
const P_AT_MAX_SENSITIVITY: f32 = 0.80;

static MODEL: &[u8] = include_bytes!("../../assets/silero_vad.onnx");

/// Streaming Silero endpointer.
pub struct SileroVad {
    session: Session,
    state: Vec<f32>,
    context: Vec<f32>,
    pending: Vec<i16>,
    tuning: VadTuning,
    threshold: f32,
    speech_started: bool,
    trailing_silence_ms: u32,
}

impl SileroVad {
    /// Load the embedded model. Fails if ONNX Runtime is unavailable — callers
    /// must fall back to [`super::EnergyVad`], never propagate.
    pub fn new(tuning: VadTuning) -> anyhow::Result<Self> {
        let session = Session::builder()?.commit_from_memory(MODEL)?;
        let threshold = P_AT_MIN_SENSITIVITY
            + (P_AT_MAX_SENSITIVITY - P_AT_MIN_SENSITIVITY) * tuning.sensitivity;
        Ok(Self {
            session,
            state: vec![0.0; STATE_LEN],
            context: vec![0.0; CONTEXT],
            pending: Vec::with_capacity(CHUNK * 2),
            tuning,
            threshold,
            speech_started: false,
            trailing_silence_ms: 0,
        })
    }

    /// Run one 512-sample chunk, updating the carried state.
    ///
    /// The graph takes `CONTEXT + CHUNK` samples: the model's STFT needs the tail
    /// of the previous chunk to produce a full first window. Feeding a bare 512
    /// samples runs without error but returns near-zero probabilities for
    /// everything, so the context is not optional.
    fn speech_probability(&mut self, chunk: &[i16]) -> anyhow::Result<f32> {
        let mut samples: Vec<f32> = Vec::with_capacity(CONTEXT + CHUNK);
        samples.extend_from_slice(&self.context);
        samples.extend(chunk.iter().map(|s| *s as f32 / 32768.0));
        self.context
            .copy_from_slice(&samples[samples.len() - CONTEXT..]);
        let input = Tensor::from_array(([1usize, CONTEXT + CHUNK], samples))?;
        let state = Tensor::from_array(([2usize, 1, 128], self.state.clone()))?;
        // `sr` is a rank-0 int64 in the graph, not a one-element vector.
        let rate = Tensor::from_array(([] as [usize; 0], vec![
            i64::from(UPLINK_SAMPLE_RATE_HINT),
        ]))?;

        let outputs = self
            .session
            .run(ort::inputs!["input" => input, "state" => state, "sr" => rate])?;

        let (_, probability) = outputs["output"].try_extract_tensor::<f32>()?;
        let p = probability.first().copied().unwrap_or(0.0);
        let (_, next_state) = outputs["stateN"].try_extract_tensor::<f32>()?;
        if next_state.len() == STATE_LEN {
            self.state.copy_from_slice(next_state);
        }
        Ok(p)
    }
}

impl VadEngine for SileroVad {
    fn name(&self) -> &'static str {
        "silero"
    }

    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision {
        self.pending.extend_from_slice(pcm);
        let mut max_p = 0.0f32;
        let mut ran = false;
        while self.pending.len() >= CHUNK {
            let chunk: Vec<i16> = self.pending.drain(..CHUNK).collect();
            match self.speech_probability(&chunk) {
                Ok(p) => {
                    ran = true;
                    max_p = max_p.max(p);
                }
                Err(error) => {
                    // A mid-stream inference error must not end the call; treat
                    // the frame as silence and keep going.
                    tracing::warn!(%error, "robot: silero inference failed for a chunk");
                }
            }
        }
        if !ran {
            return VadDecision::Silence;
        }
        if max_p >= self.threshold {
            self.speech_started = true;
            self.trailing_silence_ms = 0;
            return VadDecision::Speech;
        }
        if !self.speech_started {
            return VadDecision::Silence;
        }
        self.trailing_silence_ms = self
            .trailing_silence_ms
            .saturating_add(frame_ms(pcm.len(), UPLINK_SAMPLE_RATE_HINT));
        if self.trailing_silence_ms > self.tuning.min_silence_ms {
            self.speech_started = false;
            self.trailing_silence_ms = 0;
            VadDecision::EndOfUtterance
        } else {
            VadDecision::Silence
        }
    }

    fn reset(&mut self) {
        self.state.iter_mut().for_each(|v| *v = 0.0);
        self.context.iter_mut().for_each(|v| *v = 0.0);
        self.pending.clear();
        self.speech_started = false;
        self.trailing_silence_ms = 0;
    }
}

