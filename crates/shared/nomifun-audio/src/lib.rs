//! Explicit-format PCM operations shared by media endpoints and adapters.
use nomifun_voice_contracts::voice::AudioFormat;
pub fn resample_linear(pcm: &[i16], from: u32, to: u32) -> Vec<i16> {
    if pcm.is_empty() || from == 0 || to == 0 { return Vec::new(); }
    if from == to { return pcm.to_vec(); }
    let out_len = ((pcm.len() as u64 * u64::from(to)) / u64::from(from)).max(1) as usize;
    let ratio = f64::from(from) / f64::from(to);
    (0..out_len).map(|i| {
        let source = i as f64 * ratio; let left = source.floor() as usize; let frac = source - left as f64;
        let a = f64::from(pcm[left.min(pcm.len() - 1)]); let b = f64::from(pcm[(left + 1).min(pcm.len() - 1)]);
        (a + (b - a) * frac).round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
    }).collect()
}
pub fn pcm_to_wav(pcm: &[i16], sample_rate: u32) -> Vec<u8> { pcm16_to_wav(pcm, sample_rate, 1).expect("valid mono PCM") }
pub fn pcm16_to_wav(pcm: &[i16], sample_rate: u32, channels: u16) -> Result<Vec<u8>, String> {
    AudioFormat::pcm16(sample_rate, channels).validate()?;
    if !pcm.len().is_multiple_of(usize::from(channels)) { return Err("incomplete PCM sample frame".into()); }
    let data_len = u32::try_from(pcm.len().checked_mul(2).ok_or("PCM size overflow")?).map_err(|_| "PCM too large")?;
    let riff_len = data_len.checked_add(36).ok_or("WAV size overflow")?;
    let align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF"); out.extend_from_slice(&riff_len.to_le_bytes()); out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes()); out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes()); out.extend_from_slice(&(sample_rate * u32::from(align)).to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&16u16.to_le_bytes()); out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes()); for sample in pcm { out.extend_from_slice(&sample.to_le_bytes()); }
    Ok(out)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn stereo_48k_and_mono_16k_keep_duration() {
        let stereo = vec![1000i16; 4800 * 2];
        let mono = vec![1000i16; 4800];
        let converted = resample_linear(&mono, 48_000, 16_000);
        assert_eq!(converted.len(), 1600); assert!(converted.iter().all(|s| *s == 1000));
        let wav = pcm16_to_wav(&stereo, 48_000, 2).unwrap();
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 192_000);
    }
}
