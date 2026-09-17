use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use nomifun_api_types::{AutoWorkState, AutoWorkTargetKind, Requirement, RequirementStatus};
use nomifun_common::{AppError, ConversationId};
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::service::{DEFAULT_LEASE_MS, RequirementService};
use crate::autowork_config::{AutoWorkConfig, AutoWorkConfigSnapshot};
use crate::attachments::PromptAttachmentPlan;
use crate::execution_port::{
    AutoWorkBindingLookup, AutoWorkExecutionPort, AutoWorkExecutionReceipt,
    AutoWorkExecutionRequest, AutoWorkExecutionSource,
    autowork_execution_operation_id,
};

pub use crate::execution_port::{
    AutoWorkWorkspacePort, AutoWorkWorkspaceResolution, FrozenAutoWorkWorkspace,
};

/// Lease is renewed on this cadence while a turn is in flight.
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(30);
/// Idle cadence for a persistent loop with nothing to do (tag drained, claim
/// error, or a paused queue). The `wake` Notify makes a freshly
/// created/re-pended requirement claim near-instantly; this is the safety-net
/// poll for anything the waker cannot observe.
const IDLE_POLL: Duration = Duration::from_secs(10);
/// API callers wait only this long for deterministic target cleanup. The
/// cleanup task itself remains owned and continues after the waiter returns.
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
/// One global shutdown ceiling for coordinator joins and every target cleanup.
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Shared dependencies for all AutoWork loops.
pub struct AutoWorkRunnerDeps {
    /// Canonical installation owner. AutoWork is part of the installation-wide
    /// Requirements control plane and may only resume or drive this user's
    /// AgentSession targets.
    pub authoritative_user_id: Arc<str>,
    pub service: Arc<RequirementService>,
    /// The only Agent work entrypoint. AgentExecution owns every Attempt,
    /// AgentSession turn receipt, retry and recovery transition.
    pub execution: Arc<dyn AutoWorkExecutionPort>,
    /// Resolves only the exact owner-scoped workspace frozen in the target
    /// AgentSession. Attachment staging must never infer this from data_dir,
    /// process cwd, environment variables, or legacy Conversation metadata.
    pub workspace: Arc<dyn AutoWorkWorkspacePort>,
    /// Notified whenever a requirement becomes claimable. Idle loops await this
    /// (with `IDLE_POLL` as a fallback) so newly created/re-pended work is picked
    /// up immediately. Shared with the `RequirementService` that fires it.
    pub wake: Arc<tokio::sync::Notify>,
}

/// Live progress for one AutoWork loop, shared between the loop task and the
/// API (`get_autowork`). Read by `AutoWorkRunner::live_progress`.
#[derive(Clone)]
struct LiveClaim {
    requirement_id: String,
    claim_generation: i64,
    /// Opaque execution capability. Intentionally has no `Debug` derive and is
    /// never projected through `live_progress` or tracing fields.
    claim_token: String,
}

#[derive(Default)]
struct LiveProgress {
    current_claim: Mutex<Option<LiveClaim>>,
    completed_count: AtomicU32,
}

impl LiveProgress {
    fn set_current(&self, claim: Option<LiveClaim>) {
        *self.current_claim.lock().expect("progress lock") = claim;
    }
    fn current(&self) -> Option<String> {
        self.current_claim
            .lock()
            .expect("progress lock")
            .as_ref()
            .map(|claim| claim.requirement_id.clone())
    }
    fn current_claim(&self) -> Option<LiveClaim> {
        self.current_claim.lock().expect("progress lock").clone()
    }
    fn incr_completed(&self) -> u32 {
        self.completed_count.fetch_add(1, Ordering::SeqCst) + 1
    }
    fn completed(&self) -> u32 {
        self.completed_count.load(Ordering::SeqCst)
    }
}

impl LiveClaim {
    fn execution_source(&self) -> AutoWorkExecutionSource {
        AutoWorkExecutionSource {
            requirement_id: self.requirement_id.clone(),
            claim_generation: self.claim_generation,
            operation_id: autowork_execution_operation_id(
                &self.requirement_id,
                self.claim_generation,
                &self.claim_token,
            ),
        }
    }
}

/// Domain-qualified key for the per-target loop maps. Business identifiers are
/// intentionally unprefixed UUIDv7 values, so the loop registry MUST key on
/// `(kind, target_id)` rather than assuming the identifier alone conveys its
/// domain.
type TargetKey = (AutoWorkTargetKind, String);
type TargetTransitionMap = DashMap<TargetKey, Arc<tokio::sync::Mutex<()>>>;

