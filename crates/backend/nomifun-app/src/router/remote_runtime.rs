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
use tokio::sync::Notify;

use crate::bootstrap::runtime_artifact;

const REMOTE_OPEN_FAILURE_CODE: &str = "REMOTE_OPEN_FAILED";
const RUNTIME_OPEN_WORK_DIR: &str = "runtime-sessions";
const REMOTE_OPEN_SCHEDULE_TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_OPEN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(35);
const OPEN_FAILURE_PERSIST_TIMEOUT: Duration = Duration::from_secs(10);
const OPEN_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_OPEN_FAILURE_MESSAGE_CHARS: usize = 2_000;

#[derive(Clone)]
pub(crate) struct RemoteRuntimeCoordinator {
    platform: Arc<AgentPlatform>,
    runtime_root: PathBuf,
    tasks: Arc<StdMutex<OpeningTaskRegistry>>,
    changed: Arc<Notify>,
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
        }
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
                coordinator
                    .record_failure_or_mark(
                        &session_id,
                        "Remote Runtime admission task panicked",
                    )
                    .await;
            }
            task_lease.complete();
        });
        Ok(())
    }

    /// Recover Sessions that were committed before the previous host process
    /// could schedule its post-commit launch task.
    pub(crate) async fn reconcile_opening_sessions(
        self: &Arc<Self>,
    ) -> Result<(), AgentPlatformError> {
        let sessions = self
            .platform
            .session_store()
            .list_opening_remote_sessions()
            .await?;
        for session_id in sessions {
            self.ensure_started(session_id).await?;
        }
        Ok(())
    }

    /// Stop admitting new launches and wait a bounded interval for an
    /// in-flight launch to finish its own process cleanup.
    pub(crate) async fn shutdown(&self) {
        {
            let Ok(mut registry) = self.tasks.lock() else {
                tracing::error!("Remote Runtime task registry is poisoned during shutdown");
                return;
            };
            registry.closed = true;
            if registry.sessions.is_empty() {
                return;
            }
        }

        let deadline = tokio::time::Instant::now() + OPEN_TASK_SHUTDOWN_TIMEOUT;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let is_empty = self
                .tasks
                .lock()
                .map(|registry| registry.sessions.is_empty())
                .unwrap_or(true);
            if is_empty {
                return;
            }
            tokio::select! {
                _ = notified.as_mut() => {}
                _ = tokio::time::sleep_until(deadline) => {
                    let remaining_sessions = self
                        .tasks
                        .lock()
                        .map(|registry| registry.sessions.iter().cloned().collect::<Vec<_>>())
                        .unwrap_or_default();
                    tracing::error!(
                        remaining = remaining_sessions.len(),
                        "Remote Runtime admission tasks did not quiesce before shutdown timeout"
                    );
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
        let message = bounded_failure_message(message);
        let result = tokio::time::timeout(
            OPEN_FAILURE_PERSIST_TIMEOUT,
            self.platform.session_store().append_open_failed(
                session_id,
                REMOTE_OPEN_FAILURE_CODE,
                &message,
                true,
            ),
        )
        .await;
        match result {
            Ok(Ok(Some(_))) | Ok(Ok(None)) => {
                self.clear_failure_persistence_blocker(session_id);
            }
            Ok(Err(error)) => {
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
            Err(_) => {
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

    fn failure_persistence_blocked(&self, session_id: &AgentSessionId) -> bool {
        match self.tasks.lock() {
            Ok(registry) => registry
                .failure_persistence_blockers
                .contains_key(session_id),
            Err(poisoned) => poisoned
                .into_inner()
                .failure_persistence_blockers
                .contains_key(session_id),
        }
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
}
