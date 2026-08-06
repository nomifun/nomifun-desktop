//! Audio primitives: Opus codec wrappers, WAV packing, resampling, container
//! decode. Everything here is synchronous and allocation-explicit so the
//! pipelines can be tested without a device or a provider.

pub mod container;
pub mod opus;
pub mod resample;
pub mod wav;

pub use container::decode_container;
pub use opus::{DOWNLINK_FRAME_SAMPLES, OpusStreamDecoder, OpusStreamEncoder, UPLINK_FRAME_SAMPLES};
pub use resample::resample_linear;
pub use wav::pcm_to_wav;

/// Mono PCM with its sample rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBuffer {
    pub pcm: Vec<i16>,
    pub sample_rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 440 Hz mono tone, `ms` milliseconds at `rate`.
    pub(crate) fn tone(rate: u32, ms: u32) -> Vec<i16> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
            })
            .collect()
    }

    #[test]
    fn opus_round_trip_preserves_length_and_energy() {
        let pcm = tone(16_000, 180); // three 60 ms frames
        let mut encoder = OpusStreamEncoder::new_uplink_for_test().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(
            frames.len(),
            3,
            "180 ms of 16 kHz audio is three 60 ms frames"
        );
        assert!(frames.iter().all(|f| !f.is_empty()));

        let mut decoder = OpusStreamDecoder::new_uplink().unwrap();
        let mut decoded = Vec::new();
        for frame in &frames {
            decoded.extend(decoder.decode(frame).unwrap());
        }
        assert_eq!(
            decoded.len(),
            pcm.len(),
            "sample count survives the round trip"
        );

        let rms = |s: &[i16]| {
            (s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt()
        };
        let (before, after) = (rms(&pcm), rms(&decoded));
        assert!(
            (after / before - 1.0).abs() < 0.35,
            "lossy but recognisable: before={before:.0} after={after:.0}"
        );
    }

    #[test]
    fn downlink_encoder_emits_1440_sample_frames() {
        let pcm = tone(24_000, 120);
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(frames.len(), 2, "120 ms of 24 kHz audio is two 60 ms frames");
        assert_eq!(DOWNLINK_FRAME_SAMPLES, 1440);
        assert_eq!(UPLINK_FRAME_SAMPLES, 960);
    }

    #[test]
    fn trailing_partial_frame_is_zero_padded_not_dropped() {
        let pcm = tone(24_000, 90); // one full frame + half a frame
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let frames = encoder.encode_frames(&pcm).unwrap();
        assert_eq!(
            frames.len(),
            2,
            "the tail is padded so the last words are not cut off"
        );
    }

    #[test]
    fn wav_header_is_44_bytes_and_declares_the_rate() {
        let wav = pcm_to_wav(&[0, 1, -1, 32767], 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 4 * 2);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(
            u16::from_le_bytes(wav[22..24].try_into().unwrap()),
            1,
            "mono"
        );
        assert_eq!(
            u16::from_le_bytes(wav[34..36].try_into().unwrap()),
            16,
            "16-bit"
        );
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
    }

    #[test]
    fn resample_scales_length_and_is_identity_at_equal_rates() {
        let pcm = tone(16_000, 100);
        let up = resample_linear(&pcm, 16_000, 24_000);
        assert!((up.len() as i64 - 2400).abs() <= 1, "got {}", up.len());
        assert_eq!(
            resample_linear(&pcm, 16_000, 16_000),
            pcm,
            "same rate copies through"
        );
        assert!(resample_linear(&[], 16_000, 24_000).is_empty());
    }

    #[test]
    fn decode_container_reads_our_own_wav() {
        let pcm = tone(24_000, 60);
        let wav = pcm_to_wav(&pcm, 24_000);
        let buffer = decode_container(&wav, Some("audio/wav")).unwrap();
        assert_eq!(buffer.sample_rate, 24_000);
        assert_eq!(buffer.pcm.len(), pcm.len());
    }

    #[test]
    fn decode_container_rejects_garbage() {
        assert!(decode_container(b"not audio at all", None).is_err());
    }
}
