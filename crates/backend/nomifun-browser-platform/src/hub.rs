use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, Notify, OnceCell, RwLock, broadcast};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::identity::IdentityGenerationCoordinator;
use crate::{
    Admission, BrowserCapacitySnapshot, BrowserErrorCode, BrowserHostDriver,
    BrowserHostFactory, BrowserHostId, BrowserHostSnapshot, BrowserIdentityMode,
    BrowserInventoryEvent, BrowserLaneDriver, BrowserLaneId, BrowserLaneScheduler,
    BrowserLaneSnapshot, BrowserOperation, BrowserOperationKind, BrowserOperationResult,
    BrowserOverview, BrowserPlatformError, CallerIdentity, CanonicalIdentitySnapshot,
    BrowserVisibility, CapturedIdentitySnapshot, Clock, CloseResult,
    DriverOperationContext, HostLaunchRequest, IdentitySnapshotPayload,
    HostCircuitBreaker, HostRestartTransition, LaneFreezeOutcome, LaneKey, LaneLaunchRequest,
    LaneLifecycleState, LanePriority, OperationContext, OwnerLease, OwnerLeaseId,
    OwnerLeaseService,
    PerKeyHostRestartSingleFlight, PromotionPolicy, ResourceDecision, ResourcePolicy,
    ResourcePressureState, ResourceTelemetry, ResourceWorkload, SchedulerConfig, SnapshotCoverage,
    SystemClock, stale_browser_epoch_error,
};

