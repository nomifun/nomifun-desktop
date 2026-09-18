//! Remote task admission for the Conversation-backed product.
//! Shared task leases bound detached operations and shutdown across Engines.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::FutureExt;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};

const OPEN_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_REMOTE_DETACHED_MUTATIONS: usize = 64;

/// Process-local admission for Remote mutations whose HTTP waiter may time
/// out before the underlying command has reached a durable outcome.
///
/// The registry is deliberately separate from the Session store. It only
/// prevents duplicate in-flight work and bounds retained task state; durable
/// idempotency remains the source of truth after a permit is released.
#[derive(Clone)]
pub(crate) struct RemoteDetachedMutationRegistry {
    state: Arc<StdMutex<DetachedMutationRegistryState>>,
    slots: Arc<Semaphore>,
    changed: Arc<Notify>,
}

#[derive(Default)]
struct DetachedMutationRegistryState {
    active: BTreeSet<String>,
    abort_handles: BTreeMap<String, tokio::task::AbortHandle>,
    closed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteDetachedMutationAdmissionError {
    Closed,
    AlreadyRunning,
    CapacityExceeded,
}

/// A mutation permit stays alive inside the detached task after its HTTP
/// waiter has timed out.
#[derive(Clone)]
#[must_use = "dropping the permit releases the Remote mutation slot"]
pub(crate) struct RemoteDetachedMutationPermit {
    lease: Arc<RemoteDetachedMutationLease>,
}

struct RemoteDetachedMutationLease {
    registry: RemoteDetachedMutationRegistry,
    key: String,
    slot: Option<OwnedSemaphorePermit>,
}

impl RemoteDetachedMutationRegistry {
    pub(crate) fn new() -> Self {
        Self::new_with_limit(MAX_REMOTE_DETACHED_MUTATIONS)
    }

    fn new_with_limit(limit: usize) -> Self {
        Self {
            state: Arc::new(StdMutex::new(DetachedMutationRegistryState::default())),
            slots: Arc::new(Semaphore::new(limit.max(1))),
            changed: Arc::new(Notify::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self::new_with_limit(limit)
    }

    pub(crate) fn try_admit(
        &self,
        key: impl Into<String>,
    ) -> Result<RemoteDetachedMutationPermit, RemoteDetachedMutationAdmissionError> {
        let key = key.into();
        let mut state = self.lock_state();
        if state.closed {
            return Err(RemoteDetachedMutationAdmissionError::Closed);
        }
        if state.active.contains(&key) {
            return Err(RemoteDetachedMutationAdmissionError::AlreadyRunning);
        }
        let slot = match Arc::clone(&self.slots).try_acquire_owned() {
            Ok(slot) => slot,
            Err(TryAcquireError::Closed) => {
                return Err(RemoteDetachedMutationAdmissionError::Closed);
            }
            Err(TryAcquireError::NoPermits) => {
                return Err(RemoteDetachedMutationAdmissionError::CapacityExceeded);
            }
        };
        state.active.insert(key.clone());
        drop(state);

        Ok(RemoteDetachedMutationPermit {
            lease: Arc::new(RemoteDetachedMutationLease {
                registry: self.clone(),
                key,
                slot: Some(slot),
            }),
        })
    }

    pub(crate) fn close(&self) {
        {
            let mut state = self.lock_state();
            state.closed = true;
            self.slots.close();
        }
        self.changed.notify_waiters();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock_state().active.is_empty()
    }

    pub(crate) fn active_keys(&self) -> Vec<String> {
        self.lock_state().active.iter().cloned().collect()
    }

    pub(crate) fn change_notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.changed)
    }

    fn register_abort_handle(&self, key: &str, handle: tokio::task::AbortHandle) {
        let mut state = self.lock_state();
        if state.closed || !state.active.contains(key) {
            drop(state);
            handle.abort();
            return;
        }
        state.abort_handles.insert(key.to_owned(), handle);
    }

