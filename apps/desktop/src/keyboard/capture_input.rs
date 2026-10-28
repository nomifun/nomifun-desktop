//! Native capture sinks share one ingress contract. Streaming uploads chunks;
//! ordinary ASR retains one bounded in-memory recording until native stop;
//! local microphone checks retain scalar measurements only.
use super::super::models::StreamingAsrInput;
use serde::Serialize;
use std::sync::{Arc, Mutex};

pub const MAX_BUFFERED_SAMPLES: usize = 16_000 * 600;
const MAX_CHUNK_SAMPLES: usize = 960;

#[derive(Clone)]
enum Sink {
    Streaming(StreamingAsrInput),
    Buffered(BufferedCapture),
    Meter(MeterCapture),
}
#[derive(Clone)]
pub struct CaptureInput {
    sink: Sink,
    stats: Arc<Mutex<AudioStats>>,
}
/// Aggregate levels only: this contains no waveform, text or credentials.
/// A zero peak/nonzero count proves zero digital level, never whether a person
/// spoke. Callers must not classify low RMS as silence or speech absence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStats {
    pub samples: u64,
    pub nonzero: u64,
    pub peak: u32,
    pub sum_squares: u64,
}
impl AudioStats {
    fn accept(&mut self, samples: &[i16]) {
        self.samples = self.samples.saturating_add(samples.len() as u64);
        for sample in samples {
            let sample = i64::from(*sample);
            self.nonzero = self.nonzero.saturating_add(u64::from(sample != 0));
            self.peak = self.peak.max(sample.unsigned_abs() as u32);
            self.sum_squares = self.sum_squares.saturating_add((sample * sample) as u64);
        }
    }
}
impl From<StreamingAsrInput> for CaptureInput {
    fn from(input: StreamingAsrInput) -> Self {
        Self::streaming(input)
    }
}
impl CaptureInput {
    pub fn streaming(input: StreamingAsrInput) -> Self {
        Self {
            sink: Sink::Streaming(input),
            stats: Arc::new(Mutex::new(AudioStats::default())),
        }
    }
    pub fn stats(&self) -> AudioStats {
        *self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub fn append_pcm16_16k(&self, samples: &[i16]) -> Result<(), String> {
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| "录音采样统计状态损坏".to_string())?;
        match &self.sink {
            Sink::Streaming(input) => input.append_pcm16_16k(samples)?,
            Sink::Buffered(input) => input.append(samples)?,
            Sink::Meter(input) => input.append(samples)?,
        };
        stats.accept(samples);
        Ok(())
    }
}

struct Meter {
    open: bool,
    recent: AudioStats,
}

