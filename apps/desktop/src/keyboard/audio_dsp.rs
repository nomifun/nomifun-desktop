//! Stateful mono conversion for native OS input. A short windowed-sinc filter
//! removes frequencies above the 16 kHz destination's Nyquist limit. Source
//! phase and history survive callback boundaries; flush preserves the tail.
use std::collections::VecDeque;

const OUTPUT_RATE: u64 = 16_000;
const HALF_TAPS: i64 = 16;

pub(super) struct PcmConverter {
    rate: u32,
    channels: usize,
    source: VecDeque<f32>,
    base: u64,
    received: u64,
    emitted: u64,
    pending: Vec<i16>,
    closed: bool,
}
impl PcmConverter {
    pub fn new(rate: u32, channels: usize) -> Result<Self, String> {
        if !(8_000..=192_000).contains(&rate) || !(1..=32).contains(&channels) {
            return Err("麦克风采样率或通道数不受支持".into());
        }
        Ok(Self {
            rate,
            channels,
            source: VecDeque::new(),
            base: 0,
            received: 0,
            emitted: 0,
            pending: Vec::with_capacity(1920),
            closed: false,
        })
    }
    pub fn push(&mut self, input: &[f32]) -> Result<Vec<Vec<i16>>, String> {
        if self.closed {
            return Err("麦克风采集已经关闭".into());
        }
        if input.len() % self.channels != 0 || input.len() > self.rate as usize * self.channels {
            return Err("麦克风音频块格式或时长无效".into());
        }
        for frame in input.chunks_exact(self.channels) {
            if frame.iter().any(|sample| !sample.is_finite()) {
                return Err("麦克风返回无效采样值".into());
            }
            let mono =
                frame.iter().map(|sample| f64::from(*sample)).sum::<f64>() / self.channels as f64;
            self.source.push_back(mono.clamp(-1.0, 1.0) as f32);
            self.received += 1;
        }
        self.render(false);
        Ok(self.take_chunks(false))
    }
    pub fn finish(&mut self, flush: bool) -> Result<Vec<Vec<i16>>, String> {
        if self.closed {
            return Ok(Vec::new());
        }
        self.closed = true;
        if flush {
            self.render(true);
        } else {
            self.pending.clear();
        }
        let chunks = self.take_chunks(flush);
        self.source.clear();
        Ok(chunks)
    }
    fn render(&mut self, final_: bool) {
        let total = self.received * OUTPUT_RATE / u64::from(self.rate);
        while self.emitted < total {
            let numerator = self.emitted * u64::from(self.rate);
            let left = numerator / OUTPUT_RATE;
            if !final_ && self.rate != 16_000 && left + HALF_TAPS as u64 >= self.received {
                break;
            }
            let sample = if self.rate == 16_000 {
                self.source[(left - self.base) as usize] as f64
            } else {
                let position = numerator as f64 / OUTPUT_RATE as f64;
                let cutoff = (OUTPUT_RATE as f64 / self.rate as f64).min(1.0) * 0.46;
                let mut value = 0.0;
                let mut weight = 0.0;
                for index in left as i64 - HALF_TAPS + 1..=left as i64 + HALF_TAPS {
                    let distance = index as f64 - position;
                    let x = 2.0 * cutoff * distance;
                    let sinc = if x.abs() < 1e-12 {
                        1.0
                    } else {
                        (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                    };
                    let window = (0.5
                        + 0.5 * (std::f64::consts::PI * distance / HALF_TAPS as f64).cos())
                    .max(0.0);
                    let tap = 2.0 * cutoff * sinc * window;
                    weight += tap;
                    if index >= 0 && (index as u64) >= self.base && (index as u64) < self.received {
                        value += f64::from(self.source[(index as u64 - self.base) as usize]) * tap;
                    }
                }
                if weight.abs() > 1e-12 {
                    value / weight
                } else {
                    0.0
                }
            };
            self.pending.push(
                (sample * 32768.0)
                    .round()
                    .clamp(i16::MIN as f64, i16::MAX as f64) as i16,
            );
            self.emitted += 1;
        }
        let retain_from =
            (self.emitted * u64::from(self.rate) / OUTPUT_RATE).saturating_sub(HALF_TAPS as u64);
        while self.base < retain_from && !self.source.is_empty() {
            self.source.pop_front();
            self.base += 1;
        }
    }
    fn take_chunks(&mut self, tail: bool) -> Vec<Vec<i16>> {
        let mut result = Vec::new();
        while self.pending.len() >= 960 {
            result.push(self.pending.drain(..960).collect());
        }
        if tail && !self.pending.is_empty() {
            result.push(std::mem::take(&mut self.pending));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn converted(rate: u32, channels: usize, chunk: usize) -> Vec<i16> {
        let input: Vec<f32> = (0..rate / 10)
            .flat_map(|index| {
                std::iter::repeat_n(
                    (index as f64 * std::f64::consts::TAU * 440.0 / rate as f64).sin() as f32 * 0.5,
                    channels,
                )
            })
            .collect();
        let mut converter = PcmConverter::new(rate, channels).unwrap();
        let mut out = Vec::new();
        for block in input.chunks(chunk * channels) {
            out.extend(converter.push(block).unwrap().into_iter().flatten());
        }
        out.extend(converter.finish(true).unwrap().into_iter().flatten());
        out
    }
    #[test]
    fn sample_counts_and_phase_survive_arbitrary_native_callback_boundaries() {
        for rate in [8_000, 16_000, 24_000, 44_100, 48_000, 96_000] {
            let all = converted(rate, 2, rate as usize / 10);
            let split = converted(rate, 2, 37);
            assert_eq!(all.len(), 1600, "{rate}");
            assert_eq!(all, split, "{rate}");
            assert!(all.iter().any(|v| v.abs() > 8000));
        }
    }
    #[test]
    fn stereo_downmix_and_partial_tail_are_exact_at_16k() {
        let mut converter = PcmConverter::new(16_000, 2).unwrap();
        assert!(
            converter
                .push(&[0.5, -0.5, 1.0, 0.0, -1.0, 0.0])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            converter.finish(true).unwrap(),
            vec![vec![0, 16384, -16384]]
        );
        assert!(converter.finish(true).unwrap().is_empty());
    }
    #[test]
    fn cancellation_discards_tail_and_rejects_late_callbacks() {
        let mut converter = PcmConverter::new(48_000, 1).unwrap();
        assert!(converter.push(&[0.5; 111]).unwrap().is_empty());
        assert!(converter.finish(false).unwrap().is_empty());
        assert!(converter.push(&[0.5; 111]).is_err());
    }
    #[test]
    fn downsampling_attenuates_out_of_band_audio() {
        let mut converter = PcmConverter::new(48_000, 1).unwrap();
        let input: Vec<f32> = (0..4800)
            .map(|n| (n as f64 * std::f64::consts::TAU * 12_000.0 / 48_000.0).sin() as f32)
            .collect();
        let mut output: Vec<i16> = converter
            .push(&input)
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        output.extend(converter.finish(true).unwrap().into_iter().flatten());
        let rms = (output[40..1560]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            / 1520.0)
            .sqrt();
        assert!(rms < 500.0, "out-of-band RMS {rms}");
    }
}