fn target_transition_lock(
    transitions: &TargetTransitionMap,
    key: &TargetKey,
) -> Arc<tokio::sync::Mutex<()>> {
    transitions
        .entry(key.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

struct AutoWorkHandle {
    /// Cooperative cancel flag, checked between turns by the loop.
    cancelled: Arc<AtomicBool>,
    join: tokio::task::JoinHandle<()>,
    tag: String,
    max_requirements: Option<u32>,
    /// Live progress (current requirement + completed count).
    progress: Arc<LiveProgress>,
    /// Monotonic id distinguishing this loop instance from a later restart on
    /// the same target, so a naturally-exiting loop only removes its own entry
    /// (not a fresh one a concurrent `start()` just inserted).
    generation: u64,
    /// Explicit `stop_locked` takes responsibility for deterministic cleanup,
    /// while process quiescence deliberately leaves durable work recoverable;
    /// both set this before aborting. Every other task drop (panic, external
    /// cancellation, runtime owner drop) is closed by `HandleGuard` instead.
    cleanup_handoff: Arc<AtomicBool>,
    /// Orders an asynchronously spawned Drop custodian against a later explicit
    /// stop/restart transition for the same handle generation.
    cleanup_barrier: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoWorkStartOutcome {
    Started,
    Restarted,
    AlreadyRunning,
    StalePersistedConfig,
    CleanupPending,
    ShuttingDown,
    InvalidTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoWorkStopOutcome {
    AlreadyStopped,
    Stopped,
    CleanupPending,
}

#[derive(Default)]
struct CleanupCompletion {
    done: AtomicBool,
    notify: Notify,
}

impl CleanupCompletion {
    fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.done.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

struct CleanupOwner {
    generation: u64,
    completion: Arc<CleanupCompletion>,
    join: JoinHandle<()>,
}

#[derive(Default)]
struct CoordinatorTasks {
    sweeper: Option<JoinHandle<()>>,
    resume: Option<JoinHandle<()>>,
}

/// Removes a loop's handle from the map on task exit —normal OR panic (Drop runs
/// during unwind). The generation guard prevents clobbering a fresh handle that a
/// concurrent `start()` may have inserted.
struct HandleGuard {
    handles: Arc<DashMap<TargetKey, AutoWorkHandle>>,
    key: TargetKey,
    generation: u64,
    deps: Arc<AutoWorkRunnerDeps>,
    tag: String,
    cleanup_handoff: Arc<AtomicBool>,
    cleanup_barrier: Arc<tokio::sync::Mutex<()>>,
    process_shutdown: CancellationToken,
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if self.cleanup_handoff.load(Ordering::SeqCst) || self.process_shutdown.is_cancelled() {
            self.handles
                .remove_if(&self.key, |_, h| h.generation == self.generation);
            return;
        }

        // A task can be dropped after its claim COMMIT but before publishing
        // process-local progress. Keep the handle installed (therefore blocking
        // a replacement start) until an independent task has inspected the
        // durable typed owner and closed the exact claim from receipt evidence.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // During runtime destruction no async cleanup can be driven. The
            // durable claim/receipt remains absorbing and boot recovery will
            // close it; never attempt a synchronous or best-effort unclaim.
            self.handles
                .remove_if(&self.key, |_, h| h.generation == self.generation);
            return;
        };
        let handles = self.handles.clone();
        let key = self.key.clone();
        let generation = self.generation;
        let deps = self.deps.clone();
        let tag = self.tag.clone();
        let cleanup_handoff = self.cleanup_handoff.clone();
        let cleanup_barrier = self.cleanup_barrier.clone();
        let process_shutdown = self.process_shutdown.clone();
        runtime.spawn(async move {
            let _cleanup_guard = cleanup_barrier.lock().await;
            // A racing explicit stop owns deterministic cleanup and holds the
            // start/stop transition barrier. Once handed off, this background
            // task must not release a replacement execution generation.
            if !cleanup_handoff.load(Ordering::SeqCst) && !process_shutdown.is_cancelled() {
                cleanup_abandoned_loop_claim(&deps, key.0, &key.1, &tag).await;
            }
            handles.remove_if(&key, |_, h| h.generation == generation);
        });
    }
}

/// Drives per-session AutoWork loops and the lease sweeper.
#[derive(Clone)]
pub struct AutoWorkRunner {
    deps: Arc<AutoWorkRunnerDeps>,
    handles: Arc<DashMap<TargetKey, AutoWorkHandle>>,
    /// Serializes start/stop transitions per target. Removing a handle before
    /// its abort/receipt cleanup completes must not let a racing start install a
    /// replacement loop, and two concurrent starts must not both spawn.
    transitions: Arc<TargetTransitionMap>,
    /// Exact-claim cleanup owners outlive bounded API waiters. A replacement
    /// generation cannot start while its target remains in this map.
    cleanups: Arc<DashMap<TargetKey, CleanupOwner>>,
    next_generation: Arc<std::sync::atomic::AtomicU64>,
    /// Process-lifetime owner for the sweeper and boot-resume coordinators.
    /// Per-target loops keep their exact claim cleanup protocol in `handles`;
    /// this token only governs the two host-owned coordinator tasks.
    shutdown: CancellationToken,
    coordinators: Arc<Mutex<CoordinatorTasks>>,
}

impl AutoWorkRunner {
    pub fn new(deps: Arc<AutoWorkRunnerDeps>) -> Self {
        Self {
            deps,
            handles: Arc::new(DashMap::new()),
            transitions: Arc::new(DashMap::new()),
            cleanups: Arc::new(DashMap::new()),
            next_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            shutdown: CancellationToken::new(),
            coordinators: Arc::new(Mutex::new(CoordinatorTasks::default())),
        }
    }

    fn transition_lock(&self, key: &TargetKey) -> Arc<tokio::sync::Mutex<()>> {
        target_transition_lock(&self.transitions, key)
    }

    pub fn is_running(&self, kind: AutoWorkTargetKind, target_id: &str) -> bool {
        valid_target_id(kind, target_id)
            && self.handles.contains_key(&(kind, target_id.to_string()))
    }

    pub fn running_tag(&self, kind: AutoWorkTargetKind, target_id: &str) -> Option<String> {
        if !valid_target_id(kind, target_id) {
            return None;
        }
        self.handles.get(&(kind, target_id.to_string())).map(|h| h.tag.clone())
    }

    /// Live progress for a running loop: `(current_requirement_id, completed_count)`.
    pub fn live_progress(&self, kind: AutoWorkTargetKind, target_id: &str) -> Option<(Option<String>, u32)> {
        if !valid_target_id(kind, target_id) {
            return None;
        }
        self.handles
            .get(&(kind, target_id.to_string()))
            .map(|h| (h.progress.current(), h.progress.completed()))
    }

    /// Atomically reconcile persisted config and the live loop under the same
    /// per-target transition barrier.
    pub async fn apply_config(
        &self,
        owner_id: &str,
        kind: AutoWorkTargetKind,
        target_id: &str,
        config: AutoWorkConfig,
        expected_revision: Option<&str>,
        operation_id: Option<&str>,
    ) -> Result<AutoWorkConfigSnapshot, AppError> {
        let canonical = AutoWorkConfig::normalize(
            config.enabled,
            config.tag.as_deref(),
            config.max_requirements,
        )?;
        if canonical != config {
            return Err(AppError::BadRequest(
                "AutoWork config must use its canonical normalized tag".to_owned(),
            ));
        }
        if !valid_target_id(kind, target_id) {
            return Err(AppError::BadRequest(format!(
                "target_id is not a canonical {} ID",
                kind.as_str()
            )));
        }
        if self.shutdown.is_cancelled() {
            return Err(AppError::Conflict(
                "AutoWork runner is shutting down".to_owned(),
            ));
        }

        let key = (kind, target_id.to_owned());
        let transition = self.transition_lock(&key);
        let _transition_guard = transition.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(AppError::Conflict(
                "AutoWork runner is shutting down".to_owned(),
            ));
        }

        let current = self
            .deps
            .service
            .read_autowork_config_snapshot(owner_id, kind, target_id)
            .await?;
        let historical_replay_candidate =
            expected_revision.is_some_and(|revision| revision != current.revision);
        if historical_replay_candidate && operation_id.is_none() {
            return Err(AppError::Conflict(format!(
                "AutoWork config for {} {target_id} changed concurrently",
                kind.as_str()
            )));
        }

        // A semantic reconfiguration must quiesce the old generation before
        // publishing the new durable binding. An identical enable deliberately
        // leaves the active Requirement and AgentExecution untouched.
        if !historical_replay_candidate
            && config.enabled
            && !self.running_config_matches(&key, &config)
        {
            match self.stop_locked(kind, target_id).await {
                AutoWorkStopOutcome::CleanupPending => {
                    return Err(AppError::Conflict(format!(
                        "AutoWork cleanup for {} {target_id} is still in progress",
                        kind.as_str()
                    )));
                }
                AutoWorkStopOutcome::AlreadyStopped | AutoWorkStopOutcome::Stopped => {}
            }
        }

        let requested_revision = expected_revision.unwrap_or(&current.revision);
        let saved = self
            .deps
            .service
            .save_autowork_config(
                owner_id,
                kind,
                target_id,
                config.clone(),
                requested_revision,
                operation_id,
            )
            .await?;

        // The Store returns the immutable receipt for an idempotent operation,
        // including an A -> B -> replay(A) sequence. Reconcile the live loop
        // only when that receipt is still the current Store head; a historical
        // replay is a response, not a request to roll runtime state backwards.
        let persisted = self
            .deps
            .service
            .read_autowork_config_snapshot(owner_id, kind, target_id)
            .await?;
        if persisted != saved {
            return Ok(saved);
        }

        if config.enabled {
            let tag = config.enabled_tag()?.to_owned();
            if let Err(error) = self.deps.service.resume_tag_for_enable(&tag).await {
                warn!(
                    tag,
                    %error,
                    "AutoWork enable persisted, but shared tag resume failed"
                );
            }
            if self.start_snapshot_locked(kind, target_id, saved.clone()).await
                == AutoWorkStartOutcome::CleanupPending
            {
                return Err(AppError::Conflict(format!(
                    "AutoWork cleanup for {} {target_id} is still in progress",
                    kind.as_str()
                )));
            }
        } else if self.stop_locked(kind, target_id).await
            == AutoWorkStopOutcome::CleanupPending
        {
            return Err(AppError::Conflict(format!(
                "AutoWork was disabled, but cleanup for {} {target_id} is still in progress",
                kind.as_str()
            )));
        }

        Ok(saved)
    }

    /// Start AutoWork only when the supplied config still matches the latest
    /// persisted owner-scoped snapshot.
    pub async fn start(
        &self,
        kind: AutoWorkTargetKind,
        target_id: String,
        tag: String,
        max_requirements: Option<u32>,
    ) -> AutoWorkStartOutcome {
        if self.shutdown.is_cancelled() {
            return AutoWorkStartOutcome::ShuttingDown;
        }
        if !valid_target_id(kind, &target_id) {
            error!(target_id, ?kind, "Refusing to start AutoWork for an invalid target id");
            return AutoWorkStartOutcome::InvalidTarget;
        }
        let config = match AutoWorkConfig::normalize(true, Some(&tag), max_requirements) {
            Ok(config) => config,
            Err(error) => {
                warn!(target_id, ?kind, %error, "Refusing invalid AutoWork config");
                return AutoWorkStartOutcome::StalePersistedConfig;
            }
        };
        let key: TargetKey = (kind, target_id.clone());
        let transition = self.transition_lock(&key);
        let _transition_guard = transition.lock().await;
        if self.shutdown.is_cancelled() {
            return AutoWorkStartOutcome::ShuttingDown;
        }
        let snapshot = match self
            .deps
            .service
            .read_autowork_config_snapshot(
                &self.deps.authoritative_user_id,
                kind,
                &target_id,
            )
            .await
        {
            Ok(snapshot) if snapshot.config == config => snapshot,
            Ok(_) => return AutoWorkStartOutcome::StalePersistedConfig,
            Err(error) => {
                warn!(
                    target_id,
                    ?kind,
                    %error,
                    "Refusing to start AutoWork without a current persisted config"
                );
                return AutoWorkStartOutcome::StalePersistedConfig;
            }
        };
        self.start_snapshot_locked(kind, &target_id, snapshot).await
    }

    /// Caller must hold this target's `transition_lock`.
    async fn start_snapshot_locked(
        &self,
        kind: AutoWorkTargetKind,
        target_id: &str,
        snapshot: AutoWorkConfigSnapshot,
    ) -> AutoWorkStartOutcome {
        if self.shutdown.is_cancelled() {
            return AutoWorkStartOutcome::ShuttingDown;
        }
        let tag = match snapshot.config.enabled_tag() {
            Ok(tag) => tag.to_owned(),
            Err(_) => return AutoWorkStartOutcome::StalePersistedConfig,
        };
        let max_requirements = snapshot.config.max_requirements;
        let key = (kind, target_id.to_owned());
        if !self.wait_for_cleanup_locked(&key, STOP_WAIT_TIMEOUT).await {
            return AutoWorkStartOutcome::CleanupPending;
        }
        if let Some(handle) = self.handles.get(&key)
            && !handle.join.is_finished()
            && !handle.cancelled.load(Ordering::SeqCst)
            && handle.tag == tag
            && handle.max_requirements == max_requirements
        {
            return AutoWorkStartOutcome::AlreadyRunning;
        }
        let restarted = self.handles.contains_key(&key);
        if restarted
            && self.stop_locked(kind, target_id).await == AutoWorkStopOutcome::CleanupPending
        {
            return AutoWorkStartOutcome::CleanupPending;
        }

        // Share the coordinator lock with shutdown so the final check and
        // handle publication cannot cross shutdown's collection of live loops.
        // No await is allowed between this guard and publication.
        let _admission_guard = self.coordinators.lock().expect("AutoWork coordinator lock");
        if self.shutdown.is_cancelled() {
            return AutoWorkStartOutcome::ShuttingDown;
        }
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cleanup_handoff = Arc::new(AtomicBool::new(false));
        let cleanup_barrier = Arc::new(tokio::sync::Mutex::new(()));
        let progress = Arc::new(LiveProgress::default());
        let cancelled_for_task = cancelled.clone();
        let progress_for_task = progress.clone();
        let deps = self.deps.clone();
        let transitions = self.transitions.clone();
        let handles = self.handles.clone();
        let conv = target_id.to_owned();
        let loop_tag = tag.clone();
        let guard_key = key.clone();
        let cleanup_handoff_for_task = cleanup_handoff.clone();
        let cleanup_barrier_for_task = cleanup_barrier.clone();
        let process_shutdown_for_task = self.shutdown.clone();
        let guard_deps = deps.clone();
        let config_revision = snapshot.revision;
        let (published_tx, published_rx) = oneshot::channel();

        // The spawned task cannot construct its Drop guard or enter run_loop
        // until the handle has been published. This closes the ready-future race
        // where ownership validation could fail before `handles.insert`.
        let join = tokio::spawn(async move {
            if published_rx.await.is_err() {
                return;
            }
            // Drop runs on normal exit AND panic-unwind -> handle always removed.
            let _guard = HandleGuard {
                handles,
                key: guard_key,
                generation,
                deps: guard_deps,
                tag: loop_tag.clone(),
                cleanup_handoff: cleanup_handoff_for_task,
                cleanup_barrier: cleanup_barrier_for_task,
                process_shutdown: process_shutdown_for_task,
            };
            info!(target_id = %conv, ?kind, tag = %loop_tag, "AutoWork loop started");
            run_loop(
                deps,
                transitions,
                &conv,
                kind,
                &loop_tag,
                cancelled_for_task,
                progress_for_task,
                max_requirements,
                config_revision,
            )
            .await;
            info!(target_id = %conv, ?kind, tag = %loop_tag, "AutoWork loop exited");
        });

        self.handles.insert(
            key.clone(),
            AutoWorkHandle {
                cancelled,
                join,
                tag,
                max_requirements,
                progress,
                generation,
                cleanup_handoff,
                cleanup_barrier,
            },
        );
        if published_tx.send(()).is_err() {
            self.handles
                .remove_if(&key, |_, handle| handle.generation == generation);
            return AutoWorkStartOutcome::CleanupPending;
        }
        if restarted {
            AutoWorkStartOutcome::Restarted
        } else {
            AutoWorkStartOutcome::Started
        }
    }

    fn running_config_matches(&self, key: &TargetKey, config: &AutoWorkConfig) -> bool {
        self.handles.get(key).is_some_and(|handle| {
            !handle.join.is_finished()
                && !handle.cancelled.load(Ordering::SeqCst)
                && handle.tag.as_str() == config.tag.as_deref().unwrap_or_default()
                && handle.max_requirements == config.max_requirements
        })
    }

    /// Stop a session's loop. Sets the cancel flag, aborts the task, cancels
    /// the in-flight agent turn (conversation targets), and releases the
    /// in-flight claim (if any) back to `pending` so the requirement is not
    /// orphaned `in_progress` until the sweeper runs. Cancelling the live turn
    /// matters: disabling AutoWork must actually stop the work —historically
    /// the orphan turn kept the conversation showing "running" after the user
    /// flipped the switch off, and raced any later re-enable.
    pub async fn stop(
        &self,
        kind: AutoWorkTargetKind,
        target_id: &str,
    ) -> AutoWorkStopOutcome {
        if !valid_target_id(kind, target_id) {
            return AutoWorkStopOutcome::AlreadyStopped;
        }
        let key = (kind, target_id.to_string());
        let transition = self.transition_lock(&key);
        let _transition_guard = transition.lock().await;
        self.stop_locked(kind, target_id).await
    }

    /// Strict deletion owner: no cleanup error is downgraded to a warning.
    /// The canonical Session delete saga calls this after its admission fence
    /// and before inspecting effect blockers.
    pub async fn stop_for_session_delete(&self, target_id: &str) -> Result<(), AppError> {
        if !valid_target_id(AutoWorkTargetKind::Conversation, target_id) {
            return Err(AppError::BadRequest(
                "AgentSession delete received a non-canonical AutoWork target".to_owned(),
            ));
        }
        let key = (AutoWorkTargetKind::Conversation, target_id.to_owned());
        let transition = self.transition_lock(&key);
        let _transition_guard = transition.lock().await;
        if !self.wait_for_cleanup_locked(&key, SHUTDOWN_WAIT_TIMEOUT).await {
            return Err(AppError::Conflict(
                "AutoWork cleanup owner did not settle before Session deletion".to_owned(),
            ));
        }
        if let Some((_, handle)) = self.handles.remove(&key) {
            handle.cancelled.store(true, Ordering::SeqCst);
            handle.cleanup_handoff.store(true, Ordering::SeqCst);
            let cleanup_barrier = handle.cleanup_barrier.clone();
            handle.join.abort();
            let _ = handle.join.await;
            let _cleanup_guard = timeout(SHUTDOWN_WAIT_TIMEOUT, cleanup_barrier.lock())
                .await
                .map_err(|_| {
                    AppError::Conflict(
                        "AutoWork cleanup barrier did not settle before Session deletion"
                            .to_owned(),
                    )
                })?;
        }

        let parked = self
            .deps
            .service
            .park_owner_for_session_delete(
                target_id,
                AutoWorkTargetKind::Conversation,
            )
            .await?;
        for source in parked.execution_sources {
            self.deps
                .execution
                .cancel_automation(&self.deps.authoritative_user_id, &source)
                .await?;
        }
        Ok(())
    }

    /// Caller must hold this target's `transition_lock`.
    async fn stop_locked(
        &self,
        kind: AutoWorkTargetKind,
        target_id: &str,
    ) -> AutoWorkStopOutcome {
        let key = (kind, target_id.to_owned());
        if !self.wait_for_cleanup_locked(&key, STOP_WAIT_TIMEOUT).await {
            return AutoWorkStopOutcome::CleanupPending;
        }
        let Some((_, handle)) = self.handles.remove(&key) else {
            return AutoWorkStopOutcome::AlreadyStopped;
        };
        handle.cancelled.store(true, Ordering::SeqCst);
        handle.cleanup_handoff.store(true, Ordering::SeqCst);
        handle.join.abort();

        let completion = self.spawn_cleanup_owner_locked(key, handle);
        if timeout(STOP_WAIT_TIMEOUT, completion.wait()).await.is_ok() {
            AutoWorkStopOutcome::Stopped
        } else {
            AutoWorkStopOutcome::CleanupPending
        }
    }

    /// Caller must hold this target's `transition_lock`.
    async fn wait_for_cleanup_locked(&self, key: &TargetKey, wait: Duration) -> bool {
        let Some(completion) = self
            .cleanups
            .get(key)
            .map(|owner| owner.completion.clone())
        else {
            return true;
        };
        timeout(wait, completion.wait()).await.is_ok()
    }

    /// Caller must hold this target's `transition_lock`.
    fn spawn_cleanup_owner_locked(
        &self,
        key: TargetKey,
        handle: AutoWorkHandle,
    ) -> Arc<CleanupCompletion> {
        let completion = Arc::new(CleanupCompletion::default());
        let completion_for_task = completion.clone();
        let this = self.clone();
        let cleanups = self.cleanups.clone();
        let cleanup_key = key.clone();
        let generation = handle.generation;
        let kind = key.0;
        let target_id = key.1.clone();
        let (published_tx, published_rx) = oneshot::channel();
        let join = tokio::spawn(async move {
            if published_rx.await.is_err() {
                completion_for_task.finish();
                return;
            }
            this.cleanup_stopped_handle(kind, &target_id, handle)
                .await;
            completion_for_task.finish();
            cleanups.remove_if(&cleanup_key, |_, owner| {
                owner.generation == generation
            });
        });
        self.cleanups.insert(
            key.clone(),
            CleanupOwner {
                generation,
                completion: completion.clone(),
                join,
            },
        );
        if published_tx.send(()).is_err() {
            completion.finish();
            self.cleanups.remove_if(&key, |_, owner| {
                owner.generation == generation
            });
        }
        completion
    }

    async fn cleanup_stopped_handle(
        &self,
        kind: AutoWorkTargetKind,
        target_id: &str,
        handle: AutoWorkHandle,
    ) {
        let _ = handle.join.await;
        let _cleanup_guard = handle.cleanup_barrier.lock().await;
        let published_claim = handle.progress.current_claim();
        let recovered_claim = match self
            .deps
            .service
            .recover_active_claim_for_runner(
                &handle.tag,
                target_id,
                kind,
                DEFAULT_LEASE_MS,
            )
            .await
        {
            Ok(claim) => claim.map(|claim| LiveClaim {
                requirement_id: claim.requirement.requirement_id,
                claim_generation: claim.claim_generation,
                claim_token: claim.claim_token,
            }),
            Err(error) => {
                warn!(
                    target_id,
                    error = %error,
                    "Failed to recover a just-committed AutoWork claim during stop"
                );
                None
            }
        };
        if let Some(active_claim) = recovered_claim.or(published_claim)
            && let Err(error) = settle_stopped_agent_execution_claim(
                &self.deps,
                target_id,
                &handle.tag,
                &active_claim,
                "AutoWork was stopped.",
            )
            .await
        {
            warn!(
                requirement_id = active_claim.requirement_id,
                claim_generation = active_claim.claim_generation,
                error = %error,
                "Failed to settle AgentExecution-backed claim on AutoWork stop"
            );
        }
    }

    /// Spawn the lease sweeper: every 60s, park `in_progress` requirements whose
    /// lease expired and whose owning session is not a live AutoWork loop here.
    /// Expiry alone never makes an execution safe to repeat.
    /// Detached for the process lifetime (the runner lives in router state).
    pub fn start_sweeper(&self) {
        let mut coordinators = self
            .coordinators
            .lock()
            .expect("AutoWork coordinator lock");
        if self.shutdown.is_cancelled()
            || coordinators
                .sweeper
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let _ = coordinators.sweeper.take();
        let handles = self.handles.clone();
        let service = self.deps.service.clone();
        let shutdown = self.shutdown.clone();
        let task = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(60));
            ticker.tick().await; // consume the immediate first tick
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {}
                }
                if shutdown.is_cancelled() {
                    break;
                }
                // The active set is keyed by `(kind, target_id)`. The sweep
                // matches each typed owner column against its corresponding
                // canonical string-ID set.
                let active_conversations: Vec<String> = handles
                    .iter()
                    .filter(|entry| entry.key().0 == AutoWorkTargetKind::Conversation)
                    .map(|entry| entry.key().1.clone())
                    .collect();
                match service
                    .repo()
                    .sweep_expired_leases(
                        &active_conversations,
                        &[],
                        nomifun_common::now_ms(),
                    )
                    .await
                {
                    Ok(n) if n > 0 => {
                        info!(parked = n, "Requirement lease sweeper parked ambiguous claims")
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "Requirement lease sweeper failed"),
                }
            }
        });
        coordinators.sweeper = Some(task);
    }

    /// Resume every persisted-enabled AgentSession AutoWork binding at boot.
    ///
    /// The running set (`handles`) is in-memory, but the enabled/tag config is
    /// persisted by the canonical AgentSession owner. On
    /// a process restart nothing would drive those bindings until a user opened
    /// each session page —the old behaviour that made AutoWork look like it
    /// "only works in the foreground". Spawning the loops here makes the backend
    /// the single source of truth: a bound session works in the background from
    /// boot, no UI visit required. Detached + best-effort.
    pub fn resume_persisted_bindings(&self) {
        let mut coordinators = self
            .coordinators
            .lock()
            .expect("AutoWork coordinator lock");
        if self.shutdown.is_cancelled()
            || coordinators
                .resume
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let _ = coordinators.resume.take();
        let this = self.clone();
        let shutdown = self.shutdown.clone();
        let task = tokio::spawn(async move {
            let mut resumed = 0usize;
            let owner_id = this.deps.authoritative_user_id.clone();
            let bindings = match tokio::select! {
                _ = shutdown.cancelled() => return,
                result = this.deps.service.list_enabled_autowork_bindings(&owner_id) => result,
            } {
                Ok(bindings) => bindings,
                Err(error) => {
                    warn!(
                        user_id = %owner_id,
                        %error,
                        "AutoWork resume: persisted binding lookup failed"
                    );
                    return;
                }
            };
            for binding in bindings {
                if shutdown.is_cancelled() {
                    return;
                }
                if this.resume_binding_if_current(binding).await {
                    resumed += 1;
                }
            }
            if resumed > 0 {
                info!(resumed, "AutoWork resumed persisted bindings on boot");
            }
        });
        coordinators.resume = Some(task);
    }

    async fn resume_binding_if_current(
        &self,
        binding: crate::execution_port::PersistedAutoWorkBinding,
    ) -> bool {
        let key = (binding.kind, binding.target_id.clone());
        let transition = self.transition_lock(&key);
        let _transition_guard = tokio::select! {
            _ = self.shutdown.cancelled() => return false,
            guard = transition.lock() => guard,
        };
        let latest = match tokio::select! {
            _ = self.shutdown.cancelled() => return false,
            result = self.deps.service.read_autowork_config_snapshot(
                &self.deps.authoritative_user_id,
                binding.kind,
                &binding.target_id,
            ) => result,
        } {
            Ok(snapshot) => snapshot,
            Err(error) => {
                warn!(
                    target_id = binding.target_id,
                    ?binding.kind,
                    %error,
                    "AutoWork boot resume quarantined a binding that could not be re-read"
                );
                return false;
            }
        };
        let still_current = latest.config.enabled
            && latest.config.tag.as_deref() == Some(binding.tag.as_str())
            && latest.config.max_requirements == binding.max_requirements
            && latest.revision == binding.config_revision;
        if !still_current {
            debug!(
                target_id = binding.target_id,
                ?binding.kind,
                "AutoWork boot snapshot became stale before its transition lock"
            );
            return false;
        }
        matches!(
            self.start_snapshot_locked(binding.kind, &binding.target_id, latest)
                .await,
            AutoWorkStartOutcome::Started | AutoWorkStartOutcome::Restarted
        )
    }

    /// Quiesce process-owned coordinators, target waiters, and lease renewal
    /// before SQLite closes. This is deliberately not the user-facing
    /// [`Self::stop`] command: durable Planning/Running AgentExecutions and
    /// their exact Requirement claims remain recoverable for the next process
    /// and are never semantically cancelled during host shutdown.
    pub async fn quiesce(&self) -> Result<(), String> {
        self.shutdown.cancel();
        let deadline = Instant::now() + SHUTDOWN_WAIT_TIMEOUT;

        let (sweeper, resume) = {
            let mut coordinators = self
                .coordinators
                .lock()
                .expect("AutoWork coordinator lock");
            (coordinators.sweeper.take(), coordinators.resume.take())
        };
        let mut errors = Vec::new();
        for (name, task) in [("sweeper", sweeper), ("boot resume", resume)] {
            let Some(mut task) = task else {
                continue;
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                task.abort();
                errors.push(format!("AutoWork {name} shutdown timed out"));
                continue;
            }
            match timeout(remaining, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if error.is_cancelled() => {}
                Ok(Err(error)) => errors.push(format!("AutoWork {name} join failed: {error}")),
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    errors.push(format!("AutoWork {name} shutdown timed out"));
                }
            }
        }

        let keys = self
            .handles
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for key in keys {
            let kind = key.0;
            let target_id = key.1.clone();
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                errors.push("AutoWork target quiesce deadline elapsed".to_owned());
                break;
            }
            let transition = self.transition_lock(&key);
            let transition_guard = match timeout(remaining, transition.lock()).await {
                Ok(guard) => guard,
                Err(_) => {
                    errors.push(format!(
                        "AutoWork {} {target_id} quiesce barrier timed out",
                        kind.as_str()
                    ));
                    break;
                }
            };
            let Some((_, handle)) = self.handles.remove(&key) else {
                drop(transition_guard);
                continue;
            };
            // Fence HandleGuard before aborting: process quiescence must not
            // enter explicit-stop cleanup and cancel the durable execution.
            handle.cleanup_handoff.store(true, Ordering::SeqCst);
            handle.cancelled.store(true, Ordering::SeqCst);
            let mut join = handle.join;
            join.abort();
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || timeout(remaining, &mut join).await.is_err() {
                join.abort();
                errors.push(format!(
                    "AutoWork {} {target_id} waiter did not quiesce",
                    kind.as_str()
                ));
            }
            drop(transition_guard);
        }

        while !self.cleanups.is_empty() && Instant::now() < deadline {
            sleep(Duration::from_millis(20)).await;
        }
        if !self.handles.is_empty() {
            errors.push(format!(
                "AutoWork retained {} target loop(s) after quiesce",
                self.handles.len()
            ));
        }
        if !self.cleanups.is_empty() {
            let unfinished = self
                .cleanups
                .iter()
                .filter(|owner| !owner.join.is_finished())
                .count();
            errors.push(format!(
                "AutoWork retained {} exact cleanup owner(s), {unfinished} still running",
                self.cleanups.len()
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

fn valid_target_id(kind: AutoWorkTargetKind, target_id: &str) -> bool {
    match kind {
        AutoWorkTargetKind::Conversation => ConversationId::try_from(target_id).is_ok(),
        AutoWorkTargetKind::Terminal => false,
    }
}

/// The autowork loop body. Claims -> injects -> waits -> finalizes -> repeats.
///
/// The loop is *persistent*: it does NOT exit when the tag drains or a claim
/// errors —it idles (waking on `deps.wake`, with `IDLE_POLL` as a fallback) and
/// keeps claiming, so a bound session keeps picking up new requirements in the
/// background forever. It exits only on cancel (disable / stop), after
/// `max_requirements` completions, or when its AgentSession is deleted.
/// Outcome of one claimed requirement's turn, used to drive the failure backoff.
/// Apply one exact claim verdict. `Ok(None)` from the write is not success: it
/// must be followed by an exact terminal-state re-confirmation, which covers
/// the legitimate case where the claim's declaration tool committed first.
#[allow(clippy::too_many_arguments)]
async fn resolve_claim_verdict_required(
    service: &RequirementService,
    requirement_id: &str,
    claim_generation: i64,
    claim_token: &str,
    owner_id: &str,
    kind: AutoWorkTargetKind,
    status: RequirementStatus,
    note: Option<String>,
) -> Result<(), AppError> {
    match service
        .resolve_claim_verdict_exact(
            requirement_id,
            claim_generation,
            claim_token,
            owner_id,
            kind,
            status,
            note,
        )
        .await?
    {
        Some(updated) if updated.status == status => Ok(()),
        Some(updated) => Err(AppError::Conflict(format!(
            "exact AutoWork claim generation {claim_generation} returned status '{}' \
             instead of '{}' for requirement {requirement_id}",
            updated.status.as_db(),
            status.as_db(),
        ))),
        None => match service
            .confirm_claim_verdict_exact(
                requirement_id,
                claim_generation,
                claim_token,
                owner_id,
                kind,
                status,
            )
            .await?
        {
            true => Ok(()),
            false => Err(AppError::Conflict(format!(
                "exact AutoWork claim generation {claim_generation} lost authority before \
                 '{}' verdict for requirement {requirement_id}; no exact terminal winner \
                 could be confirmed",
                status.as_db(),
            ))),
        },
    }
}

async fn resolve_interrupted_agent_execution_claim(
    deps: &AutoWorkRunnerDeps,
    requirement_id: &str,
    conversation_id: &str,
    claim_generation: i64,
    claim_token: &str,
    reason: &str,
) -> Result<(), AppError> {
    match deps
        .service
        .release_claim_exact(
            requirement_id,
            conversation_id,
            claim_generation,
            claim_token,
        )
        .await
    {
        Ok(true) => {}
        result @ (Ok(false) | Err(_)) => {
            let detail = match result {
                Ok(false) => format!(
                    "{reason} Requirement claim generation {claim_generation} could not be \
                     released after its AgentExecution was cancelled; it will not be dispatched again."
                ),
                Err(error) => format!(
                    "{reason} Exact release for AgentExecution-backed claim generation \
                     {claim_generation} failed: {error}. It will not be delivered again."
                ),
                Ok(true) => unreachable!(),
            };
            resolve_claim_verdict_required(
                &deps.service,
                    requirement_id,
                    claim_generation,
                    claim_token,
                    conversation_id,
                    AutoWorkTargetKind::Conversation,
                    RequirementStatus::NeedsReview,
                    Some(detail),
            )
            .await?;
        }
    }
    Ok(())
}

async fn settle_stopped_agent_execution_claim(
    deps: &AutoWorkRunnerDeps,
    session_id: &str,
    tag: &str,
    claim: &LiveClaim,
    reason: &str,
) -> Result<(), AppError> {
    let receipt = deps
        .execution
        .cancel_automation(
            &deps.authoritative_user_id,
            &claim.execution_source(),
        )
        .await;
    match receipt {
        // No aggregate exists: cancellation won before AgentExecution
        // admission, so this exact Requirement generation is safe to release.
        Ok(None) => {
            resolve_interrupted_agent_execution_claim(
                deps,
                &claim.requirement_id,
                session_id,
                claim.claim_generation,
                &claim.claim_token,
                reason,
            )
            .await
        }
        Ok(Some(AutoWorkExecutionReceipt::Cancelled {
            replay_safe: true,
            ..
        })) => {
            resolve_interrupted_agent_execution_claim(
                deps,
                &claim.requirement_id,
                session_id,
                claim.claim_generation,
                &claim.claim_token,
                reason,
            )
            .await
        }
        Ok(Some(AutoWorkExecutionReceipt::Cancelled {
            execution_id,
            replay_safe: false,
        })) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::NeedsReview,
                Some(format!(
                    "{reason} AgentExecution {execution_id} was cancelled after an Attempt was admitted; effects cannot be proven absent and this Requirement will not be replayed."
                )),
            )
            .await?;
            deps.service
                .pause_for_execution_attention(
                    &claim.requirement_id,
                    tag,
                    "execution_failed",
                )
                .await
        }
        Ok(Some(AutoWorkExecutionReceipt::Completed {
            execution_id,
            summary,
        })) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::Done,
                summary.or_else(|| Some(format!("AgentExecution {execution_id} completed."))),
            )
            .await
        }
        Ok(Some(AutoWorkExecutionReceipt::CompletedWithFailures {
            execution_id,
            summary,
        })) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::NeedsReview,
                summary.or_else(|| {
                    Some(format!(
                        "AgentExecution {execution_id} completed with failures."
                    ))
                }),
            )
            .await?;
            deps.service
                .pause_for_execution_attention(
                    &claim.requirement_id,
                    tag,
                    "execution_failed",
                )
                .await
        }
        Ok(Some(AutoWorkExecutionReceipt::Failed {
            execution_id,
            error,
        })) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::Failed,
                error.or_else(|| Some(format!("AgentExecution {execution_id} failed."))),
            )
            .await?;
            deps.service
                .pause_for_execution_attention(
                    &claim.requirement_id,
                    tag,
                    "execution_failed",
                )
                .await
        }
        Ok(Some(AutoWorkExecutionReceipt::OutcomeUnknown {
            execution_id,
            error,
        })) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::NeedsReview,
                error.or_else(|| {
                    Some(format!(
                        "AgentExecution {execution_id} ended without a canonical turn receipt; prior effects were not replayed."
                    ))
                }),
            )
            .await?;
            deps.service
                .pause_for_execution_attention(
                    &claim.requirement_id,
                    tag,
                    "execution_failed",
                )
                .await
        }
        Err(error) => {
            resolve_claim_verdict_required(
                &deps.service,
                &claim.requirement_id,
                claim.claim_generation,
                &claim.claim_token,
                session_id,
                AutoWorkTargetKind::Conversation,
                RequirementStatus::NeedsReview,
                Some(format!(
                    "{reason} The canonical AgentExecution could not be settled: {error}. Prior effects were not replayed."
                )),
            )
            .await
        }
    }
}

