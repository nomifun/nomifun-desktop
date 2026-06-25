//! Pre-tracing boot notes.
//!
//! The desktop shell resolves (and possibly relocates) the data dir BEFORE
//! the embedded backend exists, and tracing is only initialized by the
//! backend (`init_environment` → `init_tracing`). Anything the relocation
//! wants to report at that point can only go to stderr, which is invisible
//! in packaged Windows builds (`windows_subsystem = "windows"`).
//!
//! The shell records its outcome here; [`super::environment::init_environment`]
//! drains the queue right after `init_tracing`, so the notes land in the
//! persistent log files under `{data_dir}/logs/` — the earliest point where
//! a structured record is possible.

use std::sync::Mutex;

/// Severity of a pre-tracing boot note. Deliberately minimal: boot notes are
/// rare one-liners, not a general logging facade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootNoteLevel {
    Info,
    Warn,
}

static BOOT_NOTES: Mutex<Vec<(BootNoteLevel, String)>> = Mutex::new(Vec::new());

/// Queue a message produced before tracing is up. Safe from any thread. A
/// poisoned queue (a recorder panicked mid-push) just drops the note — boot
/// notes are diagnostics and never worth propagating a failure for.
pub fn record_boot_note(level: BootNoteLevel, message: impl Into<String>) {
    if let Ok(mut notes) = BOOT_NOTES.lock() {
        notes.push((level, message.into()));
    }
}

/// Drain all queued notes in FIFO order. Called once per process by
/// `init_environment` right after tracing initialization.
pub(crate) fn drain_boot_notes() -> Vec<(BootNoteLevel, String)> {
    BOOT_NOTES
        .lock()
        .map(|mut notes| std::mem::take(&mut *notes))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_drain_is_fifo_and_empties_the_queue() {
        // Single test touching the static queue in this binary, so no
        // cross-test interference under the parallel runner.
        record_boot_note(BootNoteLevel::Info, "first");
        record_boot_note(BootNoteLevel::Warn, String::from("second"));

        let drained = drain_boot_notes();
        assert_eq!(
            drained,
            vec![
                (BootNoteLevel::Info, "first".to_string()),
                (BootNoteLevel::Warn, "second".to_string()),
            ]
        );
        assert!(drain_boot_notes().is_empty());
    }
}
