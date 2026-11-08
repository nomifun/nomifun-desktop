//! A reversible pause owned by the caller until package handoff succeeds.
//! Dropping an interrupted preflight restores admission without restoring any
//! held key or recording intent. Final application shutdown has a separate flag.
use super::Message;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use tokio::sync::{mpsc, oneshot};

const UPDATE_FENCE: u64 = 1 << 62;
const SHUTDOWN_FENCE: u64 = 1 << 63;
const OWNER_MASK: u64 = UPDATE_FENCE - 1;

/// The existing native capture producer's stop authority. UI/Actor errors and
/// dropped handles cannot release an owner; only its positive native receipt
/// (or proof that dispatch never opened a stream) can do that. Fences and owner
/// claims share one atomic state so a handoff cannot race a late native start.
#[derive(Default)]
pub(super) struct CaptureSafety {
    state: AtomicU64,
    next: AtomicU64,
    changed: tokio::sync::Notify,
}
#[derive(Clone)]
pub(super) struct CaptureLease { safety: Arc<CaptureSafety>, generation: u64 }
impl CaptureSafety {
    pub fn claim_for_host(self: &Arc<Self>, host_stop: &AtomicBool) -> Result<CaptureLease, String> {
        if host_stop.load(Ordering::Acquire) {
            self.fence_shutdown();
            return Err("应用正在更新或退出，不能启动麦克风".into());
        }
        let lease = self.claim()?;
        if host_stop.load(Ordering::Acquire) {
            self.fence_shutdown();
            lease.confirm_stopped(); // Native dispatch has not happened.
            return Err("应用已经接管退出，麦克风未启动".into());
        }
        Ok(lease)
    }
    pub fn claim(self: &Arc<Self>) -> Result<CaptureLease, String> {
        let generation = self.next.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |value| value.checked_add(1).filter(|next| *next <= OWNER_MASK))
            .map_err(|_| "麦克风所有权代次已耗尽".to_owned())? + 1;
        self.state.compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "麦克风仍有未确认停止的所有者，或应用正在更新/退出".to_owned())?;
        Ok(CaptureLease { safety: self.clone(), generation })
    }
    pub fn stopped(&self) -> bool { self.state.load(Ordering::Acquire) & OWNER_MASK == 0 }
    pub fn fenced(&self) -> bool { self.state.load(Ordering::Acquire) & !OWNER_MASK != 0 }
    pub fn fence_update(&self) { self.state.fetch_or(UPDATE_FENCE, Ordering::AcqRel); }
    pub fn resume_update(&self) { self.state.fetch_and(!UPDATE_FENCE, Ordering::AcqRel); }
    pub fn fence_shutdown(&self) { self.state.fetch_or(SHUTDOWN_FENCE, Ordering::AcqRel); }
    pub async fn wait_stopped(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.stopped() { return; }
            changed.await;
        }
    }
}
impl CaptureLease {
    pub fn confirm_stopped(&self) {
        let mut state = self.safety.state.load(Ordering::Acquire);
        loop {
            if state & OWNER_MASK != self.generation { return; }
            match self.safety.state.compare_exchange_weak(state, state & !OWNER_MASK, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => { self.safety.changed.notify_waiters(); return; }
                Err(current) => state = current,
            }
        }
    }
}

pub(super) fn require_stopped(capture: Option<&CaptureSafety>) -> Result<(), String> {
    let Some(capture) = capture else { return Ok(()); };
    capture.fence_shutdown();
    if capture.stopped() { Ok(()) }
    else { Err("系统麦克风仍有未确认停止的所有者，不能替换应用文件".into()) }
}

pub(super) async fn await_stopped(capture: &CaptureSafety, mut reply: oneshot::Receiver<Result<(), String>>, wait: std::time::Duration) -> Result<(), String> {
    if capture.stopped() { return Ok(()); }
    let _ = tokio::time::timeout(wait, capture.wait_stopped()).await;
    // The native producer is authoritative, including when the Actor callback
    // timed out, disappeared, or reported an unrelated optional-device error.
    if capture.stopped() { return Ok(()); }
    match reply.try_recv() {
        Ok(Err(error)) => Err(format!("麦克风停止仍未确认：{error}")),
        Ok(Ok(())) => Err("语音控制已应答，但系统麦克风停止仍未确认".into()),
        Err(_) => Err("等待系统麦克风停止超时，所有者仍未确认释放".into()),
    }
}

