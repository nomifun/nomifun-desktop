use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use nomifun_ai_agent::registry::AgentRegistry;
use nomifun_api_types::{AutoWorkState, AutoWorkTargetKind, Requirement, RequirementStatus};
use nomifun_common::{AppError, ConversationId, TerminalId};
use nomifun_db::{
    RequirementConversationTurnAuthority, TerminalTurnAdmissionKey, TerminalTurnAdmissionRow,
    TerminalTurnAdmissionScope, TerminalTurnEffectsStart, TerminalTurnOutcome,
};
use nomifun_terminal::{ExactTerminalLifecycleReceiver, LifecycleKind, TerminalDriver};
use tokio::sync::{Notify, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::prompt::{build_requirement_prompt, build_terminal_requirement_prompt};
use crate::service::{DEFAULT_LEASE_MS, RequirementService};
use crate::attachments::PromptAttachmentPlan;
use crate::autowork_config::{AutoWorkConfig, AutoWorkConfigSnapshot};
use crate::conversation_port::{
    AutoWorkBindingLookup, AutoWorkMessage, AutoWorkMessageDelivery, AutoWorkPreSendHook,
    AutoWorkReconciliationDisposition, AutoWorkRuntimeBuildLease, AutoWorkRuntimeOverlay,
    AutoWorkSessionPort, AutoWorkTurnDeliveryState, AutoWorkTurnRequest,
};

/// Lease is renewed on this cadence while a turn is in flight.
const LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(30);
/// Durable Conversation receipts are the authoritative completion channel.
/// This is intentionally a short poll: SQLite lookup is indexed by the stable
/// operation identity and a fast turn must not wait for a lease tick.
const CONVERSATION_RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Accepted-receipt reconciliation runs before the durable receipt wait. It
/// may acquire a preparation gate and consult several persistence/runtime
/// seams, so it needs its own finite deadline instead of inheriting the
/// receipt wait's much larger turn timeout.
const CONVERSATION_RECEIPT_RECONCILIATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Hard ceiling on a single requirement turn.
const TURN_TIMEOUT: Duration = Duration::from_secs(3600);
/// Idle cadence for a persistent loop with nothing to do (tag drained, claim
/// error, or a terminal awaiting relaunch). The `wake` Notify makes a freshly
/// created/re-pended requirement claim near-instantly; this is the safety-net
/// poll for anything the waker can't observe (for example, a terminal coming
/// back alive).
const IDLE_POLL: Duration = Duration::from_secs(10);
/// Cap on the completion note captured from a tool-free agent's final message,
/// in characters. The tail is kept (agents usually summarise at the end).
const MAX_NOTE_CHARS: usize = 4000;
/// API callers wait only this long for deterministic target cleanup. The
/// cleanup task itself remains owned and continues after the waiter returns.
const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
/// One global shutdown ceiling for coordinator joins and every target cleanup.
const SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Shared dependencies for all AutoWork loops.
pub struct AutoWorkRunnerDeps {
    /// Canonical installation owner. AutoWork is part of the installation-wide
    /// Requirements control plane and may only resume or drive this user's
    /// Conversation/Terminal targets.
    pub authoritative_user_id: Arc<str>,
    pub service: Arc<RequirementService>,
    pub conversation: Arc<dyn AutoWorkSessionPort>,
    pub agent_registry: Arc<AgentRegistry>,
    /// Drives terminal targets (write PTY input, observe output). `None` if the
    /// terminal subsystem is not wired (e.g. some test harnesses).
    pub terminal_driver: Option<Arc<dyn TerminalDriver>>,
    /// Optional IDMM supervisor. When present, AutoWork ensures the target is
    /// supervised while each turn runs so provider faults / decision stalls are
    /// auto-handled and the turn can complete instead of hanging to timeout.
    /// `None` if IDMM is not wired (tests, or the feature disabled at assembly).
    pub idmm: Option<Arc<dyn crate::hooks::IdmmHandle>>,
    /// Notified whenever a requirement becomes claimable. Idle loops await this
    /// (with `IDLE_POLL` as a fallback) so newly created/re-pended work is picked
    /// up immediately. Shared with the `RequirementService` that fires it.
    pub wake: Arc<tokio::sync::Notify>,
    /// Whether the requirement MCP server is running and injected into agent
    /// TERMINAL sessions (bootstrap-level flag). When true, those terminals
    /// expose the `requirement_complete` / `requirement_update_status`
    /// declaration tools over the stdio bridge, so the runner expects an
    /// explicit verdict (a clean turn with no declaration -> needs_review, not
    /// done). Chat-engine sessions register the same tools in-process instead;
    /// see [`crate::prompt::has_native_requirement_tools`].
    pub requirement_mcp_enabled: bool,
}

/// Sealed, in-process preparation handed to the host-owned Session port. It is
/// invoked only after the exact Requirement capability, typed turn receipt,
/// durable Running generation, and local turn owner have been admitted under
/// one preparation fence. Public request payloads cannot inject this hook.
struct AutoWorkAttachmentActivation {
    service: Arc<RequirementService>,
    plan: PromptAttachmentPlan,
}

#[async_trait::async_trait]
impl AutoWorkPreSendHook for AutoWorkAttachmentActivation {
    async fn prepare(&self) -> Result<(), AppError> {
        self.service.activate_attachment_plan(&self.plan).await
    }
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
    /// Explicit `stop_locked` takes responsibility for deterministic cleanup
    /// and sets this before aborting. Every other task drop (panic, external
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
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if self.cleanup_handoff.load(Ordering::SeqCst) {
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
        runtime.spawn(async move {
            let _cleanup_guard = cleanup_barrier.lock().await;
            // A racing explicit stop owns deterministic cleanup and holds the
            // start/stop transition barrier. Once handed off, this background
            // task must not broadly park a replacement Terminal generation.
            if !cleanup_handoff.load(Ordering::SeqCst) {
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
        if expected_revision.is_some_and(|revision| revision != current.revision) {
            return Err(AppError::Conflict(format!(
                "AutoWork config for {} {target_id} changed concurrently",
                kind.as_str()
            )));
        }

        // A semantic reconfiguration must quiesce the old generation before
        // publishing the new durable binding. An identical enable deliberately
        // leaves the active Requirement and runtime untouched.
        if config.enabled && !self.running_config_matches(&key, &config) {
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

        let saved = self
            .deps
            .service
            .save_autowork_config(
                owner_id,
                kind,
                target_id,
                config.clone(),
                &current.revision,
                operation_id,
            )
            .await?;

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
        let handles = self.handles.clone();
        let conv = target_id.to_owned();
        let loop_tag = tag.clone();
        let guard_key = key.clone();
        let cleanup_handoff_for_task = cleanup_handoff.clone();
        let cleanup_barrier_for_task = cleanup_barrier.clone();
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
            };
            info!(target_id = %conv, ?kind, tag = %loop_tag, "AutoWork loop started");
            run_loop(
                deps,
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
        // Await cancellation before durable cleanup. A claim/admission DB
        // future that won the race is therefore visible to the cleanup below;
        // a future that lost cannot later resume and write.
        let _ = handle.join.await;
        // If a natural-exit/panic Drop custodian started just before this
        // explicit stop took ownership, wait until it has either finished or
        // observed `cleanup_handoff` and skipped. The retained `cleanups` entry
        // blocks a replacement generation even after the bounded waiter leaves.
        let _cleanup_guard = handle.cleanup_barrier.lock().await;
            // Do not trust only the process-local progress slot here. The
            // claim transaction can COMMIT immediately before task abortion
            // and the task can then be dropped before `progress.set_current`.
            // Re-read the exact typed owner while the start/stop transition
            // barrier is still held. This recovery seam never allocates a new
            // pending claim; it only exposes an already-committed active one.
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
                        ?kind,
                        error = %error,
                        "Failed to recover a just-committed AutoWork claim during stop"
                    );
                    None
                }
            };
            // The durable owner is authoritative when present. The progress
            // value remains a useful exact-CAS fallback when the row was
            // already finalized between abortion and this read.
            let active_claim = recovered_claim.or(published_claim);

            if kind == AutoWorkTargetKind::Conversation {
                if let Err(error) = self.deps.conversation.cancel_active_turn(target_id).await {
                    warn!(
                        target_id,
                        error = %error,
                        "Failed to cancel in-flight AutoWork turn on stop"
                    );
                }
            }
            if let Some(active_claim) = active_claim {
                let req_id = active_claim.requirement_id;
                let claim_generation = active_claim.claim_generation;
                let claim_token = active_claim.claim_token;
                if kind == AutoWorkTargetKind::Conversation {
                    if let Err(error) = resolve_interrupted_conversation_claim(
                        &self.deps,
                        &req_id,
                        target_id,
                        claim_generation,
                        &claim_token,
                        "AutoWork was stopped.",
                    )
                    .await
                    {
                        warn!(
                            requirement_id = req_id,
                            claim_generation,
                            error = %error,
                            "Failed to resolve Conversation claim on AutoWork stop"
                        );
                    }
                } else if let Some(driver) = &self.deps.terminal_driver {
                    let detail =
                        "AutoWork was stopped while this Terminal claim was active; any durable \
                         admission was absorbed and will not be written again.";
                    let park_succeeded = match driver
                        .park_open_turn_admissions(target_id, None, detail)
                        .await
                    {
                        Ok(_) => true,
                        Err(error) => {
                            warn!(
                                terminal_id = target_id,
                                requirement_id = req_id,
                                error = %error,
                                "Failed to park Terminal turn admission during AutoWork stop"
                            );
                            false
                        }
                    };
                    match driver
                        .get_turn_admission_for_claim(
                            target_id,
                            &req_id,
                            claim_generation,
                            &claim_token,
                        )
                        .await
                    {
                        Ok(None) if park_succeeded => {
                            // The receiver-side lookup is only diagnostic. The
                            // database abandon command independently repeats
                            // exact authority and all receiver-admission
                            // absence proofs in the same writer transaction as
                            // active->pending.
                            if let Err(error) = abandon_pre_effect_or_quarantine(
                                &self.deps,
                                &req_id,
                                target_id,
                                AutoWorkTargetKind::Terminal,
                                claim_generation,
                                &claim_token,
                                "AutoWork stop found no exact Terminal admission.",
                            )
                            .await
                            {
                                warn!(
                                    terminal_id = target_id,
                                    requirement_id = req_id,
                                    error = %error,
                                    "Failed to safely resolve pre-admission Terminal claim"
                                );
                            }
                        }
                        lookup => {
                            let (status, note) = match lookup {
                                Ok(Some(row)) => match terminal_turn_end_from_receipt(&row) {
                                    Some(TerminalTurnEnd::AuthoritativeVerdict { status, note }) => {
                                        (status, note)
                                    }
                                    _ => (
                                        RequirementStatus::NeedsReview,
                                        Some(detail.to_owned()),
                                    ),
                                },
                                Ok(None) => (
                                    RequirementStatus::NeedsReview,
                                    Some(format!(
                                        "{detail} Parking or receipt verification failed, so \
                                         pre-admission absence was not proven."
                                    )),
                                ),
                                Err(error) => (
                                    RequirementStatus::NeedsReview,
                                    Some(format!(
                                        "{detail} Exact receipt lookup failed: {error}"
                                    )),
                                ),
                            };
                            if let Err(status_error) = resolve_claim_verdict_required(
                                &self.deps.service,
                                    &req_id,
                                    claim_generation,
                                    &claim_token,
                                    target_id,
                                    AutoWorkTargetKind::Terminal,
                                    status,
                                    note,
                            )
                            .await
                            {
                                warn!(
                                    terminal_id = target_id,
                                    requirement_id = req_id,
                                    error = %status_error,
                                    "Failed to resolve Terminal Requirement during AutoWork stop"
                                );
                            }
                        }
                    }
                } else {
                    let detail = "AutoWork stopped an active Terminal claim without a durable \
                                  Terminal driver; its execution state is unknown.";
                    if let Err(error) = resolve_claim_verdict_required(
                        &self.deps.service,
                            &req_id,
                            claim_generation,
                            &claim_token,
                            target_id,
                            AutoWorkTargetKind::Terminal,
                            RequirementStatus::NeedsReview,
                            Some(detail.to_owned()),
                    )
                    .await
                    {
                        warn!(
                            terminal_id = target_id,
                            requirement_id = req_id,
                            error = %error,
                            "Failed to park Terminal Requirement without a driver"
                        );
                    }
                }
            } else if kind == AutoWorkTargetKind::Terminal
                && let Some(driver) = &self.deps.terminal_driver
            {
                // Even if the Requirement owner lookup failed, absorb any
                // already-created receipt under the same lifecycle lock as the
                // PTY writer. A later exact Requirement recovery can project
                // the parked receipt, but no submit byte may escape this stop.
                if let Err(error) = driver
                    .park_open_turn_admissions(
                        target_id,
                        None,
                        "AutoWork stopped while claim recovery was unavailable; any durable Terminal admission was parked.",
                    )
                    .await
                {
                    warn!(
                        terminal_id = target_id,
                        error = %error,
                        "Failed to park Terminal admissions without a recovered claim"
                    );
                }
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
                let active_terminals: Vec<String> = handles
                    .iter()
                    .filter(|entry| entry.key().0 == AutoWorkTargetKind::Terminal)
                    .map(|entry| entry.key().1.clone())
                    .collect();
                match service
                    .repo()
                    .sweep_expired_leases(
                        &active_conversations,
                        &active_terminals,
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

    /// Resume every persisted-enabled AutoWork binding across all users at boot.
    ///
    /// The running set (`handles`) is in-memory, but the enabled/tag config is
    /// persisted (conversation `extra.autowork` / terminal `autowork` column). On
    /// a process restart nothing would drive those bindings until a user opened
    /// each session page —the old behaviour that made AutoWork look like it
    /// "only works in the foreground". Spawning the loops here makes the backend
    /// the single source of truth: a bound session works in the background from
    /// boot, no UI visit required. Conversation loops start driving immediately;
    /// a terminal whose PTY is not yet live idles until the user relaunches it
    /// (the loop self-heals —see `run_loop`). Detached + best-effort.
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
        binding: crate::conversation_port::PersistedAutoWorkBinding,
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

    /// Stop the host-owned sweeper/resume coordinators and every active target
    /// loop before the shared SQLite pool is closed. Target loops use their
    /// existing exact claim cleanup path instead of simply dropping handles.
    pub async fn shutdown(&self) -> Result<(), String> {
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
        for (kind, target_id) in keys {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                errors.push("AutoWork target shutdown deadline elapsed".to_owned());
                break;
            }
            if timeout(remaining, self.stop(kind, &target_id)).await.is_err() {
                errors.push(format!(
                    "AutoWork {} {target_id} stop waiter timed out",
                    kind.as_str()
                ));
                break;
            }
        }

        while !self.cleanups.is_empty() && Instant::now() < deadline {
            sleep(Duration::from_millis(20)).await;
        }
        if !self.handles.is_empty() {
            errors.push(format!(
                "AutoWork retained {} target loop(s) after shutdown",
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
        AutoWorkTargetKind::Terminal => TerminalId::try_from(target_id).is_ok(),
    }
}

/// The autowork loop body. Claims -> injects -> waits -> finalizes -> repeats.
///
/// The loop is *persistent*: it does NOT exit when the tag drains or a claim
/// errors —it idles (waking on `deps.wake`, with `IDLE_POLL` as a fallback) and
/// keeps claiming, so a bound session keeps picking up new requirements in the
/// background forever. It exits only on cancel (disable / stop), after
/// `max_requirements` completions, or when a terminal target's session row is
/// deleted. A terminal whose PTY merely exited idles until a relaunch revives it.
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
            .finalize_claim_if_needed(
                requirement_id,
                claim_generation,
                claim_token,
                owner_id,
                kind,
                false,
                None,
                true,
            )
            .await?
        {
            Some(existing) if existing.status == status => Ok(()),
            Some(existing) => Err(AppError::Conflict(format!(
                "exact AutoWork claim generation {claim_generation} was already resolved \
                 as '{}' instead of '{}' for requirement {requirement_id}",
                existing.status.as_db(),
                status.as_db(),
            ))),
            _ => Err(AppError::Conflict(format!(
                "exact AutoWork claim generation {claim_generation} lost authority before \
                 '{}' verdict for requirement {requirement_id}; no exact terminal winner \
                 could be confirmed",
                status.as_db(),
            ))),
        },
    }
}

async fn resolve_interrupted_conversation_claim(
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
                    "{reason} The same SQLite writer transaction could not prove that Conversation \
                     claim generation {claim_generation} had no authority receipt or active turn; \
                     it will not be delivered again."
                ),
                Err(error) => format!(
                    "{reason} Atomic pre-effect abandon for Conversation claim generation \
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

#[allow(clippy::too_many_arguments)]
async fn abandon_pre_effect_or_quarantine(
    deps: &AutoWorkRunnerDeps,
    requirement_id: &str,
    owner_id: &str,
    kind: AutoWorkTargetKind,
    claim_generation: i64,
    claim_token: &str,
    reason: &str,
) -> Result<bool, AppError> {
    match deps
        .service
        .unclaim_busy(
            requirement_id,
            owner_id,
            kind,
            claim_generation,
            claim_token,
        )
        .await
    {
        Ok(true) => Ok(true),
        abandon_result @ (Ok(false) | Err(_)) => {
            let detail = match abandon_result {
                Ok(false) => format!(
                    "{reason} Atomic pre-effect abandon could not prove receiver-admission absence \
                     for claim generation {claim_generation}; the exact capability was quarantined."
                ),
                Err(error) => format!(
                    "{reason} Atomic pre-effect abandon failed for claim generation \
                     {claim_generation}: {error}. The exact capability was quarantined."
                ),
                Ok(true) => unreachable!(),
            };
            resolve_claim_verdict_required(
                &deps.service,
                    requirement_id,
                    claim_generation,
                    claim_token,
                    owner_id,
                    kind,
                    RequirementStatus::NeedsReview,
                    Some(detail),
            )
            .await?;
            Ok(false)
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
    resolve_interrupted_conversation_claim(
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

    match kind {
        AutoWorkTargetKind::Conversation => {
            if let Err(error) = resolve_interrupted_conversation_claim(
                deps,
                &requirement_id,
                target_id,
                claim_generation,
                &claim_token,
                "The AutoWork loop ended without a normal completion boundary.",
            )
            .await
            {
                warn!(
                    conversation_id = target_id,
                    requirement_id,
                    claim_generation,
                    error = %error,
                    "Failed to close an abandoned Conversation claim"
                );
            }
        }
        AutoWorkTargetKind::Terminal => {
            let detail = "The AutoWork loop ended without a normal completion boundary; any \
                          durable Terminal admission was absorbed and will not be written again.";
            let Some(driver) = deps.terminal_driver.as_ref() else {
                if let Err(error) = resolve_claim_verdict_required(
                    &deps.service,
                        &requirement_id,
                        claim_generation,
                        &claim_token,
                        target_id,
                        AutoWorkTargetKind::Terminal,
                        RequirementStatus::NeedsReview,
                        Some(detail.to_owned()),
                )
                .await
                {
                    warn!(
                        terminal_id = target_id,
                        requirement_id,
                        claim_generation,
                        error = %error,
                        "Failed to park an abandoned Terminal claim without a driver"
                    );
                }
                return;
            };

            // This takes the same lifecycle lock as both admitted PTY writers.
            // Because the old handle remains installed until this function
            // returns, no replacement loop can acquire a new admission while
            // broad terminal parking is in progress.
            let park_succeeded = match driver
                .park_open_turn_admissions(target_id, None, detail)
                .await
            {
                Ok(_) => true,
                Err(error) => {
                    warn!(
                        terminal_id = target_id,
                        requirement_id,
                        claim_generation,
                        error = %error,
                        "Failed to park admission for an abandoned Terminal claim"
                    );
                    false
                }
            };

            match driver
                .get_turn_admission_for_claim(
                    target_id,
                    &requirement_id,
                    claim_generation,
                    &claim_token,
                )
                .await
            {
                Ok(None) if park_succeeded => {
                    // The database command is the authoritative proof and
                    // transition. This prior lookup merely avoids a needless
                    // quarantine attempt when an exact receipt is visible.
                    if let Err(error) = abandon_pre_effect_or_quarantine(
                        deps,
                        &requirement_id,
                        target_id,
                        AutoWorkTargetKind::Terminal,
                        claim_generation,
                        &claim_token,
                        "Abandoned Terminal loop found no exact admission.",
                    )
                    .await
                    {
                        warn!(
                            terminal_id = target_id,
                            requirement_id,
                            claim_generation,
                            error = %error,
                            "Failed to safely resolve a pre-admission abandoned Terminal claim"
                        );
                    }
                }
                lookup => {
                    let (status, note) = match lookup {
                        Ok(Some(row)) => match terminal_turn_end_from_receipt(&row) {
                            Some(TerminalTurnEnd::AuthoritativeVerdict { status, note }) => {
                                (status, note)
                            }
                            _ => (
                                RequirementStatus::NeedsReview,
                                Some(detail.to_owned()),
                            ),
                        },
                        Ok(None) => (
                            RequirementStatus::NeedsReview,
                            Some(format!(
                                "{detail} Receipt absence was not proven because parking failed."
                            )),
                        ),
                        Err(error) => (
                            RequirementStatus::NeedsReview,
                            Some(format!("{detail} Exact receipt lookup failed: {error}")),
                        ),
                    };
                    if let Err(error) = resolve_claim_verdict_required(
                        &deps.service,
                            &requirement_id,
                            claim_generation,
                            &claim_token,
                            target_id,
                            AutoWorkTargetKind::Terminal,
                            status,
                            note,
                    )
                    .await
                    {
                        warn!(
                            terminal_id = target_id,
                            requirement_id,
                            claim_generation,
                            error = %error,
                            "Failed to resolve an abandoned Terminal claim"
                        );
                    }
                }
            }
        }
    }
}

enum TurnResult {
    /// Turn finished and finalized as done.
    Done,
    /// Turn errored (re-pended or, when exhausted, failed -> tag paused).
    Errored,
    /// Inject was rejected because the session was busy; the claim was reverted
    /// without consuming an attempt. Back off and retry.
    Busy,
    /// The USER deliberately stopped the turn (conversation cancel). The tag
    /// was paused (`user_interrupted`) and the claim released without consuming
    /// an attempt —the loop idles until the user resumes the tag. NOT a
    /// failure: no backoff, no retry.
    UserInterrupted,
    /// The exact claim verdict/cleanup could not be committed or proven.
    /// Stop this loop rather than claiming another requirement under
    /// authority uncertainty.
    Blocked,
}

/// How a conversation turn ended, from the AutoWork runner's perspective.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TurnEnd {
    /// The durable Conversation receipt completed successfully.
    Clean,
    /// A pre-admission failure that is safe to retry after exact absence proof.
    Errored,
    /// Deliberately cancelled. Engines emit `Finish(Cancelled)` only on the
    /// user-stop path, so this is the event-level user-interrupt signal
    /// (cross-checked with the Session owner's cancellation epoch).
    Cancelled,
    /// Admission happened, but the durable receipt cannot prove a terminal
    /// result. This is absorbing and must be parked for human review.
    Ambiguous,
}

enum DurableConversationReceiptWait {
    Completed(AutoWorkMessageDelivery),
    Ambiguous(String),
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
    target_id: &str,
    kind: AutoWorkTargetKind,
    tag: &str,
    cancelled: Arc<AtomicBool>,
    progress: Arc<LiveProgress>,
    max_requirements: Option<u32>,
    config_revision: String,
) {
    let owner_id = target_id;
    // Close the startup preflight window as well as the per-claim window. The
    // first conversation lease is acquired before ownership verification's
    // first await, then consumed by the first claim iteration.
    let mut startup_conversation_lease = if kind == AutoWorkTargetKind::Conversation {
        match deps
            .conversation
            .begin_runtime_preparation(owner_id, &deps.authoritative_user_id)
        {
            Ok(lease) => Some(lease),
            Err(error) => {
                debug!(target_id, tag, error = %error, "AutoWork startup admission is fenced");
                return;
            }
        }
    } else {
        None
    };
    let owner_check = match kind {
        AutoWorkTargetKind::Conversation => {
            deps.service
                .verify_conversation_owner(owner_id, &deps.authoritative_user_id)
                .await
        }
        AutoWorkTargetKind::Terminal => {
            deps.service
                .verify_terminal_owner(target_id, &deps.authoritative_user_id)
                .await
        }
    };
    if let Err(error) = owner_check {
        warn!(target_id, ?kind, %error, "AutoWork target is not installation-owner owned —not starting");
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

        // NOTE: IDMM is armed PER TURN inside `inject_and_wait` /
        // `inject_and_wait_terminal` (right after the Agent runtime/PTY exists), NOT here.
        // Arming at the loop top fired on every idle poll too —and since an idle
        // conversation has no live Agent runtime, IDMM's probe.observe() got a closed
        // channel and the supervisor died instantly, only to be re-armed 10s later:
        // a runaway "IDMM supervisor armed" churn that did no work. Arming once the
        // turn's runtime exists makes the supervisor actually attach to the turn.

        // Recover an already-active terminal claim BEFORE consulting volatile
        // PTY liveness. A dead PTY cannot prove whether the previous process
        // submitted the prompt; the recovered claim must reach the fail-closed
        // NeedsReview branch below without writing to the terminal again.
        let recovered_terminal_claim = if kind == AutoWorkTargetKind::Terminal {
            match deps
                .service
                .recover_active_claim_for_runner(tag, owner_id, kind, DEFAULT_LEASE_MS)
                .await
            {
                Ok(claim) => claim,
                Err(error) => {
                    warn!(
                        target_id,
                        tag,
                        error = %error,
                        "AutoWork active terminal claim recovery failed —retrying"
                    );
                    sleep(IDLE_POLL).await;
                    continue;
                }
            }
        } else {
            None
        };

        // Only a NEW terminal claim is gated on the current PTY. An ambiguous
        // recovered claim above is parked even when the PTY is offline.
        if recovered_terminal_claim.is_none()
            && kind == AutoWorkTargetKind::Terminal
            && let Some(driver) = &deps.terminal_driver
            && !driver.is_alive(owner_id)
        {
            if matches!(driver.describe(owner_id).await, Ok(None)) {
                info!(target_id, tag, "AutoWork terminal removed —stopping");
                break;
            }
            sleep(IDLE_POLL).await;
            continue;
        }

        // Conversation AutoWork is a runtime initiator. Fence it before the
        // claim await, not inside `inject_and_wait`: otherwise a stop can fully
        // finish while the old claim is pending and that old wakeup can later
        // resurrect a runtime under a fresh lease.
        let claim_started_ms = nomifun_common::now_ms();
        let mut conversation_build_lease = if kind == AutoWorkTargetKind::Conversation {
            let lease = match startup_conversation_lease.take() {
                Some(lease) => Ok(lease),
                None => deps
                    .conversation
                    .begin_runtime_preparation(owner_id, &deps.authoritative_user_id),
            };
            match lease {
                Ok(lease) => {
                    if let Err(error) = lease.ensure_active() {
                        info!(target_id, tag, error = %error, "AutoWork preparation was cancelled before claim");
                        break;
                    }
                    Some(lease)
                }
                Err(error) => {
                    debug!(target_id, tag, error = %error, "AutoWork conversation admission is fenced");
                    if deps
                        .conversation
                        .user_cancelled_since(target_id, claim_started_ms)
                    {
                        info!(target_id, tag, "AutoWork idle preparation was stopped by user");
                        break;
                    }
                    sleep(IDLE_POLL).await;
                    continue;
                }
            }
        } else {
            None
        };

        // Claim the next requirement. The wake future is armed BEFORE the claim
        // (and dropped right after) so a requirement created/re-pended between the
        // claim returning None and our await is never lost. On drain or a transient
        // error the loop idles and retries instead of exiting —persistent by design.
        let claimed = if let Some(claim) = recovered_terminal_claim {
            claim
        } else {
            let wake = deps.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            match deps
                .service
                .claim_next_for_runner(tag, owner_id, kind, DEFAULT_LEASE_MS)
                .await
            {
                Ok(Some(claim)) => claim,
                Ok(None) => {
                    // Tag drained (or paused) -> not a failure spin; reset backoff.
                    consecutive_failures = 0;
                    drop(conversation_build_lease.take());
                    tokio::select! {
                        _ = wake.as_mut() => {}
                        _ = sleep(IDLE_POLL) => {}
                    }
                    continue;
                }
                Err(e) => {
                    warn!(target_id, tag, error = %e, "AutoWork claim failed —retrying");
                    drop(conversation_build_lease.take());
                    tokio::select! {
                        _ = wake.as_mut() => {}
                        _ = sleep(IDLE_POLL) => {}
                    }
                    continue;
                }
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
                // Stamp BEFORE inject: a user cancel at or after this instant
                // can only be aimed at this AutoWork-driven turn (the session
                // is claim-locked while it runs), so it is read as "stop this
                // work", not as a failed attempt.
                let turn_started_ms = claim_started_ms;
                let build_lease = conversation_build_lease
                    .take()
                    .expect("conversation AutoWork acquired a pre-claim runtime lease");
                match inject_and_wait(
                    &deps,
                    target_id,
                    tag,
                    &claimed,
                    claim_generation,
                    &claim_token,
                    recovered_active,
                    build_lease,
                )
                .await
                {
                    Ok((end, note, expects_verdict)) => {
                        // User interrupt: the engine reported Cancelled, OR the
                        // user hit the cancel endpoint during the turn (covers
                        // engines whose cancel path surfaces as a generic
                        // Error). Pause the tag and release the claim instead
                        // of finalizing —re-pending a deliberate stop is what
                        // made AutoWork "resume by itself" seconds after the
                        // user pressed stop.
                        let user_cancelled = end == TurnEnd::Cancelled
                            || deps
                                .conversation
                                .user_cancelled_since(target_id, turn_started_ms);
                        if user_cancelled {
                            info!(
                                target_id,
                                tag,
                                requirement_id = %req_id,
                                "AutoWork turn stopped by user —pausing tag"
                            );
                            if let Err(e) = pause_and_resolve_user_interruption(
                                &deps,
                                &req_id,
                                owner_id,
                                claim_generation,
                                &claim_token,
                                tag,
                            )
                            .await
                            {
                                error!(target_id, requirement_id = %req_id, error = %e, "AutoWork user-interrupt failed");
                            }
                            TurnResult::UserInterrupted
                        } else if end == TurnEnd::Ambiguous {
                            match resolve_claim_verdict_required(
                                &deps.service,
                                    &req_id,
                                    claim_generation,
                                    &claim_token,
                                    owner_id,
                                    AutoWorkTargetKind::Conversation,
                                    RequirementStatus::NeedsReview,
                                    note,
                            )
                            .await
                            {
                                Ok(()) => TurnResult::Done,
                                Err(error) => {
                                    error!(
                                        target_id,
                                        requirement_id = %req_id,
                                        error = %error,
                                        "AutoWork could not park an ambiguous Conversation turn"
                                    );
                                    TurnResult::Blocked
                                }
                            }
                        } else {
                            let turn_errored = end == TurnEnd::Errored;
                            // `note` carries the agent's final plain-text message for tool-free
                            // engines (ACP/codex/gemini) so the platform records what was done.
                            if let Err(e) = deps
                                .service
                                .finalize_claim_if_needed(
                                    &req_id,
                                    claim_generation,
                                    &claim_token,
                                    owner_id,
                                    AutoWorkTargetKind::Conversation,
                                    turn_errored,
                                    note,
                                    expects_verdict,
                                )
                                .await
                            {
                                error!(target_id, requirement_id = %req_id, error = %e, "AutoWork finalize failed");
                            }
                            if turn_errored { TurnResult::Errored } else { TurnResult::Done }
                        }
                    }
                    // The session was busy (a foreground user turn or IDMM owns turn
                    // admission). The requirement's turn never ran —revert its work claim
                    // WITHOUT consuming an attempt, then back off and retry. Without
                    // this, a transient busy window burns the requirement's retries
                    // and falsely fails it (and pauses its tag).
                    Err(AppError::Conflict(conflict)) => {
                        if deps
                            .conversation
                            .user_cancelled_since(target_id, turn_started_ms)
                        {
                            info!(
                                target_id,
                                tag,
                                requirement_id = %req_id,
                                "AutoWork preparation was stopped by user —pausing tag"
                            );
                            if let Err(e) = pause_and_resolve_user_interruption(
                                &deps,
                                &req_id,
                                owner_id,
                                claim_generation,
                                &claim_token,
                                tag,
                            )
                            .await
                            {
                                error!(target_id, requirement_id = %req_id, error = %e, "AutoWork user-interrupt failed");
                            }
                            TurnResult::UserInterrupted
                        } else {
                            warn!(
                                target_id,
                                requirement_id = %req_id,
                                error = %conflict,
                                "AutoWork inject hit a conflict; proving pre-effect absence atomically"
                            );
                            match abandon_pre_effect_or_quarantine(
                                &deps,
                                &req_id,
                                owner_id,
                                kind,
                                claim_generation,
                                &claim_token,
                                &format!("Conversation injection was rejected: {conflict}."),
                            )
                            .await
                            {
                                Ok(true) => TurnResult::Busy,
                                Ok(false) => TurnResult::Done,
                                Err(error) => {
                                    error!(
                                        target_id,
                                        requirement_id = %req_id,
                                        error = %error,
                                        "AutoWork conflict could not be abandoned or quarantined"
                                    );
                                    TurnResult::Blocked
                                }
                            }
                        }
                    }
                    // The Session backing this loop is gone (for example,
                    // deleted while the claim was in flight). This is NOT the
                    // requirement's fault: revert the claim WITHOUT
                    // consuming an attempt (so a deleted session can't burn a
                    // requirement's retries and PAUSE the whole tag for sibling
                    // conversations bound to it (the observed
                    // "delete conv 29 -> tag test stuck" cascade), then STOP
                    // this loop (no target left to drive).
                    Err(AppError::NotFound(not_found)) => {
                        warn!(
                            target_id,
                            requirement_id = %req_id,
                            error = %not_found,
                            "AutoWork target conversation is gone; closing exact claim safely"
                        );
                        if let Err(error) = abandon_pre_effect_or_quarantine(
                            &deps,
                            &req_id,
                            owner_id,
                            kind,
                            claim_generation,
                            &claim_token,
                            &format!("The target Conversation disappeared: {not_found}."),
                        )
                        .await
                        {
                            error!(
                                target_id,
                                requirement_id = %req_id,
                                error = %error,
                                "Deleted-target claim could not be abandoned or quarantined"
                            );
                        }
                        break;
                    }
                    Err(e) => {
                        error!(target_id, requirement_id = %req_id, error = %e, "AutoWork inject failed");
                        // errored turn -> expects_verdict is irrelevant (re-pend / fail).
                        if let Err(e) = deps
                            .service
                            .finalize_claim_if_needed(
                                &req_id,
                                claim_generation,
                                &claim_token,
                                owner_id,
                                AutoWorkTargetKind::Conversation,
                                true,
                                None,
                                false,
                            )
                            .await
                        {
                            error!(target_id, requirement_id = %req_id, error = %e, "AutoWork finalize failed");
                        }
                        TurnResult::Errored
                    }
                }
            }
            AutoWorkTargetKind::Terminal => {
                match inject_and_wait_terminal(
                    &deps,
                    owner_id,
                    tag,
                    &claimed,
                    claim_generation,
                    &claim_token,
                    recovered_active,
                )
                .await
                {
                    Ok(TerminalTurnEnd::AuthoritativeVerdict { status, note }) => {
                        match resolve_claim_verdict_required(
                            &deps.service,
                                &req_id,
                                claim_generation,
                                &claim_token,
                                owner_id,
                                AutoWorkTargetKind::Terminal,
                                status,
                                note,
                        )
                        .await
                        {
                            Ok(()) => {
                                if status == RequirementStatus::Failed {
                                    TurnResult::Errored
                                } else {
                                    TurnResult::Done
                                }
                            }
                            Err(error) => {
                                error!(
                                    target_id,
                                    requirement_id = %req_id,
                                    claim_generation,
                                    error = %error,
                                    "Failed to project durable Terminal verdict onto Requirement"
                                );
                                TurnResult::Blocked
                            }
                        }
                    }
                    Ok(TerminalTurnEnd::AmbiguousAfterSubmission) => {
                        // Once any PTY write was attempted, death, timeout, a
                        // closed/lagged lifecycle stream, or even a partial
                        // two-chunk submit cannot prove the command did not
                        // run. Never re-pend and write it again.
                        let note = format!(
                            "AutoWork terminal claim generation {claim_generation} attempted PTY \
                             submission, but its final outcome is unknown; it was not executed again."
                        );
                        match resolve_claim_verdict_required(
                            &deps.service,
                                &req_id,
                                claim_generation,
                                &claim_token,
                                owner_id,
                                AutoWorkTargetKind::Terminal,
                                RequirementStatus::NeedsReview,
                                Some(note),
                        )
                        .await
                        {
                            Ok(()) => TurnResult::Done,
                            Err(error) => {
                                error!(
                                    target_id,
                                    requirement_id = %req_id,
                                    claim_generation,
                                    error = %error,
                                    "Failed to park ambiguous terminal submission for review"
                                );
                                TurnResult::Blocked
                            }
                        }
                    }
                    Err(e) => {
                        // Preparation failed before any PTY write was
                        // attempted, so the normal retry budget remains safe.
                        error!(target_id, requirement_id = %req_id, error = %e, "AutoWork terminal inject failed");
                        if let Err(e) = deps
                            .service
                            .finalize_claim_if_needed(
                                &req_id,
                                claim_generation,
                                &claim_token,
                                owner_id,
                                AutoWorkTargetKind::Terminal,
                                true,
                                None,
                                false,
                            )
                            .await
                        {
                            error!(target_id, requirement_id = %req_id, error = %e, "AutoWork finalize failed");
                        }
                        TurnResult::Errored
                    }
                }
            }
        };

        // 3. Re-read the final status to count completions + honor max.
        let final_status = deps.service.get(&req_id).await.ok().map(|d| d.status);
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
                let disabled = AutoWorkConfig::normalize(false, None, None)
                    .expect("disabled AutoWork config is canonical");
                let operation_id =
                    format!("autowork:max:v1:{target_id}:{config_revision}");
                if let Err(e) = deps
                    .service
                    .save_autowork_config(
                        &deps.authoritative_user_id,
                        kind,
                        target_id,
                        disabled,
                        &config_revision,
                        Some(&operation_id),
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
            TurnResult::Errored | TurnResult::Busy => {
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

/// Resolve runtime options, acquire the Agent runtime, subscribe, send the prompt, and
/// wait for a terminal event while renewing the lease. Returns
/// `(end, note, expects_verdict)` where `end` classifies how the turn ended
/// (clean / errored / user-cancelled), `note` is the agent's final plain-text
/// message captured for the completion record (only on a clean finish), and
/// `expects_verdict` is true when this engine has an explicit declaration
/// channel (native requirement tools / requirement MCP) so a clean turn with
/// no declaration is parked for review rather than assumed done.
async fn inject_and_wait(
    deps: &Arc<AutoWorkRunnerDeps>,
    conversation_id: &str,
    tag: &str,
    req: &Requirement,
    claim_generation: i64,
    claim_token: &str,
    recovered_active: bool,
    build_lease: AutoWorkRuntimeBuildLease,
) -> Result<(TurnEnd, Option<String>, bool), AppError> {
    build_lease.ensure_scope(&deps.authoritative_user_id, conversation_id)?;
    build_lease.ensure_active()?;
    let preparation = deps
        .conversation
        .prepare_autowork_turn(
            &deps.authoritative_user_id,
            conversation_id,
            &build_lease,
        )
        .await?;
    preparation
        .snapshot
        .ensure_scope(&deps.authoritative_user_id, conversation_id)?;
    build_lease.ensure_snapshot(&preparation.snapshot)?;
    build_lease.ensure_active()?;
    let user_id = deps.authoritative_user_id.to_string();
    let agent_type = preparation.agent_type;
    let workspace = preparation.workspace;

    // Plan without creating the workspace. This yields the exact prompt paths
    // needed for full-payload receipt preflight before runtime/KB activation.
    let ws_path = (!workspace.is_empty()).then(|| std::path::Path::new(workspace.as_str()));
    let attachment_plan = deps
        .service
        .plan_attachments_for_prompt(&req.requirement_id, ws_path)
        .await?;
    build_lease.ensure_snapshot(&preparation.snapshot)?;
    build_lease.ensure_active()?;
    let prompt = build_requirement_prompt(
        tag,
        req,
        claim_generation,
        claim_token,
        agent_type,
        &attachment_plan.attachments,
    );
    let send_message = AutoWorkMessage {
        content: prompt,
        files: vec![],
        inject_skills: vec![],
        hidden: true,
        origin: Some("autowork".into()),
        channel_platform: None,
    };
    // Keep an immutable payload copy for post-error receipt reconciliation.
    // The send seam consumes `send_req`; an error may nevertheless arrive
    // after its atomic receipt INSERT committed, and treating that as
    // pre-admission would mint a new generation and duplicate the turn.
    let send_receipt_probe = send_message.clone();
    let operation_id =
        autowork_turn_idempotency_key(&req.requirement_id, claim_generation, claim_token);
    let expects_verdict = crate::prompt::has_native_requirement_tools(agent_type);

    if recovered_active {
        match deps
            .conversation
            .public_turn_delivery_state(&user_id, conversation_id, &operation_id)
            .await
        {
            Ok(state) => {
                if let Err(reason) =
                    recovered_claim_receipt_gate(&state, claim_generation)
                {
                    return Ok(autowork_blocked_delivery_outcome(reason));
                }
            }
            Err(error) => {
                return Ok(autowork_blocked_delivery_outcome(format!(
                    "the durable receipt for recovered Conversation claim generation \
                     {claim_generation} could not be read: {error}"
                )));
            }
        }
    }
    build_lease.ensure_active()?;

    // Runtime construction and canonical Session projection are receiver-owned.
    // The host resolves model/delegation/workspace/creation identity and applies
    // this narrow overlay only after durable admission.
    let authority = RequirementConversationTurnAuthority {
        requirement_id: req.requirement_id.clone(),
        claim_generation,
        claim_token: claim_token.to_owned(),
    };
    let authority_probe = authority.clone();
    let request = AutoWorkTurnRequest {
        message: send_message,
        runtime_overlay: AutoWorkRuntimeOverlay {
            clear_context: false,
            pre_send_hook: Some(Arc::new(AutoWorkAttachmentActivation {
                service: Arc::clone(&deps.service),
                plan: attachment_plan,
            })),
        },
        session_snapshot: preparation.snapshot,
    };
    let (delivery, send_error_context) = match deps
        .conversation
        .send_turn(
            &user_id,
            conversation_id,
            &operation_id,
            request,
            build_lease,
            authority,
        )
        .await
    {
        Ok(observed) => (observed, None),
        Err(error) => {
            let error_message = error.to_string();
            // An error category (including Conflict) cannot tell us whether
            // receiver admission committed. Its receipt INSERT may already
            // have committed before a later runtime setup/dispatch error was
            // returned. Only an exact durable lookup may classify the turn:
            // present continues the same generation, absent escapes to the
            // caller's atomic pre-effect abandon, and ambiguous is quarantined.
            match deps
                .conversation
                .delivery_result(
                    &user_id,
                    conversation_id,
                    &operation_id,
                    &send_receipt_probe,
                    &authority_probe,
                )
                .await
            {
                Ok(Some(delivery)) => (delivery, Some(error_message)),
                Ok(None) => return Err(error),
                Err(lookup_error) => {
                    return Ok(autowork_blocked_delivery_outcome(format!(
                        "send returned {error}, and exact receipt reconciliation failed: \
                         {lookup_error}"
                    )));
                }
            }
        }
    };
    let reconciled = match reconcile_accepted_autowork_delivery(
        deps.conversation.as_ref(),
        &user_id,
        conversation_id,
        &operation_id,
        delivery,
        CONVERSATION_RECEIPT_RECONCILIATION_TIMEOUT,
    )
    .await
    {
        Ok(reconciled) => reconciled,
        Err(error) => {
            let reason = match send_error_context {
                Some(send_error) => format!(
                    "send returned {send_error}, and accepted receipt reconciliation failed \
                     closed: {error}"
                ),
                None => error.to_string(),
            };
            return Ok(autowork_blocked_delivery_outcome(reason));
        }
    };
    let delivery = reconciled.delivery;
    let receipt_leader = !delivery.replayed;

    if !reconciled.accepted_wait_authorized {
        if let Some(outcome) =
            autowork_replayed_delivery_outcome(&delivery, claim_generation, expects_verdict)
        {
            return Ok(outcome);
        }
    }

    // Arm IDMM only for the atomic receipt winner that actually started this
    // turn. Replay followers must not attach a supervisor to an idle runtime.
    if receipt_leader {
        if let Some(idmm) = &deps.idmm {
            idmm.ensure_supervising(AutoWorkTargetKind::Conversation, conversation_id);
        }
    }

    // Durable completion is polled by the same logical Requirement operation;
    // runtime/event internals remain entirely inside the canonical Session host.
    let outcome = wait_for_conversation_receipt_with_renewal(
        deps.service.as_ref(),
        deps.conversation.as_ref(),
        conversation_id,
        &req.requirement_id,
        claim_generation,
        claim_token,
        &user_id,
        &operation_id,
        &delivery.message_id,
        expects_verdict,
        ConversationReceiptWaitTiming::PRODUCTION,
    )
    .await;
    Ok((
        outcome.0,
        outcome.1,
        expects_verdict || outcome.2,
    ))
}

fn autowork_turn_idempotency_key(
    requirement_id: &str,
    claim_generation: i64,
    claim_token: &str,
) -> String {
    // The public-key field is capped at 128 bytes. Hash the complete,
    // domain-separated capability scope so neither the opaque token nor a
    // variable-length Requirement id is exposed in the durable operation id.
    // The Session owner independently stores/compares the token's SHA-256
    // fingerprint in the receipt payload and validates the raw capability in
    // the atomic Requirement + receipt + Running admission transaction.
    let scope = format!(
        "nomifun-autowork-conversation-turn-v2\0{requirement_id}\0{claim_generation}\0{claim_token}"
    );
    format!(
        "autowork:v2:{}",
        nomifun_auth::token_sha256_hex(&scope)
    )
}

struct ReconciledAutoWorkDelivery {
    delivery: AutoWorkMessageDelivery,
    /// An accepted replay may wait only after the Session owner proves that
    /// the exact operation still has a live local owner or has completed the
    /// audited local orphan-reconciliation path.
    accepted_wait_authorized: bool,
}

fn authorize_accepted_receipt_wait(
    disposition: AutoWorkReconciliationDisposition,
    message_id: &str,
) -> Result<(), AppError> {
    match disposition {
        AutoWorkReconciliationDisposition::LiveExactOwnerWait
        | AutoWorkReconciliationDisposition::ReconciledOrTerminalReRead => Ok(()),
        AutoWorkReconciliationDisposition::ExternalProofRequiredFailClosed => {
            Err(AppError::Conflict(format!(
                "accepted delivery {message_id} belongs to an external or unknown runtime whose \
                 terminal state cannot be proven locally"
            )))
        }
        AutoWorkReconciliationDisposition::StaleConflict => {
            Err(AppError::Conflict(format!(
                "accepted delivery {message_id} no longer owns the exact Conversation operation \
                 generation"
            )))
        }
    }
}

async fn reconcile_accepted_autowork_delivery(
    conversation: &dyn AutoWorkSessionPort,
    user_id: &str,
    conversation_id: &str,
    idempotency_key: &str,
    delivery: AutoWorkMessageDelivery,
    reconciliation_timeout: Duration,
) -> Result<ReconciledAutoWorkDelivery, AppError> {
    if !delivery.replayed || delivery.completed {
        return Ok(ReconciledAutoWorkDelivery {
            delivery,
            accepted_wait_authorized: false,
        });
    }

    // Never infer death from elapsed time. The Session owner holds the
    // preparation gate, re-reads the exact receipt/Running generation, and only
    // local process-backed runtimes with positive parent-exit proof may be
    // terminalized. Remote/OpenClaw/unknown ownership remains accepted and
    // returns Conflict, which the runner parks for explicit review.
    let expected_message_id = delivery.message_id.clone();
    let reconcile = async {
        let disposition = conversation
            .reconcile_quiescent_running_turn(
                user_id,
                conversation_id,
                idempotency_key,
            )
            .await?;
        authorize_accepted_receipt_wait(disposition, &expected_message_id)?;

        match conversation
            .public_turn_delivery_state(user_id, conversation_id, idempotency_key)
            .await?
        {
            AutoWorkTurnDeliveryState::Accepted { message_id }
                if message_id == expected_message_id =>
            {
                Ok(ReconciledAutoWorkDelivery {
                    delivery,
                    accepted_wait_authorized: true,
                })
            }
            AutoWorkTurnDeliveryState::Completed(refreshed)
                if refreshed.message_id == expected_message_id =>
            {
                Ok(ReconciledAutoWorkDelivery {
                    delivery: refreshed,
                    accepted_wait_authorized: false,
                })
            }
            AutoWorkTurnDeliveryState::Accepted { message_id }
            | AutoWorkTurnDeliveryState::Completed(AutoWorkMessageDelivery { message_id, .. }) => {
                Err(AppError::Conflict(format!(
                    "accepted delivery {} resolved to a different immutable message {message_id}",
                    expected_message_id
                )))
            }
            AutoWorkTurnDeliveryState::Missing => Err(AppError::Conflict(format!(
                "accepted delivery {} disappeared during exact quiescent reconciliation",
                expected_message_id
            ))),
        }
    };

    match timeout(reconciliation_timeout, reconcile).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Conflict(format!(
            "accepted delivery {expected_message_id} reconciliation exceeded its {} ms deadline",
            reconciliation_timeout.as_millis()
        ))),
    }
}

fn autowork_blocked_delivery_outcome(reason: String) -> (TurnEnd, Option<String>, bool) {
    (
        TurnEnd::Ambiguous,
        Some(format!(
            "AutoWork did not start another turn because durable Conversation \
             state is ambiguous: {reason}. Explicit reset or human review is required."
        )),
        true,
    )
}

fn recovered_claim_receipt_gate(
    state: &AutoWorkTurnDeliveryState,
    claim_generation: i64,
) -> Result<(), String> {
    match state {
        AutoWorkTurnDeliveryState::Accepted { .. }
        | AutoWorkTurnDeliveryState::Completed(_) => Ok(()),
        AutoWorkTurnDeliveryState::Missing => Err(format!(
            "recovered Conversation claim generation {claim_generation} has no durable \
             delivery receipt; its prior execution outcome cannot be proven"
        )),
    }
}

fn autowork_replayed_delivery_outcome(
    delivery: &AutoWorkMessageDelivery,
    claim_generation: i64,
    expects_verdict: bool,
) -> Option<(TurnEnd, Option<String>, bool)> {
    if !delivery.replayed {
        return None;
    }
    if !delivery.completed {
        // `accepted` is absorbing: the prior process may have crossed an
        // irreversible model/tool boundary before crashing. Never wait on a
        // newly subscribed idle runtime and never manufacture a new attempt.
        // Forcing the verdict contract parks the Requirement in NeedsReview.
        return Some((
            TurnEnd::Ambiguous,
            Some(format!(
                "AutoWork delivery {} was already accepted for claim generation \
                 {claim_generation}; its outcome is unknown, so it was not executed again.",
                delivery.message_id
            )),
            true,
        ));
    }

    let note = delivery
        .result_text
        .as_deref()
        .and_then(finalize_note)
        .or_else(|| delivery.result_error.as_deref().and_then(finalize_note));
    match delivery.result_ok {
        Some(true) => Some((TurnEnd::Clean, note, expects_verdict)),
        // A completed error proves only that the observer saw an error; it
        // cannot prove that earlier model/tool effects did not happen. A new
        // claim generation would mint a different delivery key and could
        // execute the same Requirement twice, so absorb it for review.
        Some(false) => Some((
            TurnEnd::Ambiguous,
            note.or_else(|| {
                Some(format!(
                    "Completed AutoWork delivery {} reported an error after durable admission; \
                     it was not executed again.",
                    delivery.message_id
                ))
            }),
            true,
        )),
        None => Some((
            TurnEnd::Ambiguous,
            note.or_else(|| {
                Some(format!(
                    "Completed AutoWork delivery {} has no durable outcome; \
                     it was not executed again.",
                    delivery.message_id
                ))
            }),
            true,
        )),
    }
}

/// Observe one already-admitted AutoWork Conversation turn through its durable
/// receipt while renewing the exact Requirement capability.
///
/// Only the unique logical AutoWork receipt decides whether the work finished.
/// Once this function starts, absence, lookup failure, lease loss and timeout
/// are all ambiguous post-admission outcomes and therefore force NeedsReview.
#[derive(Clone, Copy)]
struct ConversationReceiptWaitTiming {
    lease_renew_interval: Duration,
    receipt_poll_interval: Duration,
    hard_timeout: Duration,
}

impl ConversationReceiptWaitTiming {
    const PRODUCTION: Self = Self {
        lease_renew_interval: LEASE_RENEW_INTERVAL,
        receipt_poll_interval: CONVERSATION_RECEIPT_POLL_INTERVAL,
        hard_timeout: TURN_TIMEOUT,
    };
}

#[allow(clippy::too_many_arguments)]
async fn wait_for_conversation_receipt_with_renewal(
    service: &RequirementService,
    conversation: &dyn AutoWorkSessionPort,
    conversation_id: &str,
    req_id: &str,
    claim_generation: i64,
    claim_token: &str,
    user_id: &str,
    operation_id: &str,
    expected_message_id: &str,
    expects_verdict: bool,
    timing: ConversationReceiptWaitTiming,
) -> (TurnEnd, Option<String>, bool) {
    let mut renew = interval(timing.lease_renew_interval);
    renew.tick().await;
    let mut receipts = interval(timing.receipt_poll_interval);
    let wait = async {
        loop {
            tokio::select! {
                _ = renew.tick() => {
                    match service
                        .renew_lease(
                            req_id,
                            conversation_id,
                            AutoWorkTargetKind::Conversation,
                            claim_generation,
                            claim_token,
                            DEFAULT_LEASE_MS,
                        )
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "AutoWork Conversation claim generation {claim_generation} lost \
                                 exact lease authority after durable admission; it was not \
                                 executed again."
                            ));
                        }
                        Err(error) => {
                            warn!(
                                conversation_id,
                                requirement_id = req_id,
                                error = %error,
                                "Exact lease renewal failed while polling durable AutoWork receipt"
                            );
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "Lease renewal failed after durable admission of AutoWork \
                                 Conversation claim generation {claim_generation}: {error}. The \
                                 Requirement was not executed again."
                            ));
                        }
                    }
                }
                _ = receipts.tick() => {
                    match conversation
                        .public_turn_delivery_state(
                            user_id,
                            conversation_id,
                            operation_id,
                        )
                        .await
                    {
                        Ok(AutoWorkTurnDeliveryState::Completed(delivery))
                            if delivery.message_id == expected_message_id && delivery.completed =>
                        {
                            return DurableConversationReceiptWait::Completed(delivery);
                        }
                        Ok(AutoWorkTurnDeliveryState::Completed(delivery)) => {
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "The durable AutoWork receipt for Conversation claim generation \
                                 {claim_generation} changed message identity or was not \
                                 terminally completed (expected {expected_message_id}, got \
                                 {}); the Requirement was not executed again.",
                                delivery.message_id
                            ));
                        }
                        Ok(AutoWorkTurnDeliveryState::Accepted { message_id })
                            if message_id == expected_message_id => {}
                        Ok(AutoWorkTurnDeliveryState::Accepted { message_id }) => {
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "The accepted AutoWork receipt for Conversation claim generation \
                                 {claim_generation} changed message identity (expected \
                                 {expected_message_id}, got {message_id}); the Requirement was \
                                 not executed again."
                            ));
                        }
                        Ok(AutoWorkTurnDeliveryState::Missing) => {
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "The exact AutoWork receipt for Conversation claim generation \
                                 {claim_generation} disappeared after admission; the Requirement \
                                 was not executed again."
                            ));
                        }
                        Err(error) => {
                            return DurableConversationReceiptWait::Ambiguous(format!(
                                "The exact AutoWork receipt for Conversation claim generation \
                                 {claim_generation} could not be verified after admission: \
                                 {error}. The Requirement was not executed again."
                            ));
                        }
                    }
                }
            }
        }
    };

    match timeout(timing.hard_timeout, wait).await {
        Ok(DurableConversationReceiptWait::Completed(delivery)) => {
            autowork_replayed_delivery_outcome(
                &delivery,
                claim_generation,
                expects_verdict,
            )
            .unwrap_or_else(|| {
                autowork_blocked_delivery_outcome(
                    "durable receipt observation returned fresh execution authority".to_owned(),
                )
            })
        }
        Ok(DurableConversationReceiptWait::Ambiguous(detail)) => {
            (TurnEnd::Ambiguous, Some(detail), true)
        }
        Err(_) => (
            TurnEnd::Ambiguous,
            Some(format!(
                "AutoWork Conversation claim generation {claim_generation} exceeded its hard \
                 timeout after durable admission; prior model/tool effects cannot be excluded and \
                 the Requirement was not executed again."
            )),
            true,
        ),
    }
}

