use std::sync::Arc;
use std::time::Duration;

use nomifun_common::AgentKillReason;
use tracing::{debug, info};

use crate::task_manager::IWorkerTaskManager;

/// Default idle timeout for ACP agents (5 minutes).
const DEFAULT_IDLE_TIMEOUT_SECS: i64 = 5 * 60;

/// Scan interval for idle agent cleanup (1 minute).
const SCAN_INTERVAL_SECS: u64 = 60;

/// Start the background idle agent scanner.
///
/// Periodically scans active tasks and kills ACP agents that have been
/// idle (finished + no activity) beyond the configured threshold.
///
/// The scanner runs until the provided `shutdown` signal resolves.
pub fn start_idle_scanner(
    worker_task_manager: Arc<dyn IWorkerTaskManager>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    idle_timeout_secs: Option<i64>,
    scan_interval_secs: Option<u64>,
) -> tokio::task::JoinHandle<()> {
    let threshold = idle_timeout_secs.unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS);
    let scan_interval = scan_interval_secs.unwrap_or(SCAN_INTERVAL_SECS);
    info!(
        threshold_secs = threshold,
        scan_interval_secs = scan_interval,
        "Starting idle agent scanner"
    );

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(scan_interval));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    scan_and_cleanup(&worker_task_manager, threshold*1000);
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("Idle scanner received shutdown signal");
                        break;
                    }
                }
            }
        }

        info!("Idle scanner stopped");
    })
}

/// Perform one scan: find idle tasks and kill them.
fn scan_and_cleanup(manager: &Arc<dyn IWorkerTaskManager>, threshold_ms: i64) {
    let idle_ids = manager.collect_idle(threshold_ms);

    if idle_ids.is_empty() {
        debug!(active = manager.active_count(), "Idle scan: no idle agents found");
        return;
    }

    info!(count = idle_ids.len(), "Idle scan: cleaning up idle agents");

    for id in idle_ids {
        let manager = Arc::clone(manager);
        tokio::spawn(async move {
            info!(conversation_id = %id, "Idle scan: awaiting idle agent shutdown");
            manager.kill_and_wait(&id, Some(AgentKillReason::IdleTimeout)).await;
        });
    }
}