    pub(crate) fn abort_active_tasks(&self) {
        let handles = self
            .lock_state()
            .abort_handles
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for handle in handles {
            handle.abort();
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DetachedMutationRegistryState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl RemoteDetachedMutationPermit {
    pub(crate) fn register_abort_handle(&self, handle: tokio::task::AbortHandle) {
        self.lease
            .registry
            .register_abort_handle(&self.lease.key, handle);
    }
}

impl Drop for RemoteDetachedMutationLease {
    fn drop(&mut self) {
        // Release the capacity token before publishing that the key is gone.
        // A new caller can therefore acquire both pieces of admission as soon
        // as it observes the notification.
        drop(self.slot.take());
        let mut state = self.registry.lock_state();
        let removed = state.active.remove(&self.key);
        state.abort_handles.remove(&self.key);
        drop(state);
        if removed {
            self.registry.changed.notify_waiters();
        }
    }
}

/// Sidecar-free task supervisor used by the Nomi-core Remote adapter.
///
/// The current Nomi product owns its live runtime in
/// the local Runtime registry/AgentSession owner; Remote must therefore never
/// create a second process runtime merely to give `open` an asynchronous
/// shape.  This supervisor only keeps a bounded, single-flight task alive while
/// the Nomi service settles a real operation (usually the optional initial
/// turn).  Durable state and idempotency remain owned by the injected
/// projection store and canonical AgentSession owner.
#[derive(Clone)]
pub(crate) struct NomiCoreRemoteRuntimeCoordinator {
    tasks: Arc<StdMutex<NomiCoreRemoteTaskRegistry>>,
    changed: Arc<Notify>,
    detached_mutations: RemoteDetachedMutationRegistry,
}

#[derive(Default)]
struct NomiCoreRemoteTaskRegistry {
    active: BTreeSet<String>,
    abort_handles: BTreeMap<String, tokio::task::AbortHandle>,
    closed: bool,
}

struct NomiCoreRemoteTaskLease {
    registry: Arc<StdMutex<NomiCoreRemoteTaskRegistry>>,
    changed: Arc<Notify>,
    key: String,
}

impl Drop for NomiCoreRemoteTaskLease {
    fn drop(&mut self) {
        let removed = match self.registry.lock() {
            Ok(mut registry) => {
                registry.abort_handles.remove(&self.key);
                registry.active.remove(&self.key)
            }
            Err(poisoned) => {
                let mut registry = poisoned.into_inner();
                registry.abort_handles.remove(&self.key);
                registry.active.remove(&self.key)
            }
        };
        if removed {
            self.changed.notify_waiters();
        }
    }
}

impl NomiCoreRemoteRuntimeCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            tasks: Arc::new(StdMutex::new(NomiCoreRemoteTaskRegistry::default())),
            changed: Arc::new(Notify::new()),
            detached_mutations: RemoteDetachedMutationRegistry::new(),
        }
    }

    /// Start one sidecar-free Nomi operation exactly once for `key`.
    ///
    /// The task is intentionally detached from the request that admitted it:
    /// an HTTP timeout cannot cancel a canonical AgentSession operation after its
    /// durable receipt may have been committed.  The returned error is only an
    /// admission verdict; the task itself must persist its own terminal result.
    pub(crate) fn start_once<F, Fut>(
        &self,
        key: impl Into<String>,
        task: F,
    ) -> Result<(), RemoteDetachedMutationAdmissionError>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let key = key.into();
        let permit = self.detached_mutations.try_admit(key.clone())?;
        let tasks = Arc::clone(&self.tasks);
        let changed = Arc::clone(&self.changed);
        {
            let mut registry = self.lock_tasks();
            if registry.closed {
                drop(permit);
                return Err(RemoteDetachedMutationAdmissionError::Closed);
            }
            if !registry.active.insert(key.clone()) {
                drop(permit);
                return Err(RemoteDetachedMutationAdmissionError::AlreadyRunning);
            }

            // Keep an RAII lease inside the task.  It removes both the active
            // marker and its abort handle even when shutdown aborts the task;
            // otherwise a cancelled detached future could make the
            // coordinator look permanently busy and, worse, keep shutdown
            // waiting while the database is already being closed.
            let task_lease = NomiCoreRemoteTaskLease {
                registry: Arc::clone(&tasks),
                changed: Arc::clone(&changed),
                key: key.clone(),
            };
            let task_log_key = key.clone();
            let handle = tokio::spawn(async move {
                let _task_lease = task_lease;
                let _permit = permit;
                let result = std::panic::AssertUnwindSafe(task()).catch_unwind().await;
                if result.is_err() {
                    tracing::error!(
                        mutation_key = %task_log_key,
                        "Nomi-core Remote detached task panicked before its durable finalizer completed"
                    );
                }
            });
            registry.abort_handles.insert(key, handle.abort_handle());
        }
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock_tasks().active.is_empty() && self.detached_mutations.is_empty()
    }