pub(crate) struct UpdatePause {
    recovery: Option<Recovery>,
}

struct Recovery {
    urgent: mpsc::UnboundedSender<Message>,
    admission: Arc<UpdateAdmission>,
    capture: Arc<CaptureSafety>,
}

#[derive(Default)]
pub(super) struct UpdateAdmission {
    paused: AtomicBool,
}

impl UpdateAdmission {
    pub(super) fn paused(&self) -> bool { self.paused.load(Ordering::Acquire) }

    fn release_after_resume(&self, enqueue_resume: impl FnOnce()) {
        // A subsequent prepare can start as soon as paused becomes false. Its
        // PrepareUpdate must never overtake the previous owner's queued resume.
        enqueue_resume();
        self.paused.store(false, Ordering::Release);
    }
}

impl Recovery {
    fn resume(self, reply: Option<oneshot::Sender<()>>) {
        // This releases the actor pause while retaining its serial transport.
        // The actor independently revokes key intent and waits for native capture teardown before it can
        // accept another recording.
        self.admission.release_after_resume(|| {
            let _ = self.urgent.send(Message::ResumeAfterUpdate(reply));
            self.capture.resume_update();
        });
    }
}

impl UpdatePause {
    pub(super) fn begin(
        urgent: mpsc::UnboundedSender<Message>,
        admission: Arc<UpdateAdmission>,
        capture: Arc<CaptureSafety>,
    ) -> Result<(Self, oneshot::Receiver<Result<(), String>>), String> {
        if admission.paused.swap(true, Ordering::AcqRel) {
            return Err("语音更新预检已在进行".into());
        }
        capture.fence_update();
        let guard = Self { recovery: Some(Recovery { urgent, admission, capture }) };
        let (reply, result) = oneshot::channel();
        if let Err(tokio::sync::mpsc::error::SendError(Message::PrepareUpdate(reply))) = guard.recovery.as_ref().unwrap().urgent.send(Message::PrepareUpdate(reply)) {
            let safe = guard.recovery.as_ref().unwrap().capture.stopped();
            let _ = reply.send(if safe { Ok(()) } else { Err("语音控制通道已关闭，麦克风停止仍未确认".into()) });
        }
        Ok((guard, result))
    }

    /// Only commit after an installer has accepted ownership of replacement.
    /// Windows' upstream installer terminates the host process at that handoff,
    /// so it deliberately never returns to drop this guard on success.
    pub(crate) fn commit(mut self) {
        self.recovery.take();
    }

    pub(crate) async fn cancel(mut self) -> Result<(), String> {
        if let Some(recovery) = self.recovery.take() {
            let (reply, result) = oneshot::channel();
            let capture = recovery.capture.clone();
            recovery.resume(Some(reply));
            // Recovery is optional when the Actor is gone and no native owner
            // remains. Retain the real capture error when stop is unproven.
            let resumed = tokio::time::timeout(std::time::Duration::from_secs(3), result).await;
            if !matches!(resumed, Ok(Ok(()))) && !capture.stopped() {
                return Err("语音恢复未确认，麦克风仍有活动所有者".into());
            }
        }
        Ok(())
    }
}

impl Drop for UpdatePause {
    fn drop(&mut self) {
        if let Some(recovery) = self.recovery.take() {
            recovery.resume(None);
        }
    }
}

#[derive(Default)]
pub(super) struct Suspension {
    system: bool,
    update: bool,
}

impl Suspension {
    pub(super) fn initial() -> Self { Self { system: true, update: false } }
    pub(super) fn system(&mut self, paused: bool) -> bool {
        self.system = paused;
        self.paused()
    }
    pub(super) fn update(&mut self, paused: bool) -> bool {
        self.update = paused;
        self.paused()
    }
    fn paused(&self) -> bool { self.system || self.update }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_voice_runtime_does_not_gate_application_handoff() {
        require_stopped(None).unwrap();
        let host_stop = AtomicBool::new(true);
        let late_runtime = Arc::new(CaptureSafety::default());
        assert!(late_runtime.claim_for_host(&host_stop).is_err());
        assert!(late_runtime.stopped());
    }

    #[test]
    fn dropping_an_owner_or_its_actor_is_not_native_stop_proof() {
        let capture = Arc::new(CaptureSafety::default());
        let lease = capture.claim().unwrap();
        drop(lease);
        assert!(!capture.stopped());
        assert!(require_stopped(Some(&capture)).is_err());
    }

