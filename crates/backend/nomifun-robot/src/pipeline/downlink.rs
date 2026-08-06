//! Reply audio → device, at the speed the device can swallow.
//!
//! Two firmware facts shape this whole file:
//!
//! 1. The decode queue holds ~40 packets (≈2.4 s) and **silently drops** what
//!    does not fit. Bursting a whole sentence tears the audio, so frames leave
//!    here on a 60 ms cadence with a small priming burst to cover jitter.
//! 2. `abort` does **not** flush the device's own queue. Cancelling a reply
//!    therefore means dropping our queued frames *immediately* — anything we
//!    still hand over will be played. That is what generations are for: `flush`
//!    bumps the counter and every frame tagged with an older one is discarded.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep_until};

use crate::audio::{AudioBuffer, OpusStreamEncoder, resample_linear};
use crate::link::Frame;
use crate::protocol::binary::encode_binary_v1;
use crate::protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS};

/// Frames allowed out back-to-back before pacing engages. Fills the device's
/// jitter buffer so the first syllable is not choppy, while staying far below
/// its ~40-packet ceiling.
pub const PRIME_FRAMES: u64 = 3;

struct PacedFrame {
    generation: u64,
    packet: Vec<u8>,
}

/// Hands Opus packets to the device on a real-time cadence.
pub struct DownlinkPacer {
    tx: mpsc::Sender<PacedFrame>,
    generation: Arc<AtomicU64>,
}

impl DownlinkPacer {
    /// Start the pacing task. `out` is the session's writer channel.
    pub fn spawn(out: mpsc::Sender<Frame>) -> (Self, JoinHandle<()>) {
        // Deep enough to hold a long sentence without blocking the encoder.
        let (tx, mut rx) = mpsc::channel::<PacedFrame>(512);
        let generation = Arc::new(AtomicU64::new(0));
        let task_generation = generation.clone();

        let handle = tokio::spawn(async move {
            let frame_gap = Duration::from_millis(FRAME_DURATION_MS as u64);
            let mut current: Option<(u64, Instant, u64)> = None; // (generation, start, index)

            while let Some(item) = rx.recv().await {
                if item.generation != task_generation.load(Ordering::SeqCst) {
                    // A cancelled turn's audio must never be played.
                    current = None;
                    continue;
                }
                let (start, index) = match current {
                    Some((generation, start, index)) if generation == item.generation => {
                        (start, index)
                    }
                    // New (or resumed) generation: restart the cadence so the
                    // next reply primes instead of inheriting an old deadline.
                    _ => (Instant::now(), 0),
                };
                // The first `PRIME_FRAMES` leave together; from then on one frame
                // per frame duration, which is exactly playback speed.
                let paced = index.saturating_sub(PRIME_FRAMES.saturating_sub(1));
                let deadline = start + frame_gap * u32::try_from(paced).unwrap_or(u32::MAX);
                sleep_until(deadline).await;

                // Re-check: the turn may have been cancelled while we slept.
                if task_generation.load(Ordering::SeqCst) != item.generation {
                    current = None;
                    continue;
                }
                if out
                    .send(Frame::Binary(encode_binary_v1(&item.packet)))
                    .await
                    .is_err()
                {
                    break;
                }
                current = Some((item.generation, start, index + 1));
            }
        });

        (Self { tx, generation }, handle)
    }

    /// The generation a new reply should be tagged with.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Queue one sentence's packets. Stale generations are dropped here so a
    /// cancelled turn cannot even occupy queue space.
    pub async fn enqueue(&self, generation: u64, packets: Vec<Vec<u8>>) {
        if generation != self.generation() {
            return;
        }
        for packet in packets {
            if generation != self.generation() {
                return;
            }
            if self
                .tx
                .send(PacedFrame { generation, packet })
                .await
                .is_err()
            {
                return;
            }
        }
    }

    /// Cancel everything queued and return the new generation.
    pub fn flush(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Resample to the rate we declared to the device and cut 60 ms Opus frames.
pub fn encode_for_downlink(
    encoder: &mut OpusStreamEncoder,
    audio: &AudioBuffer,
) -> anyhow::Result<Vec<Vec<u8>>> {
    if audio.pcm.is_empty() {
        return Ok(Vec::new());
    }
    let pcm = if audio.sample_rate == DOWNLINK_SAMPLE_RATE {
        std::borrow::Cow::Borrowed(&audio.pcm)
    } else {
        std::borrow::Cow::Owned(resample_linear(
            &audio.pcm,
            audio.sample_rate,
            DOWNLINK_SAMPLE_RATE,
        ))
    };
    encoder.encode_frames(&pcm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{AudioBuffer, OpusStreamEncoder};
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn packets(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|i| vec![0xfc, i as u8]).collect()
    }

    async fn drain(rx: &mut mpsc::Receiver<Frame>, want: usize) -> usize {
        let mut got = 0;
        while got < want {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(_)) => got += 1,
                _ => break,
            }
        }
        got
    }

