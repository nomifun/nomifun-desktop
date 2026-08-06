//! RMS energy endpointer. No model, no weights, no warm-up — the dependable
//! floor under the Silero engine and its fallback when ONNX is unavailable.

use crate::audio::UPLINK_SAMPLE_RATE_HINT;

use super::{VadDecision, VadEngine, VadTuning, frame_ms};

/// RMS below which a frame counts as silence, at `sensitivity = 0.0` and `1.0`.
/// Speech at a normal distance from an INMP441 lands well above 900; room noise
/// sits under 250.
const RMS_AT_MIN_SENSITIVITY: f32 = 200.0;
const RMS_AT_MAX_SENSITIVITY: f32 = 1200.0;

/// Energy-threshold VAD.
pub struct EnergyVad {
    tuning: VadTuning,
    threshold: f32,
    speech_started: bool,
    trailing_silence_ms: u32,
}

impl EnergyVad {
    pub fn new(tuning: VadTuning) -> Self {
        let threshold = RMS_AT_MIN_SENSITIVITY
            + (RMS_AT_MAX_SENSITIVITY - RMS_AT_MIN_SENSITIVITY) * tuning.sensitivity;
        Self {
            tuning,
            threshold,
            speech_started: false,
            trailing_silence_ms: 0,
        }
    }

    fn rms(pcm: &[i16]) -> f32 {
        if pcm.is_empty() {
            return 0.0;
        }
        let sum: f64 = pcm.iter().map(|s| (*s as f64) * (*s as f64)).sum();
        (sum / pcm.len() as f64).sqrt() as f32
    }
}

impl VadEngine for EnergyVad {
    fn name(&self) -> &'static str {
        "energy"
    }

    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision {
        let is_speech = Self::rms(pcm) >= self.threshold;
        if is_speech {
            self.speech_started = true;
            self.trailing_silence_ms = 0;
            return VadDecision::Speech;
        }
        if !self.speech_started {
            // Leading silence: nothing to end.
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
        self.speech_started = false;
        self.trailing_silence_ms = 0;
    }
}