const EVENT_BUFFER: usize = 256;
const LANE_CLEANUP_WAITER_TIMEOUT: Duration = Duration::from_secs(6);
const CLEANUP_BATCH_WAIT_TIMEOUT: Duration = Duration::from_secs(7);
// On Windows the engine may legitimately spend up to 30 seconds waiting for
// DevToolsActivePort, followed by its first bounded CDP initialization command.
// The platform must not cancel that engine-owned cold start first. A caller
// waiting on the initialization gate needs the same budget because it is
// joining that exact in-flight Host launch.
const HOST_INITIALIZATION_GATE_TIMEOUT: Duration = Duration::from_secs(65);
const HOST_INITIALIZATION_LAUNCH_TIMEOUT: Duration = Duration::from_secs(65);
// Host replacement first shuts down the old process, then performs the same
// bounded cold start and rebinds its Lanes.
const HOST_RESTART_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(75);
const HOST_SHUTDOWN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_RETIREMENT_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_FINALIZATION_WAITER_TIMEOUT: Duration = Duration::from_secs(7);
const PENDING_LANE_START_WAIT_TIMEOUT: Duration = Duration::from_secs(6);
const PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubConfig {
    pub resource_policy: ResourcePolicy,
    pub owner_lease_ttl_ms: u64,
    pub headful: bool,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            resource_policy: ResourcePolicy::default(),
            owner_lease_ttl_ms: 5 * 60_000,
            headful: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenLaneOutcome {
    Running { lane: BrowserLaneSnapshot },
    Queued { lane: BrowserLaneSnapshot },
}

impl OpenLaneOutcome {
    pub fn lane(&self) -> &BrowserLaneSnapshot {
        match self {
            Self::Running { lane } | Self::Queued { lane } => lane,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct HostKey {
    identity_mode: BrowserIdentityMode,
    identity_generation: u64,
    isolation_lane_id: Option<BrowserLaneId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OwnerCleanupTarget {
    user_id: String,
    host_key: HostKey,
    browser_epoch: u64,
}

impl HostKey {
    fn for_lane(
        identity_mode: BrowserIdentityMode,
        identity_generation: u64,
        lane_id: &BrowserLaneId,
    ) -> Self {
        Self {
            identity_mode,
            // Only a replica snapshot has a meaningful version. Primary is
            // one canonical live profile, and anonymous hosts are unversioned.
            identity_generation: if identity_mode == BrowserIdentityMode::AuthenticatedReplica {
                identity_generation
            } else {
                0
            },
            // Explicit isolation must never collapse two lanes into one
            // profile/Host merely because callers used generation zero.
            isolation_lane_id: (identity_mode == BrowserIdentityMode::Isolated)
                .then(|| lane_id.clone()),
        }
    }
}

#[derive(Clone, Copy)]
enum PressureCloseFilter {
    AnyIdle,
    FrozenExpansion,
    RunningExpansion,
    IdleCrawl,
}

const fn is_crawl_identity(identity_mode: BrowserIdentityMode) -> bool {
    matches!(
        identity_mode,
        BrowserIdentityMode::Anonymous | BrowserIdentityMode::AuthenticatedReplica
    )
}

struct HostSlot {
    driver: OnceCell<Arc<dyn BrowserHostDriver>>,
    initialization_gate: Mutex<()>,
    shutdown_gate: Mutex<()>,
    shutdown_complete: AtomicBool,
    retired: AtomicBool,
    headful: AtomicBool,
    epoch: u64,
}

impl HostSlot {
    fn new(epoch: u64, headful: bool) -> Self {
        Self {
            driver: OnceCell::new(),
            initialization_gate: Mutex::new(()),
            shutdown_gate: Mutex::new(()),
            shutdown_complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            headful: AtomicBool::new(headful),
            epoch,
        }
    }

    fn is_headful(&self) -> bool {
        self.headful.load(Ordering::Acquire)
    }

    fn get(&self) -> Option<&Arc<dyn BrowserHostDriver>> {
        self.driver.get()
    }

    async fn get_or_try_init<F, Fut>(
        &self,
        init: F,
    ) -> Result<&Arc<dyn BrowserHostDriver>, BrowserPlatformError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError>>,
    {
        let _initialization_guard = tokio::time::timeout(
            HOST_INITIALIZATION_GATE_TIMEOUT,
            self.initialization_gate.lock(),
        )
        .await
        .map_err(|_| {
            host_initialization_timeout_error(
                self.epoch,
                "gate",
                HOST_INITIALIZATION_GATE_TIMEOUT,
            )
        })?;
        if self.retired.load(Ordering::Acquire) {
            return Err(host_slot_retired_error());
        }
        // Gate contention must not consume the factory's own cold-start
        // budget. This matters after a failed initializer: the next caller is
        // allowed one complete, independently bounded launch attempt.
        let host = tokio::time::timeout(
            HOST_INITIALIZATION_LAUNCH_TIMEOUT,
            self.driver.get_or_try_init(init),
        )
        .await
        .map_err(|_| {
            host_initialization_timeout_error(
                self.epoch,
                "launch",
                HOST_INITIALIZATION_LAUNCH_TIMEOUT,
            )
        })??;
        // Retirement is published before cleanup waits for the initialization
        // gate. If shutdown/sweep selected this slot while launch was in
        // flight, never hand the late Host back to a Lane; the cleanup waiter
        // will shut it down as soon as this guard is released.
        if self.retired.load(Ordering::Acquire) {
            return Err(host_slot_retired_error());
        }
        Ok(host)
    }

    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    async fn shutdown_retired(&self) -> Result<bool, BrowserPlatformError> {
        self.retire();
        let deadline = Instant::now() + HOST_SHUTDOWN_ATTEMPT_TIMEOUT;
        let _shutdown_guard = tokio::time::timeout_at(deadline, self.shutdown_gate.lock())
            .await
            .map_err(|_| {
                host_cleanup_timeout_error(
                    self.epoch,
                    "shutdown_gate",
                    HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                )
            })?;
        if self.shutdown_complete.load(Ordering::Acquire) {
            return Ok(self.driver.get().is_some());
        }
        let _initialization_guard =
            tokio::time::timeout_at(deadline, self.initialization_gate.lock())
                .await
                .map_err(|_| {
                    host_cleanup_timeout_error(
                        self.epoch,
                        "initialization",
                        HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                    )
                })?;
        let Some(host) = self.driver.get() else {
            self.shutdown_complete.store(true, Ordering::Release);
            return Ok(false);
        };
        tokio::time::timeout_at(deadline, host.shutdown())
            .await
            .map_err(|_| {
                host_cleanup_timeout_error(
                    self.epoch,
                    "driver_shutdown",
                    HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                )
            })??;
        self.shutdown_complete.store(true, Ordering::Release);
        Ok(true)
    }
}

struct HostHandle {
    key: HostKey,
    slot: Arc<HostSlot>,
    driver: Arc<dyn BrowserHostDriver>,
}

struct LaneRecord {
    snapshot: RwLock<BrowserLaneSnapshot>,
    driver: RwLock<Option<Arc<dyn BrowserLaneDriver>>>,
    start_flight: Mutex<Option<Arc<LaneStartFlight>>>,
    start_claimed: AtomicBool,
    operation_gate: Mutex<()>,
    activity_gate: RwLock<()>,
    close_gate: Mutex<()>,
    cancellation: CancellationToken,
    closing: AtomicBool,
    active_operation_count: AtomicUsize,
    fresh_observe_required: AtomicBool,
    restart_from_epoch: AtomicU64,
    priority: LanePriority,
    frozen_at_ms: AtomicU64,
    workspace_hint: Option<String>,
}

impl LaneRecord {
    fn new(
        snapshot: BrowserLaneSnapshot,
        priority: LanePriority,
        workspace_hint: Option<String>,
        start_flight: Option<Arc<LaneStartFlight>>,
        start_claimed: bool,
    ) -> Self {
        Self {
            snapshot: RwLock::new(snapshot),
            driver: RwLock::new(None),
            start_flight: Mutex::new(start_flight),
            start_claimed: AtomicBool::new(start_claimed),
            operation_gate: Mutex::new(()),
            activity_gate: RwLock::new(()),
            close_gate: Mutex::new(()),
            cancellation: CancellationToken::new(),
            closing: AtomicBool::new(false),
            active_operation_count: AtomicUsize::new(0),
            fresh_observe_required: AtomicBool::new(false),
            restart_from_epoch: AtomicU64::new(0),
            priority,
            frozen_at_ms: AtomicU64::new(u64::MAX),
            workspace_hint,
        }
    }

    async fn current_snapshot(&self) -> BrowserLaneSnapshot {
        let mut snapshot = self.snapshot.read().await.clone();
        snapshot.active_operation_count =
            self.active_operation_count.load(Ordering::Acquire);
        snapshot
    }
}

struct PendingLaneCleanup {
    cleanup_id: u64,
    lane_id: BrowserLaneId,
    user_id: String,
    owner_lease_id: OwnerLeaseId,
    host_key: HostKey,
    browser_epoch: u64,
    driver: Arc<dyn BrowserLaneDriver>,
    flight: Mutex<Option<Arc<LaneCleanupFlight>>>,
}

/// A detached Lane may still have an in-flight start operation. The Host must
/// not be retired until that start either publishes its driver (which is then
/// represented by a pending lane cleanup) or finishes without one. This record
/// is Hub-owned so cancellation of the caller cannot lose the final shutdown
/// obligation.
#[derive(Clone)]
struct PendingHostRetirement {
    key: HostKey,
    lane_id: BrowserLaneId,
    user_id: String,
    owner_lease_id: OwnerLeaseId,
    start_flight: Arc<LaneStartFlight>,
}

struct DetachedLane {
    host_key: HostKey,
    browser_epoch: u64,
    cleanup_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RemainingResources {
    lane_count: usize,
    cleanup_count: usize,
    managed_host_count: usize,
}

type LaneStartResult = Result<BrowserLaneSnapshot, BrowserPlatformError>;

struct LaneStartFlight {
    result: OnceLock<LaneStartResult>,
    completed: Notify,
    waiters: AtomicUsize,
}

impl LaneStartFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
            waiters: AtomicUsize::new(0),
        }
    }

    fn complete(&self, result: LaneStartResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> LaneStartResult {
        loop {
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }

}

type LaneCleanupResult = Result<(), BrowserPlatformError>;

struct LaneCleanupFlight {
    result: OnceLock<LaneCleanupResult>,
    completed: Notify,
}

type HostFinalizationResult = Result<(), BrowserPlatformError>;

/// Hub-owned single-flight for retiring one empty Host. Explicit close,
/// cleanup completion callbacks and the lifecycle sweep must observe the same
/// attempt/result; otherwise a background retry can consume a transient
/// shutdown failure and make the original close falsely look successful.
struct HostFinalizationFlight {
    result: OnceLock<HostFinalizationResult>,
    completed: Notify,
}

impl HostFinalizationFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: HostFinalizationResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> HostFinalizationResult {
        loop {
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }
}

impl LaneCleanupFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: LaneCleanupResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> LaneCleanupResult {
        loop {
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }
}

struct LaneActiveOperation<'a> {
    count: &'a AtomicUsize,
}

impl<'a> LaneActiveOperation<'a> {
    fn begin(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self { count }
    }
}

impl Drop for LaneActiveOperation<'_> {
    fn drop(&mut self) {
        let previous = self.count.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "Lane active-operation count underflow");
    }
}

struct BrowserSessionHubInner {
    factory: Arc<dyn BrowserHostFactory>,
    clock: Arc<dyn Clock>,
    config: RwLock<HubConfig>,
    telemetry: RwLock<ResourceTelemetry>,
    scheduler: BrowserLaneScheduler,
    operation_budget_gate: Mutex<()>,
    operation_weight_limit: AtomicU64,
    active_operation_weight: AtomicU64,
    active_regular_operations: AtomicU64,
    operation_capacity_changed: Notify,
    active_heavy_operations: AtomicU64,
    owner_leases: OwnerLeaseService,
    identity_generations: IdentityGenerationCoordinator,
    identity_refresh_gate: Mutex<()>,
    lanes: RwLock<HashMap<BrowserLaneId, Arc<LaneRecord>>>,
    lane_keys: RwLock<HashMap<LaneKey, BrowserLaneId>>,
    // Retirement lock order is contractual for every path that touches more
    // than one of these structures:
    //   open_gate -> retiring_host_keys -> host_slots
    //     -> retiring_host_slots -> orphaned_host_slots
    // Never acquire an earlier authority while holding a later one.
    host_slots: RwLock<HashMap<HostKey, Arc<HostSlot>>>,
    host_empty_since_ms: RwLock<HashMap<HostKey, u64>>,
    retiring_host_slots: Mutex<Vec<(HostKey, Arc<HostSlot>)>>,
    retiring_host_keys: RwLock<HashSet<HostKey>>,
    retiring_hosts_changed: Notify,
    orphaned_host_slots: Mutex<Vec<(HostKey, Arc<HostSlot>)>>,
    pending_lane_cleanups: Mutex<Vec<Arc<PendingLaneCleanup>>>,
    pending_host_retirements: Mutex<Vec<PendingHostRetirement>>,
    owner_cleanup_targets: Mutex<HashMap<OwnerLeaseId, HashSet<OwnerCleanupTarget>>>,
    host_finalizations: Mutex<HashMap<HostKey, Arc<HostFinalizationFlight>>>,
    lane_cleanup_retry_gate: Mutex<()>,
    host_cleanup_retry_gate: Mutex<()>,
    cleanup_sequence: AtomicU64,
    host_epoch_sequence: AtomicU64,
    host_restarts: PerKeyHostRestartSingleFlight<HostKey>,
    host_circuits: Mutex<HashMap<HostKey, Arc<HostCircuitBreaker>>>,
    // Primary process visibility is a Host-wide property. Serialize explicit
    // display transitions with Primary Host selection/start so opposite
    // headful/headless requests cannot join the same restart flight or launch
    // a process from a stale default in the middle of a transition.
    primary_visibility_gate: Mutex<()>,
    drain_gate: Mutex<()>,
    draining: AtomicBool,
    open_gate: Mutex<()>,
    shutdown_gate: Mutex<()>,
    shutdown_result: RwLock<Option<Result<(), BrowserPlatformError>>>,
    shutting_down: AtomicBool,
    sequence: AtomicU64,
    events: broadcast::Sender<BrowserInventoryEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DriverResourceClass {
    Regular,
    Heavy,
}

impl DriverResourceClass {
    fn for_operation(operation: &BrowserOperation) -> Self {
        // Heavy classification is reserved for Agent work whose transient
        // encoding/rendering cost must still consume two units of the global
        // operation budget.
        let captures_image = operation.kind == BrowserOperationKind::Screenshot
            || operation
                .input
                .get("include_screenshot")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
        let renders_pdf = operation.kind == BrowserOperationKind::Download
            && operation.action == "save_as_pdf";
        if captures_image || renders_pdf {
            Self::Heavy
        } else {
            Self::Regular
        }
    }

    const fn weight(self) -> u64 {
        match self {
            Self::Regular => 1,
            Self::Heavy => 2,
        }
    }
}

struct HubDriverPermit {
    inner: Arc<BrowserSessionHubInner>,
    resource_class: DriverResourceClass,
    acquired_weight: u64,
}

struct HubDrainGuard {
    inner: Arc<BrowserSessionHubInner>,
}

impl Drop for HubDrainGuard {
    fn drop(&mut self) {
        self.inner.draining.store(false, Ordering::Release);
    }
}

impl Drop for HubDriverPermit {
    fn drop(&mut self) {
        let previous = match self.resource_class {
            DriverResourceClass::Regular => self
                .inner
                .active_regular_operations
                .fetch_sub(1, Ordering::AcqRel),
            DriverResourceClass::Heavy => self
                .inner
                .active_heavy_operations
                .fetch_sub(1, Ordering::AcqRel),
        };
        debug_assert!(previous > 0, "Hub operation permit count underflow");
        let previous_weight = self
            .inner
            .active_operation_weight
            .fetch_sub(self.acquired_weight, Ordering::AcqRel);
        debug_assert!(
            previous_weight >= self.acquired_weight,
            "Hub operation weight underflow"
        );
        self.inner.operation_capacity_changed.notify_waiters();
    }
}

#[derive(Clone)]
pub struct BrowserSessionHub {
    inner: Arc<BrowserSessionHubInner>,
}

struct LaneStartWaiter {
    hub: BrowserSessionHub,
    lane_id: BrowserLaneId,
    lane: Arc<LaneRecord>,
    flight: Arc<LaneStartFlight>,
}

enum OpenLaneAction {
    Return(OpenLaneOutcome),
    Wait(LaneStartWaiter),
}

impl LaneStartWaiter {
    fn new(
        hub: BrowserSessionHub,
        lane_id: BrowserLaneId,
        lane: Arc<LaneRecord>,
        flight: Arc<LaneStartFlight>,
    ) -> Self {
        flight.waiters.fetch_add(1, Ordering::AcqRel);
        Self {
            hub,
            lane_id,
            lane,
            flight,
        }
    }

    fn claim(&self) {
        self.lane.start_claimed.store(true, Ordering::Release);
    }
}

impl Drop for LaneStartWaiter {
    fn drop(&mut self) {
        let previous = self.flight.waiters.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "Lane start-flight waiter count underflow");
        if previous == 1 && !self.lane.start_claimed.load(Ordering::Acquire) {
            let hub = self.hub.clone();
            let lane_id = self.lane_id.clone();
            let lane = Arc::clone(&self.lane);
            tokio::spawn(async move {
                hub.abandon_unclaimed_lane_start(&lane_id, &lane).await;
            });
        }
    }
}

impl BrowserSessionHub {
    pub fn new(factory: Arc<dyn BrowserHostFactory>, config: HubConfig) -> Self {
        Self::with_clock(factory, config, Arc::new(SystemClock))
    }

    pub fn with_clock(
        factory: Arc<dyn BrowserHostFactory>,
        mut config: HubConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        if let Err(error) = config.resource_policy.validate() {
            tracing::error!(
                field = error.field,
                reason = error.reason,
                "invalid initial browser resource policy; using safe defaults"
            );
            config.resource_policy = ResourcePolicy::default();
        }
        let scheduler_config = SchedulerConfig {
            max_open_lanes: config.resource_policy.max_open_lanes,
            max_global_queue: config.resource_policy.max_global_queue,
            max_owner_queue: config.resource_policy.max_owner_queue,
            ..SchedulerConfig::default()
        };
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            inner: Arc::new(BrowserSessionHubInner {
                factory,
                scheduler: BrowserLaneScheduler::new(scheduler_config, Arc::clone(&clock)),
                operation_budget_gate: Mutex::new(()),
                operation_weight_limit: AtomicU64::new(
                    config.resource_policy.max_active_operations.max(1) as u64,
                ),
                active_operation_weight: AtomicU64::new(0),
                active_regular_operations: AtomicU64::new(0),
                operation_capacity_changed: Notify::new(),
                active_heavy_operations: AtomicU64::new(0),
                owner_leases: OwnerLeaseService::new(
                    Arc::clone(&clock),
                    config.owner_lease_ttl_ms,
                ),
                identity_generations: IdentityGenerationCoordinator::new(Arc::clone(&clock)),
                identity_refresh_gate: Mutex::new(()),
                clock,
                config: RwLock::new(config),
                telemetry: RwLock::new(ResourceTelemetry::default()),
                lanes: RwLock::new(HashMap::new()),
                lane_keys: RwLock::new(HashMap::new()),
                host_slots: RwLock::new(HashMap::new()),
                host_empty_since_ms: RwLock::new(HashMap::new()),
                retiring_host_slots: Mutex::new(Vec::new()),
                retiring_host_keys: RwLock::new(HashSet::new()),
                retiring_hosts_changed: Notify::new(),
                orphaned_host_slots: Mutex::new(Vec::new()),
                pending_lane_cleanups: Mutex::new(Vec::new()),
                pending_host_retirements: Mutex::new(Vec::new()),
                owner_cleanup_targets: Mutex::new(HashMap::new()),
                host_finalizations: Mutex::new(HashMap::new()),
                lane_cleanup_retry_gate: Mutex::new(()),
                host_cleanup_retry_gate: Mutex::new(()),
                cleanup_sequence: AtomicU64::new(0),
                host_epoch_sequence: AtomicU64::new(0),
                host_restarts: PerKeyHostRestartSingleFlight::default(),
                host_circuits: Mutex::new(HashMap::new()),
                primary_visibility_gate: Mutex::new(()),
                drain_gate: Mutex::new(()),
                draining: AtomicBool::new(false),
                open_gate: Mutex::new(()),
                shutdown_gate: Mutex::new(()),
                shutdown_result: RwLock::new(None),
                shutting_down: AtomicBool::new(false),
                sequence: AtomicU64::new(0),
                events,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BrowserInventoryEvent> {
        self.inner.events.subscribe()
    }

    /// Publish metadata for a freshly captured canonical Primary identity.
    ///
    /// The generation is assigned here and cannot be supplied by a caller.
    /// Actual browser-state capture/storage remains a trusted adapter seam.
    pub fn publish_identity_snapshot(
        &self,
        payload: IdentitySnapshotPayload,
        coverage: SnapshotCoverage,
    ) -> Result<CanonicalIdentitySnapshot, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        self.inner
            .identity_generations
            .publish_snapshot(coverage, payload)
    }

    pub fn current_identity_snapshot(
        &self,
    ) -> Result<Option<CanonicalIdentitySnapshot>, BrowserPlatformError> {
        self.inner.identity_generations.current_snapshot()
    }

    /// Process IDs for initialized, managed browser hosts.
    ///
    /// This stays process-internal so the application telemetry collector can
    /// account for each Chromium process tree without exposing host internals
    /// through public management DTOs.
    pub async fn managed_host_process_ids(&self) -> Vec<u32> {
        let slots = self.managed_host_slots().await;
        let mut process_ids = slots
            .iter()
            .filter_map(|slot| slot.get().and_then(|host| host.process_id()))
            .filter(|process_id| *process_id != 0)
            .collect::<Vec<_>>();
        process_ids.sort_unstable();
        process_ids.dedup();
        process_ids
    }

    async fn managed_host_slots(&self) -> Vec<Arc<HostSlot>> {
        let mut slots: Vec<_> = self
            .inner
            .host_slots
            .read()
            .await
            .values()
            .cloned()
            .collect();
        slots.extend(
            self.inner
                .retiring_host_slots
                .lock()
                .await
                .iter()
                .map(|(_, slot)| Arc::clone(slot)),
        );
        slots.extend(
            self.inner
                .orphaned_host_slots
                .lock()
                .await
                .iter()
                .map(|(_, slot)| Arc::clone(slot)),
        );
        let mut seen = HashSet::new();
        slots.retain(|slot| seen.insert(Arc::as_ptr(slot) as usize));
        slots
    }

    async fn remaining_resources(&self) -> RemainingResources {
        let lane_count = self.inner.lanes.read().await.len();
        let pending_lane_cleanups = self.inner.pending_lane_cleanups.lock().await.len();
        let pending_host_retirements =
            self.inner.pending_host_retirements.lock().await.len();
        let retiring_host_slots = self.inner.retiring_host_slots.lock().await.len();
        let orphaned_host_slots = self.inner.orphaned_host_slots.lock().await.len();
        RemainingResources {
            lane_count,
            cleanup_count: pending_lane_cleanups
                .saturating_add(pending_host_retirements)
                .saturating_add(retiring_host_slots)
                .saturating_add(orphaned_host_slots),
            managed_host_count: self.managed_host_slots().await.len(),
        }
    }

    async fn close_result(&self, closed: usize, already_closed: bool) -> CloseResult {
        let remaining = self.remaining_resources().await;
        CloseResult {
            closed,
            already_closed,
            remaining_lane_count: remaining.lane_count,
            remaining_cleanup_count: remaining.cleanup_count,
            remaining_managed_host_count: remaining.managed_host_count,
        }
    }

    fn scoped_close_result(closed: usize, already_closed: bool) -> CloseResult {
        // Caller/owner-scoped close paths must not disclose installation-wide
        // Host or cleanup inventory. The installation-owner `close_all` path
        // replaces these zeroed fields with authoritative global counts.
        CloseResult {
            closed,
            already_closed,
            ..CloseResult::default()
        }
    }

    async fn pending_cleanup_count_for_user(&self, user_id: &str) -> usize {
        let lane_cleanups = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let host_retirements = self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let owner_targets = self
            .inner
            .owner_cleanup_targets
            .lock()
            .await
            .iter()
            .flat_map(|(owner_lease_id, targets)| {
                targets
                    .iter()
                    .cloned()
                    .map(|target| (owner_lease_id.clone(), target))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let lane_records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut live_lanes = Vec::with_capacity(lane_records.len());
        for lane in lane_records {
            live_lanes.push(lane.current_snapshot().await);
        }
        // A target whose exact (HostKey, epoch) is still the live active Host
        // is a retained live-Host obligation that owner EndTurn, the warm
        // sweep, or an installation drain will retire; it is Host inventory,
        // not pending cleanup residue.
        let active_host_epochs = {
            let slots = self.inner.host_slots.read().await;
            slots
                .iter()
                .filter(|(_, slot)| slot.get().is_some())
                .map(|(key, slot)| (key.clone(), slot.epoch))
                .collect::<HashMap<_, _>>()
        };
        let lane_cleanup_count = lane_cleanups
            .iter()
            .filter(|entry| entry.user_id == user_id)
            .count();
        let host_retirement_count = host_retirements
            .iter()
            .filter(|entry| entry.user_id == user_id)
            .count();
        let standalone_target_count = owner_targets
            .iter()
            .filter(|(owner_lease_id, target)| {
                target.user_id == user_id
                    && active_host_epochs
                        .get(&target.host_key)
                        .is_none_or(|epoch| *epoch != target.browser_epoch)
                    && !lane_cleanups.iter().any(|entry| {
                        &entry.owner_lease_id == owner_lease_id
                            && entry.host_key == target.host_key
                            && entry.browser_epoch == target.browser_epoch
                    })
                    && !host_retirements.iter().any(|entry| {
                        &entry.owner_lease_id == owner_lease_id
                            && entry.key == target.host_key
                    })
                    && !live_lanes.iter().any(|lane| {
                        lane.browser_epoch == target.browser_epoch
                            && HostKey::for_lane(
                                lane.identity_mode,
                                lane.identity_generation,
                                &lane.lane_id,
                            ) == target.host_key
                    })
            })
            .count();
        lane_cleanup_count
            .saturating_add(host_retirement_count)
            .saturating_add(standalone_target_count)
    }

    async fn cleanup_error_with_remaining(
        &self,
        error: BrowserPlatformError,
        detached_closed: usize,
    ) -> BrowserPlatformError {
        let remaining = self.remaining_resources().await;
        let mut metadata = error.metadata.as_object().cloned().unwrap_or_default();
        metadata.insert("cleanup_pending".to_owned(), json!(true));
        metadata.insert("detached_closed".to_owned(), json!(detached_closed));
        metadata.insert("remaining_lane_count".to_owned(), json!(remaining.lane_count));
        metadata.insert(
            "remaining_cleanup_count".to_owned(),
            json!(remaining.cleanup_count),
        );
        metadata.insert(
            "remaining_managed_host_count".to_owned(),
            json!(remaining.managed_host_count),
        );
        BrowserPlatformError {
            metadata: serde_json::Value::Object(metadata),
            ..error
        }
    }

    fn scoped_cleanup_error(
        error: BrowserPlatformError,
        detached_closed: usize,
    ) -> BrowserPlatformError {
        let mut metadata = error.metadata.as_object().cloned().unwrap_or_default();
        metadata.insert("cleanup_pending".to_owned(), json!(true));
        metadata.insert("detached_closed".to_owned(), json!(detached_closed));
        metadata.insert("remaining_lane_count".to_owned(), json!(0));
        // The error itself proves at least one caller-authorized cleanup is
        // pending. Report that safe lower bound without exposing unrelated
        // installation inventory.
        metadata.insert("remaining_cleanup_count".to_owned(), json!(1));
        metadata.insert("remaining_managed_host_count".to_owned(), json!(0));
        BrowserPlatformError {
            metadata: serde_json::Value::Object(metadata),
            ..error
        }
    }

    pub fn bind(&self, caller: CallerIdentity) -> Result<BrowserLaneClient, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        // The owner lease is the mutable authority for a runtime capability.
        // Binding may only establish or narrow its policy, never broaden it.
        self.inner.owner_leases.bind_policy(&caller)?;
        Ok(BrowserLaneClient {
            hub: self.clone(),
            caller,
        })
    }

    pub fn issue_owner_lease(
        &self,
        user_id: impl Into<String>,
        conversation_id: Option<String>,
        runtime_instance_id: impl Into<String>,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        self.inner.owner_leases.issue(
            user_id,
            conversation_id,
            runtime_instance_id,
        )
    }

    pub fn renew_owner_lease(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<OwnerLease, BrowserPlatformError> {
        self.inner
            .owner_leases
            .renew(lease_id)
    }

    pub async fn revoke_owner_lease(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.close_owner_lease(lease_id).await
    }

    /// Closes all lanes currently bound to one owner lease while preserving
    /// the lease itself. This is the semantics needed by an Agent's
    /// `browser_close_all`: resources end, but the runtime may open a fresh
    /// default or named lane afterwards.
    pub async fn close_owner_lanes(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let result = self
            .close_matching(|lane| &lane.caller.owner_lease_id == lease_id)
            .await?;
        // A pending owner-cleanup error must not discard the accurate close
        // count: sweep and management callers read `detached_closed` to credit
        // the lanes that did close in this call.
        if let Err(error) = self.finish_owner_cleanup(lease_id).await {
            return Err(Self::scoped_cleanup_error(error, result.closed));
        }
        Ok(result)
    }

    /// Revokes one exact owner lease and closes only the lanes that carry that
    /// lease. This is the capability-scoped counterpart to `close_runtime`,
    /// which remains reserved for trusted runtime lifecycle teardown.
    pub async fn close_owner_lease(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        // Revoke first so no new operation can validate this owner while its
        // resources are being detached. `renew` removes an already-expired
        // lease before returning its error, so Lane cleanup must not depend on
        // this boolean: an expired/revoked lease may still have orphaned Lane
        // records that require authoritative cleanup.
        self.inner.owner_leases.revoke(lease_id);

        // `open_lane` validates again after acquiring this same gate. Taking it
        // here is therefore a barrier for an open that validated immediately
        // before revocation: an open already in its allocation critical section
        // finishes first and is included below; a waiting open observes the
        // revoked lease and fails before inserting a Lane.
        {
            let _open_guard = self.inner.open_gate.lock().await;
        }

        // Scope cleanup to the exact owner lease rather than the runtime. This
        // avoids a stale capability closing resources issued to a replacement
        // lease that happens to carry the same runtime identifier.
        self.close_owner_lanes(lease_id).await
    }

    async fn finish_owner_cleanup(
        &self,
        owner_lease_id: &OwnerLeaseId,
    ) -> Result<(), BrowserPlatformError> {
        // A Lane may have been detached while its Host.open_lane call was
        // still running. Settle only this owner's starts before looking for
        // retained target drivers; a late driver is published into the same
        // owner-scoped pending cleanup inventory.
        self.wait_for_pending_owner_starts(owner_lease_id).await?;
        self.retry_pending_lane_cleanups_for_owner(owner_lease_id)
            .await?;

        let targets = self
            .inner
            .owner_cleanup_targets
            .lock()
            .await
            .get(owner_lease_id)
            .cloned()
            .unwrap_or_default();
        let lane_records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(lane_records.len());
        for lane in lane_records {
            snapshots.push(lane.current_snapshot().await);
        }
        let pending_starts = self.inner.pending_host_retirements.lock().await.clone();
        let pending_cleanups = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        let mut completed = HashSet::new();
        let mut first_error = None;
        for target in targets {
            let matching_lanes = snapshots
                .iter()
                .filter(|snapshot| {
                    snapshot.browser_epoch == target.browser_epoch
                        && HostKey::for_lane(
                            snapshot.identity_mode,
                            snapshot.identity_generation,
                            &snapshot.lane_id,
                        ) == target.host_key
                })
                .collect::<Vec<_>>();
            if matching_lanes
                .iter()
                .any(|snapshot| &snapshot.caller.owner_lease_id == owner_lease_id)
            {
                if first_error.is_none() {
                    first_error = Some(owner_cleanup_pending_error());
                }
                continue;
            }
            if pending_cleanups.iter().any(|entry| {
                &entry.owner_lease_id == owner_lease_id
                    && entry.host_key == target.host_key
                    && entry.browser_epoch == target.browser_epoch
            }) {
                if first_error.is_none() {
                    first_error = Some(owner_cleanup_pending_error());
                }
                continue;
            }

            // Once the exact target is gone, a sibling Lane (or a foreign
            // in-flight start) owns the shared Primary Host. This owner is
            // fully clean and must not shut down that shared process.
            let shared_by_sibling = !matching_lanes.is_empty()
                || pending_starts.iter().any(|pending| {
                    pending.key == target.host_key
                        && &pending.owner_lease_id != owner_lease_id
                });
            if shared_by_sibling || target.browser_epoch == 0 {
                completed.insert(target);
                continue;
            }

            match self
                .retire_empty_host_authoritatively(
                    &target.host_key,
                    target.browser_epoch,
                )
                .await
            {
                Ok(true) => {
                    completed.insert(target);
                }
                Ok(false) => {
                    if first_error.is_none() {
                        first_error = Some(owner_cleanup_pending_error());
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        if !completed.is_empty() {
            let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
            if let Some(targets) = owner_targets.get_mut(owner_lease_id) {
                targets.retain(|target| !completed.contains(target));
                if targets.is_empty() {
                    owner_targets.remove(owner_lease_id);
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        if self
            .inner
            .owner_cleanup_targets
            .lock()
            .await
            .get(owner_lease_id)
            .is_some_and(|targets| !targets.is_empty())
        {
            return Err(owner_cleanup_pending_error());
        }
        Ok(())
    }

    fn validate_caller(&self, caller: &CallerIdentity) -> Result<(), BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        caller.validate(self.inner.clock.now_ms())?;
        self.inner.owner_leases.validate(caller)?;
        Ok(())
    }

    fn require_operation(
        &self,
        caller: &CallerIdentity,
        kind: BrowserOperationKind,
    ) -> Result<(), BrowserPlatformError> {
        self.validate_caller(caller)?;
        if caller.allows(kind) {
            return Ok(());
        }
        Err(BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            "This browser capability does not allow the requested operation.",
            false,
            "Request a capability with the required browser operation.",
        ))
    }

    fn require_identity_mode(
        &self,
        caller: &CallerIdentity,
        identity_mode: BrowserIdentityMode,
    ) -> Result<(), BrowserPlatformError> {
        let has_crawl = caller.allows(BrowserOperationKind::Crawl);
        let missing_crawl = match &caller.surface {
            crate::BrowserSurface::Native
            | crate::BrowserSurface::Gateway
            | crate::BrowserSurface::Acp
            | crate::BrowserSurface::Remote
            | crate::BrowserSurface::Cluster => {
                identity_mode == BrowserIdentityMode::Anonymous && !has_crawl
            }
            crate::BrowserSurface::System => {
                identity_mode == BrowserIdentityMode::AuthenticatedReplica && !has_crawl
            }
            crate::BrowserSurface::User => false,
        };
        if missing_crawl {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "This browser capability does not allow a crawl identity.",
                false,
                "Request a trusted browser capability with the crawl operation.",
            )
            .with_metadata(json!({
                "surface": &caller.surface,
                "requested_identity_mode": identity_mode,
                "required_operation": BrowserOperationKind::Crawl,
            })));
        }

        let allowed = match &caller.surface {
            crate::BrowserSurface::Native
            | crate::BrowserSurface::Gateway
            | crate::BrowserSurface::Acp
            | crate::BrowserSurface::Remote
            | crate::BrowserSurface::Cluster => matches!(
                identity_mode,
                BrowserIdentityMode::Primary | BrowserIdentityMode::Anonymous
            ),
            crate::BrowserSurface::System => matches!(
                identity_mode,
                BrowserIdentityMode::Anonymous | BrowserIdentityMode::AuthenticatedReplica
            ),
            crate::BrowserSurface::User => matches!(
                identity_mode,
                BrowserIdentityMode::Primary
                    | BrowserIdentityMode::Anonymous
                    | BrowserIdentityMode::Isolated
            ),
        };
        if allowed {
            return Ok(());
        }

        Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "The requested browser identity mode is not authorized for this caller surface.",
            false,
            "Use an identity mode granted by the trusted application surface.",
        )
        .with_metadata(json!({
            "surface": &caller.surface,
            "requested_identity_mode": identity_mode,
        })))
    }

    async fn acquire_driver_permit(
        &self,
        operation: &BrowserOperation,
        cancellation: &CancellationToken,
    ) -> Result<HubDriverPermit, BrowserPlatformError> {
        let resource_class = DriverResourceClass::for_operation(operation);
        let acquired_weight = self
            .acquire_operation_weight(resource_class.weight(), cancellation)
            .await?;
        match resource_class {
            DriverResourceClass::Regular => {
                self.inner
                    .active_regular_operations
                    .fetch_add(1, Ordering::AcqRel);
            }
            DriverResourceClass::Heavy => {
                self.inner
                    .active_heavy_operations
                    .fetch_add(1, Ordering::AcqRel);
            }
        }
        Ok(HubDriverPermit {
            inner: Arc::clone(&self.inner),
            resource_class,
            acquired_weight,
        })
    }

    async fn acquire_operation_weight(
        &self,
        weight: u64,
        cancellation: &CancellationToken,
    ) -> Result<u64, BrowserPlatformError> {
        loop {
            let notified = self.inner.operation_capacity_changed.notified();
            let acquired = {
                // Serialize admissions with pressure/policy limit updates.
                // Releases remain lock-free and can only create more room.
                let _budget_guard = self.inner.operation_budget_gate.lock().await;
                let limit = self.inner.operation_weight_limit.load(Ordering::Acquire);
                let current = self.inner.active_operation_weight.load(Ordering::Acquire);
                // A weighted operation must still make progress when a user
                // policy or Critical pressure lowers the entire budget below
                // its nominal weight. Admit an oversized operation only into
                // an empty budget, and retain its nominal weight so it stays
                // exclusive even if the policy limit rises while it runs.
                let oversized_exclusive = current == 0 && weight > limit;
                if oversized_exclusive || current.saturating_add(weight) <= limit {
                    self.inner
                        .active_operation_weight
                        .fetch_add(weight, Ordering::AcqRel);
                    Some(weight)
                } else {
                    None
                }
            };
            if let Some(acquired_weight) = acquired {
                return Ok(acquired_weight);
            }
            tokio::select! {
                _ = notified => {}
                _ = cancellation.cancelled() => {
                    return Err(BrowserPlatformError::shutting_down());
                }
            }
        }
    }

    async fn apply_operation_weight_limit(&self, limit: usize) {
        let _budget_guard = self.inner.operation_budget_gate.lock().await;
        self.inner
            .operation_weight_limit
            .store(limit.max(1) as u64, Ordering::Release);
        drop(_budget_guard);
        self.inner.operation_capacity_changed.notify_waiters();
    }

    async fn resource_workload(&self, lane_cold_start_bytes: u64) -> ResourceWorkload {
        let active_requests = self.inner.scheduler.active_requests();
        let queued_requests = self.inner.scheduler.queued_requests();
        let records = self.inner.lanes.read().await.clone();
        let mut workload = ResourceWorkload {
            active_lanes: active_requests.len(),
            queued_lanes: queued_requests.len(),
            queued_first_lanes: queued_requests
                .iter()
                .filter(|request| request.first_lane)
                .count(),
            active_operation_permits: usize::try_from(
                self.inner
                    .active_regular_operations
                    .load(Ordering::Acquire),
            )
            .unwrap_or(usize::MAX),
            active_heavy_operation_permits: usize::try_from(
                self.inner.active_heavy_operations.load(Ordering::Acquire),
            )
            .unwrap_or(usize::MAX),
            ..ResourceWorkload::default()
        };

        for request in active_requests {
            let snapshot = match records.get(&request.lane_id) {
                Some(lane) => Some(lane.current_snapshot().await),
                None => None,
            };
            let estimate = snapshot
                .as_ref()
                .map(|snapshot| snapshot.resource_estimate_bytes)
                .filter(|estimate| *estimate > 0)
                .unwrap_or(lane_cold_start_bytes);
            workload.active_lane_ewma_bytes = workload
                .active_lane_ewma_bytes
                .saturating_add(estimate);
            if let Some(snapshot) = snapshot {
                workload.frozen_lanes +=
                    usize::from(snapshot.lifecycle_state == LaneLifecycleState::Frozen);
                workload.primary_lanes +=
                    usize::from(snapshot.identity_mode == BrowserIdentityMode::Primary);
            }
        }
        for request in queued_requests {
            let estimate = match records.get(&request.lane_id) {
                Some(lane) => lane
                    .current_snapshot()
                    .await
                    .resource_estimate_bytes
                    .max(1),
                None => lane_cold_start_bytes,
            };
            workload.queued_lane_estimate_bytes = workload
                .queued_lane_estimate_bytes
                .saturating_add(estimate);
        }
        let pending_cleanup_count = self.inner.pending_lane_cleanups.lock().await.len() as u64;
        workload.queued_lane_estimate_bytes = workload.queued_lane_estimate_bytes.saturating_add(
            lane_cold_start_bytes.saturating_mul(pending_cleanup_count),
        );
        workload
    }

    async fn decide_resources(
        &self,
        policy: &ResourcePolicy,
        telemetry: &ResourceTelemetry,
    ) -> ResourceDecision {
        let workload = self
            .resource_workload(policy.lane_cold_start_bytes)
            .await;
        policy.decide_with_workload(telemetry, &workload)
    }

    fn promotion_policy(decision: &ResourceDecision) -> PromotionPolicy {
        PromotionPolicy::new(
            decision.admit_first_lane,
            decision.admit_expansion_lane,
            decision
                .first_lane_reason_code
                .unwrap_or("browser_capacity_queued"),
            decision
                .expansion_lane_reason_code
                .unwrap_or("browser_capacity_queued"),
        )
    }

    pub async fn open_lane(
        &self,
        caller: &CallerIdentity,
        lane_name: Option<&str>,
        identity_mode: BrowserIdentityMode,
        workspace_hint: Option<String>,
    ) -> Result<OpenLaneOutcome, BrowserPlatformError> {
        self.require_operation(caller, BrowserOperationKind::Manage)?;
        // Identity authorization is evaluated before both named-Lane lookup
        // and allocation. A narrower replacement capability therefore cannot
        // regain access merely because a same-name Lane already exists.
        self.require_identity_mode(caller, identity_mode)?;
        let lane_key = LaneKey::new(caller.runtime_instance_id.clone(), lane_name)?;

        // The gate covers only key allocation and scheduler bookkeeping.  It is
        // deliberately released before Host/Chromium I/O.
        let _open_guard = self.inner.open_gate.lock().await;
        // Owner revocation can race the first validation while this request is
        // waiting for allocation. Revalidate under the shared allocation gate
        // so a revoked owner cannot insert a Lane after authoritative cleanup.
        self.validate_caller(caller)?;
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(platform_drain_in_progress_error());
        }
        let existing = { self.inner.lane_keys.read().await.get(&lane_key).cloned() };
        if let Some(existing) = existing {
            let lane = { self.inner.lanes.read().await.get(&existing).cloned() };
            if let Some(lane) = lane {
                let snapshot = lane.current_snapshot().await;
                if snapshot.caller.user_id != caller.user_id
                    || snapshot.caller.runtime_instance_id != caller.runtime_instance_id
                    || snapshot.caller.owner_lease_id != caller.owner_lease_id
                {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::InvalidCallerIdentity,
                        "The named browser lane belongs to a different owner lease.",
                        false,
                        "Use the original capability or request a new runtime instance.",
                    ));
                }
                if snapshot.identity_mode != identity_mode {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::InvalidCallerIdentity,
                        "The named browser lane already exists with a different identity mode.",
                        false,
                        "Use the existing lane identity or choose a different lane name.",
                    )
                    .for_lane(snapshot.lane_id.clone())
                    .with_metadata(json!({
                        "requested_identity_mode": identity_mode,
                        "existing_identity_mode": snapshot.identity_mode,
                    })));
                }
                if snapshot.identity_mode == BrowserIdentityMode::AuthenticatedReplica {
                    self.inner
                        .identity_generations
                        .require_current_snapshot(snapshot.identity_generation)
                        .map_err(|error| error.for_lane(snapshot.lane_id.clone()))?;
                }
                if snapshot.lifecycle_state == LaneLifecycleState::Starting {
                    let flight = lane.start_flight.lock().await.clone().ok_or_else(|| {
                        lane_not_ready_error(snapshot.lane_id.clone())
                    })?;
                    let waiter = LaneStartWaiter::new(
                        self.clone(),
                        snapshot.lane_id.clone(),
                        Arc::clone(&lane),
                        flight,
                    );
                    let action = OpenLaneAction::Wait(waiter);
                    drop(_open_guard);
                    return self.finish_open_action(action).await;
                }
                if matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Running | LaneLifecycleState::Frozen
                ) {
                    lane.start_claimed.store(true, Ordering::Release);
                }
                let action = OpenLaneAction::Return(outcome_for_snapshot(snapshot)?);
                drop(_open_guard);
                return self.finish_open_action(action).await;
            }
        }

        let identity_generation = match identity_mode {
            BrowserIdentityMode::AuthenticatedReplica => {
                self.inner
                    .identity_generations
                    .require_published_snapshot()?
                    .generation
            }
            BrowserIdentityMode::Primary
            | BrowserIdentityMode::Anonymous
            | BrowserIdentityMode::Isolated => 0,
        };
        let lane_id = BrowserLaneId::new();
        let owner_id = caller.runtime_instance_id.clone();
        let existing_records: Vec<_> =
            self.inner.lanes.read().await.values().cloned().collect();
        let mut first_lane = true;
        for lane in existing_records {
            if lane.snapshot.read().await.caller.runtime_instance_id == owner_id {
                first_lane = false;
                break;
            }
        }
        let priority = if first_lane {
            LanePriority::First
        } else {
            LanePriority::Expansion
        };
        let decision = {
            let policy = self.inner.config.read().await.resource_policy.clone();
            let telemetry = self.inner.telemetry.read().await.clone();
            self.decide_resources(&policy, &telemetry).await
        };
        self.inner
            .scheduler
            .update_recommended_concurrency(decision.recommended_concurrency);
        let allow_immediate = match priority {
            LanePriority::First => decision.admit_first_lane,
            LanePriority::Expansion => decision.admit_expansion_lane,
        };
        let reason_code = match priority {
            LanePriority::First => decision.first_lane_reason_code,
            LanePriority::Expansion => decision.expansion_lane_reason_code,
        }
        .unwrap_or("browser_capacity_queued");
        let admission = self.inner.scheduler.admit(
            owner_id,
            lane_id.clone(),
            priority,
            allow_immediate,
            reason_code,
        )?;

        let now = self.inner.clock.now_ms();
        let (lifecycle_state, queue) = match &admission {
            Admission::Ready => (LaneLifecycleState::Starting, None),
            Admission::Queued(request) => (
                LaneLifecycleState::Queued,
                Some(self.inner.scheduler.metadata(&request.request_id)?),
            ),
        };
        let snapshot = BrowserLaneSnapshot {
            lane_id: lane_id.clone(),
            lane_key: lane_key.clone(),
            caller: caller.clone(),
            identity_mode,
            identity_generation,
            lifecycle_state,
            browser_epoch: 0,
            tabs: Vec::new(),
            active_tab_id: None,
            active_frame_id: None,
            ref_generation: 0,
            queue,
            resource_estimate_bytes: self
                .inner
                .config
                .read()
                .await
                .resource_policy
                .lane_cold_start_bytes,
            active_operation_count: 0,
            last_active_at_ms: now,
            created_at_ms: now,
            error_code: None,
            error_message: None,
            recoverable: true,
        };
        let start_flight =
            matches!(admission, Admission::Ready).then(|| Arc::new(LaneStartFlight::new()));
        let lane = Arc::new(LaneRecord::new(
            snapshot.clone(),
            priority,
            workspace_hint,
            start_flight.clone(),
            matches!(admission, Admission::Queued(_)),
        ));
        self.inner.lanes.write().await.insert(lane_id.clone(), Arc::clone(&lane));
        self.inner.lane_keys.write().await.insert(lane_key, lane_id.clone());
        drop(_open_guard);
        self.emit("lane_created", Some(&snapshot));

        let action = match admission {
            Admission::Ready => {
                let flight = start_flight.expect("Ready Lane must have a start flight");
                self.spawn_lane_start(lane_id.clone(), Arc::clone(&lane), Arc::clone(&flight));
                OpenLaneAction::Wait(LaneStartWaiter::new(
                    self.clone(),
                    lane_id,
                    lane,
                    flight,
                ))
            }
            Admission::Queued(_) => {
                OpenLaneAction::Return(OpenLaneOutcome::Queued { lane: snapshot })
            }
        };
        self.finish_open_action(action).await
    }

    async fn finish_open_action(
        &self,
        action: OpenLaneAction,
    ) -> Result<OpenLaneOutcome, BrowserPlatformError> {
        match action {
            OpenLaneAction::Return(outcome) => Ok(outcome),
            OpenLaneAction::Wait(waiter) => {
                let result = waiter.flight.wait().await;
                if result.is_ok() {
                    waiter.claim();
                }
                result.map(|lane| OpenLaneOutcome::Running { lane })
            }
        }
    }

    fn spawn_lane_start(
        &self,
        lane_id: BrowserLaneId,
        lane: Arc<LaneRecord>,
        flight: Arc<LaneStartFlight>,
    ) {
        let hub = self.clone();
        tokio::spawn(async move {
            // The inner task is the panic boundary. The outer Hub-owned task
            // always converges the flight and scheduler state, even if a
            // factory/Host implementation unwinds while starting.
            let start_hub = hub.clone();
            let start_lane_id = lane_id.clone();
            let start_lane = Arc::clone(&lane);
            let result = match tokio::spawn(async move {
                start_hub
                    .start_lane_once(start_lane_id, start_lane)
                    .await
            })
            .await
            {
                Ok(result) => result,
                Err(join_error) => {
                    tracing::error!(
                        lane_id = %lane_id,
                        cancelled = join_error.is_cancelled(),
                        panic = join_error.is_panic(),
                        "browser Lane start task terminated unexpectedly"
                    );
                    Err(lane_start_task_failed_error(lane_id.clone(), &join_error))
                }
            };
            let failed = result.is_err();
            if failed {
                // Detach and retain any late driver before publishing the
                // terminal start result. A close racing this task waits for
                // the flight, and may retire the Host only after this cleanup
                // authority has been established.
                hub.discard_lane_after_start_failure(&lane_id).await;
            }
            let mut active = lane.start_flight.lock().await;
            if active
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &flight))
            {
                *active = None;
            }
            drop(active);
            flight.complete(result);
            if failed {
                hub.finalize_hosts_ready_after_cleanup().await;
                hub.promote_released_capacity().await;
            }
        });
    }

    async fn ensure_lane_start_flight(
        &self,
        lane_id: BrowserLaneId,
        lane: Arc<LaneRecord>,
    ) -> Arc<LaneStartFlight> {
        let mut active = lane.start_flight.lock().await;
        if let Some(flight) = active.clone() {
            return flight;
        }
        let flight = Arc::new(LaneStartFlight::new());
        *active = Some(Arc::clone(&flight));
        drop(active);
        self.spawn_lane_start(lane_id, lane, Arc::clone(&flight));
        flight
    }

    async fn abandon_unclaimed_lane_start(
        &self,
        lane_id: &BrowserLaneId,
        lane: &Arc<LaneRecord>,
    ) {
        // Serialize the zero-waiter decision with duplicate-open waiter
        // registration, which is performed while holding open_gate. Without
        // this gate, a last-waiter drop could observe zero and detach just as
        // a duplicate call registered against the same Starting flight.
        let _open_guard = self.inner.open_gate.lock().await;
        if lane.start_claimed.load(Ordering::Acquire) {
            return;
        }
        let flight = lane.start_flight.lock().await.clone();
        if flight
            .as_ref()
            .is_some_and(|flight| flight.waiters.load(Ordering::Acquire) != 0)
        {
            return;
        }
        let lane_is_current = self
            .inner
            .lanes
            .read()
            .await
            .get(lane_id)
            .is_some_and(|current| Arc::ptr_eq(current, lane));
        let detached = if lane_is_current {
            self.detach_lane_for_close_locked(lane_id).await
        } else {
            None
        };
        drop(_open_guard);
        if let Some(detached) = detached {
            if let Some(cleanup_id) = detached.cleanup_id {
                let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
            }
            let _ = self.finalize_detached_host(detached).await;
            self.promote_released_capacity().await;
        }
    }

    async fn start_lane_once(
        &self,
        lane_id: BrowserLaneId,
        lane: Arc<LaneRecord>,
    ) -> LaneStartResult {
        if lane.closing.load(Ordering::Acquire) {
            return Err(lane_closed_error(lane_id));
        }
        // A queued Lane can outlive its capability while it waits for
        // capacity.  Validate the exact CallerIdentity again immediately
        // before doing Host/Chromium work; the periodic sweep is not the only
        // authority that is allowed to notice an expired owner.
        if let Err(error) = self.validate_lane_owner(&lane).await {
            return Err(error.for_lane(lane_id));
        }
        {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.lifecycle_state = LaneLifecycleState::Starting;
            snapshot.queue = None;
            snapshot.error_code = None;
            snapshot.error_message = None;
        }
        let (identity_mode, identity_generation) = {
            let snapshot = lane.snapshot.read().await;
            (snapshot.identity_mode, snapshot.identity_generation)
        };
        // The Lane start flight is Hub-owned, so holding this guard across
        // Host selection, target creation and driver publication survives
        // caller cancellation. A display transition can therefore never stop
        // the selected Primary Host in the gap before this Lane publishes its
        // browser epoch.
        let _primary_visibility_guard = if identity_mode == BrowserIdentityMode::Primary {
            Some(self.inner.primary_visibility_gate.lock().await)
        } else {
            None
        };
        if identity_mode == BrowserIdentityMode::AuthenticatedReplica {
            if let Err(error) = self
                .inner
                .identity_generations
                .require_current_snapshot(identity_generation)
            {
                self.mark_lane_failed(&lane, &error).await;
                return Err(error.for_lane(lane_id));
            }
        }
        let host = match self
            .get_or_launch_host(identity_mode, identity_generation, &lane_id)
            .await
        {
            Ok(host) => host,
            Err(error) => {
                self.mark_lane_failed(&lane, &error).await;
                return Err(error.for_lane(lane_id));
            }
        };
        // Record the exact selected Host epoch before target creation. If
        // open_lane later fails or completes after this Lane was detached, an
        // exact-owner retry still knows which retained Host process it must
        // prove stopped; browser_epoch has not yet been published to the Lane
        // snapshot at this point.
        {
            let caller = lane.snapshot.read().await.caller.clone();
            self.inner
                .owner_cleanup_targets
                .lock()
                .await
                .entry(caller.owner_lease_id)
                .or_default()
                .insert(OwnerCleanupTarget {
                    user_id: caller.user_id,
                    host_key: host.key.clone(),
                    browser_epoch: host.slot.epoch,
                });
        }
        let host_driver = Arc::clone(&host.driver);
        let request = LaneLaunchRequest {
            lane_id: lane_id.clone(),
            identity_mode,
            workspace_hint: lane.workspace_hint.clone(),
        };
        let open_lane = tokio::spawn(async move { host_driver.open_lane(request).await }).await;
        let driver = match open_lane {
            Ok(Ok(driver)) => driver,
            Ok(Err(error)) => {
                self.mark_lane_failed(&lane, &error).await;
                return Err(error.for_lane(lane_id));
            }
            Err(join_error) => {
                tracing::error!(
                    lane_id = %lane_id,
                    browser_epoch = host.slot.epoch,
                    cancelled = join_error.is_cancelled(),
                    panic = join_error.is_panic(),
                    "browser Host open_lane task terminated unexpectedly"
                );
                let error = host_open_lane_task_failed_error(lane_id.clone(), &join_error);
                if join_error.is_panic() {
                    if self
                        .retire_host_slot_for_cleanup(&host.key, host.slot.epoch, &host.slot)
                        .await
                    {
                        let cleanup_key = host.key.clone();
                        let cleanup_slot = Arc::clone(&host.slot);
                        if let Err(cleanup_error) = self
                            .attempt_orphaned_host_slot_cleanup(&cleanup_key, &cleanup_slot)
                            .await
                        {
                            tracing::warn!(
                                identity_mode = ?host.key.identity_mode,
                                browser_epoch = host.slot.epoch,
                                code = ?cleanup_error.code,
                                "panicked browser Host cleanup remains pending"
                            );
                        }
                    }
                }
                self.mark_lane_failed(&lane, &error).await;
                return Err(error);
            }
        };
        // `detach_lane_for_close` owns the same gate.  Keep the gate across
        // the final closing check, driver assignment, and lifecycle
        // transition so a close cannot remove the Lane and then leave a newly
        // opened driver behind in the detached record.
        let close_guard = lane.close_gate.lock().await;
        let lane_is_current = self
            .inner
            .lanes
            .read()
            .await
            .get(&lane_id)
            .is_some_and(|current| Arc::ptr_eq(current, &lane));
        if lane.closing.load(Ordering::Acquire) || !lane_is_current {
            let host_key = HostKey::for_lane(identity_mode, identity_generation, &lane_id);
            let cleanup_id = self
                .retain_pending_lane_cleanup(
                    lane_id.clone(),
                    lane.snapshot.read().await.caller.user_id.clone(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .owner_lease_id
                        .clone(),
                    host_key,
                    host.slot.epoch,
                    driver,
                )
                .await;
            drop(close_guard);
            self.discard_lane_after_start_failure(&lane_id).await;
            let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
            return Err(lane_closed_error(lane_id));
        }
        if let Err(error) = self.validate_lane_owner(&lane).await {
            let host_key = HostKey::for_lane(identity_mode, identity_generation, &lane_id);
            let cleanup_id = self
                .retain_pending_lane_cleanup(
                    lane_id.clone(),
                    lane.snapshot.read().await.caller.user_id.clone(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .owner_lease_id
                        .clone(),
                    host_key,
                    host.slot.epoch,
                    driver,
                )
                .await;
            drop(close_guard);
            // The owner may expire while Host I/O is in flight.  Do not leave
            // a failed Lane in inventory or an active scheduler permit behind
            // after that final validation fails.
            self.discard_lane_after_start_failure(&lane_id).await;
            let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
            return Err(error.for_lane(lane_id));
        }
        *lane.driver.write().await = Some(driver);
        let snapshot = {
            let mut snapshot = lane.snapshot.write().await;
            // The Hub owns the logical epoch. An adapter may expose a local
            // process epoch that resets when it constructs a replacement
            // driver, so never copy that value into the caller-visible
            // stale-handle fence.
            snapshot.browser_epoch = host.slot.epoch;
            snapshot.lifecycle_state = LaneLifecycleState::Running;
            snapshot.last_active_at_ms = self.inner.clock.now_ms();
            snapshot.clone()
        };
        self.emit("lane_running", Some(&snapshot));
        drop(close_guard);
        Ok(snapshot)
    }

    async fn validate_lane_owner(
        &self,
        lane: &LaneRecord,
    ) -> Result<(), BrowserPlatformError> {
        let caller = lane.snapshot.read().await.caller.clone();
        self.validate_caller(&caller)
    }

    /// Removes a Lane whose admission/start path can no longer be trusted.
    /// This deliberately does not promote another queued request; the caller
    /// that owns the scheduler transition performs promotion after it has
    /// finished handling the failed start.
    async fn discard_lane_after_start_failure(&self, lane_id: &BrowserLaneId) {
        let Some(detached) = self.detach_lane_for_close(lane_id).await else {
            return;
        };
        if let Some(cleanup_id) = detached.cleanup_id {
            let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
        }
    }

    async fn retain_pending_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
        user_id: String,
        owner_lease_id: OwnerLeaseId,
        host_key: HostKey,
        browser_epoch: u64,
        driver: Arc<dyn BrowserLaneDriver>,
    ) -> u64 {
        let cleanup_id = self.inner.cleanup_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let mut pending = self.inner.pending_lane_cleanups.lock().await;
        let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
        owner_targets
            .entry(owner_lease_id.clone())
            .or_default()
            .insert(OwnerCleanupTarget {
                user_id: user_id.clone(),
                host_key: host_key.clone(),
                browser_epoch,
            });
        pending.push(Arc::new(PendingLaneCleanup {
            cleanup_id,
            lane_id,
            user_id,
            owner_lease_id,
            host_key,
            browser_epoch,
            driver,
            flight: Mutex::new(None),
        }));
        cleanup_id
    }

    async fn mark_lane_failed(
        &self,
        lane: &LaneRecord,
        error: &BrowserPlatformError,
    ) {
        let snapshot = {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.lifecycle_state = LaneLifecycleState::Failed;
            snapshot.error_code = Some(error.code);
            snapshot.error_message = Some(error.message.clone());
            snapshot.recoverable = error.retryable;
            snapshot.clone()
        };
        self.emit("lane_failed", Some(&snapshot));
    }

    async fn get_or_launch_host(
        &self,
        identity_mode: BrowserIdentityMode,
        identity_generation: u64,
        lane_id: &BrowserLaneId,
    ) -> Result<HostHandle, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        let key = HostKey::for_lane(identity_mode, identity_generation, lane_id);
        let circuit = self.host_circuit(&key).await;
        let circuit_attempt = circuit.acquire_attempt()?;
        let half_open_probe = circuit_attempt.is_half_open();
        let retirement_deadline = Instant::now() + HOST_RETIREMENT_WAIT_TIMEOUT;
        loop {
            let changed = self.inner.retiring_hosts_changed.notified();
            if !self.inner.retiring_host_keys.read().await.contains(&key) {
                break;
            }
            if tokio::time::timeout_at(retirement_deadline, changed)
                .await
                .is_err()
            {
                return Err(retiring_host_wait_timeout_error(&key));
            }
        }
        self.inner
            .host_empty_since_ms
            .write()
            .await
            .remove(&key);
        let slot = {
            // The caller inserted its Lane while holding `open_gate`, then
            // released that gate before Chromium I/O. Reacquire it for Host
            // slot selection so explicit final-Lane retirement cannot detach
            // a slot between the retiring-key check above and this map access.
            let _open_guard = self.inner.open_gate.lock().await;
            if self.inner.retiring_host_keys.read().await.contains(&key) {
                return Err(retiring_host_wait_timeout_error(&key));
            }
            let current = { self.inner.host_slots.read().await.get(&key).cloned() };
            if let Some(slot) = current {
                slot
            } else {
                let mut slots = self.inner.host_slots.write().await;
                // Shutdown publishes `shutting_down` before taking this write
                // lock and draining the map. Checking under the same lock prevents
                // a start that passed the earlier fast check from inserting a new
                // HostSlot after the authoritative drain.
                if self.inner.shutting_down.load(Ordering::Acquire) {
                    return Err(BrowserPlatformError::shutting_down());
                }
                if self.inner.draining.load(Ordering::Acquire) {
                    return Err(platform_drain_in_progress_error());
                }
                Arc::clone(
                    slots.entry(key.clone()).or_insert_with(|| {
                        let epoch =
                            self.inner.host_epoch_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                        Arc::new(HostSlot::new(epoch, false))
                    }),
                )
            }
        };
        match self.initialize_host_slot(&key, Arc::clone(&slot)).await {
            Ok(driver) => {
                circuit_attempt.succeed();
                Ok(HostHandle { key, slot, driver })
            }
            Err(error) => {
                if half_open_probe {
                    circuit_attempt.fail();
                } else {
                    let _ = circuit.record_failure();
                }
                Err(error)
            }
        }
    }

    async fn initialize_host_slot(
        &self,
        key: &HostKey,
        slot: Arc<HostSlot>,
    ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
        self.initialize_host_slot_with_visibility(key, slot, None).await
    }

    async fn initialize_host_slot_with_visibility(
        &self,
        key: &HostKey,
        slot: Arc<HostSlot>,
        requested_headful: Option<bool>,
    ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
        let identity_mode = key.identity_mode;
        let host_identity_generation = key.identity_generation;
        let identity_snapshot_payload =
            if identity_mode == BrowserIdentityMode::AuthenticatedReplica {
                Some(
                    self.inner
                        .identity_generations
                        .require_current_payload(host_identity_generation)?,
                )
            } else {
                None
            };
        let factory = Arc::clone(&self.inner.factory);
        let browser_epoch = slot.epoch;
        let configured_headful = self.inner.config.read().await.headful;
        let headful = identity_mode == BrowserIdentityMode::Primary
            && requested_headful.unwrap_or(configured_headful);
        let host = slot
            .get_or_try_init(|| async move {
                factory
                    .launch(HostLaunchRequest {
                        host_id: BrowserHostId::new(),
                        browser_epoch,
                        identity_mode,
                        identity_generation: host_identity_generation,
                        identity_snapshot_payload,
                        headful,
                    })
                    .await
            })
            .await?;
        // A factory is the final launch-policy authority. In particular, do
        // not write `requested_headful` before OnceCell initialization: when
        // the slot already contains a Host that would only falsify metadata
        // without replacing the process.
        slot.headful.store(host.is_headful(), Ordering::Release);
        Ok(Arc::clone(host))
    }

    async fn host_circuit(&self, key: &HostKey) -> Arc<HostCircuitBreaker> {
        let mut circuits = self.inner.host_circuits.lock().await;
        Arc::clone(circuits.entry(key.clone()).or_insert_with(|| {
            Arc::new(HostCircuitBreaker::new(
                Arc::clone(&self.inner.clock),
                Default::default(),
            ))
        }))
    }

    async fn lanes_for_host_key(&self, key: &HostKey) -> Vec<Arc<LaneRecord>> {
        let lanes = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut matching = Vec::new();
        for lane in lanes {
            let snapshot = lane.snapshot.read().await;
            if HostKey::for_lane(
                snapshot.identity_mode,
                snapshot.identity_generation,
                &snapshot.lane_id,
            ) == *key
            {
                drop(snapshot);
                matching.push(lane);
            }
        }
        matching
    }

    async fn mark_host_restarting(&self, key: &HostKey, observed_epoch: u64) {
        for lane in self.lanes_for_host_key(key).await {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = {
                let mut snapshot = lane.snapshot.write().await;
                if snapshot.browser_epoch != observed_epoch {
                    continue;
                }
                let first_invalidation = !lane
                    .fresh_observe_required
                    .swap(true, Ordering::AcqRel);
                if first_invalidation {
                    lane.restart_from_epoch
                        .store(observed_epoch, Ordering::Release);
                    snapshot.ref_generation = snapshot.ref_generation.saturating_add(1);
                }
                snapshot.lifecycle_state = LaneLifecycleState::Starting;
                snapshot.tabs.clear();
                snapshot.active_tab_id = None;
                snapshot.active_frame_id = None;
                snapshot.error_code = Some(BrowserErrorCode::BrowserRestarted);
                snapshot.error_message =
                    Some("The managed browser is restarting.".to_owned());
                snapshot.recoverable = true;
                snapshot.clone()
            };
            self.emit("host_restarting", Some(&snapshot));
        }
    }

    async fn mark_host_recovery_failed(
        &self,
        key: &HostKey,
        observed_epoch: u64,
        error: &BrowserPlatformError,
    ) {
        for lane in self.lanes_for_host_key(key).await {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = {
                let mut snapshot = lane.snapshot.write().await;
                if snapshot.browser_epoch != observed_epoch {
                    continue;
                }
                // Recovery did not install a replacement driver. Keep the
                // stale-epoch fence armed and make the Lane visibly failed;
                // reporting Running here would dispatch the next operation to
                // the old driver after its Host has already been shut down.
                lane.fresh_observe_required.store(true, Ordering::Release);
                snapshot.lifecycle_state = LaneLifecycleState::Starting;
                snapshot.tabs.clear();
                snapshot.active_tab_id = None;
                snapshot.active_frame_id = None;
                snapshot.error_code = Some(error.code);
                snapshot.error_message = Some(error.message.clone());
                snapshot.recoverable = error.retryable;
                snapshot.clone()
            };
            self.emit("host_recovery_failed", Some(&snapshot));
        }
    }

    async fn retire_host_slot_for_cleanup(
        &self,
        key: &HostKey,
        observed_epoch: u64,
        observed_slot: &Arc<HostSlot>,
    ) -> bool {
        // Keep the active map authoritative until the retained cleanup queue
        // is also locked. Cancellation while waiting for the second lock
        // therefore leaves the slot active; after both locks are held, removal
        // and authority transfer contain no await point.
        let _open_guard = self.inner.open_gate.lock().await;
        let mut retiring_keys = self.inner.retiring_host_keys.write().await;
        let mut slots = self.inner.host_slots.write().await;
        let is_exact = slots.get(key).is_some_and(|current| {
            current.epoch == observed_epoch && Arc::ptr_eq(current, observed_slot)
        });
        if !is_exact {
            return false;
        }
        let mut orphaned = self.inner.orphaned_host_slots.lock().await;
        let slot = slots
            .remove(key)
            .expect("exact browser Host slot disappeared while write-locked");
        slot.retire();
        retiring_keys.insert(key.clone());
        if !orphaned.iter().any(|(pending_key, pending_slot)| {
            pending_key == key && Arc::ptr_eq(pending_slot, &slot)
        }) {
            orphaned.push((key.clone(), slot));
        }
        drop(orphaned);
        drop(slots);
        drop(retiring_keys);
        self.inner.host_empty_since_ms.write().await.remove(key);
        true
    }

    async fn recover_host_failure(
        &self,
        key: HostKey,
        observed_epoch: u64,
    ) -> Result<HostRestartTransition, BrowserPlatformError> {
        let hub = self.clone();
        let restart_key = key.clone();
        let restart_identity_mode = key.identity_mode;
        let flight = self
            .inner
            .host_restarts
            .run_bounded(
                key.clone(),
                observed_epoch,
                HOST_RESTART_ATTEMPT_TIMEOUT,
                move || async move {
                    let _primary_visibility_guard =
                        if restart_identity_mode == BrowserIdentityMode::Primary {
                            Some(hub.inner.primary_visibility_gate.lock().await)
                        } else {
                            None
                        };
                    hub.mark_host_restarting(&restart_key, observed_epoch)
                        .await;
                    let result = hub
                        .restart_host_once(restart_key.clone(), observed_epoch)
                        .await;
                    if let Err(error) = &result {
                        hub.mark_host_recovery_failed(&restart_key, observed_epoch, error)
                            .await;
                    }
                    result
                },
            )
            .await;
        flight.result
    }

    async fn restart_host_once(
        &self,
        key: HostKey,
        observed_epoch: u64,
    ) -> Result<HostRestartTransition, BrowserPlatformError> {
        self.restart_host_once_with_visibility(key, observed_epoch, None)
            .await
    }

    async fn restart_host_once_with_visibility(
        &self,
        key: HostKey,
        observed_epoch: u64,
        requested_headful: Option<bool>,
    ) -> Result<HostRestartTransition, BrowserPlatformError> {
        let current = { self.inner.host_slots.read().await.get(&key).cloned() };
        if let Some(current) = current {
            if current.epoch > observed_epoch {
                let host = self
                    .initialize_host_slot_with_visibility(
                        &key,
                        Arc::clone(&current),
                        requested_headful,
                    )
                    .await?;
                if requested_headful
                    .is_some_and(|desired_headful| current.is_headful() != desired_headful)
                {
                    return Err(visibility_transition_not_applied_error(
                        requested_headful.unwrap_or(false),
                    ));
                }
                let transition = HostRestartTransition::new(observed_epoch, current.epoch)?;
                self.rebind_host_lanes(
                    &key,
                    observed_epoch,
                    transition,
                    host,
                    requested_headful.is_some(),
                )
                .await?;
                return Ok(transition);
            }
        }

        let circuit = self.host_circuit(&key).await;
        // A user-requested visibility transition is not a Host failure. It
        // deliberately replaces a healthy headless process and must not
        // consume the recovery circuit's failure budget.
        let circuit_attempt = if requested_headful.is_some() {
            None
        } else {
            Some(circuit.acquire_attempt()?)
        };
        if requested_headful.is_none()
            && !circuit_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.is_half_open())
        {
            let recorded = circuit.record_failure();
            if recorded.is_open() {
            // Do not carry the map's read guard into exact-slot retirement,
            // which must acquire the write side of the same lock.
            let old_slot = {
                self.inner.host_slots.read().await.get(&key).cloned()
            };
            if let Some(old_slot) = old_slot {
                if self
                    .retire_host_slot_for_cleanup(&key, observed_epoch, &old_slot)
                    .await
                {
                    if let Err(error) = self
                        .attempt_orphaned_host_slot_cleanup(&key, &old_slot)
                        .await
                    {
                        tracing::warn!(
                            identity_mode = ?key.identity_mode,
                            browser_epoch = observed_epoch,
                            code = ?error.code,
                            "browser Host circuit opened and retired Host cleanup remains pending"
                        );
                    }
                }
            }
            return Err(recorded.browser_unavailable_error());
            }
        }

        // A restart only exists for the benefit of live Lanes. When an
        // explicit close has already emptied this key (it may even have
        // retired the slot entirely), relaunching would create a Host with
        // zero Lanes that nothing owns — for the trusted foreground
        // transition that would be a visible headful window the user just
        // closed. Fail the flight instead; the caller re-validates its Lane.
        let mut has_live_lane = false;
        for lane in self.lanes_for_host_key(&key).await {
            if !lane.closing.load(Ordering::Acquire) {
                has_live_lane = true;
                break;
            }
        }
        if !has_live_lane {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser Host changed during recovery.",
                true,
                "Refresh browser status and retry.",
            ));
        }

        let old_slot = self.inner.host_slots.read().await.get(&key).cloned();
        if let Some(old_slot) = &old_slot {
            if old_slot.epoch != observed_epoch {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "The managed browser Host changed during recovery.",
                    true,
                    "Refresh browser status and retry.",
                ));
            }
            // Primary uses a stable profile directory, so the old process must
            // be explicitly stopped before launching its replacement.
            old_slot.shutdown_retired().await?;
        }

        let new_epoch = self.inner.host_epoch_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let new_slot = Arc::new(HostSlot::new(
            new_epoch,
            requested_headful.unwrap_or(false),
        ));
        {
            let mut slots = self.inner.host_slots.write().await;
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::shutting_down());
            }
            if self.inner.draining.load(Ordering::Acquire) {
                return Err(platform_drain_in_progress_error());
            }
            if let Some(old_slot) = &old_slot {
                if !slots
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, old_slot))
                {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::BrowserUnavailable,
                        "The managed browser Host changed during recovery.",
                        true,
                        "Refresh browser status and retry.",
                    ));
                }
            }
            slots.insert(key.clone(), Arc::clone(&new_slot));
        }

        let host = match self
            .initialize_host_slot_with_visibility(
                &key,
                Arc::clone(&new_slot),
                requested_headful,
            )
            .await
        {
            Ok(host) => host,
            Err(error) => {
                if self
                    .retire_host_slot_for_cleanup(&key, new_epoch, &new_slot)
                    .await
                {
                    let _ = self
                        .attempt_orphaned_host_slot_cleanup(&key, &new_slot)
                        .await;
                }
                return Err(error);
            }
        };
        let transition = HostRestartTransition::new(observed_epoch, new_epoch)?;
        self.rebind_host_lanes(
            &key,
            observed_epoch,
            transition,
            host,
            requested_headful.is_some(),
        )
        .await?;
        // Close the remaining race window: an explicit close may have emptied
        // this key between the live-lane check above and the slot insert.
        // `finalize_empty_host` re-validates under `open_gate` and retires the
        // fresh Host immediately when no Lane is attached; with live Lanes it
        // is a no-op. This keeps "explicit last-Lane close retires the Host
        // immediately" true across a concurrent restart.
        if let Err(error) = self.finalize_empty_host(key.clone()).await {
            tracing::warn!(
                identity_mode = ?key.identity_mode,
                code = ?error.code,
                "browser Host restart left a pending retirement for an emptied key"
            );
        }
        if let Some(circuit_attempt) = circuit_attempt {
            circuit_attempt.succeed();
        }
        self.emit("host_restarted", None);
        Ok(transition)
    }

    async fn rebind_host_lanes(
        &self,
        key: &HostKey,
        observed_epoch: u64,
        transition: HostRestartTransition,
        host: Arc<dyn BrowserHostDriver>,
        intentional_visibility_transition: bool,
    ) -> Result<(), BrowserPlatformError> {
        let mut prepared = Vec::new();
        for lane in self.lanes_for_host_key(key).await {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let (lane_id, identity_mode, workspace_hint, epoch) = {
                let snapshot = lane.snapshot.read().await;
                (
                    snapshot.lane_id.clone(),
                    snapshot.identity_mode,
                    lane.workspace_hint.clone(),
                    snapshot.browser_epoch,
                )
            };
            if epoch != observed_epoch {
                continue;
            }
            let driver = match host
                .open_lane(LaneLaunchRequest {
                    lane_id,
                    identity_mode,
                    workspace_hint,
                })
                .await
            {
                Ok(driver) => driver,
                Err(error) => {
                    // A replacement Host may have already created targets for
                    // earlier Lanes when a later open fails.  Do not let those
                    // drivers fall out of scope: retain them under Hub
                    // authority and make a best-effort close before returning
                    // the recovery error.  Failed closes remain in the
                    // lifecycle retry queue.
                    let cleanup_error = self
                        .cleanup_prepared_rebind_drivers(
                            std::mem::take(&mut prepared),
                            transition.new_epoch,
                        )
                        .await;
                    if let Some(cleanup_error) = cleanup_error {
                        tracing::warn!(
                            code = ?cleanup_error.code,
                            "browser Host rebind failed and prepared Lane cleanup remains pending"
                        );
                    }
                    return Err(error);
                }
            };
            prepared.push((lane, driver));
        }

        for (lane, driver) in prepared {
            let (lane_id, host_key) = {
                let snapshot = lane.snapshot.read().await;
                (
                    snapshot.lane_id.clone(),
                    HostKey::for_lane(
                        snapshot.identity_mode,
                        snapshot.identity_generation,
                        &snapshot.lane_id,
                    ),
                )
            };
            // Closing and host recovery may race after the replacement driver
            // has been prepared.  Serialize the final driver assignment with
            // detach_lane_for_close so a detached Lane can never receive a
            // driver after its inventory record has been removed.
            let close_guard = lane.close_gate.lock().await;
            let lane_is_current = self
                .inner
                .lanes
                .read()
                .await
                .get(&lane_id)
                .is_some_and(|current| Arc::ptr_eq(current, &lane));
            let can_rebind = !lane.closing.load(Ordering::Acquire)
                && lane_is_current
                && lane.snapshot.read().await.browser_epoch == observed_epoch;
            if !can_rebind {
                drop(close_guard);
                let cleanup_id = self
                    .retain_pending_lane_cleanup(
                        lane_id,
                        lane.snapshot.read().await.caller.user_id.clone(),
                        lane.snapshot
                            .read()
                            .await
                            .caller
                            .owner_lease_id
                            .clone(),
                        host_key,
                        transition.new_epoch,
                        driver,
                    )
                    .await;
                let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
                continue;
            }
            *lane.driver.write().await = Some(driver);
            let snapshot = {
                let mut snapshot = lane.snapshot.write().await;
                lane.restart_from_epoch
                    .store(transition.old_epoch, Ordering::Release);
                lane.fresh_observe_required
                    .store(true, Ordering::Release);
                snapshot.browser_epoch = transition.new_epoch;
                snapshot.lifecycle_state = LaneLifecycleState::Running;
                snapshot.tabs.clear();
                snapshot.active_tab_id = None;
                snapshot.active_frame_id = None;
                if intentional_visibility_transition {
                    // A user-requested visibility replacement is not a
                    // failure. The stale-epoch fence stays armed through
                    // `fresh_observe_required`, but the Lane must not carry a
                    // persistent restart error that nothing will ever clear
                    // (login lanes may never observe again).
                    snapshot.error_code = None;
                    snapshot.error_message = None;
                } else {
                    snapshot.error_code = Some(BrowserErrorCode::BrowserRestarted);
                    snapshot.error_message = Some(
                        "The managed browser restarted; a fresh observe is required.".to_owned(),
                    );
                }
                snapshot.recoverable = true;
                snapshot.clone()
            };
            drop(close_guard);
            self.emit("lane_rebound_after_host_restart", Some(&snapshot));
        }
        Ok(())
    }

    async fn cleanup_prepared_rebind_drivers(
        &self,
        prepared: Vec<(Arc<LaneRecord>, Arc<dyn BrowserLaneDriver>)>,
        browser_epoch: u64,
    ) -> Option<BrowserPlatformError> {
        let mut cleanup_ids = Vec::with_capacity(prepared.len());
        for (lane, driver) in prepared {
            let (lane_id, host_key) = {
                let snapshot = lane.snapshot.read().await;
                (
                    snapshot.lane_id.clone(),
                    HostKey::for_lane(
                        snapshot.identity_mode,
                        snapshot.identity_generation,
                        &snapshot.lane_id,
                    ),
                )
            };
            let cleanup_id = self
                .retain_pending_lane_cleanup(
                    lane_id,
                    lane.snapshot.read().await.caller.user_id.clone(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .owner_lease_id
                        .clone(),
                    host_key,
                    browser_epoch,
                    driver,
                )
                .await;
            cleanup_ids.push(cleanup_id);
        }
        let deadline = Instant::now() + CLEANUP_BATCH_WAIT_TIMEOUT;
        let mut attempts = tokio::task::JoinSet::new();
        for cleanup_id in cleanup_ids {
            let hub = self.clone();
            attempts.spawn(async move {
                hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                    .await
            });
        }
        let mut first_error = None;
        while let Some(attempt) = attempts.join_next().await {
            match attempt {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(join_error) => {
                    if first_error.is_none() {
                        first_error =
                            Some(cleanup_batch_task_failed_error("lane", &join_error));
                    }
                }
            }
        }
        first_error
    }

    pub async fn execute(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.execute_with_confirmation(caller, lane_id, operation, false)
            .await
    }

    /// Trusted in-process dispatch after a transport has consumed a matching
    /// one-shot human approval. This is intentionally not a wire operation and
    /// must never be selected from model-controlled input.
    async fn execute_confirmed(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.execute_with_confirmation(caller, lane_id, operation, true)
            .await
    }

    async fn execute_with_confirmation(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
        trusted_out_of_band_confirmation: bool,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.require_operation(caller, operation.kind)?;
        let lane = self.authorized_lane(caller, lane_id).await?;
        if lane.closing.load(Ordering::Acquire) {
            return Err(lane_closed_error(lane_id.clone()));
        }

        // Correctness is serialized per Lane.  Other Lane gates remain free,
        // and the global semaphore is only a resource bound.
        let _lane_guard = tokio::select! {
            guard = lane.operation_gate.lock() => guard,
            _ = lane.cancellation.cancelled() => return Err(lane_closed_error(lane_id.clone())),
        };
        self.execute_lane_driver(
            &lane,
            lane_id,
            operation,
            trusted_out_of_band_confirmation,
        )
        .await
    }

    async fn execute_lane_driver(
        &self,
        lane: &LaneRecord,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
        trusted_out_of_band_confirmation: bool,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let should_refresh_identity = identity_operation_needs_refresh(&operation);
        let is_fresh_observe = operation.kind == BrowserOperationKind::Observe;
        let pending_recovery = {
            let snapshot = lane.snapshot.read().await;
            (lane.fresh_observe_required.load(Ordering::Acquire)
                && snapshot.lifecycle_state == LaneLifecycleState::Starting)
                .then(|| {
                    (
                        HostKey::for_lane(
                            snapshot.identity_mode,
                            snapshot.identity_generation,
                            &snapshot.lane_id,
                        ),
                        snapshot.browser_epoch,
                    )
                })
        };
        if let Some((host_key, observed_epoch)) = pending_recovery {
            return match self
                .recover_host_failure(host_key, observed_epoch)
                .await
            {
                Ok(transition) => Err(transition
                    .browser_restarted_error()
                    .for_lane(lane_id.clone())),
                Err(error) => Err(error.for_lane(lane_id.clone())),
            };
        }
        // All driver work participates in this read side so pressure lifecycle
        // work can prove a lane is idle before freezing or closing it.
        let activity_guard = lane.activity_gate.read().await;
        let (context, identity_mode, host_key) = {
            let snapshot = lane.snapshot.read().await;
            if let Some(expected_epoch) = operation.expected_browser_epoch {
                if expected_epoch != snapshot.browser_epoch {
                    return Err(stale_browser_epoch_error(
                        expected_epoch,
                        snapshot.browser_epoch,
                    )
                    .for_lane(lane_id.clone()));
                }
            }
            if lane.fresh_observe_required.load(Ordering::Acquire)
                && (snapshot.lifecycle_state != LaneLifecycleState::Running || !is_fresh_observe)
            {
                return Err(lane_restart_notice(lane, &snapshot).for_lane(lane_id.clone()));
            }
            if snapshot.lifecycle_state != LaneLifecycleState::Running {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "The browser lane is not ready.",
                    true,
                    "Wait for the lane to become running or open another lane.",
                )
                .for_lane(lane_id.clone()));
            }
            if snapshot.identity_mode == BrowserIdentityMode::AuthenticatedReplica {
                self.inner
                    .identity_generations
                    .require_current_snapshot(snapshot.identity_generation)
                    .map_err(|error| error.for_lane(lane_id.clone()))?;
            }
            if let Some(expected) = operation.ref_generation {
                if expected != snapshot.ref_generation {
                    return Err(BrowserPlatformError::new(
                        BrowserErrorCode::StaleLaneRef,
                        "The browser reference belongs to an older observation.",
                        true,
                        "Observe the lane again before acting.",
                    )
                    .for_lane(lane_id.clone()));
                }
            }
            (
                OperationContext {
                    browser_epoch: snapshot.browser_epoch,
                    lane_id: lane_id.clone(),
                    target_id: operation.target_id.clone().or_else(|| {
                        snapshot.active_tab_id.as_ref().and_then(|active_tab_id| {
                            snapshot
                                .tabs
                                .iter()
                                .find(|tab| &tab.tab_id == active_tab_id)
                                .map(|tab| tab.target_id.clone())
                        })
                    }),
                    frame_id: operation
                        .frame_id
                        .clone()
                        .or_else(|| snapshot.active_frame_id.clone()),
                    ref_generation: snapshot.ref_generation,
                    cancellation_id: crate::QueueRequestId::new().to_string(),
                },
                snapshot.identity_mode,
                HostKey::for_lane(
                    snapshot.identity_mode,
                    snapshot.identity_generation,
                    &snapshot.lane_id,
                ),
            )
        };
        let dispatch_epoch = context.browser_epoch;
        if let Err(error) = require_lane_operation(identity_mode, &operation) {
            return Err(error.for_lane(lane_id.clone()));
        }
        let permit = self
            .acquire_driver_permit(&operation, &lane.cancellation)
            .await
            .map_err(|_| lane_closed_error(lane_id.clone()))?;
        let driver = lane.driver.read().await.clone().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser lane driver is unavailable.",
                true,
                "Retry after the lane recovers.",
            )
            .for_lane(lane_id.clone())
        })?;
        let active_operation =
            LaneActiveOperation::begin(&lane.active_operation_count);
        {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.active_operation_count =
                lane.active_operation_count.load(Ordering::Acquire);
            snapshot.last_active_at_ms = self.inner.clock.now_ms();
        }
        let child_cancel = lane.cancellation.child_token();
        let result = tokio::select! {
            result = driver.execute(
                operation,
                DriverOperationContext {
                    operation: context,
                    cancellation: child_cancel,
                    trusted_out_of_band_confirmation,
                },
            ) => result,
            _ = lane.cancellation.cancelled() => Err(lane_closed_error(lane_id.clone())),
        };
        if result
            .as_ref()
            .err()
            .is_some_and(is_host_fatal_error)
        {
            drop(permit);
            drop(active_operation);
            drop(activity_guard);
            return match self
                .recover_host_failure(host_key, dispatch_epoch)
                .await
            {
                Ok(transition) => Err(transition
                    .browser_restarted_error()
                    .for_lane(lane_id.clone())),
                Err(error) => Err(error.for_lane(lane_id.clone())),
            };
        }
        if result.is_ok()
            && identity_mode == BrowserIdentityMode::Primary
            && should_refresh_identity
        {
            let _identity_guard = self.inner.identity_refresh_gate.lock().await;
            let captured = tokio::select! {
                captured = driver.capture_identity_snapshot() => captured,
                _ = lane.cancellation.cancelled() => Err(lane_closed_error(lane_id.clone())),
            };
            match captured {
                Ok(Some(CapturedIdentitySnapshot { payload, coverage })) => {
                    if let Err(error) = self.publish_identity_snapshot(payload, coverage) {
                        if let Err(invalidation_error) =
                            self.inner.identity_generations.invalidate_current_snapshot()
                        {
                            tracing::error!(
                                code = ?invalidation_error.code,
                                lane_id = %lane_id,
                                "Primary identity snapshot publication and invalidation both failed"
                            );
                        }
                        tracing::warn!(
                            code = ?error.code,
                            lane_id = %lane_id,
                            "Primary identity snapshot publication failed; invalidated the previous replica generation"
                        );
                    }
                }
                Ok(None) => {
                    if let Err(error) =
                        self.inner.identity_generations.invalidate_current_snapshot()
                    {
                        tracing::error!(
                            code = ?error.code,
                            lane_id = %lane_id,
                            "Primary identity capture returned no snapshot and invalidation failed"
                        );
                    }
                    tracing::warn!(
                        lane_id = %lane_id,
                        "Primary identity capture returned no snapshot; invalidated the previous replica generation"
                    );
                }
                Err(error) => {
                    if let Err(invalidation_error) =
                        self.inner.identity_generations.invalidate_current_snapshot()
                    {
                        tracing::error!(
                            code = ?invalidation_error.code,
                            lane_id = %lane_id,
                            "Primary identity capture and invalidation both failed"
                        );
                    }
                    tracing::warn!(
                        code = ?error.code,
                        lane_id = %lane_id,
                        "Primary identity capture failed; invalidated the previous replica generation"
                    );
                }
            }
        }
        drop(permit);
        drop(active_operation);
        let (snapshot, epoch_changed) = {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.active_operation_count =
                lane.active_operation_count.load(Ordering::Acquire);
            snapshot.last_active_at_ms = self.inner.clock.now_ms();
            let epoch_changed = snapshot.browser_epoch != dispatch_epoch
                || snapshot.lifecycle_state != LaneLifecycleState::Running;
            if !epoch_changed && let Ok(output) = &result {
                let active_tab_changed =
                    output.active_tab_id.as_ref().is_some_and(|active_tab_id| {
                        snapshot
                            .active_tab_id
                            .as_ref()
                            .is_some_and(|current_tab_id| current_tab_id != active_tab_id)
                    });
                let next_ref_generation = if active_tab_changed {
                    Some(
                        snapshot
                            .ref_generation
                            .checked_add(1)
                            .ok_or_else(|| ref_generation_exhausted_error(lane_id.clone()))?,
                    )
                } else {
                    None
                };
                if !output.tabs.is_empty() {
                    snapshot.tabs.clone_from(&output.tabs);
                }
                if let Some(next_ref_generation) = next_ref_generation {
                    snapshot.ref_generation = next_ref_generation;
                    // A frame cursor is target-local.  A tab transition must
                    // invalidate it even if an adapter happens to return a
                    // stale frame id alongside the new active tab.
                    snapshot.active_frame_id = None;
                } else if output.active_frame_id.is_some() {
                    snapshot.active_frame_id.clone_from(&output.active_frame_id);
                }
                if output.active_tab_id.is_some() {
                    snapshot.active_tab_id.clone_from(&output.active_tab_id);
                }
                if let Some(ref_generation) = output.ref_generation {
                    // Reference invalidation is monotonic. A late or stale
                    // adapter result must not make an older handle current
                    // again by moving the generation backwards.
                    snapshot.ref_generation = snapshot.ref_generation.max(ref_generation);
                }
                if is_fresh_observe {
                    lane.fresh_observe_required
                        .store(false, Ordering::Release);
                    snapshot.error_code = None;
                    snapshot.error_message = None;
                }
            }
            (snapshot.clone(), epoch_changed)
        };
        drop(activity_guard);
        self.emit("lane_operation_finished", Some(&snapshot));
        if epoch_changed {
            if lane.closing.load(Ordering::Acquire) {
                return Err(lane_closed_error(lane_id.clone()));
            }
            return Err(lane_restart_notice(lane, &snapshot).for_lane(lane_id.clone()));
        }
        result.map_err(|error| error.for_lane(lane_id.clone()))
    }

    async fn authorized_lane(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
    ) -> Result<Arc<LaneRecord>, BrowserPlatformError> {
        let lane = self
            .inner
            .lanes
            .read()
            .await
            .get(lane_id)
            .cloned()
            .ok_or_else(|| BrowserPlatformError::lane_not_found(lane_id.clone()))?;
        let snapshot = lane.snapshot.read().await;
        if snapshot.caller.user_id != caller.user_id
            || snapshot.caller.runtime_instance_id != caller.runtime_instance_id
            || snapshot.caller.owner_lease_id != caller.owner_lease_id
            || snapshot.caller.surface != caller.surface
            || !caller
                .allowed_operations
                .is_subset(&snapshot.caller.allowed_operations)
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::OperationNotAllowed,
                "This browser lane belongs to another caller.",
                false,
                "Use a lane handle issued for the current runtime.",
            )
            .for_lane(lane_id.clone()));
        }
        drop(snapshot);
        Ok(lane)
    }

    /// Returns the default visibility applied to future Primary Host launches.
    pub async fn primary_visibility(&self) -> BrowserVisibility {
        if self.inner.config.read().await.headful {
            BrowserVisibility::Headful
        } else {
            BrowserVisibility::Headless
        }
    }

    /// Applies the installation-wide Primary display policy.
    ///
    /// A live Primary Host is replaced in-place and every Lane is rebound to a
    /// fresh epoch before the future-launch default is committed. Primary Host
    /// selection is serialized with this transition so no launch can observe a
    /// half-applied policy.
    pub async fn set_primary_visibility(
        &self,
        visibility: BrowserVisibility,
    ) -> Result<BrowserVisibility, BrowserPlatformError> {
        let hub = self.clone();
        tokio::spawn(async move { hub.set_primary_visibility_once(visibility).await })
            .await
            .map_err(|error| visibility_task_failed_error("primary", &error))?
    }

    async fn set_primary_visibility_once(
        &self,
        visibility: BrowserVisibility,
    ) -> Result<BrowserVisibility, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(platform_drain_in_progress_error());
        }
        let _visibility_guard = self.inner.primary_visibility_gate.lock().await;
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(platform_drain_in_progress_error());
        }
        let desired_headful = visibility.is_headful();
        let key = HostKey {
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 0,
            isolation_lane_id: None,
        };

        let slot = { self.inner.host_slots.read().await.get(&key).cloned() };
        if let Some(slot) = slot {
            if slot.is_headful() != desired_headful {
                let live_lanes = self.lanes_for_host_key(&key).await;
                if live_lanes
                    .iter()
                    .any(|lane| !lane.closing.load(Ordering::Acquire))
                {
                    self.transition_primary_visibility_locked(
                        &key,
                        slot.epoch,
                        desired_headful,
                    )
                    .await?;
                } else {
                    // An empty visible Host has no user work to rebind. Retire
                    // it instead of launching another empty process solely to
                    // change its mode.
                    if !self
                        .retire_empty_host_authoritatively(&key, slot.epoch)
                        .await?
                    {
                        return Err(visibility_transition_not_applied_error(
                            desired_headful,
                        ));
                    }
                }
            }
        }
        self.inner.config.write().await.headful = desired_headful;
        Ok(visibility)
    }

    /// Changes the live Primary Host visibility for an authenticated Lane.
    ///
    /// This does not mutate the installation default; the management policy
    /// endpoint owns that choice. Because Primary Lanes share one canonical
    /// Host, the process replacement and epoch transition apply to every live
    /// Primary Lane.
    pub async fn set_lane_visibility_for_user(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        visibility: BrowserVisibility,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        let hub = self.clone();
        let user_id = user_id.to_owned();
        let lane_id = lane_id.clone();
        tokio::spawn(async move {
            hub.set_lane_visibility_and_maybe_focus_once(
                &user_id,
                &lane_id,
                visibility,
                false,
            )
            .await
        })
        .await
        .map_err(|error| visibility_task_failed_error("lane", &error))?
    }

    async fn set_lane_visibility_and_maybe_focus_once(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
        visibility: BrowserVisibility,
        focus: bool,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(platform_drain_in_progress_error());
        }
        if user_id.trim().is_empty() {
            return Err(foreground_operation_not_allowed(lane_id.clone()));
        }
        let lane = self
            .inner
            .lanes
            .read()
            .await
            .get(lane_id)
            .cloned()
            .ok_or_else(|| BrowserPlatformError::lane_not_found(lane_id.clone()))?;
        let _activity_guard = tokio::select! {
            guard = lane.activity_gate.read() => guard,
            _ = lane.cancellation.cancelled() => return Err(lane_closed_error(lane_id.clone())),
        };
        let _visibility_guard = self.inner.primary_visibility_gate.lock().await;
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(platform_drain_in_progress_error());
        }
        let (host_key, authorized_epoch) = {
            let snapshot = lane.snapshot.read().await;
            if snapshot.caller.user_id != user_id {
                return Err(foreground_operation_not_allowed(lane_id.clone()));
            }
            if lane.closing.load(Ordering::Acquire) {
                return Err(lane_closed_error(lane_id.clone()));
            }
            if snapshot.identity_mode != BrowserIdentityMode::Primary {
                return Err(foreground_needs_primary_identity_error(lane_id));
            }
            if snapshot.lifecycle_state != LaneLifecycleState::Running {
                return Err(foreground_lane_not_ready_error(lane_id.clone()));
            }
            (
                HostKey::for_lane(
                    snapshot.identity_mode,
                    snapshot.identity_generation,
                    &snapshot.lane_id,
                ),
                snapshot.browser_epoch,
            )
        };
        let slot = self
            .inner
            .host_slots
            .read()
            .await
            .get(&host_key)
            .cloned()
            .ok_or_else(|| foreground_lane_not_ready_error(lane_id.clone()))?;
        let desired_headful = visibility.is_headful();
        let transitioned = slot.is_headful() != desired_headful;
        if transitioned {
            self.transition_primary_visibility_locked(
                &host_key,
                authorized_epoch,
                desired_headful,
            )
            .await
            .map_err(|error| error.for_lane(lane_id.clone()))?;
        }

        if focus {
            let driver = lane
                .driver
                .read()
                .await
                .clone()
                .ok_or_else(|| foreground_lane_not_ready_error(lane_id.clone()))?;
            tokio::select! {
                result = driver.bring_to_front() => result,
                _ = lane.cancellation.cancelled() => Err(lane_closed_error(lane_id.clone())),
            }
            .map_err(|error| error.for_lane(lane_id.clone()))?;
        }
        let snapshot = {
            let mut snapshot = lane.snapshot.write().await;
            if lane.closing.load(Ordering::Acquire) {
                return Err(lane_closed_error(lane_id.clone()));
            }
            if snapshot.caller.user_id != user_id
                || snapshot.identity_mode != BrowserIdentityMode::Primary
                || snapshot.lifecycle_state != LaneLifecycleState::Running
            {
                return Err(foreground_lane_not_ready_error(lane_id.clone()));
            }
            snapshot.last_active_at_ms = self.inner.clock.now_ms();
            // The intentional replacement above completed for this exact Lane.
            // Its restart marker is not an error the user can act on; clearing
            // it keeps the requesting Lane usable while
            // `fresh_observe_required` still fences stale page state. A pure
            // focus without a transition must not clear a genuine crash marker.
            if transitioned {
                snapshot.error_code = None;
                snapshot.error_message = None;
            }
            snapshot.clone()
        };
        if focus {
            self.emit("lane_foregrounded", Some(&snapshot));
        } else {
            self.emit("lane_visibility_changed", Some(&snapshot));
        }
        Ok(snapshot)
    }

    async fn transition_primary_visibility_locked(
        &self,
        host_key: &HostKey,
        observed_epoch: u64,
        desired_headful: bool,
    ) -> Result<(), BrowserPlatformError> {
        self.mark_host_restarting(host_key, observed_epoch).await;
        let hub = self.clone();
        let restart_key = host_key.clone();
        let flight = self
            .inner
            .host_restarts
            .run_bounded(
                host_key.clone(),
                observed_epoch,
                HOST_RESTART_ATTEMPT_TIMEOUT,
                move || async move {
                    hub.restart_host_once_with_visibility(
                        restart_key,
                        observed_epoch,
                        Some(desired_headful),
                    )
                    .await
                },
            )
            .await;
        if let Err(error) = flight.result {
            self.mark_host_recovery_failed(host_key, observed_epoch, &error)
                .await;
            return Err(error);
        }
        let actual = self
            .inner
            .host_slots
            .read()
            .await
            .get(host_key)
            .cloned()
            .ok_or_else(|| visibility_transition_not_applied_error(desired_headful))?;
        if actual.is_headful() != desired_headful {
            return Err(visibility_transition_not_applied_error(desired_headful));
        }
        Ok(())
    }

    /// Brings one authenticated user's running Primary Lane to the foreground.
    ///
    /// Headless Primary Hosts are first replaced through the same symmetric
    /// visibility transition used by browser management.
    pub async fn foreground_lane_for_user(
        &self,
        user_id: &str,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        let hub = self.clone();
        let user_id = user_id.to_owned();
        let lane_id = lane_id.clone();
        tokio::spawn(async move {
            hub.set_lane_visibility_and_maybe_focus_once(
                &user_id,
                &lane_id,
                BrowserVisibility::Headful,
                true,
            )
            .await
        })
        .await
        .map_err(|error| visibility_task_failed_error("foreground", &error))?
    }

    pub async fn list_lanes(&self) -> Vec<BrowserLaneSnapshot> {
        let lanes: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut snapshots = Vec::with_capacity(lanes.len());
        for lane in lanes {
            let mut snapshot = lane.current_snapshot().await;
            self.refresh_queue_metadata(&mut snapshot);
            snapshots.push(snapshot);
        }
        snapshots.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.lane_id.cmp(&b.lane_id))
        });
        snapshots
    }

    pub async fn list_lanes_for(
        &self,
        caller: &CallerIdentity,
    ) -> Result<Vec<BrowserLaneSnapshot>, BrowserPlatformError> {
        self.require_operation(caller, BrowserOperationKind::Manage)?;
        Ok(self
            .list_lanes()
            .await
            .into_iter()
            .filter(|lane| {
                lane.caller.user_id == caller.user_id
                    && lane.caller.runtime_instance_id == caller.runtime_instance_id
                    && lane.caller.owner_lease_id == caller.owner_lease_id
            })
            .collect())
    }

    pub async fn lane_snapshot(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.require_operation(caller, BrowserOperationKind::Manage)?;
        let lane = self.authorized_lane(caller, lane_id).await?;
        let mut snapshot = lane.current_snapshot().await;
        self.refresh_queue_metadata(&mut snapshot);
        Ok(snapshot)
    }

    fn refresh_queue_metadata(&self, snapshot: &mut BrowserLaneSnapshot) {
        if let Some(queue) = snapshot.queue.as_ref() {
            snapshot.queue = self.inner.scheduler.metadata(&queue.request_id).ok();
        }
    }

    #[cfg(test)]
    async fn lane_snapshot_unchecked(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Option<BrowserLaneSnapshot> {
        let lane = self.inner.lanes.read().await.get(lane_id).cloned()?;
        let mut snapshot = lane.current_snapshot().await;
        self.refresh_queue_metadata(&mut snapshot);
        Some(snapshot)
    }

    pub async fn overview(&self) -> BrowserOverview {
        let lanes = self.list_lanes().await;
        self.overview_for_lanes(lanes, None).await
    }

    /// Returns management inventory counts and Host attribution scoped to one
    /// authenticated user while retaining global capacity/pressure telemetry.
    ///
    /// Capacity is intentionally system-wide because callers need truthful
    /// queue pressure. Lane and Host lane-count fields are user-scoped so the
    /// management surface never reveals another user's inventory.
    pub async fn overview_for_user(&self, user_id: &str) -> BrowserOverview {
        let lanes = self
            .list_lanes()
            .await
            .into_iter()
            .filter(|lane| lane.caller.user_id == user_id)
            .collect();
        self.overview_for_lanes(lanes, Some(user_id)).await
    }

    async fn overview_for_lanes(
        &self,
        lanes: Vec<BrowserLaneSnapshot>,
        user_scope: Option<&str>,
    ) -> BrowserOverview {
        let include_empty_hosts = user_scope.is_none();
        let running = lanes
            .iter()
            .filter(|lane| lane.lifecycle_state == LaneLifecycleState::Running)
            .count();
        let queued = lanes
            .iter()
            .filter(|lane| lane.lifecycle_state == LaneLifecycleState::Queued)
            .count();
        let config = self.inner.config.read().await.clone();
        let telemetry = self.inner.telemetry.read().await.clone();
        let decision = self
            .decide_resources(&config.resource_policy, &telemetry)
            .await;
        self.inner
            .scheduler
            .update_recommended_concurrency(decision.recommended_concurrency);
        let slots: Vec<_> = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect();
        let mut hosts = Vec::new();
        for (key, slot) in slots {
            if let Some(host) = slot.get() {
                let lane_count = lanes
                    .iter()
                    .filter(|lane| {
                        HostKey::for_lane(
                            lane.identity_mode,
                            lane.identity_generation,
                            &lane.lane_id,
                        ) == key
                            && lane.browser_epoch == slot.epoch
                            && lane.lifecycle_state == LaneLifecycleState::Running
                    })
                    .count();
                if lane_count == 0 && !include_empty_hosts {
                    continue;
                }
                hosts.push(BrowserHostSnapshot {
                    host_id: host.host_id(),
                    state: host.state(),
                    epoch: slot.epoch,
                    headful: slot.is_headful(),
                    identity_mode: key.identity_mode,
                    lane_count,
                    rss_bytes: host.process_id().and_then(|process_id| {
                        telemetry
                            .host_rss_by_process_id
                            .get(&process_id)
                            .copied()
                    }),
                });
            }
        }
        let remaining = self.remaining_resources().await;
        let (managed_host_count, pending_cleanup_count) = if let Some(user_id) = user_scope {
            (
                hosts.len(),
                self.pending_cleanup_count_for_user(user_id).await,
            )
        } else {
            (remaining.managed_host_count, remaining.cleanup_count)
        };
        BrowserOverview {
            supported: true,
            enabled: !self.inner.shutting_down.load(Ordering::Acquire),
            running_lanes: running,
            queued_lanes: queued,
            total_lanes: lanes.len(),
            managed_host_count,
            pending_cleanup_count,
            pressure_state: decision.state,
            capacity: BrowserCapacitySnapshot {
                active: self.inner.scheduler.active_count(),
                queued: self.inner.scheduler.queued_count(),
                max_active: config.resource_policy.max_active_operations,
                max_open_lanes: config.resource_policy.max_open_lanes,
                recommended_concurrency: decision.recommended_concurrency,
                reason_code: decision.reason_code.map(str::to_owned),
            },
            hosts,
            updated_at_ms: self.inner.clock.now_ms(),
        }
    }

    pub async fn close_lane(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let Some(detached) = self.detach_lane_for_close(lane_id).await else {
            return Ok(Self::scoped_close_result(0, true));
        };
        self.promote_released_capacity().await;
        if let Some(cleanup_id) = detached.cleanup_id {
            if let Err(error) = self.attempt_pending_lane_cleanup(cleanup_id).await {
                let cleanup_still_running = error
                    .metadata
                    .get("cleanup_wait_timeout")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if !cleanup_still_running {
                    match self
                        .retire_empty_host_authoritatively(
                            &detached.host_key,
                            detached.browser_epoch,
                        )
                        .await
                    {
                        Ok(true) => return Ok(Self::scoped_close_result(1, false)),
                        Ok(false) => {}
                        Err(host_error) => {
                            return Err(Self::scoped_cleanup_error(host_error, 1));
                        }
                    }
                }
                return Err(Self::scoped_cleanup_error(error, 1));
            }
        }
        if let Err(error) = self.finalize_detached_host(detached).await {
            return Err(Self::scoped_cleanup_error(error, 1));
        }
        Ok(Self::scoped_close_result(1, false))
    }

    /// Detach one Lane from inventory and scheduler capacity before any driver
    /// I/O. The driver is first copied into the Hub-owned retry queue, so caller
    /// cancellation cannot lose cleanup authority after detachment.
    async fn detach_lane_for_close(&self, lane_id: &BrowserLaneId) -> Option<DetachedLane> {
        // Lane removal and pending-start retirement registration must be
        // atomic under the same gate that `finalize_empty_host` holds while it
        // decides a Host is empty. Without this, a finalization racing between
        // "Lane removed" and "pending start registered" can retire the Host
        // while its late `open_lane` driver is still in flight.
        let _open_guard = self.inner.open_gate.lock().await;
        self.detach_lane_for_close_locked(lane_id).await
    }

    /// Gate-held detachment core. The caller must hold `open_gate`; this keeps
    /// Lane removal, pending Lane-start retirement registration and Host
    /// retirement authority publication in one critical section.
    /// `abandon_unclaimed_lane_start` shares this exact lock semantics.
    async fn detach_lane_for_close_locked(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Option<DetachedLane> {
        let lane = self.inner.lanes.read().await.get(lane_id).cloned()?;
        let _close_guard = lane.close_gate.lock().await;
        if !self
            .inner
            .lanes
            .read()
            .await
            .get(lane_id)
            .is_some_and(|current| Arc::ptr_eq(current, &lane))
        {
            return None;
        }

        lane.closing.store(true, Ordering::Release);
        lane.cancellation.cancel();
        let snapshot = {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.lifecycle_state = LaneLifecycleState::Stopping;
            snapshot.clone()
        };
        let host_key = HostKey::for_lane(
            snapshot.identity_mode,
            snapshot.identity_generation,
            &snapshot.lane_id,
        );
        // Keep the start flight alive independently of the removed Lane. If a
        // caller closes a Lane while Host.open_lane is still in flight, the
        // late driver must be cleaned before the last Host is retired.
        let start_flight = lane.start_flight.lock().await.clone();
        self.emit("lane_stopping", Some(&snapshot));

        // Acquire every async lock before mutating any authoritative structure.
        // Once this block starts there is no cancellation point until the
        // driver is retained and both inventory indexes/capacity are detached.
        let cleanup_id = {
            let mut lanes = self.inner.lanes.write().await;
            if !lanes
                .get(lane_id)
                .is_some_and(|current| Arc::ptr_eq(current, &lane))
            {
                return None;
            }
            let mut keys = self.inner.lane_keys.write().await;
            let mut driver = lane.driver.write().await;
            let mut pending = self.inner.pending_lane_cleanups.lock().await;
            let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
            owner_targets
                .entry(snapshot.caller.owner_lease_id.clone())
                .or_default()
                .insert(OwnerCleanupTarget {
                    user_id: snapshot.caller.user_id.clone(),
                    host_key: host_key.clone(),
                    browser_epoch: snapshot.browser_epoch,
                });
            let cleanup_id = driver.take().map(|driver| {
                let cleanup_id =
                    self.inner.cleanup_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                pending.push(Arc::new(PendingLaneCleanup {
                    cleanup_id,
                    lane_id: lane_id.clone(),
                    user_id: snapshot.caller.user_id.clone(),
                    owner_lease_id: snapshot.caller.owner_lease_id.clone(),
                    host_key: host_key.clone(),
                    browser_epoch: snapshot.browser_epoch,
                    driver,
                    flight: Mutex::new(None),
                }));
                cleanup_id
            });
            lanes.remove(lane_id);
            if keys.get(&snapshot.lane_key) == Some(lane_id) {
                keys.remove(&snapshot.lane_key);
            }
            self.inner.scheduler.cancel_lane(lane_id);
            self.inner.scheduler.release_without_promotion(lane_id);
            cleanup_id
        };
        let pending_start_flight = start_flight
            .as_ref()
            .filter(|flight| flight.result.get().is_none())
            .cloned();
        if let Some(start_flight) = pending_start_flight {
            self.inner
                .pending_host_retirements
                .lock()
                .await
                .push(PendingHostRetirement {
                    key: host_key.clone(),
                    lane_id: lane_id.clone(),
                    user_id: snapshot.caller.user_id.clone(),
                    owner_lease_id: snapshot.caller.owner_lease_id.clone(),
                    start_flight,
                });
        }
        self.emit("lane_closed", Some(&snapshot));
        Some(DetachedLane {
            host_key,
            browser_epoch: snapshot.browser_epoch,
            cleanup_id,
        })
    }

    async fn attempt_pending_lane_cleanup(
        &self,
        cleanup_id: u64,
    ) -> Result<(), BrowserPlatformError> {
        self.attempt_pending_lane_cleanup_until(
            cleanup_id,
            Instant::now() + LANE_CLEANUP_WAITER_TIMEOUT,
        )
        .await
    }

    async fn attempt_pending_lane_cleanup_until(
        &self,
        cleanup_id: u64,
        wait_deadline: Instant,
    ) -> Result<(), BrowserPlatformError> {
        let entry = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .find(|entry| entry.cleanup_id == cleanup_id)
            .cloned();
        let Some(entry) = entry else {
            return Ok(());
        };
        let flight = {
            let mut active = entry.flight.lock().await;
            if let Some(flight) = active.clone() {
                flight
            } else {
                let flight = Arc::new(LaneCleanupFlight::new());
                *active = Some(Arc::clone(&flight));
                let hub = self.clone();
                let entry_for_task = Arc::clone(&entry);
                let flight_for_task = Arc::clone(&flight);
                tokio::spawn(async move {
                    let result = hub.run_pending_lane_cleanup(Arc::clone(&entry_for_task)).await;
                    // Publish completion before clearing a failed flight. A
                    // new waiter must never observe an empty slot while the
                    // previous close result is still unpublished and start a
                    // second close against the same target.
                    flight_for_task.complete(result.clone());
                    if result.is_err() {
                        let mut active = entry_for_task.flight.lock().await;
                        if active
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &flight_for_task))
                        {
                            *active = None;
                        }
                    }
                });
                flight
            }
        };
        match tokio::time::timeout_at(wait_deadline, flight.wait()).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    lane_id = %entry.lane_id,
                    timeout_ms = LANE_CLEANUP_WAITER_TIMEOUT.as_millis() as u64,
                    "browser Lane cleanup is still running after caller wait timeout"
                );
                self.emit("lane_cleanup_pending", None);
                Err(lane_cleanup_wait_timeout_error(entry.lane_id.clone()))
            }
        }
    }

    async fn run_pending_lane_cleanup(
        &self,
        entry: Arc<PendingLaneCleanup>,
    ) -> Result<(), BrowserPlatformError> {
        // The driver close belongs to the Hub, not to any caller waiter.
        // Isolate driver panics in an inner task, but do not abort a slow close:
        // caller and lifecycle waits are bounded by
        // `attempt_pending_lane_cleanup_until`, while this single-flight remains
        // the authoritative cleanup until the driver actually finishes. Starting
        // a second close against the same target after a waiter timeout would
        // violate cleanup ownership and can race the still-running first close.
        let driver = Arc::clone(&entry.driver);
        let result = tokio::spawn(async move { driver.close().await }).await;
        match result {
            Ok(Ok(())) => {
                let host_key = entry.host_key.clone();
                self.inner
                    .pending_lane_cleanups
                    .lock()
                    .await
                    .retain(|pending| !Arc::ptr_eq(pending, &entry));
                self.emit("lane_cleanup_finished", None);
                let hub = self.clone();
                tokio::spawn(async move {
                    hub.finalize_hosts_ready_after_cleanup().await;
                });
                // This task is Hub-owned. Even when the explicit close caller
                // is cancelled after target cleanup, the final Host retirement
                // obligation remains live and can be retried by sweep. It
                // joins the same per-Host finalization flight as explicit
                // close, so it can never consume a failure that an explicit
                // caller still has to observe.
                let hub = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = hub.finalize_host_once(host_key.clone()).await {
                        tracing::warn!(
                            identity_mode = ?host_key.identity_mode,
                            code = ?error.code,
                            "browser Host finalization remains pending after Lane cleanup"
                        );
                    }
                });
                Ok(())
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    lane_id = %entry.lane_id,
                    code = ?error.code,
                    "browser Lane cleanup failed; retained for lifecycle retry"
                );
                self.emit("lane_cleanup_pending", None);
                Err(error
                    .for_lane(entry.lane_id.clone())
                    .with_metadata(json!({ "cleanup_pending": true })))
            }
            Err(join_error) => {
                tracing::warn!(
                    lane_id = %entry.lane_id,
                    cancelled = join_error.is_cancelled(),
                    panic = join_error.is_panic(),
                    "browser Lane cleanup task terminated unexpectedly; retained for lifecycle retry"
                );
                self.emit("lane_cleanup_pending", None);
                Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "The browser lane was closed, but target cleanup did not complete.",
                    true,
                    "Retry cleanup through the lifecycle worker.",
                )
                .for_lane(entry.lane_id.clone())
                .with_metadata(json!({
                    "cleanup_pending": true,
                    "cleanup_task_failed": true,
                    "task_cancelled": join_error.is_cancelled(),
                    "task_panicked": join_error.is_panic(),
                })))
            }
        }
    }

    async fn retry_pending_lane_cleanups(&self) -> Result<(), BrowserPlatformError> {
        let _retry_guard = self.inner.lane_cleanup_retry_gate.lock().await;
        let cleanup_ids = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .map(|entry| entry.cleanup_id)
            .collect::<Vec<_>>();
        self.retry_pending_lane_cleanup_ids(cleanup_ids).await
    }

    async fn retry_pending_lane_cleanups_for_owner(
        &self,
        owner_lease_id: &OwnerLeaseId,
    ) -> Result<(), BrowserPlatformError> {
        let _retry_guard = self.inner.lane_cleanup_retry_gate.lock().await;
        let cleanup_ids = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .filter(|entry| &entry.owner_lease_id == owner_lease_id)
            .map(|entry| entry.cleanup_id)
            .collect::<Vec<_>>();
        self.retry_pending_lane_cleanup_ids(cleanup_ids).await
    }

    async fn retry_pending_lane_cleanup_ids(
        &self,
        cleanup_ids: Vec<u64>,
    ) -> Result<(), BrowserPlatformError> {
        let deadline = Instant::now() + CLEANUP_BATCH_WAIT_TIMEOUT;
        let mut attempts = tokio::task::JoinSet::new();
        for cleanup_id in cleanup_ids {
            let hub = self.clone();
            attempts.spawn(async move {
                hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                    .await
            });
        }
        let mut first_error = None;
        while let Some(result) = attempts.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(join_error) => {
                    if first_error.is_none() {
                        first_error = Some(cleanup_batch_task_failed_error(
                            "lane",
                            &join_error,
                        ));
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn promote_released_capacity(&self) {
        loop {
            let promoted = {
                // Serialize a fresh workload/telemetry decision with both new
                // admissions and other promotion attempts. The guard is
                // released before any Host or Chromium I/O.
                let _open_guard = self.inner.open_gate.lock().await;
                if self.inner.shutting_down.load(Ordering::Acquire) {
                    return;
                }
                if self.inner.draining.load(Ordering::Acquire) {
                    return;
                }
                let policy = self.inner.config.read().await.resource_policy.clone();
                let telemetry = self.inner.telemetry.read().await.clone();
                let decision = self.decide_resources(&policy, &telemetry).await;
                self.inner
                    .scheduler
                    .update_recommended_concurrency(decision.recommended_concurrency);
                self.inner
                    .scheduler
                    .promote_one_with_policy(&Self::promotion_policy(&decision))
            };
            let Some(request) = promoted else {
                return;
            };
            if self.inner.shutting_down.load(Ordering::Acquire) {
                self.inner
                    .scheduler
                    .release_without_promotion(&request.lane_id);
                return;
            }
            let lane = self
                .inner
                .lanes
                .read()
                .await
                .get(&request.lane_id)
                .cloned();
            let Some(lane) = lane else {
                self.inner
                    .scheduler
                    .release_without_promotion(&request.lane_id);
                continue;
            };
            if let Err(error) = self.validate_lane_owner(&lane).await {
                tracing::warn!(
                    lane_id = %request.lane_id,
                    code = ?error.code,
                    "discarding queued browser lane whose owner is no longer valid"
                );
                self.discard_lane_after_start_failure(&request.lane_id).await;
                continue;
            }
            lane.start_claimed.store(true, Ordering::Release);
            let flight = self
                .ensure_lane_start_flight(request.lane_id.clone(), Arc::clone(&lane))
                .await;
            if let Err(error) = flight.wait().await {
                tracing::warn!(
                    lane_id = %request.lane_id,
                    code = ?error.code,
                    "queued browser lane failed to start"
                );
            }
        }
    }

    pub async fn close_runtime(
        &self,
        runtime_instance_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.close_matching(|lane| lane.caller.runtime_instance_id == runtime_instance_id)
            .await
    }

    pub async fn close_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.close_matching(|lane| lane.conversation_id() == Some(conversation_id))
            .await
    }

    pub async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
        let hub = self.clone();
        tokio::spawn(async move { hub.close_all_once().await })
            .await
            .map_err(|error| drain_task_failed_error(&error))?
    }

    async fn close_all_once(&self) -> Result<CloseResult, BrowserPlatformError> {
        let _drain_gate = self.inner.drain_gate.lock().await;
        {
            let _open_guard = self.inner.open_gate.lock().await;
            self.inner.draining.store(true, Ordering::Release);
        }
        let _drain_state = HubDrainGuard {
            inner: Arc::clone(&self.inner),
        };
        let initial = self.remaining_resources().await;
        let already_closed = initial == RemainingResources::default();
        let mut first_error = None;
        let closed = match self.close_matching(|_| true).await {
            Ok(result) => result.closed,
            Err(error) => {
                let detached_closed = error
                    .metadata
                    .get("detached_closed")
                    .and_then(serde_json::Value::as_u64)
                    .map(|count| count as usize)
                    .unwrap_or(0);
                first_error = Some(error);
                detached_closed
            }
        };

        let pending_start_keys = self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .map(|pending| pending.key.clone())
            .collect::<HashSet<_>>();
        for key in pending_start_keys {
            if let Err(error) = self.wait_for_pending_host_starts(&key).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Err(error) = self.retry_pending_lane_cleanups().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        self.finalize_hosts_ready_after_cleanup().await;

        self.retire_all_active_host_slots().await;
        if let Err(error) = self.retry_retiring_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = self.retry_orphaned_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        // Host shutdown is an authoritative target-disappearance proof. The
        // retry methods clear completed failed target cleanups for stopped
        // epochs; a genuinely in-flight close remains single-flight here.
        if let Err(error) = self.retry_pending_lane_cleanups().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        self.finalize_hosts_ready_after_cleanup().await;
        if let Err(error) = self.retry_retiring_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = self.retry_orphaned_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        self.reconcile_retiring_host_keys().await;

        let remaining = self.remaining_resources().await;
        if remaining == RemainingResources::default() {
            self.inner.owner_cleanup_targets.lock().await.clear();
            return Ok(self.close_result(closed, already_closed).await);
        }
        let error = first_error.unwrap_or_else(close_all_incomplete_error);
        Err(self.cleanup_error_with_remaining(error, closed).await)
    }

    async fn retire_all_active_host_slots(&self) {
        let _open_guard = self.inner.open_gate.lock().await;
        let mut retiring_keys = self.inner.retiring_host_keys.write().await;
        let mut active = self.inner.host_slots.write().await;
        let mut retiring = self.inner.retiring_host_slots.lock().await;
        for (key, slot) in active.drain() {
            slot.retire();
            retiring_keys.insert(key.clone());
            if !retiring.iter().any(|(pending_key, pending_slot)| {
                pending_key == &key && Arc::ptr_eq(pending_slot, &slot)
            }) {
                retiring.push((key, slot));
            }
        }
        for (key, _) in self.inner.orphaned_host_slots.lock().await.iter() {
            retiring_keys.insert(key.clone());
        }
        drop(retiring);
        drop(active);
        drop(retiring_keys);
        drop(_open_guard);
        self.inner.host_empty_since_ms.write().await.clear();
    }

    async fn reconcile_retiring_host_keys(&self) {
        let mut keys = self.inner.retiring_host_keys.write().await;
        let retiring = self.inner.retiring_host_slots.lock().await;
        let orphaned = self.inner.orphaned_host_slots.lock().await;
        keys.retain(|key| {
            retiring
                .iter()
                .any(|(pending_key, _)| pending_key == key)
                || orphaned
                    .iter()
                    .any(|(pending_key, _)| pending_key == key)
        });
        drop(orphaned);
        drop(retiring);
        drop(keys);
        self.inner.retiring_hosts_changed.notify_waiters();
    }

    async fn close_matching(
        &self,
        predicate: impl Fn(&BrowserLaneSnapshot) -> bool,
    ) -> Result<CloseResult, BrowserPlatformError> {
        let ids: Vec<_> = self
            .list_lanes()
            .await
            .into_iter()
            .filter(predicate)
            .map(|lane| lane.lane_id)
            .collect();
        let mut closed = 0;
        let mut detached_lanes = Vec::new();
        for lane_id in &ids {
            if let Some(detached) = self.detach_lane_for_close(lane_id).await {
                closed += 1;
                detached_lanes.push(detached);
            }
        }
        self.promote_released_capacity().await;
        let deadline = Instant::now() + CLEANUP_BATCH_WAIT_TIMEOUT;
        let mut attempts = tokio::task::JoinSet::new();
        for detached in detached_lanes
            .iter()
            .filter(|detached| detached.cleanup_id.is_some())
        {
            let hub = self.clone();
            let cleanup_id = detached.cleanup_id.expect("filtered cleanup id");
            let host_key = detached.host_key.clone();
            let browser_epoch = detached.browser_epoch;
            attempts.spawn(async move {
                (
                    host_key,
                    browser_epoch,
                    hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                        .await,
                )
            });
        }
        let mut first_error = None;
        let mut terminal_cleanup_errors =
            HashMap::<(HostKey, u64), BrowserPlatformError>::new();
        let mut running_cleanup_targets = HashSet::<(HostKey, u64)>::new();
        let mut unknown_cleanup_task_failure = false;
        while let Some(attempt) = attempts.join_next().await {
            match attempt {
                Ok((_, _, Ok(()))) => {}
                Ok((host_key, browser_epoch, Err(error))) => {
                    let target = (host_key, browser_epoch);
                    if error
                        .metadata
                        .get("cleanup_wait_timeout")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        running_cleanup_targets.insert(target);
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    } else {
                        terminal_cleanup_errors.entry(target).or_insert(error);
                    }
                }
                Err(join_error) => {
                    unknown_cleanup_task_failure = true;
                    if first_error.is_none() {
                        first_error = Some(cleanup_batch_task_failed_error(
                            "lane",
                            &join_error,
                        ));
                    }
                }
            }
        }
        let host_targets = detached_lanes
            .into_iter()
            .map(|detached| (detached.host_key, detached.browser_epoch))
            .collect::<HashSet<_>>();
        for (host_key, browser_epoch) in host_targets {
            let target = (host_key.clone(), browser_epoch);
            if let Some(cleanup_error) = terminal_cleanup_errors.remove(&target) {
                if unknown_cleanup_task_failure || running_cleanup_targets.contains(&target) {
                    if first_error.is_none() {
                        first_error = Some(cleanup_error);
                    }
                    continue;
                }
                match self
                    .retire_empty_host_authoritatively(&host_key, browser_epoch)
                    .await
                {
                    Ok(true) => continue,
                    Ok(false) => {
                        if first_error.is_none() {
                            first_error = Some(cleanup_error);
                        }
                    }
                    Err(error) => {
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
                continue;
            }
            if unknown_cleanup_task_failure || running_cleanup_targets.contains(&target) {
                continue;
            }
            if let Err(error) = self.wait_for_pending_host_starts(&host_key).await {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
            if let Err(error) = self.finalize_host_once(host_key).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(Self::scoped_cleanup_error(error, closed));
        }
        Ok(Self::scoped_close_result(closed, closed == 0))
    }

    async fn finalize_detached_host(
        &self,
        detached: DetachedLane,
    ) -> Result<(), BrowserPlatformError> {
        self.wait_for_pending_host_starts(&detached.host_key).await?;
        self.finalize_host_once(detached.host_key).await
    }

    /// Stops every retained HostSlot for an empty key even when target cleanup
    /// itself completed with an error.
    ///
    /// A successful process shutdown is the stronger cleanup proof: production
    /// Lane drivers only keep weak Host references, so no target can remain
    /// live after every process for the key is gone. The emptiness check and
    /// active-to-retiring hand-off occur under `open_gate`; a shared Primary
    /// Host with any sibling Lane is therefore never selected.
    async fn retire_empty_host_authoritatively(
        &self,
        key: &HostKey,
        browser_epoch: u64,
    ) -> Result<bool, BrowserPlatformError> {
        let slots = {
            let _open_guard = self.inner.open_gate.lock().await;
            if self.host_has_unsettled_lane_start(key).await {
                return Ok(false);
            }
            for lane in self
                .inner
                .lanes
                .read()
                .await
                .values()
                .cloned()
                .collect::<Vec<_>>()
            {
                let snapshot = lane.snapshot.read().await;
                if snapshot.browser_epoch == browser_epoch
                    && HostKey::for_lane(
                        snapshot.identity_mode,
                        snapshot.identity_generation,
                        &snapshot.lane_id,
                    ) == *key
                {
                    return Ok(false);
                }
            }

            let mut retiring_keys = self.inner.retiring_host_keys.write().await;
            let mut active = self.inner.host_slots.write().await;
            let mut retiring = self.inner.retiring_host_slots.lock().await;
            if active
                .get(key)
                .is_some_and(|slot| slot.epoch == browser_epoch)
            {
                let slot = active
                    .remove(key)
                    .expect("epoch-matched active Host disappeared while write-locked");
                slot.retire();
                retiring_keys.insert(key.clone());
                if !retiring.iter().any(|(pending_key, pending_slot)| {
                    pending_key == key && Arc::ptr_eq(pending_slot, &slot)
                }) {
                    retiring.push((key.clone(), slot));
                }
            }
            let mut slots = retiring
                .iter()
                .filter(|(pending_key, slot)| {
                    pending_key == key && slot.epoch == browser_epoch
                })
                .map(|(_, slot)| (Arc::clone(slot), false))
                .collect::<Vec<_>>();
            drop(retiring);
            drop(active);
            drop(retiring_keys);
            slots.extend(
                self.inner
                    .orphaned_host_slots
                    .lock()
                    .await
                    .iter()
                    .filter(|(pending_key, slot)| {
                        pending_key == key && slot.epoch == browser_epoch
                    })
                    .map(|(_, slot)| (Arc::clone(slot), true)),
            );
            self.inner.host_empty_since_ms.write().await.remove(key);
            let mut seen = HashSet::new();
            slots.retain(|(slot, _)| seen.insert(Arc::as_ptr(slot) as usize));
            slots
        };

        let _retry_guard = self.inner.host_cleanup_retry_gate.lock().await;
        for (slot, orphaned) in slots {
            slot.shutdown_retired().await?;
            if orphaned {
                self.forget_orphaned_host_slot(key, &slot).await;
            } else {
                self.forget_retired_host_slot(key, &slot).await;
            }
            self.emit("host_shutdown_finished", None);
        }
        drop(_retry_guard);
        if !self
            .managed_host_exists_for_key_epoch(key, browser_epoch)
            .await
        {
            self.clear_completed_cleanup_authority_for_stopped_host(key, browser_epoch)
                .await;
        }
        Ok(true)
    }

    async fn managed_host_exists_for_key(&self, key: &HostKey) -> bool {
        if self.inner.host_slots.read().await.contains_key(key) {
            return true;
        }
        if self
            .inner
            .retiring_host_slots
            .lock()
            .await
            .iter()
            .any(|(pending_key, _)| pending_key == key)
        {
            return true;
        }
        self.inner
            .orphaned_host_slots
            .lock()
            .await
            .iter()
            .any(|(pending_key, _)| pending_key == key)
    }

    async fn managed_host_exists_for_key_epoch(
        &self,
        key: &HostKey,
        browser_epoch: u64,
    ) -> bool {
        if self
            .inner
            .host_slots
            .read()
            .await
            .get(key)
            .is_some_and(|slot| slot.epoch == browser_epoch)
        {
            return true;
        }
        if self
            .inner
            .retiring_host_slots
            .lock()
            .await
            .iter()
            .any(|(pending_key, slot)| {
                pending_key == key && slot.epoch == browser_epoch
            })
        {
            return true;
        }
        self.inner
            .orphaned_host_slots
            .lock()
            .await
            .iter()
            .any(|(pending_key, slot)| {
                pending_key == key && slot.epoch == browser_epoch
            })
    }

    /// Releases only completed/no-flight target authority after all processes
    /// for a Host key are proven stopped. An unfinished target close remains
    /// authoritative and is never duplicated or discarded.
    async fn clear_completed_cleanup_authority_for_stopped_host(
        &self,
        key: &HostKey,
        browser_epoch: u64,
    ) {
        let entries = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .filter(|entry| {
                &entry.host_key == key && entry.browser_epoch == browser_epoch
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut removable = HashSet::new();
        for entry in entries {
            let flight = entry.flight.lock().await.clone();
            if flight
                .as_ref()
                .is_none_or(|flight| flight.result.get().is_some())
            {
                removable.insert(entry.cleanup_id);
            }
        }
        if !removable.is_empty() {
            self.inner
                .pending_lane_cleanups
                .lock()
                .await
                .retain(|entry| !removable.contains(&entry.cleanup_id));
            self.emit("lane_cleanup_finished", None);
        }
        {
            let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
            owner_targets.retain(|_, targets| {
                targets.retain(|target| {
                    &target.host_key != key || target.browser_epoch != browser_epoch
                });
                !targets.is_empty()
            });
        }
        if !self.managed_host_exists_for_key(key).await {
            self.inner
                .host_finalizations
                .lock()
                .await
                .retain(|pending_key, flight| {
                    pending_key != key || flight.result.get().is_none()
                });
        }
    }

    /// Single-flight entry for retiring one empty Host. Explicit close, the
    /// cleanup completion callback and the lifecycle sweep all join the same
    /// Hub-owned attempt, so a transient shutdown failure cannot be consumed
    /// by one caller while another falsely reports success. The failed result
    /// is published to every waiter; retirement authority stays in
    /// `retiring_host_slots`, and only a later sweep or a fresh explicit call
    /// opens a deliberate new retry.
    async fn finalize_host_once(&self, key: HostKey) -> Result<(), BrowserPlatformError> {
        let (flight, is_runner) = {
            let mut flights = self.inner.host_finalizations.lock().await;
            match flights.get(&key) {
                // Join an in-flight attempt or a retained first failure. A
                // settled success is never replayed: the Host may have gained
                // and lost lanes since that attempt proved emptiness, so a
                // later caller needs a fresh authoritative attempt.
                Some(flight) if !matches!(flight.result.get(), Some(Ok(()))) => {
                    (Arc::clone(flight), false)
                }
                _ => {
                    let flight = Arc::new(HostFinalizationFlight::new());
                    flights.insert(key.clone(), Arc::clone(&flight));
                    (flight, true)
                }
            }
        };
        if is_runner {
            // The attempt itself is Hub-owned. Caller cancellation or a caller
            // wait timeout must not abort a shutdown that is already talking
            // to the process, and every overlapping caller must observe this
            // exact first result.
            let hub = self.clone();
            let run_key = key.clone();
            let run_flight = Arc::clone(&flight);
            tokio::spawn(async move {
                let result = hub.finalize_empty_host(run_key.clone()).await;
                let failed = result.is_err();
                // Publish the terminal result before releasing the flight
                // slot. A new caller must never observe an empty slot while
                // the previous attempt's result is still unpublished and start
                // a second shutdown whose outcome hides the first failure.
                run_flight.complete(result);
                if !failed {
                    let mut flights = hub.inner.host_finalizations.lock().await;
                    if flights
                        .get(&run_key)
                        .is_some_and(|current| Arc::ptr_eq(current, &run_flight))
                    {
                        flights.remove(&run_key);
                    }
                }
                // A failed attempt stays in the map. Every later explicit
                // caller joins the same first failure instead of silently
                // consuming the retirement with a fresh attempt; only the
                // lifecycle sweep opens a deliberate new retry and clears
                // the settled flight.
            });
        }
        match tokio::time::timeout(HOST_FINALIZATION_WAITER_TIMEOUT, flight.wait()).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    identity_mode = ?key.identity_mode,
                    timeout_ms = HOST_FINALIZATION_WAITER_TIMEOUT.as_millis() as u64,
                    "browser Host finalization is still running after caller wait timeout"
                );
                self.emit("host_shutdown_pending", None);
                Err(host_finalization_wait_timeout_error(&key))
            }
        }
    }

    async fn wait_for_pending_host_starts(
        &self,
        key: &HostKey,
    ) -> Result<(), BrowserPlatformError> {
        // Bounded wait: a Lane start that never settles must not block an
        // explicit close forever. On timeout the Hub-owned retirement record
        // is deliberately retained so the periodic sweep resumes the Host
        // retirement once the start flight finally publishes its result — and
        // the timeout is surfaced to the close caller as pending cleanup, so
        // an explicit close can never report full success while its Host is
        // still provably running.
        let wait_deadline = Instant::now() + PENDING_LANE_START_WAIT_TIMEOUT;
        let pending = self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .filter(|pending| &pending.key == key)
            .cloned()
            .collect::<Vec<_>>();
        for pending in pending {
            match tokio::time::timeout_at(wait_deadline, pending.start_flight.wait()).await {
                Ok(_) => {
                    self.inner
                        .pending_host_retirements
                        .lock()
                        .await
                        .retain(|current| {
                            current.key != pending.key || current.lane_id != pending.lane_id
                        });
                }
                Err(_) => {
                    tracing::warn!(
                        lane_id = %pending.lane_id,
                        identity_mode = ?pending.key.identity_mode,
                        timeout_ms = PENDING_LANE_START_WAIT_TIMEOUT.as_millis() as u64,
                        "browser Lane start is still unsettled after retirement wait timeout; retained for sweep"
                    );
                    self.emit("host_retirement_pending", None);
                    return Err(pending_lane_start_wait_timeout_error(
                        pending.lane_id.clone(),
                        key,
                    ));
                }
            }
        }
        Ok(())
    }

    async fn wait_for_pending_owner_starts(
        &self,
        owner_lease_id: &OwnerLeaseId,
    ) -> Result<(), BrowserPlatformError> {
        let wait_deadline = Instant::now() + PENDING_LANE_START_WAIT_TIMEOUT;
        let pending = self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .filter(|pending| &pending.owner_lease_id == owner_lease_id)
            .cloned()
            .collect::<Vec<_>>();
        for pending in pending {
            match tokio::time::timeout_at(wait_deadline, pending.start_flight.wait()).await {
                Ok(_) => {
                    self.inner
                        .pending_host_retirements
                        .lock()
                        .await
                        .retain(|current| {
                            current.owner_lease_id != pending.owner_lease_id
                                || current.key != pending.key
                                || current.lane_id != pending.lane_id
                        });
                }
                Err(_) => {
                    self.emit("host_retirement_pending", None);
                    return Err(pending_lane_start_wait_timeout_error(
                        pending.lane_id.clone(),
                        &pending.key,
                    ));
                }
            }
        }
        Ok(())
    }

    async fn host_has_pending_lane_cleanup(&self, key: &HostKey) -> bool {
        self.inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .any(|cleanup| &cleanup.host_key == key)
    }

    async fn host_has_unsettled_lane_start(&self, key: &HostKey) -> bool {
        self.inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .any(|pending| &pending.key == key && pending.start_flight.result.get().is_none())
    }

    /// Retry the Host part of a close after a late Lane-start or retained
    /// target cleanup has converged. This is a recovery path, so it never
    /// waits on unfinished work and never hides an error from the explicit
    /// close caller; failed Host shutdown remains in the existing retirement
    /// queue for the periodic lifecycle retry.
    async fn finalize_hosts_ready_after_cleanup(&self) {
        let pending_starts = self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .clone();
        let keys = pending_starts
            .iter()
            .filter(|pending| pending.start_flight.result.get().is_some())
            .map(|pending| pending.key.clone())
            .collect::<HashSet<_>>();
        self.inner
            .pending_host_retirements
            .lock()
            .await
            .retain(|pending| pending.start_flight.result.get().is_none());
        for key in keys {
            if let Err(error) = self.finalize_host_once(key.clone()).await {
                tracing::warn!(
                    identity_mode = ?key.identity_mode,
                    code = ?error.code,
                    "browser Host finalization remains pending after late Lane cleanup"
                );
            }
        }
    }

    /// Retire and shut down a Host as soon as its final Lane cleanup has
    /// completed. The active Host map is detached while holding `open_gate`,
    /// so an overlapping open cannot attach a new Lane to a process that is
    /// already being stopped. Failed shutdown remains in the durable
    /// retirement queue and is retried by the lifecycle worker.
    async fn finalize_empty_host(&self, key: HostKey) -> Result<(), BrowserPlatformError> {
        let slot = {
            let _open_guard = self.inner.open_gate.lock().await;

            // A detached target or a Lane start that has not yet published its
            // terminal result still belongs to this Host. Retiring the process
            // first would make target cleanup race a dead CDP connection and
            // could discard a driver which appears just after this check.
            if self.host_has_pending_lane_cleanup(&key).await
                || self.host_has_unsettled_lane_start(&key).await
            {
                return Ok(());
            }
            let lanes = self
                .inner
                .lanes
                .read()
                .await
                .values()
                .cloned()
                .collect::<Vec<_>>();
            for lane in lanes {
                let snapshot = lane.snapshot.read().await;
                if HostKey::for_lane(
                    snapshot.identity_mode,
                    snapshot.identity_generation,
                    &snapshot.lane_id,
                ) == key
                {
                    return Ok(());
                }
            }
            // Acquire the retirement authority and active map before mutating
            // either one. Once every lock is held, the hand-off below contains
            // no await point: cancellation therefore leaves the slot either
            // wholly active or wholly retained for retry.
            let mut retiring_keys = self.inner.retiring_host_keys.write().await;
            let mut slots = self.inner.host_slots.write().await;
            let mut retiring_slots = self.inner.retiring_host_slots.lock().await;
            let slot = if let Some(slot) = slots.remove(&key) {
                slot.retire();
                retiring_keys.insert(key.clone());
                retiring_slots.push((key.clone(), Arc::clone(&slot)));
                slot
            } else if let Some((_, slot)) = retiring_slots
                .iter()
                .find(|(pending_key, _)| pending_key == &key)
            {
                Arc::clone(slot)
            } else {
                return Ok(());
            };
            drop(retiring_slots);
            drop(slots);
            drop(retiring_keys);
            self.inner.host_empty_since_ms.write().await.remove(&key);
            slot
        };

        let result = slot.shutdown_retired().await;
        match result {
            Ok(_) => {
                self.forget_retired_host_slot(&key, &slot).await;
                self.emit("host_shutdown_finished", None);
                Ok(())
            }
            Err(error) => {
                self.emit("host_shutdown_pending", None);
                Err(error)
            }
        }
    }

    /// Atomically release the retry queue entry and its reopen fence after an
    /// exact Host shutdown proof. Cancellation before both locks are held
    /// leaves both authorities intact; after that there is no await point.
    async fn forget_retired_host_slot(&self, key: &HostKey, slot: &Arc<HostSlot>) {
        let mut retiring_keys = self.inner.retiring_host_keys.write().await;
        let mut retiring_slots = self.inner.retiring_host_slots.lock().await;
        let orphaned_slots = self.inner.orphaned_host_slots.lock().await;
        retiring_slots.retain(|(pending_key, pending_slot)| {
            pending_key != key || !Arc::ptr_eq(pending_slot, slot)
        });
        if !retiring_slots
            .iter()
            .any(|(pending_key, _)| pending_key == key)
            && !orphaned_slots
                .iter()
                .any(|(pending_key, _)| pending_key == key)
        {
            retiring_keys.remove(key);
        }
        drop(orphaned_slots);
        drop(retiring_slots);
        drop(retiring_keys);
        self.inner.retiring_hosts_changed.notify_waiters();
    }

    async fn forget_orphaned_host_slot(&self, key: &HostKey, slot: &Arc<HostSlot>) {
        let mut retiring_keys = self.inner.retiring_host_keys.write().await;
        let retiring_slots = self.inner.retiring_host_slots.lock().await;
        let mut orphaned_slots = self.inner.orphaned_host_slots.lock().await;
        orphaned_slots.retain(|(pending_key, pending_slot)| {
            pending_key != key || !Arc::ptr_eq(pending_slot, slot)
        });
        if !retiring_slots
            .iter()
            .any(|(pending_key, _)| pending_key == key)
            && !orphaned_slots
                .iter()
                .any(|(pending_key, _)| pending_key == key)
        {
            retiring_keys.remove(key);
        }
        drop(orphaned_slots);
        drop(retiring_slots);
        drop(retiring_keys);
        self.inner.retiring_hosts_changed.notify_waiters();
    }

    async fn owner_live_lane_count(&self, runtime_instance_id: &str) -> usize {
        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut count = 0;
        for lane in records {
            let snapshot = lane.snapshot.read().await;
            if snapshot.caller.runtime_instance_id == runtime_instance_id
                && matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Starting
                        | LaneLifecycleState::Running
                        | LaneLifecycleState::Frozen
                )
                && !lane.closing.load(Ordering::Acquire)
            {
                count += 1;
            }
        }
        count
    }

    async fn freeze_idle_lane_for_pressure(
        &self,
        lane_id: &BrowserLaneId,
        now: u64,
        idle_limit_ms: u64,
    ) -> Result<usize, BrowserPlatformError> {
        let Some(lane) = self.inner.lanes.read().await.get(lane_id).cloned() else {
            return Ok(0);
        };
        let Ok(_activity_guard) = lane.activity_gate.try_write() else {
            return Ok(0);
        };
        let snapshot = lane.current_snapshot().await;
        if lane.closing.load(Ordering::Acquire)
            || snapshot.lifecycle_state != LaneLifecycleState::Running
            || snapshot.active_operation_count != 0
            || now.saturating_sub(snapshot.last_active_at_ms) < idle_limit_ms
            || !(lane.priority == LanePriority::Expansion
                || is_crawl_identity(snapshot.identity_mode))
            || self
                .owner_live_lane_count(&snapshot.caller.runtime_instance_id)
                .await
                <= 1
        {
            return Ok(0);
        }

        let driver = lane.driver.read().await.clone();
        let outcome = match driver {
            Some(driver) => driver.freeze().await,
            None => Ok(LaneFreezeOutcome::Unsupported),
        };
        match outcome {
            Ok(LaneFreezeOutcome::Frozen) => {
                lane.frozen_at_ms.store(now, Ordering::Release);
                let frozen = {
                    let mut current = lane.snapshot.write().await;
                    current.lifecycle_state = LaneLifecycleState::Frozen;
                    current.clone()
                };
                self.emit("lane_frozen_pressure", Some(&frozen));
                Ok(0)
            }
            Ok(LaneFreezeOutcome::Unsupported) | Err(_) => {
                self.emit("lane_freeze_fallback_close", Some(&snapshot));
                Ok(self.close_lane(lane_id).await?.closed)
            }
        }
    }

    async fn close_idle_lane_if_eligible(
        &self,
        lane_id: &BrowserLaneId,
        now: u64,
        idle_limit_ms: u64,
        pressure_filter: PressureCloseFilter,
        protect_only_owner_lane: bool,
    ) -> Result<usize, BrowserPlatformError> {
        let Some(lane) = self.inner.lanes.read().await.get(lane_id).cloned() else {
            return Ok(0);
        };
        let Ok(_activity_guard) = lane.activity_gate.try_write() else {
            return Ok(0);
        };
        let snapshot = lane.current_snapshot().await;
        let lifecycle_matches = match pressure_filter {
            PressureCloseFilter::AnyIdle => matches!(
                snapshot.lifecycle_state,
                LaneLifecycleState::Running | LaneLifecycleState::Frozen
            ),
            PressureCloseFilter::FrozenExpansion => {
                snapshot.lifecycle_state == LaneLifecycleState::Frozen
                    && lane.priority == LanePriority::Expansion
            }
            PressureCloseFilter::RunningExpansion => {
                snapshot.lifecycle_state == LaneLifecycleState::Running
                    && lane.priority == LanePriority::Expansion
            }
            PressureCloseFilter::IdleCrawl => {
                matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Running | LaneLifecycleState::Frozen
                ) && is_crawl_identity(snapshot.identity_mode)
            }
        };
        if lane.closing.load(Ordering::Acquire)
            || !lifecycle_matches
            || snapshot.active_operation_count != 0
            || now.saturating_sub(snapshot.last_active_at_ms) < idle_limit_ms
        {
            return Ok(0);
        }
        if protect_only_owner_lane
            && self
                .owner_live_lane_count(&snapshot.caller.runtime_instance_id)
                .await
                <= 1
        {
            return Ok(0);
        }
        Ok(self.close_lane(lane_id).await?.closed)
    }

    async fn sweep_empty_hosts(
        &self,
        now: u64,
        crawl_warm_ms: u64,
    ) -> Result<usize, BrowserPlatformError> {
        // Lane creation inserts inventory while holding this gate before it
        // accesses a Host slot, so an empty slot can be detached without
        // racing a new Lane onto the old Host.
        let _open_guard = self.inner.open_gate.lock().await;
        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut used_keys = HashSet::new();
        for lane in records {
            let snapshot = lane.snapshot.read().await;
            used_keys.insert(HostKey::for_lane(
                snapshot.identity_mode,
                snapshot.identity_generation,
                &snapshot.lane_id,
            ));
        }

        let slot_keys: Vec<_> = self.inner.host_slots.read().await.keys().cloned().collect();
        // Mirror finalize_empty_host: a retained target cleanup or an
        // unsettled Lane start still belongs to its Host. Retiring the process
        // first would make that in-flight close/open race a dead CDP
        // connection, violating the ordering invariant that target cleanup
        // settles before the process is stopped.
        let mut blocked_keys = HashSet::new();
        for key in &slot_keys {
            if used_keys.contains(key) {
                continue;
            }
            if self.host_has_pending_lane_cleanup(key).await
                || self.host_has_unsettled_lane_start(key).await
            {
                blocked_keys.insert(key.clone());
            }
        }
        let mut ready = Vec::new();
        {
            let mut empty = self.inner.host_empty_since_ms.write().await;
            for key in &used_keys {
                empty.remove(key);
            }
            for key in slot_keys {
                if used_keys.contains(&key) || blocked_keys.contains(&key) {
                    continue;
                }
                let empty_since = *empty.entry(key.clone()).or_insert(now);
                let warm_ms = if key.identity_mode == BrowserIdentityMode::Primary {
                    0
                } else {
                    crawl_warm_ms
                };
                if now.saturating_sub(empty_since) >= warm_ms {
                    ready.push(key);
                }
            }
            for key in &ready {
                empty.remove(key);
            }
        }
        {
            // Keep the active host map authoritative until the durable
            // retirement queue is also locked. Cancellation while waiting for
            // any lock before this block leaves each HostSlot active and
            // reusable. Once all locks below are held, removing the active
            // slot, publishing the retiring key, and enqueuing cleanup
            // authority happen with no await points between them; cancellation
            // after that can only leave work in `retiring_host_slots`, which
            // retry_retiring_host_slots/shutdown will drive later.
            let mut retiring_keys = self.inner.retiring_host_keys.write().await;
            let mut slots = self.inner.host_slots.write().await;
            let mut retiring_slots = self.inner.retiring_host_slots.lock().await;
            for key in ready {
                if let Some(slot) = slots.remove(&key) {
                    slot.retire();
                    retiring_keys.insert(key.clone());
                    retiring_slots.push((key, slot));
                }
            }
        }
        drop(_open_guard);

        self.retry_retiring_host_slots().await
    }

    async fn retry_retiring_host_slots(&self) -> Result<usize, BrowserPlatformError> {
        let _retry_guard = self.inner.host_cleanup_retry_gate.lock().await;
        // This is the deliberate new retry boundary. Settled finalization
        // flights have already published their first failure to every explicit
        // caller; releasing them here lets the next explicit close start a
        // fresh attempt instead of replaying a stale result. In-flight
        // attempts are kept so a running shutdown is never duplicated.
        self.inner
            .host_finalizations
            .lock()
            .await
            .retain(|_, flight| flight.result.get().is_none());
        // Clone, do not drain: cancellation or a failed shutdown must leave the
        // authoritative slot in the retry queue.
        let slots = self.inner.retiring_host_slots.lock().await.clone();
        let mut attempts = tokio::task::JoinSet::new();
        for (key, slot) in slots {
            attempts.spawn(async move {
                let result = slot.shutdown_retired().await;
                (key, slot, result)
            });
        }
        let mut stopped = 0;
        let mut first_error = None;
        while let Some(attempt) = attempts.join_next().await {
            let (key, slot, result) = match attempt {
                Ok(result) => result,
                Err(join_error) => {
                    if first_error.is_none() {
                        first_error =
                            Some(cleanup_batch_task_failed_error("host", &join_error));
                    }
                    continue;
                }
            };
            self.emit("host_warm_shutdown_started", None);
            match result {
                Ok(had_host) => {
                    stopped += usize::from(had_host);
                    self.forget_retired_host_slot(&key, &slot).await;
                    if !self
                        .managed_host_exists_for_key_epoch(&key, slot.epoch)
                        .await
                    {
                        self.clear_completed_cleanup_authority_for_stopped_host(
                            &key,
                            slot.epoch,
                        )
                        .await;
                    }
                    self.emit("host_warm_shutdown_finished", None);
                }
                Err(error) => {
                    self.emit("host_warm_shutdown_failed", None);
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(stopped), Err)
    }

    async fn retry_orphaned_host_slots(&self) -> Result<usize, BrowserPlatformError> {
        let _retry_guard = self.inner.host_cleanup_retry_gate.lock().await;
        let slots = self.inner.orphaned_host_slots.lock().await.clone();
        let mut attempts = tokio::task::JoinSet::new();
        for (key, slot) in slots {
            attempts.spawn(async move {
                let result = slot.shutdown_retired().await;
                (key, slot, result)
            });
        }
        let mut stopped = 0;
        let mut first_error = None;
        while let Some(attempt) = attempts.join_next().await {
            let (key, slot, result) = match attempt {
                Ok(result) => result,
                Err(join_error) => {
                    if first_error.is_none() {
                        first_error =
                            Some(cleanup_batch_task_failed_error("host", &join_error));
                    }
                    continue;
                }
            };
            match result {
                Ok(had_host) => {
                    stopped += usize::from(had_host);
                    self.forget_orphaned_host_slot(&key, &slot).await;
                    if !self
                        .managed_host_exists_for_key_epoch(&key, slot.epoch)
                        .await
                    {
                        self.clear_completed_cleanup_authority_for_stopped_host(
                            &key,
                            slot.epoch,
                        )
                        .await;
                    }
                    self.emit("host_cleanup_finished", None);
                }
                Err(error) => {
                    tracing::warn!(
                        identity_mode = ?key.identity_mode,
                        code = ?error.code,
                        "browser Host cleanup failed; retained for lifecycle retry"
                    );
                    self.emit("host_cleanup_pending", None);
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(stopped), Err)
    }

    async fn attempt_orphaned_host_slot_cleanup(
        &self,
        key: &HostKey,
        slot: &Arc<HostSlot>,
    ) -> Result<bool, BrowserPlatformError> {
        let _retry_guard = self.inner.host_cleanup_retry_gate.lock().await;
        self.attempt_orphaned_host_slot_cleanup_locked(key, slot)
            .await
    }

    async fn attempt_orphaned_host_slot_cleanup_locked(
        &self,
        key: &HostKey,
        slot: &Arc<HostSlot>,
    ) -> Result<bool, BrowserPlatformError> {
        match slot.shutdown_retired().await {
            Ok(had_host) => {
                self.forget_orphaned_host_slot(key, slot).await;
                if !self
                    .managed_host_exists_for_key_epoch(key, slot.epoch)
                    .await
                {
                    self.clear_completed_cleanup_authority_for_stopped_host(
                        key,
                        slot.epoch,
                    )
                    .await;
                }
                self.emit("host_cleanup_finished", None);
                Ok(had_host)
            }
            Err(error) => {
                tracing::warn!(
                    identity_mode = ?key.identity_mode,
                    code = ?error.code,
                    "browser Host cleanup failed; retained for lifecycle retry"
                );
                self.emit("host_cleanup_pending", None);
                Err(error)
            }
        }
    }

    pub async fn set_resource_policy(
        &self,
        policy: ResourcePolicy,
    ) -> Result<(), BrowserPlatformError> {
        policy.validate().map_err(|error| {
            BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser resource policy is invalid.",
                false,
                "Correct the invalid resource-policy field and retry.",
            )
            .with_metadata(json!({
                "field": error.field,
                "reason": error.reason,
            }))
        })?;
        let operation_weight_limit = {
            let _open_guard = self.inner.open_gate.lock().await;
            let telemetry = self.inner.telemetry.read().await.clone();
            let decision = self.decide_resources(&policy, &telemetry).await;
            self.inner
                .scheduler
                .update_policy_limits_without_promotion(
                    policy.max_open_lanes,
                    policy.max_global_queue,
                    policy.max_owner_queue,
                    decision.recommended_concurrency,
                );
            self.inner.config.write().await.resource_policy = policy;
            decision.operation_weight_limit
        };
        self.apply_operation_weight_limit(operation_weight_limit).await;
        self.promote_released_capacity().await;
        self.emit("resource_policy_changed", None);
        Ok(())
    }

    pub async fn resource_policy(&self) -> ResourcePolicy {
        self.inner.config.read().await.resource_policy.clone()
    }

    pub async fn update_resource_telemetry(&self, telemetry: ResourceTelemetry) {
        self.refresh_lane_resource_estimates(&telemetry).await;
        *self.inner.telemetry.write().await = telemetry;
        let policy = self.inner.config.read().await.resource_policy.clone();
        let decision = {
            let telemetry = self.inner.telemetry.read().await.clone();
            self.decide_resources(&policy, &telemetry).await
        };
        self.inner
            .scheduler
            .update_recommended_concurrency(decision.recommended_concurrency);
        self.apply_operation_weight_limit(decision.operation_weight_limit)
            .await;
        self.promote_released_capacity().await;
        self.emit("resource_pressure_sampled", None);
    }

    async fn refresh_lane_resource_estimates(&self, telemetry: &ResourceTelemetry) {
        let policy = self.inner.config.read().await.resource_policy.clone();
        let slots: Vec<_> = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect();
        let mut rss_by_host = HashMap::new();
        for (key, slot) in slots {
            let Some(host) = slot.get() else {
                continue;
            };
            if let Some(rss) = host
                .process_id()
                .and_then(|process_id| telemetry.host_rss_by_process_id.get(&process_id))
                .copied()
            {
                rss_by_host.insert(key, rss);
            }
        }

        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut live = Vec::new();
        let mut lane_count_by_host = HashMap::<HostKey, u64>::new();
        for lane in records {
            let snapshot = lane.snapshot.read().await;
            if matches!(
                snapshot.lifecycle_state,
                LaneLifecycleState::Running | LaneLifecycleState::Frozen
            ) {
                let key = HostKey::for_lane(
                    snapshot.identity_mode,
                    snapshot.identity_generation,
                    &snapshot.lane_id,
                );
                *lane_count_by_host.entry(key.clone()).or_default() += 1;
                live.push((Arc::clone(&lane), key));
            }
        }
        for (lane, key) in live {
            let Some(host_rss) = rss_by_host.get(&key).copied() else {
                continue;
            };
            let lane_count = lane_count_by_host.get(&key).copied().unwrap_or(1).max(1);
            let sample = host_rss.saturating_add(lane_count - 1) / lane_count;
            let mut snapshot = lane.snapshot.write().await;
            snapshot.resource_estimate_bytes = crate::resource::next_lane_resource_ewma(
                snapshot.resource_estimate_bytes,
                sample,
                policy.lane_ewma_min_bytes,
                policy.lane_ewma_max_bytes,
            );
        }
    }

    /// Authoritative periodic cleanup.  The application should call this every
    /// 30 seconds and also call the explicit owner/runtime cleanup methods at
    /// their lifecycle boundaries.
    pub async fn sweep(&self) -> Result<CloseResult, BrowserPlatformError> {
        let mut closed = 0;
        let mut first_error = None;
        if let Err(error) = self.retry_pending_lane_cleanups().await {
            first_error = Some(error);
        }
        // A close whose pending Lane-start wait timed out has retained its
        // Hub-owned retirement record. Settled start flights are resolved
        // here so the retained Host retirement converges without requiring
        // another explicit close call.
        self.finalize_hosts_ready_after_cleanup().await;
        // Lane cleanup is the prerequisite for exact Host shutdown. Retry
        // retained retirements immediately afterwards; explicit close already
        // attempts this synchronously, while this path covers cancellation or
        // a previously failed Host shutdown.
        if let Err(error) = self.retry_retiring_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = self.retry_orphaned_host_slots().await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        let mut expired_owner_lease_ids = self.inner.owner_leases.sweep_expired_ids();
        for lane in self.list_lanes().await {
            if self.inner.owner_leases.validate(&lane.caller).is_err() {
                expired_owner_lease_ids.push(lane.caller.owner_lease_id);
            }
        }
        expired_owner_lease_ids.sort();
        expired_owner_lease_ids.dedup();
        for owner_lease_id in expired_owner_lease_ids {
            accumulate_close_outcome(
                self.close_owner_lease(&owner_lease_id).await,
                &mut closed,
                &mut first_error,
            );
        }
        let config = self.inner.config.read().await.clone();
        let telemetry = self.inner.telemetry.read().await.clone();
        let pressure = self
            .decide_resources(&config.resource_policy, &telemetry)
            .await
            .state;
        let now = self.inner.clock.now_ms();
        match pressure {
            ResourcePressureState::Normal => {
                let mut lanes = self.list_lanes().await;
                lanes.sort_by_key(|lane| (lane.last_active_at_ms, lane.created_at_ms));
                for lane in lanes {
                    accumulate_close_outcome(
                        self.close_idle_lane_if_eligible(
                            &lane.lane_id,
                            now,
                            config.resource_policy.idle_expiry_ms,
                            PressureCloseFilter::AnyIdle,
                            false,
                        )
                        .await
                        .map(|closed| CloseResult {
                            closed,
                            already_closed: closed == 0,
                            ..CloseResult::default()
                        }),
                        &mut closed,
                        &mut first_error,
                    );
                }
            }
            ResourcePressureState::Pressured => {
                let mut lanes = self.list_lanes().await;
                lanes.sort_by_key(|lane| (lane.last_active_at_ms, lane.created_at_ms));
                for lane in lanes {
                    accumulate_close_outcome(
                        self.freeze_idle_lane_for_pressure(
                            &lane.lane_id,
                            now,
                            config.resource_policy.pressured_idle_expiry_ms,
                        )
                        .await
                        .map(|closed| CloseResult {
                            closed,
                            already_closed: closed == 0,
                            ..CloseResult::default()
                        }),
                        &mut closed,
                        &mut first_error,
                    );
                }
            }
            ResourcePressureState::Critical => {
                // The order is contractual: reclaim oldest already-frozen
                // expansion lanes, then idle running expansion lanes which
                // may not have observed an intermediate Pressured sweep, and
                // only then touch an idle crawl lane.
                let records: Vec<_> =
                    self.inner.lanes.read().await.values().cloned().collect();
                let mut frozen_expansions = Vec::new();
                let mut running_expansions = Vec::new();
                let mut idle_crawl = Vec::new();
                for lane in records {
                    let snapshot = lane.current_snapshot().await;
                    if snapshot.lifecycle_state == LaneLifecycleState::Frozen
                        && lane.priority == LanePriority::Expansion
                    {
                        frozen_expansions.push((
                            lane.frozen_at_ms.load(Ordering::Acquire),
                            snapshot.created_at_ms,
                            snapshot.lane_id,
                        ));
                    } else if snapshot.lifecycle_state == LaneLifecycleState::Running
                        && lane.priority == LanePriority::Expansion
                    {
                        running_expansions.push((
                            snapshot.last_active_at_ms,
                            snapshot.created_at_ms,
                            snapshot.lane_id,
                        ));
                    } else if is_crawl_identity(snapshot.identity_mode) {
                        idle_crawl.push((
                            snapshot.last_active_at_ms,
                            snapshot.created_at_ms,
                            snapshot.lane_id,
                        ));
                    }
                }
                frozen_expansions.sort();
                running_expansions.sort();
                idle_crawl.sort();
                for (_, _, lane_id) in frozen_expansions {
                    accumulate_close_outcome(
                        self.close_idle_lane_if_eligible(
                            &lane_id,
                            now,
                            0,
                            PressureCloseFilter::FrozenExpansion,
                            true,
                        )
                        .await
                        .map(|closed| CloseResult {
                            closed,
                            already_closed: closed == 0,
                            ..CloseResult::default()
                        }),
                        &mut closed,
                        &mut first_error,
                    );
                }
                for (_, _, lane_id) in running_expansions {
                    accumulate_close_outcome(
                        self.close_idle_lane_if_eligible(
                            &lane_id,
                            now,
                            0,
                            PressureCloseFilter::RunningExpansion,
                            true,
                        )
                        .await
                        .map(|closed| CloseResult {
                            closed,
                            already_closed: closed == 0,
                            ..CloseResult::default()
                        }),
                        &mut closed,
                        &mut first_error,
                    );
                }
                for (_, _, lane_id) in idle_crawl {
                    accumulate_close_outcome(
                        self.close_idle_lane_if_eligible(
                            &lane_id,
                            now,
                            0,
                            PressureCloseFilter::IdleCrawl,
                            true,
                        )
                        .await
                        .map(|closed| CloseResult {
                            closed,
                            already_closed: closed == 0,
                            ..CloseResult::default()
                        }),
                        &mut closed,
                        &mut first_error,
                    );
                }
            }
        }
        if let Err(error) = self
            .sweep_empty_hosts(now, config.resource_policy.host_warm_ms)
            .await
        {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        // A close attempted during this same sweep may have entered the
        // retained queue. Retry once after all capacity decisions, without
        // letting one failed target prevent cleanup of the remaining Lanes.
        if let Err(error) = self.retry_pending_lane_cleanups().await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(self.close_result(closed, closed == 0).await)
    }

    pub async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
        let _shutdown_guard = self.inner.shutdown_gate.lock().await;
        let cached_result = { self.inner.shutdown_result.read().await.clone() };
        if let Some(result) = cached_result {
            return result;
        }
        match tokio::time::timeout(
            PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT,
            self.shutdown_once(),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                self.emit("platform_shutdown_cleanup_pending", None);
                Err(platform_shutdown_timeout_error())
            }
        }
    }

    async fn shutdown_once(&self) -> Result<(), BrowserPlatformError> {
        self.inner.shutting_down.store(true, Ordering::Release);
        // Preserve the first explicit cleanup failure for the caller, but do
        // not short-circuit the remaining Hub-owned cleanup authority. A
        // close may already have retired the last Host before returning its
        // target error; swallowing that error would falsely report exact
        // application shutdown.
        let mut first_error = self.close_matching(|_| true).await.err();
        if let Err(error) = self.retry_pending_lane_cleanups().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }

        // Move active slots into a retained queue before the first shutdown
        // await. A cancelled/failed shutdown therefore leaves every Host under
        // Hub ownership for the next explicit shutdown or lifecycle sweep.
        {
            let mut host_slots = self.inner.host_slots.write().await;
            let mut orphaned = self.inner.orphaned_host_slots.lock().await;
            orphaned.extend(host_slots.drain().map(|(key, slot)| {
                slot.retire();
                (key, slot)
            }));
        }
        self.inner.host_empty_since_ms.write().await.clear();

        if let Err(error) = self.retry_orphaned_host_slots().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = self.retry_retiring_host_slots().await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        // A successful Host shutdown may make a previously failing Lane close
        // trivially complete (the weak Host is gone), so retry once more.
        if let Err(error) = self.retry_pending_lane_cleanups().await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            self.emit("platform_shutdown_cleanup_pending", None);
            // Do not cache failure: retained queues are the authority for the
            // next shutdown call.
            return Err(error);
        }

        self.inner.retiring_host_keys.write().await.clear();
        self.inner.retiring_hosts_changed.notify_waiters();
        self.emit("platform_stopped", None);
        let result = Ok(());
        *self.inner.shutdown_result.write().await = Some(result.clone());
        result
    }

    fn emit(&self, change_kind: &str, lane: Option<&BrowserLaneSnapshot>) {
        let _ = self.inner.events.send(BrowserInventoryEvent {
            sequence: self.inner.sequence.fetch_add(1, Ordering::AcqRel) + 1,
            change_kind: change_kind.to_owned(),
            lane_id: lane.map(|snapshot| snapshot.lane_id.clone()),
            user_id: lane.map(|snapshot| snapshot.caller.user_id.clone()),
            conversation_id: lane.and_then(|snapshot| snapshot.caller.conversation_id.clone()),
            at_ms: self.inner.clock.now_ms(),
        });
    }
}

