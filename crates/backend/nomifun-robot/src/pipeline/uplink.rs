//! Microphone → transcript-ready WAV.
//!
//! The firmware never tells us a turn is over in `auto`/`realtime` mode — its own
//! VAD only drives an LED — so endpointing happens here. In `manual` mode the
//! device sends `listen stop` and we just hand over what we buffered.
//!
//! A hard ceiling guards against a stuck-open microphone: without it a `manual`
//! session whose `listen stop` never arrives would grow this buffer forever.

use crate::audio::{OpusStreamDecoder, pcm_to_wav};
use crate::protocol::{ListeningMode, UPLINK_SAMPLE_RATE};
use crate::vad::{VadDecision, VadEngine};

/// Longest single utterance we will buffer, in milliseconds.
pub const MAX_UTTERANCE_MS: u32 = 60_000;

// A continuous microphone may be quiet indefinitely. Retain a small lead-in
// for VAD onset, without turning idle silence into a new ASR request at 60 s.
const LEADING_SILENCE_MS: u32 = 300;

fn max_samples() -> usize {
    (UPLINK_SAMPLE_RATE as u64 * MAX_UTTERANCE_MS as u64 / 1000) as usize
}

/// What a packet did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UplinkOutcome {
    /// Keep listening.
    Continue,
    /// The utterance ended; here is the WAV to transcribe.
    Utterance(Vec<u8>),
}

/// Decodes uplink Opus, buffers PCM, and decides when the user stopped talking.
pub struct UplinkPipeline {
    decoder: OpusStreamDecoder,
    vad: Box<dyn VadEngine>,
    pcm: Vec<i16>,
    mode: ListeningMode,
    active: bool,
    speech_started: bool,
    decoded_frame: Option<Vec<i16>>,
}

impl UplinkPipeline {
    /// Build with the companion's chosen endpointer.
    pub fn new(vad: Box<dyn VadEngine>) -> anyhow::Result<Self> {
        Ok(Self {
            decoder: OpusStreamDecoder::new_uplink()?,
            vad,
            pcm: Vec::new(),
            mode: ListeningMode::Auto,
            active: false,
            speech_started: false,
            decoded_frame: None,
        })
    }

    /// Open a listening window (device sent `listen start`).
    pub fn begin(&mut self, mode: ListeningMode) {
        self.mode = mode;
        self.active = true;
        self.pcm.clear();
        self.speech_started = false;
        self.decoded_frame = None;
        self.vad.reset();
    }

    /// Whether a listening window is open.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The current continuous utterance has crossed its VAD speech onset.
    pub(crate) fn is_speech_started(&self) -> bool {
        self.speech_started
    }

    /// A single decoded packet, drained by the connection's bounded ASR tap.
    /// This is captured during input, before a complete WAV exists.
    pub(crate) fn take_decoded_frame(&mut self) -> Option<Vec<i16>> {
        self.decoded_frame.take()
    }

    /// Continuous-listening onset can include the retained VAD lead-in once.
    pub(crate) fn buffered_pcm(&self) -> &[i16] { &self.pcm }

    /// Feed one uplink Opus packet.
    pub fn push_packet(&mut self, packet: &[u8]) -> UplinkOutcome {
        self.decoded_frame = None;
        if !self.active {
            // Wake-word audio arrives before `listen start`; it is not part of a
            // turn and must not be transcribed as one.
            return UplinkOutcome::Continue;
        }
        let frame = match self.decoder.decode(packet) {
            Ok(pcm) => pcm,
            Err(error) => {
                tracing::warn!(%error, "robot: dropping undecodable uplink packet");
                return UplinkOutcome::Continue;
            }
        };
        self.decoded_frame = Some(frame.clone());
        self.pcm.extend_from_slice(&frame);

        // The listen window and an utterance have different lifetimes. In
        // auto/realtime the device keeps sending after a VAD endpoint, even
        // when the Agent finishes without speech or TTS never restarts it.
        let decision = if matches!(self.mode, ListeningMode::Manual) {
            None
        } else {
            let decision = self.vad.push_frame(&frame);
            self.speech_started |= matches!(decision, VadDecision::Speech);
            if !self.speech_started {
                let retained = (UPLINK_SAMPLE_RATE as u64 * LEADING_SILENCE_MS as u64 / 1000) as usize;
                if self.pcm.len() > retained {
                    self.pcm.drain(..self.pcm.len() - retained);
                }
                return UplinkOutcome::Continue;
            }
            Some(decision)
        };

        if self.pcm.len() >= max_samples() {
            tracing::warn!(
                seconds = MAX_UTTERANCE_MS / 1000,
                "robot: utterance hit the ceiling, forcing transcription"
            );
            if matches!(self.mode, ListeningMode::Manual) {
                self.active = false;
            }
            return UplinkOutcome::Utterance(self.take_wav());
        }

        // Manual mode is ended by the device, never by us.
        if matches!(self.mode, ListeningMode::Manual) {
            return UplinkOutcome::Continue;
        }

        match decision {
            Some(VadDecision::EndOfUtterance) => UplinkOutcome::Utterance(self.take_wav()),
            _ => UplinkOutcome::Continue,
        }
    }