async fn pause_and_resolve_user_interruption(
    deps: &AutoWorkRunnerDeps,
    requirement_id: &str,
    conversation_id: &str,
    claim_generation: i64,
    claim_token: &str,
    tag: &str,
) -> Result<(), AppError> {
    deps.service
        .pause_for_user_interrupt(requirement_id, tag)
        .await?;
    resolve_interrupted_agent_execution_claim(
        deps,
        requirement_id,
        conversation_id,
        claim_generation,
        claim_token,
        "The user interrupted AutoWork.",
    )
    .await
}

/// Independent custodian for a loop task that ended without going through
/// `stop_locked` (panic, external abort, or owner future drop). Recovery reads
/// only an already-active typed owner; it never allocates new work.
async fn cleanup_abandoned_loop_claim(
    deps: &AutoWorkRunnerDeps,
    kind: AutoWorkTargetKind,
    target_id: &str,
    tag: &str,
) {
    let claim = match deps
        .service
        .recover_active_claim_for_runner(tag, target_id, kind, DEFAULT_LEASE_MS)
        .await
    {
        Ok(claim) => claim,
        Err(error) => {
            warn!(
                target_id,
                ?kind,
                error = %error,
                "Unable to recover an abandoned AutoWork claim for durable cleanup"
            );
            return;
        }
    };
    let Some(claim) = claim else {
        return;
    };
    let requirement_id = claim.requirement.requirement_id;
    let claim_generation = claim.claim_generation;
    let claim_token = claim.claim_token;

    if kind != AutoWorkTargetKind::Conversation {
        return;
    }
    let active_claim = LiveClaim {
        requirement_id,
        claim_generation,
        claim_token,
    };
    if let Err(error) = settle_stopped_agent_execution_claim(
        deps,
        target_id,
        tag,
        &active_claim,
        "The AutoWork loop ended without a normal completion boundary.",
    )
    .await
    {
        warn!(
            agent_session_id = target_id,
            requirement_id = active_claim.requirement_id,
            claim_generation = active_claim.claim_generation,
            error = %error,
            "Failed to settle an abandoned AgentExecution claim"
        );
    }
}

