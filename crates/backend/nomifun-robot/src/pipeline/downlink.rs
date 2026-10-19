//! Reply audio → device, at the speed the device can swallow.
//!
//! Two firmware facts shape this whole file:
//!
//! 1. The decode queue holds ~40 packets (≈2.4 s) and **silently drops** what
//!    does not fit. Bursting a whole sentence tears the audio, so frames leave
//!    here on a 60 ms cadence with a small priming burst to cover jitter.
//! 2. `abort` used not to flush the device's own queue, so cancelling a reply
//!    meant dropping our queued frames *immediately* — anything we still handed
//!    over would be played. That is what generations are for: `flush` bumps the
//!    counter and every frame tagged with an older one is discarded. Current
//!    firmware does now flush on abort (`AbortSpeaking` calls `ResetDecoder`),
//!    but generations stay: they are what makes cancellation correct against any
//!    firmware version, and they cost nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep_until};

use crate::audio::{AudioBuffer, OpusStreamEncoder, resample_linear};
use crate::link::Frame;
use crate::protocol::binary::encode_binary_v1;
use crate::protocol::{DOWNLINK_SAMPLE_RATE, FRAME_DURATION_MS};
use crate::services::RobotOutputGuard;

/// Frames allowed out back-to-back before pacing engages. Fills the device's
/// jitter buffer so the first syllable is not choppy, while staying far below
/// its ~40-packet ceiling.
pub const PRIME_FRAMES: u64 = 3;

/// One queued outbound frame. Text rides the same queue as audio so a `tts
/// stop` cannot overtake the sentence it ends.
struct PacedItem {
    generation: u64,
    frame: Frame,
    delivered:Option<tokio::sync::oneshot::Sender<()>>,
    guard:Option<RobotOutputGuard>,
}

/// Hands Opus packets to the device on a real-time cadence.
pub struct DownlinkPacer {
    tx: mpsc::Sender<PacedItem>,
    generation: Arc<AtomicU64>,
    generation_changed: Arc<tokio::sync::Notify>,
}

impl DownlinkPacer {
    /// Start the pacing task. `out` is the session's writer channel.
    pub fn spawn(out: mpsc::Sender<Frame>) -> (Self, JoinHandle<()>) {
        // At most 1.92 seconds of 60ms audio. Pull-based provider streaming
        // stops here while the real-time consumer catches up.
        let (tx, mut rx) = mpsc::channel::<PacedItem>(32);
        let generation = Arc::new(AtomicU64::new(0));
        let task_generation = generation.clone();
        let generation_changed=Arc::new(tokio::sync::Notify::new());
        let task_changed=generation_changed.clone();

        let handle = tokio::spawn(async move {
            let frame_gap = Duration::from_millis(FRAME_DURATION_MS as u64);
            let mut current: Option<(u64, Instant, u64)> = None; // (generation, start, index)

            while let Some(item) = rx.recv().await {
                if item.generation != task_generation.load(Ordering::SeqCst) {
                    // A cancelled turn's audio must never be played.
                    current = None;
                    continue;
                }
                let now = Instant::now();
                let (start, index) = match current {
                    Some((generation, start, index)) if generation == item.generation => {
                        // The deadline this frame would inherit. If it is already
                        // in the past, the pacer has been idle — a reply is many
                        // sentences and each one is synthesised by a network round
                        // trip *before* it is enqueued, so between sentences there
                        // is nothing to send. Keeping the old anchor turns that
                        // idle time into credit: every deadline is overdue, so the
                        // whole next sentence leaves at once. The credit also
                        // accumulates across sentences, so a long reply eventually
                        // dumps more than the device's ~40-packet decode queue
                        // holds, it silently drops the overflow, and the listener
                        // hears a fragment skipped mid-reply. Re-base instead, so
                        // the sentence re-primes into a jitter buffer that has
                        // drained anyway while we waited for the synthesiser.
                        let due = start
                            + frame_gap
                                * u32::try_from(
                                    index.saturating_sub(PRIME_FRAMES.saturating_sub(1)),
                                )
                                .unwrap_or(u32::MAX);
                        if now > due + frame_gap {
                            (now, 0)
                        } else {
                            (start, index)
                        }
                    }
                    // New (or resumed) generation: restart the cadence so the
                    // next reply primes instead of inheriting an old deadline.
                    _ => (now, 0),
                };
                // The first `PRIME_FRAMES` leave together; from then on one frame
                // per frame duration, which is exactly playback speed. Text
                // frames cost no playback time, so they inherit the deadline of
                // the audio they follow instead of advancing it.
                let paced = index.saturating_sub(PRIME_FRAMES.saturating_sub(1));
                let is_audio = matches!(item.frame, Frame::Binary(_));
                let deadline = start + frame_gap * u32::try_from(paced).unwrap_or(u32::MAX);
                sleep_until(deadline).await;

                // Re-check: the turn may have been cancelled while we slept.
                if task_generation.load(Ordering::SeqCst) != item.generation {
                    current = None;
                    continue;
                }
                let changed=task_changed.notified();tokio::pin!(changed);changed.as_mut().enable();
                if task_generation.load(Ordering::SeqCst)!=item.generation {current=None;continue;}
                // Reserve capacity before acquiring canonical read authority.
                // Backpressure must never hold up a clear/rebind write lease.
                let permit=tokio::select! {biased;
                    _=changed.as_mut()=>{current=None;continue;},
                    permit=out.reserve()=>match permit {Ok(permit)=>permit,Err(_)=>break},
                };
                let lease=if let Some(guard)=item.guard {
                    match tokio::select! {biased;
                        _=changed.as_mut()=>{current=None;continue;},
                        lease=guard.acquire()=>lease,
                    } {
                        Ok(Some(lease))=>Some(lease),
                        Ok(None)|Err(_)=>{
                            if task_generation.compare_exchange(item.generation,item.generation.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst).is_ok() {
                                task_changed.notify_waiters();
                            }
                            current=None;continue;
                        }
                    }
                }else{None};
                if task_generation.load(Ordering::SeqCst)!=item.generation {current=None;continue;}
                permit.send(item.frame);
                drop(lease);
                if let Some(delivered)=item.delivered {let _=delivered.send(());}
                current = Some((
                    item.generation,
                    start,
                    if is_audio { index + 1 } else { index },
                ));
            }
        });

        (Self { tx, generation, generation_changed }, handle)
    }

