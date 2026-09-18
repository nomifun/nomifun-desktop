//! Process-wide, result-bearing shutdown for the existing Session registry.
//! No new Session coordinator and no timeout-as-exit or kill-as-proof shortcut.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::{
    FutureExt, StreamExt,
    future::{BoxFuture, Shared},
    stream,
};
use nomifun_common::AppError;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::{AgentRuntimeSessions, InMemoryAgentRuntimeSessions};

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;

struct Flight {
    completion: Completion,
}

#[derive(Default)]
pub(super) struct ShutdownState {
    pub(super) closed: CancellationToken,
    pub(super) admission: Arc<RwLock<()>>,
    /// An acquisition task disappeared without returning a factory result.
    /// Empty slots no longer prove absence of effects after such an exit.
    pub(super) uncertain_acquisition: AtomicBool,
    flight: Mutex<Option<Arc<Flight>>>,
}

pub(super) fn closed_error() -> AppError {
    AppError::Conflict("Agent runtime registry is shutting down; admission is closed".into())
}

pub(super) fn start(registry: &InMemoryAgentRuntimeSessions) -> crate::RuntimeTeardown {
    registry.shutdown.closed.cancel();
    // Prompt cancellation of already visible runtimes while cold admissions
    // drain. This request is not the proof used by the cleanup flight below.
    registry.terminate_all();
    let executor = match tokio::runtime::Handle::try_current() {
        Ok(executor) => executor,
        Err(_) => {
            return Box::pin(async {
                Err(AppError::Internal(
                    "Registry shutdown requires the async host executor".into(),
                ))
            });
        }
    };
    let mut slot = match registry.shutdown.flight.lock() {
        Ok(slot) => slot,
        Err(_) => {
            return Box::pin(async {
                Err(AppError::Conflict(
                    "Registry shutdown flight lock poisoned".into(),
                ))
            });
        }
    };
    let flight = slot
        .get_or_insert_with(|| {
            let registry = registry.clone();
            // Spawn immediately. Neither a host timeout nor a dropped result future
            // aborts a cold constructor or discards a runtime's cleanup witness.
            let task = executor.spawn(async move { drain(&registry).await });
            Arc::new(Flight {
                completion: async move {
                    task.await
                        .map_err(|_| "Registry shutdown task failed".to_owned())?
                }
                .boxed()
                .shared(),
            })
        })
        .clone();
    drop(slot);
    let state = registry.shutdown.clone();
    Box::pin(async move {
        let result = flight.completion.clone().await;
        if result.is_err() {
            let mut slot = state
                .flight
                .lock()
                .map_err(|_| AppError::Conflict("Registry shutdown flight lock poisoned".into()))?;
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &flight))
            {
                *slot = None;
            }
        }
        result.map_err(AppError::Conflict)
    })
}

async fn drain(registry: &InMemoryAgentRuntimeSessions) -> Result<(), String> {
    // Wait for every pre-close admission before snapshotting. The reader spans
    // provider resolution, per-Session gate waits, factory and post-build checks.
    // A stuck factory keeps this owned flight pending; callers may time out but
    // cannot close storage or claim cleanup success.
    let _admission = registry.shutdown.admission.write().await;
    registry.terminate_all();
    let ids = registry
        .runtimes
        .iter()
        .map(|entry| entry.key().clone())
        .chain(
            registry
                .teardown_quarantine
                .iter()
                .map(|entry| entry.key().clone()),
        )
        .collect::<std::collections::BTreeSet<_>>();
    let mut results = stream::iter(ids)
        .map(|id| {
            let registry = registry.clone();
            async move {
                // This uses the existing per-Session gate and exact-slot teardown.
                // Failed exits retain the slot and its workspace lease in quarantine.
                registry.terminate_owned_runtime(&id, None).await
            }
        })
        .buffer_unordered(16);
    let mut failures = 0usize;
    while let Some(result) = results.next().await {
        if result.is_err() {
            failures += 1;
        }
    }
    if failures != 0 {
        return Err(format!(
            "{failures} Agent runtime(s) did not prove shutdown"
        ));
    }
    // active_runtime_count deliberately hides quarantine and empty builds.
    // It must never be used as a substitute for these retained-owner checks.
    if registry
        .shutdown
        .uncertain_acquisition
        .load(Ordering::Acquire)
        || !registry.runtimes.is_empty()
        || !registry.teardown_quarantine.is_empty()
        || !registry.turn_admissions.is_empty()
        || !registry.workspace_bindings.is_empty()
        || !registry.model_config_bindings.is_empty()
    {
        return Err("Agent runtime ownership remains after registry shutdown".into());
    }
    Ok(())
}
