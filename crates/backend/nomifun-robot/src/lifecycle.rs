//! Retained witnesses for Robot-owned tasks, including session children.
//! Aborting is a request; only joining the same task proves local completion.
use std::{future::Future, sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, Ordering}}};

use futures_util::{FutureExt, future::{BoxFuture, Shared}};

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;

#[derive(Clone)]
pub(crate) struct OwnedTask {
    completion: Completion,
    abort: tokio::task::AbortHandle,
    abort_requested: Arc<AtomicBool>,
}

impl OwnedTask {
    pub(crate) fn abort(&self) {
        self.abort_requested.store(true, Ordering::Release);
        self.abort.abort();
    }

    pub(crate) async fn join(&self) -> Result<(), String> {
        self.completion.clone().await
    }
}

#[derive(Default)]
struct State {
    tasks: Vec<OwnedTask>,
    errors: Vec<String>,
    closing: bool,
}

#[derive(Default)]
pub(crate) struct OwnedTasks(Mutex<State>);

impl OwnedTasks {
    fn state(&self) -> MutexGuard<'_, State> {
        match self.0.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.errors.push("Robot task inventory lock was poisoned".into());
                state
            }
        }
    }
    pub(crate) fn track(&self, handle: tokio::task::JoinHandle<()>) -> OwnedTask {
        let abort = handle.abort_handle();
        let abort_requested = Arc::new(AtomicBool::new(false));
        let expected_abort = abort_requested.clone();
        let task = OwnedTask {
            completion: async move {
                match handle.await {
                    Ok(()) => Ok(()),
                    Err(error) if error.is_cancelled() && expected_abort.load(Ordering::Acquire) => Ok(()),
                    Err(error) => Err(format!("Robot-owned task ended abnormally: {error}")),
                }
            }.boxed().shared(),
            abort,
            abort_requested,
        };
        let mut state = self.state();
        reap_completed(&mut state);
        if state.closing { task.abort(); }
        state.tasks.push(task.clone());
        task
    }

    pub(crate) fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) -> OwnedTask {
        self.track(tokio::spawn(work))
    }

    pub(crate) fn record_error(&self, error: String) {
        self.state().errors.push(error);
    }

    #[cfg(test)]
    pub(crate) fn task_count(&self) -> usize { self.state().tasks.len() }

    fn retain(&self, task: OwnedTask) {
        let mut state = self.state();
        reap_completed(&mut state);
        if state.closing { task.abort(); }
        state.tasks.push(task);
    }

    /// Called only after session ingress is fenced. Retaining the shared
    /// witnesses makes cancellation of this waiter safe for an explicit retry.
    pub(crate) async fn abort_and_join(&self) -> Result<(), String> {
        loop {
            let tasks = {
                let mut state = self.state();
                state.closing = true;
                reap_completed(&mut state);
                if state.tasks.is_empty() {
                    return if state.errors.is_empty() { Ok(()) } else { Err(state.errors.join("; ")) };
                }
                for task in &state.tasks { task.abort(); }
                state.tasks.clone()
            };
            for task in tasks { let _ = task.join().await; }
        }
    }
}

/// A session joins its producers before dropping the last writer sender.
/// The gateway retains the same witnesses if that session itself panics.
pub(crate) struct TaskScope { local: OwnedTasks, parent: Arc<OwnedTasks> }
impl TaskScope {
    pub(crate) fn new(parent: Arc<OwnedTasks>) -> Self { Self { local: Default::default(), parent } }
    pub(crate) fn track(&self, handle: tokio::task::JoinHandle<()>) -> OwnedTask {
        let task = self.local.track(handle);
        self.parent.retain(task.clone());
        task
    }
    pub(crate) fn spawn(&self, work: impl Future<Output=()> + Send + 'static) -> OwnedTask {
        self.track(tokio::spawn(work))
    }
    pub(crate) async fn abort_and_join(&self) -> Result<(), String> { self.local.abort_and_join().await }
}

fn reap_completed(state: &mut State) {
    let State { tasks, errors, .. } = state;
    tasks.retain(|task| match task.completion.clone().now_or_never() {
        None => true,
        Some(Ok(())) => false,
        Some(Err(error)) => { errors.push(error); false }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }

    #[tokio::test]
    async fn abort_join_preserves_replaced_children_and_waiter_cancellation() {
        let tasks = Arc::new(OwnedTasks::default());
        let dropped = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(tokio::sync::Notify::new());
        let seen = entered.clone();
        let child = tasks.spawn({ let dropped = dropped.clone(); async move {
            let _guard = Dropped(dropped);
            seen.notify_one();
            std::future::pending::<()>().await;
        }});
        entered.notified().await;
        child.abort();
        drop(child); // A replaced turn's caller no longer retains its handle.
        tasks.abort_and_join().await.unwrap();
        assert!(dropped.load(Ordering::Acquire));
        tasks.abort_and_join().await.unwrap();
    }

    #[tokio::test]
    async fn owned_task_panic_is_sticky_after_all_other_children_join() {
        let tasks = OwnedTasks::default();
        let panic = tasks.spawn(async { panic!("controlled Robot child panic") });
        assert!(panic.join().await.is_err());
        assert!(tasks.abort_and_join().await.is_err());
        assert!(tasks.abort_and_join().await.is_err());
    }
}
