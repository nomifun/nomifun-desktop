//! Endpointing. The device's own VAD only drives its LED and never ends a turn,
//! so `mode=auto` sessions end **only** when this decides they did.

pub mod energy;
pub mod silero;

pub use energy::EnergyVad;

/// The engine a companion profile names by default. Mirrors
/// `nomifun_companion::profile`'s own default so the two cannot drift into
/// disagreeing about what "unset" means.
pub const DEFAULT_VAD_ENGINE: &str = "silero";

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

/// Build the engine a companion asked for. Silero is preferred; if its model or
/// the ONNX runtime is unavailable this degrades to [`EnergyVad`] with a warning
/// rather than breaking the voice link. Any name other than `"silero"` — including
/// one this build does not know — resolves to the energy engine, so a profile
/// written by a newer build still talks.
pub fn build_engine(engine: &str, tuning: VadTuning) -> Box<dyn VadEngine> {
    if engine == DEFAULT_VAD_ENGINE {
        match silero::SileroVad::new(tuning) {
            Ok(vad) => return Box::new(vad),
            Err(error) => {
                tracing::warn!(%error, "robot: silero VAD unavailable, falling back to energy VAD");
            }
        }
    }
    Box::new(EnergyVad::new(tuning))
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

    #[test]
    fn build_engine_prefers_silero_and_never_fails() {
        let engine = build_engine("silero", VadTuning::default());
        assert!(
            engine.name() == "silero" || engine.name() == "energy",
            "silero is preferred but a load failure must degrade, not panic"
        );
    }

    #[test]
    fn build_engine_honours_an_explicit_energy_choice() {
        assert_eq!(build_engine("energy", VadTuning::default()).name(), "energy");
        assert_eq!(
            build_engine("anything-else", VadTuning::default()).name(),
            "energy"
        );
    }

    #[test]
    fn silero_ends_an_utterance_on_real_speech_then_silence() {
        let Ok(mut vad) = crate::vad::silero::SileroVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        }) else {
            eprintln!("skipping: ONNX runtime unavailable in this environment");
            return;
        };
        assert_eq!(vad.name(), "silero");

        // Silero wants 512-sample chunks at 16 kHz; the engine buffers whatever
        // frame size we hand it, so feed it our real 60 ms frames.
        let mut saw_speech = false;
        for frame in 0..10 {
            let speech = speech_like_frame(frame * crate::audio::UPLINK_FRAME_SAMPLES);
            if vad.push_frame(&speech) == VadDecision::Speech {
                saw_speech = true;
            }
        }
        assert!(saw_speech, "a speech-shaped signal must register as speech");

        let quiet = vec![0i16; crate::audio::UPLINK_FRAME_SAMPLES];
        let mut ended = false;
        for _ in 0..30 {
            if vad.push_frame(&quiet) == VadDecision::EndOfUtterance {
                ended = true;
                break;
            }
        }
        assert!(ended, "trailing silence must end the utterance");
    }

    /// A frame Silero accepts as speech: a glottal pulse train (a harmonic stack
    /// on a vibrato-modulated fundamental) shaped by two sweeping formants and an
    /// amplitude envelope. Static tone mixes read as ~1% speech probability — the
    /// model keys on the *motion* of the formants, not just their presence.
    fn speech_like_frame(offset: usize) -> Vec<i16> {
        use crate::audio::UPLINK_FRAME_SAMPLES;
        let tau = std::f32::consts::TAU;
        (offset..offset + UPLINK_FRAME_SAMPLES)
            .map(|i| {
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
            .collect()
    }
}