fn accumulate_close_outcome(
    result: Result<CloseResult, BrowserPlatformError>,
    closed: &mut usize,
    first_error: &mut Option<BrowserPlatformError>,
) {
    match result {
        Ok(result) => *closed += result.closed,
        Err(error) => {
            if error
                .metadata
                .get("cleanup_pending")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                *closed += error
                    .metadata
                    .get("detached_closed")
                    .and_then(serde_json::Value::as_u64)
                    .map(|count| count as usize)
                    .unwrap_or(1);
            }
            if first_error.is_none() {
                *first_error = Some(error);
            }
        }
    }
}

fn is_host_fatal_error(error: &BrowserPlatformError) -> bool {
    error.code == BrowserErrorCode::BrowserRestarted
        && error
            .metadata
            .get("failure_scope")
            .and_then(serde_json::Value::as_str)
            == Some("host")
}

fn lane_restart_notice(
    lane: &LaneRecord,
    snapshot: &BrowserLaneSnapshot,
) -> BrowserPlatformError {
    let old_epoch = lane.restart_from_epoch.load(Ordering::Acquire);
    if snapshot.browser_epoch > old_epoch
        && let Ok(transition) = HostRestartTransition::new(old_epoch, snapshot.browser_epoch)
    {
        return transition.browser_restarted_error();
    }
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserRestarted,
        "The managed browser is restarting and previous page state is no longer current.",
        true,
        "Wait for recovery, then run a fresh observe.",
    )
    .with_metadata(json!({
        "old_epoch": old_epoch,
        "new_epoch": snapshot.browser_epoch,
        "fresh_observe_required": true,
        "restart_in_progress": true,
    }))
}

