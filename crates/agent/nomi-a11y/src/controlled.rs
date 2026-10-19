//! Cancellation of one command in an existing native actor queue. This slot
//! contains no native handle and never replaces the actor's result owner.

use std::sync::{Arc, atomic::{AtomicU8, Ordering}, mpsc::{Receiver, RecvTimeoutError}};
use tokio_util::sync::CancellationToken;
use crate::A11yError;

const QUEUED: u8 = 0;
const DISPATCHED: u8 = 1;
const WITHDRAWN: u8 = 2;

#[derive(Clone)]
pub(crate) struct MutationAdmission {
    cancellation: CancellationToken,
    phase: Arc<AtomicU8>,
}

impl MutationAdmission {
    pub(crate) fn new(cancellation: CancellationToken) -> Self {
        Self { cancellation, phase: Arc::new(AtomicU8::new(QUEUED)) }
    }

    /// Called on the original actor immediately before native work. A caller
    /// may withdraw only a queued command; it cannot abandon a dispatched one.
    pub(crate) fn begin(&self) -> bool {
        if self.cancellation.is_cancelled() {
            let _ = self.phase.compare_exchange(QUEUED, WITHDRAWN, Ordering::AcqRel, Ordering::Acquire);
            return false;
        }
        if self.phase.compare_exchange(QUEUED, DISPATCHED, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        !self.cancellation.is_cancelled()
    }

    pub(crate) fn receive<T>(&self, reply: Receiver<Result<T, A11yError>>,
        lost_reply: impl Fn() -> A11yError) -> Result<T, A11yError> {
        loop {
            if self.cancellation.is_cancelled() && self.phase.compare_exchange(
                QUEUED, WITHDRAWN, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return Err(A11yError::CancelledBeforeDispatch);
            }
            match reply.recv_timeout(std::time::Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => {},
                Err(RecvTimeoutError::Disconnected) => return Err(lost_reply()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn cancelled_actor_queue_returns_without_waiting_or_admitting_late_native_input() {
        let cancellation = CancellationToken::new();
        let admission = MutationAdmission::new(cancellation.clone());
        let queued = admission.clone();
        let (_sender, reply) = mpsc::channel::<Result<(), A11yError>>();
        cancellation.cancel();
        assert!(matches!(admission.receive(reply, || panic!("reply still owned")), Err(A11yError::CancelledBeforeDispatch)));
        assert!(!queued.begin(), "actor recovery cannot dispatch the withdrawn command");
    }

    #[test]
    fn cancelled_dispatched_actor_command_retains_the_original_native_receipt() {
        let cancellation = CancellationToken::new();
        let admission = MutationAdmission::new(cancellation.clone());
        assert!(admission.begin());
        cancellation.cancel();
        let (sender, reply) = mpsc::channel();
        let receiver = std::thread::spawn(move || admission.receive(reply, || panic!("original native owner lost")));
        assert!(!receiver.is_finished());
        sender.send(Ok(19)).unwrap();
        assert_eq!(receiver.join().unwrap().unwrap(), 19);
    }
}