    #[tokio::test(start_paused = true)]
    async fn primes_a_burst_then_holds_a_60ms_cadence() {
        let (out_tx, mut out_rx) = mpsc::channel(256);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);

        let started = tokio::time::Instant::now();
        pacer.enqueue(pacer.generation(), packets(10)).await;
        assert_eq!(drain(&mut out_rx, 10).await, 10, "every frame is delivered");

        let elapsed = started.elapsed();
        // 3 frames prime immediately; the remaining 7 are paced 60 ms apart.
        let expected = Duration::from_millis((10 - PRIME_FRAMES) * FRAME_DURATION_MS as u64);
        assert!(
            elapsed >= expected && elapsed < expected + Duration::from_millis(180),
            "expected ~{expected:?} of pacing, got {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn frames_go_out_as_bare_binary_with_no_header() {
        let (out_tx, mut out_rx) = mpsc::channel(8);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        pacer
            .enqueue(pacer.generation(), vec![vec![0xfc, 0x01, 0x02]])
            .await;

        let frame = out_rx.recv().await.unwrap();
        match frame {
            Frame::Binary(bytes) => assert_eq!(
                bytes.as_ref(),
                &[0xfc, 0x01, 0x02],
                "v1 framing means the Opus packet travels unwrapped"
            ),
            Frame::Text(t) => panic!("audio must be a binary frame, got text: {t}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn flush_drops_everything_still_queued() {
        let (out_tx, mut out_rx) = mpsc::channel(512);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let generation = pacer.generation();

        pacer.enqueue(generation, packets(200)).await;
        let bumped = pacer.flush();
        assert_eq!(bumped, generation + 1, "flush advances the generation");

        // Let the pacer work through its queue; stale frames must be discarded.
        tokio::time::advance(Duration::from_secs(30)).await;
        let mut delivered = 0;
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await
        {
            delivered += 1;
        }
        assert!(
            delivered <= PRIME_FRAMES as usize + 2,
            "only the already-primed frames may escape a flush, got {delivered}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stale_enqueues_are_ignored_outright() {
        let (out_tx, mut out_rx) = mpsc::channel(64);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let stale = pacer.generation();
        pacer.flush();

        pacer.enqueue(stale, packets(5)).await;
        tokio::time::advance(Duration::from_secs(5)).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), out_rx.recv())
                .await
                .is_err(),
            "frames from a cancelled turn must never reach the device"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cadence_restarts_after_a_flush_so_the_next_turn_primes_again() {
        let (out_tx, mut out_rx) = mpsc::channel(256);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        pacer.enqueue(pacer.generation(), packets(4)).await;
        drain(&mut out_rx, 4).await;

        let generation = pacer.flush();
        let started = tokio::time::Instant::now();
        pacer.enqueue(generation, packets(3)).await;
        assert_eq!(drain(&mut out_rx, 3).await, 3);
        assert!(
            started.elapsed() < Duration::from_millis(30),
            "a fresh generation primes immediately, it does not inherit the old deadline"
        );
    }

    #[test]
    fn encode_for_downlink_resamples_to_24k_and_frames_at_60ms() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        // 120 ms at 16 kHz becomes 120 ms at 24 kHz = two 60 ms frames.
        let audio = AudioBuffer {
            pcm: vec![0i16; 16_000 * 120 / 1000],
            sample_rate: 16_000,
        };
        let frames = encode_for_downlink(&mut encoder, &audio).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| !f.is_empty()));
    }

    #[test]
    fn encode_for_downlink_passes_24k_through_untouched() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let audio = AudioBuffer {
            pcm: vec![0i16; 24_000 * 180 / 1000],
            sample_rate: 24_000,
        };
        assert_eq!(encode_for_downlink(&mut encoder, &audio).unwrap().len(), 3);
    }

    #[test]
    fn encode_for_downlink_tolerates_empty_audio() {
        let mut encoder = OpusStreamEncoder::new_downlink().unwrap();
        let audio = AudioBuffer {
            pcm: Vec::new(),
            sample_rate: 24_000,
        };
        assert!(encode_for_downlink(&mut encoder, &audio).unwrap().is_empty());
    }
}
