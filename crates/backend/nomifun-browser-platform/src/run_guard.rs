//! AgentSession Browser Resource input admission, driven by authoritative Agent runs.
//!
//! The host owns this coordinator; no renderer command may start or finish a run.
//! Dropping a caller's future never releases native input: operations and lifecycle
//! transitions finish in owned tasks, including their platform callback cleanup.

use std::{
    future::Future,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use futures_util::FutureExt;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunAdmissionError {
    #[error("The browser is already assigned to an Agent run.")]
    Busy,
    #[error("The browser operation belongs to an inactive Agent run.")]
    StaleRun,
    #[error("The browser operation was cancelled.")]
    Cancelled,
    #[error("Native browser input could not be safely changed.")]
    InputGateFailed,
    #[error("Browser input is locked until Agent cleanup completes.")]
    UserInputLocked,
    #[error("Browser lifecycle worker failed; input remains locked.")]
    WorkerFailed,
}

/// Platform implementations acknowledge only after every affected native view
/// has changed its input policy. Failure may leave input disabled, never enabled.
#[async_trait]
pub trait NativeInputGate: Send + Sync {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError>;
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError>;
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserInputState {
    UserReady,
    AgentRunning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserRunSnapshot {
    pub revision: u64,
    pub input_state: BrowserInputState,
    /// A failed gate remains locked and must be retried or the runtime closed.
    pub input_gate_failed: bool,
}

struct Run {
    cancelled: CancellationToken,
}

/// Unforgeable, in-process authority for one run on one Browser Resource. Even if a
/// caller reuses an external run ID, it cannot reuse a completed run's authority.
#[derive(Clone)]
pub struct BrowserRunGuard(Arc<RunAuthority>);

struct RunAuthority {
    run: Arc<Run>,
    coordinator: Weak<BrowserRunCoordinator>,
    release_on_drop: AtomicBool,
}

impl Drop for RunAuthority {
    fn drop(&mut self) {
        self.run.cancelled.cancel();
        if !self.release_on_drop.load(Ordering::Acquire) {
            return;
        }
        // Dropping the authoritative owner's last guard also settles input.
        // At runtime shutdown no task can be started; native host shutdown is
        // responsible for closing the views, and we never unlock synchronously.
        if let (Some(coordinator), Ok(runtime)) = (
            self.coordinator.upgrade(),
            tokio::runtime::Handle::try_current(),
        ) {
            let run = self.run.clone();
            runtime.spawn(async move {
                let _ = coordinator.finish_inner(&run).await;
            });
        }
    }
}

impl BrowserRunGuard {
    /// Agent lifecycle owners use explicit terminal proof. If their cleanup
    /// unwinds, cancellation remains active but native input stays locked.
    pub fn require_explicit_finish(&self) {
        self.0.release_on_drop.store(false, Ordering::Release);
    }
    pub fn cancel(&self) {
        self.0.run.cancelled.cancel();
    }
}

#[derive(Default)]
struct State {
    active: Option<Arc<Run>>,
    gate_failed: bool,
    revision: u64,
}

pub struct BrowserRunCoordinator {
    changes: crate::revision::BrowserRevision,
    gate: Arc<dyn NativeInputGate>,
    state: Mutex<State>,
    /// Run start/finish must not interleave their native gate transitions.
    transition: Mutex<()>,
    /// Observe, act, user commands and cleanup all serialize on the same gate.
    operation: Mutex<()>,
}

impl BrowserRunCoordinator {
    pub async fn has_active_run(&self)->bool {self.state.lock().await.active.is_some()}
    /// An explicit human close must never cancel or take ownership from a run.
    /// Keep the transition lock through the close, including native failure.
    pub async fn close_idle_runtime<T,F,Fut>(self: &Arc<Self>,close:F)->Result<T,RunAdmissionError>
    where T:Send+'static,F:FnOnce()->Fut+Send+'static,Fut:Future<Output=T>+Send+'static {
        let coordinator=self.clone();
        tokio::spawn(async move {
            if coordinator.state.lock().await.active.is_some() {return Err(RunAdmissionError::UserInputLocked);}
            let _transition=coordinator.transition.lock().await;
            if coordinator.state.lock().await.active.is_some() {return Err(RunAdmissionError::UserInputLocked);}
            let _operation=coordinator.operation.lock().await;
            {
                let mut state=coordinator.state.lock().await;
                if state.active.is_some() {return Err(RunAdmissionError::UserInputLocked);}
                // Also admit retries of a failed close. No active run exists,
                // and a failed gate must not make native destruction impossible.
                state.gate_failed=true;
                state.revision+=1;
                coordinator.changes.bump();
            }
            // Disable any surviving page during teardown. A gate failure must
            // not prevent an explicit attempt to destroy the native runtime.
            let _=coordinator.gate.lock_user_input().await;
            Ok(close().await)
        }).await.map_err(|_|RunAdmissionError::WorkerFailed)?
    }

    /// Host shutdown closes the runtime while retaining the run/input gate until
    /// native work settles. It never unlocks a closing browser for human use.
    pub async fn close_runtime<T, F, Fut>(
        self: &Arc<Self>,
        close: F,
    ) -> Result<T, RunAdmissionError>
    where
        T: Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        if let Some(run) = &self.state.lock().await.active {
            run.cancelled.cancel();
        }
        let coordinator = self.clone();
        tokio::spawn(async move {
            let _transition = coordinator.transition.lock().await;
            // A begin may have been awaiting native lock acknowledgement when
            // close was requested. Cancel the run it published before draining
            // its operations as well as the run visible at initial admission.
            if let Some(run) = &coordinator.state.lock().await.active {
                run.cancelled.cancel();
            }
            let _operation = coordinator.operation.lock().await;
            {
                let mut state = coordinator.state.lock().await;
                state.gate_failed = true;
                state.revision += 1;
                coordinator.changes.bump();
            }
            close().await
        })
        .await
        .map_err(|_| RunAdmissionError::WorkerFailed)
    }

    pub fn new(gate: Arc<dyn NativeInputGate>) -> Arc<Self> {
        Arc::new(Self {
            changes: Default::default(),
            gate,
            state: Mutex::new(State::default()),
            transition: Mutex::new(()),
            operation: Mutex::new(()),
        })
    }

    pub async fn snapshot(&self) -> BrowserRunSnapshot {
        let state = self.state.lock().await;
        BrowserRunSnapshot {
            revision: state.revision,
            input_state: if state.active.is_some() || state.gate_failed {
                BrowserInputState::AgentRunning
            } else {
                BrowserInputState::UserReady
            },
            input_gate_failed: state.gate_failed,
        }
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Called by the authoritative run owner before accepting any Browser work.
    /// The returned guard must be retained until `finish` succeeds.
    pub async fn begin(self: &Arc<Self>) -> Result<BrowserRunGuard, RunAdmissionError> {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move { coordinator.begin_inner().await })
            .await
            .map_err(|_| RunAdmissionError::WorkerFailed)?
    }

    async fn begin_inner(self: &Arc<Self>) -> Result<BrowserRunGuard, RunAdmissionError> {
        let _transition = self.transition.lock().await;
        let _operation = self.operation.lock().await;
        {
            let state = self.state.lock().await;
            if state.active.is_some() || state.gate_failed {
                return Err(RunAdmissionError::Busy);
            }
        }
        // Set the failure latch before awaiting native code. A panic or a failed
        // platform callback must never make the projection look user-ready.
        {
            let mut state = self.state.lock().await;
            state.gate_failed = true;
            state.revision += 1;
            self.changes.bump();
        }
        self.gate.lock_user_input().await?;
        self.gate.release_pressed_input().await?;
        let run = Arc::new(Run {
            cancelled: CancellationToken::new(),
        });
        let mut state = self.state.lock().await;
        state.gate_failed = false;
        state.active = Some(Arc::clone(&run));
        state.revision += 1;
        self.changes.bump();
        Ok(BrowserRunGuard(Arc::new(RunAuthority {
            run,
            coordinator: Arc::downgrade(self),
            release_on_drop: AtomicBool::new(true),
        })))
    }

    /// Run cancellation prevents queued work immediately. The native input gate
    /// opens only after the current operation and pressed-input cleanup settle.
    /// A failed finish can be retried with the same guard.
    pub async fn finish(self: &Arc<Self>, run: &BrowserRunGuard) -> Result<(), RunAdmissionError> {
        self.require_current(&run.0.run).await?;
        run.cancel();
        let coordinator = Arc::clone(self);
        let run = run.0.run.clone();
        tokio::spawn(async move { coordinator.finish_inner(&run).await })
            .await
            .map_err(|_| RunAdmissionError::WorkerFailed)?
    }

    /// Drain browser work before publishing the Agent terminal, keeping hardware
    /// input locked. `finish` releases it only after the terminal was published.
    pub async fn settle(self: &Arc<Self>, run: &BrowserRunGuard) -> Result<(), RunAdmissionError> {
        self.require_current(&run.0.run).await?;
        run.cancel();
        let coordinator = self.clone();
        let run = run.0.run.clone();
        tokio::spawn(async move {
            let _transition = coordinator.transition.lock().await;
            let _operation = coordinator.operation.lock().await;
            coordinator.require_current(&run).await?;
            coordinator.gate.release_pressed_input().await
        })
        .await
        .map_err(|_| RunAdmissionError::WorkerFailed)?
    }

    async fn finish_inner(&self, run: &Arc<Run>) -> Result<(), RunAdmissionError> {
        let _transition = self.transition.lock().await;
        let _operation = self.operation.lock().await;
        self.require_current(run).await?;
        self.unlock().await?;
        let mut state = self.state.lock().await;
        state.active = None;
        state.revision += 1;
        self.changes.bump();
        Ok(())
    }

    /// Recover a failed *start*, for which no run authority was issued. This is
    /// a host lifecycle action, never a human unlock/takeover command.
    pub async fn recover_failed_start(self: &Arc<Self>) -> Result<(), RunAdmissionError> {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            let _transition = coordinator.transition.lock().await;
            let _operation = coordinator.operation.lock().await;
            if coordinator.state.lock().await.active.is_some() {
                return Err(RunAdmissionError::Busy);
            }
            coordinator.unlock().await
        })
        .await
        .map_err(|_| RunAdmissionError::WorkerFailed)?
    }

    async fn unlock(&self) -> Result<(), RunAdmissionError> {
        {
            let mut state = self.state.lock().await;
            state.gate_failed = true;
            state.revision += 1;
            self.changes.bump();
        }
        self.gate.release_pressed_input().await?;
        self.gate.unlock_user_input().await?;
        let mut state = self.state.lock().await;
        state.gate_failed = false;
        state.revision += 1;
        self.changes.bump();
        Ok(())
    }

    async fn require_current(&self, run: &Arc<Run>) -> Result<(), RunAdmissionError> {
        if self
            .state
            .lock()
            .await
            .active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, run))
        {
            Ok(())
        } else {
            Err(RunAdmissionError::StaleRun)
        }
    }

    /// `work` must settle native callbacks or transfer them to a gate-owned
    /// task under this same cancellation token before returning an explicit
    /// awaiting-dialog result. NativeInputGate must drain that task before
    /// unlocking. We never race/drop `work` itself against cancellation.
    pub async fn agent_operation<T, F, Fut>(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
        work: F,
    ) -> Result<T, RunAdmissionError>
    where
        T: Send + 'static,
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, RunAdmissionError>> + Send + 'static,
    {
        self.require_current(&run.0.run).await?;
        let coordinator = Arc::clone(self);
        let run = run.0.run.clone();
        tokio::spawn(async move {
            let _operation = tokio::select! {
                biased;
                _ = run.cancelled.cancelled() => return Err(RunAdmissionError::Cancelled),
                lock = coordinator.operation.lock() => lock,
            };
            coordinator.require_current(&run).await?;
            if run.cancelled.is_cancelled() {
                return Err(RunAdmissionError::Cancelled);
            }
            if coordinator.state.lock().await.gate_failed {
                return Err(RunAdmissionError::InputGateFailed);
            }
            match std::panic::AssertUnwindSafe(async { work(run.cancelled.clone()).await })
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    run.cancelled.cancel();
                    let mut state = coordinator.state.lock().await;
                    state.gate_failed = true;
                    state.revision += 1;
                    coordinator.changes.bump();
                    Err(RunAdmissionError::WorkerFailed)
                }
            }
        })
        .await
        .map_err(|_| RunAdmissionError::WorkerFailed)?
    }

    /// Human navigation/tab commands use the same operation gate as Agent work.
    /// Hardware input is controlled by the native adapter, never by this API.
    pub async fn user_operation<T, F, Fut>(
        self: &Arc<Self>,
        work: F,
    ) -> Result<T, RunAdmissionError>
    where
        T: Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, RunAdmissionError>> + Send + 'static,
    {
        // A user request made during an Agent run is rejected now, not queued
        // behind native work and replayed after that run happens to finish.
        let admitted_revision={
            let state=self.state.lock().await;
            if state.active.is_some() || state.gate_failed { return Err(RunAdmissionError::UserInputLocked); }
            state.revision
        };
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            let _operation = coordinator.operation.lock().await;
            {
                let state = coordinator.state.lock().await;
                if state.active.is_some() || state.gate_failed || state.revision!=admitted_revision {
                    return Err(RunAdmissionError::UserInputLocked);
                }
            }
            work().await
        })
        .await
        .map_err(|_| RunAdmissionError::WorkerFailed)?
    }
}

#[cfg(test)]
mod tests;