/// Bounded, escalating delay before the next claim after a failed (or busy)
/// turn, so a deterministic failure cannot spin back into claim at millisecond
/// speed and burn every attempt across the tag in a fraction of a second.
/// `consecutive` is the count of back-to-back failed turns (1-based): 1s, 2s,
/// 4s, 8s, 16s, then capped at 30s. Reset to 0 on success / idle.
fn failure_backoff(consecutive: u32) -> Duration {
    let exp = consecutive.saturating_sub(1).min(5);
    let secs = (1u64 << exp).min(30);
    Duration::from_secs(secs)
}

/// Trim + tail-truncate the accumulated agent text into a completion note.
/// `None` when the agent produced no prose (e.g. only tool calls).
fn finalize_note(buf: &str) -> Option<String> {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Find the retained tail without counting or allocating the entire note.
    match trimmed.char_indices().rev().nth(MAX_NOTE_CHARS - 1) {
        Some((start, _)) if start > 0 => Some(format!("…{}", &trimmed[start..])),
        _ => Some(trimmed.to_owned()),
    }
}

/// How a terminal turn ended (structured completion via lifecycle / error).
#[derive(Clone, PartialEq, Eq, Debug)]
enum TerminalTurnEnd {
    /// The lifecycle reported a `TurnEnd` event —the agent finished its turn.
    /// Whether the agent called `requirement_complete` is reflected in the DB
    /// row's status; the AutoWork runner just knows the turn ended cleanly.
    AuthoritativeVerdict {
        status: RequirementStatus,
        note: Option<String>,
    },
    /// At least one PTY submission write was attempted, but PTY death, hard
    /// timeout, or lifecycle loss prevents proving the command's final state.
    /// This is absorbing and must be parked for review, never retried.
    AmbiguousAfterSubmission,
}

