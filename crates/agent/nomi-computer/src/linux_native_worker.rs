//! Cancellation ownership for a blocking desktop worker. Dropping the caller
//! cancels future effects and joins that exact worker's cleanup before returning.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::{Receiver, SyncSender}};

pub(crate) struct Completion(pub SyncSender<()>);
impl Drop for Completion {
    fn drop(&mut self) { let _ = self.0.send(()); }
}

pub(crate) struct CancelAndJoin {
    cancellation: Arc<AtomicBool>,
    completion: Option<Receiver<()>>,
}
impl CancelAndJoin {
    pub(crate) fn new(cancellation: Arc<AtomicBool>, completion: Receiver<()>) -> Self {
        Self { cancellation, completion: Some(completion) }
    }
    pub(crate) fn disarm(&mut self) { self.completion = None; }
}
impl Drop for CancelAndJoin {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            self.cancellation.store(true, Ordering::Release);
            // The worker and the portal's permanent native runtime are
            // independent of the dropped caller future. A missing completion
            // sender cannot manufacture cleanup proof or a successful result.
            let _ = completion.recv();
        }
    }
}