    /// The generation a new reply should be tagged with.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
    pub async fn generation_cancelled(&self,generation:u64) {
        loop {
            let changed=self.generation_changed.notified();tokio::pin!(changed);changed.as_mut().enable();
            if self.generation()!=generation{return;}
            changed.await;
        }
    }

    /// Queue one sentence's packets. Stale generations are dropped here so a
    /// cancelled turn cannot even occupy queue space.
    pub async fn enqueue(&self, generation: u64, packets: Vec<Vec<u8>>) {
        self.enqueue_guarded(generation,packets,None).await;
    }

    pub async fn enqueue_guarded(&self,generation:u64,packets:Vec<Vec<u8>>,guard:Option<RobotOutputGuard>) {
        if generation != self.generation() {
            return;
        }
        for packet in packets {
            if generation != self.generation() {
                return;
            }
            if self
                .tx
                .send(PacedItem {
                    generation,
                    frame: Frame::Binary(encode_binary_v1(&packet)),
                    delivered:None,
                    guard:guard.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
        }
    }

    /// Queue a text frame behind the audio already queued for `generation`.
    /// The device drops downlink audio the moment it leaves the speaking state,
    /// so `tts stop` has to travel this way rather than jumping the queue.
    /// Returns whether it was accepted.
    pub async fn enqueue_text(&self, generation: u64, text: String) -> bool {
        if generation != self.generation() {
            return false;
        }
        self.tx
            .send(PacedItem {
                generation,
                frame: Frame::Text(text),
                delivered:None,
                guard:None,
            })
            .await
            .is_ok()
    }
    /// Await transport enqueue of the final stop behind all prior audio.
    /// This is writer acceptance, never a claim that the device played it.
    /// A generation flush releases the waiter immediately.
    pub async fn enqueue_text_and_wait(&self,generation:u64,text:String)->bool {
        if self.generation()!=generation{return false;}
        let (delivered,received)=tokio::sync::oneshot::channel();
        tokio::select! {biased;
            _=self.generation_cancelled(generation)=>return false,
            sent=self.tx.send(PacedItem {generation,frame:Frame::Text(text),delivered:Some(delivered),guard:None})=>if sent.is_err(){return false;},
        }
        tokio::select! {biased;
            _=self.generation_cancelled(generation)=>false,
            result=received=>result.is_ok()&&self.generation()==generation,
        }
    }

    /// Cancel everything queued and return the new generation.
    pub fn flush(&self) -> u64 {
        let generation=self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.generation_changed.notify_waiters();generation
    }
    pub fn flush_if_current(&self,generation:u64)->bool {
        if self.generation.compare_exchange(generation,generation.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst).is_ok() {
            self.generation_changed.notify_waiters();true
        }else{false}
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

    struct GuardDispatcher {
        owner:Arc<tokio::sync::RwLock<()>>,
        allowed:std::sync::atomic::AtomicBool,
        acquired:std::sync::atomic::AtomicUsize,
        entered:tokio::sync::Notify,
        gate:Option<Arc<tokio::sync::Semaphore>>,
    }
    #[async_trait::async_trait]
    impl crate::services::CompanionTurnDispatcher for GuardDispatcher {
        async fn acquire_output_context(&self,_:&crate::services::RobotTurnRequest,_:Option<u64>,_:Option<&crate::services::RobotQuestionPlaybackAuthority>)->anyhow::Result<Option<crate::services::RobotOutputLease>> {
            let owner=self.owner.clone().read_owned().await;
            if !self.allowed.load(Ordering::SeqCst) {return Ok(None);}
            self.acquired.fetch_add(1,Ordering::SeqCst);self.entered.notify_one();
            if let Some(gate)=&self.gate {gate.acquire().await.unwrap().forget();}
            Ok(Some(crate::services::RobotOutputLease::new(owner)))
        }
        async fn ensure_companion_session(&self,_:&str,_:&str)->anyhow::Result<String> {anyhow::bail!("unused fixture")}
        async fn dispatch(&self,_:crate::services::RobotTurnRequest)->anyhow::Result<mpsc::Receiver<crate::services::TurnEvent>> {anyhow::bail!("unused fixture")}
        async fn cancel(&self,_:&crate::services::RobotTurnRequest)->anyhow::Result<()> {Ok(())}
        async fn vad_tuning(&self,_:&str)->crate::vad::VadTuning {Default::default()}
        async fn vad_engine(&self,_:&str)->String {"energy".into()}
    }
    fn output_guard(dispatcher:Arc<GuardDispatcher>)->RobotOutputGuard {
        RobotOutputGuard::new(dispatcher,crate::services::RobotTurnRequest {
            robot_id:"robot".into(),companion_id:"companion".into(),conversation_id:"session".into(),connection_id:"connection".into(),
            request_id:"request".into(),text:"public answer".into(),presented_question_id:None,captured_context:None,
        })
    }
    fn guard_dispatcher(gate:Option<Arc<tokio::sync::Semaphore>>)->Arc<GuardDispatcher> {
        Arc::new(GuardDispatcher {owner:Arc::new(tokio::sync::RwLock::new(())),allowed:std::sync::atomic::AtomicBool::new(true),
            acquired:std::sync::atomic::AtomicUsize::new(0),entered:tokio::sync::Notify::new(),gate})
    }

    #[tokio::test(start_paused=true)]
    async fn canonical_guard_acquires_after_backpressure_and_revocation_fences_the_entire_generation() {
        let (out,mut received)=mpsc::channel(1);
        out.send(Frame::Text("occupied".into())).await.unwrap();
        let (pacer,task)=DownlinkPacer::spawn(out);
        let owner=guard_dispatcher(None);
        let generation=pacer.generation();
        pacer.enqueue_guarded(generation,packets(3),Some(output_guard(owner.clone()))).await;
        tokio::time::advance(Duration::from_millis(10)).await;
        assert_eq!(owner.acquired.load(Ordering::SeqCst),0,"writer backpressure must not hold canonical read authority");
        let exclusive=tokio::time::timeout(Duration::from_secs(1),owner.owner.clone().write_owned()).await.unwrap();
        owner.allowed.store(false,Ordering::SeqCst);drop(exclusive);
        assert!(matches!(received.recv().await,Some(Frame::Text(_))));
        tokio::time::timeout(Duration::from_secs(1),pacer.generation_cancelled(generation)).await.unwrap();
        assert!(received.try_recv().is_err(),"no stale audio may enter the writer after canonical revocation");
        task.abort();let _=task.await;
    }

    #[tokio::test]
    async fn generation_flush_cancels_a_pending_guard_acquisition_and_releases_its_owner_lease() {
        let (out,mut received)=mpsc::channel(1);
        let (pacer,task)=DownlinkPacer::spawn(out);
        let owner=guard_dispatcher(Some(Arc::new(tokio::sync::Semaphore::new(0))));
        let generation=pacer.generation();
        pacer.enqueue_guarded(generation,packets(1),Some(output_guard(owner.clone()))).await;
        tokio::time::timeout(Duration::from_secs(1),owner.entered.notified()).await.unwrap();
        pacer.flush();
        let exclusive=tokio::time::timeout(Duration::from_secs(1),owner.owner.clone().write_owned()).await
            .expect("cancelling guard acquisition must release the exact read owner");
        drop(exclusive);
        assert!(received.try_recv().is_err());
        task.abort();let _=task.await;
    }

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

    /// A reply is many sentences, and each one is synthesised by a network round
    /// trip *before* it is enqueued. The pacer must not treat that idle gap as
    /// credit: if it does, the whole next sentence leaves at once, the device's
    /// ~40-packet decode queue silently drops the overflow, and the listener
    /// hears a fragment skipped mid-reply.
    #[tokio::test(start_paused = true)]
    async fn a_sentence_arriving_after_tts_latency_must_not_burst() {
        let (out_tx, mut out_rx) = mpsc::channel(512);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let generation = pacer.generation();

        // Sentence 1 goes out on cadence.
        pacer.enqueue(generation, packets(10)).await;
        assert_eq!(drain(&mut out_rx, 10).await, 10, "sentence 1 delivered");

        // The gateway now calls the TTS provider for sentence 2. The pacer is
        // idle for that whole round trip.
        tokio::time::advance(Duration::from_millis(1500)).await;

        let started = tokio::time::Instant::now();
        pacer.enqueue(generation, packets(50)).await;
        assert_eq!(drain(&mut out_rx, 50).await, 50, "sentence 2 delivered");
        let elapsed = started.elapsed();

        // 3 frames prime, the other 47 are one per 60 ms of playback.
        let expected = Duration::from_millis((50 - PRIME_FRAMES) * FRAME_DURATION_MS as u64);
        assert!(
            elapsed >= expected,
            "sentence 2 was burst after the TTS gap: delivered 50 frames in {elapsed:?}, \
             but 50 frames are {expected:?} of playback. The device drops the overflow."
        );
    }

    /// The real shape of the bug: credit accumulated across several sentences.
    /// Each TTS round trip adds idle time, and with a stale anchor every gap is
    /// banked, so by the third or fourth sentence the pacer dumps several
    /// seconds at once — far past the device's ~40-packet ceiling.
    #[tokio::test(start_paused = true)]
    async fn idle_gaps_do_not_accumulate_into_credit_across_sentences() {
        let (out_tx, mut out_rx) = mpsc::channel(1024);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let generation = pacer.generation();

        // Four sentences, each preceded by a synthesiser round trip.
        for _ in 0..4 {
            tokio::time::advance(Duration::from_millis(900)).await;
            let started = tokio::time::Instant::now();
            pacer.enqueue(generation, packets(20)).await;
            assert_eq!(drain(&mut out_rx, 20).await, 20, "sentence delivered");
            let elapsed = started.elapsed();
            let expected =
                Duration::from_millis((20 - PRIME_FRAMES) * FRAME_DURATION_MS as u64);
            assert!(
                elapsed >= expected,
                "a sentence was burst: 20 frames in {elapsed:?}, but that is {expected:?} \
                 of playback. Banked idle time must not become credit."
            );
        }
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

        pacer.enqueue(generation, packets(20)).await;
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

    #[tokio::test(start_paused = true)]
    async fn a_paced_text_frame_lands_after_the_audio_queued_before_it() {
        let (out_tx, mut out_rx) = mpsc::channel(256);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let generation = pacer.generation();
        pacer.enqueue(generation, packets(8)).await;
        assert!(pacer.enqueue_text(generation, "stop".to_owned()).await);

        let mut order = Vec::new();
        while order.len() < 9 {
            match tokio::time::timeout(Duration::from_secs(5), out_rx.recv()).await {
                Ok(Some(frame)) => order.push(frame),
                _ => break,
            }
        }
        assert_eq!(order.len(), 9, "audio plus the stop");
        assert!(
            order[..8].iter().all(|f| matches!(f, Frame::Binary(_))),
            "the audio comes first"
        );
        assert!(
            matches!(order[8], Frame::Text(_)),
            "the stop must not overtake the audio it ends"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_stale_text_frame_is_refused_so_the_caller_can_fall_back() {
        let (out_tx, _out_rx) = mpsc::channel(8);
        let (pacer, _task) = DownlinkPacer::spawn(out_tx);
        let stale = pacer.generation();
        pacer.flush();
        assert!(!pacer.enqueue_text(stale, "stop".to_owned()).await);
    }

    #[tokio::test(start_paused=true)]
    async fn flush_releases_a_pending_stop_barrier_without_waiting_for_audio_time() {
        let (out,_receiver)=mpsc::channel(1);
        let (pacer,_task)=DownlinkPacer::spawn(out);let pacer=Arc::new(pacer);
        let generation=pacer.generation();pacer.enqueue(generation,packets(12)).await;
        let pending={let pacer=pacer.clone();tokio::spawn(async move {pacer.enqueue_text_and_wait(generation,"stop".into()).await})};
        tokio::task::yield_now().await;assert!(!pending.is_finished());
        pacer.flush();assert!(!pending.await.unwrap());
    }

    #[tokio::test(start_paused=true)]
    async fn flush_cancels_the_old_frame_waiting_for_writer_capacity() {
        let (out,mut receiver)=mpsc::channel(1);
        let (pacer,_task)=DownlinkPacer::spawn(out);
        let generation=pacer.generation();pacer.enqueue(generation,packets(5)).await;
        tokio::task::yield_now().await; // One accepted frame; a second send is blocked.
        pacer.flush();
        assert!(receiver.recv().await.is_some(),"writer acceptance before flush remains an uncertain sent tail");
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(receiver.try_recv().is_err(),"a formerly blocked old frame cannot enter the writer after flush");
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