/// Inject a prompt into a terminal CLI and submit it. The bracketed-paste body
/// and the submit CR are written as SEPARATE PTY writes, with
/// `TERMINAL_SUBMIT_DELAY` between them. A CR that rides in the same write as
/// the paste-end marker is swallowed by the paste-burst detection modern agent
/// TUIs (claude/codex/gemini) use to keep a pasted block from auto-running —it
/// leaves the requirement text sitting unsubmitted in the input box (the bug
/// this fixes). Writing the CR on its own, a beat later, makes the TUI treat it
/// as a real Enter keystroke. Mirrors the cron terminal executor's fix.
fn terminal_turn_end_from_receipt(row: &TerminalTurnAdmissionRow) -> Option<TerminalTurnEnd> {
    if row.phase != "settled" {
        return None;
    }
    let status = match row.outcome.as_deref()? {
        "done" => RequirementStatus::Done,
        "failed" => RequirementStatus::Failed,
        "needs_review" => RequirementStatus::NeedsReview,
        "cancelled" => RequirementStatus::Cancelled,
        _ => return None,
    };
    Some(TerminalTurnEnd::AuthoritativeVerdict {
        status,
        note: row.detail.clone(),
    })
}

fn terminal_outcome_from_status(status: RequirementStatus) -> Option<TerminalTurnOutcome> {
    match status {
        RequirementStatus::Done => Some(TerminalTurnOutcome::Done),
        RequirementStatus::Failed => Some(TerminalTurnOutcome::Failed),
        RequirementStatus::NeedsReview => Some(TerminalTurnOutcome::NeedsReview),
        RequirementStatus::Cancelled => Some(TerminalTurnOutcome::Cancelled),
        RequirementStatus::Pending | RequirementStatus::InProgress => None,
    }
}