enum TurnResult {
    /// Turn finished and finalized as done.
    Done,
    /// AgentExecution reached a failed terminal state and queue policy paused.
    Errored,
    /// The USER deliberately stopped the turn (conversation cancel). The tag
    /// was paused (`user_interrupted`) and the claim released without consuming
    /// an attempt —the loop idles until the user resumes the tag. NOT a
    /// failure and never manufactures another execution generation.
    UserInterrupted,
    /// The exact claim verdict/cleanup could not be committed or proven.
    /// Stop this loop rather than claiming another requirement under
    /// authority uncertainty.
    Blocked,
}


/// Broadcast this loop target's live AutoWork run-state so EVERY surface stays in
/// sync across idle→active transitions. The per-session control GETs fresh state
/// on open, but the session-list capability icon updates ONLY from this event (no
/// per-row GET); without an emit on claim/finish it kept the run-state from its
/// initial bulk load and showed a stale colour —active/green in the header but
/// idle/orange in the sidebar for the same session. `enabled=false` is emitted
/// when the max-requirements cap just disabled the binding so the icon drops off.
fn emit_autowork_progress(
    deps: &AutoWorkRunnerDeps,
    kind: AutoWorkTargetKind,
    target_id: &str,
    tag: &str,
    progress: &LiveProgress,
    enabled: bool,
) {
    let current_requirement_id = progress.current();
    deps.service.emit_autowork_state(&AutoWorkState {
        kind,
        target_id: target_id.to_string(),
        enabled,
        tag: Some(tag.to_string()),
        running: enabled,
        run_state: AutoWorkState::run_state(enabled, current_requirement_id.as_deref()),
        current_requirement_id,
        completed_count: progress.completed(),
    });
}