/// A local-only sink with no waveform allocation or retention. Each accepted
/// chunk replaces the recent scalar window (at most 60 ms of 16 kHz input), so
/// displayed levels can fall while cumulative evidence remains available.
#[derive(Clone)]
pub struct MeterCapture {
    meter: Arc<Mutex<Meter>>,
    stats: Arc<Mutex<AudioStats>>,
}
impl Default for MeterCapture {
    fn default() -> Self {
        Self::new()
    }
}
impl MeterCapture {
    pub fn new() -> Self {
        Self {
            meter: Arc::new(Mutex::new(Meter {
                open: true,
                recent: AudioStats::default(),
            })),
            stats: Arc::new(Mutex::new(AudioStats::default())),
        }
    }
    pub fn input(&self) -> CaptureInput {
        CaptureInput {
            sink: Sink::Meter(self.clone()),
            stats: self.stats.clone(),
        }
    }
    fn append(&self, samples: &[i16]) -> Result<(), String> {
        let mut meter = self
            .meter
            .lock()
            .map_err(|_| "麦克风检测采样状态损坏".to_string())?;
        if !meter.open {
            return Err("麦克风检测已关闭，拒绝迟到采样".into());
        }
        if samples.is_empty() || samples.len() > MAX_CHUNK_SAMPLES {
            meter.open = false;
            return Err("麦克风检测需要每块1–960个16kHz单声道PCM16采样".into());
        }
        let mut recent = AudioStats::default();
        recent.accept(samples);
        meter.recent = recent;
        Ok(())
    }
    /// Fence all callback clones and wait for admitted cumulative measurements.
    /// This does not stop the native owner; callers must prove native stop
    /// separately before releasing its capture slot.
    pub fn close(&self) {
        // Ingress takes stats before the sink. Keep the same order so close
        // cannot return while an admitted append still has stats to publish.
        let _stats = self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.meter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .open = false;
    }
    pub fn stats(&self) -> AudioStats {
        *self
            .stats
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// Normalized peak and RMS from the most recent accepted chunk. Digital
    /// zero remains distinct from no callback through cumulative sample counts.
    pub fn levels(&self) -> (f64, f64) {
        let recent = self
            .meter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .recent;
        if recent.samples == 0 {
            return (0.0, 0.0);
        }
        let peak = f64::from(recent.peak) / 32768.0;
        let rms = (recent.sum_squares as f64 / recent.samples as f64).sqrt() / 32768.0;
        (peak, rms)
    }
}

enum State {
    Open(Vec<i16>),
    Finished,
    Cancelled,
    Failed(&'static str),
}
struct Buffer {
    state: State,
    limit: usize,
}

/// Create a new owner for each recording identity; never reuse it for a later
/// document. Its input clones share admission, so finish/cancel also fences
/// callback clones already held by the native driver. No PCM is written to disk.
#[derive(Clone)]
pub struct BufferedCapture {
    buffer: Arc<Mutex<Buffer>>,
    stats: Arc<Mutex<AudioStats>>,
}
impl Default for BufferedCapture {
    fn default() -> Self {
        Self::new()
    }
}
impl BufferedCapture {
    pub fn new() -> Self {
        Self::with_limit(MAX_BUFFERED_SAMPLES)
    }
    fn with_limit(limit: usize) -> Self {
        let mut samples = Vec::new();
        // Reserve the single bounded allocation once. Incremental geometric
        // growth could temporarily hold two large allocations during realloc.
        let state = match samples.try_reserve_exact(limit) {
            Ok(()) => State::Open(samples),
            Err(_) => State::Failed("无法为普通识别分配音频内存，录音已拒绝"),
        };
        Self {
            buffer: Arc::new(Mutex::new(Buffer { state, limit })),
            stats: Arc::new(Mutex::new(AudioStats::default())),
        }
    }
    pub fn input(&self) -> CaptureInput {
        CaptureInput {
            sink: Sink::Buffered(self.clone()),
            stats: self.stats.clone(),
        }
    }
    fn append(&self, samples: &[i16]) -> Result<(), String> {
        let mut buffer = self
            .buffer
            .lock()
            .map_err(|_| "普通识别音频状态损坏".to_string())?;
        let limit = buffer.limit;
        match &mut buffer.state {
            State::Open(saved) => {
                let failure = if samples.is_empty() || samples.len() > MAX_CHUNK_SAMPLES {
                    Some("普通识别需要每块1–960个16kHz单声道PCM16采样")
                } else if samples.len() > limit.saturating_sub(saved.len()) {
                    Some("普通识别录音超过10分钟上限，已停止采集")
                } else {
                    None
                };
                if let Some(error) = failure {
                    buffer.state = State::Failed(error); // drops the entire partial recording
                    return Err(error.into());
                }
                saved.extend_from_slice(samples);
                Ok(())
            }
            State::Finished => Err("普通识别音频已经取出，拒绝迟到采样".into()),
            State::Cancelled => Err("普通识别录音已取消，拒绝迟到采样".into()),
            State::Failed(error) => Err((*error).into()),
        }
    }
    /// Call only after the corresponding native owner positively stopped and
    /// drained its tail. Exactly one caller receives the existing allocation;
    /// an empty recording returns an empty Vec and must not invoke cloud ASR.
    pub fn finish(&self) -> Result<Vec<i16>, String> {
        let mut buffer = self
            .buffer
            .lock()
            .map_err(|_| "普通识别音频状态损坏".to_string())?;
        match &buffer.state {
            State::Open(_) => {}
            State::Finished => return Err("普通识别音频不能重复取出".into()),
            State::Cancelled => return Err("普通识别录音已取消".into()),
            State::Failed(error) => return Err((*error).into()),
        }
        let State::Open(samples) = std::mem::replace(&mut buffer.state, State::Finished) else {
            unreachable!()
        };
        Ok(samples)
    }
    pub fn cancel(&self) {
        let mut buffer = self
            .buffer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        buffer.state = State::Cancelled; // releases PCM even after a poisoned callback lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meter_only_tracks_cumulative_stats_across_input_clones() {
        let capture = MeterCapture::new();
        let input = capture.input();
        assert_eq!(capture.stats(), AudioStats::default());
        assert_eq!(capture.levels(), (0.0, 0.0));
        input.append_pcm16_16k(&[0, i16::MIN, i16::MAX]).unwrap();
        input.clone().append_pcm16_16k(&[0, 1, -1]).unwrap();
        let expected = AudioStats {
            samples: 6,
            nonzero: 4,
            peak: 32768,
            sum_squares: 2_147_418_115,
        };
        assert_eq!(capture.stats(), expected);
        assert_eq!(input.stats(), expected);
    }
    #[test]
    fn meter_only_levels_follow_recent_input_and_fall_after_digital_zero() {
        let capture = MeterCapture::new();
        let input = capture.input();
        input.append_pcm16_16k(&[i16::MIN, 0]).unwrap();
        let (peak, rms) = capture.levels();
        assert_eq!(peak, 1.0);
        assert!((rms - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        input.append_pcm16_16k(&[8192, -8192]).unwrap();
        assert_eq!(capture.levels(), (0.25, 0.25));
        input.append_pcm16_16k(&[0; 960]).unwrap();
        assert_eq!(capture.levels(), (0.0, 0.0));
        assert_eq!(capture.stats().samples, 964);
        assert_eq!(capture.stats().nonzero, 3);
        assert_eq!(capture.stats().peak, 32768);
    }
    #[test]
    fn meter_only_close_fences_existing_and_new_inputs_without_erasing_evidence() {
        let capture = MeterCapture::new();
        let input = capture.input();
        let callback_clone = input.clone();
        input.append_pcm16_16k(&[16384; 960]).unwrap();
        let expected = capture.stats();
        capture.clone().close();
        capture.close();
        assert!(callback_clone.append_pcm16_16k(&[1]).is_err());
        assert!(capture.input().append_pcm16_16k(&[1]).is_err());
        assert_eq!(capture.stats(), expected);
        assert_eq!(capture.levels(), (0.5, 0.5));
        let next = MeterCapture::new();
        next.input().append_pcm16_16k(&[1]).unwrap();
        assert_eq!(next.stats().samples, 1);
        assert_eq!(capture.stats(), expected);
    }
    #[test]
    fn meter_only_malformed_chunks_fail_closed_without_counting_rejected_input() {
        for chunk in [Vec::new(), vec![0; 961]] {
            let capture = MeterCapture::new();
            let input = capture.input();
            input.append_pcm16_16k(&[16384]).unwrap();
            let expected = capture.stats();
            assert!(input.append_pcm16_16k(&chunk).is_err());
            assert!(input.clone().append_pcm16_16k(&[2]).is_err());
            assert_eq!(capture.stats(), expected);
            assert_eq!(capture.levels(), (0.5, 0.5));
        }
    }
    #[test]
    fn meter_only_close_and_append_linearize_without_late_stats() {
        let capture = MeterCapture::new();
        let input = capture.input();
        let append = std::thread::spawn(move || input.append_pcm16_16k(&[8192; 960]));
        capture.close();
        let stopped_stats = capture.stats();
        let accepted = append.join().unwrap().is_ok();
        assert_eq!(capture.stats(), stopped_stats);
        assert_eq!(stopped_stats.samples, if accepted { 960 } else { 0 });
        assert!(capture.input().append_pcm16_16k(&[42]).is_err());
    }
    #[test]
    fn meter_counts_only_successful_pcm_and_is_shared_without_retaining_audio() {
        let capture = BufferedCapture::with_limit(6);
        let input = capture.input();
        input.append_pcm16_16k(&[0, i16::MIN, i16::MAX]).unwrap();
        input.clone().append_pcm16_16k(&[0, 1, -1]).unwrap();
        let expected = AudioStats {
            samples: 6,
            nonzero: 4,
            peak: 32768,
            sum_squares: 2_147_418_115,
        };
        assert_eq!(capture.input().stats(), expected);
        assert_eq!(input.stats(), expected);
        assert!(input.append_pcm16_16k(&[1]).is_err());
        assert_eq!(input.stats(), expected);
        capture.cancel();
        assert!(matches!(
            capture.buffer.lock().unwrap().state,
            State::Cancelled
        ));
        assert!(input.append_pcm16_16k(&[2]).is_err());
        assert_eq!(capture.input().stats(), expected);
    }
    #[test]
    fn no_samples_and_digital_zero_have_distinct_aggregate_evidence() {
        let capture = BufferedCapture::with_limit(960);
        assert_eq!(capture.input().stats(), AudioStats::default());
        capture.input().append_pcm16_16k(&[0; 320]).unwrap();
        assert_eq!(
            capture.input().stats(),
            AudioStats {
                samples: 320,
                ..Default::default()
            }
        );
        assert_eq!(capture.finish().unwrap(), vec![0; 320]);
    }
    #[test]
    fn native_resampler_tail_reaches_the_same_buffer_and_meter() {
        let capture = BufferedCapture::with_limit(960);
        let input = capture.input();
        let mut converter = super::super::dsp::PcmConverter::new(48000, 2).unwrap();
        assert!(converter.push(&[0.25; 220]).unwrap().is_empty());
        for tail in converter.finish(true).unwrap() {
            input.append_pcm16_16k(&tail).unwrap();
        }
        let samples = capture.finish().unwrap();
        assert_eq!(samples.len(), 36);
        assert_eq!(input.stats().samples, 36);
        assert!(input.stats().nonzero > 0);
        assert!(input.append_pcm16_16k(&[1]).is_err());
    }
    #[test]
    fn ordered_chunks_and_tail_are_taken_once_without_copying() {
        let capture = BufferedCapture::with_limit(1920);
        let input = capture.input();
        input.append_pcm16_16k(&[1; 960]).unwrap();
        input.clone().append_pcm16_16k(&[2, 3, 4]).unwrap();
        let pointer = match &capture.buffer.lock().unwrap().state {
            State::Open(samples) => samples.as_ptr(),
            _ => panic!("open"),
        };
        let samples = capture.finish().unwrap();
        assert_eq!(samples.as_ptr(), pointer);
        assert_eq!(&samples[..960], &[1; 960]);
        assert_eq!(&samples[960..], &[2, 3, 4]);
        assert!(capture.finish().is_err());
        assert!(input.append_pcm16_16k(&[5]).is_err());
    }
    #[test]
    fn cancel_clears_data_and_closes_every_callback_clone() {
        let capture = BufferedCapture::with_limit(960);
        let old = capture.input();
        old.append_pcm16_16k(&[7, 8]).unwrap();
        capture.cancel();
        assert!(matches!(
            capture.buffer.lock().unwrap().state,
            State::Cancelled
        ));
        assert!(old.append_pcm16_16k(&[9]).is_err());
        assert!(capture.finish().is_err());
        capture.cancel();
    }
    #[test]
    fn exact_ten_minute_limit_is_accepted_and_overflow_is_terminal() {
        assert_eq!(MAX_BUFFERED_SAMPLES * size_of::<i16>(), 19_200_000);
        let capture = BufferedCapture::new();
        let input = capture.input();
        for _ in 0..10_000 {
            input.append_pcm16_16k(&[123; 960]).unwrap();
        }
        let count = match &capture.buffer.lock().unwrap().state {
            State::Open(samples) => {
                assert_eq!(samples.capacity(), MAX_BUFFERED_SAMPLES);
                samples.len()
            }
            _ => panic!("open"),
        };
        assert_eq!(count, MAX_BUFFERED_SAMPLES);
        assert!(input.append_pcm16_16k(&[1]).is_err());
        assert!(capture.finish().is_err());
        assert!(matches!(
            capture.buffer.lock().unwrap().state,
            State::Failed(_)
        ));
        assert!(input.append_pcm16_16k(&[1]).is_err());
    }
    #[test]
    fn malformed_chunks_fail_closed_without_retaining_partial_audio() {
        for chunk in [Vec::new(), vec![0; 961]] {
            let capture = BufferedCapture::with_limit(1920);
            let input = capture.input();
            input.append_pcm16_16k(&[1]).unwrap();
            assert!(input.append_pcm16_16k(&chunk).is_err());
            assert!(capture.finish().is_err());
            assert!(input.append_pcm16_16k(&[2]).is_err());
        }
    }
    #[test]
    fn empty_recording_and_next_document_have_separate_admission() {
        let first = BufferedCapture::with_limit(960);
        let old = first.input();
        assert!(first.finish().unwrap().is_empty());
        let second = BufferedCapture::with_limit(960);
        second.input().append_pcm16_16k(&[22]).unwrap();
        assert!(old.append_pcm16_16k(&[11]).is_err());
        assert_eq!(second.finish().unwrap(), vec![22]);
    }
    #[test]
    fn simultaneous_finish_callers_cannot_duplicate_a_recording() {
        let capture = BufferedCapture::with_limit(960);
        capture.input().append_pcm16_16k(&[31, 32]).unwrap();
        let other = capture.clone();
        let worker = std::thread::spawn(move || other.finish());
        let local = capture.finish();
        let remote = worker.join().unwrap();
        assert_ne!(local.is_ok(), remote.is_ok());
        assert_eq!(local.or(remote).unwrap(), vec![31, 32]);
    }
    #[test]
    fn finish_and_append_linearize_without_late_audio_leaking() {
        let capture = BufferedCapture::with_limit(960);
        let input = capture.input();
        let append = std::thread::spawn(move || input.append_pcm16_16k(&[41]));
        let samples = capture.finish().unwrap();
        let accepted = append.join().unwrap().is_ok();
        assert_eq!(samples, if accepted { vec![41] } else { Vec::new() });
        assert!(capture.input().append_pcm16_16k(&[42]).is_err());
    }
}