    #[test]
    fn only_matching_native_receipt_releases_the_current_generation() {
        let capture = Arc::new(CaptureSafety::default());
        let old = capture.claim().unwrap();
        old.confirm_stopped();
        let current = capture.claim().unwrap();
        old.confirm_stopped();
        assert!(!capture.stopped());
        current.confirm_stopped();
        assert!(capture.stopped());
    }

    #[test]
    fn update_cancellation_cannot_remove_final_shutdown_fence() {
        let capture = Arc::new(CaptureSafety::default());
        let lease = capture.claim().unwrap();
        capture.fence_update();
        capture.fence_shutdown();
        lease.confirm_stopped();
        capture.resume_update();
        assert!(capture.stopped());
        assert!(capture.claim().is_err());
    }

    #[test]
    fn handed_off_capture_fence_rejects_a_racing_native_dispatch() {
        let capture = Arc::new(CaptureSafety::default());
        require_stopped(Some(&capture)).unwrap();
        let late = capture.clone();
        std::thread::spawn(move || assert!(late.claim().is_err())).join().unwrap();
        assert!(capture.stopped());
    }

    #[tokio::test]
    async fn closed_actor_channel_without_capture_retains_pause_but_does_not_gate_update() {
        let capture = Arc::new(CaptureSafety::default());
        let (urgent, messages) = mpsc::unbounded_channel();
        drop(messages);
        let admission = Arc::new(UpdateAdmission::default());
        let (guard, reply) = UpdatePause::begin(urgent, admission.clone(), capture.clone()).unwrap();
        await_stopped(&capture, reply, std::time::Duration::from_millis(10)).await.unwrap();
        assert!(admission.paused());
        assert!(capture.claim().is_err());
        guard.cancel().await.unwrap();
        assert!(!admission.paused());
        assert!(!capture.fenced());
    }

    #[tokio::test]
    async fn closed_actor_channel_with_live_capture_fails_closed() {
        let capture = Arc::new(CaptureSafety::default());
        let lease = capture.claim().unwrap();
        let (urgent, messages) = mpsc::unbounded_channel();
        drop(messages);
        let (guard, reply) = UpdatePause::begin(urgent, Arc::new(UpdateAdmission::default()), capture.clone()).unwrap();
        assert!(await_stopped(&capture, reply, std::time::Duration::from_millis(10)).await.is_err());
        assert!(require_stopped(Some(&capture)).is_err());
        drop(guard);
        assert!(!capture.stopped());
        lease.confirm_stopped();
        require_stopped(Some(&capture)).unwrap();
    }