async fn park_terminal_turn(
    deps: &Arc<AutoWorkRunnerDeps>,
    driver: &Arc<dyn TerminalDriver>,
    key: &TerminalTurnAdmissionKey,
    detail: &str,
) -> TerminalTurnEnd {
    if let Err(error) = driver
        .park_open_turn_admissions(&key.terminal_id, Some(key.pty_epoch), detail)
        .await
    {
        warn!(
            terminal_id = %key.terminal_id,
            requirement_id = %key.requirement_id,
            claim_generation = key.claim_generation,
            error = %error,
            "Failed to atomically park an ambiguous Terminal turn"
        );
        if let Err(status_error) = resolve_claim_verdict_required(
            &deps.service,
                &key.requirement_id,
                key.claim_generation,
                &key.claim_token,
                &key.terminal_id,
                AutoWorkTargetKind::Terminal,
                RequirementStatus::NeedsReview,
                Some(detail.to_owned()),
        )
        .await
        {
            warn!(
                terminal_id = %key.terminal_id,
                requirement_id = %key.requirement_id,
                error = %status_error,
                "Failed to park ambiguous Terminal Requirement directly"
            );
        }
    }

    match driver.get_turn_admission(key).await {
        Ok(Some(row)) => terminal_turn_end_from_receipt(&row)
            .unwrap_or(TerminalTurnEnd::AmbiguousAfterSubmission),
        Ok(None) => {
            warn!(
                terminal_id = %key.terminal_id,
                requirement_id = %key.requirement_id,
                "Durable Terminal admission disappeared while parking"
            );
            TerminalTurnEnd::AmbiguousAfterSubmission
        }
        Err(error) => {
            warn!(
                terminal_id = %key.terminal_id,
                requirement_id = %key.requirement_id,
                error = %error,
                "Failed to re-read parked Terminal admission"
            );
            TerminalTurnEnd::AmbiguousAfterSubmission
        }
    }
}