    pub(crate) fn active_keys(&self) -> Vec<String> {
        self.lock_tasks().active.iter().cloned().collect()
    }

    /// Stop new Nomi-core Remote work and wait for already admitted tasks.
    pub(crate) async fn shutdown(&self) -> bool {
        self.detached_mutations.close();
        {
            let mut registry = self.lock_tasks();
            registry.closed = true;
            if registry.active.is_empty() && self.detached_mutations.is_empty() {
                return true;
            }
        }

        let deadline = tokio::time::Instant::now() + OPEN_TASK_SHUTDOWN_TIMEOUT;
        loop {
            let notified = self.changed.notified();
            let mutation_notifier = self.detached_mutations.change_notifier();
            let mutation_notified = mutation_notifier.notified();
            tokio::pin!(notified);
            tokio::pin!(mutation_notified);
            notified.as_mut().enable();
            mutation_notified.as_mut().enable();
            if self.is_empty() {
                return true;
            }
            tokio::select! {
                _ = notified.as_mut() => {}
                _ = mutation_notified.as_mut() => {}
                _ = tokio::time::sleep_until(deadline) => {
                    let remaining_tasks = self.active_keys();
                    let remaining_mutations = self.detached_mutations.active_keys();
                    tracing::error!(
                        tasks = ?remaining_tasks,
                        mutations = ?remaining_mutations,
                        "Nomi-core Remote tasks did not quiesce before shutdown timeout"
                    );
                    self.detached_mutations.abort_active_tasks();
                    let abort_handles = {
                        let registry = self.lock_tasks();
                        registry.abort_handles.values().cloned().collect::<Vec<_>>()
                    };
                    for handle in abort_handles {
                        handle.abort();
                    }
                    let abort_deadline =
                        tokio::time::Instant::now() + Duration::from_secs(2);
                    while !self.is_empty()
                        && tokio::time::Instant::now() < abort_deadline
                    {
                        let notified = self.changed.notified();
                        let mutation_notifier = self.detached_mutations.change_notifier();
                        let mutation_notified = mutation_notifier.notified();
                        tokio::pin!(notified);
                        tokio::pin!(mutation_notified);
                        tokio::select! {
                            _ = notified => {}
                            _ = mutation_notified => {}
                            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                        }
                    }
                    if self.is_empty() {
                        return true;
                    }
                    tracing::error!(
                        tasks = ?self.active_keys(),
                        mutations = ?self.detached_mutations.active_keys(),
                        "Nomi-core Remote tasks remained active after abort deadline"
                    );
                    return false;
                }
            }
        }
    }

    fn lock_tasks(&self) -> std::sync::MutexGuard<'_, NomiCoreRemoteTaskRegistry> {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_registry_enforces_key_deduplication_capacity_and_close() {
        let registry = RemoteDetachedMutationRegistry::with_limit(1);
        let first = registry
            .try_admit("remote:turn:first")
            .expect("first mutation should be admitted");

        assert!(matches!(
            registry.try_admit("remote:turn:first"),
            Err(RemoteDetachedMutationAdmissionError::AlreadyRunning)
        ));
        assert!(matches!(
            registry.try_admit("remote:turn:second"),
            Err(RemoteDetachedMutationAdmissionError::CapacityExceeded)
        ));
        assert_eq!(registry.active_keys(), vec!["remote:turn:first".to_owned()]);

        let child = first.clone();
        drop(first);
        assert!(
            matches!(
                registry.try_admit("remote:turn:first"),
                Err(RemoteDetachedMutationAdmissionError::AlreadyRunning)
            ),
            "a workflow child clone must keep the idempotency key occupied"
        );
        drop(child);
        assert!(registry.is_empty());

        registry.close();
        assert!(matches!(
            registry.try_admit("remote:turn:after-close"),
            Err(RemoteDetachedMutationAdmissionError::Closed)
        ));
    }
}