async fn run_loop(
    deps: Arc<AutoWorkRunnerDeps>,
    transitions: Arc<TargetTransitionMap>,
    target_id: &str,
    kind: AutoWorkTargetKind,
    tag: &str,
    cancelled: Arc<AtomicBool>,
    progress: Arc<LiveProgress>,
    max_requirements: Option<u32>,
    config_revision: String,
) {
    let owner_id = target_id;
    if kind != AutoWorkTargetKind::Conversation {
        warn!(target_id, "Terminal AutoWork was retired; bind an AgentPreset Session instead");
        return;
    }
    if let Err(error) = deps
        .workspace
        .resolve_frozen_workspace(&deps.authoritative_user_id, owner_id)
        .await
    {
        warn!(target_id, %error, "AutoWork AgentSession is not installation-owner owned");
        return;
    }
    // Count of back-to-back failed/busy turns, driving the failure backoff so a
    // deterministic failure cannot spin into claim at millisecond speed. Reset on
    // a clean done or when the tag drains (idle).
    let mut consecutive_failures: u32 = 0;
    loop {
        // Cancellation check before each claim.
        if cancelled.load(Ordering::SeqCst) {
            break;
        }

        // Claim the next requirement. The wake future is armed BEFORE the claim
        // (and dropped right after) so a requirement created/re-pended between the
        // claim returning None and our await is never lost. On drain or a transient
        // error the loop idles and retries instead of exiting —persistent by design.
        let wake = deps.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let claimed = match deps
            .service
            .claim_next_for_runner(tag, owner_id, kind, DEFAULT_LEASE_MS)
            .await
        {
            Ok(Some(claim)) => claim,
            Ok(None) => {
                consecutive_failures = 0;
                tokio::select! {
                    _ = wake.as_mut() => {}
                    _ = sleep(IDLE_POLL) => {}
                }
                continue;
            }
            Err(error) => {
                warn!(target_id, tag, %error, "AutoWork claim failed —retrying");
                tokio::select! {
                    _ = wake.as_mut() => {}
                    _ = sleep(IDLE_POLL) => {}
                }
                continue;
            }
        };
        let claim_generation = claimed.claim_generation;
        let claim_token = claimed.claim_token;
        let recovered_active = claimed.recovered_active;
        let claimed = claimed.requirement;
        let req_id = claimed.requirement_id.clone();
        progress.set_current(Some(LiveClaim {
            requirement_id: req_id.clone(),
            claim_generation,
            claim_token: claim_token.clone(),
        }));
        info!(
            target_id,
            tag,
            requirement_id = %req_id,
            claim_generation,
            recovered_active,
            "AutoWork claimed requirement"
        );
        // active: a requirement is now in flight -> broadcast so the session-list
        // icon turns active-coloured in step with the per-session control.
        emit_autowork_progress(&deps, kind, target_id, tag, &progress, true);

        // 2. Inject + wait for the turn to finish (per target kind).
        let result = match kind {
            AutoWorkTargetKind::Conversation => {
                match execute_requirement(
                    &deps,
                    target_id,
                    tag,
                    &claimed,
                    claim_generation,
                    &claim_token,
                )
                .await
                {
                    Ok(AutoWorkExecutionReceipt::Completed {
                        execution_id,
                        summary,
                    }) => match resolve_claim_verdict_required(
                        &deps.service,
                        &req_id,
                        claim_generation,
                        &claim_token,
                        owner_id,
                        AutoWorkTargetKind::Conversation,
                        RequirementStatus::Done,
                        summary.or_else(|| Some(format!("AgentExecution {execution_id} completed."))),
                    )
                    .await
                    {
                        Ok(()) => TurnResult::Done,
                        Err(error) => {
                            error!(target_id, requirement_id = %req_id, %error, "Failed to project completed AgentExecution");
                            TurnResult::Blocked
                        }
                    },
                    Ok(AutoWorkExecutionReceipt::CompletedWithFailures {
                        execution_id,
                        summary,
                    }) => match resolve_claim_verdict_required(
                        &deps.service,
                        &req_id,
                        claim_generation,
                        &claim_token,
                        owner_id,
                        AutoWorkTargetKind::Conversation,
                        RequirementStatus::NeedsReview,
                        summary.or_else(|| Some(format!("AgentExecution {execution_id} completed with failures."))),
                    )
                    .await
                    {
                        Ok(()) => {
                            if let Err(error) = deps
                                .service
                                .pause_for_execution_attention(
                                    &req_id,
                                    tag,
                                    "execution_failed",
                                )
                                .await
                            {
                                warn!(tag, %error, "Failed to pause partially failed AutoWork execution");
                            }
                            TurnResult::Done
                        }
                        Err(error) => {
                            error!(target_id, requirement_id = %req_id, %error, "Failed to park partially failed AgentExecution");
                            TurnResult::Blocked
                        }
                    },
                    Ok(AutoWorkExecutionReceipt::Failed { execution_id, error: failure }) => {
                        let note = failure.unwrap_or_else(|| format!("AgentExecution {execution_id} failed."));
                        match resolve_claim_verdict_required(
                            &deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            owner_id,
                            AutoWorkTargetKind::Conversation,
                            RequirementStatus::Failed,
                            Some(note),
                        )
                        .await
                        {
                            Ok(()) => {
                                if let Err(error) = deps
                                    .service
                                    .pause_for_execution_attention(
                                        &req_id,
                                        tag,
                                        "execution_failed",
                                    )
                                    .await
                                {
                                    warn!(tag, %error, "Failed to pause failed AutoWork execution");
                                }
                                TurnResult::Errored
                            }
                            Err(error) => {
                                error!(target_id, requirement_id = %req_id, %error, "Failed to project failed AgentExecution");
                                TurnResult::Blocked
                            }
                        }
                    }
                    Ok(AutoWorkExecutionReceipt::OutcomeUnknown {
                        execution_id,
                        error: uncertainty,
                    }) => {
                        let note = uncertainty.unwrap_or_else(|| {
                            format!(
                                "AgentExecution {execution_id} ended without a canonical turn receipt; prior effects were not replayed."
                            )
                        });
                        match resolve_claim_verdict_required(
                            &deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            owner_id,
                            AutoWorkTargetKind::Conversation,
                            RequirementStatus::NeedsReview,
                            Some(note),
                        )
                        .await
                        {
                            Ok(()) => {
                                if let Err(error) = deps
                                    .service
                                    .pause_for_execution_attention(
                                        &req_id,
                                        tag,
                                        "execution_failed",
                                    )
                                    .await
                                {
                                    warn!(tag, %error, "Failed to pause outcome-unknown AutoWork execution");
                                }
                                TurnResult::Done
                            }
                            Err(error) => {
                                error!(target_id, requirement_id = %req_id, %error, "Failed to park outcome-unknown AgentExecution");
                                TurnResult::Blocked
                            }
                        }
                    }
                    Ok(AutoWorkExecutionReceipt::Cancelled {
                        replay_safe: true,
                        ..
                    }) => {
                        if let Err(error) = pause_and_resolve_user_interruption(
                            &deps,
                            &req_id,
                            owner_id,
                            claim_generation,
                            &claim_token,
                            tag,
                        )
                        .await
                        {
                            error!(target_id, requirement_id = %req_id, %error, "Failed to close cancelled AgentExecution claim");
                            TurnResult::Blocked
                        } else {
                            TurnResult::UserInterrupted
                        }
                    }
                    Ok(AutoWorkExecutionReceipt::Cancelled {
                        execution_id,
                        replay_safe: false,
                    }) => {
                        match resolve_claim_verdict_required(
                            &deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            owner_id,
                            AutoWorkTargetKind::Conversation,
                            RequirementStatus::NeedsReview,
                            Some(format!(
                                "AgentExecution {execution_id} was cancelled after an Attempt was admitted; effects cannot be proven absent and this Requirement will not be replayed."
                            )),
                        )
                        .await
                        {
                            Ok(()) => {
                                if let Err(error) = deps
                                    .service
                                    .pause_for_execution_attention(
                                        &req_id,
                                        tag,
                                        "execution_failed",
                                    )
                                    .await
                                {
                                    warn!(tag, %error, "Failed to pause cancelled ambiguous AutoWork execution");
                                }
                                TurnResult::Done
                            }
                            Err(error) => {
                                error!(target_id, requirement_id = %req_id, %error, "Failed to park cancelled ambiguous AgentExecution");
                                TurnResult::Blocked
                            }
                        }
                    }
                    Err(AppError::NotFound(not_found)) => {
                        warn!(
                            target_id,
                            requirement_id = %req_id,
                            error = %not_found,
                            "AutoWork AgentExecution target disappeared; parking the exact claim"
                        );
                        if let Err(error) = resolve_claim_verdict_required(
                            &deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            owner_id,
                            AutoWorkTargetKind::Conversation,
                            RequirementStatus::NeedsReview,
                            Some(format!(
                                "The bound AgentSession or AgentExecution disappeared: {not_found}. Prior effects were not replayed."
                            )),
                        )
                        .await
                        {
                            error!(
                                target_id,
                                requirement_id = %req_id,
                                error = %error,
                                "Disappeared execution claim could not be parked"
                            );
                        }
                        break;
                    }
                    Err(e) => {
                        error!(target_id, requirement_id = %req_id, error = %e, "AutoWork AgentExecution dispatch failed");
                        match resolve_claim_verdict_required(
                            &deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            owner_id,
                            AutoWorkTargetKind::Conversation,
                            RequirementStatus::NeedsReview,
                            Some(format!("AgentExecution dispatch failed: {e}")),
                        )
                        .await
                        {
                            Ok(()) => {
                                if let Err(pause_error) = deps
                                    .service
                                    .pause_for_execution_attention(
                                        &req_id,
                                        tag,
                                        "execution_failed",
                                    )
                                    .await
                                {
                                    warn!(tag, error = %pause_error, "Failed to pause rejected AutoWork dispatch");
                                }
                                TurnResult::Done
                            }
                            Err(error) => {
                                error!(target_id, requirement_id = %req_id, %error, "Failed to park dispatch failure");
                                TurnResult::Blocked
                            }
                        }
                    }
                }
            }
            AutoWorkTargetKind::Terminal => {
                unreachable!("Terminal AutoWork is rejected before queue execution")
            }
        };

        // Re-read the final Requirement status after AgentExecution settlement.
        let final_status = deps.service.get(&req_id).await.ok().map(|detail| detail.status);
        progress.set_current(None);

        if final_status == Some(RequirementStatus::Done) {
            let done_n = progress.incr_completed();
            if let Some(max) = max_requirements
                && done_n >= max
            {
                info!(
                    target_id,
                    tag,
                    completed = done_n,
                    "AutoWork reached max_requirements —stopping"
                );
                // Persist disabled so the cap survives restarts: boot resume must
                // not resurrect a binding that already met its completion cap.
                if let Err(e) = persist_maximum_completion_disable(
                    &deps,
                    &transitions,
                    kind,
                    target_id,
                    &config_revision,
                    &cancelled,
                )
                .await
                {
                    warn!(
                        target_id,
                        tag,
                        error = %e,
                        "AutoWork cap did not overwrite a newer persisted config"
                    );
                }
                // off: the cap disabled the binding -> drop the session-list icon.
                emit_autowork_progress(&deps, kind, target_id, tag, &progress, false);
                break;
            }
        }

        // idle: the turn finished and no requirement is in flight -> broadcast so
        // the session-list icon returns to the idle colour in step with the
        // per-session control (which only looks fresh because it re-GETs on open).
        emit_autowork_progress(&deps, kind, target_id, tag, &progress, true);

        // 4. Failure backoff: a failed or busy turn inserts a bounded, escalating
        // delay before the next claim so a deterministic failure cannot spin the
        // whole tag to `failed` in a fraction of a second. Interruptible by the
        // wake (resume / new work) and re-checked against cancel. Success resets.
        // A user interrupt also resets: the tag is paused, the loop will idle on
        // the next claim (None), and the user's resume must not inherit a backoff.
        match result {
            TurnResult::Done | TurnResult::UserInterrupted => consecutive_failures = 0,
            TurnResult::Blocked => {
                cancelled.store(true, Ordering::SeqCst);
            }
            TurnResult::Errored => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let delay = failure_backoff(consecutive_failures);
                let wake = deps.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                tokio::select! {
                    _ = wake.as_mut() => {}
                    _ = sleep(delay) => {}
                }
            }
        }
    }
}