async fn submit_admitted_terminal_prompt(
    driver: &Arc<dyn TerminalDriver>,
    key: &TerminalTurnAdmissionKey,
    prompt: &str,
) -> Result<TerminalTurnEffectsStart, AppError> {
    match nomifun_terminal::encode_submit_chunks(prompt, true) {
        nomifun_terminal::SubmitChunks::PasteThenCr { paste, cr } => {
            let body = driver.write_admitted_body(key, &paste).await?;
            if body != TerminalTurnEffectsStart::Started {
                return Ok(body);
            }
            sleep(nomifun_terminal::TERMINAL_SUBMIT_DELAY).await;
            Ok(driver.write_admitted_submit(key, &cr).await?)
        }
        nomifun_terminal::SubmitChunks::Single(bytes) => {
            Ok(driver.write_admitted_turn(key, &bytes).await?)
        }
    }
}

/// One terminal turn: inject the requirement prompt, then await the lifecycle
/// `TurnEnd` event (the agent's Stop hook), the PTY dying, or the hard timeout.
///
/// **No quiescence fallback:** a lifecycle subscription is the ONLY structured
/// turn-end signal. When lifecycle is unavailable (server not wired / non-agent
/// CLI) the turn runs until the hard `TURN_TIMEOUT` and then ends as
/// `TerminalTurnEnd::AmbiguousAfterSubmission` —honest and at-most-once (no
/// false "done", and no second PTY injection).
async fn inject_and_wait_terminal(
    deps: &Arc<AutoWorkRunnerDeps>,
    terminal_id: &str,
    tag: &str,
    req: &Requirement,
    claim_generation: i64,
    claim_token: &str,
    recovered_active: bool,
) -> Result<TerminalTurnEnd, AppError> {
    let Some(driver) = deps.terminal_driver.as_ref() else {
        if recovered_active {
            let detail = format!(
                "Recovered AutoWork Terminal claim generation {claim_generation} without a \
                 Terminal driver; prior effects cannot be excluded."
            );
            if let Err(error) = resolve_claim_verdict_required(
                &deps.service,
                    &req.requirement_id,
                    claim_generation,
                    claim_token,
                    terminal_id,
                    AutoWorkTargetKind::Terminal,
                    RequirementStatus::NeedsReview,
                    Some(detail),
            )
            .await
            {
                warn!(
                    terminal_id,
                    requirement_id = %req.requirement_id,
                    claim_generation,
                    error = %error,
                    "Failed to park recovered Terminal claim without a driver"
                );
            }
            return Ok(TerminalTurnEnd::AmbiguousAfterSubmission);
        }
        return Err(AppError::Internal("terminal driver not attached".into()));
    };

    let Some(pty_epoch) = driver.current_epoch(terminal_id) else {
        if recovered_active {
            let detail = format!(
                "Recovered AutoWork Terminal claim generation {claim_generation} has no live PTY \
                 generation; prior effects cannot be excluded."
            );
            if let Err(error) = resolve_claim_verdict_required(
                &deps.service,
                    &req.requirement_id,
                    claim_generation,
                    claim_token,
                    terminal_id,
                    AutoWorkTargetKind::Terminal,
                    RequirementStatus::NeedsReview,
                    Some(detail),
            )
            .await
            {
                warn!(
                    terminal_id,
                    requirement_id = %req.requirement_id,
                    claim_generation,
                    error = %error,
                    "Failed to park recovered Terminal claim without a live PTY"
                );
            }
            return Ok(TerminalTurnEnd::AmbiguousAfterSubmission);
        }
        return Err(AppError::Conflict(format!(
            "terminal {terminal_id} has no live PTY generation"
        )));
    };

    // Build the exact Terminal payload before durable turn admission. The
    // legacy best-effort staging wrapper collapsed source-integrity failures
    // into an empty attachment list, silently changing the requested work.
    let attachment_plan = deps
        .service
        .plan_attachments_for_prompt(&req.requirement_id, None)
        .await?;
    // Live per-turn knowledge state (RC-5): the hint is included only when the
    // workspace actually has bases mounted at THIS turn's start.
    let knowledge_mounted = driver.knowledge_mounted(terminal_id).await;
    let prompt = build_terminal_requirement_prompt(
        tag,
        req,
        claim_generation,
        claim_token,
        &attachment_plan.attachments,
        knowledge_mounted,
    );

    let scope = TerminalTurnAdmissionScope {
        terminal_id: terminal_id.to_owned(),
        pty_epoch,
        requirement_id: req.requirement_id.clone(),
        claim_generation,
        claim_token: claim_token.to_owned(),
    };
    let claim = match driver.claim_turn_admission(&scope).await {
        Ok(claim) => claim,
        Err(first_error) => {
            // The reply may have been lost after COMMIT. Repeat the same scope
            // only to recover/create an absorbing row, then park without ever
            // granting write authority.
            match driver.claim_turn_admission(&scope).await {
                Ok(claim) => {
                    let key = match TerminalTurnAdmissionKey::from_row(&claim.row) {
                        Ok(key) => key,
                        Err(error) => {
                            let detail = format!(
                                "Recovered Terminal admission for AutoWork claim generation \
                                 {claim_generation} was invalid and was not executed."
                            );
                            warn!(
                                terminal_id,
                                requirement_id = %req.requirement_id,
                                error = %error,
                                "Invalid recovered Terminal admission"
                            );
                            let _ = driver
                                .park_open_turn_admissions(terminal_id, None, &detail)
                                .await;
                            if let Err(error) = resolve_claim_verdict_required(
                                &deps.service,
                                    &req.requirement_id,
                                    claim_generation,
                                    claim_token,
                                    terminal_id,
                                    AutoWorkTargetKind::Terminal,
                                    RequirementStatus::NeedsReview,
                                    Some(detail),
                            )
                            .await
                            {
                                warn!(
                                    terminal_id,
                                    requirement_id = %req.requirement_id,
                                    claim_generation,
                                    error = %error,
                                    "Failed to park invalid recovered Terminal admission"
                                );
                            }
                            return Ok(TerminalTurnEnd::AmbiguousAfterSubmission);
                        }
                    };
                    let detail = format!(
                        "Terminal admission for AutoWork claim generation \
                         {claim_generation} returned an uncertain result and was not executed."
                    );
                    warn!(
                        terminal_id,
                        requirement_id = %req.requirement_id,
                        error = %first_error,
                        "Recovered an uncertain durable Terminal admission"
                    );
                    return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
                }
                Err(second_error) => {
                    let detail = format!(
                        "Terminal admission for AutoWork claim generation \
                         {claim_generation} could not be verified and was not retried."
                    );
                    warn!(
                        terminal_id,
                        requirement_id = %req.requirement_id,
                        first_error = %first_error,
                        error = %second_error,
                        "Unable to verify durable Terminal admission"
                    );
                    if let Err(status_error) = resolve_claim_verdict_required(
                        &deps.service,
                            &req.requirement_id,
                            claim_generation,
                            claim_token,
                            terminal_id,
                            AutoWorkTargetKind::Terminal,
                            RequirementStatus::NeedsReview,
                            Some(detail),
                    )
                    .await
                    {
                        warn!(
                            terminal_id,
                            requirement_id = %req.requirement_id,
                            error = %status_error,
                            "Failed to park Requirement after uncertain Terminal admission"
                        );
                    }
                    return Ok(TerminalTurnEnd::AmbiguousAfterSubmission);
                }
            }
        }
    };
    let key = match TerminalTurnAdmissionKey::from_row(&claim.row) {
        Ok(key) => key,
        Err(error) => {
            let detail = format!(
                "Terminal admission receipt for AutoWork claim generation \
                 {claim_generation} is invalid and was not executed."
            );
            warn!(
                terminal_id,
                requirement_id = %req.requirement_id,
                error = %error,
                "Invalid durable Terminal admission receipt"
            );
            let _ = driver
                .park_open_turn_admissions(terminal_id, None, &detail)
                .await;
            if let Err(error) = resolve_claim_verdict_required(
                &deps.service,
                    &req.requirement_id,
                    claim_generation,
                    claim_token,
                    terminal_id,
                    AutoWorkTargetKind::Terminal,
                    RequirementStatus::NeedsReview,
                    Some(detail),
            )
            .await
            {
                warn!(
                    terminal_id,
                    requirement_id = %req.requirement_id,
                    claim_generation,
                    error = %error,
                    "Failed to park invalid Terminal admission receipt"
                );
            }
            return Ok(TerminalTurnEnd::AmbiguousAfterSubmission);
        }
    };

    if !claim.claimed_new {
        if let Some(settled) = terminal_turn_end_from_receipt(&claim.row) {
            return Ok(settled);
        }
        let detail = format!(
            "Replayed open Terminal admission for AutoWork claim generation \
             {claim_generation}; prior PTY effects are unknown and were not executed again."
        );
        return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
    }

    if recovered_active {
        let detail = format!(
            "Recovered AutoWork Terminal claim generation {claim_generation} without a prior \
             durable admission; pre-recovery PTY effects cannot be excluded, so it was not executed."
        );
        return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
    }

    let lifecycle_rx = match driver.subscribe_lifecycle_exact(terminal_id, pty_epoch) {
        Some(receiver) => receiver,
        None => {
            let detail = format!(
                "Exact lifecycle subscription was unavailable for AutoWork Terminal claim \
                 generation {claim_generation}; the admitted turn was not executed."
            );
            return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
        }
    };

    // Terminals have no workspace concept —the prompt carries absolute paths
    // into the data dir and the CLI reads them directly.
    let effects = match submit_admitted_terminal_prompt(driver, &key, &prompt).await {
        Ok(effects) => effects,
        Err(error) => {
            let detail = format!(
                "AutoWork Terminal claim generation {claim_generation} crossed or may have crossed \
                 its PTY effects boundary, but submission failed: {error}"
            );
            warn!(
                terminal_id,
                requirement_id = %req.requirement_id,
                error = %error,
                "AutoWork Terminal submission may have been partially delivered"
            );
            return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
        }
    };
    if effects != TerminalTurnEffectsStart::Started {
        let detail = format!(
            "AutoWork Terminal claim generation {claim_generation} replayed an already-started or \
             settled admission and emitted no PTY bytes."
        );
        return Ok(park_terminal_turn(deps, driver, &key, &detail).await);
    }

    // Arm IDMM for THIS terminal turn (its probe subscribes to the durable PTY
    // output/lifecycle, so it attaches regardless of task state). Per-turn, not on
    // every idle poll —same churn fix as the conversation path. Idempotent + a
    // no-op when IDMM is disabled for the terminal.
    if let Some(idmm) = &deps.idmm {
        idmm.ensure_supervising(AutoWorkTargetKind::Terminal, terminal_id);
    }

    Ok(wait_terminal_turn_end(deps, driver, &key, lifecycle_rx).await)
}

