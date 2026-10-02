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

/// Debug native acceptance may lose a completion acknowledgement, but only
/// for an explicitly marked, isolated data root. Never read this in release.
#[cfg(debug_assertions)]
pub(super) fn lose_completion_ack(root: &std::path::Path) -> bool {
    let Ok(key) = std::env::var("NOMIFUN_RELIABILITY_CEF_ACK_LOSS_KEY") else { return false; };
    acknowledgement_scope_matches(root, &key)
}

#[cfg(debug_assertions)]
fn acknowledgement_scope_matches(root: &std::path::Path, key: &str) -> bool {
    let Ok(id) = uuid::Uuid::parse_str(key) else { return false; };
    if id.to_string() != key { return false; }
    let Some(data) = root.parent().filter(|path| path.file_name().is_some_and(|name| name == "browser-v3"))
        .and_then(std::path::Path::parent) else { return false; };
    root.file_name().is_some_and(|name| name == "agent-sessions")
        && std::fs::read(data.join(".reliability-cef-ack-loss")).is_ok_and(|bytes| bytes == key.as_bytes())
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

    #[test]
    #[cfg(debug_assertions)]
    fn native_ack_loss_requires_an_exact_isolated_root_marker() {
        let data = tempfile::tempdir().unwrap();
        let root = data.path().join("browser-v3/agent-sessions");
        let key = "01a0fbba-4472-7c02-a40f-1c1a3104669d";
        assert!(!acknowledgement_scope_matches(&root, key));
        std::fs::write(data.path().join(".reliability-cef-ack-loss"), key).unwrap();
        assert!(acknowledgement_scope_matches(&root, key));
        assert!(!acknowledgement_scope_matches(&root, "01a0fbba-4472-7c02-a40f-1c1a3104669e"));
        assert!(!acknowledgement_scope_matches(&root, "not-a-key"));
        assert!(!acknowledgement_scope_matches(&root, &key.to_uppercase()));
        assert!(!acknowledgement_scope_matches(&data.path().join("other/agent-sessions"), key));
        assert!(!acknowledgement_scope_matches(&data.path().join("browser-v3/Default"), key));
        std::fs::write(data.path().join(".reliability-cef-ack-loss"), format!("{key}\n")).unwrap();
        assert!(!acknowledgement_scope_matches(&root, key));
    }
}
