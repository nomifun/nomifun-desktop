//! A caller owns only its wait, never the lifetime of an admitted constructor.
//! The existing registry still owns slots, per-Session gates and exact teardown.
use std::sync::{Arc, atomic::Ordering};

use nomifun_common::AppError;
use tokio::sync::{OwnedRwLockReadGuard, oneshot};
use tokio_util::sync::CancellationToken;

use super::{AgentRuntimeBuildOptions, AgentRuntimeHandle, InMemoryAgentRuntimeSessions, shutdown};

struct CancelWait(Option<CancellationToken>);
impl Drop for CancelWait {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

struct AcquisitionGuard {
    shutdown: Arc<shutdown::ShutdownState>,
    // Field drop follows Drop::drop: publish uncertainty before releasing the
    // read barrier, so a waiting shutdown writer cannot observe false quiescence.
    _admission: OwnedRwLockReadGuard<()>,
    returned: bool,
}

fn mark_uncertain(shutdown: &shutdown::ShutdownState) {
    shutdown
        .uncertain_acquisition
        .store(true, Ordering::Release);
    shutdown.closed.cancel();
}

/// Must be declared after the per-Session lifecycle guard. Unwinding publishes
/// uncertainty before releasing that gate, not only before the outer read lock.
pub(super) struct UnwindFence(Arc<shutdown::ShutdownState>);

impl UnwindFence {
    pub(super) fn new(shutdown: Arc<shutdown::ShutdownState>) -> Self {
        Self(shutdown)
    }
}

impl Drop for UnwindFence {
    fn drop(&mut self) {
        if std::thread::panicking() {
            mark_uncertain(&self.0);
        }
    }
}

impl Drop for AcquisitionGuard {
    fn drop(&mut self) {
        if !self.returned {
            mark_uncertain(&self.shutdown);
        }
    }
}

pub(super) async fn run(
    registry: &InMemoryAgentRuntimeSessions,
    conversation_id: &str,
    generation: Option<u64>,
    cancellation: Option<CancellationToken>,
    options: AgentRuntimeBuildOptions,
) -> Result<AgentRuntimeHandle, AppError> {
    let executor = tokio::runtime::Handle::try_current().map_err(|_| {
        AppError::Internal("Runtime acquisition requires the async host executor".into())
    })?;
    // A dropped preparation/send waiter may cancel this acquisition, never the
    // caller's parent token or another waiter's acquisition of the same slot.
    let cancellation =
        cancellation.map_or_else(CancellationToken::new, |parent| parent.child_token());
    let mut waiter = CancelWait(Some(cancellation.clone()));
    let admission = tokio::select! {
        biased;
        _ = registry.shutdown.closed.cancelled() => return Err(shutdown::closed_error()),
        _ = cancellation.cancelled() => return Err(AppError::Conflict("Agent runtime acquisition was cancelled".into())),
        admission = registry.shutdown.admission.clone().read_owned() => admission,
    };
    if registry.shutdown.closed.is_cancelled() {
        return Err(shutdown::closed_error());
    }
    let guard = AcquisitionGuard {
        shutdown: registry.shutdown.clone(),
        _admission: admission,
        returned: false,
    };
    let registry = registry.clone();
    let conversation_id = conversation_id.to_owned();
    let (sender, receiver) = oneshot::channel();
    // No await between the admission guard and transferring it into the owned
    // task. Dropping the JoinHandle detaches; no abort handle reaches callers.
    let _task = executor.spawn(async move {
        let mut guard = guard;
        let result = registry
            .get_or_create_runtime_inner(&conversation_id, generation, Some(cancellation), options)
            .await;
        // Normal Err follows the existing factory contract: the constructor
        // must settle partial resources before returning Err. A panic/abort
        // cannot substitute for that contract and leaves the guard armed.
        guard.returned = true;
        // If delivery loses a final race with waiter drop, a successful runtime
        // remains in its exact registered slot. Do not terminate by Session ID
        // here: a successor may already have adopted the same reusable runtime.
        // Existing owner teardown / process shutdown still owns that slot.
        let _ = sender.send(result);
    });
    let result = receiver.await.map_err(|_| {
        AppError::Conflict(
            "Agent runtime acquisition ended without a result; registry shutdown is unproven"
                .into(),
        )
    });
    waiter.0 = None;
    result?
}