fn foreground_operation_not_allowed(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "This browser lane is not available to the current user.",
        false,
        "Refresh the browser inventory and select one of your running Primary lanes.",
    )
    .for_lane(lane_id)
}

fn foreground_needs_primary_identity_error(lane_id: &BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::NeedsPrimaryIdentity,
        "Only a Primary browser lane can be brought to the foreground.",
        false,
        "Select a running Primary browser lane and retry.",
    )
    .for_lane(lane_id.clone())
}

fn foreground_lane_not_ready_error(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser lane is not ready to be brought to the foreground.",
        true,
        "Wait for the Primary lane to become running, then retry.",
    )
    .for_lane(lane_id)
}

fn visibility_transition_not_applied_error(
    desired_headful: bool,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed Primary browser did not apply the requested display mode.",
        true,
        "Refresh browser status and retry the display-mode transition.",
    )
    .with_metadata(json!({
        "visibility_transition_failed": true,
        "requested_visibility": if desired_headful { "headful" } else { "headless" },
    }))
}

fn visibility_task_failed_error(
    scope: &'static str,
    join_error: &tokio::task::JoinError,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser display transition terminated unexpectedly.",
        true,
        "Refresh browser status and retry the display-mode transition.",
    )
    .with_metadata(json!({
        "visibility_transition_failed": true,
        "scope": scope,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
    }))
}

