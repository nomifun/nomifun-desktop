//! Shared ownership of admitted effects. Dropping a caller or cleanup waiter
//! does not abort an effect or consume the only task-completion witness.
use std::future::Future;
use std::sync::Mutex;

use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_common::AppError;
use tokio::sync::oneshot;

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;
struct State {
    tasks: Vec<Completion>,
    closed: bool,
    failed: bool,
}

/// This owns tasks, not authorization or receipts. The platform must durably
/// admit each operation first and record its result inside the owned future,
/// before returning that result to the engine. Process-tree exit remains the
/// actual process owner's responsibility; joining a dispatch is not enough.
pub struct EngineTaskGroup {
    state: Mutex<State>,
    max_pending: usize,
}

pub struct EngineOwnedTask<T> {
    result: oneshot::Receiver<T>,
}
impl<T> EngineOwnedTask<T> {
    pub async fn result(self) -> Result<T, AppError> {
        self.result.await.map_err(|_| failure())
    }
}

fn failure() -> AppError {
    AppError::Conflict("Engine-owned task failed; cleanup cannot be proven".into())
}

impl EngineTaskGroup {
    pub fn new(max_pending: usize) -> Result<Self, AppError> {
        if !(1..=4096).contains(&max_pending) {
            return Err(AppError::BadRequest(
                "Engine task bound must be 1..4096".into(),
            ));
        }
        Ok(Self {
            state: Mutex::new(State {
                tasks: Vec::new(),
                closed: false,
                failed: false,
            }),
            max_pending,
        })
    }

    pub fn spawn<T, F>(&self, operation: F) -> Result<EngineOwnedTask<T>, AppError>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let mut state = self.state.lock().map_err(|_| failure())?;
        reap_completed(&mut state);
        if state.failed {
            return Err(failure());
        }
        if state.closed {
            return Err(AppError::Conflict("Engine task admission is closed".into()));
        }
        if state.tasks.len() >= self.max_pending {
            return Err(AppError::Conflict(
                "Engine pending task bound reached".into(),
            ));
        }
        let executor = tokio::runtime::Handle::try_current().map_err(|_| {
            AppError::Internal("Engine tasks require the async host executor".into())
        })?;
        let (sender, result) = oneshot::channel();
        let task = executor.spawn(async move {
            let result = operation.await;
            let _ = sender.send(result);
        });
        // No await between spawn and registration. The caller gets only its
        // result channel, never an abort handle for the admitted operation.
        state.tasks.push(
            async move { task.await.map_err(|error| error.to_string()) }
                .boxed()
                .shared(),
        );
        Ok(EngineOwnedTask { result })
    }

    pub fn close(&self) -> Result<(), AppError> {
        self.state.lock().map_err(|_| failure())?.closed = true;
        Ok(())
    }

    /// A point-in-time count, not a process-tree or cleanup proof.
    pub fn is_quiescent(&self) -> Result<bool, AppError> {
        let mut state = self.state.lock().map_err(|_| failure())?;
        reap_completed(&mut state);
        if state.failed {
            return Err(failure());
        }
        Ok(state.tasks.is_empty())
    }

    /// A point-in-time count, not a process-tree or cleanup proof.
    pub fn pending_tasks(&self) -> Result<usize, AppError> {
        Ok(self.state.lock().map_err(|_| failure())?.tasks.len())
    }

    /// Reusable barrier. The host must first fence producers at its turn or
    /// activation boundary. For final release prefer close_and_join().
    pub async fn join(&self) -> Result<(), AppError> {
        loop {
            let tasks = {
                let mut state = self.state.lock().map_err(|_| failure())?;
                reap_completed(&mut state);
                if state.tasks.is_empty() {
                    return if state.failed { Err(failure()) } else { Ok(()) };
                }
                state.tasks.clone()
            };
            // Keep witnesses in state throughout awaiting, including errors.
            // A panic in one task must not skip waiting for all other effects.
            for task in tasks {
                let _ = task.await;
            }
        }
    }

    pub async fn close_and_join(&self) -> Result<(), AppError> {
        self.close()?;
        self.join().await
    }
}

fn reap_completed(state: &mut State) {
    let mut failed = state.failed;
    state
        .tasks
        .retain(|task| match task.clone().now_or_never() {
            None => true,
            Some(Ok(())) => false,
            Some(Err(_)) => {
                failed = true;
                false
            }
        });
    state.failed = failed;
    if failed {
        state.closed = true;
    }
}
