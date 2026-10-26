//! A reversible pause owned by the caller until package handoff succeeds.
//! Dropping an interrupted preflight restores admission without restoring any
//! held key or recording intent. Final application shutdown has a separate flag.
use super::Message;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};
use tokio::sync::{mpsc, oneshot};

pub(crate) struct UpdatePause {
    recovery: Option<Recovery>,
}

struct Recovery {
    urgent: mpsc::UnboundedSender<Message>,
    admission: Arc<UpdateAdmission>,
}

#[derive(Default)]
pub(super) struct UpdateAdmission {
    paused: AtomicBool,
    generation: AtomicU64,
}

impl UpdateAdmission {
    pub(super) fn paused(&self) -> bool { self.paused.load(Ordering::Acquire) }
    pub(super) fn generation(&self) -> u64 { self.generation.load(Ordering::Acquire) }
    pub(super) fn owns_connection_stop(&self, connected_generation: u64) -> bool {
        self.paused() || self.generation() != connected_generation
    }

    fn release_after_resume(&self, enqueue_resume: impl FnOnce()) {
        // A subsequent prepare can start as soon as paused becomes false. Its
        // PrepareUpdate must never overtake the previous owner's queued resume.
        enqueue_resume();
        self.paused.store(false, Ordering::Release);
    }
}

impl Recovery {
    fn resume(self, reply: Option<oneshot::Sender<()>>) {
        // This only releases the IPC reconnect pause. The actor independently
        // revokes key intent and waits for native capture teardown before it can
        // accept another recording.
        self.admission.release_after_resume(|| {
            let _ = self.urgent.send(Message::ResumeAfterUpdate(reply));
        });
    }
}

impl UpdatePause {
    pub(super) fn begin(
        urgent: mpsc::UnboundedSender<Message>,
        admission: Arc<UpdateAdmission>,
    ) -> Result<(Self, oneshot::Receiver<Result<(), String>>), String> {
        if admission.paused.swap(true, Ordering::AcqRel) {
            return Err("键盘更新预检已在进行".into());
        }
        admission.generation.fetch_add(1, Ordering::AcqRel);
        let guard = Self { recovery: Some(Recovery { urgent, admission }) };
        let (reply, result) = oneshot::channel();
        guard.recovery.as_ref().unwrap().urgent.send(Message::PrepareUpdate(reply))
            .map_err(|_| "字幕服务已停止")?;
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
            recovery.resume(Some(reply));
            tokio::time::timeout(std::time::Duration::from_secs(3), result)
                .await.map_err(|_| "等待键盘恢复超时")?
                .map_err(|_| "键盘恢复通道关闭")?;
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

    #[tokio::test]
    async fn dropping_a_cancelled_preflight_restores_reconnect_and_queues_actor_recovery() {
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let paused = Arc::new(UpdateAdmission::default());
        let (guard, result) = UpdatePause::begin(urgent, paused.clone()).unwrap();
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
        let (guard, result) = UpdatePause::begin(urgent.clone(), paused.clone()).unwrap();
        let Message::PrepareUpdate(reply) = messages.recv().await.unwrap() else { panic!("prepare first") };
        reply.send(Err("native owner has not stopped".into())).unwrap();
        assert!(result.await.unwrap().is_err());
        let cancelled = tokio::spawn(guard.cancel());
        let Message::ResumeAfterUpdate(Some(reply)) = messages.recv().await.unwrap() else { panic!("resume expected") };
        reply.send(()).unwrap();
        cancelled.await.unwrap().unwrap();
        assert!(!paused.paused());
        let (retry, _) = UpdatePause::begin(urgent, paused.clone()).unwrap();
        drop(retry);
        assert!(!paused.paused());
    }

    #[tokio::test]
    async fn committed_handoff_keeps_admission_paused_until_process_replacement() {
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let paused = Arc::new(UpdateAdmission::default());
        let (guard, result) = UpdatePause::begin(urgent.clone(), paused.clone()).unwrap();
        assert!(UpdatePause::begin(urgent, paused.clone()).is_err());
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
    fn a_late_stop_from_the_previous_connection_cannot_exit_the_recovered_app() {
        let admission = Arc::new(UpdateAdmission::default());
        let connected_generation = admission.generation();
        let (urgent, _messages) = mpsc::unbounded_channel();
        let (pause, _) = UpdatePause::begin(urgent, admission.clone()).unwrap();
        drop(pause);
        assert!(!admission.paused());
        assert!(admission.owns_connection_stop(connected_generation));
        assert!(!admission.owns_connection_stop(admission.generation()));
    }

    #[test]
    fn a_new_prepare_cannot_overtake_the_previous_owners_resume_publication() {
        let admission = Arc::new(UpdateAdmission::default());
        let (urgent, mut messages) = mpsc::unbounded_channel();
        let (mut old, _) = UpdatePause::begin(urgent.clone(), admission.clone()).unwrap();
        assert!(matches!(messages.try_recv(), Ok(Message::PrepareUpdate(_))));
        let recovery = old.recovery.take().unwrap();
        // Attempt the competing begin exactly while resume is being published,
        // rather than relying on scheduler timing to reproduce the old race.
        recovery.admission.release_after_resume(|| {
            assert!(UpdatePause::begin(urgent.clone(), admission.clone()).is_err());
            recovery.urgent.send(Message::ResumeAfterUpdate(None)).unwrap();
        });
        let (next, _) = UpdatePause::begin(urgent, admission).unwrap();
        assert!(matches!(messages.try_recv(), Ok(Message::ResumeAfterUpdate(None))));
        assert!(matches!(messages.try_recv(), Ok(Message::PrepareUpdate(_))));
        next.commit();
    }
}
