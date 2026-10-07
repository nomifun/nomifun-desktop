use std::future::Future;
use std::time::Duration;

use nomifun_common::AppError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared lifecycle for every background task owned by one Agent Execution
/// engine composition.
#[derive(Clone, Default)]
pub struct AgentExecutionLifecycle {
    cancellation: CancellationToken,
    tasks: TaskTracker,
}

impl AgentExecutionLifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub(crate) async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }

    pub(crate) fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.tasks.spawn(future)
    }

    /// Stop new work, wake every tracked task, and wait for bounded cleanup.
    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.cancellation.cancel();
        self.tasks.close();
        tokio::time::timeout(SHUTDOWN_TIMEOUT, self.tasks.wait())
            .await
            .map_err(|_| {
                AppError::Timeout(format!(
                    "Agent Execution background shutdown exceeded {} seconds",
                    SHUTDOWN_TIMEOUT.as_secs()
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[tokio::test]
    async fn shutdown_cancels_and_joins_tracked_tasks() {
        let lifecycle = AgentExecutionLifecycle::new();
        let stopped = Arc::new(AtomicBool::new(false));
        let task_stopped = Arc::clone(&stopped);
        let task_lifecycle = lifecycle.clone();
        lifecycle.spawn(async move {
            task_lifecycle.cancelled().await;
            task_stopped.store(true, Ordering::Release);
        });

        lifecycle.shutdown().await.unwrap();
        assert!(stopped.load(Ordering::Acquire));
        lifecycle.shutdown().await.unwrap();
    }
}