    /// Close the window and emit whatever was buffered (device sent `listen
    /// stop`). Returns `None` for empty or continuous silence-only audio.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        self.active = false;
        if self.pcm.is_empty() || (!matches!(self.mode, ListeningMode::Manual) && !self.speech_started) {
            self.pcm.clear();
            self.speech_started = false;
            self.vad.reset();
            return None;
        }
        Some(self.take_wav())
    }

    /// Throw the buffer away (device sent `abort`).
    pub fn abort(&mut self) {
        self.active = false;
        self.pcm.clear();
        self.speech_started = false;
        self.decoded_frame = None;
        self.vad.reset();
    }

    fn take_wav(&mut self) -> Vec<u8> {
        let pcm = std::mem::take(&mut self.pcm);
        self.speech_started = false;
        self.vad.reset();
        pcm_to_wav(&pcm, UPLINK_SAMPLE_RATE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{OpusStreamEncoder, UPLINK_FRAME_SAMPLES};
    use crate::vad::{EnergyVad, VadTuning};

    /// Encode `ms` of loud audio into 60 ms Opus packets, as the device would.
    fn loud_packets(ms: u32) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        let pcm: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                ((t * 300.0 * std::f32::consts::TAU).sin() * 9000.0) as i16
            })
            .collect();
        OpusStreamEncoder::new_uplink_for_test()
            .unwrap()
            .encode_frames(&pcm)
            .unwrap()
    }

    fn quiet_packets(ms: u32) -> Vec<Vec<u8>> {
        let n = (16_000u64 * ms as u64 / 1000) as usize;
        OpusStreamEncoder::new_uplink_for_test()
            .unwrap()
            .encode_frames(&vec![0i16; n])
            .unwrap()
    }

    fn pipeline() -> UplinkPipeline {
        UplinkPipeline::new(Box::new(EnergyVad::new(VadTuning {
            sensitivity: 0.5,
            min_silence_ms: 700,
        })))
        .unwrap()
    }

    fn wav_sample_count(wav: &[u8]) -> usize {
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        data_len / 2
    }

    #[test]
    fn auto_mode_ends_the_utterance_on_trailing_silence() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Auto);
        assert!(p.is_active());

        for packet in loud_packets(300) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        let mut wav = None;
        for packet in quiet_packets(900) {
            if let UplinkOutcome::Utterance(bytes) = p.push_packet(&packet) {
                wav = Some(bytes);
                break;
            }
        }
        let wav = wav.expect("700 ms of silence must end the turn");
        assert_eq!(&wav[0..4], b"RIFF");
        assert!(
            wav_sample_count(&wav) >= 16_000 * 300 / 1000,
            "the speech is all in there"
        );
        assert!(p.is_active(), "a VAD endpoint does not end continuous listening");
    }

    #[test]
    fn manual_mode_never_ends_on_its_own_and_finish_emits() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        for packet in loud_packets(180) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        for packet in quiet_packets(2000) {
            assert!(
                matches!(p.push_packet(&packet), UplinkOutcome::Continue),
                "manual mode waits for `listen stop`, however long the pause"
            );
        }
        let wav = p.finish().expect("finish emits what was buffered");
        assert!(wav_sample_count(&wav) > 0);
        assert!(p.finish().is_none(), "finish drains");
    }

    #[test]
    fn packets_before_begin_are_dropped() {
        let mut p = pipeline();
        for packet in loud_packets(120) {
            assert!(matches!(p.push_packet(&packet), UplinkOutcome::Continue));
        }
        assert!(
            p.finish().is_none(),
            "audio outside a listen window is not an utterance"
        );
    }

    #[test]
    fn incremental_pcm_tap_is_available_before_a_complete_utterance() {
        let mut pipeline=pipeline();
        let packet=loud_packets(60).pop().unwrap();
        pipeline.push_packet(&packet);
        assert!(pipeline.take_decoded_frame().is_none(),"unadmitted wake-word audio has no ASR tap");
        pipeline.begin(ListeningMode::Manual);
        assert_eq!(pipeline.push_packet(&packet),UplinkOutcome::Continue);
        assert_eq!(pipeline.take_decoded_frame().unwrap().len(),UPLINK_FRAME_SAMPLES);
        assert!(pipeline.take_decoded_frame().is_none(),"a frame is only admitted once");
        assert!(pipeline.finish().is_some(),"the complete WAV did not exist when the PCM tap was consumed");
        pipeline.abort();
        assert!(pipeline.take_decoded_frame().is_none());
    }

    #[test]
    fn abort_discards_the_buffer_and_closes() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Auto);
        for packet in loud_packets(180) {
            p.push_packet(&packet);
        }
        p.abort();
        assert!(!p.is_active());
        assert!(p.finish().is_none());
    }

    #[test]
    fn hitting_the_ceiling_force_emits_instead_of_growing_forever() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        let mut emitted = None;
        // Feed well past the 60 s ceiling; manual mode would otherwise never end.
        'outer: for _ in 0..70 {
            for packet in loud_packets(1000) {
                if let UplinkOutcome::Utterance(wav) = p.push_packet(&packet) {
                    emitted = Some(wav);
                    break 'outer;
                }
            }
        }
        let wav = emitted.expect("the ceiling must force an emit");
        assert!(!p.is_active(), "the manual recording ceiling still closes its window");
        let seconds = wav_sample_count(&wav) as f64 / 16_000.0;
        assert!(
            (59.0..=62.0).contains(&seconds),
            "capped near the ceiling, got {seconds:.1}s"
        );
    }

    #[test]
    fn a_second_utterance_works_after_the_first() {
        for mode in [ListeningMode::Auto, ListeningMode::Realtime] {
            let mut p = pipeline();
            p.begin(mode);
            for round in 0..3 {
                for packet in loud_packets(200) {
                    p.push_packet(&packet);
                }
                let mut got = false;
                for packet in quiet_packets(900) {
                    if matches!(p.push_packet(&packet), UplinkOutcome::Utterance(_)) {
                        got = true;
                        break;
                    }
                }
                assert!(got, "round {round} must produce an utterance (VAD state was reset)");
                assert!(p.is_active());
            }
        }
    }

    #[test]
    fn continuous_speech_ceiling_bounds_each_chunk_without_closing_listening() {
        for mode in [ListeningMode::Auto, ListeningMode::Realtime] {
            let mut p = pipeline(); p.begin(mode);
            let mut emitted = 0;
            for _ in 0..65 {
                for packet in loud_packets(1000) {
                    if let UplinkOutcome::Utterance(wav) = p.push_packet(&packet) {
                        emitted += 1;
                        assert!(wav_sample_count(&wav) <= max_samples() + UPLINK_FRAME_SAMPLES);
                    }
                    assert!(p.pcm.len() < max_samples());
                }
            }
            assert_eq!(emitted, 1);
            assert!(p.is_active());
            assert!(p.finish().is_some(), "audio following the cap belongs to a fresh utterance");
            assert!(!p.is_active());
        }
    }

    #[test]
    fn continuous_silence_stays_bounded_without_transcription_and_keeps_the_next_speech() {
        for mode in [ListeningMode::Auto, ListeningMode::Realtime] {
            let mut p = pipeline();
            p.begin(mode);
            for _ in 0..70 {
                for packet in quiet_packets(1000) {
                    assert_eq!(p.push_packet(&packet), UplinkOutcome::Continue);
                    assert!(p.pcm.len() <= (UPLINK_SAMPLE_RATE * LEADING_SILENCE_MS / 1000) as usize);
                }
            }
            assert!(p.is_active());
            for packet in loud_packets(300) { p.push_packet(&packet); }
            let wav = quiet_packets(900).into_iter().find_map(|packet| match p.push_packet(&packet) {
                UplinkOutcome::Utterance(wav) => Some(wav), _ => None,
            }).expect("speech after long idle must still be transcribed");
            assert!(wav_sample_count(&wav) >= (UPLINK_SAMPLE_RATE * 300 / 1000) as usize);
            assert!(wav_sample_count(&wav) < (UPLINK_SAMPLE_RATE * 2000 / 1000) as usize);
            assert!(p.is_active());
        }
    }

    #[test]
    fn explicit_stop_and_abort_close_continuous_listening_without_a_late_reopen() {
        for mode in [ListeningMode::Auto, ListeningMode::Realtime] {
            for abort in [false, true] {
                let mut p = pipeline(); p.begin(mode);
                for packet in loud_packets(180) { p.push_packet(&packet); }
                if abort { p.abort(); } else { assert!(p.finish().is_some()); }
                for packet in loud_packets(180).into_iter().chain(quiet_packets(900)) {
                    assert_eq!(p.push_packet(&packet), UplinkOutcome::Continue);
                }
                assert!(!p.is_active());
                assert!(p.finish().is_none());
            }
            let mut p = pipeline(); p.begin(mode);
            for packet in quiet_packets(900) { p.push_packet(&packet); }
            assert!(p.finish().is_none(), "explicitly stopping silence is not an ASR utterance");
            assert!(!p.is_active());
        }
    }

    #[test]
    fn a_corrupt_packet_is_skipped_without_killing_the_turn() {
        let mut p = pipeline();
        p.begin(crate::protocol::ListeningMode::Manual);
        p.push_packet(&[0xff, 0xff, 0xff, 0xff]);
        for packet in loud_packets(120) {
            p.push_packet(&packet);
        }
        assert!(
            p.finish().is_some(),
            "the good audio still made it through"
        );
        assert_eq!(UPLINK_FRAME_SAMPLES, 960);
    }
}