    #[tokio::test]
    async fn native_producer_proof_completes_stop_without_actor_acknowledgement() {
        let capture = Arc::new(CaptureSafety::default());
        let lease = capture.claim().unwrap();
        let (_reply, result) = oneshot::channel();
        let waiting = capture.clone();
        let task = tokio::spawn(async move { await_stopped(&waiting, result, std::time::Duration::from_secs(1)).await });
        tokio::task::yield_now().await;
        lease.confirm_stopped();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn late_native_proof_wins_after_an_actor_error() {
        let capture = Arc::new(CaptureSafety::default());
        let lease = capture.claim().unwrap();
        let (reply, result) = oneshot::channel();
        reply.send(Err("Actor is unavailable".into())).unwrap();
        let waiting = capture.clone();
        let task = tokio::spawn(async move { await_stopped(&waiting, result, std::time::Duration::from_secs(1)).await });
        tokio::task::yield_now().await;
        lease.confirm_stopped();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn optional_actor_errors_and_timeout_cannot_override_no_native_owner() {
        let capture = Arc::new(CaptureSafety::default());
        let (reply, result) = oneshot::channel();
        reply.send(Err("serial or plugin cleanup failed".into())).unwrap();
        await_stopped(&capture, result, std::time::Duration::ZERO).await.unwrap();
        let (_reply, result) = oneshot::channel();
        await_stopped(&capture, result, std::time::Duration::ZERO).await.unwrap();
    }

    #[tokio::test]
    async fn actor_success_is_not_a_substitute_for_native_producer_proof() {
        let capture = Arc::new(CaptureSafety::default());
        let _lease = capture.claim().unwrap();
        let (reply, result) = oneshot::channel();
        reply.send(Ok(())).unwrap();
        assert!(await_stopped(&capture, result, std::time::Duration::from_millis(10)).await.is_err());
        assert!(!capture.stopped());
    }

    #[test]
    fn exhausted_capture_generations_never_reuse_a_retired_owner_identity() {
        let capture = Arc::new(CaptureSafety::default());
        capture.next.store(OWNER_MASK, Ordering::Release);
        assert!(capture.claim().is_err());
        assert!(capture.stopped());
    }

    #[tokio::test]
    async fn dropping_a_cancelled_preflight_restores_admission_and_queues_actor_recovery() {
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let paused = Arc::new(UpdateAdmission::default());
        let (guard, result) = UpdatePause::begin(urgent, paused.clone(), Arc::new(CaptureSafety::default())).unwrap();
        assert!(paused.paused());
        let Message::PrepareUpdate(reply) = messages.recv().await.unwrap() else { panic!("prepare first") };
        // A dropped command future can leave native stop in flight. Recovery
        // still follows prepare on the non-lossy actor control channel.
        drop(result);
        drop(guard);
        assert!(!paused.paused());
        assert!(matches!(messages.recv().await, Some(Message::ResumeAfterUpdate(None))));
        assert!(reply.send(Ok(())).is_err());
    }

    #[tokio::test]
    async fn native_stop_failure_can_cancel_and_retry_without_losing_the_actor() {
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let paused = Arc::new(UpdateAdmission::default());
        let (guard, result) = UpdatePause::begin(urgent.clone(), paused.clone(), Arc::new(CaptureSafety::default())).unwrap();
        let Message::PrepareUpdate(reply) = messages.recv().await.unwrap() else { panic!("prepare first") };
        reply.send(Err("native owner has not stopped".into())).unwrap();
        assert!(result.await.unwrap().is_err());
        let cancelled = tokio::spawn(guard.cancel());
        let Message::ResumeAfterUpdate(Some(reply)) = messages.recv().await.unwrap() else { panic!("resume expected") };
        reply.send(()).unwrap();
        cancelled.await.unwrap().unwrap();
        assert!(!paused.paused());
        let (retry, _) = UpdatePause::begin(urgent, paused.clone(), Arc::new(CaptureSafety::default())).unwrap();
        drop(retry);
        assert!(!paused.paused());
    }

    #[tokio::test]
    async fn committed_handoff_keeps_admission_paused_until_process_replacement() {
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let paused = Arc::new(UpdateAdmission::default());
        let (guard, result) = UpdatePause::begin(urgent.clone(), paused.clone(), Arc::new(CaptureSafety::default())).unwrap();
        assert!(UpdatePause::begin(urgent, paused.clone(), Arc::new(CaptureSafety::default())).is_err());
        let Message::PrepareUpdate(reply) = messages.recv().await.unwrap() else { panic!("prepare first") };
        reply.send(Ok(())).unwrap();
        result.await.unwrap().unwrap();
        guard.commit();
        assert!(paused.paused());
        assert!(messages.try_recv().is_err());
    }

    #[test]
    fn cancel_does_not_resume_a_locked_desktop_and_unlock_does_not_override_update() {
        let mut state = Suspension::initial();
        assert!(!state.system(false));
        assert!(state.update(true));
        assert!(state.system(false));
        assert!(state.system(true));
        assert!(state.update(false));
        assert!(!state.system(false));
    }

    #[test]
    fn a_new_prepare_cannot_overtake_the_previous_owners_resume_publication() {
        let admission = Arc::new(UpdateAdmission::default());
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let (mut old, _) = UpdatePause::begin(urgent.clone(), admission.clone(), Arc::new(CaptureSafety::default())).unwrap();
        assert!(matches!(messages.try_recv(), Ok(Message::PrepareUpdate(_))));
        let recovery = old.recovery.take().unwrap();
        // Attempt the competing begin exactly while resume is being published,
        // rather than relying on scheduler timing to reproduce the old race.
        recovery.admission.release_after_resume(|| {
            assert!(UpdatePause::begin(urgent.clone(), admission.clone(), Arc::new(CaptureSafety::default())).is_err());
            recovery.urgent.send(Message::ResumeAfterUpdate(None)).unwrap();
        });
        let (next, _) = UpdatePause::begin(urgent, admission, Arc::new(CaptureSafety::default())).unwrap();
        assert!(matches!(messages.try_recv(), Ok(Message::ResumeAfterUpdate(None))));
        assert!(matches!(messages.try_recv(), Ok(Message::PrepareUpdate(_))));
        next.commit();
    }
}
