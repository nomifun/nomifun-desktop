use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, error, info, warn};

use crate::constants::DEBOUNCE_MS;
use crate::registry::ExtensionRegistry;

// ---------------------------------------------------------------------------
// ExtensionWatcher
// ---------------------------------------------------------------------------

/// Watches extension directories for file changes and triggers a debounced
/// hot-reload of the [`ExtensionRegistry`].
///
/// Uses the `notify` crate for cross-platform file system event monitoring
/// and a custom debounce mechanism (1000ms) to collapse rapid changes into
/// a single reload.
pub struct ExtensionWatcher {
    /// Handle to the notify watcher — kept alive to maintain the watch.
    _watcher: RecommendedWatcher,
    /// Signal to request a graceful shutdown of the debounce task.
    shutdown: Arc<Notify>,
}

impl ExtensionWatcher {
    /// Start watching the given directories for changes.
    ///
    /// File change events are debounced by [`DEBOUNCE_MS`] milliseconds
    /// before triggering `registry.hot_reload()`.
    ///
    /// Returns `None` if no valid directories are provided or if the watcher
    /// fails to initialise.
    pub fn start(directories: Vec<PathBuf>, registry: ExtensionRegistry) -> Option<Self> {
        if directories.is_empty() {
            debug!("no directories to watch — skipping extension watcher");
            return None;
        }

        let shutdown = Arc::new(Notify::new());

        // Channel for raw FS events → debounce task.
        let (tx, rx) = mpsc::channel::<()>(16);

        // Spawn the debounce consumer.
        let shutdown_clone = Arc::clone(&shutdown);
        tokio::spawn(debounce_loop(rx, registry, shutdown_clone));

        // Create the notify watcher with a callback that feeds the channel.
        let watcher = create_watcher(tx, &directories);
        let watcher = match watcher {
            Ok(w) => w,
            Err(e) => {
                error!(error = %e, "failed to create file watcher");
                return None;
            }
        };

        info!(dirs = directories.len(), "extension watcher started");

        Some(Self {
            _watcher: watcher,
            shutdown,
        })
    }

    /// Signal the debounce task to stop.
    ///
    /// The background task will finish its current cycle (if any) and exit.
    pub fn stop(&self) {
        self.shutdown.notify_one();
    }
}

impl Drop for ExtensionWatcher {
    fn drop(&mut self) {
        self.shutdown.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Internal: watcher creation
// ---------------------------------------------------------------------------

/// Create a `RecommendedWatcher` that sends a unit signal for every relevant
/// file-system event.
fn create_watcher(tx: mpsc::Sender<()>, directories: &[PathBuf]) -> Result<RecommendedWatcher, notify::Error> {
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            match result {
                Ok(event) if is_relevant_event(&event) => {
                    // Best-effort send — if the channel is full we'll coalesce
                    // anyway via debounce.
                    let _ = tx.try_send(());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, "file watcher error");
                }
            }
        },
        Config::default(),
    )?;

    for dir in directories {
        if dir.exists() {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                warn!(
                    dir = %dir.display(),
                    error = %e,
                    "failed to watch directory"
                );
            } else {
                debug!(dir = %dir.display(), "watching directory");
            }
        } else {
            debug!(dir = %dir.display(), "skipping non-existent directory");
        }
    }

    Ok(watcher)
}

/// Decide whether a file-system event should trigger a reload.
///
/// We care about creates, all modifications (data, metadata, renames), and
/// removes. Only access events and unclassified `Other` events are ignored.
///
/// Note: `Modify(_)` intentionally matches all modify sub-kinds including
/// metadata changes, because some platforms (e.g., macOS/FSEvents) report
/// content changes as generic `Modify(Any)` rather than specific sub-kinds.
fn is_relevant_event(event: &Event) -> bool {
    use notify::EventKind;
    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

// ---------------------------------------------------------------------------
// Internal: debounce loop
// ---------------------------------------------------------------------------

/// Consume raw FS event signals, debounce by [`DEBOUNCE_MS`], and trigger
/// `registry.hot_reload()`.
async fn debounce_loop(mut rx: mpsc::Receiver<()>, registry: ExtensionRegistry, shutdown: Arc<Notify>) {
    let debounce = Duration::from_millis(DEBOUNCE_MS);

    loop {
        tokio::select! {
            // Wait for the first event signal.
            event = rx.recv() => {
                if event.is_none() {
                    // Channel closed — sender (watcher) dropped.
                    debug!("watcher channel closed, stopping debounce loop");
                    break;
                }

                // Drain any additional events that arrived during debounce.
                tokio::time::sleep(debounce).await;
                while rx.try_recv().is_ok() {}

                info!("file change detected, triggering hot reload");
                registry.hot_reload().await;
            }
            // Shutdown signal.
            _ = shutdown.notified() => {
                debug!("watcher shutdown signal received");
                break;
            }
        }
    }

    debug!("debounce loop exited");
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevant_event_create() {
        let event = Event::new(notify::EventKind::Create(notify::event::CreateKind::File));
        assert!(is_relevant_event(&event));
    }

    #[test]
    fn relevant_event_modify() {
        let event = Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Content,
        )));
        assert!(is_relevant_event(&event));
    }

    #[test]
    fn relevant_event_remove() {
        let event = Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File));
        assert!(is_relevant_event(&event));
    }

    #[test]
    fn irrelevant_event_access() {
        let event = Event::new(notify::EventKind::Access(notify::event::AccessKind::Read));
        assert!(!is_relevant_event(&event));
    }

    #[test]
    fn irrelevant_event_other() {
        let event = Event::new(notify::EventKind::Other);
        assert!(!is_relevant_event(&event));
    }
}