/// Persist the completion-cap transition under the same target transition used
/// by route writes and handle publication. Without this shared critical
/// section, a cap could commit `disabled` after `apply_config`'s final Store
/// read but before it published a stale enabled loop.
async fn persist_maximum_completion_disable(
    deps: &AutoWorkRunnerDeps,
    transitions: &TargetTransitionMap,
    kind: AutoWorkTargetKind,
    target_id: &str,
    config_revision: &str,
    retiring: &AtomicBool,
) -> Result<(), AppError> {
    let key = (kind, target_id.to_owned());
    let transition = target_transition_lock(transitions, &key);
    let _transition_guard = transition.lock().await;
    let disabled = AutoWorkConfig::normalize(false, None, None)
        .expect("disabled AutoWork config is canonical");
    let operation_id = format!("autowork:max:v1:{target_id}:{config_revision}");
    let result = deps
        .service
        .save_autowork_config(
            &deps.authoritative_user_id,
            kind,
            target_id,
            disabled,
            config_revision,
            Some(&operation_id),
        )
        .await
        .map(|_| ());
    // Publish retirement before releasing the transition. A following enable
    // must stop/restart this generation instead of mistaking it for an
    // AlreadyRunning loop that is about to remove itself.
    retiring.store(true, Ordering::SeqCst);
    result
}

/// Submit one queue-selected Requirement to the canonical AgentExecution
/// aggregate. AutoWork neither creates an AgentSession turn nor observes its
/// receipts; the execution owner waits through attention states and returns
/// one canonical terminal result.
#[derive(Debug)]
struct PreparedAutomationExecution {
    request: AutoWorkExecutionRequest,
    attachment_plan: PromptAttachmentPlan,
    workspace: AutoWorkWorkspaceResolution,
}

async fn prepare_automation_execution_request(
    deps: &Arc<AutoWorkRunnerDeps>,
    session_id: &str,
    tag: &str,
    requirement: &Requirement,
    claim_generation: i64,
    claim_token: &str,
) -> Result<PreparedAutomationExecution, AppError> {
    let workspace = deps
        .workspace
        .resolve_frozen_workspace(&deps.authoritative_user_id, session_id)
        .await?;
    let attachment_plan = deps
        .service
        .plan_attachments_for_prompt(
            &requirement.requirement_id,
            workspace
                .workspace()
                .map(FrozenAutoWorkWorkspace::as_path),
        )
        .await?;
    let goal = crate::prompt::build_agent_execution_requirement_goal(
        tag,
        requirement,
        &attachment_plan.attachments,
    )?;
    Ok(PreparedAutomationExecution {
        request: AutoWorkExecutionRequest {
            source: AutoWorkExecutionSource {
                requirement_id: requirement.requirement_id.clone(),
                claim_generation,
                operation_id: autowork_execution_operation_id(
                    &requirement.requirement_id,
                    claim_generation,
                    claim_token,
                ),
            },
            lead_session_id: session_id.to_owned(),
            goal,
            workspace: workspace
                .workspace()
                .map(|workspace| workspace.as_str().to_owned()),
        },
        attachment_plan,
        workspace,
    })
}

async fn execute_requirement(
    deps: &Arc<AutoWorkRunnerDeps>,
    session_id: &str,
    tag: &str,
    requirement: &Requirement,
    claim_generation: i64,
    claim_token: &str,
) -> Result<AutoWorkExecutionReceipt, AppError> {
    let prepared = prepare_automation_execution_request(
        deps,
        session_id,
        tag,
        requirement,
        claim_generation,
        claim_token,
    )
    .await?;
    deps.execution
        .preflight_automation(&deps.authoritative_user_id, &prepared.request)
        .await?;
    // Preflight and the subsequent durable admission are enclosed by the same
    // Session operation lease. A delete fence cannot commit between staging
    // and AgentExecution's second exact validation.
    deps.service
        .activate_attachment_plan_with_operation_lease(
            &prepared.attachment_plan,
            prepared.workspace.operation_lease(),
        )
        .await?;
    let admission = deps
        .execution
        .admit_automation(&deps.authoritative_user_id, prepared.request)
        .await?;
    drop(prepared.workspace);
    let execution = deps
        .execution
        .await_automation(&deps.authoritative_user_id, &admission);
    tokio::pin!(execution);
    let mut renew = interval(LEASE_RENEW_INTERVAL);
    renew.tick().await;
    loop {
        tokio::select! {
            receipt = &mut execution => return receipt,
            _ = renew.tick() => {
                match deps
                    .service
                    .renew_lease(
                        &requirement.requirement_id,
                        session_id,
                        AutoWorkTargetKind::Conversation,
                        claim_generation,
                        claim_token,
                        DEFAULT_LEASE_MS,
                    )
                    .await?
                {
                    true => {}
                    false => {
                        return Err(AppError::Conflict(format!(
                            "Requirement {} generation {claim_generation} lost queue ownership while AgentExecution was active",
                            requirement.requirement_id
                        )));
                    }
                }
            }
        }
    }
}

