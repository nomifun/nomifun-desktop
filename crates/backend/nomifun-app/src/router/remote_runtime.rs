//! Runtime admission coordinator for the canonical Remote REST surface.
//!
//! Remote `open` deliberately commits the Session before it crosses the
//! sidecar boundary. This coordinator owns the post-commit step: one launch
//! attempt per Session, bounded Runtime open/handshake, and a durable
//! `session/open-failed` fallback for every ordinary failure.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::{FutureExt, future::join_all};
use nomifun_agent_contracts::AgentSessionId;
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformError, SessionRuntimeLaunchConfig,
};
use nomifun_codex_runtime::{
    ClientLimits, InheritedHandleCredential, RuntimeProcessConfig,
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::bootstrap::runtime_artifact;

const REMOTE_OPEN_FAILURE_CODE: &str = "REMOTE_OPEN_FAILED";
const RUNTIME_OPEN_WORK_DIR: &str = "runtime-sessions";
const REMOTE_OPEN_SCHEDULE_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OPEN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(35);
const OPEN_FAILURE_PERSIST_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_RECONCILE_TIMEOUT: Duration = Duration::from_secs(30);
const OPEN_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_OPEN_FAILURE_MESSAGE_CHARS: usize = 2_000;
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
/// `AgentRuntimeRegistry`/`ConversationService`; Remote must therefore never
/// create a second process runtime merely to give `open` an asynchronous
/// shape.  This supervisor only keeps a bounded, single-flight task alive while
/// the Nomi service settles a real operation (usually the optional initial
/// turn).  Durable state and idempotency remain owned by the injected
/// projection store and `ConversationService`.
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
    /// an HTTP timeout cannot cancel a ConversationService operation after its
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
            registry
                .abort_handles
                .insert(key, handle.abort_handle());
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

#[derive(Clone)]
pub(crate) struct RemoteRuntimeCoordinator {
    platform: Arc<AgentPlatform>,
    runtime_root: PathBuf,
    tasks: Arc<StdMutex<OpeningTaskRegistry>>,
    changed: Arc<Notify>,
    detached_mutations: RemoteDetachedMutationRegistry,
}

#[derive(Default)]
struct OpeningTaskRegistry {
    sessions: BTreeSet<AgentSessionId>,
    /// A failed `session/open-failed` append is a storage/recovery blocker,
    /// not a reason to launch the same sidecar again for every incoming
    /// request. The marker is process-local: after a host restart, durable
    /// `opening` Sessions are reconciled by the fresh coordinator.
    failure_persistence_blockers: BTreeMap<AgentSessionId, String>,
    closed: bool,
}

struct OpeningTaskLease {
    registry: Arc<StdMutex<OpeningTaskRegistry>>,
    changed: Arc<Notify>,
    coordinator: Arc<RemoteRuntimeCoordinator>,
    session_id: AgentSessionId,
    completed: bool,
}

impl OpeningTaskLease {
    fn new(
        registry: Arc<StdMutex<OpeningTaskRegistry>>,
        changed: Arc<Notify>,
        coordinator: Arc<RemoteRuntimeCoordinator>,
        session_id: AgentSessionId,
    ) -> Self {
        Self {
            registry,
            changed,
            coordinator,
            session_id,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for OpeningTaskLease {
    fn drop(&mut self) {
        if self.completed {
            match self.registry.lock() {
                Ok(mut registry) => {
                    registry.sessions.remove(&self.session_id);
                }
                Err(poisoned) => {
                    poisoned.into_inner().sessions.remove(&self.session_id);
                }
            };
            self.changed.notify_waiters();
            return;
        }

        let coordinator = Arc::clone(&self.coordinator);
        let session_id = self.session_id.clone();
        let registry = Arc::clone(&self.registry);
        let changed = Arc::clone(&self.changed);
        let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
            self.coordinator.mark_failure_persistence_blocker(
                &self.session_id,
                bounded_failure_message(
                    "Remote Runtime admission was cancelled without a Tokio runtime; \
                     session/open-failed could not be scheduled",
                ),
            );
            match self.registry.lock() {
                Ok(mut registry) => {
                    registry.sessions.remove(&self.session_id);
                }
                Err(poisoned) => {
                    poisoned.into_inner().sessions.remove(&self.session_id);
                }
            };
            self.changed.notify_waiters();
            tracing::error!(
                session_id = session_id.as_ref(),
                "Remote Runtime task was cancelled without a Tokio runtime; recovery will retry it on host restart"
            );
            return;
        };

        // Keep the task registered until the cancellation failure fact has
        // been attempted.  This prevents host shutdown from observing an
        // empty registry and closing the pool before the durable convergence
        // event is written.
        handle.spawn(async move {
            coordinator
                .record_failure_or_mark(
                    &session_id,
                    "Remote Runtime admission task was cancelled",
                )
                .await;
            match registry.lock() {
                Ok(mut registry) => {
                    registry.sessions.remove(&session_id);
                }
                Err(poisoned) => {
                    poisoned.into_inner().sessions.remove(&session_id);
                }
            };
            changed.notify_waiters();
        });
    }
}

impl RemoteRuntimeCoordinator {
    pub(crate) fn new(platform: Arc<AgentPlatform>, data_root: PathBuf) -> Self {
        Self {
            platform,
            runtime_root: data_root.join(RUNTIME_OPEN_WORK_DIR),
            tasks: Arc::new(StdMutex::new(OpeningTaskRegistry::default())),
            changed: Arc::new(Notify::new()),
            detached_mutations: RemoteDetachedMutationRegistry::new(),
        }
    }

    pub(crate) fn detached_mutation_registry(&self) -> RemoteDetachedMutationRegistry {
        self.detached_mutations.clone()
    }

    /// Start the post-commit Runtime admission exactly once for an opening
    /// Session. The HTTP handler does not wait for the sidecar; clients read
    /// the resulting `ready` or `open_failed` fact through the normal cursor.
    pub(crate) async fn ensure_started(
        self: &Arc<Self>,
        session_id: AgentSessionId,
    ) -> Result<(), AgentPlatformError> {
        let result = match tokio::time::timeout(
            REMOTE_OPEN_SCHEDULE_TIMEOUT,
            self.ensure_started_inner(session_id.clone()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(AgentPlatformError::Contract(format!(
                "Remote Runtime admission scheduling exceeded its {} second deadline",
                REMOTE_OPEN_SCHEDULE_TIMEOUT.as_secs()
            ))),
        };
        if let Err(error) = &result {
            // A storage failure while writing the convergence event is a
            // host/recovery blocker. Do not relaunch the same sidecar from
            // every subsequent `observe`, `turn`, or idempotent `open`.
            if !self.failure_persistence_blocked(&session_id) {
                let message = bounded_failure_message(&format!(
                    "Remote Runtime admission could not be scheduled: {error}"
                ));
                self.record_failure_or_mark(&session_id, &message).await;
            }
        }
        result
    }

    async fn ensure_started_inner(
        self: &Arc<Self>,
        session_id: AgentSessionId,
    ) -> Result<(), AgentPlatformError> {
        let head = self.platform.session_store().head(&session_id).await?;
        if head.status != "opening" {
            self.clear_failure_persistence_blocker(&session_id);
            return Ok(());
        }

        let mut registry = self.tasks.lock().map_err(|_| {
            AgentPlatformError::Contract(
                "Remote Runtime task registry is poisoned".to_owned(),
            )
        })?;
        if let Some(reason) = registry
            .failure_persistence_blockers
            .get(&session_id)
            .cloned()
        {
            return Err(AgentPlatformError::Contract(format!(
                "Remote Runtime admission remains unresolved because \
                 session/open-failed could not be durably recorded: {reason}; \
                 host restart reconciliation is required"
            )));
        }
        if registry.closed {
            return Err(AgentPlatformError::Contract(
                "Remote Runtime coordinator is closed".to_owned(),
            ));
        }
        if !registry.sessions.insert(session_id.clone()) {
            return Ok(());
        }
        drop(registry);

        let coordinator = Arc::clone(self);
        let task_lease = OpeningTaskLease::new(
            Arc::clone(&self.tasks),
            Arc::clone(&self.changed),
            Arc::clone(self),
            session_id.clone(),
        );
        tokio::spawn(async move {
            let mut task_lease = task_lease;
            let result = std::panic::AssertUnwindSafe(
                coordinator.launch_and_record_with_failure(session_id.clone()),
            )
            .catch_unwind()
            .await;
            if let Err(_) = result {
                if !coordinator.failure_persistence_blocked(&session_id) {
                    coordinator
                        .record_failure_or_mark(
                            &session_id,
                            "Remote Runtime admission task panicked",
                        )
                        .await;
                }
            }
            task_lease.complete();
        });
        Ok(())
    }

    /// Recover Sessions that were committed before the previous host process
    /// could schedule its post-commit launch task. The whole recovery pass is
    /// bounded so a damaged store or an unexpectedly large stale set cannot
    /// hold host startup forever.
    pub(crate) async fn reconcile_opening_sessions(
        self: &Arc<Self>,
    ) -> Result<(), AgentPlatformError> {
        match tokio::time::timeout(
            REMOTE_RECONCILE_TIMEOUT,
            self.reconcile_opening_sessions_inner(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(AgentPlatformError::Contract(format!(
                "Remote Runtime opening-session reconciliation exceeded its {} second deadline",
                REMOTE_RECONCILE_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn reconcile_opening_sessions_inner(
        self: &Arc<Self>,
    ) -> Result<(), AgentPlatformError> {
        let sessions = tokio::time::timeout(
            REMOTE_OPEN_SCHEDULE_TIMEOUT,
            self.platform
                .session_store()
                .list_opening_remote_sessions(),
        )
        .await
        .map_err(|_| {
            AgentPlatformError::Contract(format!(
                "Remote Runtime opening-session discovery exceeded its {} second deadline",
                REMOTE_OPEN_SCHEDULE_TIMEOUT.as_secs()
            ))
        })??;
        for session_id in sessions {
            self.ensure_started(session_id).await?;
        }
        Ok(())
    }

    /// Stop admitting new launches/mutations and wait a bounded interval for
    /// in-flight work to finish its own durable convergence and cleanup.
    pub(crate) async fn shutdown(&self) {
        self.detached_mutations.close();
        {
            let Ok(mut registry) = self.tasks.lock() else {
                tracing::error!("Remote Runtime task registry is poisoned during shutdown");
                return;
            };
            registry.closed = true;
            if registry.sessions.is_empty() && self.detached_mutations.is_empty() {
                return;
            }
        }

        let deadline = tokio::time::Instant::now() + OPEN_TASK_SHUTDOWN_TIMEOUT;
        loop {
            let opening_changed = self.changed.notified();
            let mutation_changed = self.detached_mutations.change_notifier();
            let mutation_changed = mutation_changed.notified();
            tokio::pin!(opening_changed);
            tokio::pin!(mutation_changed);
            opening_changed.as_mut().enable();
            mutation_changed.as_mut().enable();
            let opening_is_empty = self
                .tasks
                .lock()
                .map(|registry| registry.sessions.is_empty())
                .unwrap_or(true);
            if opening_is_empty && self.detached_mutations.is_empty() {
                return;
            }
            tokio::select! {
                _ = opening_changed.as_mut() => {}
                _ = mutation_changed.as_mut() => {}
                _ = tokio::time::sleep_until(deadline) => {
                    let remaining_sessions = self
                        .tasks
                        .lock()
                        .map(|registry| registry.sessions.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    let remaining_mutations = self.detached_mutations.active_keys();
                    tracing::error!(
                        opening_tasks = remaining_sessions.len(),
                        detached_mutations = remaining_mutations.len(),
                        "Remote tasks did not quiesce before shutdown timeout"
                    );
                    self.detached_mutations.abort_active_tasks();
                    if !remaining_mutations.is_empty() {
                        tracing::error!(
                            mutation_keys = ?remaining_mutations,
                            "Remote detached mutations remain unresolved at shutdown"
                        );
                    }
                    join_all(remaining_sessions.into_iter().map(|session_id| async move {
                        self.record_failure_or_mark(
                            &session_id,
                            "Remote Runtime admission was interrupted during host shutdown",
                        )
                        .await;
                    }))
                    .await;
                    return;
                }
            }
        }
    }

    async fn launch_and_record(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentPlatformError> {
        let head = self.platform.session_store().head(&session_id).await?;
        if head.status != "opening" {
            return Ok(());
        }

        let artifact = tokio::task::spawn_blocking(runtime_artifact::resolve)
            .await
            .map_err(|error| {
                AgentPlatformError::Contract(format!(
                    "Codex Runtime artifact resolution task failed: {error}"
                ))
            })?
            .map_err(|error| {
                AgentPlatformError::Contract(format!(
                    "Codex Runtime artifact is unavailable: {error}"
                ))
            })?;
        tracing::debug!(
            target_id = %artifact.target_id,
            runtime_target = ?artifact.runtime_target,
            executable_digest = ?artifact.executable_digest,
            "Codex Runtime artifact resolved for Remote admission"
        );

        let working_directory = self.runtime_root.join(session_id.as_ref());
        std::fs::create_dir_all(&working_directory).map_err(|error| {
            AgentPlatformError::Contract(format!(
                "Codex Runtime working directory could not be created: {error}"
            ))
        })?;

        // This is an opaque per-session bootstrap value for the sidecar
        // inherited-handle channel. Provider credentials remain in the
        // host-owned ChatModelBroker and are never copied into this request.
        let credential =
            InheritedHandleCredential::new(nomifun_auth::generate_random_hex_secret().into_bytes())
            .map_err(|error| AgentPlatformError::Contract(error.to_string()))?;
        let process = RuntimeProcessConfig::pinned_app_server(
            artifact.executable,
            working_directory,
            artifact.target_id,
            artifact.executable_digest,
            &artifact.release,
        )
        .map_err(AgentPlatformError::from)?;

        self.platform
            .launch_session_runtime(
                &session_id,
                SessionRuntimeLaunchConfig {
                    process,
                    credential,
                    release: artifact.release,
                    hello_expectation: artifact.hello_expectation,
                    client_limits: ClientLimits::default(),
                    dispose_timeout: Duration::from_secs(5),
                },
            )
            .await
    }

    async fn record_failure_or_mark(&self, session_id: &AgentSessionId, message: &str) {
        if self.failure_persistence_blocked(session_id) {
            return;
        }
        let message = bounded_failure_message(message);
        let result = std::panic::AssertUnwindSafe(tokio::time::timeout(
            OPEN_FAILURE_PERSIST_TIMEOUT,
            self.platform.session_store().append_open_failed(
                session_id,
                REMOTE_OPEN_FAILURE_CODE,
                &message,
                true,
            ),
        ))
        .catch_unwind()
        .await;
        match result {
            Ok(Ok(Ok(Some(_)))) | Ok(Ok(Ok(None))) => {
                self.clear_failure_persistence_blocker(session_id);
            }
            Ok(Ok(Err(error))) => {
                let reason = bounded_failure_message(&format!(
                    "{message}; durable session/open-failed append failed: {error}"
                ));
                self.mark_failure_persistence_blocker(session_id, reason);
                tracing::error!(
                    ?error,
                    session_id = session_id.as_ref(),
                    "Remote Runtime failure could not be persisted as session/open-failed"
                );
            }
            Ok(Err(_)) => {
                let reason = bounded_failure_message(&format!(
                    "{message}; durable session/open-failed append exceeded its {} second deadline",
                    OPEN_FAILURE_PERSIST_TIMEOUT.as_secs()
                ));
                self.mark_failure_persistence_blocker(session_id, reason);
                tracing::error!(
                    session_id = session_id.as_ref(),
                    timeout_seconds = OPEN_FAILURE_PERSIST_TIMEOUT.as_secs(),
                    "Remote Runtime failure persistence timed out"
                );
            }
            Err(_) => {
                let reason = bounded_failure_message(&format!(
                    "{message}; durable session/open-failed append panicked"
                ));
                self.mark_failure_persistence_blocker(session_id, reason);
                tracing::error!(
                    session_id = session_id.as_ref(),
                    "Remote Runtime failure persistence panicked"
                );
            }
        }
    }

    async fn launch_and_record_with_failure(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentPlatformError> {
        let result = tokio::time::timeout(
            REMOTE_OPEN_ATTEMPT_TIMEOUT,
            self.launch_and_record(session_id.clone()),
        )
        .await;
        match result {
            Err(_) => {
                let error = AgentPlatformError::Contract(format!(
                    "Remote Runtime admission exceeded its {} second deadline",
                    REMOTE_OPEN_ATTEMPT_TIMEOUT.as_secs()
                ));
                let message = bounded_failure_message(&error.to_string());
                tracing::warn!(
                    session_id = session_id.as_ref(),
                    timeout_seconds = REMOTE_OPEN_ATTEMPT_TIMEOUT.as_secs(),
                    "Remote Runtime admission timed out"
                );
                self.record_failure_or_mark(&session_id, &message).await;
                Err(error)
            }
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                let message = bounded_failure_message(&format!(
                    "Codex Runtime open failed: {error}"
                ));
                tracing::warn!(
                    ?error,
                    session_id = session_id.as_ref(),
                    "Remote Runtime admission failed"
                );
                self.record_failure_or_mark(&session_id, &message).await;
                Err(error)
            }
        }
    }

    pub(crate) fn failure_persistence_blocker(
        &self,
        session_id: &AgentSessionId,
    ) -> Option<String> {
        match self.tasks.lock() {
            Ok(registry) => registry
                .failure_persistence_blockers
                .get(session_id)
                .cloned(),
            Err(poisoned) => poisoned
                .into_inner()
                .failure_persistence_blockers
                .get(session_id)
                .cloned(),
        }
    }

    fn failure_persistence_blocked(&self, session_id: &AgentSessionId) -> bool {
        self.failure_persistence_blocker(session_id).is_some()
    }

    fn mark_failure_persistence_blocker(&self, session_id: &AgentSessionId, reason: String) {
        match self.tasks.lock() {
            Ok(mut registry) => {
                registry
                    .failure_persistence_blockers
                    .entry(session_id.clone())
                    .or_insert(reason);
            }
            Err(poisoned) => {
                poisoned
                    .into_inner()
                    .failure_persistence_blockers
                    .entry(session_id.clone())
                    .or_insert(reason);
            }
        }
    }

    fn clear_failure_persistence_blocker(&self, session_id: &AgentSessionId) {
        match self.tasks.lock() {
            Ok(mut registry) => {
                registry.failure_persistence_blockers.remove(session_id);
            }
            Err(poisoned) => {
                poisoned
                    .into_inner()
                    .failure_persistence_blockers
                    .remove(session_id);
            }
        }
    }
}

impl nomifun_public::CanonicalRemoteRuntimeAdmission for RemoteRuntimeCoordinator {
    fn ensure_started<'a>(
        &'a self,
        session_id: AgentSessionId,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let coordinator = Arc::new(self.clone());
        Box::pin(async move {
            coordinator
                .ensure_started(session_id)
                .await
                .map_err(|error| error.to_string())
        })
    }
}

fn bounded_failure_message(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "Codex Runtime open failed".to_owned();
    }
    normalized
        .chars()
        .take(MAX_OPEN_FAILURE_MESSAGE_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_blocker_keeps_the_first_actionable_cause() {
        let session_id = AgentSessionId::from("remote-session");
        let mut registry = OpeningTaskRegistry::default();

        registry
            .failure_persistence_blockers
            .insert(session_id.clone(), "database is read-only".to_owned());
        registry
            .failure_persistence_blockers
            .entry(session_id.clone())
            .or_insert("sidecar unavailable".to_owned());

        assert_eq!(
            registry.failure_persistence_blockers.get(&session_id),
            Some(&"database is read-only".to_owned())
        );
    }

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
        assert_eq!(
            registry.active_keys(),
            vec!["remote:turn:first".to_owned()]
        );

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

    #[test]
    fn remote_open_and_reconcile_deadlines_are_explicitly_bounded() {
        assert!(REMOTE_OPEN_SCHEDULE_TIMEOUT < REMOTE_OPEN_ATTEMPT_TIMEOUT);
        assert!(OPEN_FAILURE_PERSIST_TIMEOUT < REMOTE_RECONCILE_TIMEOUT);
        assert!(REMOTE_RECONCILE_TIMEOUT < OPEN_TASK_SHUTDOWN_TIMEOUT);
    }
}