fn platform_drain_in_progress_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser platform is closing all managed resources.",
        true,
        "Retry after browser cleanup finishes.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_drain_in_progress": true,
    }))
}

#[derive(Clone)]
pub struct BrowserLaneClient {
    hub: BrowserSessionHub,
    caller: CallerIdentity,
}

impl BrowserLaneClient {
    pub fn caller(&self) -> &CallerIdentity {
        &self.caller
    }

    pub async fn open(
        &self,
        lane_name: Option<&str>,
        identity_mode: BrowserIdentityMode,
        workspace_hint: Option<String>,
    ) -> Result<OpenLaneOutcome, BrowserPlatformError> {
        self.hub
            .open_lane(
                &self.caller,
                lane_name,
                identity_mode,
                workspace_hint,
            )
            .await
    }

    pub async fn execute(
        &self,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.hub.execute(&self.caller, lane_id, operation).await
    }

    /// Dispatch an operation carrying a trusted, one-shot out-of-band approval.
    ///
    /// Only an authenticated in-process transport should call this after it
    /// atomically consumes the approval for this caller, lane, and operation.
    /// The authority is carried out-of-band and cannot be forged in operation
    /// JSON.
    pub async fn execute_confirmed(
        &self,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        self.hub
            .execute_confirmed(&self.caller, lane_id, operation)
            .await
    }

    pub async fn list(&self) -> Result<Vec<BrowserLaneSnapshot>, BrowserPlatformError> {
        self.hub.list_lanes_for(&self.caller).await
    }

    pub async fn status(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.hub.lane_snapshot(&self.caller, lane_id).await
    }

    pub async fn close(
        &self,
        lane_id: &BrowserLaneId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.hub
            .require_operation(&self.caller, BrowserOperationKind::Manage)?;
        self.hub.authorized_lane(&self.caller, lane_id).await?;
        self.hub.close_lane(lane_id).await
    }

    pub async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
        self.hub
            .require_operation(&self.caller, BrowserOperationKind::Manage)?;
        self.hub
            .close_owner_lanes(&self.caller.owner_lease_id)
            .await
    }
}

fn outcome_for_snapshot(
    snapshot: BrowserLaneSnapshot,
) -> Result<OpenLaneOutcome, BrowserPlatformError> {
    match snapshot.lifecycle_state {
        LaneLifecycleState::Queued => Ok(OpenLaneOutcome::Queued { lane: snapshot }),
        LaneLifecycleState::Starting
        | LaneLifecycleState::Running
        | LaneLifecycleState::Frozen => Ok(OpenLaneOutcome::Running { lane: snapshot }),
        LaneLifecycleState::Stopping => Err(lane_closed_error(snapshot.lane_id)),
        LaneLifecycleState::Failed => Err(BrowserPlatformError::new(
            snapshot
                .error_code
                .unwrap_or(BrowserErrorCode::BrowserUnavailable),
            snapshot
                .error_message
                .unwrap_or_else(|| "The browser lane failed to start.".to_owned()),
            snapshot.recoverable,
            "Open the lane again to retry with a fresh managed browser target.",
        )
        .for_lane(snapshot.lane_id)),
    }
}

/// Authenticated replicas are read-only copies of canonical identity state.
///
/// `BrowserOperation::may_modify_identity` crosses transport boundaries and is
/// therefore only an additional fail-closed hint. The Hub owns the final
/// classification so a caller cannot forge `false` to dispatch an interactive,
/// script-evaluating, or otherwise unreviewed operation into a replica.
fn replica_operation_requires_primary(operation: &BrowserOperation) -> bool {
    if operation.may_modify_identity {
        return true;
    }

    match operation.kind {
        // Ordinary GET-style navigation is the supported authenticated crawl
        // path. Explicitly state-changing request shapes remain Primary-only.
        // `reload` is deliberately never treated as safe: the current
        // history entry may have been produced by a POST, and an empty reload
        // input cannot prove that replaying it is side-effect free.
        BrowserOperationKind::Navigate => {
            !matches!(
                operation.action.as_str(),
                "navigate" | "back" | "forward" | "reload"
            ) || operation.action == "reload"
                || operation_declares_stateful_request(&operation.input)
        }
        BrowserOperationKind::Crawl => {
            !matches!(
                operation.action.as_str(),
                "navigate"
                    | "observe"
                    | "get_page_text"
                    | "extract"
                    | "rendered_html"
            ) || (operation.action == "navigate"
                && operation_declares_stateful_request(&operation.input))
        }
        BrowserOperationKind::Observe => !matches!(
            operation.action.as_str(),
            "observe"
                | "get_page_text"
                | "search_page"
                | "find_elements"
                | "get_dropdown_options"
                | "cursor"
        ),
        BrowserOperationKind::Screenshot => !matches!(
            operation.action.as_str(),
            "screenshot"
        ),
        BrowserOperationKind::Tabs => !matches!(
            operation.action.as_str(),
            "tabs" | "switch_tab" | "close_tab"
        ),
        BrowserOperationKind::Download => !matches!(
            operation.action.as_str(),
            "download" | "save_as_pdf"
        ),
        BrowserOperationKind::Debug => !matches!(
            operation.action.as_str(),
            "get_console_logs"
                | "get_page_errors"
                | "get_network_log"
                | "rendered_html"
        ),
        BrowserOperationKind::Manage => !matches!(
            operation.action.as_str(),
            "capabilities" | "device_pixel_ratio"
        ),
        // Any interaction can submit forms, execute page behavior, or mutate
        // durable account state. It must run against Primary live identity.
        BrowserOperationKind::Act => true,
    }
}

/// Identity mode is also an immutable Lane purpose boundary.
///
/// Anonymous and authenticated-replica lanes exist only for bounded crawl
/// workloads.  A model may learn their owner-scoped `lane_id` from a crawl
/// result, but that handle must not turn the lane into an interactive browser
/// by selecting a different operation kind on a later call.  Cleanup and
/// management stay outside this dispatch classifier, so lifecycle authority
/// can always close or inspect a lane.
fn require_lane_operation(
    identity_mode: BrowserIdentityMode,
    operation: &BrowserOperation,
) -> Result<(), BrowserPlatformError> {
    match identity_mode {
        BrowserIdentityMode::Anonymous => {
            if anonymous_operation_not_allowed(operation) {
                return Err(crawl_lane_operation_error(identity_mode));
            }
        }
        BrowserIdentityMode::AuthenticatedReplica => {
            if replica_operation_requires_primary(operation) {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::NeedsPrimaryIdentity,
                    "This operation may change account identity and cannot run in a replica.",
                    false,
                    "Open a Primary live-identity lane and retry.",
                ));
            }
        }
        BrowserIdentityMode::Primary | BrowserIdentityMode::Isolated => {}
    }
    Ok(())
}

fn anonymous_operation_not_allowed(operation: &BrowserOperation) -> bool {
    operation.may_modify_identity || replica_operation_requires_primary(operation)
}

fn crawl_lane_operation_error(identity_mode: BrowserIdentityMode) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "This browser lane is reserved for read-only crawl operations.",
        false,
        "Use browser_crawl_many, or open a Primary interactive lane.",
    )
    .with_metadata(json!({
        "identity_mode": identity_mode,
        "required_operation": BrowserOperationKind::Crawl,
        "lane_purpose": "crawl",
    }))
}

fn operation_declares_stateful_request(input: &serde_json::Value) -> bool {
    input
        .get("method")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| {
            !method.eq_ignore_ascii_case("get") && !method.eq_ignore_ascii_case("head")
        })
        || input
            .get("submits_form")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

fn identity_operation_needs_refresh(operation: &BrowserOperation) -> bool {
    operation.may_modify_identity
        || matches!(
            (operation.kind, operation.action.as_str()),
            (BrowserOperationKind::Navigate, _)
                | (BrowserOperationKind::Crawl, "navigate")
                | (BrowserOperationKind::Act, _)
                | (BrowserOperationKind::Debug, "evaluate")
                | (BrowserOperationKind::Tabs, "open_link_new_tab")
                | (BrowserOperationKind::Download, _)
        )
}

fn lane_closed_error(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::LaneClosedByUser,
        "The browser lane was closed.",
        false,
        "Open a new browser lane if more work is required.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({ "closed": true }))
}

fn lane_not_ready_error(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser lane is still starting.",
        true,
        "Wait for the current lane start to finish and retry.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({ "lane_not_ready": true }))
}

fn ref_generation_exhausted_error(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser reference generation cannot advance.",
        false,
        "Open a fresh browser lane before issuing another tab switch.",
    )
    .for_lane(lane_id)
}

fn lane_start_task_failed_error(
    lane_id: BrowserLaneId,
    join_error: &tokio::task::JoinError,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser lane could not be started.",
        true,
        "Retry opening the browser lane.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "start_task_failed": true,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
    }))
}

fn host_open_lane_task_failed_error(
    lane_id: BrowserLaneId,
    join_error: &tokio::task::JoinError,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host could not open the lane.",
        true,
        "Retry opening the browser lane with a fresh managed Host.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "host_open_lane_task_failed": true,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
        "host_retired": join_error.is_panic(),
    }))
}

fn lane_cleanup_wait_timeout_error(lane_id: BrowserLaneId) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser lane was closed, but target cleanup is still pending.",
        true,
        "Wait for the Hub-owned cleanup to finish.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "cleanup_pending": true,
        "cleanup_wait_timeout": true,
        "timeout_ms": LANE_CLEANUP_WAITER_TIMEOUT.as_millis() as u64,
    }))
}

fn host_initialization_timeout_error(
    browser_epoch: u64,
    phase: &'static str,
    timeout: Duration,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host did not finish starting before its deadline.",
        true,
        "Retry opening the browser lane after the timed-out Host is cleaned up.",
    )
    .with_metadata(json!({
        "host_initialization_timeout": true,
        "browser_epoch": browser_epoch,
        "phase": phase,
        "timeout_ms": timeout.as_millis() as u64,
    }))
}

fn host_cleanup_timeout_error(
    browser_epoch: u64,
    phase: &'static str,
    timeout: Duration,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host did not finish shutting down before its deadline.",
        true,
        "Retry browser cleanup; the Hub retained shutdown authority.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "host_cleanup_timeout": true,
        "browser_epoch": browser_epoch,
        "phase": phase,
        "timeout_ms": timeout.as_millis() as u64,
    }))
}

fn retiring_host_wait_timeout_error(key: &HostKey) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The managed browser Host is still finishing cleanup for this identity.",
        true,
        "Retry after the retained Host cleanup completes.",
    )
    .with_metadata(json!({
        "reason_code": "browser_host_cleanup_pending",
        "cleanup_pending": true,
        "identity_mode": key.identity_mode,
        "timeout_ms": HOST_RETIREMENT_WAIT_TIMEOUT.as_millis() as u64,
        "retry_delay_ms": 1_000,
    }))
}

fn cleanup_batch_task_failed_error(
    cleanup_kind: &'static str,
    join_error: &tokio::task::JoinError,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "A managed browser cleanup task terminated unexpectedly.",
        true,
        "Retry cleanup through the lifecycle worker.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "cleanup_kind": cleanup_kind,
        "cleanup_task_failed": true,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
    }))
}

fn drain_task_failed_error(join_error: &tokio::task::JoinError) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The installation-wide browser cleanup task terminated unexpectedly.",
        true,
        "Retry closing all managed browser resources.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_drain_task_failed": true,
        "task_cancelled": join_error.is_cancelled(),
        "task_panicked": join_error.is_panic(),
    }))
}

fn owner_cleanup_pending_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The exact browser owner still has retained cleanup authority.",
        true,
        "Retry exact-owner cleanup through the lifecycle owner.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "owner_cleanup_pending": true,
    }))
}

fn close_all_incomplete_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "Some managed browser resources are still closing.",
        true,
        "Retry closing all managed browser resources.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_drain_incomplete": true,
    }))
}

fn platform_shutdown_timeout_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser platform did not finish cleanup before its shutdown deadline.",
        true,
        "Retry browser shutdown; retained Lane and Host cleanup will be attempted again.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_shutdown_timeout": true,
        "timeout_ms": PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT.as_millis() as u64,
    }))
}

fn host_slot_retired_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host was retired while it was starting.",
        true,
        "Retry after browser cleanup finishes.",
    )
    .with_metadata(json!({ "host_retired": true }))
}

fn host_finalization_wait_timeout_error(key: &HostKey) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host is still shutting down.",
        true,
        "Retry after the retained Host shutdown completes.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "host_finalization_pending": true,
        "identity_mode": key.identity_mode,
        "timeout_ms": HOST_FINALIZATION_WAITER_TIMEOUT.as_millis() as u64,
    }))
}

