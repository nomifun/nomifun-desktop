//! Ownership for the source jobs that outlive their originating requests.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use nomifun_common::AppError;
use tokio::sync::{Mutex as AsyncMutex, OwnedRwLockReadGuard, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

type RetainedTask = Arc<AsyncMutex<JoinHandle<()>>>;

#[derive(Default)]
struct State {
    closed: bool,
    tasks: Vec<RetainedTask>,
    unknown_completion: Option<String>,
}

fn lock_state(state: &Mutex<State>) -> MutexGuard<'_, State> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.closed = true;
            guard.unknown_completion.get_or_insert_with(|| {
                "knowledge background owner state was poisoned; task completion is unknown".into()
            });
            guard
        }
    }
}

fn completion_error(error: tokio::task::JoinError) -> String {
    format!("knowledge background task completion is unknown: {error}")
}

#[derive(Clone)]
struct Context {
    cancellation: CancellationToken,
    publication: Arc<RwLock<()>>,
    state: Arc<Mutex<State>>,
}

tokio::task_local! {
    static CONTEXT: Context;
}

#[derive(Default)]
pub(super) struct BackgroundTasks {
    state: Arc<Mutex<State>>,
    cancellation: CancellationToken,
    publication: Arc<RwLock<()>>,
    drain: AsyncMutex<()>,
}

fn stopped() -> AppError {
    AppError::Conflict(
        "knowledge background work is shutting down; source fetching can resume on the next start"
            .into(),
    )
}

impl BackgroundTasks {
    /// Admission and handle retention happen together, before any row can be
    /// inserted. Dropping a request or a drain caller never drops this handle.
    pub(super) fn spawn(
        &self,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), AppError> {
        let mut state = lock_state(&self.state);
        if state.closed {
            return Err(stopped());
        }
        // Reap only actual joined completions. Failed joins remain sticky
        // evidence after their finished handles are removed.
        let mut failed_join = None;
        state.tasks.retain(|task| {
            let Ok(mut handle) = task.try_lock() else {
                return true;
            };
            if !handle.is_finished() {
                return true;
            }
            match (&mut *handle).now_or_never() {
                None => true,
                Some(result) => {
                    if let Err(error) = result {
                        failed_join.get_or_insert_with(|| completion_error(error));
                    }
                    false
                }
            }
        });
        if let Some(error) = failed_join {
            state.unknown_completion.get_or_insert(error);
        }
        let context = Context {
            cancellation: self.cancellation.clone(),
            publication: Arc::clone(&self.publication),
            state: Arc::clone(&self.state),
        };
        state.tasks.push(Arc::new(AsyncMutex::new(tokio::spawn(
            CONTEXT.scope(context, future),
        ))));
        Ok(())
    }