/// Bounded, escalating delay before the next claim after a failed (or busy)
/// turn, so a deterministic failure cannot spin queue selection at millisecond
/// speed. Agent retry remains entirely inside AgentExecution.
/// `consecutive` is the count of back-to-back failed turns (1-based): 1s, 2s,
/// 4s, 8s, 16s, then capped at 30s. Reset to 0 on success / idle.
fn failure_backoff(consecutive: u32) -> Duration {
    let exp = consecutive.saturating_sub(1).min(5);
    let secs = (1u64 << exp).min(30);
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::pending;
    use std::sync::atomic::AtomicUsize;

    use nomifun_api_types::{CreateRequirementRequest, NewAttachmentRef};
    use nomifun_db::{
        IAttachmentRepository, IConversationRepository, IRequirementRepository,
        SqliteAttachmentRepository, SqliteConversationRepository, SqliteRequirementRepository,
        init_database_memory,
    };
    use nomifun_realtime::UserEventSink;
    use crate::execution_port::{
        AutoWorkExecutionAdmission, AutoWorkSessionConfigPort,
    };
    use crate::autowork_config::AutoWorkSessionConfigCommand;

    use super::*;

    const REQUIREMENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const CLAIM_TOKEN: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[derive(Default)]
    struct NoopBroadcaster;

    impl UserEventSink for NoopBroadcaster {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    struct FixedWorkspacePort {
        workspace: Option<FrozenAutoWorkWorkspace>,
    }

    struct ReplayAwareConfigState {
        current: AutoWorkConfigSnapshot,
        receipts: HashMap<
            String,
            (String, AutoWorkConfig, AutoWorkConfigSnapshot),
        >,
    }

    struct ReplayAwareConfigPort {
        state: Mutex<ReplayAwareConfigState>,
    }

    impl ReplayAwareConfigPort {
        fn new() -> Self {
            Self {
                state: Mutex::new(ReplayAwareConfigState {
                    current: AutoWorkConfigSnapshot::new(
                        AutoWorkConfig::default(),
                        "0",
                        None,
                    )
                    .unwrap(),
                    receipts: HashMap::new(),
                }),
            }
        }
    }

    #[async_trait::async_trait]
    impl AutoWorkSessionConfigPort for ReplayAwareConfigPort {
        async fn read_config(
            &self,
            _owner_id: &str,
            _session_id: &str,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            Ok(self.state.lock().expect("config state").current.clone())
        }

        async fn save_config(
            &self,
            command: AutoWorkSessionConfigCommand,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            let mut state = self.state.lock().expect("config state");
            if let Some(operation_id) = command.operation_id.as_deref() {
                if let Some((expected_revision, config, receipt)) =
                    state.receipts.get(operation_id)
                {
                    if expected_revision != &command.expected_revision
                        || config != &command.config
                    {
                        return Err(AppError::Conflict(
                            "AutoWork operation replay changed input".to_owned(),
                        ));
                    }
                    return Ok(receipt.clone());
                }
            }
            if command.expected_revision != state.current.revision {
                return Err(AppError::Conflict(
                    "AutoWork config revision changed concurrently".to_owned(),
                ));
            }
            let current_revision = state.current.revision.parse::<u64>().unwrap();
            let revision = if command.config == state.current.config {
                current_revision
            } else {
                current_revision + 1
            };
            let receipt = AutoWorkConfigSnapshot::new(
                command.config.clone(),
                revision.to_string(),
                command.operation_id.clone(),
            )?;
            if let Some(operation_id) = command.operation_id {
                state.receipts.insert(
                    operation_id,
                    (command.expected_revision, command.config, receipt.clone()),
                );
            }
            state.current = receipt.clone();
            Ok(receipt)
        }
    }

    #[async_trait::async_trait]
    impl AutoWorkWorkspacePort for FixedWorkspacePort {
        async fn resolve_frozen_workspace(
            &self,
            _owner_id: &str,
            _agent_session_id: &str,
        ) -> Result<AutoWorkWorkspaceResolution, AppError> {
            Ok(AutoWorkWorkspaceResolution::new(self.workspace.clone()))
        }
    }

    struct ScriptedExecutionPort {
        cancel_receipt: Mutex<Option<AutoWorkExecutionReceipt>>,
        cancel_calls: AtomicUsize,
        execute_started: AtomicBool,
        execute_started_notify: Notify,
        execute_dropped: Arc<AtomicBool>,
    }

    impl ScriptedExecutionPort {
        fn with_cancel_receipt(receipt: Option<AutoWorkExecutionReceipt>) -> Self {
            Self {
                cancel_receipt: Mutex::new(receipt),
                cancel_calls: AtomicUsize::new(0),
                execute_started: AtomicBool::new(false),
                execute_started_notify: Notify::new(),
                execute_dropped: Arc::new(AtomicBool::new(false)),
            }
        }

        async fn wait_until_execute_started(&self) {
            loop {
                let notified = self.execute_started_notify.notified();
                if self.execute_started.load(Ordering::SeqCst) {
                    return;
                }
                notified.await;
            }
        }
    }

    struct ExecuteDropGuard(Arc<AtomicBool>);

    impl Drop for ExecuteDropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl AutoWorkExecutionPort for ScriptedExecutionPort {
        async fn preflight_automation(
            &self,
            _owner_id: &str,
            _request: &AutoWorkExecutionRequest,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn admit_automation(
            &self,
            _owner_id: &str,
            _request: AutoWorkExecutionRequest,
        ) -> Result<AutoWorkExecutionAdmission, AppError> {
            Ok(AutoWorkExecutionAdmission {
                execution_id: "execution-test".to_owned(),
            })
        }

        async fn await_automation(
            &self,
            _owner_id: &str,
            _admission: &AutoWorkExecutionAdmission,
        ) -> Result<AutoWorkExecutionReceipt, AppError> {
            let _drop_guard = ExecuteDropGuard(self.execute_dropped.clone());
            self.execute_started.store(true, Ordering::SeqCst);
            self.execute_started_notify.notify_waiters();
            pending::<Result<AutoWorkExecutionReceipt, AppError>>().await
        }

        async fn cancel_automation(
            &self,
            _owner_id: &str,
            _source: &AutoWorkExecutionSource,
        ) -> Result<Option<AutoWorkExecutionReceipt>, AppError> {
            self.cancel_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.cancel_receipt.lock().expect("cancel receipt lock").clone())
        }
    }

    async fn claimed_fixture(
        tag: &str,
        execution: Arc<ScriptedExecutionPort>,
    ) -> (
        nomifun_db::Database,
        Arc<RequirementService>,
        Arc<AutoWorkRunnerDeps>,
        String,
        LiveClaim,
    ) {
        let database = init_database_memory().await.expect("in-memory database");
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        let session_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?1, ?2, 'AutoWork correctness', 'nomi', 0, 0)",
        )
        .bind(&session_id)
        .bind(&owner_id)
        .execute(database.pool())
        .await
        .expect("conversation fixture");
        let repository: Arc<dyn IRequirementRepository> = Arc::new(
            SqliteRequirementRepository::new(database.pool().clone()),
        );
        let wake = Arc::new(Notify::new());
        let service = Arc::new(
            RequirementService::new(
                repository,
                crate::events::RequirementEventEmitter::new(
                    Arc::new(NoopBroadcaster),
                    Arc::from(owner_id.as_str()),
                ),
            )
            .with_autowork_waker(wake.clone()),
        );
        let requirement = service
            .create(CreateRequirementRequest {
                title: "Automation correctness".to_owned(),
                content: "Preserve exact execution outcome authority.".to_owned(),
                tag: tag.to_owned(),
                order_key: None,
                status: None,
                created_by: None,
                attachments: Vec::new(),
            })
            .await
            .expect("requirement fixture");
        let claim = service
            .claim_next_for_runner(
                tag,
                &session_id,
                AutoWorkTargetKind::Conversation,
                DEFAULT_LEASE_MS,
            )
            .await
            .expect("claim fixture")
            .expect("claim available");
        let deps = Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id),
            service: service.clone(),
            execution,
            workspace: Arc::new(FixedWorkspacePort { workspace: None }),
            wake,
        });
        (
            database,
            service,
            deps,
            session_id,
            LiveClaim {
                requirement_id: requirement.requirement_id,
                claim_generation: claim.claim_generation,
                claim_token: claim.claim_token,
            },
        )
    }

    #[test]
    fn execution_operation_identity_is_stable_and_hides_claim_capability() {
        let first = autowork_execution_operation_id(REQUIREMENT_ID, 7, CLAIM_TOKEN);
        let replay = autowork_execution_operation_id(REQUIREMENT_ID, 7, CLAIM_TOKEN);
        let next_generation = autowork_execution_operation_id(REQUIREMENT_ID, 8, CLAIM_TOKEN);
        assert_eq!(first, replay);
        assert_ne!(first, next_generation);
        assert!(first.starts_with("autowork:"));
        assert!(!first.contains(REQUIREMENT_ID));
        assert!(!first.contains(CLAIM_TOKEN));
        assert!(first.len() <= 128);
    }

    #[test]
    fn live_claim_projects_only_digest_identity_to_execution() {
        let claim = LiveClaim {
            requirement_id: REQUIREMENT_ID.to_owned(),
            claim_generation: 3,
            claim_token: CLAIM_TOKEN.to_owned(),
        };
        let source = claim.execution_source();
        assert_eq!(source.requirement_id, REQUIREMENT_ID);
        assert_eq!(source.claim_generation, 3);
        assert!(!source.operation_id.contains(CLAIM_TOKEN));
    }

    #[tokio::test]
    async fn historical_config_replay_does_not_roll_live_loop_back() {
        let database = init_database_memory().await.expect("in-memory database");
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        let session_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?1, ?2, 'AutoWork replay', 'nomi', 0, 0)",
        )
        .bind(&session_id)
        .bind(&owner_id)
        .execute(database.pool())
        .await
        .unwrap();
        let repository: Arc<dyn IRequirementRepository> = Arc::new(
            SqliteRequirementRepository::new(database.pool().clone()),
        );
        let conversation_repository: Arc<dyn IConversationRepository> = Arc::new(
            SqliteConversationRepository::new(database.pool().clone()),
        );
        let config_port: Arc<dyn AutoWorkSessionConfigPort> =
            Arc::new(ReplayAwareConfigPort::new());
        let wake = Arc::new(Notify::new());
        let service = Arc::new(
            RequirementService::new(
                repository,
                crate::events::RequirementEventEmitter::new(
                    Arc::new(NoopBroadcaster),
                    Arc::from(owner_id.as_str()),
                ),
            )
            .with_session_config_port(config_port, conversation_repository)
            .with_autowork_waker(wake.clone()),
        );
        let runner = AutoWorkRunner::new(Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id.as_str()),
            service: service.clone(),
            execution: Arc::new(ScriptedExecutionPort::with_cancel_receipt(None)),
            workspace: Arc::new(FixedWorkspacePort { workspace: None }),
            wake,
        }));
        let enabled = AutoWorkConfig {
            enabled: true,
            tag: Some("release".to_owned()),
            max_requirements: Some(3),
        };
        let first = runner
            .apply_config(
                owner_id.as_str(),
                AutoWorkTargetKind::Conversation,
                &session_id,
                enabled.clone(),
                Some("0"),
                Some("enable-a"),
            )
            .await
            .unwrap();
        assert_eq!(first.revision, "1");
        assert!(runner.is_running(AutoWorkTargetKind::Conversation, &session_id));

        let disabled = AutoWorkConfig {
            enabled: false,
            tag: Some("release".to_owned()),
            max_requirements: Some(3),
        };
        let second = runner
            .apply_config(
                owner_id.as_str(),
                AutoWorkTargetKind::Conversation,
                &session_id,
                disabled.clone(),
                Some("1"),
                Some("disable-b"),
            )
            .await
            .unwrap();
        assert_eq!(second.revision, "2");
        assert!(!runner.is_running(AutoWorkTargetKind::Conversation, &session_id));

        let replay = runner
            .apply_config(
                owner_id.as_str(),
                AutoWorkTargetKind::Conversation,
                &session_id,
                enabled,
                Some("0"),
                Some("enable-a"),
            )
            .await
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            service
                .read_autowork_config_snapshot(
                    owner_id.as_str(),
                    AutoWorkTargetKind::Conversation,
                    &session_id,
                )
                .await
                .unwrap()
                .config,
            disabled,
        );
        assert!(
            !runner.is_running(AutoWorkTargetKind::Conversation, &session_id),
            "replaying historical enable must not restart a disabled loop"
        );
        runner.quiesce().await.unwrap();
    }

    #[tokio::test]
    async fn maximum_completion_disable_waits_for_target_transition() {
        let database = init_database_memory().await.expect("in-memory database");
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        let session_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?1, ?2, 'AutoWork cap race', 'nomi', 0, 0)",
        )
        .bind(&session_id)
        .bind(&owner_id)
        .execute(database.pool())
        .await
        .unwrap();
        let repository: Arc<dyn IRequirementRepository> = Arc::new(
            SqliteRequirementRepository::new(database.pool().clone()),
        );
        let conversation_repository: Arc<dyn IConversationRepository> = Arc::new(
            SqliteConversationRepository::new(database.pool().clone()),
        );
        let config_port: Arc<dyn AutoWorkSessionConfigPort> =
            Arc::new(ReplayAwareConfigPort::new());
        let wake = Arc::new(Notify::new());
        let service = Arc::new(
            RequirementService::new(
                repository,
                crate::events::RequirementEventEmitter::new(
                    Arc::new(NoopBroadcaster),
                    Arc::from(owner_id.as_str()),
                ),
            )
            .with_session_config_port(config_port, conversation_repository)
            .with_autowork_waker(wake.clone()),
        );
        let enabled = AutoWorkConfig {
            enabled: true,
            tag: Some("release".to_owned()),
            max_requirements: Some(1),
        };
        let saved = service
            .save_autowork_config(
                owner_id.as_str(),
                AutoWorkTargetKind::Conversation,
                &session_id,
                enabled.clone(),
                "0",
                Some("enable-before-cap"),
            )
            .await
            .unwrap();
        assert_eq!(saved.revision, "1");
        let deps = Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id.as_str()),
            service: service.clone(),
            execution: Arc::new(ScriptedExecutionPort::with_cancel_receipt(None)),
            workspace: Arc::new(FixedWorkspacePort { workspace: None }),
            wake,
        });
        let transitions = Arc::new(DashMap::new());
        let retiring = Arc::new(AtomicBool::new(false));
        let key = (AutoWorkTargetKind::Conversation, session_id.clone());
        let publication_guard = target_transition_lock(&transitions, &key)
            .lock_owned()
            .await;
        let mut cap = {
            let deps = Arc::clone(&deps);
            let transitions = Arc::clone(&transitions);
            let retiring = Arc::clone(&retiring);
            let session_id = session_id.clone();
            tokio::spawn(async move {
                persist_maximum_completion_disable(
                    &deps,
                    &transitions,
                    AutoWorkTargetKind::Conversation,
                    &session_id,
                    "1",
                    &retiring,
                )
                .await
            })
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut cap)
                .await
                .is_err(),
            "cap write must wait while handle publication owns the target transition"
        );

        let retirement_observer = {
            let transition = target_transition_lock(&transitions, &key);
            let retiring = Arc::clone(&retiring);
            tokio::spawn(async move {
                let _after_cap = transition.lock_owned().await;
                retiring.load(Ordering::SeqCst)
            })
        };
        assert_eq!(
            service
                .read_autowork_config_snapshot(
                    owner_id.as_str(),
                    AutoWorkTargetKind::Conversation,
                    &session_id,
                )
                .await
                .unwrap()
                .config,
            enabled,
        );

        drop(publication_guard);
        tokio::time::timeout(Duration::from_secs(5), cap)
            .await
            .expect("cap write must finish after publication releases")
            .expect("cap task")
            .expect("cap persistence");
        assert!(
            retirement_observer.await.expect("retirement observer"),
            "the capped generation must be marked retiring before the transition is released"
        );
        assert!(
            !service
                .read_autowork_config_snapshot(
                    owner_id.as_str(),
                    AutoWorkTargetKind::Conversation,
                    &session_id,
                )
                .await
                .unwrap()
                .config
                .enabled
        );
    }

    #[tokio::test]
    async fn attachment_staging_uses_frozen_workspace_and_replays_idempotently() {
        let database = init_database_memory().await.expect("in-memory database");
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        let repository: Arc<dyn IRequirementRepository> = Arc::new(
            SqliteRequirementRepository::new(database.pool().clone()),
        );
        let attachment_repository: Arc<dyn IAttachmentRepository> = Arc::new(
            SqliteAttachmentRepository::new(database.pool().clone()),
        );
        let data_dir = tempfile::tempdir().unwrap();
        let upload_dir = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let source = upload_dir.path().join("architecture.png");
        std::fs::write(&source, b"frozen-workspace-image").unwrap();
        let attachment_store = Arc::new(
            crate::attachments::AttachmentStore::new(
                data_dir.path().to_path_buf(),
                attachment_repository,
            )
            .with_upload_root(upload_dir.path().to_path_buf()),
        );
        let wake = Arc::new(Notify::new());
        let service = Arc::new(
            RequirementService::new(
                repository,
                crate::events::RequirementEventEmitter::new(
                    Arc::new(NoopBroadcaster),
                    Arc::from(owner_id.as_str()),
                ),
            )
            .with_attachment_store(attachment_store)
            .with_autowork_waker(wake.clone()),
        );
        let requirement = service
            .create(CreateRequirementRequest {
                title: "Inspect attached architecture".to_owned(),
                content: "Use the staged image.".to_owned(),
                tag: "architecture".to_owned(),
                order_key: None,
                status: None,
                created_by: None,
                attachments: vec![NewAttachmentRef {
                    source_path: source.to_string_lossy().into_owned(),
                    file_name: "architecture.png".to_owned(),
                }],
            })
            .await
            .unwrap();
        let execution = Arc::new(ScriptedExecutionPort::with_cancel_receipt(None));
        let session_id = ConversationId::new().into_string();

        let no_workspace = Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id.as_str()),
            service: service.clone(),
            execution: execution.clone(),
            workspace: Arc::new(FixedWorkspacePort { workspace: None }),
            wake: wake.clone(),
        });
        let error = prepare_automation_execution_request(
            &no_workspace,
            &session_id,
            "architecture",
            &requirement,
            1,
            CLAIM_TOKEN,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("frozen AgentSession workspace"));
        assert!(!error.to_string().contains(&data_dir.path().to_string_lossy().to_string()));
        assert!(!workspace.path().join(".nomi").exists());

        let frozen_workspace =
            FrozenAutoWorkWorkspace::new(workspace.path().to_string_lossy().into_owned())
                .unwrap();
        let deps = Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id),
            service,
            execution,
            workspace: Arc::new(FixedWorkspacePort {
                workspace: Some(frozen_workspace.clone()),
            }),
            wake,
        });
        let first = prepare_automation_execution_request(
            &deps,
            &session_id,
            "architecture",
            &requirement,
            1,
            CLAIM_TOKEN,
        )
        .await
        .unwrap();
        let replay = prepare_automation_execution_request(
            &deps,
            &session_id,
            "architecture",
            &requirement,
            1,
            CLAIM_TOKEN,
        )
        .await
        .unwrap();
        assert_eq!(first.request, replay.request);
        assert_eq!(
            first.attachment_plan.attachments,
            replay.attachment_plan.attachments
        );
        assert_eq!(
            first.request.workspace.as_deref(),
            Some(frozen_workspace.as_str())
        );
        assert!(first.request.goal.contains("./.nomi/requirement-attachments/"));
        assert!(!first.request.goal.contains(&data_dir.path().to_string_lossy().to_string()));
        assert!(!first.request.goal.contains(&source.to_string_lossy().to_string()));
        deps.execution
            .preflight_automation(&deps.authoritative_user_id, &first.request)
            .await
            .unwrap();
        deps.service
            .activate_attachment_plan(&first.attachment_plan)
            .await
            .unwrap();
        deps.execution
            .admit_automation(&deps.authoritative_user_id, first.request)
            .await
            .unwrap();
        drop(first.workspace);

        let staged_dir = workspace
            .path()
            .join(".nomi/requirement-attachments")
            .join(&requirement.requirement_id);
        let staged = std::fs::read_dir(staged_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "png"))
            .expect("staged attachment");
        assert_eq!(std::fs::read(staged).unwrap(), b"frozen-workspace-image");
    }

    #[test]
    fn failure_backoff_is_bounded() {
        assert_eq!(failure_backoff(1), Duration::from_secs(1));
        assert_eq!(failure_backoff(2), Duration::from_secs(2));
        assert_eq!(failure_backoff(6), Duration::from_secs(30));
        assert_eq!(failure_backoff(u32::MAX), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn cancelled_execution_with_an_admitted_attempt_is_never_requeued() {
        let execution_id = nomifun_common::AgentExecutionId::new().into_string();
        let execution = Arc::new(ScriptedExecutionPort::with_cancel_receipt(Some(
            AutoWorkExecutionReceipt::Cancelled {
                execution_id,
                replay_safe: false,
            },
        )));
        let (_database, service, deps, session_id, claim) =
            claimed_fixture("cancelled-ambiguous", execution.clone()).await;

        settle_stopped_agent_execution_claim(
            &deps,
            &session_id,
            "cancelled-ambiguous",
            &claim,
            "AutoWork was stopped.",
        )
        .await
        .expect("ambiguous cancellation must be parked");

        let requirement = service
            .get(&claim.requirement_id)
            .await
            .expect("parked requirement");
        assert_eq!(requirement.status, RequirementStatus::NeedsReview);
        assert_eq!(execution.cancel_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelled_execution_requeues_only_with_zero_attempt_proof() {
        let execution_id = nomifun_common::AgentExecutionId::new().into_string();
        let execution = Arc::new(ScriptedExecutionPort::with_cancel_receipt(Some(
            AutoWorkExecutionReceipt::Cancelled {
                execution_id,
                replay_safe: true,
            },
        )));
        let (_database, service, deps, session_id, claim) =
            claimed_fixture("cancelled-before-attempt", execution).await;

        settle_stopped_agent_execution_claim(
            &deps,
            &session_id,
            "cancelled-before-attempt",
            &claim,
            "AutoWork was stopped.",
        )
        .await
        .expect("pre-attempt cancellation may release the exact claim");

        let requirement = service
            .get(&claim.requirement_id)
            .await
            .expect("released requirement");
        assert_eq!(requirement.status, RequirementStatus::Pending);
    }

    #[tokio::test]
    async fn outcome_unknown_receipt_projects_needs_review_without_requeue() {
        let execution_id = nomifun_common::AgentExecutionId::new().into_string();
        let execution = Arc::new(ScriptedExecutionPort::with_cancel_receipt(Some(
            AutoWorkExecutionReceipt::OutcomeUnknown {
                execution_id,
                error: Some("agent_delivery_receipt_missing: receipt absent".to_owned()),
            },
        )));
        let (_database, service, deps, session_id, claim) =
            claimed_fixture("outcome-unknown", execution).await;

        settle_stopped_agent_execution_claim(
            &deps,
            &session_id,
            "outcome-unknown",
            &claim,
            "AutoWork was stopped.",
        )
        .await
        .expect("unknown outcome must be parked");

        let requirement = service
            .get(&claim.requirement_id)
            .await
            .expect("parked requirement");
        assert_eq!(requirement.status, RequirementStatus::NeedsReview);
    }

    #[tokio::test]
    async fn process_quiesce_drops_waiter_without_cancelling_durable_execution() {
        let execution = Arc::new(ScriptedExecutionPort::with_cancel_receipt(None));
        let (_database, service, deps, session_id, claim) =
            claimed_fixture("process-quiesce", execution.clone()).await;
        let runner = AutoWorkRunner::new(deps);
        let snapshot = AutoWorkConfigSnapshot::new(
            AutoWorkConfig::normalize(true, Some("process-quiesce"), None)
                .expect("enabled config"),
            "test:process-quiesce",
            None,
        )
        .expect("config snapshot");
        let key = (AutoWorkTargetKind::Conversation, session_id.clone());
        let transition = runner.transition_lock(&key);
        let transition_guard = transition.lock().await;
        assert_eq!(
            runner
                .start_snapshot_locked(
                    AutoWorkTargetKind::Conversation,
                    &session_id,
                    snapshot,
                )
                .await,
            AutoWorkStartOutcome::Started
        );
        drop(transition_guard);

        timeout(Duration::from_secs(2), execution.wait_until_execute_started())
            .await
            .expect("execution waiter started");
        runner.quiesce().await.expect("runner quiesced");

        assert!(execution.execute_dropped.load(Ordering::SeqCst));
        assert_eq!(execution.cancel_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            service
                .get(&claim.requirement_id)
                .await
                .expect("durable claim")
                .status,
            RequirementStatus::InProgress,
            "process shutdown must leave the durable claim for boot recovery"
        );
    }
}