fn pending_lane_start_wait_timeout_error(
    lane_id: BrowserLaneId,
    key: &HostKey,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser lane was closed, but its Host is still finishing a pending lane start.",
        true,
        "Retry cleanup through the lifecycle worker.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "cleanup_pending": true,
        "host_retirement_pending": true,
        "identity_mode": key.identity_mode,
        "timeout_ms": PENDING_LANE_START_WAIT_TIMEOUT.as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::{Notify, Semaphore};

    use super::*;
    use crate::{BrowserSurface, HostLifecycleState, ManualClock, OwnerLeaseId};

    struct Probe {
        active: AtomicUsize,
        maximum: AtomicUsize,
        entries: AtomicUsize,
        foregrounds: AtomicUsize,
        foreground_failures_remaining: AtomicUsize,
        block_foreground: AtomicBool,
        foreground_release: Semaphore,
        foreground_changed: Notify,
        host_fatal_executions_remaining: AtomicUsize,
        generic_failures_remaining: AtomicUsize,
        releases: Semaphore,
        changed: Notify,
        lane_closes: AtomicUsize,
        lane_close_failures_remaining: AtomicUsize,
        lane_close_panics_remaining: AtomicUsize,
        lane_close_completions: AtomicUsize,
        block_lane_close: AtomicBool,
        lane_close_release: Semaphore,
        lane_close_changed: Notify,
        lane_close_completed: Notify,
        lane_freezes: AtomicUsize,
        freeze_supported: AtomicBool,
        host_shutdowns: AtomicUsize,
        host_shutdown_failures_remaining: AtomicUsize,
        block_host_shutdown: AtomicBool,
        host_shutdown_release: Semaphore,
        host_shutdown_changed: Notify,
        host_launch_panics_remaining: AtomicUsize,
        block_host_launch: AtomicBool,
        host_launch_release: Semaphore,
        host_launch_changed: Notify,
        block_open_lane: AtomicBool,
        open_lane_panics_remaining: AtomicUsize,
        open_lane_release: Semaphore,
        open_lane_changed: Notify,
        open_lane_calls: AtomicUsize,
        open_lane_failure_at: AtomicUsize,
        workspace_hints: std::sync::Mutex<Vec<Option<String>>>,
        host_launch_requests: std::sync::Mutex<Vec<HostLaunchRequest>>,
        identity_capture: std::sync::Mutex<Option<CapturedIdentitySnapshot>>,
        fail_identity_capture: AtomicBool,
        agent_snapshot_release: Semaphore,
        operation_results: std::sync::Mutex<HashMap<String, BrowserOperationResult>>,
    }

    impl Probe {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                active: AtomicUsize::new(0),
                maximum: AtomicUsize::new(0),
                entries: AtomicUsize::new(0),
                foregrounds: AtomicUsize::new(0),
                foreground_failures_remaining: AtomicUsize::new(0),
                block_foreground: AtomicBool::new(false),
                foreground_release: Semaphore::new(0),
                foreground_changed: Notify::new(),
                host_fatal_executions_remaining: AtomicUsize::new(0),
                generic_failures_remaining: AtomicUsize::new(0),
                releases: Semaphore::new(0),
                changed: Notify::new(),
                lane_closes: AtomicUsize::new(0),
                lane_close_failures_remaining: AtomicUsize::new(0),
                lane_close_panics_remaining: AtomicUsize::new(0),
                lane_close_completions: AtomicUsize::new(0),
                block_lane_close: AtomicBool::new(false),
                lane_close_release: Semaphore::new(0),
                lane_close_changed: Notify::new(),
                lane_close_completed: Notify::new(),
                lane_freezes: AtomicUsize::new(0),
                freeze_supported: AtomicBool::new(false),
                host_shutdowns: AtomicUsize::new(0),
                host_shutdown_failures_remaining: AtomicUsize::new(0),
                block_host_shutdown: AtomicBool::new(false),
                host_shutdown_release: Semaphore::new(0),
                host_shutdown_changed: Notify::new(),
                host_launch_panics_remaining: AtomicUsize::new(0),
                block_host_launch: AtomicBool::new(false),
                host_launch_release: Semaphore::new(0),
                host_launch_changed: Notify::new(),
                block_open_lane: AtomicBool::new(false),
                open_lane_panics_remaining: AtomicUsize::new(0),
                open_lane_release: Semaphore::new(0),
                open_lane_changed: Notify::new(),
                open_lane_calls: AtomicUsize::new(0),
                open_lane_failure_at: AtomicUsize::new(0),
                workspace_hints: std::sync::Mutex::new(Vec::new()),
                host_launch_requests: std::sync::Mutex::new(Vec::new()),
                identity_capture: std::sync::Mutex::new(None),
                fail_identity_capture: AtomicBool::new(false),
                agent_snapshot_release: Semaphore::new(0),
                operation_results: std::sync::Mutex::new(HashMap::new()),
            })
        }

        fn enter(self: &Arc<Self>) -> ActiveCall {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            self.maximum.fetch_max(active, Ordering::AcqRel);
            self.entries.fetch_add(1, Ordering::AcqRel);
            self.changed.notify_waiters();
            ActiveCall(Arc::clone(self))
        }

        async fn wait_for_active(&self, expected: usize) {
            loop {
                if self.active.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.changed.notified().await;
            }
        }

        async fn wait_for_entries(&self, expected: usize) {
            loop {
                if self.entries.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.changed.notified().await;
            }
        }

        async fn wait_for_foregrounds(&self, expected: usize) {
            loop {
                if self.foregrounds.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.foreground_changed.notified().await;
            }
        }

        async fn wait_for_host_shutdowns(&self, expected: usize) {
            loop {
                if self.host_shutdowns.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.host_shutdown_changed.notified().await;
            }
        }

        async fn wait_for_lane_closes(&self, expected: usize) {
            loop {
                if self.lane_closes.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.lane_close_changed.notified().await;
            }
        }

        async fn wait_for_lane_close_completions(&self, expected: usize) {
            loop {
                if self.lane_close_completions.load(Ordering::Acquire) >= expected {
                    return;
                }
                self.lane_close_completed.notified().await;
            }
        }

        async fn wait_for_open_lane_calls(&self, expected: usize) {
            loop {
                if self
                    .workspace_hints
                    .lock()
                    .expect("workspace hint probe poisoned")
                    .len()
                    >= expected
                {
                    return;
                }
                self.open_lane_changed.notified().await;
            }
        }

        async fn wait_for_host_launches(&self, expected: usize) {
            loop {
                let changed = self.host_launch_changed.notified();
                if self
                    .host_launch_requests
                    .lock()
                    .expect("host launch probe poisoned")
                    .len()
                    >= expected
                {
                    return;
                }
                changed.await;
            }
        }
    }

    struct ActiveCall(Arc<Probe>);

    impl Drop for ActiveCall {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::AcqRel);
            self.0.changed.notify_waiters();
        }
    }

    struct FakeLane {
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserLaneDriver for FakeLane {
        async fn execute(
            &self,
            operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            if self
                .probe
                .host_fatal_executions_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserRestarted,
                    "Synthetic whole-CDP failure.",
                    false,
                    "Restart the synthetic Host.",
                )
                .with_metadata(json!({ "failure_scope": "host" })));
            }
            if self
                .probe
                .generic_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic operation timeout.",
                    true,
                    "Retry the synthetic operation.",
                ));
            }
            let _active = self.probe.enter();
            if operation.action == "agent_snapshot_isolation" {
                let permit = self
                    .probe
                    .agent_snapshot_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
                return Ok(snapshot_result("agent-tab", "agent-target", "agent-frame", 41));
            }
            if let Some(result) = self
                .probe
                .operation_results
                .lock()
                .expect("operation result probe poisoned")
                .get(&operation.action)
                .cloned()
            {
                return Ok(result);
            }
            let permit = self
                .probe
                .releases
                .acquire()
                .await
                .map_err(|_| BrowserPlatformError::shutting_down())?;
            permit.forget();
            Ok(BrowserOperationResult {
                output: json!({ "ok": true }),
                ..BrowserOperationResult::default()
            })
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            self.probe.lane_closes.fetch_add(1, Ordering::AcqRel);
            self.probe.lane_close_changed.notify_waiters();
            if self.probe.block_lane_close.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .lane_close_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self
                .probe
                .lane_close_panics_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("synthetic lane close panic");
            }
            if self
                .probe
                .lane_close_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic lane cleanup failure.",
                    true,
                    "Retry the lifecycle sweep.",
                ));
            }
            self.probe
                .lane_close_completions
                .fetch_add(1, Ordering::AcqRel);
            self.probe.lane_close_completed.notify_waiters();
            Ok(())
        }

        async fn bring_to_front(&self) -> Result<(), BrowserPlatformError> {
            self.probe.foregrounds.fetch_add(1, Ordering::AcqRel);
            self.probe.foreground_changed.notify_waiters();
            if self.probe.block_foreground.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .foreground_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self
                .probe
                .foreground_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic foreground failure.",
                    true,
                    "Retry the synthetic foreground request.",
                ));
            }
            Ok(())
        }

        async fn capture_identity_snapshot(
            &self,
        ) -> Result<Option<CapturedIdentitySnapshot>, BrowserPlatformError> {
            if self.probe.fail_identity_capture.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic identity capture failure.",
                    true,
                    "Retry the test capture.",
                ));
            }
            Ok(self
                .probe
                .identity_capture
                .lock()
                .expect("identity capture probe poisoned")
                .clone())
        }

        async fn freeze(&self) -> Result<LaneFreezeOutcome, BrowserPlatformError> {
            self.probe.lane_freezes.fetch_add(1, Ordering::AcqRel);
            Ok(
                if self.probe.freeze_supported.load(Ordering::Acquire) {
                    LaneFreezeOutcome::Frozen
                } else {
                    LaneFreezeOutcome::Unsupported
                },
            )
        }
    }

    struct FakeHost {
        host_id: BrowserHostId,
        epoch: u64,
        probe: Arc<Probe>,
        process_id: u32,
        headful: bool,
    }

    #[async_trait]
    impl BrowserHostDriver for FakeHost {
        fn host_id(&self) -> BrowserHostId {
            self.host_id.clone()
        }

        fn epoch(&self) -> u64 {
            self.epoch
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        fn is_headful(&self) -> bool {
            // Reflect the launch request exactly. The Hub's foreground seam
            // chooses between window focus and Host replacement based on this
            // bit, so a fake that always reports headless would silently move
            // every test onto the replacement path.
            self.headful
        }

        fn process_id(&self) -> Option<u32> {
            Some(self.process_id)
        }

        async fn open_lane(
            &self,
            request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            let call = self.probe.open_lane_calls.fetch_add(1, Ordering::AcqRel) + 1;
            self.probe
                .workspace_hints
                .lock()
                .expect("workspace hint probe poisoned")
                .push(request.workspace_hint);
            self.probe.open_lane_changed.notify_waiters();
            if self
                .probe
                .open_lane_panics_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("synthetic host open_lane panic");
            }
            if self.probe.block_open_lane.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .open_lane_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self.probe.open_lane_failure_at.load(Ordering::Acquire) == call {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic lane open failure.",
                    true,
                    "Retry the synthetic Host recovery.",
                ));
            }
            Ok(Arc::new(FakeLane {
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            self.probe.host_shutdowns.fetch_add(1, Ordering::AcqRel);
            self.probe.host_shutdown_changed.notify_waiters();
            if self.probe.block_host_shutdown.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .host_shutdown_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self
                .probe
                .host_shutdown_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic host shutdown failure.",
                    true,
                    "Retry the lifecycle sweep.",
                ));
            }
            Ok(())
        }
    }

    struct FixedEpochHost {
        host_id: BrowserHostId,
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserHostDriver for FixedEpochHost {
        fn host_id(&self) -> BrowserHostId {
            self.host_id.clone()
        }

        fn epoch(&self) -> u64 {
            0
        }

        fn state(&self) -> HostLifecycleState {
            HostLifecycleState::Running
        }

        async fn open_lane(
            &self,
            _request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            Ok(Arc::new(FakeLane {
                probe: Arc::clone(&self.probe),
            }))
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    struct FakeFactory {
        probe: Arc<Probe>,
        launches: AtomicUsize,
        next_process_id: AtomicUsize,
    }

    #[async_trait]
    impl BrowserHostFactory for FakeFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            self.probe
                .host_launch_requests
                .lock()
                .expect("host launch probe poisoned")
                .push(request.clone());
            self.launches.fetch_add(1, Ordering::AcqRel);
            self.probe.host_launch_changed.notify_waiters();
            if self
                .probe
                .host_launch_panics_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("synthetic host factory launch panic");
            }
            if self.probe.block_host_launch.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .host_launch_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            Ok(Arc::new(FakeHost {
                host_id: request.host_id,
                epoch: request.browser_epoch,
                probe: Arc::clone(&self.probe),
                process_id: self.next_process_id.fetch_add(1, Ordering::AcqRel) as u32,
                headful: request.headful,
            }))
        }
    }

    struct FixedEpochFactory {
        probe: Arc<Probe>,
    }

    #[async_trait]
    impl BrowserHostFactory for FixedEpochFactory {
        async fn launch(
            &self,
            request: HostLaunchRequest,
        ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
            Ok(Arc::new(FixedEpochHost {
                host_id: request.host_id,
                probe: Arc::clone(&self.probe),
            }))
        }
    }

    struct Harness {
        hub: BrowserSessionHub,
        client: BrowserLaneClient,
        probe: Arc<Probe>,
        factory: Arc<FakeFactory>,
        clock: Arc<ManualClock>,
    }

    fn harness() -> Harness {
        harness_with_config(HubConfig::default())
    }

    fn harness_with_config(config: HubConfig) -> Harness {
        harness_with_config_and_owner_ttl(config, 2 * 60 * 60_000)
    }

    fn harness_with_config_and_owner_ttl(
        mut config: HubConfig,
        owner_lease_ttl_ms: u64,
    ) -> Harness {
        let clock = Arc::new(ManualClock::new(1_000));
        let probe = Probe::new();
        let factory = Arc::new(FakeFactory {
            probe: Arc::clone(&probe),
            launches: AtomicUsize::new(0),
            next_process_id: AtomicUsize::new(4_242),
        });
        config.owner_lease_ttl_ms = owner_lease_ttl_ms;
        let hub =
            BrowserSessionHub::with_clock(factory.clone(), config, clock.clone());
        let owner = hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-1".to_owned()),
                "runtime-1",
            )
            .unwrap();
        let caller = CallerIdentity {
            user_id: "user-1".to_owned(),
            conversation_id: Some("conversation-1".to_owned()),
            runtime_instance_id: "runtime-1".to_owned(),
            agent_id: Some("agent-1".to_owned()),
            companion_id: None,
            execution_id: Some("execution-1".to_owned()),
            step_id: None,
            attempt_id: Some("attempt-1".to_owned()),
            remote_connection_id: None,
            surface: BrowserSurface::Native,
            owner_lease_id: owner.lease_id,
            capability_expires_at_ms: clock.now_ms() + 2 * 60 * 60_000,
            allowed_operations: BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Navigate,
                BrowserOperationKind::Observe,
                BrowserOperationKind::Crawl,
            ]),
        };
        let client = hub.bind(caller).unwrap();
        Harness {
            hub,
            client,
            probe,
            factory,
            clock,
        }
    }

    fn client_for_runtime(harness: &Harness, runtime_instance_id: &str) -> BrowserLaneClient {
        let owner = harness
            .hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-1".to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.runtime_instance_id = runtime_instance_id.to_owned();
        caller.owner_lease_id = owner.lease_id;
        caller.capability_expires_at_ms = harness.clock.now_ms() + 2 * 60 * 60_000;
        harness.hub.bind(caller).unwrap()
    }

    fn client_for_runtime_with_lease(
        harness: &Harness,
        runtime_instance_id: &str,
    ) -> (BrowserLaneClient, OwnerLeaseId) {
        let owner = harness
            .hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-1".to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.runtime_instance_id = runtime_instance_id.to_owned();
        caller.owner_lease_id = owner.lease_id.clone();
        caller.capability_expires_at_ms = u64::MAX;
        (
            harness.hub.bind(caller).unwrap(),
            owner.lease_id,
        )
    }

    fn client_for_surface(
        harness: &Harness,
        runtime_instance_id: &str,
        surface: BrowserSurface,
        allowed_operations: BTreeSet<BrowserOperationKind>,
    ) -> BrowserLaneClient {
        let owner = harness
            .hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-1".to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.runtime_instance_id = runtime_instance_id.to_owned();
        caller.surface = surface;
        caller.owner_lease_id = owner.lease_id;
        caller.capability_expires_at_ms = harness.clock.now_ms() + 2 * 60 * 60_000;
        caller.allowed_operations = allowed_operations;
        harness.hub.bind(caller).unwrap()
    }

    fn client_with_tabs(harness: &Harness, runtime_instance_id: &str) -> BrowserLaneClient {
        let mut allowed_operations = harness.client.caller().allowed_operations.clone();
        allowed_operations.insert(BrowserOperationKind::Tabs);
        client_for_surface(
            harness,
            runtime_instance_id,
            BrowserSurface::Native,
            allowed_operations,
        )
    }

    fn trusted_system_client(harness: &Harness, runtime_instance_id: &str) -> BrowserLaneClient {
        client_for_surface(
            harness,
            runtime_instance_id,
            BrowserSurface::System,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Navigate,
                BrowserOperationKind::Observe,
                BrowserOperationKind::Act,
                BrowserOperationKind::Debug,
                BrowserOperationKind::Crawl,
            ]),
        )
    }

    fn trusted_user_client(harness: &Harness, runtime_instance_id: &str) -> BrowserLaneClient {
        client_for_surface(
            harness,
            runtime_instance_id,
            BrowserSurface::User,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Navigate,
                BrowserOperationKind::Observe,
            ]),
        )
    }

    async fn open(client: &BrowserLaneClient, name: &str) -> BrowserLaneId {
        open_identity(client, name, BrowserIdentityMode::Primary).await
    }

    async fn open_identity(
        client: &BrowserLaneClient,
        name: &str,
        identity_mode: BrowserIdentityMode,
    ) -> BrowserLaneId {
        client
            .open(Some(name), identity_mode, None)
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone()
    }

    async fn open_for_user(
        harness: &Harness,
        user_id: &str,
        runtime_instance_id: &str,
        name: &str,
        identity_mode: BrowserIdentityMode,
    ) -> BrowserLaneId {
        let owner = harness
            .hub
            .issue_owner_lease(
                user_id,
                Some("conversation-1".to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.user_id = user_id.to_owned();
        caller.runtime_instance_id = runtime_instance_id.to_owned();
        caller.owner_lease_id = owner.lease_id;
        caller.capability_expires_at_ms = harness.clock.now_ms() + 2 * 60 * 60_000;
        let client = harness.hub.bind(caller).unwrap();
        open_identity(&client, name, identity_mode).await
    }

    fn navigate() -> BrowserOperation {
        BrowserOperation {
            kind: BrowserOperationKind::Navigate,
            action: "navigate".to_owned(),
            input: json!({ "url": "https://example.test" }),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            may_modify_identity: false,
        }
    }

    fn observe() -> BrowserOperation {
        BrowserOperation {
            kind: BrowserOperationKind::Observe,
            action: "observe".to_owned(),
            input: json!({}),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            may_modify_identity: false,
        }
    }

    fn screenshot() -> BrowserOperation {
        BrowserOperation {
            kind: BrowserOperationKind::Screenshot,
            action: "screenshot".to_owned(),
            input: json!({}),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            may_modify_identity: false,
        }
    }

    fn snapshot_result(
        tab_id: &str,
        target_id: &str,
        frame_id: &str,
        ref_generation: u64,
    ) -> BrowserOperationResult {
        BrowserOperationResult {
            output: json!({ "ok": true }),
            tabs: vec![crate::BrowserTabSnapshot {
                tab_id: tab_id.to_owned(),
                target_id: target_id.to_owned(),
                title: Some(tab_id.to_owned()),
                url: Some(format!("https://{tab_id}.test")),
                active: true,
                crashed: false,
            }],
            active_tab_id: Some(tab_id.to_owned()),
            active_frame_id: Some(frame_id.to_owned()),
            ref_generation: Some(ref_generation),
        }
    }

    #[tokio::test]
    async fn foreground_lane_for_user_uses_trusted_seam_and_publishes_activity() {
        // Headful seam: the Host already owns a native window, so the trusted
        // request focuses it directly without replacing the process.
        let mut config = HubConfig::default();
        config.headful = true;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "foreground-primary").await;
        let before = harness.client.status(&lane_id).await.unwrap();
        let mut events = harness.hub.subscribe();
        harness.clock.advance(25);

        let foregrounded = harness
            .hub
            .foreground_lane_for_user("user-1", &lane_id)
            .await
            .unwrap();

        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 1);
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "a headful Host must be focused in place, not replaced"
        );
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "foregrounding must not manufacture a model-visible operation"
        );
        assert_eq!(foregrounded.lane_id, lane_id);
        assert_eq!(foregrounded.browser_epoch, before.browser_epoch);
        assert_eq!(foregrounded.last_active_at_ms, before.last_active_at_ms + 25);
        let event = events.recv().await.unwrap();
        assert_eq!(event.change_kind, "lane_foregrounded");
        assert_eq!(event.lane_id.as_ref(), Some(&lane_id));
        assert_eq!(event.user_id.as_deref(), Some("user-1"));
        assert_eq!(event.at_ms, foregrounded.last_active_at_ms);
    }

    #[tokio::test]
    async fn foreground_lane_for_user_replaces_headless_host_with_headful_replacement() {
        // Headless transition: the default policy launches a truly headless
        // Primary; the trusted foreground request performs one exact Host
        // replacement with the same identity and a fresh epoch.
        let harness = harness();
        let lane_id = open(&harness.client, "foreground-headless").await;
        let before = harness.client.status(&lane_id).await.unwrap();
        harness.clock.advance(25);

        let foregrounded = harness
            .hub
            .foreground_lane_for_user("user-1", &lane_id)
            .await
            .unwrap();

        let launch_requests = harness
            .probe
            .host_launch_requests
            .lock()
            .expect("host launch probe poisoned")
            .clone();
        assert_eq!(launch_requests.len(), 2);
        assert!(
            !launch_requests[0].headful,
            "routine Agent work must launch the Primary Host headless"
        );
        assert!(
            launch_requests[1].headful,
            "the trusted foreground replacement must request a headful Host"
        );
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            1,
            "the old headless Host must be explicitly stopped before its replacement"
        );
        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 1);
        assert_ne!(
            foregrounded.browser_epoch, before.browser_epoch,
            "the process replacement must advance the browser epoch"
        );
        assert_eq!(
            foregrounded.error_code, None,
            "the lane that requested the visibility transition must not be left errored"
        );
        assert_eq!(foregrounded.error_message, None);
        let refreshed = harness.client.status(&lane_id).await.unwrap();
        assert_eq!(
            refreshed.error_code, None,
            "the requesting lane must not surface a persistent restart banner"
        );
        let fence = harness
            .client
            .execute(&lane_id, navigate())
            .await
            .unwrap_err();
        assert_eq!(
            fence.code,
            BrowserErrorCode::BrowserRestarted,
            "old refs must be invalidated and a fresh observe still required"
        );
        assert_eq!(foregrounded.last_active_at_ms, before.last_active_at_ms + 25);
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headless,
            "one foreground request must not mutate the installation default"
        );
    }

    #[tokio::test]
    async fn primary_visibility_round_trip_restarts_and_rebinds_all_primary_lanes() {
        let harness = harness();
        let second_client = client_for_runtime(&harness, "runtime-visibility-sibling");
        let first = open(&harness.client, "visibility-a").await;
        let second = open(&second_client, "visibility-b").await;
        let initial_first = harness.client.status(&first).await.unwrap();
        let initial_second = second_client.status(&second).await.unwrap();
        assert_eq!(initial_first.browser_epoch, initial_second.browser_epoch);
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headless
        );

        harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headful)
            .await
            .unwrap();
        let headful_first = harness.client.status(&first).await.unwrap();
        let headful_second = second_client.status(&second).await.unwrap();
        assert_ne!(headful_first.browser_epoch, initial_first.browser_epoch);
        assert_eq!(headful_first.browser_epoch, headful_second.browser_epoch);
        assert_eq!(
            headful_first.error_code, None,
            "an intentional visibility change must not surface a restart error"
        );
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headful
        );

        harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headless)
            .await
            .unwrap();
        let headless_first = harness.client.status(&first).await.unwrap();
        let headless_second = second_client.status(&second).await.unwrap();
        assert_ne!(headless_first.browser_epoch, headful_first.browser_epoch);
        assert_eq!(headless_first.browser_epoch, headless_second.browser_epoch);
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headless
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
        let requests = harness
            .probe
            .host_launch_requests
            .lock()
            .expect("host launch probe poisoned")
            .clone();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.headful)
                .collect::<Vec<_>>(),
            vec![false, true, false]
        );

        let launches = harness.factory.launches.load(Ordering::Acquire);
        harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headless)
            .await
            .unwrap();
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            launches,
            "setting the actual mode again must be a no-op"
        );

        let drained = tokio::time::timeout(Duration::from_secs(2), harness.hub.close_all())
            .await
            .expect("installation drain hung after visibility round trip")
            .unwrap();
        assert_eq!(drained.remaining_lane_count, 0);
        assert_eq!(drained.remaining_cleanup_count, 0);
        assert_eq!(drained.remaining_managed_host_count, 0);
        harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headful)
            .await
            .unwrap();
        let reopened = open(&harness.client, "visibility-reopened").await;
        assert!(harness.client.status(&reopened).await.is_ok());
        assert!(
            harness
                .probe
                .host_launch_requests
                .lock()
                .expect("host launch probe poisoned")
                .last()
                .expect("reopened Host launch")
                .headful,
            "a future Primary Host must use the updated default"
        );
    }

    #[tokio::test]
    async fn opposite_primary_visibility_transitions_are_serialized() {
        let harness = harness();
        let lane_id = open(&harness.client, "visibility-serialized").await;
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);

        let headful_hub = harness.hub.clone();
        let headful = tokio::spawn(async move {
            headful_hub
                .set_primary_visibility(BrowserVisibility::Headful)
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_launches(2),
        )
        .await
        .expect("headful replacement never reached the blocked launch");

        let headless_hub = harness.hub.clone();
        let headless = tokio::spawn(async move {
            headless_hub
                .set_primary_visibility(BrowserVisibility::Headless)
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            2,
            "the opposite transition must wait behind the first visibility gate"
        );

        harness.probe.host_launch_release.add_permits(1);
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_launches(3),
        )
        .await
        .expect("headless successor never started after the first transition");
        harness.probe.host_launch_release.add_permits(1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), headful)
                .await
                .expect("headful transition did not settle")
                .unwrap()
                .unwrap(),
            BrowserVisibility::Headful
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), headless)
                .await
                .expect("headless transition did not settle")
                .unwrap()
                .unwrap(),
            BrowserVisibility::Headless
        );
        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);

        let requests = harness
            .probe
            .host_launch_requests
            .lock()
            .expect("host launch probe poisoned")
            .iter()
            .map(|request| request.headful)
            .collect::<Vec<_>>();
        assert_eq!(requests, vec![false, true, false]);
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headless
        );
        let overview = harness.hub.overview().await;
        assert_eq!(overview.hosts.len(), 1);
        assert!(!overview.hosts[0].headful);
        assert!(harness.client.status(&lane_id).await.is_ok());
    }

    #[tokio::test]
    async fn visibility_transition_racing_start_never_publishes_old_epoch_driver() {
        let harness = harness();
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);
        let opening_client = harness.client.clone();
        let opening = tokio::spawn(async move {
            opening_client
                .open(
                    Some("visibility-start-race"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_open_lane_calls(1).await;

        let visibility_hub = harness.hub.clone();
        let visibility = tokio::spawn(async move {
            visibility_hub
                .set_primary_visibility(BrowserVisibility::Headful)
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "visibility replacement must wait until the Hub-owned Lane start publishes its driver"
        );

        harness.probe.open_lane_release.add_permits(1);
        let opened = tokio::time::timeout(Duration::from_secs(1), opening)
            .await
            .expect("initial Lane start did not settle")
            .unwrap()
            .unwrap()
            .lane()
            .clone();
        harness
            .probe
            .block_open_lane
            .store(false, Ordering::Release);
        // If the replacement observed the old blocking bit before this task
        // resumed, release that exact rebind call as well.
        harness.probe.open_lane_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), visibility)
            .await
            .expect("visibility transition did not settle after the Lane start")
            .unwrap()
            .unwrap();
        let current = harness.client.status(&opened.lane_id).await.unwrap();
        assert_ne!(current.browser_epoch, opened.browser_epoch);
        assert_eq!(
            current.error_code, None,
            "an intentional visibility change must not surface a restart error"
        );
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
        assert!(harness.hub.overview().await.hosts[0].headful);
    }

    #[tokio::test]
    async fn close_racing_visibility_transition_leaves_no_replacement_host() {
        let harness = harness();
        let lane_id = open(&harness.client, "visibility-close-race").await;
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);
        let visibility_hub = harness.hub.clone();
        let visibility_lane = lane_id.clone();
        let visibility = tokio::spawn(async move {
            visibility_hub
                .set_lane_visibility_for_user(
                    "user-1",
                    &visibility_lane,
                    BrowserVisibility::Headful,
                )
                .await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_launches(2),
        )
        .await
        .expect("visibility replacement did not reach the blocked launch");

        let close_hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close = tokio::spawn(async move { close_hub.close_lane(&closing_lane).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if harness.hub.list_lanes().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("close waited behind the visibility transition before detaching its Lane");

        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);
        harness.probe.host_launch_release.add_permits(1);
        let visibility_error = tokio::time::timeout(Duration::from_secs(2), visibility)
            .await
            .expect("visibility transition did not settle after close")
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            visibility_error.code,
            BrowserErrorCode::LaneClosedByUser | BrowserErrorCode::BrowserUnavailable
        ));
        let closed = tokio::time::timeout(Duration::from_secs(2), close)
            .await
            .expect("close did not finish replacement Host cleanup")
            .unwrap()
            .unwrap();
        assert_eq!(closed.closed, 1);
        let remaining = harness.hub.remaining_resources().await;
        assert_eq!(remaining, RemainingResources::default());
    }

    #[tokio::test]
    async fn foreground_lane_for_user_fails_closed_for_other_user_and_non_primary_lane() {
        let harness = harness();
        let foreign_lane = open_for_user(
            &harness,
            "user-2",
            "runtime-user-2",
            "foreign-primary",
            BrowserIdentityMode::Primary,
        )
        .await;
        let anonymous_lane = open_identity(
            &harness.client,
            "foreground-anonymous",
            BrowserIdentityMode::Anonymous,
        )
        .await;

        let foreign_error = harness
            .hub
            .foreground_lane_for_user("user-1", &foreign_lane)
            .await
            .unwrap_err();
        assert_eq!(foreign_error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(foreign_error.lane_id.as_ref(), Some(&foreign_lane));

        let identity_error = harness
            .hub
            .foreground_lane_for_user("user-1", &anonymous_lane)
            .await
            .unwrap_err();
        assert_eq!(identity_error.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(identity_error.lane_id.as_ref(), Some(&anonymous_lane));
        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn foreground_lane_for_user_rejects_blank_trusted_user_without_dispatch() {
        let harness = harness();
        let lane_id = open(&harness.client, "foreground-blank-user").await;

        let error = harness
            .hub
            .foreground_lane_for_user("  ", &lane_id)
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(error.lane_id.as_ref(), Some(&lane_id));
        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn foreground_lane_for_user_rejects_missing_lane_without_leaking_other_inventory() {
        let harness = harness();
        let missing = BrowserLaneId::parse("missing-foreground-lane").unwrap();

        let error = harness
            .hub
            .foreground_lane_for_user("user-1", &missing)
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::LaneNotFound);
        assert_eq!(error.lane_id.as_ref(), Some(&missing));
        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn foreground_lane_for_user_requires_running_lane_and_propagates_safe_driver_error() {
        // Headful seam so a driver failure surfaces directly, without the
        // headless-replacement path emitting restart events first.
        let mut config = HubConfig::default();
        config.headful = true;
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let capacity_holder = open(&harness.client, "foreground-capacity-holder").await;
        let queued = harness
            .client
            .open(
                Some("foreground-queued"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();

        let queued_error = harness
            .hub
            .foreground_lane_for_user("user-1", &queued)
            .await
            .unwrap_err();
        assert_eq!(queued_error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(queued_error.lane_id.as_ref(), Some(&queued));
        assert_eq!(harness.probe.foregrounds.load(Ordering::Acquire), 0);

        harness.hub.close_lane(&capacity_holder).await.unwrap();
        let running = queued;
        assert_eq!(
            harness.client.status(&running).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running
        );
        harness
            .probe
            .foreground_failures_remaining
            .store(1, Ordering::Release);
        let mut events = harness.hub.subscribe();
        let before = harness.client.status(&running).await.unwrap();
        harness.clock.advance(50);
        let driver_error = harness
            .hub
            .foreground_lane_for_user("user-1", &running)
            .await
            .unwrap_err();
        assert_eq!(driver_error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(driver_error.message, "Synthetic foreground failure.");
        assert_eq!(driver_error.lane_id.as_ref(), Some(&running));
        assert_eq!(
            harness.client.status(&running).await.unwrap().last_active_at_ms,
            before.last_active_at_ms,
            "a failed foreground request must not publish successful activity"
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn closing_lane_cancels_in_flight_foreground_without_success_event() {
        let harness = harness();
        let lane_id = open(&harness.client, "foreground-close-race").await;
        harness
            .probe
            .block_foreground
            .store(true, Ordering::Release);
        let mut events = harness.hub.subscribe();
        let foreground_hub = harness.hub.clone();
        let foreground_lane = lane_id.clone();
        let foreground = tokio::spawn(async move {
            foreground_hub
                .foreground_lane_for_user("user-1", &foreground_lane)
                .await
        });
        harness.probe.wait_for_foregrounds(1).await;

        harness.hub.close_lane(&lane_id).await.unwrap();
        let error = foreground.await.unwrap().unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::LaneClosedByUser);
        assert_eq!(error.lane_id.as_ref(), Some(&lane_id));
        let kinds = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.change_kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"lane_stopping".to_owned()));
        assert!(kinds.contains(&"lane_closed".to_owned()));
        assert!(!kinds.contains(&"lane_foregrounded".to_owned()));
    }

    #[tokio::test]
    async fn switching_tabs_invalidates_frame_and_stales_old_refs_without_refresh_bump() {
        let harness = harness();
        let client = client_with_tabs(&harness, "tab-ref-fence");
        let lane_id = open(&client, "tab-ref-fence").await;

        {
            let mut results = harness
                .probe
                .operation_results
                .lock()
                .expect("operation result probe poisoned");
            results.insert(
                "seed_snapshot".to_owned(),
                snapshot_result("old-tab", "old-target", "old-frame", 7),
            );
            results.insert(
                "switch_tab".to_owned(),
                snapshot_result("new-tab", "new-target", "new-frame", 1),
            );
            results.insert(
                "same_tab_refresh".to_owned(),
                BrowserOperationResult {
                    output: json!({ "ok": true }),
                    tabs: vec![crate::BrowserTabSnapshot {
                        tab_id: "new-tab".to_owned(),
                        target_id: "new-target".to_owned(),
                        title: Some("new-tab".to_owned()),
                        url: Some("https://new-tab.test".to_owned()),
                        active: true,
                        crashed: false,
                    }],
                    active_tab_id: Some("new-tab".to_owned()),
                    active_frame_id: Some("new-frame-after-refresh".to_owned()),
                    ref_generation: Some(8),
                },
            );
        }

        let mut seed = observe();
        seed.action = "seed_snapshot".to_owned();
        client.execute(&lane_id, seed).await.unwrap();
        let seeded = client.status(&lane_id).await.unwrap();
        assert_eq!(seeded.active_tab_id.as_deref(), Some("old-tab"));
        assert_eq!(seeded.active_frame_id.as_deref(), Some("old-frame"));
        assert_eq!(seeded.ref_generation, 7);

        let switched = client
            .execute(
                &lane_id,
                BrowserOperation {
                    kind: BrowserOperationKind::Tabs,
                    action: "switch_tab".to_owned(),
                    input: json!({ "tab_id": "new-tab" }),
                    expected_browser_epoch: None,
                    target_id: None,
                    frame_id: None,
                    ref_generation: Some(seeded.ref_generation),
                    may_modify_identity: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(switched.active_tab_id.as_deref(), Some("new-tab"));

        let after_switch = client.status(&lane_id).await.unwrap();
        assert_eq!(after_switch.active_tab_id.as_deref(), Some("new-tab"));
        assert_eq!(
            after_switch.active_frame_id, None,
            "a frame cursor belongs to the old tab and must be cleared"
        );
        assert_eq!(
            after_switch.ref_generation,
            seeded.ref_generation + 1,
            "a tab switch must advance the Hub ref fence exactly once"
        );

        let mut stale = navigate();
        stale.expected_browser_epoch = Some(after_switch.browser_epoch);
        stale.ref_generation = Some(seeded.ref_generation);
        let stale_error = client.execute(&lane_id, stale).await.unwrap_err();
        assert_eq!(stale_error.code, BrowserErrorCode::StaleLaneRef);

        let refresh = BrowserOperation {
            kind: BrowserOperationKind::Tabs,
            action: "same_tab_refresh".to_owned(),
            input: json!({}),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: Some(after_switch.ref_generation),
            may_modify_identity: false,
        };
        let refresh_result = client.execute(&lane_id, refresh).await.unwrap();
        assert_eq!(refresh_result.active_tab_id.as_deref(), Some("new-tab"));

        let after_refresh = client.status(&lane_id).await.unwrap();
        assert_eq!(
            after_refresh.ref_generation, after_switch.ref_generation,
            "refreshing inventory for the same tab must not advance the ref fence"
        );
        assert_eq!(
            after_refresh.active_frame_id.as_deref(),
            Some("new-frame-after-refresh")
        );
    }

    #[tokio::test]
    async fn managed_host_process_ids_include_only_initialized_hosts() {
        let harness = harness();
        assert!(harness.hub.managed_host_process_ids().await.is_empty());

        open(&harness.client, "telemetry").await;

        assert_eq!(
            harness.hub.managed_host_process_ids().await,
            vec![4_242]
        );
    }

    #[tokio::test]
    async fn hub_epoch_does_not_trust_an_adapter_local_epoch() {
        let clock = Arc::new(ManualClock::new(1_000));
        let probe = Probe::new();
        let hub = BrowserSessionHub::with_clock(
            Arc::new(FixedEpochFactory {
                probe: Arc::clone(&probe),
            }),
            HubConfig::default(),
            clock.clone(),
        );
        let owner = hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-1".to_owned()),
                "runtime-fixed-epoch",
            )
            .unwrap();
        let client = hub
            .bind(CallerIdentity {
                user_id: "user-1".to_owned(),
                conversation_id: Some("conversation-1".to_owned()),
                runtime_instance_id: "runtime-fixed-epoch".to_owned(),
                agent_id: Some("agent-1".to_owned()),
                companion_id: None,
                execution_id: Some("execution-1".to_owned()),
                step_id: None,
                attempt_id: None,
                remote_connection_id: None,
                surface: BrowserSurface::Native,
                owner_lease_id: owner.lease_id,
                capability_expires_at_ms: clock.now_ms() + 10_000,
                allowed_operations: BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Navigate,
                ]),
            })
            .unwrap();

        let lane = client
            .open(
                Some("fixed-driver-epoch"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(lane.browser_epoch, 1);
        assert_eq!(hub.overview().await.hosts[0].epoch, 1);
    }

    #[tokio::test]
    async fn revoke_owner_lease_closes_lanes_even_after_the_lease_expired() {
        let harness =
            harness_with_config_and_owner_ttl(HubConfig::default(), 10);
        let (client, lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-expired-owner");
        let lane_id = open(&client, "expired-owner").await;

        harness.clock.advance(10);
        assert_eq!(
            harness
                .hub
                .renew_owner_lease(&lease_id)
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );

        let result = harness.hub.revoke_owner_lease(&lease_id).await.unwrap();
        assert_eq!(result.closed, 1);
        assert!(!result.already_closed);
        assert!(harness.hub.lane_snapshot_unchecked(&lane_id).await.is_none());
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn exact_owner_retry_retains_host_obligation_after_target_cleanup() {
        let harness = harness();
        let (client, lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-owner-host-retry");
        let _lane_id = open(&client, "owner-host-retry").await;
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);

        let first = harness
            .hub
            .revoke_owner_lease(&lease_id)
            .await
            .unwrap_err();
        assert_eq!(first.metadata["cleanup_pending"], true);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.list_lanes().await.is_empty());

        harness
            .hub
            .revoke_owner_lease(&lease_id)
            .await
            .expect("exact owner must retry its retained Host epoch");
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            1,
            "Host-only retry must not close the already-clean target again"
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 0);
        assert_eq!(overview.pending_cleanup_count, 0);
        assert_eq!(overview.managed_host_count, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn owner_close_pending_cleanup_error_reports_accurate_detached_count() {
        let harness = harness();
        let (client, lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-owner-close-count");
        let stuck_lane = open(&client, "owner-count-stuck").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        // A slow target close times out for its caller and retains cleanup
        // authority while the driver close keeps running.
        let hub = harness.hub.clone();
        let closing = stuck_lane.clone();
        let stuck_close = tokio::spawn(async move { hub.close_lane(&closing).await });
        harness.probe.wait_for_lane_closes(1).await;
        tokio::time::advance(LANE_CLEANUP_WAITER_TIMEOUT).await;
        tokio::task::yield_now().await;
        let stuck_error = stuck_close
            .await
            .expect("close task panicked")
            .expect_err("blocked cleanup must time out for its caller");
        assert_eq!(stuck_error.metadata["cleanup_pending"], true);

        // Two more owner lanes close successfully in the same owner call, but
        // the retained cleanup keeps finish_owner_cleanup pending. The error
        // must still credit both lanes that actually closed.
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        open(&client, "owner-count-a").await;
        open(&client, "owner-count-b").await;
        let error = harness
            .hub
            .close_owner_lanes(&lease_id)
            .await
            .expect_err("retained cleanup must keep the owner call pending");
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(
            error.metadata["detached_closed"], 2,
            "the pending owner-cleanup error must not discard the accurate close count"
        );

        // Convergence: releasing the blocked driver lets the exact owner
        // finish its retained Host obligation.
        harness.probe.lane_close_release.add_permits(1);
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_lane_close_completions(3),
        )
        .await
        .expect("the blocked driver close did not finish after release");
        let converged = harness.hub.revoke_owner_lease(&lease_id).await.unwrap();
        assert_eq!(converged.closed, 0);
        assert!(converged.already_closed);
        assert_eq!(harness.hub.remaining_resources().await, RemainingResources::default());
    }

    #[tokio::test]
    async fn exact_owner_target_retry_never_kills_or_waits_on_shared_primary_sibling() {
        let harness = harness();
        let (target, target_lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-owner-target-retry");
        let (sibling, _sibling_lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-owner-target-sibling");
        let _target_lane = open(&target, "owner-target-retry").await;
        let sibling_lane = open(&sibling, "owner-target-sibling").await;
        let sibling_epoch = sibling.status(&sibling_lane).await.unwrap().browser_epoch;
        harness
            .probe
            .lane_close_failures_remaining
            .store(2, Ordering::Release);

        assert!(
            harness
                .hub
                .revoke_owner_lease(&target_lease_id)
                .await
                .is_err()
        );
        assert!(
            harness
                .hub
                .revoke_owner_lease(&target_lease_id)
                .await
                .is_err(),
            "a second failure of this owner's detached target must not be reported as success"
        );
        harness
            .hub
            .revoke_owner_lease(&target_lease_id)
            .await
            .expect("the third exact target attempt must converge");

        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 3);
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "exact-owner cleanup must never stop a shared Primary Host"
        );
        assert_eq!(
            sibling.status(&sibling_lane).await.unwrap().browser_epoch,
            sibling_epoch
        );
    }

    #[tokio::test]
    async fn revoking_stale_lease_does_not_close_replacement_with_same_runtime() {
        let harness = harness();
        let runtime = "runtime-replacement";
        let (old_client, old_lease_id) =
            client_for_runtime_with_lease(&harness, runtime);
        let old_lane = open(&old_client, "old-owner").await;
        let (replacement_client, replacement_lease_id) =
            client_for_runtime_with_lease(&harness, runtime);

        assert_eq!(
            replacement_client
                .open(
                    Some("old-owner"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        let replacement_lane = open(&replacement_client, "replacement-owner").await;

        let result = harness
            .hub
            .revoke_owner_lease(&old_lease_id)
            .await
            .unwrap();
        assert_eq!(result.closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&old_lane).await.is_none());
        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&replacement_lane)
                .await
                .is_some()
        );
        assert!(harness.hub.renew_owner_lease(&replacement_lease_id).is_ok());
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn close_all_preserves_owner_lease_for_a_fresh_lane() {
        let harness = harness();
        let first = open(&harness.client, "close-all-first").await;
        let result = harness.client.close_all().await.unwrap();
        assert_eq!(result.closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&first).await.is_none());

        let reopened = open(&harness.client, "close-all-reopened").await;
        assert_ne!(first, reopened);
        assert_eq!(harness.hub.list_lanes().await.len(), 1);
    }

    #[tokio::test]
    async fn agent_surfaces_allow_primary_and_crawl_scoped_anonymous_only() {
        let harness = harness();
        for (surface, label) in [
            (BrowserSurface::Native, "native"),
            (BrowserSurface::Gateway, "gateway"),
            (BrowserSurface::Acp, "acp"),
            (BrowserSurface::Remote, "remote"),
            (BrowserSurface::Cluster, "cluster"),
        ] {
            let authorized = client_for_surface(
                &harness,
                &format!("runtime-{label}-authorized"),
                surface,
                BTreeSet::from([
                    BrowserOperationKind::Manage,
                    BrowserOperationKind::Crawl,
                ]),
            );
            assert!(
                authorized
                    .open(
                        Some(&format!("{label}-primary")),
                        BrowserIdentityMode::Primary,
                        None,
                    )
                    .await
                    .is_ok(),
                "{surface:?} must retain Primary access"
            );
            assert!(
                authorized
                    .open(
                        Some(&format!("{label}-anonymous")),
                        BrowserIdentityMode::Anonymous,
                        None,
                    )
                    .await
                    .is_ok(),
                "{surface:?} with Crawl must be allowed Anonymous access"
            );
            for identity_mode in [
                BrowserIdentityMode::AuthenticatedReplica,
                BrowserIdentityMode::Isolated,
            ] {
                let error = authorized
                    .open(
                        Some(&format!("{label}-forbidden-{identity_mode:?}")),
                        identity_mode,
                        None,
                    )
                    .await
                    .unwrap_err();
                assert_eq!(
                    error.code,
                    BrowserErrorCode::InvalidCallerIdentity,
                    "{surface:?} unexpectedly received {identity_mode:?}"
                );
            }

            let no_crawl = client_for_surface(
                &harness,
                &format!("runtime-{label}-no-crawl"),
                surface,
                BTreeSet::from([BrowserOperationKind::Manage]),
            );
            assert!(
                no_crawl
                    .open(
                        Some(&format!("{label}-primary-no-crawl")),
                        BrowserIdentityMode::Primary,
                        None,
                    )
                    .await
                    .is_ok(),
                "{surface:?} must allow Primary without Crawl"
            );
            assert_eq!(
                no_crawl
                    .open(
                        Some(&format!("{label}-anonymous-no-crawl")),
                        BrowserIdentityMode::Anonymous,
                        None,
                    )
                    .await
                    .unwrap_err()
                    .code,
                BrowserErrorCode::OperationNotAllowed,
                "{surface:?} must not receive Anonymous without Crawl"
            );
        }
        assert_eq!(
            harness.hub.list_lanes().await.len(),
            15,
            "rejected identity modes must not allocate lanes"
        );
    }

    #[tokio::test]
    async fn system_surface_allows_anonymous_and_crawl_scoped_replica_only() {
        let harness = harness();
        let no_crawl = client_for_surface(
            &harness,
            "runtime-system-no-crawl",
            BrowserSurface::System,
            BTreeSet::from([BrowserOperationKind::Manage]),
        );
        assert!(
            no_crawl
                .open(
                    Some("system-anonymous"),
                    BrowserIdentityMode::Anonymous,
                    None,
                )
                .await
                .is_ok(),
            "System Anonymous is a trusted existing path"
        );
        assert_eq!(
            no_crawl
                .open(
                    Some("system-replica-no-crawl"),
                    BrowserIdentityMode::AuthenticatedReplica,
                    None,
                )
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::OperationNotAllowed
        );
        for identity_mode in [
            BrowserIdentityMode::Primary,
            BrowserIdentityMode::Isolated,
        ] {
            assert_eq!(
                no_crawl
                    .open(
                        Some(&format!("system-forbidden-{identity_mode:?}")),
                        identity_mode,
                        None,
                    )
                    .await
                    .unwrap_err()
                    .code,
                BrowserErrorCode::InvalidCallerIdentity
            );
        }

        harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({"cookies": ["system"]})),
                SnapshotCoverage::cookies_only(),
            )
            .unwrap();
        let crawler = client_for_surface(
            &harness,
            "runtime-system-crawler",
            BrowserSurface::System,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Crawl,
            ]),
        );
        assert!(
            crawler
                .open(
                    Some("system-replica"),
                    BrowserIdentityMode::AuthenticatedReplica,
                    None,
                )
                .await
                .is_ok()
        );
        assert_eq!(harness.hub.list_lanes().await.len(), 2);
    }

    #[tokio::test]
    async fn user_surface_allows_primary_anonymous_and_isolated_but_not_replica() {
        let harness = harness();
        harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({"cookies": ["user"]})),
                SnapshotCoverage::cookies_only(),
            )
            .unwrap();
        let user = trusted_user_client(&harness, "runtime-user-identities");
        for (identity_mode, lane_name) in [
            (BrowserIdentityMode::Primary, "user-primary"),
            (BrowserIdentityMode::Anonymous, "user-anonymous"),
            (BrowserIdentityMode::Isolated, "user-isolated"),
        ] {
            assert!(
                user.open(Some(lane_name), identity_mode, None)
                    .await
                    .is_ok(),
                "User must be allowed {identity_mode:?}"
            );
        }
        assert_eq!(
            user.open(
                Some("user-replica"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap_err()
            .code,
            BrowserErrorCode::InvalidCallerIdentity
        );
        assert_eq!(harness.hub.list_lanes().await.len(), 3);
    }

    #[tokio::test]
    async fn existing_named_lane_is_rechecked_against_current_identity_authority() {
        let harness = harness();
        let lane = harness
            .client
            .open(
                Some("existing-anonymous"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        let mut narrowed = harness.client.caller().clone();
        narrowed
            .allowed_operations
            .remove(&BrowserOperationKind::Crawl);
        let narrowed = harness.hub.bind(narrowed).unwrap();

        let error = narrowed
            .open(
                Some("existing-anonymous"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(
            error.lane_id.is_none(),
            "identity authorization must fail before named-Lane lookup"
        );
        assert_eq!(harness.hub.list_lanes().await.len(), 1);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&lane.lane_id)
                .await
                .unwrap()
                .identity_mode,
            BrowserIdentityMode::Anonymous
        );
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn primary_is_canonical_but_isolated_lanes_get_distinct_hosts() {
        let primary = harness();
        open_identity(&primary.client, "primary-a", BrowserIdentityMode::Primary).await;
        open_identity(&primary.client, "primary-b", BrowserIdentityMode::Primary).await;
        assert_eq!(primary.factory.launches.load(Ordering::Acquire), 1);

        let isolated = harness();
        let user = trusted_user_client(&isolated, "runtime-isolated");
        open_identity(
            &user,
            "isolated-a",
            BrowserIdentityMode::Isolated,
        )
        .await;
        open_identity(
            &user,
            "isolated-b",
            BrowserIdentityMode::Isolated,
        )
        .await;
        assert_eq!(isolated.factory.launches.load(Ordering::Acquire), 2);
        assert_eq!(isolated.hub.overview().await.hosts.len(), 2);
    }

    #[tokio::test]
    async fn idempotent_open_cannot_change_a_named_lane_identity() {
        let harness = harness();
        let primary = harness
            .client
            .open(
                Some("identity-bound"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();

        let error = harness
            .client
            .open(
                Some("identity-bound"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        assert_eq!(error.lane_id.as_ref(), Some(&primary.lane_id));
        assert_eq!(error.metadata["requested_identity_mode"], "anonymous");
        assert_eq!(error.metadata["existing_identity_mode"], "primary");
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert_eq!(harness.hub.list_lanes().await.len(), 1);
    }

    #[tokio::test]
    async fn authenticated_replica_requires_a_published_snapshot() {
        let harness = harness();
        let system = trusted_system_client(&harness, "runtime-replica-no-snapshot");

        let error = system
            .open(
                Some("replica-without-snapshot"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity, "{error:?}");
        assert_eq!(
            error.metadata,
            json!({
                "current_generation": null,
                "snapshot_available": false,
                "refresh_required": true,
            })
        );
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 0);
        assert!(harness.hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn authenticated_replica_reload_requires_primary_identity_before_dispatch() {
        let harness = harness();
        harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({
                    "cookies": [{"name": "session", "value": "canonical"}],
                })),
                SnapshotCoverage::cookies_only(),
            )
            .unwrap();
        let client = trusted_system_client(&harness, "runtime-replica-reload");
        let lane = client
            .open(
                Some("replica-reload"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();

        // A reload has no request method in its input, so the Hub must not
        // infer that the current history entry was a safe GET. It may be a
        // POST result, and replaying it could submit the form again.
        let error = client
            .execute(
                &lane.lane_id,
                BrowserOperation {
                    kind: BrowserOperationKind::Navigate,
                    action: "reload".to_owned(),
                    input: json!({}),
                    expected_browser_epoch: None,
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(error.lane_id.as_ref(), Some(&lane.lane_id));
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "reload must be rejected before reaching the replica driver"
        );
    }

    #[tokio::test]
    async fn authenticated_replica_uses_hub_owned_read_only_classification() {
        let harness = harness();
        let payload = IdentitySnapshotPayload::from_json(json!({
            "cookies": [{"name": "session", "value": "canonical"}],
            "localStorage": [{
                "origin": "https://example.test",
                "localStorage": [{"name": "account", "value": "canonical"}]
            }]
        }));
        let published = harness
            .hub
            .publish_identity_snapshot(
                payload,
                SnapshotCoverage::current_origin("https://example.test"),
            )
            .unwrap();
        let canonical_before = harness
            .hub
            .current_identity_snapshot()
            .unwrap()
            .expect("published canonical identity snapshot");
        let payload_before = harness
            .hub
            .inner
            .identity_generations
            .require_current_payload(published.generation)
            .unwrap();

        // The shared harness caller is intentionally read-only for most tests.
        // Bind a second trusted client with the Act permission so the rejection
        // below reaches the replica identity guard rather than the operation
        // authorization check.
        let client = trusted_system_client(&harness, "runtime-replica-mutation");
        let lane = client
            .open(
                Some("replica-identity-mutation"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(lane.identity_generation, published.generation);

        let mutation = BrowserOperation {
            kind: BrowserOperationKind::Act,
            action: "set_value".to_owned(),
            input: json!({"value": "replica-change"}),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            // This is an untrusted transport hint. Forging it to false must
            // not bypass the Hub's replica classifier.
            may_modify_identity: false,
        };
        let error = client
            .execute(&lane.lane_id, mutation)
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "identity-mutating replica operation must be rejected before driver dispatch"
        );

        let evaluate = BrowserOperation {
            kind: BrowserOperationKind::Debug,
            action: "evaluate".to_owned(),
            input: json!({"expression": "localStorage.setItem('account', 'changed')"}),
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            may_modify_identity: false,
        };
        assert_eq!(
            client
                .execute(&lane.lane_id, evaluate)
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::NeedsPrimaryIdentity
        );

        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "forged replica operations must not reach the driver"
        );
        assert_eq!(
            harness
                .hub
                .current_identity_snapshot()
                .unwrap()
                .expect("canonical snapshot must remain published"),
            canonical_before
        );
        assert_eq!(
            harness
                .hub
                .inner
                .identity_generations
                .require_current_payload(published.generation)
                .unwrap(),
            payload_before
        );

        // Ordinary navigation and observation remain available on the same
        // replica lane and never write back into the canonical snapshot.
        harness.probe.releases.add_permits(2);
        client
            .execute(&lane.lane_id, navigate())
            .await
            .expect("replica navigation should remain usable");
        client
            .execute(&lane.lane_id, observe())
            .await
            .expect("replica observe should remain usable");
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            2,
            "the fake driver should only see read-only navigation and observe"
        );
        assert_eq!(
            harness
                .hub
                .current_identity_snapshot()
                .unwrap()
                .expect("canonical snapshot must remain published"),
            canonical_before
        );
        assert_eq!(
            harness
                .hub
                .inner
                .identity_generations
                .require_current_payload(published.generation)
                .unwrap(),
            payload_before
        );
    }

    #[tokio::test]
    async fn anonymous_lane_is_crawl_only_even_when_owner_can_act_and_debug() {
        let harness = harness();
        let client = client_for_surface(
            &harness,
            "runtime-anonymous-purpose",
            BrowserSurface::Native,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Crawl,
                BrowserOperationKind::Navigate,
                BrowserOperationKind::Act,
                BrowserOperationKind::Debug,
            ]),
        );
        let lane = client
            .open(
                Some("anonymous-purpose"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();

        for operation in [
            BrowserOperation {
                kind: BrowserOperationKind::Act,
                action: "set_value".to_owned(),
                input: json!({"value": "must-not-dispatch"}),
                expected_browser_epoch: None,
                target_id: None,
                frame_id: None,
                ref_generation: None,
                may_modify_identity: false,
            },
            BrowserOperation {
                kind: BrowserOperationKind::Debug,
                action: "evaluate".to_owned(),
                input: json!({"expression": "document.body.dataset.changed = 'true'"}),
                expected_browser_epoch: None,
                target_id: None,
                frame_id: None,
                ref_generation: None,
                may_modify_identity: false,
            },
        ] {
            let error = client
                .execute(&lane.lane_id, operation)
                .await
                .unwrap_err();
            assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
            assert_eq!(error.metadata["lane_purpose"], "crawl");
        }
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "interactive operation kinds must be rejected before driver dispatch"
        );

        harness.probe.releases.add_permits(2);
        client
            .execute(
                &lane.lane_id,
                BrowserOperation {
                    kind: BrowserOperationKind::Crawl,
                    action: "navigate".to_owned(),
                    input: json!({"url": "https://example.test"}),
                    expected_browser_epoch: None,
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                },
            )
            .await
            .unwrap();
        client
            .execute(
                &lane.lane_id,
                BrowserOperation {
                    kind: BrowserOperationKind::Crawl,
                    action: "observe".to_owned(),
                    input: json!({}),
                    expected_browser_epoch: None,
                    target_id: None,
                    frame_id: None,
                    ref_generation: None,
                    may_modify_identity: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(harness.probe.entries.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn owner_policy_narrowing_invalidates_stale_broader_clients() {
        let harness = harness();
        let broad = client_for_surface(
            &harness,
            "runtime-policy-narrowing",
            BrowserSurface::Native,
            BTreeSet::from([
                BrowserOperationKind::Manage,
                BrowserOperationKind::Navigate,
                BrowserOperationKind::Observe,
                BrowserOperationKind::Act,
            ]),
        );
        let lane = open(&broad, "policy-narrowing").await;

        let mut narrowed = broad.caller().clone();
        narrowed
            .allowed_operations
            .remove(&BrowserOperationKind::Act);
        let narrowed = harness.hub.bind(narrowed).unwrap();
        assert!(narrowed.status(&lane).await.is_ok());

        let error = broad.execute(&lane, navigate()).await.unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);

        let mut broadened_again = narrowed.caller().clone();
        broadened_again
            .allowed_operations
            .insert(BrowserOperationKind::Act);
        let broadened_error = match harness.hub.bind(broadened_again) {
            Ok(_) => panic!("a stale broader capability must not be rebound"),
            Err(error) => error,
        };
        assert_eq!(
            broadened_error.code,
            BrowserErrorCode::InvalidCallerIdentity
        );
    }

    #[tokio::test]
    async fn hub_assigns_current_generation_and_rejects_stale_replica_lanes() {
        let harness = harness();
        let system = trusted_system_client(&harness, "runtime-replica-generations");
        let first = harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({ "cookies": ["first"] })),
                SnapshotCoverage::current_origin("https://example.test"),
            )
            .unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.issued_at_ms, 1_000);

        let first_lane = system
            .open(
                Some("replica-one"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(first_lane.identity_generation, 1);

        harness.clock.advance(25);
        let second = harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({ "cookies": ["second"] })),
                SnapshotCoverage::current_origin("https://example.test"),
            )
            .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(
            harness.hub.current_identity_snapshot().unwrap(),
            Some(second.clone())
        );

        let stale_open = system
            .open(
                Some("replica-one"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(stale_open.code, BrowserErrorCode::IdentityReplicaStale);
        assert_eq!(stale_open.lane_id.as_ref(), Some(&first_lane.lane_id));
        assert_eq!(stale_open.metadata["requested_generation"], json!(1));
        assert_eq!(stale_open.metadata["current_generation"], json!(2));
        assert_eq!(stale_open.metadata["generation_relation"], json!("older"));

        let stale_execute = system
            .execute(&first_lane.lane_id, navigate())
            .await
            .unwrap_err();
        assert_eq!(
            stale_execute.code,
            BrowserErrorCode::IdentityReplicaStale,
            "{stale_execute:?}"
        );
        assert_eq!(stale_execute.lane_id.as_ref(), Some(&first_lane.lane_id));

        let current_lane = system
            .open(
                Some("replica-two"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(current_lane.identity_generation, 2);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn primary_capture_is_committed_before_replica_host_launch() {
        let harness = harness();
        let system = trusted_system_client(&harness, "runtime-replica-capture");
        let payload =
            IdentitySnapshotPayload::from_json(json!({"cookies": [{"name": "session"}]}));
        *harness
            .probe
            .identity_capture
            .lock()
            .expect("identity capture probe poisoned") = Some(CapturedIdentitySnapshot {
            payload: payload.clone(),
            coverage: SnapshotCoverage::current_origin("https://example.test"),
        });
        let primary = open_identity(
            &harness.client,
            "primary-capture",
            BrowserIdentityMode::Primary,
        )
        .await;
        harness.probe.releases.add_permits(1);
        harness
            .client
            .execute(&primary, navigate())
            .await
            .unwrap();

        let published = harness
            .hub
            .current_identity_snapshot()
            .unwrap()
            .expect("Primary operation did not publish a snapshot");
        assert_eq!(published.generation, 1);
        assert_eq!(
            published.coverage,
            SnapshotCoverage::current_origin("https://example.test")
        );

        let replica = system
            .open(
                Some("replica-from-capture"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(replica.identity_generation, 1);
        let launches = harness
            .probe
            .host_launch_requests
            .lock()
            .expect("host launch probe poisoned");
        let replica_launch = launches
            .iter()
            .find(|request| {
                request.identity_mode == BrowserIdentityMode::AuthenticatedReplica
            })
            .expect("replica Host was not launched");
        assert_eq!(replica_launch.identity_generation, 1);
        assert_eq!(
            replica_launch
                .identity_snapshot_payload
                .as_ref()
                .expect("replica payload missing")
                .as_json(),
            payload.as_json()
        );
    }

    #[tokio::test]
    async fn failed_primary_capture_invalidates_replicas_until_fresh_capture() {
        let harness = harness();
        let system = trusted_system_client(&harness, "runtime-replica-refresh");
        let initial = harness
            .hub
            .publish_identity_snapshot(
                IdentitySnapshotPayload::from_json(json!({"cookies": ["stable"]})),
                SnapshotCoverage::cookies_only(),
            )
            .unwrap();
        harness
            .probe
            .fail_identity_capture
            .store(true, Ordering::Release);
        let primary = open_identity(
            &harness.client,
            "primary-capture-failure",
            BrowserIdentityMode::Primary,
        )
        .await;
        harness.probe.releases.add_permits(1);

        let operation = harness.client.execute(&primary, navigate()).await;

        assert!(operation.is_ok(), "business operation was replaced: {operation:?}");
        assert_eq!(
            harness.hub.current_identity_snapshot().unwrap(),
            Some(initial.clone())
        );

        let stale = system
            .open(
                Some("replica-after-failed-capture"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(stale.code, BrowserErrorCode::NeedsPrimaryIdentity);
        assert_eq!(stale.metadata["current_generation"], initial.generation);
        assert_eq!(stale.metadata["snapshot_stale"], true);

        harness
            .probe
            .fail_identity_capture
            .store(false, Ordering::Release);
        *harness
            .probe
            .identity_capture
            .lock()
            .expect("identity capture probe poisoned") = Some(CapturedIdentitySnapshot {
            payload: IdentitySnapshotPayload::from_json(json!({"cookies": ["fresh"]})),
            coverage: SnapshotCoverage::cookies_only(),
        });
        harness.probe.releases.add_permits(1);
        harness
            .client
            .execute(&primary, navigate())
            .await
            .unwrap();

        let refreshed = harness
            .hub
            .current_identity_snapshot()
            .unwrap()
            .expect("fresh Primary capture should be published");
        assert_eq!(refreshed.generation, initial.generation + 1);
        let replica = system
            .open(
                Some("replica-after-fresh-capture"),
                BrowserIdentityMode::AuthenticatedReplica,
                None,
            )
            .await
            .unwrap()
            .lane()
            .clone();
        assert_eq!(replica.identity_generation, refreshed.generation);
    }

    #[tokio::test]
    async fn queued_promotion_preserves_the_trusted_workspace_hint() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let active = open(&harness.client, "active").await;
        let queued = harness
            .client
            .open(
                Some("queued"),
                BrowserIdentityMode::Primary,
                Some("workspace-b".to_owned()),
            )
            .await
            .unwrap();
        assert!(matches!(queued, OpenLaneOutcome::Queued { .. }));

        harness.hub.close_lane(&active).await.unwrap();
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued.lane().lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Running
        );
        assert_eq!(
            *harness
                .probe
                .workspace_hints
                .lock()
                .expect("workspace hint probe poisoned"),
            vec![None, Some("workspace-b".to_owned())]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn windows_sized_cold_start_outlives_old_five_second_hub_deadline() {
        let harness = harness();
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);

        let first_client = harness.client.clone();
        let first = tokio::spawn(async move {
            first_client
                .open(
                    Some("slow-cold-start-a"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        while harness.factory.launches.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }

        // A sibling Primary Lane joins the same Host initialization gate. It
        // must share the legitimate cold start instead of failing after the
        // former five-second platform deadline.
        let second_client = harness.client.clone();
        let second = tokio::spawn(async move {
            second_client
                .open(
                    Some("slow-cold-start-b"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }

        tokio::time::advance(Duration::from_secs(31)).await;
        tokio::task::yield_now().await;
        assert!(
            !first.is_finished(),
            "the Hub cancelled a Windows-sized engine cold start"
        );
        assert!(
            !second.is_finished(),
            "a gate waiter failed before the shared cold start completed"
        );

        harness.probe.host_launch_release.add_permits(1);
        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert!(matches!(first, OpenLaneOutcome::Running { .. }));
        assert!(matches!(second, OpenLaneOutcome::Running { .. }));
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "both Primary Lanes should share one cold-started Host"
        );
    }

    #[tokio::test]
    async fn failed_lane_start_is_detached_and_the_same_name_can_retry() {
        let harness = harness();
        harness
            .probe
            .open_lane_failure_at
            .store(1, Ordering::Release);

        let error = harness
            .client
            .open(
                Some("retry-after-start-failure"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);

        harness
            .probe
            .open_lane_failure_at
            .store(0, Ordering::Release);
        let retried = harness
            .client
            .open(
                Some("retry-after-start-failure"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(retried, OpenLaneOutcome::Running { .. }));
    }

    #[tokio::test]
    async fn factory_panic_completes_start_flight_releases_capacity_and_allows_retry() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .host_launch_panics_remaining
            .store(1, Ordering::Release);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .client
                .open(Some("factory-panic"), BrowserIdentityMode::Primary, None),
        )
        .await
        .expect("factory panic left the start waiter pending")
        .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(
            error.metadata["task_panicked"].as_bool().unwrap_or(false),
            "panic metadata should be preserved: {error:?}"
        );
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);

        harness
            .probe
            .host_launch_panics_remaining
            .store(0, Ordering::Release);
        let retry = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .client
                .open(Some("factory-panic"), BrowserIdentityMode::Primary, None),
        )
        .await
        .expect("capacity was not released after a factory panic")
        .unwrap();
        assert!(matches!(retry, OpenLaneOutcome::Running { .. }));
    }

    #[tokio::test]
    async fn host_open_lane_panic_completes_start_flight_releases_capacity_and_allows_retry() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .open_lane_panics_remaining
            .store(1, Ordering::Release);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .client
                .open(Some("host-open-panic"), BrowserIdentityMode::Primary, None),
        )
        .await
        .expect("Host open_lane panic left the start waiter pending")
        .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(
            error.metadata["task_panicked"].as_bool().unwrap_or(false),
            "panic metadata should be preserved: {error:?}"
        );
        assert_eq!(error.metadata["host_open_lane_task_failed"], true);
        assert_eq!(error.metadata["host_retired"], true);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .is_empty()
        );

        harness
            .probe
            .open_lane_panics_remaining
            .store(0, Ordering::Release);
        let retry = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .client
                .open(Some("host-open-panic"), BrowserIdentityMode::Primary, None),
        )
        .await
        .expect("capacity was not released after a Host open_lane panic")
        .unwrap();
        assert!(matches!(retry, OpenLaneOutcome::Running { .. }));
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            2,
            "a possibly-corrupt Host must not be reused after open_lane panics"
        );
    }

    #[tokio::test]
    async fn host_open_lane_panic_retains_failed_shutdown_for_sweep_retry() {
        let harness = harness();
        harness
            .probe
            .open_lane_panics_remaining
            .store(1, Ordering::Release);
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);

        let error = harness
            .client
            .open(
                Some("host-open-panic-cleanup"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.metadata["host_open_lane_task_failed"], true);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert_eq!(
            harness.hub.inner.orphaned_host_slots.lock().await.len(),
            1,
            "failed panic cleanup must remain under Hub authority"
        );

        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn installation_close_all_drains_orphaned_host_authority() {
        let harness = harness();
        harness
            .probe
            .open_lane_panics_remaining
            .store(1, Ordering::Release);
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);
        harness
            .client
            .open(
                Some("orphaned-installation-drain"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(harness.hub.inner.orphaned_host_slots.lock().await.len(), 1);
        let before = harness.hub.overview().await;
        assert_eq!(before.managed_host_count, 1);
        assert_eq!(before.pending_cleanup_count, 1);

        let result = harness.hub.close_all().await.unwrap();
        assert_eq!(result.closed, 0);
        assert!(!result.already_closed);
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .is_empty()
        );
        let reopened = open(&harness.client, "after-orphaned-drain").await;
        assert!(harness.client.status(&reopened).await.is_ok());
    }

    #[tokio::test]
    async fn cancelled_open_cannot_leave_starting_lane_or_capacity() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);

        let client = harness.client.clone();
        let opening = tokio::spawn(async move {
            client
                .open(
                    Some("cancelled-open"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_open_lane_calls(1).await;
        opening.abort();
        assert!(opening.await.unwrap_err().is_cancelled());

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if harness.hub.list_lanes().await.is_empty()
                    && harness.hub.overview().await.capacity.active == 0
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled open left a Starting Lane or scheduler capacity");

        harness.probe.open_lane_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), harness.probe.wait_for_lane_closes(1))
            .await
            .expect("late start driver was not retained for Hub-owned cleanup");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if harness
                    .hub
                    .inner
                    .pending_lane_cleanups
                    .lock()
                    .await
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("late start driver cleanup did not finish");

        harness
            .probe
            .block_open_lane
            .store(false, Ordering::Release);
        let replacement = harness
            .client
            .open(
                Some("cancelled-open"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(replacement, OpenLaneOutcome::Running { .. }));
    }

    #[tokio::test]
    async fn duplicate_open_waits_for_the_same_start_flight() {
        let harness = harness();
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);

        let first_client = harness.client.clone();
        let first = tokio::spawn(async move {
            first_client
                .open(Some("shared-start"), BrowserIdentityMode::Primary, None)
                .await
        });
        harness.probe.wait_for_open_lane_calls(1).await;
        let second_client = harness.client.clone();
        let second = tokio::spawn(async move {
            second_client
                .open(Some("shared-start"), BrowserIdentityMode::Primary, None)
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(harness.probe.open_lane_calls.load(Ordering::Acquire), 1);
        assert!(!second.is_finished());

        harness.probe.open_lane_release.add_permits(1);
        let first_lane = first.await.unwrap().unwrap().lane().lane_id.clone();
        let second_lane = second.await.unwrap().unwrap().lane().lane_id.clone();
        assert_eq!(first_lane, second_lane);
        assert_eq!(harness.probe.open_lane_calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn duplicate_waiter_registered_while_abandonment_is_pending_keeps_lane_alive() {
        let harness = harness();
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);

        let first_client = harness.client.clone();
        let first = tokio::spawn(async move {
            first_client
                .open(
                    Some("abandonment-race"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_open_lane_calls(1).await;

        let (lane_id, lane) = harness
            .hub
            .inner
            .lanes
            .read()
            .await
            .iter()
            .next()
            .map(|(lane_id, lane)| (lane_id.clone(), Arc::clone(lane)))
            .expect("Starting Lane should be visible");
        let flight = lane
            .start_flight
            .lock()
            .await
            .clone()
            .expect("Starting Lane should have a flight");

        // Hold the same gate used by duplicate-open registration. The original
        // waiter drops and schedules abandonment while this guard is held, then
        // a replacement waiter is registered before abandonment may decide.
        let open_guard = harness.hub.inner.open_gate.lock().await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let replacement_waiter = LaneStartWaiter::new(
            harness.hub.clone(),
            lane_id.clone(),
            Arc::clone(&lane),
            flight,
        );
        drop(open_guard);

        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(
            harness
                .hub
                .inner
                .lanes
                .read()
                .await
                .get(&lane_id)
                .is_some_and(|current| Arc::ptr_eq(current, &lane)),
            "abandonment detached a Lane after a replacement waiter registered"
        );

        harness.probe.open_lane_release.add_permits(1);
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            harness
                .hub
                .finish_open_action(OpenLaneAction::Wait(replacement_waiter)),
        )
        .await
        .expect("replacement waiter did not observe the shared start flight")
        .unwrap();
        assert_eq!(outcome.lane().lane_id, lane_id);
        assert_eq!(
            outcome.lane().lifecycle_state,
            LaneLifecycleState::Running
        );
        assert_eq!(harness.probe.open_lane_calls.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn expired_queued_owner_is_discarded_before_driver_start() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config_and_owner_ttl(config, 10);
        let active = open(&harness.client, "active-before-expiry").await;
        let (queued_client, queued_lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-queued-expired");
        let queued = queued_client
            .open(
                Some("queued-expired"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(queued, OpenLaneOutcome::Queued { .. }));
        let queued_lane_id = queued.lane().lane_id.clone();
        assert_eq!(harness.probe.workspace_hints.lock().unwrap().len(), 1);

        harness.clock.advance(10);
        assert_eq!(
            harness
                .hub
                .renew_owner_lease(&queued_lease_id)
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );
        harness.hub.close_lane(&active).await.unwrap();

        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued_lane_id)
                .await
                .is_none()
        );
        assert_eq!(
            harness.probe.workspace_hints.lock().unwrap().len(),
            1,
            "an expired queued owner must not reach Host::open_lane"
        );
        assert_eq!(harness.hub.overview().await.capacity.active, 0);
        assert_eq!(harness.hub.overview().await.capacity.queued, 0);
    }

    #[tokio::test]
    async fn close_during_lane_start_cannot_publish_a_detached_driver() {
        let harness = harness();
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);
        let client = harness.client.clone();
        let opening = tokio::spawn(async move {
            client
                .open(
                    Some("close-during-start"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_open_lane_calls(1).await;
        let lane_id = harness
            .hub
            .list_lanes()
            .await
            .into_iter()
            .find(|lane| lane.lane_key.lane_name == "close-during-start")
            .expect("starting Lane is visible")
            .lane_id;

        let hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close =
            tokio::spawn(async move { hub.close_lane(&closing_lane).await });
        tokio::task::yield_now().await;
        harness.probe.open_lane_release.add_permits(1);

        assert_eq!(
            opening.await.unwrap().unwrap_err().code,
            BrowserErrorCode::LaneClosedByUser
        );
        assert_eq!(close.await.unwrap().unwrap().closed, 1);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn normal_idle_expiry_uses_ten_minutes_and_stops_empty_primary_host() {
        let harness = harness();
        let lane_id = open(&harness.client, "normal-idle").await;
        let policy = harness.hub.resource_policy().await;

        harness.clock.advance(policy.idle_expiry_ms - 1);
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 0);
        assert!(harness.hub.lane_snapshot_unchecked(&lane_id).await.is_some());

        harness.clock.advance(1);
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 1);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn explicit_last_lane_close_shuts_down_crawl_host_immediately() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "crawl-immediate",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let warm_ms = harness.hub.resource_policy().await.host_warm_ms;

        // Explicit close of the last Lane retires the Host in the same call.
        // The warm timer must not be a precondition for explicit closure.
        harness.hub.close_lane(&lane_id).await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.managed_host_process_ids().await.is_empty());

        // The warm timer stays a passive backstop: a later sweep finds
        // nothing left to reclaim and must not double-shut the same Host.
        harness.clock.advance(warm_ms);
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    /// Simulates a Lane record that vanished without any close-path
    /// finalization — the defensive inconsistency that the passive warm-timer
    /// sweep exists to reclaim. Normal explicit close never takes this path.
    async fn strand_lane_record(harness: &Harness, lane_id: &BrowserLaneId) {
        let lane = harness
            .hub
            .inner
            .lanes
            .write()
            .await
            .remove(lane_id)
            .expect("stranded lane must exist");
        let lane_key = lane.snapshot.read().await.lane_key.clone();
        harness.hub.inner.lane_keys.write().await.remove(&lane_key);
        harness.hub.inner.scheduler.cancel_lane(lane_id);
        harness.hub.inner.scheduler.release_without_promotion(lane_id);
    }

    #[tokio::test]
    async fn warm_timer_sweep_reclaims_stranded_empty_crawl_host_as_backstop() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "crawl-stranded",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let warm_ms = harness.hub.resource_policy().await.host_warm_ms;
        strand_lane_record(&harness, &lane_id).await;

        // First sweep only marks the empty Host; the crawl warm interval has
        // not elapsed, so the passive path must not reclaim it yet.
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
        assert!(!harness.hub.managed_host_process_ids().await.is_empty());

        harness.clock.advance(warm_ms - 1);
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);

        let mut events = harness.hub.subscribe();
        harness.clock.advance(1);
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        let kinds = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.change_kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "host_warm_shutdown_started",
                "host_warm_shutdown_finished"
            ]
        );
    }

    #[tokio::test]
    async fn installation_close_all_drains_stranded_active_host_and_is_reusable() {
        let mut config = HubConfig::default();
        config.headful = true;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "stranded-primary").await;
        let epoch = harness.client.status(&lane_id).await.unwrap().browser_epoch;
        strand_lane_record(&harness, &lane_id).await;

        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 0);
        assert_eq!(overview.managed_host_count, 1);
        assert_eq!(overview.pending_cleanup_count, 0);
        assert_eq!(overview.hosts.len(), 1);
        assert_eq!(overview.hosts[0].lane_count, 0);
        assert_eq!(overview.hosts[0].epoch, epoch);
        assert!(overview.hosts[0].headful);

        let user_overview = harness.hub.overview_for_user("user-1").await;
        assert_eq!(user_overview.total_lanes, 0);
        assert_eq!(user_overview.managed_host_count, 0);
        assert_eq!(user_overview.pending_cleanup_count, 0);
        assert!(
            user_overview.hosts.is_empty(),
            "a user-scoped overview must not expose unattributed empty Hosts"
        );

        let result = harness.hub.close_all().await.unwrap();
        assert_eq!(result.closed, 0);
        assert!(!result.already_closed);
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);

        let second = harness.hub.close_all().await.unwrap();
        assert!(second.already_closed);
        let reopened = open(&harness.client, "after-installation-drain").await;
        assert!(harness.client.status(&reopened).await.is_ok());
    }

    #[tokio::test]
    async fn installation_close_all_serializes_concurrent_open_and_then_reopens() {
        let harness = harness();
        open(&harness.client, "drain-barrier-existing").await;
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);
        let hub = harness.hub.clone();
        let draining = tokio::spawn(async move { hub.close_all().await });
        harness.probe.wait_for_host_shutdowns(1).await;

        let error = harness
            .client
            .open(
                Some("drain-barrier-racing-open"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.metadata["platform_drain_in_progress"], true);
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "the drain barrier must reject a new Host before factory launch"
        );

        harness.probe.host_shutdown_release.add_permits(1);
        let result = draining.await.unwrap().unwrap();
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        harness
            .probe
            .block_host_shutdown
            .store(false, Ordering::Release);
        let reopened = open(&harness.client, "drain-barrier-reopened").await;
        assert!(harness.client.status(&reopened).await.is_ok());
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn cancelled_close_all_waiter_does_not_abandon_owned_drain() {
        let harness = harness();
        open(&harness.client, "cancelled-drain-existing").await;
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);
        let hub = harness.hub.clone();
        let waiter = tokio::spawn(async move { hub.close_all().await });
        harness.probe.wait_for_host_shutdowns(1).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        harness.probe.host_shutdown_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let remaining = harness.hub.remaining_resources().await;
                if remaining == RemainingResources::default()
                    && !harness.hub.inner.draining.load(Ordering::Acquire)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("caller cancellation abandoned the Hub-owned installation drain");
        harness
            .probe
            .block_host_shutdown
            .store(false, Ordering::Release);
        let reopened = open(&harness.client, "cancelled-drain-reopened").await;
        assert!(harness.client.status(&reopened).await.is_ok());
    }

    #[tokio::test]
    async fn failed_explicit_close_shutdown_keeps_host_authority_for_the_next_sweep() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "crawl-retry",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);

        // The explicit close performs the immediate Host shutdown and must
        // surface the real first failure to its caller.
        assert!(harness.hub.close_lane(&lane_id).await.is_err());
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(!harness.hub.managed_host_process_ids().await.is_empty());

        // The retained retirement authority is retried by the next sweep
        // without waiting for any warm interval.
        harness.hub.sweep().await.unwrap();
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn cancelling_empty_host_sweep_before_queue_handoff_does_not_strand_retiring_key() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "cancel-before-retire",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane_id);
        let warm_ms = harness.hub.resource_policy().await.host_warm_ms;
        strand_lane_record(&harness, &lane_id).await;

        // First sweep marks the stranded Host empty; the warm interval then
        // elapses so the next sweep would perform the retirement handoff.
        harness.hub.sweep().await.unwrap();
        harness.clock.advance(warm_ms);

        // Park the sweep on the first lock of the retirement handoff
        // (open_gate -> retiring_host_keys -> host_slots -> retiring_host_slots)
        // and cancel it there, before any authoritative structure changed.
        let retiring_keys_guard = harness.hub.inner.retiring_host_keys.write().await;
        let sweeping_hub = harness.hub.clone();
        let sweep = tokio::spawn(async move { sweeping_hub.sweep().await });
        tokio::task::yield_now().await;

        sweep.abort();
        assert!(sweep.await.unwrap_err().is_cancelled());
        drop(retiring_keys_guard);

        assert!(harness.hub.inner.host_slots.read().await.contains_key(&key));
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(
            harness
                .hub
                .inner
                .retiring_host_slots
                .lock()
                .await
                .is_empty()
        );

        // The cancelled sweep may already have consumed the empty-since mark
        // before it was aborted, so a fresh warm interval may be needed after
        // cancellation before the next sweep can retire the host.
        harness.hub.sweep().await.unwrap();
        assert!(harness.hub.inner.host_slots.read().await.contains_key(&key));
        harness.clock.advance(warm_ms);
        harness.hub.sweep().await.unwrap();
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(
            harness
                .hub
                .inner
                .retiring_host_slots
                .lock()
                .await
                .is_empty()
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn cancelling_empty_host_sweep_after_queue_handoff_retains_cleanup_authority() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "cancel-after-retire",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane_id);
        let warm_ms = harness.hub.resource_policy().await.host_warm_ms;
        strand_lane_record(&harness, &lane_id).await;

        harness.hub.sweep().await.unwrap();
        harness.clock.advance(warm_ms);
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);

        let sweeping_hub = harness.hub.clone();
        let sweep = tokio::spawn(async move { sweeping_hub.sweep().await });
        harness.probe.wait_for_host_shutdowns(1).await;
        sweep.abort();
        assert!(sweep.await.unwrap_err().is_cancelled());

        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert!(harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(
            harness
                .hub
                .inner
                .retiring_host_slots
                .lock()
                .await
                .iter()
                .any(|(pending_key, _)| pending_key == &key),
            "cancelled shutdown must leave the retired HostSlot in the durable queue"
        );

        harness
            .probe
            .block_host_shutdown
            .store(false, Ordering::Release);
        harness.hub.sweep().await.unwrap();
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(
            harness
                .hub
                .inner
                .retiring_host_slots
                .lock()
                .await
                .is_empty()
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn slow_warm_shutdown_does_not_block_opening_a_different_host() {
        let harness = harness();
        let lane_id = open_identity(
            &harness.client,
            "slow-crawl",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let warm_ms = harness.hub.resource_policy().await.host_warm_ms;
        harness.hub.close_lane(&lane_id).await.unwrap();
        harness.clock.advance(warm_ms);
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);

        let sweeping_hub = harness.hub.clone();
        let sweep = tokio::spawn(async move { sweeping_hub.sweep().await });
        harness.probe.wait_for_host_shutdowns(1).await;

        tokio::time::timeout(
            Duration::from_secs(1),
            open_identity(
                &harness.client,
                "different-primary-host",
                BrowserIdentityMode::Primary,
            ),
        )
        .await
        .expect("a different Host open waited behind slow Host shutdown");

        harness.probe.host_shutdown_release.add_permits(1);
        sweep.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn pressured_sweep_freezes_only_idle_expansion_and_preserves_first_lane() {
        let harness = harness();
        let first = open(&harness.client, "pressure-first").await;
        let expansion = open(&harness.client, "pressure-expansion").await;
        harness
            .probe
            .freeze_supported
            .store(true, Ordering::Release);
        let policy = harness.hub.resource_policy().await;
        harness.clock.advance(policy.pressured_idle_expiry_ms);
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                available_memory_bytes: policy.reserved_memory_bytes - 1,
                logical_cpus: 4,
                ..Default::default()
            })
            .await;

        let mut events = harness.hub.subscribe();
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 0);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&first)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Running
        );
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&expansion)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Frozen
        );
        assert_eq!(harness.probe.lane_freezes.load(Ordering::Acquire), 1);
        assert_eq!(
            events.try_recv().unwrap().change_kind,
            "lane_frozen_pressure"
        );
    }

    #[tokio::test]
    async fn unsupported_freeze_falls_back_to_close_without_closing_owner_first_lane() {
        let harness = harness();
        let first = open(&harness.client, "fallback-first").await;
        let expansion = open(&harness.client, "fallback-expansion").await;
        let policy = harness.hub.resource_policy().await;
        harness.clock.advance(policy.pressured_idle_expiry_ms);
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                available_memory_bytes: policy.reserved_memory_bytes - 1,
                logical_cpus: 4,
                ..Default::default()
            })
            .await;

        assert_eq!(harness.hub.sweep().await.unwrap().closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&first).await.is_some());
        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&expansion)
                .await
                .is_none()
        );
        assert_eq!(harness.probe.lane_freezes.load(Ordering::Acquire), 1);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn pressured_and_critical_sweeps_protect_active_and_only_owner_lanes() {
        let harness = harness();
        let only_crawl = open_identity(
            &harness.client,
            "only-crawl",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let policy = harness.hub.resource_policy().await;
        harness.clock.advance(policy.pressured_idle_expiry_ms);
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                available_memory_bytes: policy.reserved_memory_bytes - 1,
                logical_cpus: 4,
                ..Default::default()
            })
            .await;
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 0);
        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&only_crawl)
                .await
                .is_some()
        );

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                available_memory_bytes: 0,
                logical_cpus: 4,
                ..Default::default()
            })
            .await;
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 0);
        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&only_crawl)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn overview_maps_telemetry_rss_to_each_managed_host_without_exposing_pids() {
        let harness = harness();
        open(&harness.client, "primary").await;
        let user = trusted_user_client(&harness, "runtime-isolated-telemetry");
        user
            .open(
                Some("isolated"),
                BrowserIdentityMode::Isolated,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            harness.hub.managed_host_process_ids().await,
            vec![4_242, 4_243]
        );

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                chromium_rss_bytes: 1_500,
                host_rss_by_process_id: HashMap::from([
                    (4_242, 600),
                    (4_243, 900),
                ]),
                ..Default::default()
            })
            .await;
        assert_eq!(
            harness.hub.managed_host_process_ids().await,
            vec![4_242, 4_243]
        );

        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 2);
        let mut host_rss = overview
            .hosts
            .iter()
            .map(|host| host.rss_bytes)
            .collect::<Vec<_>>();
        host_rss.sort_unstable();
        assert_eq!(host_rss, vec![Some(600), Some(900)]);
        assert_eq!(overview.hosts.iter().map(|host| host.lane_count).sum::<usize>(), 2);
    }

    #[tokio::test]
    async fn telemetry_updates_bounded_per_lane_ewma_from_shared_host_rss() {
        let harness = harness();
        open(&harness.client, "rss-a").await;
        open(&harness.client, "rss-b").await;
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                chromium_rss_bytes: crate::resource::GIB,
                host_rss_by_process_id: HashMap::from([(4_242, crate::resource::GIB)]),
                ..Default::default()
            })
            .await;

        let estimates = harness
            .hub
            .list_lanes()
            .await
            .into_iter()
            .map(|lane| lane.resource_estimate_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            estimates,
            vec![320 * crate::resource::MIB, 320 * crate::resource::MIB]
        );
    }

    #[tokio::test]
    async fn pressured_release_does_not_promote_expansion_and_recovery_wakes_queue() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 2;
        let harness = harness_with_config(config);
        let other = client_for_runtime(&harness, "runtime-2");
        open(&harness.client, "owner-first").await;
        let other_lane = open(&other, "other-first").await;
        let expansion = harness
            .client
            .open(
                Some("owner-expansion"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();
        assert_eq!(
            harness.client.status(&expansion).await.unwrap().lifecycle_state,
            LaneLifecycleState::Queued
        );

        let policy = harness.hub.resource_policy().await;
        let browser_limit = ((16 * crate::resource::GIB) as f64
            * policy.max_browser_memory_ratio) as u64;
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 16 * crate::resource::GIB,
                available_memory_bytes: 12 * crate::resource::GIB,
                chromium_rss_bytes: browser_limit.saturating_mul(86) / 100,
                logical_cpus: 8,
                ..Default::default()
            })
            .await;
        assert_eq!(
            harness.client.status(&expansion).await.unwrap().queue
                .unwrap()
                .reason_code,
            "browser_resource_pressure"
        );

        harness.hub.close_lane(&other_lane).await.unwrap();
        let waiting = harness.client.status(&expansion).await.unwrap();
        assert_eq!(waiting.lifecycle_state, LaneLifecycleState::Queued);
        assert_eq!(
            waiting.queue.unwrap().reason_code,
            "browser_resource_pressure"
        );
        let pressured = harness.hub.overview().await;
        assert_eq!(pressured.pressure_state, ResourcePressureState::Pressured);
        assert_eq!(pressured.capacity.active, 1);
        assert_eq!(pressured.capacity.queued, 1);

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 16 * crate::resource::GIB,
                available_memory_bytes: 12 * crate::resource::GIB,
                logical_cpus: 8,
                ..Default::default()
            })
            .await;
        assert_eq!(
            harness.client.status(&expansion).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running
        );
    }

    #[tokio::test]
    async fn pressured_stale_sample_admits_only_one_first_lane_across_owners() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 4;
        let harness = harness_with_config(config);
        let other = client_for_runtime(&harness, "runtime-pressure-2");
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 64 * crate::resource::GIB,
                available_memory_bytes: 87 * crate::resource::GIB / 10,
                logical_cpus: 16,
                ..Default::default()
            })
            .await;

        // Both owners spend the same cached telemetry sample. The Hub's open
        // gate and scheduler-active accounting must let only the first caller
        // use the critical-floor allowance, even though the static Lane limit
        // has room for both.
        let (first, second) = tokio::join!(
            harness
                .client
                .open(Some("pressure-first-a"), BrowserIdentityMode::Primary, None),
            other.open(Some("pressure-first-b"), BrowserIdentityMode::Primary, None),
        );
        let outcomes = [first.unwrap(), second.unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| {
                    outcome.lane().lifecycle_state == LaneLifecycleState::Running
                })
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| outcome.lane().lifecycle_state == LaneLifecycleState::Queued)
                .count(),
            1
        );

        let queued = outcomes
            .iter()
            .find(|outcome| outcome.lane().lifecycle_state == LaneLifecycleState::Queued)
            .unwrap()
            .lane();
        let queue = queued.queue.as_ref().unwrap();
        assert_eq!(queue.reason_code, "system_memory_pressure");
        assert_eq!(queue.global_active, 1);
        assert_eq!(queue.global_queued, 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);

        let overview = harness.hub.overview().await;
        assert_eq!(overview.pressure_state, ResourcePressureState::Pressured);
        assert_eq!(overview.capacity.active, 1);
        assert_eq!(overview.capacity.queued, 1);
    }

    #[tokio::test]
    async fn first_lane_below_critical_floor_stays_queued_until_basic_capacity_recovers() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let other = client_for_runtime(&harness, "runtime-2");
        let seed = open(&harness.client, "seed").await;
        let waiting_lane = other
            .open(Some("waiting-first"), BrowserIdentityMode::Primary, None)
            .await
            .unwrap()
            .lane()
            .lane_id
            .clone();
        let policy = harness.hub.resource_policy().await;

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                available_memory_bytes: policy
                    .reserved_memory_bytes
                    .saturating_div(2)
                    .saturating_add(policy.lane_cold_start_bytes)
                    .saturating_sub(1),
                logical_cpus: 4,
                ..Default::default()
            })
            .await;
        harness.hub.close_lane(&seed).await.unwrap();

        let waiting = other.status(&waiting_lane).await.unwrap();
        assert_eq!(waiting.lifecycle_state, LaneLifecycleState::Queued);
        assert_eq!(
            waiting.queue.unwrap().reason_code,
            "system_memory_pressure"
        );
        let constrained = harness.hub.overview().await;
        assert_eq!(constrained.capacity.active, 0);
        assert_eq!(constrained.capacity.queued, 1);
        assert_eq!(
            constrained.capacity.reason_code.as_deref(),
            Some("system_memory_pressure")
        );

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 8 * crate::resource::GIB,
                // Still below the full reserve, but now safely above the
                // critical floor plus one cold-start Lane.
                available_memory_bytes: policy.reserved_memory_bytes.saturating_sub(1),
                logical_cpus: 4,
                ..Default::default()
            })
            .await;
        assert_eq!(
            other.status(&waiting_lane).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running
        );
        assert_eq!(
            harness.hub.overview().await.pressure_state,
            ResourcePressureState::Pressured
        );
    }

    #[tokio::test]
    async fn hub_workload_counts_pending_lane_cleanups_with_cold_start_bytes() {
        let harness = harness();
        let lane_id = open(&harness.client, "cleanup-workload").await;
        let policy = harness.hub.resource_policy().await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        let closing_hub = harness.hub.clone();
        let close = tokio::spawn(async move { closing_hub.close_lane(&lane_id).await });
        harness.probe.wait_for_lane_closes(1).await;

        let workload = harness
            .hub
            .resource_workload(policy.lane_cold_start_bytes)
            .await;
        assert_eq!(workload.queued_lane_estimate_bytes, policy.lane_cold_start_bytes);
        assert_eq!(workload.queued_lanes, 0);

        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        harness.probe.lane_close_release.add_permits(1);
        close.await.unwrap().unwrap();
        assert_eq!(
            harness
                .hub
                .resource_workload(policy.lane_cold_start_bytes)
                .await
                .queued_lane_estimate_bytes,
            0
        );
    }

    #[tokio::test]
    async fn same_lane_serializes_but_different_lanes_overlap_on_one_host() {
        let harness = harness();
        let lane_a = open(&harness.client, "a").await;
        let lane_b = open(&harness.client, "b").await;
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);

        let client_a1 = harness.client.clone();
        let id_a1 = lane_a.clone();
        let first = tokio::spawn(async move {
            client_a1.execute(&id_a1, navigate()).await
        });
        harness.probe.wait_for_active(1).await;

        let client_a2 = harness.client.clone();
        let id_a2 = lane_a.clone();
        let second_same_lane = tokio::spawn(async move {
            client_a2.execute(&id_a2, navigate()).await
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(30),
                harness.probe.wait_for_entries(2),
            )
            .await
            .is_err(),
            "the second operation entered the same lane driver concurrently"
        );

        let client_b = harness.client.clone();
        let different_lane = tokio::spawn(async move {
            client_b.execute(&lane_b, navigate()).await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_active(2),
        )
        .await
        .unwrap();
        assert_eq!(harness.probe.maximum.load(Ordering::Acquire), 2);

        harness.probe.releases.add_permits(2);
        first.await.unwrap().unwrap();
        different_lane.await.unwrap().unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_entries(3),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(1);
        second_same_lane.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn close_bypasses_a_blocked_lane_gate_and_cancels_the_operation() {
        let harness = harness();
        let lane_id = open(&harness.client, "blocked").await;
        let client = harness.client.clone();
        let executing_lane = lane_id.clone();
        let operation = tokio::spawn(async move {
            client.execute(&executing_lane, navigate()).await
        });
        harness.probe.wait_for_active(1).await;

        let closed = tokio::time::timeout(
            Duration::from_secs(1),
            harness.hub.close_lane(&lane_id),
        )
        .await
        .expect("close waited on the operation gate")
        .unwrap();
        assert_eq!(closed.closed, 1);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(
            operation.await.unwrap().unwrap_err().code,
            BrowserErrorCode::LaneClosedByUser
        );
    }

    #[tokio::test]
    async fn resource_policy_hot_update_changes_the_global_operation_limit() {
        let harness = harness();
        let lane_a = open(&harness.client, "limit-a").await;
        let lane_b = open(&harness.client, "limit-b").await;
        let mut policy = harness.hub.resource_policy().await;
        policy.max_active_operations = 1;
        harness.hub.set_resource_policy(policy).await.unwrap();

        let client_a = harness.client.clone();
        let first = tokio::spawn(async move { client_a.execute(&lane_a, navigate()).await });
        harness.probe.wait_for_active(1).await;
        let client_b = harness.client.clone();
        let second = tokio::spawn(async move { client_b.execute(&lane_b, navigate()).await });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(30),
                harness.probe.wait_for_entries(2),
            )
            .await
            .is_err(),
            "the tightened operation limit was not applied"
        );
        harness.probe.releases.add_permits(1);
        first.await.unwrap().unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_entries(2),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(1);
        second.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn agent_heavy_operations_use_two_weight_units_and_release_them() {
        let mut config = HubConfig::default();
        config.resource_policy.max_active_operations = 2;
        let harness = harness_with_config(config);
        let mut allowed_operations = harness.client.caller().allowed_operations.clone();
        allowed_operations.insert(BrowserOperationKind::Screenshot);
        let screenshot_client = client_for_surface(
            &harness,
            "runtime-heavy-screenshot",
            BrowserSurface::Native,
            allowed_operations,
        );
        let screenshot_lane = open(&screenshot_client, "heavy-screenshot").await;
        let regular_lane = open(&harness.client, "regular-after-heavy").await;

        let screenshot_task = tokio::spawn({
            let client = screenshot_client.clone();
            let lane = screenshot_lane.clone();
            async move { client.execute(&lane, screenshot()).await }
        });
        harness.probe.wait_for_active(1).await;

        let workload = harness
            .hub
            .resource_workload(harness.hub.resource_policy().await.lane_cold_start_bytes)
            .await;
        assert_eq!(workload.active_operation_permits, 0);
        assert_eq!(workload.active_heavy_operation_permits, 1);
        assert_eq!(
            harness
                .hub
                .inner
                .active_operation_weight
                .load(Ordering::Acquire),
            2
        );

        let regular_task = tokio::spawn({
            let client = harness.client.clone();
            async move { client.execute(&regular_lane, navigate()).await }
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(30),
                harness.probe.wait_for_entries(2),
            )
            .await
            .is_err(),
            "a regular operation bypassed the heavy operation's two-unit budget"
        );

        harness.probe.releases.add_permits(1);
        screenshot_task.await.unwrap().unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_entries(2),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(1);
        regular_task.await.unwrap().unwrap();

        assert_eq!(
            harness
                .hub
                .inner
                .active_operation_weight
                .load(Ordering::Acquire),
            0
        );
        let workload = harness
            .hub
            .resource_workload(harness.hub.resource_policy().await.lane_cold_start_bytes)
            .await;
        assert_eq!(workload.active_operation_permits, 0);
        assert_eq!(workload.active_heavy_operation_permits, 0);
    }

    #[tokio::test]
    async fn heavy_operation_is_not_starved_when_limit_is_below_nominal_weight() {
        let mut config = HubConfig::default();
        config.resource_policy.max_active_operations = 1;
        let harness = harness_with_config(config);
        let mut allowed_operations = harness.client.caller().allowed_operations.clone();
        allowed_operations.insert(BrowserOperationKind::Screenshot);
        let client = client_for_surface(
            &harness,
            "runtime-heavy-single-slot",
            BrowserSurface::Native,
            allowed_operations,
        );
        let lane_id = open(&client, "heavy-single-slot").await;

        let task = tokio::spawn({
            let client = client.clone();
            async move { client.execute(&lane_id, screenshot()).await }
        });
        harness.probe.wait_for_active(1).await;
        assert_eq!(
            harness
                .hub
                .inner
                .active_operation_weight
                .load(Ordering::Acquire),
            2,
            "an oversized heavy operation must retain its nominal weight while running"
        );

        let regular_lane = open(&harness.client, "regular-during-oversized").await;
        let regular_task = tokio::spawn({
            let client = harness.client.clone();
            async move { client.execute(&regular_lane, navigate()).await }
        });
        let mut raised_policy = harness.hub.resource_policy().await;
        raised_policy.max_active_operations = 2;
        harness.hub.set_resource_policy(raised_policy).await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(30),
                harness.probe.wait_for_entries(2),
            )
            .await
            .is_err(),
            "raising the limit must not admit regular work beside an oversized heavy operation"
        );

        harness.probe.releases.add_permits(1);
        task.await.unwrap().unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_entries(2),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(1);
        regular_task.await.unwrap().unwrap();
        assert_eq!(
            harness
                .hub
                .inner
                .active_operation_weight
                .load(Ordering::Acquire),
            0
        );
    }

    #[tokio::test]
    async fn invalid_resource_policy_is_rejected_and_keeps_previous_policy() {
        let harness = harness();
        let previous = harness.hub.resource_policy().await;
        let mut invalid = previous.clone();
        invalid.max_open_lanes = 0;

        let error = harness.hub.set_resource_policy(invalid).await.unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::InvalidCallerIdentity);
        assert_eq!(error.metadata["field"], "max_open_lanes");
        assert_eq!(harness.hub.resource_policy().await, previous);
    }

    #[tokio::test]
    async fn invalid_initial_resource_policy_falls_back_to_safe_default() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 0;
        let harness = harness_with_config(config);
        assert_eq!(
            harness.hub.resource_policy().await,
            ResourcePolicy::default()
        );
        assert_eq!(
            harness.hub.overview().await.capacity.max_open_lanes,
            ResourcePolicy::default().max_open_lanes
        );
    }

    #[tokio::test]
    async fn primary_host_failure_rebinds_all_lanes_and_requires_fresh_observe() {
        let harness = harness();
        let lane_a = open(&harness.client, "primary-restart-a").await;
        let lane_b = open(&harness.client, "primary-restart-b").await;
        let old_epoch = harness.client.status(&lane_a).await.unwrap().browser_epoch;
        let old_ref_generation = harness
            .client
            .status(&lane_b)
            .await
            .unwrap()
            .ref_generation;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);

        let error = harness
            .client
            .execute(&lane_a, navigate())
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted, "{error:?}");
        assert_eq!(error.metadata["old_epoch"], old_epoch);
        assert_eq!(error.metadata["new_epoch"], old_epoch + 1);
        assert_eq!(error.metadata["fresh_observe_required"], true);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);

        let snapshot_a = harness.client.status(&lane_a).await.unwrap();
        let snapshot_b = harness.client.status(&lane_b).await.unwrap();
        assert_eq!(snapshot_a.browser_epoch, old_epoch + 1);
        assert_eq!(snapshot_b.browser_epoch, old_epoch + 1);
        assert!(snapshot_a.tabs.is_empty());
        assert!(snapshot_b.tabs.is_empty());
        assert!(snapshot_a.active_frame_id.is_none());
        assert!(snapshot_b.active_frame_id.is_none());
        assert!(snapshot_a.ref_generation > 0);
        assert!(snapshot_b.ref_generation > 0);

        let sibling_error = harness
            .client
            .execute(&lane_b, navigate())
            .await
            .unwrap_err();
        assert_eq!(sibling_error.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(sibling_error.metadata["fresh_observe_required"], true);

        harness.probe.releases.add_permits(1);
        harness
            .client
            .execute(&lane_b, observe())
            .await
            .unwrap();
        assert!(
            !harness
                .hub
                .inner
                .lanes
                .read()
                .await
                .get(&lane_b)
                .unwrap()
                .fresh_observe_required
                .load(Ordering::Acquire)
        );
        let current_snapshot = harness.client.status(&lane_b).await.unwrap();
        assert_ne!(current_snapshot.ref_generation, old_ref_generation);

        let mut stale = navigate();
        stale.expected_browser_epoch = Some(old_epoch);
        let stale_error = harness.client.execute(&lane_b, stale).await.unwrap_err();
        assert_eq!(stale_error.code, BrowserErrorCode::StaleBrowserEpoch);

        let mut stale_ref = navigate();
        stale_ref.expected_browser_epoch = Some(current_snapshot.browser_epoch);
        stale_ref.ref_generation = Some(old_ref_generation);
        let stale_ref_error = harness
            .client
            .execute(&lane_b, stale_ref)
            .await
            .unwrap_err();
        assert_eq!(stale_ref_error.code, BrowserErrorCode::StaleLaneRef);
    }

    #[tokio::test]
    async fn failed_rebind_closes_already_prepared_lane_drivers() {
        let harness = harness();
        let lane_a = open(&harness.client, "rebind-cleanup-a").await;
        let lane_b = open(&harness.client, "rebind-cleanup-b").await;
        let lane_c = open(&harness.client, "rebind-cleanup-c").await;
        let old_epoch = harness.client.status(&lane_a).await.unwrap().browser_epoch;
        let calls_before_restart = harness
            .probe
            .open_lane_calls
            .load(Ordering::Acquire);
        // The replacement Host must successfully prepare one Lane, then fail
        // on the next one. HashMap iteration determines which logical Lane is
        // first, so the assertion intentionally checks cleanup cardinality,
        // not a particular Lane name.
        harness
            .probe
            .open_lane_failure_at
            .store(calls_before_restart + 2, Ordering::Release);
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);

        let error = harness
            .client
            .execute(&lane_a, navigate())
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(harness.client.status(&lane_a).await.unwrap().browser_epoch, old_epoch);
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            1,
            "the prepared replacement driver must be explicitly closed"
        );
        assert_eq!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .len(),
            1,
            "a failed prepared-driver close must remain under Hub authority"
        );
        harness.hub.sweep().await.unwrap();
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty(),
            "the lifecycle sweep must retry prepared-driver cleanup"
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 2);

        // Recovery can be retried after the synthetic failure is removed.
        harness
            .probe
            .open_lane_failure_at
            .store(0, Ordering::Release);
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);
        let retry_error = harness
            .client
            .execute(&lane_b, navigate())
            .await
            .unwrap_err();
        assert_eq!(retry_error.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(
            harness
                .client
                .status(&lane_b)
                .await
                .unwrap()
                .browser_epoch,
            old_epoch + 1
        );
        assert!(harness.client.status(&lane_c).await.is_ok());
    }

    #[tokio::test]
    async fn crawl_host_restart_does_not_change_primary_host_or_lane() {
        let harness = harness();
        let primary = open_identity(
            &harness.client,
            "primary-stable",
            BrowserIdentityMode::Primary,
        )
        .await;
        let crawl = open_identity(
            &harness.client,
            "crawl-restart",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let primary_epoch = harness.client.status(&primary).await.unwrap().browser_epoch;
        let crawl_epoch = harness.client.status(&crawl).await.unwrap().browser_epoch;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);

        let error = harness
            .client
            .execute(&crawl, navigate())
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(
            harness.client.status(&crawl).await.unwrap().browser_epoch,
            crawl_epoch + 1
        );
        assert_eq!(
            harness.client.status(&primary).await.unwrap().browser_epoch,
            primary_epoch
        );
        harness.probe.releases.add_permits(1);
        harness
            .client
            .execute(&primary, navigate())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn third_host_failure_in_window_opens_circuit_without_third_relaunch() {
        let harness = harness();
        let lane_id = open(&harness.client, "restart-circuit").await;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(3, Ordering::Release);

        let first = harness
            .client
            .execute(&lane_id, navigate())
            .await
            .unwrap_err();
        assert_eq!(first.code, BrowserErrorCode::BrowserRestarted);
        let second = harness
            .client
            .execute(&lane_id, observe())
            .await
            .unwrap_err();
        assert_eq!(second.code, BrowserErrorCode::BrowserRestarted);
        let third = harness
            .client
            .execute(&lane_id, observe())
            .await
            .unwrap_err();
        assert_eq!(third.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(third.metadata["circuit_open"], true);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 3);
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            3,
            "the Host that trips the circuit must also be shut down"
        );
        assert!(
            harness.hub.inner.host_slots.read().await.is_empty(),
            "the failed active Host slot must be removed when the circuit opens"
        );
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn third_host_failure_retains_failed_cleanup_and_sweep_retries_without_relaunch() {
        let harness = harness();
        let lane_id = open(&harness.client, "restart-circuit-cleanup").await;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(3, Ordering::Release);

        assert_eq!(
            harness
                .client
                .execute(&lane_id, navigate())
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserRestarted
        );
        assert_eq!(
            harness
                .client
                .execute(&lane_id, observe())
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserRestarted
        );
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);
        let third = harness
            .client
            .execute(&lane_id, observe())
            .await
            .unwrap_err();
        assert_eq!(third.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(third.metadata["circuit_open"], true);
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            3,
            "opening the circuit must not launch a replacement Host"
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 3);
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert_eq!(
            harness.hub.inner.orphaned_host_slots.lock().await.len(),
            1,
            "failed circuit-open shutdown must remain retryable"
        );
        let snapshot = harness.client.status(&lane_id).await.unwrap();
        assert_eq!(snapshot.lifecycle_state, LaneLifecycleState::Starting);
        assert!(
            snapshot.error_code.is_some(),
            "a recovery-blocked Lane must not look like a clean start in progress"
        );

        harness.hub.sweep().await.unwrap();
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            3,
            "cleanup retry must not relaunch while the circuit is open"
        );
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 4);
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn new_lane_cannot_bypass_an_open_host_circuit() {
        let harness = harness();
        let lane_id = open(&harness.client, "circuit-new-lane").await;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(3, Ordering::Release);

        assert_eq!(
            harness
                .client
                .execute(&lane_id, navigate())
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserRestarted
        );
        assert_eq!(
            harness
                .client
                .execute(&lane_id, observe())
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserRestarted
        );
        assert_eq!(
            harness
                .client
                .execute(&lane_id, observe())
                .await
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserUnavailable
        );
        let launches_before = harness.factory.launches.load(Ordering::Acquire);

        let error = harness
            .client
            .open(
                Some("circuit-new-lane-after-open"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(error.metadata["circuit_open"], true);
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            launches_before,
            "a new Lane must not relaunch a Host while its circuit is open"
        );
    }

    #[tokio::test]
    async fn generic_operation_failure_does_not_restart_host() {
        let harness = harness();
        let lane_id = open(&harness.client, "no-false-restart").await;
        let epoch = harness.client.status(&lane_id).await.unwrap().browser_epoch;
        harness
            .probe
            .generic_failures_remaining
            .store(1, Ordering::Release);

        let error = harness
            .client
            .execute(&lane_id, navigate())
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert_eq!(
            harness.client.status(&lane_id).await.unwrap().browser_epoch,
            epoch
        );
    }

    #[tokio::test]
    async fn late_old_epoch_success_is_fenced_after_sibling_restarts_host() {
        let harness = harness();
        let slow_lane = open(&harness.client, "late-old-success").await;
        let failing_lane = open(&harness.client, "restart-trigger").await;
        let client = harness.client.clone();
        let slow_lane_for_task = slow_lane.clone();
        let slow = tokio::spawn(async move {
            client.execute(&slow_lane_for_task, navigate()).await
        });
        harness.probe.wait_for_active(1).await;

        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);
        let trigger_error = harness
            .client
            .execute(&failing_lane, navigate())
            .await
            .unwrap_err();
        assert_eq!(trigger_error.code, BrowserErrorCode::BrowserRestarted);

        harness.probe.releases.add_permits(1);
        let late_error = slow.await.unwrap().unwrap_err();
        assert_eq!(late_error.code, BrowserErrorCode::BrowserRestarted);
        let snapshot = harness.client.status(&slow_lane).await.unwrap();
        assert!(snapshot.tabs.is_empty());
        assert!(snapshot.active_frame_id.is_none());
    }

    #[tokio::test]
    async fn hung_lane_cleanup_does_not_hold_inventory_or_scheduler_capacity() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "hung-cleanup").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close = tokio::spawn(async move { hub.close_lane(&closing_lane).await });
        harness.probe.wait_for_lane_closes(1).await;

        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);
        let replacement = tokio::time::timeout(
            Duration::from_secs(1),
            open(&harness.client, "replacement"),
        )
        .await
        .expect("replacement Lane waited behind detached cleanup");
        assert_ne!(replacement, lane_id);

        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        harness.probe.lane_close_release.add_permits(1);
        close.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn overview_exposes_zero_lane_active_host_and_cleanup_authority() {
        let mut config = HubConfig::default();
        config.headful = true;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "overview-pending-cleanup").await;
        let epoch = harness.client.status(&lane_id).await.unwrap().browser_epoch;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);
        let hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close = tokio::spawn(async move { hub.close_lane(&closing_lane).await });
        harness.probe.wait_for_lane_closes(1).await;

        let overview = harness.hub.overview().await;
        assert_eq!(overview.total_lanes, 0);
        assert_eq!(overview.managed_host_count, 1);
        assert_eq!(overview.pending_cleanup_count, 1);
        assert_eq!(overview.hosts.len(), 1);
        assert_eq!(overview.hosts[0].lane_count, 0);
        assert_eq!(overview.hosts[0].epoch, epoch);
        assert!(overview.hosts[0].headful);
        let owner = harness.hub.overview_for_user("user-1").await;
        assert_eq!(owner.managed_host_count, 0);
        assert_eq!(owner.pending_cleanup_count, 1);
        assert!(owner.hosts.is_empty());
        let foreign = harness.hub.overview_for_user("user-2").await;
        assert_eq!(foreign.managed_host_count, 0);
        assert_eq!(foreign.pending_cleanup_count, 0);

        harness.probe.lane_close_release.add_permits(1);
        close.await.unwrap().unwrap();
        let final_overview = harness.hub.overview().await;
        assert_eq!(final_overview.managed_host_count, 0);
        assert_eq!(final_overview.pending_cleanup_count, 0);
    }

    #[tokio::test]
    async fn failed_last_lane_cleanup_is_resolved_by_authoritative_host_shutdown() {
        let harness = harness();
        let lane_id = open(&harness.client, "retry-cleanup").await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let result = harness.hub.close_lane(&lane_id).await.unwrap();
        assert_eq!(result.closed, 1);
        assert!(!result.already_closed);
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        harness.hub.sweep().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn failed_lane_cleanup_on_shared_primary_host_never_kills_sibling() {
        let harness = harness();
        let first = open(&harness.client, "shared-cleanup-a").await;
        let second = open(&harness.client, "shared-cleanup-b").await;
        let second_epoch = harness.client.status(&second).await.unwrap().browser_epoch;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let error = tokio::time::timeout(Duration::from_secs(1), harness.hub.close_lane(&first))
            .await
            .expect("shared-Lane close did not return its terminal cleanup error")
            .unwrap_err();
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(error.metadata["remaining_lane_count"], 0);
        assert_eq!(error.metadata["remaining_cleanup_count"], 1);
        assert_eq!(error.metadata["remaining_managed_host_count"], 0);
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "a failed target cleanup must never kill a shared Host"
        );
        assert_eq!(
            harness.client.status(&second).await.unwrap().browser_epoch,
            second_epoch
        );
        harness.probe.releases.add_permits(1);
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.client.execute(&second, navigate()),
        )
        .await
        .expect("sibling execution stayed blocked")
        .expect("the sibling Lane must remain usable");

        let foreign_overview = harness.hub.overview_for_user("user-2").await;
        assert_eq!(foreign_overview.managed_host_count, 0);
        assert_eq!(foreign_overview.pending_cleanup_count, 0);
        let owner_overview = harness.hub.overview_for_user("user-1").await;
        assert_eq!(owner_overview.managed_host_count, 1);
        assert_eq!(owner_overview.pending_cleanup_count, 1);

        let drained = tokio::time::timeout(Duration::from_secs(2), harness.hub.close_all())
            .await
            .expect("installation drain hung after shared target cleanup failure")
            .unwrap();
        assert_eq!(drained.closed, 1);
        assert_eq!(drained.remaining_lane_count, 0);
        assert_eq!(drained.remaining_cleanup_count, 0);
        assert_eq!(drained.remaining_managed_host_count, 0);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn scoped_close_result_does_not_expose_foreign_global_resources() {
        let harness = harness();
        let own = open(&harness.client, "scoped-close-own").await;
        let foreign = open_for_user(
            &harness,
            "user-2",
            "runtime-scoped-close-foreign",
            "scoped-close-foreign",
            BrowserIdentityMode::Primary,
        )
        .await;

        let result = harness.hub.close_lane(&own).await.unwrap();
        assert_eq!(result.closed, 1);
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        assert_eq!(harness.hub.overview().await.total_lanes, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&foreign).await.is_some());
    }

    #[tokio::test]
    async fn failed_last_lane_cleanup_and_failed_host_shutdown_retains_authority() {
        let harness = harness();
        let lane_id = open(&harness.client, "failed-target-and-host").await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);

        let error = harness.hub.close_lane(&lane_id).await.unwrap_err();
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(error.metadata["remaining_lane_count"], 0);
        assert_eq!(error.metadata["remaining_cleanup_count"], 1);
        assert_eq!(error.metadata["remaining_managed_host_count"], 0);
        let overview = harness.hub.overview().await;
        assert!(overview.pending_cleanup_count > 0);
        assert_eq!(overview.managed_host_count, 1);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);

        let drained = tokio::time::timeout(Duration::from_secs(2), harness.hub.close_all())
            .await
            .expect("installation drain hung retrying failed Host shutdown")
            .unwrap();
        assert_eq!(drained.remaining_lane_count, 0);
        assert_eq!(drained.remaining_cleanup_count, 0);
        assert_eq!(drained.remaining_managed_host_count, 0);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn lane_close_panic_is_resolved_by_authoritative_host_shutdown() {
        let harness = harness();
        let lane_id = open(&harness.client, "panic-cleanup").await;
        harness
            .probe
            .lane_close_panics_remaining
            .store(1, Ordering::Release);

        let result = tokio::time::timeout(Duration::from_secs(1), harness.hub.close_lane(&lane_id))
            .await
            .expect("driver.close panic left the close caller pending")
            .unwrap();
        assert_eq!(result.closed, 1);
        assert_eq!(result.remaining_lane_count, 0);
        assert_eq!(result.remaining_cleanup_count, 0);
        assert_eq!(result.remaining_managed_host_count, 0);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(
            harness
                .probe
                .lane_close_completions
                .load(Ordering::Acquire),
            0
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_wait_timeout_keeps_single_flight_running_for_later_waiter() {
        let harness = harness();
        let lane_id = open(&harness.client, "slow-cleanup").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close = tokio::spawn(async move { hub.close_lane(&closing_lane).await });
        harness.probe.wait_for_lane_closes(1).await;

        tokio::time::advance(LANE_CLEANUP_WAITER_TIMEOUT).await;
        tokio::task::yield_now().await;
        let error = close
            .await
            .expect("close task panicked")
            .expect_err("slow cleanup should time out for the caller");
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(error.metadata["cleanup_wait_timeout"], true);
        assert_eq!(
            error.metadata["timeout_ms"],
            LANE_CLEANUP_WAITER_TIMEOUT.as_millis() as u64
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(
            harness
                .probe
                .lane_close_completions
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "a waiter timeout must not race the still-running target close by killing its Host"
        );

        let cleanup_id = harness
            .hub
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .first()
            .expect("slow close must retain cleanup authority")
            .cleanup_id;
        let follower_hub = harness.hub.clone();
        let follower =
            tokio::spawn(async move { follower_hub.attempt_pending_lane_cleanup(cleanup_id).await });
        tokio::task::yield_now().await;
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            1,
            "a later waiter must join the still-running authoritative cleanup flight"
        );

        harness.probe.lane_close_release.add_permits(1);
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_lane_close_completions(1),
        )
        .await
        .expect("the original driver.close did not finish after its block was released");
        follower.await.unwrap().unwrap();
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sweep_never_retires_a_host_while_its_target_cleanup_is_still_running() {
        let harness = harness();
        let lane_id = open(&harness.client, "sweep-pending-cleanup").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        // The close times out for its caller but the driver close keeps
        // running under Hub authority; the lane is already out of inventory.
        let hub = harness.hub.clone();
        let closing = lane_id.clone();
        let close = tokio::spawn(async move { hub.close_lane(&closing).await });
        harness.probe.wait_for_lane_closes(1).await;
        tokio::time::advance(LANE_CLEANUP_WAITER_TIMEOUT).await;
        tokio::task::yield_now().await;
        let error = close
            .await
            .expect("close task panicked")
            .expect_err("blocked cleanup must time out for its caller");
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert!(harness.hub.list_lanes().await.is_empty());

        // The key has no live lanes, but the periodic sweep must not hard-stop
        // the process while the retained target cleanup is still talking to it.
        let _ = harness.hub.sweep().await;
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "sweep retired the Host while its Lane cleanup was still in flight"
        );
        assert_eq!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .len(),
            1
        );

        // Once the close settles, retirement converges normally.
        harness.probe.lane_close_release.add_permits(1);
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_lane_close_completions(1),
        )
        .await
        .expect("the blocked driver close did not finish after release");
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_shutdowns(1),
        )
        .await
        .expect("the empty Host was not retired after cleanup settled");
        harness.hub.sweep().await.unwrap();
        assert_eq!(
            harness.hub.remaining_resources().await,
            RemainingResources::default()
        );
    }

    #[tokio::test]
    async fn cancelled_close_keeps_driver_cleanup_authority() {
        let harness = harness();
        let lane_id = open(&harness.client, "cancelled-cleanup").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let close = tokio::spawn(async move { hub.close_lane(&lane_id).await });
        harness.probe.wait_for_lane_closes(1).await;
        close.abort();
        let _ = close.await;

        assert_eq!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .len(),
            1
        );
        harness
            .probe
            .block_lane_close
            .store(false, Ordering::Release);
        harness.probe.lane_close_release.add_permits(1);
        harness.hub.sweep().await.unwrap();
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cleanup_cancellation_is_hub_owned_and_single_flight() {
        let harness = harness();
        let lane_id = open(&harness.client, "cleanup-flight").await;
        harness
            .probe
            .block_lane_close
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let closing_lane = lane_id.clone();
        let close = tokio::spawn(async move { hub.close_lane(&closing_lane).await });
        harness.probe.wait_for_lane_closes(1).await;
        close.abort();
        assert!(close.await.unwrap_err().is_cancelled());

        let cleanup_id = harness
            .hub
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .first()
            .expect("cancelled close must retain cleanup authority")
            .cleanup_id;
        let first_hub = harness.hub.clone();
        let first =
            tokio::spawn(async move { first_hub.attempt_pending_lane_cleanup(cleanup_id).await });
        let second_hub = harness.hub.clone();
        let second =
            tokio::spawn(async move { second_hub.attempt_pending_lane_cleanup(cleanup_id).await });
        tokio::task::yield_now().await;
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            1,
            "followers must join the existing cleanup attempt"
        );

        harness.probe.lane_close_release.add_permits(1);
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert!(
            harness
                .hub
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn failed_shutdown_retains_host_authority_for_explicit_retry() {
        let harness = harness();
        let _ = open(&harness.client, "shutdown-retry").await;
        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);

        // The explicit close inside platform shutdown performs the immediate
        // Host retirement, so it is the caller that observes the real first
        // shutdown failure. The retained authority is retried by the same
        // shutdown pass; the terminal result still reports that first error
        // instead of letting the background retry consume it.
        assert!(harness.hub.shutdown().await.is_err());
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert!(
            harness
                .hub
                .inner
                .retiring_host_slots
                .lock()
                .await
                .is_empty()
        );

        // A repeated explicit shutdown converges without another attempt.
        harness.hub.shutdown().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn shutdown_is_single_flight_and_reports_the_same_terminal_result() {
        let harness = harness();
        let _ = open(&harness.client, "default").await;
        harness.hub.shutdown().await.unwrap();
        harness.hub.shutdown().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.list_lanes().await.is_empty());
    }

    #[tokio::test]
    async fn cancelling_an_operation_cannot_leave_lane_activity_stuck() {
        let harness = harness();
        let lane_id = open(&harness.client, "cancelled-activity").await;
        let client = harness.client.clone();
        let operation_lane = lane_id.clone();
        let operation =
            tokio::spawn(async move { client.execute(&operation_lane, navigate()).await });
        harness.probe.wait_for_active(1).await;
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());

        let lane = harness
            .hub
            .list_lanes()
            .await
            .into_iter()
            .find(|lane| lane.lane_id == lane_id)
            .unwrap();
        assert_eq!(lane.active_operation_count, 0);
        assert_eq!(harness.probe.active.load(Ordering::Acquire), 0);
    }

}
