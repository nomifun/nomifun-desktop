//! Endpointing. The device's own VAD only drives its LED and never ends a turn,
//! so `mode=auto` sessions end **only** when this decides they did.

pub mod energy;

pub use energy::EnergyVad;

/// Tunables exposed per companion (`voice.vad` in the companion profile).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadTuning {
    /// 0.0 (permissive: almost everything is speech) … 1.0 (strict).
    pub sensitivity: f32,
    /// Trailing silence, in milliseconds, that ends an utterance.
    pub min_silence_ms: u32,
}

impl Default for VadTuning {
    fn default() -> Self {
        Self {
            sensitivity: 0.5,
            min_silence_ms: 700,
        }
    }
}

impl VadTuning {
    /// Build from companion profile values, clamping to sane ranges.
    /// `engine` is accepted (and ignored) here so callers can pass the profile
    /// field verbatim; engine selection happens in the pipeline.
    pub fn from_profile(engine: &str, sensitivity: f32, min_silence_ms: u32) -> Self {
        let _ = engine;
        Self {
            sensitivity: sensitivity.clamp(0.0, 1.0),
            min_silence_ms: min_silence_ms.clamp(200, 3000),
        }
    }
}

/// What one frame told us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadDecision {
    /// No speech yet (or still trailing silence, but not long enough).
    Silence,
    /// Speech in progress.
    Speech,
    /// Speech had started and trailing silence has now passed the threshold.
    EndOfUtterance,
}

/// A frame-at-a-time endpointer.
pub trait VadEngine: Send {
    /// Stable name for logs and the UI.
    fn name(&self) -> &'static str;
    /// Feed one frame of 16 kHz mono PCM.
    fn push_frame(&mut self, pcm: &[i16]) -> VadDecision;
    /// Forget all state (called between turns).
    fn reset(&mut self);
}

/// Duration of a frame, in whole milliseconds.
pub fn frame_ms(samples: usize, sample_rate: u32) -> u32 {
    if sample_rate == 0 {
        return 0;
    }
    (samples as u64 * 1000 / sample_rate as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::UPLINK_FRAME_SAMPLES;

    fn loud() -> Vec<i16> {
        (0..UPLINK_FRAME_SAMPLES)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect()
    }

    fn quiet() -> Vec<i16> {
        vec![0i16; UPLINK_FRAME_SAMPLES]
    }

    #[test]
    fn frame_ms_matches_the_60ms_contract() {
        assert_eq!(frame_ms(UPLINK_FRAME_SAMPLES, 16_000), 60);
        assert_eq!(frame_ms(0, 16_000), 0);
        assert_eq!(frame_ms(960, 0), 0);
    }

    #[test]
    fn tuning_clamps_out_of_range_profile_values() {
        let t = VadTuning::from_profile("silero", 5.0, 10);
        assert_eq!(t.sensitivity, 1.0);
        assert_eq!(t.min_silence_ms, 200);
        let t = VadTuning::from_profile("silero", -1.0, 99_999);
        assert_eq!(t.sensitivity, 0.0);
        assert_eq!(t.min_silence_ms, 3000);
        assert_eq!(VadTuning::default().min_silence_ms, 700);
    }

    #[test]
    fn energy_vad_ends_an_utterance_after_the_silence_window() {
        // 700 ms of silence at 60 ms per frame is 12 frames.
        let mut vad = EnergyVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        });
        assert_eq!(vad.name(), "energy");

        // Leading silence never ends a turn that never started.
        for _ in 0..20 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        // Speech.
        for _ in 0..5 {
            assert_eq!(vad.push_frame(&loud()), VadDecision::Speech);
        }
        // Trailing silence: eleven frames is not yet 700 ms.
        for i in 0..11 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence, "frame {i}");
        }
        assert_eq!(vad.push_frame(&quiet()), VadDecision::EndOfUtterance);
    }

    #[test]
    fn energy_vad_resets_between_turns() {
        let mut vad = EnergyVad::new(VadTuning::default());
        for _ in 0..3 {
            vad.push_frame(&loud());
        }
        vad.reset();
        for _ in 0..30 {
            assert_eq!(
                vad.push_frame(&quiet()),
                VadDecision::Silence,
                "reset forgets the speech"
            );
        }
    }

    #[test]
    fn brief_gaps_inside_speech_do_not_end_the_turn() {
        let mut vad = EnergyVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        });
        for _ in 0..4 {
            vad.push_frame(&loud());
        }
        for _ in 0..6 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        assert_eq!(
            vad.push_frame(&loud()),
            VadDecision::Speech,
            "speech resumes"
        );
        for _ in 0..11 {
            assert_eq!(vad.push_frame(&quiet()), VadDecision::Silence);
        }
        assert_eq!(
            vad.push_frame(&quiet()),
            VadDecision::EndOfUtterance,
            "the counter restarted"
        );
    }
}
