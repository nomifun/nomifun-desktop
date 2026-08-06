//! Linear resampling for mono speech.
//!
//! The happy path never calls this: TTS is requested as 24 kHz PCM, exactly the
//! rate we declared to the device. It only runs when a provider returns a
//! container at some other rate. Linear interpolation is adequate for mono
//! speech; this signature is the seam where a polyphase resampler (rubato) would
//! drop in if quality ever proves insufficient.

/// Resample mono PCM from `from` Hz to `to` Hz.
pub fn resample_linear(pcm: &[i16], from: u32, to: u32) -> Vec<i16> {
    if pcm.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return pcm.to_vec();
    }
    let out_len = ((pcm.len() as u64 * to as u64) / from as u64).max(1) as usize;
    let ratio = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let left = src.floor() as usize;
            let frac = src - left as f64;
            let a = pcm[left.min(pcm.len() - 1)] as f64;
            let b = pcm[(left + 1).min(pcm.len() - 1)] as f64;
            (a + (b - a) * frac)
                .round()
                .clamp(i16::MIN as f64, i16::MAX as f64) as i16
        })
        .collect()
}