    pub(super) async fn quiesce(&self, timeout: Duration) -> Result<(), String> {
        {
            let mut state = lock_state(&self.state);
            state.closed = true;
            self.cancellation.cancel();
        }
        tokio::time::timeout(timeout, async {
            let _drain = self.drain.lock().await;
            // A publication admitted before cancellation may finish. Taking
            // this writer establishes the fence after those writes finish;
            // new publication readers check cancellation before proceeding.
            let fence = self.publication.write().await;
            drop(fence);
            loop {
                let task = lock_state(&self.state).tasks.first().cloned();
                let Some(task) = task else { break };
                let result = (&mut *task.lock().await).await;
                let mut state = lock_state(&self.state);
                state.tasks.retain(|retained| !Arc::ptr_eq(retained, &task));
                if let Err(error) = result {
                    state.unknown_completion.get_or_insert_with(|| completion_error(error));
                }
            }
            match &lock_state(&self.state).unknown_completion {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }).await.map_err(|_| {
            "knowledge background work has not quiesced before the deadline; retained tasks must be joined before closing the database".to_owned()
        })?
    }
}

pub(super) fn ensure_open() -> Result<(), AppError> {
    if CONTEXT
        .try_with(|context| context.cancellation.is_cancelled())
        .unwrap_or(false)
    {
        Err(stopped())
    } else {
        Ok(())
    }
}

pub(crate) fn record_join_failure(error: &tokio::task::JoinError) {
    let _ = CONTEXT.try_with(|context| {
        lock_state(&context.state)
            .unknown_completion
            .get_or_insert_with(|| format!("knowledge local task completion is unknown: {error}"));
    });
}

pub(super) async fn publication_guard() -> Result<Option<OwnedRwLockReadGuard<()>>, AppError> {
    let context = CONTEXT.try_with(Clone::clone).ok();
    let Some(context) = context else {
        return Ok(None);
    };
    ensure_open()?;
    let guard = context.publication.read_owned().await;
    ensure_open()?;
    Ok(Some(guard))
}

/// Foreground callers retain their normal response budget. Owned source jobs
/// must finish local workers; their shutdown deadline belongs to the owner,
/// whose retained JoinHandle survives timeout or caller cancellation.
/// The caller's quiesce budget (for example five seconds) still reports an
/// incomplete drain while a local worker remains held beyond its response
/// budget; this wait does not supply earlier proof of worker completion.
pub(super) async fn local_io_timeout<F: Future>(
    budget: Duration,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    if CONTEXT.try_with(|_| ()).is_ok() {
        Ok(future.await)
    } else {
        tokio::time::timeout(budget, future).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panicked_task_never_becomes_successful_quiescence_on_retry() {
        let owner = BackgroundTasks::default();
        owner
            .spawn(async { panic!("injected source task panic") })
            .unwrap();
        loop {
            let finished = lock_state(&owner.state).tasks[0]
                .try_lock()
                .unwrap()
                .is_finished();
            if finished {
                break;
            }
            tokio::task::yield_now().await;
        }
        owner.spawn(async {}).unwrap();
        assert_eq!(
            lock_state(&owner.state).tasks.len(),
            1,
            "finished handle must be actually joined and reaped"
        );
        for _ in 0..2 {
            let error = owner.quiesce(Duration::from_secs(1)).await.unwrap_err();
            assert!(error.contains("completion is unknown"), "{error}");
        }
    }

    #[tokio::test]
    async fn poisoned_owner_state_cannot_certify_quiescence() {
        let owner = BackgroundTasks::default();
        let _ = std::panic::catch_unwind(|| {
            let _state = owner.state.lock().unwrap();
            panic!("injected owner-state poison");
        });
        assert!(owner.spawn(async {}).is_err());
        for _ in 0..2 {
            let error = owner.quiesce(Duration::from_secs(1)).await.unwrap_err();
            assert!(error.contains("poisoned"), "{error}");
        }
    }

    #[tokio::test]
    async fn owned_local_worker_survives_its_response_budget_and_drain_cancellation() {
        let owner = BackgroundTasks::default();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let entered = Arc::new(tokio::sync::Notify::new());
        let worker_entered = Arc::clone(&entered);
        let (release, held) = std::sync::mpsc::channel();
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        owner
            .spawn(async move {
                let result = super::super::bounded_root_blocking(
                    &root,
                    Duration::from_millis(1),
                    false,
                    move || {
                        worker_entered.notify_one();
                        held.recv().unwrap();
                        worker_finished.store(true, std::sync::atomic::Ordering::SeqCst);
                        true
                    },
                )
                .await;
                assert!(result, "owned inspection must await its actual worker");
            })
            .unwrap();
        entered.notified().await;
        let mut drain = Box::pin(owner.quiesce(Duration::from_secs(5)));
        assert!(futures_util::poll!(&mut drain).is_pending());
        drop(drain);
        assert!(owner.quiesce(Duration::from_millis(5)).await.is_err());
        assert!(!finished.load(std::sync::atomic::Ordering::SeqCst));
        release.send(()).unwrap();
        owner.quiesce(Duration::from_secs(5)).await.unwrap();
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
    }
}