/// Await a terminal turn's structured completion signal, renewing the lease on a
/// tick, checking PTY liveness, and enforcing the hard timeout.
async fn wait_terminal_turn_end(
    deps: &Arc<AutoWorkRunnerDeps>,
    driver: &Arc<dyn TerminalDriver>,
    key: &TerminalTurnAdmissionKey,
    mut lifecycle_rx: ExactTerminalLifecycleReceiver,
) -> TerminalTurnEnd {
    let mut renew = interval(LEASE_RENEW_INTERVAL);
    renew.tick().await; // consume the immediate first tick
    let mut tick = interval(Duration::from_secs(2));
    tick.tick().await; // consume the immediate first tick

    let fut = async {

        loop {
            tokio::select! {
                _ = renew.tick() => {
                    match deps
                        .service
                        .renew_lease(
                            &key.requirement_id,
                            &key.terminal_id,
                            AutoWorkTargetKind::Terminal,
                            key.claim_generation,
                            &key.claim_token,
                            DEFAULT_LEASE_MS,
                        )
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            let detail = format!(
                                "Exact lease authority was lost after Terminal effects started \
                                 for AutoWork claim generation {}; its final outcome is unknown.",
                                key.claim_generation
                            );
                            return park_terminal_turn(deps, driver, key, &detail).await;
                        }
                        Err(error) => {
                            let detail = format!(
                                "Lease renewal failed after Terminal effects started for AutoWork \
                                 claim generation {}: {error}",
                                key.claim_generation
                            );
                            return park_terminal_turn(deps, driver, key, &detail).await;
                        }
                    }
                }
                _ = tick.tick() => {
                    if driver.current_epoch(&key.terminal_id) != Some(key.pty_epoch) {
                        let detail = format!(
                            "PTY generation changed after AutoWork Terminal claim generation {} \
                             started; its final outcome is unknown.",
                            key.claim_generation
                        );
                        return park_terminal_turn(deps, driver, key, &detail).await;
                    }
                }
                event = lifecycle_rx.recv() => {
                    match event {
                        Ok(event) if event.kind == LifecycleKind::TurnEnd => {
                            if event.turn_token().is_none() {
                                debug!(
                                    terminal_id = %key.terminal_id,
                                    requirement_id = %key.requirement_id,
                                    "Unscoped TurnEnd only wakes an authoritative Requirement recheck"
                                );
                            }
                            match deps.service.get(&key.requirement_id).await {
                                Ok(requirement) => {
                                    let Some(outcome) =
                                        terminal_outcome_from_status(requirement.status)
                                    else {
                                        let detail = format!(
                                            "Terminal TurnEnd for AutoWork claim generation {} had \
                                             no authoritative Requirement verdict; it was not \
                                             treated as successful.",
                                            key.claim_generation
                                        );
                                        return park_terminal_turn(deps, driver, key, &detail).await;
                                    };
                                    if let Err(error) = driver
                                        .settle_turn_admission(
                                            key,
                                            outcome,
                                            requirement.completion_note.as_deref(),
                                        )
                                        .await
                                    {
                                        warn!(
                                            terminal_id = %key.terminal_id,
                                            requirement_id = %key.requirement_id,
                                            error = %error,
                                            "Failed to copy Requirement verdict into Terminal receipt"
                                        );
                                        let _ = driver
                                            .park_open_turn_admissions(
                                                &key.terminal_id,
                                                Some(key.pty_epoch),
                                                "Requirement reached an authoritative verdict, but Terminal receipt settlement failed.",
                                            )
                                            .await;
                                    }
                                    return TerminalTurnEnd::AuthoritativeVerdict {
                                        status: requirement.status,
                                        note: requirement.completion_note,
                                    };
                                }
                                Err(error) => {
                                    let detail = format!(
                                        "Terminal TurnEnd for AutoWork claim generation {} could \
                                         not re-read its authoritative Requirement verdict: {error}",
                                        key.claim_generation
                                    );
                                    return park_terminal_turn(deps, driver, key, &detail).await;
                                }
                            }
                        }
                        Ok(_) => continue,
                        Err(broadcast::error::RecvError::Closed) => {
                            let detail = format!(
                                "Exact lifecycle stream closed after AutoWork Terminal claim \
                                 generation {} started.",
                                key.claim_generation
                            );
                            return park_terminal_turn(deps, driver, key, &detail).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            let detail = format!(
                                "Exact lifecycle stream skipped {skipped} event(s) after AutoWork \
                                 Terminal claim generation {} started.",
                                key.claim_generation
                            );
                            return park_terminal_turn(deps, driver, key, &detail).await;
                        }
                    }
                }
            }
        }
    };

    match timeout(TURN_TIMEOUT, fut).await {
        Ok(end) => end,
        Err(_) => {
            let detail = format!(
                "AutoWork Terminal claim generation {} exceeded its hard timeout after effects \
                 started; its final outcome is unknown.",
                key.claim_generation
            );
            park_terminal_turn(deps, driver, key, &detail).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation_port::compatibility::{
        TestMessageDelivery as IdempotentMessageDelivery,
        TestReconciliationDisposition as BackgroundTurnReconciliationDisposition,
        TestRuntimeBuildLease as RuntimeBuildLease,
        TestTurnDeliveryState as PublicTurnDeliveryState,
    };
    use nomifun_terminal::TerminalDescription;
    use nomifun_terminal::error::TerminalError;

    #[test]
    fn completion_note_preserves_trim_and_unicode_tail_at_the_character_limit() {
        assert_eq!(finalize_note(" \n\t"), None);
        assert_eq!(finalize_note("  中文🙂  ").as_deref(), Some("中文🙂"));
        for count in [MAX_NOTE_CHARS - 1, MAX_NOTE_CHARS, MAX_NOTE_CHARS + 1] {
            let text = "界".repeat(count - 1) + "🙂";
            let expected = if count > MAX_NOTE_CHARS {
                "…".to_owned() + &"界".repeat(MAX_NOTE_CHARS - 1) + "🙂"
            } else {
                text.clone()
            };
            assert_eq!(finalize_note(&format!(" {text} ")), Some(expected));
        }
    }

    #[test]
    fn failure_backoff_escalates_and_caps() {
        // 1-based consecutive failures -> 1s, 2s, 4s, 8s, 16s, then capped at 30s.
        assert_eq!(failure_backoff(1), Duration::from_secs(1));
        assert_eq!(failure_backoff(2), Duration::from_secs(2));
        assert_eq!(failure_backoff(3), Duration::from_secs(4));
        assert_eq!(failure_backoff(4), Duration::from_secs(8));
        assert_eq!(failure_backoff(5), Duration::from_secs(16));
        assert_eq!(failure_backoff(6), Duration::from_secs(30), "capped at 30s");
        assert_eq!(failure_backoff(100), Duration::from_secs(30), "stays capped");
        // Never zero —a failure must always insert some delay before re-claim.
        assert!(failure_backoff(1) > Duration::ZERO);
    }

    struct RecordingDriver {
        writes: Mutex<Vec<Vec<u8>>>,
        effects_start: Mutex<TerminalTurnEffectsStart>,
        body_start: Mutex<TerminalTurnEffectsStart>,
        submit_start: Mutex<TerminalTurnEffectsStart>,
        epoch: u64,
    }

    impl Default for RecordingDriver {
        fn default() -> Self {
            Self {
                writes: Mutex::new(Vec::new()),
                effects_start: Mutex::new(TerminalTurnEffectsStart::Started),
                body_start: Mutex::new(TerminalTurnEffectsStart::Started),
                submit_start: Mutex::new(TerminalTurnEffectsStart::Started),
                epoch: 1,
            }
        }
    }

    #[async_trait::async_trait]
    impl TerminalDriver for RecordingDriver {
        async fn write_input(&self, _id: &str, bytes: &[u8]) -> Result<(), TerminalError> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        fn current_epoch(&self, _id: &str) -> Option<u64> {
            Some(self.epoch)
        }
        async fn write_input_exact_epoch(
            &self,
            id: &str,
            pty_epoch: u64,
            bytes: &[u8],
        ) -> Result<(), TerminalError> {
            if pty_epoch != self.epoch {
                return Err(TerminalError::StaleGeneration(id.to_owned()));
            }
            self.writes.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        async fn write_admitted_turn(
            &self,
            _key: &TerminalTurnAdmissionKey,
            bytes: &[u8],
        ) -> Result<TerminalTurnEffectsStart, TerminalError> {
            let effects = *self.effects_start.lock().unwrap();
            if effects == TerminalTurnEffectsStart::Started {
                self.writes.lock().unwrap().push(bytes.to_vec());
            }
            Ok(effects)
        }
        async fn write_admitted_body(
            &self,
            _key: &TerminalTurnAdmissionKey,
            bytes: &[u8],
        ) -> Result<TerminalTurnEffectsStart, TerminalError> {
            let effects = *self.body_start.lock().unwrap();
            if effects == TerminalTurnEffectsStart::Started {
                self.writes.lock().unwrap().push(bytes.to_vec());
            }
            Ok(effects)
        }
        async fn write_admitted_submit(
            &self,
            _key: &TerminalTurnAdmissionKey,
            bytes: &[u8],
        ) -> Result<TerminalTurnEffectsStart, TerminalError> {
            let effects = *self.submit_start.lock().unwrap();
            if effects == TerminalTurnEffectsStart::Started {
                self.writes.lock().unwrap().push(bytes.to_vec());
            }
            Ok(effects)
        }
        fn subscribe_output(&self, _id: &str) -> Option<broadcast::Receiver<Vec<u8>>> {
            None
        }
        fn is_alive(&self, _id: &str) -> bool {
            true
        }
        async fn describe(&self, _id: &str) -> Result<Option<TerminalDescription>, TerminalError> {
            Ok(None)
        }
        async fn read_autowork(&self, _id: &str) -> Result<Option<String>, TerminalError> {
            Ok(None)
        }
        async fn write_autowork(&self, _id: &str, _autowork: Option<&str>) -> Result<(), TerminalError> {
            Ok(())
        }
        async fn read_idmm(&self, _id: &str) -> Result<Option<String>, TerminalError> {
            Ok(None)
        }
        async fn write_idmm(&self, _id: &str, _idmm: Option<&str>) -> Result<(), TerminalError> {
            Ok(())
        }
        fn subscribe_lifecycle(
            &self,
            _id: &str,
        ) -> Option<tokio::sync::broadcast::Receiver<nomifun_terminal::TerminalLifecycleEvent>> {
            None
        }
    }

    fn recording_turn_key(terminal_id: String) -> TerminalTurnAdmissionKey {
        TerminalTurnAdmissionKey {
            terminal_id,
            pty_epoch: 1,
            requirement_id: nomifun_common::RequirementId::new().into_string(),
            claim_generation: 1,
            claim_token:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
            turn_token: "test-turn-token".to_owned(),
        }
    }

    #[tokio::test]
    async fn admitted_terminal_prompt_writes_paste_then_a_separate_exact_epoch_cr() {
        // The two PTY writes must be ordered: bracketed-paste body FIRST, then the
        // lone CR as its OWN write (so paste-burst-detecting TUIs treat it as a
        // real Enter). Mirrors the fix the cron terminal executor already applies.
        let recorder = Arc::new(RecordingDriver::default());
        let driver: Arc<dyn TerminalDriver> = recorder.clone();
        let terminal_id = nomifun_common::TerminalId::new().into_string();
        let key = recording_turn_key(terminal_id);
        let effects = submit_admitted_terminal_prompt(&driver, &key, "do the thing\nthen stop")
            .await
            .expect("submit must succeed");
        assert_eq!(effects, TerminalTurnEffectsStart::Started);
        assert_eq!(
            *recorder.writes.lock().unwrap(),
            [b"\x1b[200~do the thing\nthen stop\x1b[201~".to_vec(), b"\r".to_vec()],
            "the complete paste body and submit CR must be separate ordered writes"
        );
    }

    #[tokio::test]
    async fn replayed_admission_emits_zero_paste_and_zero_cr_bytes() {
        for replay in [
            TerminalTurnEffectsStart::AlreadyStarted,
            TerminalTurnEffectsStart::AlreadySettled,
        ] {
            let recorder = Arc::new(RecordingDriver::default());
            *recorder.body_start.lock().unwrap() = replay;
            let driver: Arc<dyn TerminalDriver> = recorder.clone();
            let key = recording_turn_key(nomifun_common::TerminalId::new().into_string());

            assert_eq!(
                submit_admitted_terminal_prompt(&driver, &key, "must not run\nagain")
                    .await
                    .unwrap(),
                replay
            );
            assert!(
                recorder.writes.lock().unwrap().is_empty(),
                "{replay:?} must emit neither paste nor CR"
            );
        }
    }

    #[tokio::test]
    async fn authority_loss_after_body_write_emits_zero_submit_cr() {
        let recorder = Arc::new(RecordingDriver::default());
        *recorder.submit_start.lock().unwrap() =
            TerminalTurnEffectsStart::AlreadySettled;
        let driver: Arc<dyn TerminalDriver> = recorder.clone();
        let key = recording_turn_key(nomifun_common::TerminalId::new().into_string());

        assert_eq!(
            submit_admitted_terminal_prompt(&driver, &key, "body first\nsubmit fenced")
                .await
                .unwrap(),
            TerminalTurnEffectsStart::AlreadySettled
        );
        let writes = recorder.writes.lock().unwrap();
        assert_eq!(writes.len(), 1, "only the inert paste body may be written");
        assert!(
            !writes[0].contains(&b'\r'),
            "lost exact Requirement/receipt authority must suppress the submit CR"
        );
    }

    #[tokio::test]
    async fn concurrent_restarts_share_one_target_transition_barrier() {
        let transitions = Arc::new(TargetTransitionMap::new());
        let key = (
            AutoWorkTargetKind::Conversation,
            ConversationId::new().into_string(),
        );
        let first_lock = target_transition_lock(&transitions, &key);
        let first_guard = first_lock.lock().await;

        let entered = Arc::new(AtomicBool::new(false));
        let entered_by_second = entered.clone();
        let second_lock = target_transition_lock(&transitions, &key);
        let second = tokio::spawn(async move {
            let _guard = second_lock.lock().await;
            entered_by_second.store(true, Ordering::SeqCst);
        });

        tokio::task::yield_now().await;
        assert!(
            !entered.load(Ordering::SeqCst),
            "a racing start must not cross the prior stop/cleanup barrier"
        );
        drop(first_guard);
        timeout(Duration::from_secs(1), second)
            .await
            .expect("second transition must proceed after cleanup releases")
            .expect("second transition task must not panic");
        assert!(entered.load(Ordering::SeqCst));
    }

    struct RunnerTestHostLease {
        active: bool,
    }

    struct RunnerTestSessionPort {
        issuer: crate::AutoWorkRuntimeLeaseIssuer,
        owner_id: String,
        session_id: String,
        config: Mutex<AutoWorkConfigSnapshot>,
        revision: std::sync::atomic::AtomicU64,
        cancel_count: std::sync::atomic::AtomicUsize,
        fail_begin: AtomicBool,
        block_cancel: AtomicBool,
        cancel_started: Notify,
        cancel_release: Notify,
    }

    impl RunnerTestSessionPort {
        fn new(owner_id: String, session_id: String) -> Self {
            Self {
                issuer: crate::AutoWorkRuntimeLeaseIssuer::new(),
                owner_id,
                session_id,
                config: Mutex::new(AutoWorkConfigSnapshot {
                    config: AutoWorkConfig::default(),
                    revision: "session:0".to_owned(),
                    operation_id: None,
                }),
                revision: std::sync::atomic::AtomicU64::new(0),
                cancel_count: std::sync::atomic::AtomicUsize::new(0),
                fail_begin: AtomicBool::new(false),
                block_cancel: AtomicBool::new(false),
                cancel_started: Notify::new(),
                cancel_release: Notify::new(),
            }
        }

        fn set_snapshot(&self, config: AutoWorkConfig, revision: u64) {
            self.revision.store(revision, Ordering::SeqCst);
            *self.config.lock().expect("runner test config lock") =
                AutoWorkConfigSnapshot {
                    config,
                    revision: format!("session:{revision}"),
                    operation_id: None,
                };
        }

        fn assert_scope(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
            if owner_id != self.owner_id || session_id != self.session_id {
                return Err(AppError::Forbidden(
                    "runner test Session scope mismatch".to_owned(),
                ));
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl AutoWorkSessionPort for RunnerTestSessionPort {
        async fn prepare_autowork_turn(
            &self,
            owner_id: &str,
            session_id: &str,
            build_lease: &AutoWorkRuntimeBuildLease,
        ) -> Result<crate::AutoWorkSessionPreparation, AppError> {
            self.assert_scope(owner_id, session_id)?;
            build_lease.ensure_scope(owner_id, session_id)?;
            let snapshot = self.config.lock().expect("runner test config lock").clone();
            Ok(crate::AutoWorkSessionPreparation {
                agent_type: nomifun_common::AgentType::Nomi,
                workspace: "C:\\runner-test".to_owned(),
                snapshot: self
                    .issuer
                    .issue_snapshot(owner_id, session_id, snapshot.revision)?,
            })
        }

        fn begin_runtime_preparation(
            &self,
            session_id: &str,
            requester_user_id: &str,
        ) -> Result<AutoWorkRuntimeBuildLease, AppError> {
            self.assert_scope(requester_user_id, session_id)?;
            if self.fail_begin.load(Ordering::SeqCst) {
                return Err(AppError::Conflict(
                    "runner test preparation fenced".to_owned(),
                ));
            }
            self.issuer.issue(
                requester_user_id,
                session_id,
                RunnerTestHostLease { active: true },
                |lease| {
                    lease
                        .active
                        .then_some(())
                        .ok_or_else(|| AppError::Conflict("inactive test lease".to_owned()))
                },
            )
        }

        fn user_cancelled_since(&self, _conversation_id: &str, _since_ms: i64) -> bool {
            false
        }

        async fn cancel_active_turn(&self, conversation_id: &str) -> Result<(), AppError> {
            self.assert_scope(&self.owner_id, conversation_id)?;
            self.cancel_count.fetch_add(1, Ordering::SeqCst);
            if self.block_cancel.load(Ordering::SeqCst) {
                self.cancel_started.notify_one();
                self.cancel_release.notified().await;
            }
            Ok(())
        }

        async fn read_config(
            &self,
            owner_id: &str,
            session_id: &str,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            self.assert_scope(owner_id, session_id)?;
            Ok(self.config.lock().expect("runner test config lock").clone())
        }

        async fn save_config(
            &self,
            command: crate::AutoWorkSessionConfigCommand,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            self.assert_scope(&command.owner_id, &command.session_id)?;
            let mut current = self.config.lock().expect("runner test config lock");
            if command.operation_id.is_some()
                && current.operation_id == command.operation_id
            {
                if current.config == command.config {
                    return Ok(current.clone());
                }
                return Err(AppError::Conflict(
                    "runner test operation replay changed payload".to_owned(),
                ));
            }
            if current.revision != command.expected_revision {
                return Err(AppError::Conflict(
                    "runner test config revision changed".to_owned(),
                ));
            }
            let revision = if current.config == command.config {
                current.revision.clone()
            } else {
                let next = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
                format!("session:{next}")
            };
            *current = AutoWorkConfigSnapshot {
                config: command.config,
                revision,
                operation_id: command.operation_id,
            };
            Ok(current.clone())
        }

        async fn send_turn(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _request: AutoWorkTurnRequest,
            _build_lease: AutoWorkRuntimeBuildLease,
            _authority: RequirementConversationTurnAuthority,
        ) -> Result<AutoWorkMessageDelivery, AppError> {
            Err(AppError::Conflict(
                "runner lifecycle test does not send turns".to_owned(),
            ))
        }

        async fn delivery_result(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _request: &AutoWorkMessage,
            _authority: &RequirementConversationTurnAuthority,
        ) -> Result<Option<AutoWorkMessageDelivery>, AppError> {
            Ok(None)
        }

        async fn public_turn_delivery_state(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<AutoWorkTurnDeliveryState, AppError> {
            Ok(AutoWorkTurnDeliveryState::Missing)
        }

        async fn reconcile_quiescent_running_turn(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<AutoWorkReconciliationDisposition, AppError> {
            Ok(AutoWorkReconciliationDisposition::ExternalProofRequiredFailClosed)
        }
    }

    struct BlockingScheduledLookup {
        binding: Mutex<crate::ScheduledAutoWorkSession>,
        listed: Notify,
        release: Notify,
    }

    #[async_trait::async_trait]
    impl crate::AutoWorkScheduledSessionLookup for BlockingScheduledLookup {
        async fn list_enabled_scheduled_sessions(
            &self,
            _owner_id: &str,
        ) -> Result<crate::ScheduledAutoWorkSessionScan, AppError> {
            let binding = self
                .binding
                .lock()
                .expect("blocking scheduled lookup")
                .clone();
            self.listed.notify_one();
            self.release.notified().await;
            Ok(crate::ScheduledAutoWorkSessionScan {
                sessions: vec![binding],
                quarantined: Vec::new(),
            })
        }
    }

    async fn runner_fixture(
        lookup: Option<Arc<dyn crate::AutoWorkScheduledSessionLookup>>,
    ) -> (
        AutoWorkRunner,
        Arc<RunnerTestSessionPort>,
        nomifun_db::Database,
        String,
        String,
    ) {
        use nomifun_db::{
            IAgentMetadataRepository, IConversationRepository, IRequirementRepository,
            SqliteAgentMetadataRepository, SqliteConversationRepository,
            SqliteRequirementRepository, init_database_memory,
        };
        use nomifun_realtime::UserEventSink;

        #[derive(Default)]
        struct RunnerNoopBroadcaster;
        impl UserEventSink for RunnerNoopBroadcaster {
            fn send_to_user(
                &self,
                _user_id: &str,
                _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
            ) {
            }
        }

        let database = init_database_memory().await.expect("runner test database");
        let owner_id = nomifun_db::installation_owner_id(database.pool())
            .await
            .expect("installation owner");
        let session_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations \
                 (conversation_id, user_id, name, type, extra, created_at, updated_at) \
             VALUES (?1, ?2, 'AutoWork Runner Test', 'nomi', '{}', 0, 0)",
        )
        .bind(&session_id)
        .bind(&owner_id)
        .execute(database.pool())
        .await
        .expect("runner test Session");

        let session_port = Arc::new(RunnerTestSessionPort::new(
            owner_id.clone(),
            session_id.clone(),
        ));
        let conversation_repo: Arc<dyn IConversationRepository> = Arc::new(
            SqliteConversationRepository::new(database.pool().clone()),
        );
        let requirement_repo: Arc<dyn IRequirementRepository> = Arc::new(
            SqliteRequirementRepository::new(database.pool().clone()),
        );
        let mut service = RequirementService::new(
            requirement_repo,
            crate::RequirementEventEmitter::new(
                Arc::new(RunnerNoopBroadcaster),
                Arc::from(owner_id.as_str()),
            ),
        )
        .with_session_port(session_port.clone(), conversation_repo);
        if let Some(lookup) = lookup {
            service = service.with_scheduled_session_lookup(lookup);
        }
        let agent_repo: Arc<dyn IAgentMetadataRepository> = Arc::new(
            SqliteAgentMetadataRepository::new(database.pool().clone()),
        );
        let runner = AutoWorkRunner::new(Arc::new(AutoWorkRunnerDeps {
            authoritative_user_id: Arc::from(owner_id.as_str()),
            service: Arc::new(service),
            conversation: session_port.clone(),
            agent_registry: AgentRegistry::new(agent_repo),
            terminal_driver: None,
            idmm: None,
            wake: Arc::new(Notify::new()),
            requirement_mcp_enabled: false,
        }));
        (runner, session_port, database, owner_id, session_id)
    }

    #[tokio::test]
    async fn identical_enable_is_idempotent_and_keeps_the_same_loop_generation() {
        let (runner, port, _database, owner_id, session_id) =
            runner_fixture(None).await;
        let config = AutoWorkConfig::normalize(true, Some("release"), Some(4)).unwrap();
        let first = runner
            .apply_config(
                &owner_id,
                AutoWorkTargetKind::Conversation,
                &session_id,
                config.clone(),
                None,
                Some("test:enable:1"),
            )
            .await
            .expect("first enable");
        let key = (AutoWorkTargetKind::Conversation, session_id.clone());
        let first_generation = runner
            .handles
            .get(&key)
            .expect("running handle")
            .generation;

        let replay = runner
            .apply_config(
                &owner_id,
                AutoWorkTargetKind::Conversation,
                &session_id,
                config,
                Some(&first.revision),
                Some("test:enable:2"),
            )
            .await
            .expect("identical enable");
        let second_generation = runner
            .handles
            .get(&key)
            .expect("same running handle")
            .generation;

        assert!(runner.is_running(AutoWorkTargetKind::Conversation, &session_id));
        assert!(!runner.is_running(AutoWorkTargetKind::Terminal, &session_id));
        assert_eq!(
            runner.running_tag(AutoWorkTargetKind::Conversation, &session_id).as_deref(),
            Some("release")
        );
        assert_eq!(runner.running_tag(AutoWorkTargetKind::Terminal, &session_id), None);
        assert!(runner.live_progress(AutoWorkTargetKind::Conversation, &session_id).is_some());
        assert_eq!(runner.live_progress(AutoWorkTargetKind::Terminal, &session_id), None);
        assert_eq!(first_generation, second_generation);
        assert_eq!(first.revision, replay.revision);
        assert_eq!(port.cancel_count.load(Ordering::SeqCst), 0);
        runner.shutdown().await.expect("runner shutdown");
    }

    #[tokio::test]
    async fn failed_start_after_publication_does_not_leave_a_ghost_handle() {
        let (runner, port, _database, owner_id, session_id) =
            runner_fixture(None).await;
        port.fail_begin.store(true, Ordering::SeqCst);
        let config = AutoWorkConfig::normalize(true, Some("ghost"), None).unwrap();
        runner
            .apply_config(
                &owner_id,
                AutoWorkTargetKind::Conversation,
                &session_id,
                config,
                None,
                Some("test:ghost"),
            )
            .await
            .expect("config may persist before startup preflight");

        timeout(Duration::from_secs(1), async {
            while runner.is_running(AutoWorkTargetKind::Conversation, &session_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed loop must remove its published handle");
        assert!(runner.handles.is_empty());
        runner.shutdown().await.expect("runner shutdown");
    }

    #[tokio::test]
    async fn boot_resume_rechecks_revision_and_does_not_revive_a_disabled_snapshot() {
        let enabled = AutoWorkConfig::normalize(true, Some("boot"), Some(2)).unwrap();
        let lookup = Arc::new(BlockingScheduledLookup {
            binding: Mutex::new(crate::ScheduledAutoWorkSession {
                session_id: ConversationId::new().into_string(),
                display_name: "Boot".to_owned(),
                tag: "boot".to_owned(),
                max_requirements: Some(2),
                config_revision: "session:1".to_owned(),
            }),
            listed: Notify::new(),
            release: Notify::new(),
        });
        let (runner, port, _database, _owner_id, session_id) =
            runner_fixture(Some(lookup.clone())).await;
        lookup
            .binding
            .lock()
            .expect("blocking scheduled lookup")
            .session_id = session_id.clone();
        port.set_snapshot(enabled, 1);
        runner.resume_persisted_bindings();
        lookup.listed.notified().await;
        port.set_snapshot(AutoWorkConfig::default(), 2);
        lookup.release.notify_one();

        timeout(Duration::from_secs(1), async {
            loop {
                let finished = runner
                    .coordinators
                    .lock()
                    .expect("coordinator lock")
                    .resume
                    .as_ref()
                    .is_some_and(JoinHandle::is_finished);
                if finished {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("boot resume coordinator");
        assert!(!runner.is_running(AutoWorkTargetKind::Conversation, &session_id));
        runner.shutdown().await.expect("runner shutdown");
    }

    #[tokio::test]
    async fn bounded_stop_retains_cleanup_and_shutdown_rejects_waiting_restart() {
        let (runner, port, _database, owner_id, session_id) =
            runner_fixture(None).await;
        let config = AutoWorkConfig::normalize(true, Some("cleanup"), None).unwrap();
        runner
            .apply_config(
                &owner_id,
                AutoWorkTargetKind::Conversation,
                &session_id,
                config,
                None,
                Some("test:cleanup"),
            )
            .await
            .expect("enable");
        port.block_cancel.store(true, Ordering::SeqCst);
        // Keep database fixture setup on a normal clock. Freeze only after the
        // target is running so SQLx pool acquisition cannot be starved by a
        // paused runtime clock.
        tokio::time::pause();

        let runner_for_stop = runner.clone();
        let session_for_stop = session_id.clone();
        let stop = tokio::spawn(async move {
            runner_for_stop
                .stop(AutoWorkTargetKind::Conversation, &session_for_stop)
                .await
        });
        port.cancel_started.notified().await;
        tokio::time::advance(STOP_WAIT_TIMEOUT + Duration::from_millis(1)).await;
        let outcome = stop.await.expect("stop task");
        assert_eq!(outcome, AutoWorkStopOutcome::CleanupPending);
        let key = (AutoWorkTargetKind::Conversation, session_id.clone());
        assert!(
            runner.cleanups.contains_key(&key),
            "the exact cleanup owner must survive the bounded waiter"
        );

        let restart = runner.start(
            AutoWorkTargetKind::Conversation,
            session_id.clone(),
            "cleanup".to_owned(),
            None,
        );
        tokio::pin!(restart);
        tokio::select! {
            biased;
            result = &mut restart => panic!("restart must wait for cleanup: {result:?}"),
            _ = std::future::ready(()) => {}
        }
        let shutdown = runner.shutdown();
        tokio::pin!(shutdown);
        tokio::select! {
            biased;
            result = &mut shutdown => panic!("shutdown must wait for cleanup: {result:?}"),
            _ = std::future::ready(()) => {}
        }
        port.cancel_release.notify_one();
        timeout(Duration::from_secs(1), async {
            while runner.cleanups.contains_key(&key) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retained cleanup must finish");
        assert_eq!(restart.await, AutoWorkStartOutcome::ShuttingDown);
        shutdown.await.expect("runner shutdown");
        assert!(runner.handles.is_empty());
    }

    #[test]
    fn autowork_delivery_key_is_stable_per_exact_durable_claim_capability() {
        let requirement_id = nomifun_common::RequirementId::new().into_string();
        let claim_token =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let replacement_token =
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let first = autowork_turn_idempotency_key(&requirement_id, 7, claim_token);
        let restart_replay =
            autowork_turn_idempotency_key(&requirement_id, 7, claim_token);
        let next_generation =
            autowork_turn_idempotency_key(&requirement_id, 8, claim_token);
        let replacement_capability =
            autowork_turn_idempotency_key(&requirement_id, 7, replacement_token);

        assert_eq!(
            first, restart_replay,
            "restarting the runner must address the same receiver receipt"
        );
        assert_ne!(
            first, next_generation,
            "only a newly persisted claim generation may address a new turn"
        );
        assert_ne!(
            first, replacement_capability,
            "a different capability must never address the prior receipt as fresh authority"
        );
        assert!(first.starts_with("autowork:v2:"));
        assert!(first.len() <= 128);
        assert!(!first.contains(&requirement_id));
        assert!(!first.contains(claim_token));
    }

    #[test]
    fn unreconciled_accepted_replay_is_absorbed_instead_of_starting_a_second_turn() {
        let outcome = autowork_replayed_delivery_outcome(
            &IdempotentMessageDelivery {
                message_id: nomifun_common::MessageId::new().into_string(),
                replayed: true,
                completed: false,
                result_ok: None,
                result_text: None,
                result_error: None,
                result_error_code: None,
                result_error_retryable: None,
            },
            3,
            false,
        )
        .expect("accepted replay must terminate injection before the event wait");

        assert_eq!(outcome.0, TurnEnd::Ambiguous);
        assert!(
            outcome.1.as_deref().is_some_and(|note| {
                note.contains("not executed again") && note.contains("generation 3")
            }),
            "the review note must explain the absorbing at-most-once decision"
        );
        assert!(
            outcome.2,
            "forcing the verdict contract parks an unknown accepted replay in needs_review"
        );
    }

    #[test]
    fn accepted_replay_wait_requires_an_exact_local_or_reconciled_owner() {
        let message_id = nomifun_common::MessageId::new().into_string();

        for allowed in [
            BackgroundTurnReconciliationDisposition::LiveExactOwnerWait,
            BackgroundTurnReconciliationDisposition::ReconciledOrTerminalReRead,
        ] {
            authorize_accepted_receipt_wait(allowed, &message_id)
                .expect("an exact live owner or audited local reconciliation may poll its receipt");
        }

        for blocked in [
            BackgroundTurnReconciliationDisposition::ExternalProofRequiredFailClosed,
            BackgroundTurnReconciliationDisposition::StaleConflict,
        ] {
            assert!(
                matches!(
                    authorize_accepted_receipt_wait(blocked, &message_id),
                    Err(AppError::Conflict(_))
                ),
                "external/unknown ownership and stale generations must fail closed"
            );
        }
    }

    #[test]
    fn completed_error_replay_is_never_promoted_to_a_new_claim_generation() {
        let outcome = autowork_replayed_delivery_outcome(
            &IdempotentMessageDelivery {
                message_id: nomifun_common::MessageId::new().into_string(),
                replayed: true,
                completed: true,
                result_ok: Some(false),
                result_text: None,
                result_error: Some("provider stream ended after tools may have run".to_owned()),
                result_error_code: None,
                result_error_retryable: None,
            },
            9,
            false,
        )
        .expect("a completed receipt must be absorbing");

        assert_eq!(
            outcome.0,
            TurnEnd::Ambiguous,
            "post-admission errors must bypass the RetryPending branch"
        );
        assert!(
            outcome.2,
            "the exact finalizer must park the generation in needs_review"
        );
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note.contains("tools may have run"))
        );
    }

    #[test]
    fn only_the_receipt_leader_enters_the_live_event_wait() {
        let fresh = autowork_replayed_delivery_outcome(
            &IdempotentMessageDelivery {
                message_id: nomifun_common::MessageId::new().into_string(),
                replayed: false,
                completed: false,
                result_ok: None,
                result_text: None,
                result_error: None,
                result_error_code: None,
                result_error_retryable: None,
            },
            4,
            true,
        );
        assert!(
            fresh.is_none(),
            "only the atomic receiver-side receipt leader may await a newly started turn"
        );
    }

    #[test]
    fn durable_preflight_conflict_is_parked_instead_of_minting_a_retry_turn() {
        let outcome =
            autowork_blocked_delivery_outcome("idempotency payload drift".to_owned());
        assert_eq!(outcome.0, TurnEnd::Ambiguous);
        assert!(outcome.2, "blocked delivery must finalize as needs_review");
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note.contains("Explicit reset or human review")),
            "the operator must get a recoverable explanation"
        );
    }

    #[test]
    fn recovered_claim_without_receipt_is_fail_closed_for_every_generation() {
        let outcome = autowork_blocked_delivery_outcome(
            recovered_claim_receipt_gate(&PublicTurnDeliveryState::Missing, 1)
                .expect_err("a recovered claim without a receipt must be quarantined"),
        );
        assert_eq!(outcome.0, TurnEnd::Ambiguous);
        assert!(outcome.2, "missing receipt must finalize as needs_review");
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note.contains("prior execution outcome cannot be proven"))
        );
        assert!(
            recovered_claim_receipt_gate(
                &PublicTurnDeliveryState::Missing,
                17
            )
            .is_err(),
            "the guard must not allow a later generation to bypass the receipt boundary"
        );
    }

    #[test]
    fn recovered_claim_with_accepted_or_completed_receipt_may_reconcile_only() {
        let message_id = nomifun_common::MessageId::new().into_string();
        assert!(
            recovered_claim_receipt_gate(
                &PublicTurnDeliveryState::Accepted {
                    message_id: message_id.clone(),
                },
                2,
            )
            .is_ok()
        );
        assert!(
            recovered_claim_receipt_gate(
                &PublicTurnDeliveryState::Completed(IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: true,
                    result_ok: Some(true),
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                }),
                2,
            )
            .is_ok()
        );
    }

    struct ScriptedConversationPort {
        state: Mutex<PublicTurnDeliveryState>,
        reconciliation: BackgroundTurnReconciliationDisposition,
        reconciliation_delay: Option<Duration>,
        polls: std::sync::atomic::AtomicUsize,
    }

    impl ScriptedConversationPort {
        fn new(
            state: PublicTurnDeliveryState,
            reconciliation: BackgroundTurnReconciliationDisposition,
        ) -> Self {
            Self {
                state: Mutex::new(state),
                reconciliation,
                reconciliation_delay: None,
                polls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn with_reconciliation_delay(
            state: PublicTurnDeliveryState,
            reconciliation: BackgroundTurnReconciliationDisposition,
            delay: Duration,
        ) -> Self {
            let mut port = Self::new(state, reconciliation);
            port.reconciliation_delay = Some(delay);
            port
        }

        fn set_state(&self, state: PublicTurnDeliveryState) {
            *self.state.lock().expect("scripted receipt state lock") = state;
        }

        fn poll_count(&self) -> usize {
            self.polls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl AutoWorkSessionPort for ScriptedConversationPort {
        async fn prepare_autowork_turn(
            &self,
            _owner_id: &str,
            _session_id: &str,
            _build_lease: &AutoWorkRuntimeBuildLease,
        ) -> Result<crate::conversation_port::AutoWorkSessionPreparation, AppError> {
            Err(AppError::Conflict("not used by receipt wait test".into()))
        }

        fn begin_runtime_preparation(
            &self,
            _conversation_id: &str,
            _requester_user_id: &str,
        ) -> Result<RuntimeBuildLease, AppError> {
            Err(AppError::Conflict("not used by receipt wait test".into()))
        }

        fn user_cancelled_since(&self, _conversation_id: &str, _since_ms: i64) -> bool {
            false
        }

        async fn cancel_active_turn(&self, _conversation_id: &str) -> Result<(), AppError> {
            Ok(())
        }

        async fn read_config(
            &self,
            _owner_id: &str,
            _session_id: &str,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            Err(AppError::Conflict(
                "not used by receipt wait test".into(),
            ))
        }

        async fn save_config(
            &self,
            _command: crate::AutoWorkSessionConfigCommand,
        ) -> Result<AutoWorkConfigSnapshot, AppError> {
            Err(AppError::Conflict(
                "not used by receipt wait test".into(),
            ))
        }

        async fn send_turn(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _request: AutoWorkTurnRequest,
            _build_lease: RuntimeBuildLease,
            _authority: RequirementConversationTurnAuthority,
        ) -> Result<AutoWorkMessageDelivery, AppError> {
            Err(AppError::Conflict("not used by receipt wait test".into()))
        }

        async fn delivery_result(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _request: &AutoWorkMessage,
            _authority: &RequirementConversationTurnAuthority,
        ) -> Result<Option<IdempotentMessageDelivery>, AppError> {
            Err(AppError::Conflict("receipt wait must use typed state".into()))
        }

        async fn public_turn_delivery_state(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<PublicTurnDeliveryState, AppError> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Ok(self.state.lock().expect("scripted receipt state lock").clone())
        }

        async fn reconcile_quiescent_running_turn(
            &self,
            _user_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<BackgroundTurnReconciliationDisposition, AppError> {
            if let Some(delay) = self.reconciliation_delay {
                sleep(delay).await;
            }
            Ok(self.reconciliation)
        }
    }

    async fn receipt_wait_fixture() -> (
        RequirementService,
        nomifun_db::Database,
        String,
        String,
        i64,
        String,
        String,
    ) {
        use nomifun_db::{IRequirementRepository, SqliteRequirementRepository, init_database_memory};
        use nomifun_realtime::UserEventSink;

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

        let db = init_database_memory().await.expect("in-memory database");
        let user_id = nomifun_db::installation_owner_id(db.pool())
            .await
            .expect("installation owner");
        let conversation_id = ConversationId::new().into_string();
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, created_at, updated_at) \
             VALUES (?1, ?2, 'Receipt Wait Test', 'nomi', 0, 0)",
        )
        .bind(&conversation_id)
        .bind(&user_id)
        .execute(db.pool())
        .await
        .expect("conversation fixture");

        let repo: Arc<dyn IRequirementRepository> =
            Arc::new(SqliteRequirementRepository::new(db.pool().clone()));
        let emitter = crate::events::RequirementEventEmitter::new(
            Arc::new(NoopBroadcaster),
            Arc::from(user_id.as_str()),
        );
        let service = RequirementService::new(repo, emitter);
        let requirement = service
            .create(nomifun_api_types::CreateRequirementRequest {
                title: "Receipt wait".into(),
                content: "wait for durable completion".into(),
                tag: "receipt-wait".into(),
                order_key: Some("1".into()),
                status: None,
                created_by: None,
                attachments: vec![],
            })
            .await
            .expect("requirement fixture");
        let claim = service
            .claim_next_for_runner(
                "receipt-wait",
                &conversation_id,
                AutoWorkTargetKind::Conversation,
                DEFAULT_LEASE_MS,
            )
            .await
            .expect("claim fixture")
            .expect("requirement claim");

        (
            service,
            db,
            conversation_id,
            requirement.requirement_id,
            claim.claim_generation,
            claim.claim_token,
            user_id,
        )
    }

    #[tokio::test]
    async fn exact_verdict_none_requires_matching_terminal_reconfirmation() {
        let (service, _db, conversation_id, requirement_id, generation, claim_token, _user_id) =
            receipt_wait_fixture().await;

        resolve_claim_verdict_required(
            &service,
            &requirement_id,
            generation,
            &claim_token,
            &conversation_id,
            AutoWorkTargetKind::Conversation,
            RequirementStatus::NeedsReview,
            Some("first terminal writer".to_owned()),
        )
        .await
        .expect("the first exact verdict should commit");

        let stale_done = resolve_claim_verdict_required(
            &service,
            &requirement_id,
            generation,
            &claim_token,
            &conversation_id,
            AutoWorkTargetKind::Conversation,
            RequirementStatus::Done,
            None,
        )
        .await;
        let error = stale_done.expect_err("Ok(None) must not be treated as a Done success");
        assert!(
            error
                .to_string()
                .contains("already resolved as 'needs_review' instead of 'done'"),
            "the exact terminal recheck must explain the conflicting winner: {error}"
        );

        resolve_claim_verdict_required(
            &service,
            &requirement_id,
            generation,
            &claim_token,
            &conversation_id,
            AutoWorkTargetKind::Conversation,
            RequirementStatus::NeedsReview,
            None,
        )
        .await
        .expect("a same-status exact replay may be acknowledged after re-confirmation");
    }

    fn receipt_wait_timing(hard_timeout: Duration) -> ConversationReceiptWaitTiming {
        ConversationReceiptWaitTiming {
            lease_renew_interval: Duration::from_secs(60),
            receipt_poll_interval: Duration::from_millis(5),
            hard_timeout,
        }
    }

    #[tokio::test]
    async fn conversation_wait_uses_durable_receipt_text_as_the_note() {
        let (service, _db, conversation_id, requirement_id, generation, claim_token, user_id) =
            receipt_wait_fixture().await;
        let message_id = nomifun_common::MessageId::new().into_string();
        let port = Arc::new(ScriptedConversationPort::new(
            PublicTurnDeliveryState::Accepted {
                message_id: message_id.clone(),
            },
            BackgroundTurnReconciliationDisposition::LiveExactOwnerWait,
        ));
        let wait = wait_for_conversation_receipt_with_renewal(
            &service,
            port.as_ref(),
            &conversation_id,
            &requirement_id,
            generation,
            &claim_token,
            &user_id,
            "autowork:test-receipt-wait",
            &message_id,
            false,
            receipt_wait_timing(Duration::from_secs(1)),
        );
        tokio::pin!(wait);

        let early = tokio::select! {
            outcome = &mut wait => Some(outcome),
            _ = sleep(Duration::from_millis(30)) => None,
        };
        assert!(
            early.is_none(),
            "an accepted receipt must not complete before durable settlement"
        );
        assert!(port.poll_count() > 0, "the typed receipt must be polled");

        port.set_state(PublicTurnDeliveryState::Completed(
            IdempotentMessageDelivery {
                message_id: message_id.clone(),
                replayed: true,
                completed: true,
                result_ok: Some(true),
                result_text: Some("durable completion note".to_owned()),
                result_error: None,
                result_error_code: None,
                result_error_retryable: None,
            },
        ));
        let outcome = timeout(Duration::from_secs(1), &mut wait)
            .await
            .expect("durable completion should be observed");
        assert_eq!(outcome.0, TurnEnd::Clean);
        assert!(!outcome.2, "durable success without native verdict is clean");
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note == "durable completion note"),
            "the completed receipt text is the authoritative note"
        );
    }

    #[tokio::test]
    async fn missing_receipt_is_bounded_review_and_never_a_success() {
        let (service, _db, conversation_id, requirement_id, generation, claim_token, user_id) =
            receipt_wait_fixture().await;
        let port = ScriptedConversationPort::new(
            PublicTurnDeliveryState::Missing,
            BackgroundTurnReconciliationDisposition::LiveExactOwnerWait,
        );
        let outcome = wait_for_conversation_receipt_with_renewal(
            &service,
            &port,
            &conversation_id,
            &requirement_id,
            generation,
            &claim_token,
            &user_id,
            "autowork:test-missing-receipt",
            &nomifun_common::MessageId::new().into_string(),
            false,
            receipt_wait_timing(Duration::from_secs(1)),
        )
        .await;
        assert_eq!(outcome.0, TurnEnd::Ambiguous);
        assert!(outcome.2, "missing receipt must require review");
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note.contains("disappeared")),
            "missing receipt must explain the fail-closed decision"
        );
    }

    #[tokio::test]
    async fn accepted_receipt_timeout_is_bounded_and_never_infers_done() {
        let (service, _db, conversation_id, requirement_id, generation, claim_token, user_id) =
            receipt_wait_fixture().await;
        let message_id = nomifun_common::MessageId::new().into_string();
        let port = ScriptedConversationPort::new(
            PublicTurnDeliveryState::Accepted { message_id: message_id.clone() },
            BackgroundTurnReconciliationDisposition::LiveExactOwnerWait,
        );
        let outcome = wait_for_conversation_receipt_with_renewal(
            &service,
            &port,
            &conversation_id,
            &requirement_id,
            generation,
            &claim_token,
            &user_id,
            "autowork:test-timeout",
            &message_id,
            false,
            receipt_wait_timing(Duration::from_millis(35)),
        )
        .await;
        assert_eq!(outcome.0, TurnEnd::Ambiguous);
        assert!(outcome.2, "timeout must require review");
        assert!(
            outcome
                .1
                .as_deref()
                .is_some_and(|note| note.contains("hard timeout")),
            "timeout must be reported as an unknown outcome"
        );
    }

    #[tokio::test]
    async fn accepted_replay_reconciliation_uses_typed_state_and_message_identity() {
        let message_id = nomifun_common::MessageId::new().into_string();
        let port = ScriptedConversationPort::new(
            PublicTurnDeliveryState::Accepted {
                message_id: message_id.clone(),
            },
            BackgroundTurnReconciliationDisposition::ReconciledOrTerminalReRead,
        );
        let delivery = IdempotentMessageDelivery {
            message_id: message_id.clone(),
            replayed: true,
            completed: false,
            result_ok: None,
            result_text: None,
            result_error: None,
            result_error_code: None,
            result_error_retryable: None,
        };
        let reconciled = reconcile_accepted_autowork_delivery(
            &port,
            "user",
            "conversation",
            "autowork:key",
            delivery,
            CONVERSATION_RECEIPT_RECONCILIATION_TIMEOUT,
        )
        .await
        .expect("accepted typed state should remain waitable");
        assert!(reconciled.accepted_wait_authorized);

        port.set_state(PublicTurnDeliveryState::Completed(
            IdempotentMessageDelivery {
                message_id,
                replayed: true,
                completed: true,
                result_ok: Some(true),
                result_text: None,
                result_error: None,
                result_error_code: None,
                result_error_retryable: None,
            },
        ));
        let completed = reconcile_accepted_autowork_delivery(
            &port,
            "user",
            "conversation",
            "autowork:key",
            reconciled.delivery,
            CONVERSATION_RECEIPT_RECONCILIATION_TIMEOUT,
        )
        .await
        .expect("completed typed state should be returned");
        assert!(!completed.accepted_wait_authorized);
        assert!(completed.delivery.completed);
    }

    #[tokio::test]
    async fn accepted_reconciliation_timeout_is_bounded_before_receipt_wait() {
        let message_id = nomifun_common::MessageId::new().into_string();
        let port = ScriptedConversationPort::with_reconciliation_delay(
            PublicTurnDeliveryState::Accepted {
                message_id: message_id.clone(),
            },
            BackgroundTurnReconciliationDisposition::LiveExactOwnerWait,
            Duration::from_millis(100),
        );
        let result = timeout(
            Duration::from_secs(1),
            reconcile_accepted_autowork_delivery(
                &port,
                "user",
                "conversation",
                "autowork:key",
                IdempotentMessageDelivery {
                    message_id,
                    replayed: true,
                    completed: false,
                    result_ok: None,
                    result_text: None,
                    result_error: None,
                    result_error_code: None,
                    result_error_retryable: None,
                },
                Duration::from_millis(10),
            ),
        )
        .await
        .expect("reconciliation must return before the outer guard");
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("a reconciliation past its deadline must fail closed"),
        };
        assert!(
            error
                .to_string()
                .contains("reconciliation exceeded its 10 ms deadline"),
            "deadline error must identify the bounded reconciliation phase: {error}"
        );
    }

    // -- Terminal turn-end classification tests ------------------------------
    //
    // Any uncertainty after submission is absorbing; only an exact structured
    // completion may be clean.

    #[test]
    fn only_terminal_requirement_statuses_map_to_terminal_outcomes() {
        for (status, outcome) in [
            (RequirementStatus::Pending, None),
            (RequirementStatus::InProgress, None),
            (RequirementStatus::Done, Some(TerminalTurnOutcome::Done)),
            (RequirementStatus::Failed, Some(TerminalTurnOutcome::Failed)),
            (RequirementStatus::NeedsReview, Some(TerminalTurnOutcome::NeedsReview)),
            (RequirementStatus::Cancelled, Some(TerminalTurnOutcome::Cancelled)),
        ] {
            assert_eq!(terminal_outcome_from_status(status), outcome);
        }
    }
}
