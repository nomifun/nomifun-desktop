//! Native shutdown admission and physical completion are different facts.
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Default)]
pub(super) struct ShutdownState(AtomicU8);

impl ShutdownState {
    pub(super) fn begin(&self) -> bool {
        self.0.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub(super) fn finish(&self) {
        self.0.store(2, Ordering::Release);
    }

    pub(super) fn completed(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }

    pub(super) fn blocks_work(&self) -> bool {
        self.0.load(Ordering::Acquire) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_entry_is_not_completion_and_cannot_be_entered_twice() {
        let state = ShutdownState::default();
        assert!(!state.blocks_work());
        assert!(state.begin());
        assert!(state.blocks_work());
        assert!(!state.completed(), "entering the native shutdown must not publish cleanup proof");
        assert!(!state.begin());
        state.finish();
        assert!(state.completed());
        assert!(state.blocks_work());
        assert!(!state.begin());
    }

    #[test]
    fn another_waiter_cannot_claim_success_while_native_cleanup_is_held() {
        let state = std::sync::Arc::new(ShutdownState::default());
        let (entered, started) = std::sync::mpsc::channel();
        let (release, held) = std::sync::mpsc::channel();
        let owner = state.clone();
        let native = std::thread::spawn(move || {
            assert!(owner.begin());
            entered.send(()).unwrap();
            held.recv().unwrap();
            owner.finish();
        });
        started.recv().unwrap();
        assert!(state.blocks_work());
        assert!(!state.completed());
        assert!(!state.begin(), "a retry must not enter the native shutdown again");
        release.send(()).unwrap();
        native.join().unwrap();
        assert!(state.completed());
    }
}
