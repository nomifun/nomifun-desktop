use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use futures_util::FutureExt;
use tokio::sync::{Mutex, Notify, OnceCell, RwLock, SetError, broadcast};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::cleanup_budget::{
    CleanupBudget, CleanupBudgetError, CleanupBudgetSaturation, CleanupBudgetScope,
    CleanupBudgetToken,
};
use crate::identity::IdentityGenerationCoordinator;
use crate::{
    Admission, BrowserCapacitySnapshot, BrowserErrorCode, BrowserHostDriver,
    BrowserHostFactory, BrowserHostId, BrowserHostSnapshot, BrowserIdentityMode,
    BrowserInventoryEvent, BrowserLaneDriver, BrowserLaneId, BrowserLaneScheduler,
    BrowserLaneSnapshot, BrowserOperation, BrowserOperationKind, BrowserOperationResult,
    BrowserOverview, BrowserPlatformError, BrowserTaskDownloadAuthority,
    BrowserTaskDownloadReservation, BrowserTaskTabAuthority, BrowserTaskTabReservation,
    CallerIdentity, CanonicalIdentitySnapshot, BrowserVisibility,
    BrowserPresentationIntent, BrowserVisibilityPolicy, may_escalate_lane_to_headful,
    CapturedIdentitySnapshot, Clock, CloseResult,
    DriverOperationContext, HostLaunchCleanupLease, HostLaunchCleanupTicket, HostLaunchRequest,
    IdentitySnapshotPayload,
    HostCircuitBreaker, HostRestartTransition, LaneFreezeOutcome, LaneKey, LaneLaunchRequest,
    LaneLifecycleState, LanePriority, OperationContext, OwnerLease, OwnerLeaseId,
    OwnerLeaseService,
    PerKeyHostRestartSingleFlight, PromotionPolicy, ResourceDecision, ResourcePolicy,
    ResourcePressureState, ResourceTelemetry, ResourceWorkload, SchedulerConfig, SnapshotCoverage,
    SystemClock, stale_browser_epoch_error,
};

const EVENT_BUFFER: usize = 256;
const LANE_CLEANUP_WAITER_TIMEOUT: Duration = Duration::from_secs(6);
const LANE_CLEANUP_HARD_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_BATCH_WAIT_TIMEOUT: Duration = Duration::from_secs(7);
const MAX_CONCURRENT_LANE_CLEANUPS: usize = 8;
const MAX_CONCURRENT_HOST_CLEANUPS: usize = 4;
// On Windows the engine may legitimately spend up to 30 seconds waiting for
// DevToolsActivePort, followed by its first bounded CDP initialization command.
// The platform must not cancel that engine-owned cold start first. A caller
// waiting on the initialization gate needs the same budget because it is
// joining that exact in-flight Host launch.
const HOST_INITIALIZATION_GATE_TIMEOUT: Duration = Duration::from_secs(65);
const HOST_INITIALIZATION_LAUNCH_TIMEOUT: Duration = Duration::from_secs(65);
// A Host adapter must not retain a Lane-start flight forever. This exceeds
// the engine's ordinary CDP command and attach budgets, so healthy slow starts
// keep their full window while a broken adapter is eventually fenced.
const HOST_LANE_OPEN_TIMEOUT: Duration = Duration::from_secs(90);
// Retained cleanup authority is driven by the Hub itself. The worker owns
// only a Weak reference between attempts, so permanent external failures do
// not keep an otherwise-dropped Hub alive.
const AUTONOMOUS_CLEANUP_RETRY_INITIAL: Duration = Duration::from_secs(1);
const AUTONOMOUS_CLEANUP_RETRY_MAX: Duration = Duration::from_secs(30);
// Exact handoffs only accept Lane ids still retained by the scheduler. Their
// ledger is therefore bounded by the independently validated active + queued
// admission ceilings, rather than by user/task cardinality.
const MAX_EXACT_LANE_CLEANUP_HANDOFFS: usize =
    2 * (crate::MAX_OPEN_LANES + crate::MAX_GLOBAL_QUEUE);
const MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS: usize =
    2 * (crate::MAX_TASK_OPEN_LANES + crate::MAX_OWNER_QUEUE);
const OPERATION_QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const OPERATION_ADMISSION_MULTIPLIER: usize = 4;
const MAX_LANE_OPERATION_ADMISSIONS: usize = 8;
/// Hard task-lifetime download boundaries. These are deliberately owned by
/// the Hub rather than a Chromium Host so Host/runtime replacement cannot
/// reset a task's disk-output budget.
pub const MAX_TASK_COMPLETED_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_TASK_SINGLE_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TASK_COMPLETED_DOWNLOAD_FILES: usize = 256;
pub const MAX_TASK_ACTIVE_DOWNLOADS: usize = 4;
/// Process-lifetime structural bound for sticky task download ledgers.
///
/// A completed family is never evicted by TTL/LRU or an owner-generation gap:
/// doing so would let a caller reset its byte budget by rotating runtimes. New
/// families fail closed once this many distinct families have consumed output.
pub const MAX_RETAINED_COMPLETED_DOWNLOAD_FAMILIES: usize = 4096;
const TASK_RECLAIM_IDLE_EXPANSION_STREAK: u8 = 2;
const TASK_RECLAIM_IDLE_ANY_STREAK: u8 = 3;
const TASK_RECLAIM_ACTIVE_EXPANSION_STREAK: u8 = 4;
const TASK_RECLAIM_ACTIVE_ANY_STREAK: u8 = 5;
/// Consecutive over-budget samples required before *any* reclaim stage applies.
///
/// Task attribution on a shared Host is an estimate, and a browser legitimately
/// sits at a high steady state: a few media-heavy tabs cost the same bytes
/// whether or not anything is leaking. Escalating on the first sample made a
/// normal session reclaimable roughly one sampling period (5s) after it finished
/// loading its pages, because the severity/confidence accelerators alone reach
/// an eligible stage with a streak of 1.
///
/// Requiring sustained overage is what separates "expensive" from "growing".
/// The accelerators still shorten the *escalation* once this floor is met, so a
/// genuine runaway is reclaimed promptly; it just cannot happen instantly.
const TASK_RECLAIM_MIN_SUSTAINED_SAMPLES: u8 = 3;
// A shared Chromium Host does not expose a trustworthy target -> native RSS
// mapping.  Task attribution can therefore miss a one-page renderer leak when
// many unrelated tasks share Primary/Anonymous.  Do not react to one noisy OS
// sample, but also never let measured managed-Chromium RSS remain above the
// hardware-derived browser ratio indefinitely merely because every task still
// owns its protected first Lane.
const RESOURCE_EMERGENCY_CRITICAL_SAMPLES: u64 = 3;
// CPU is a sampled Host-level signal, not a task attribution boundary. Only
// converge after the whole machine is critically busy and exact managed
// Chromium process trees account for at least half of total machine capacity.
// This avoids restarting Browser Use merely because an unrelated application
// is consuming CPU while still giving runaway page JS a physical endpoint.
const RESOURCE_EMERGENCY_SYSTEM_CPU_PRESSURE: f64 = 0.90;
const RESOURCE_EMERGENCY_MANAGED_CPU_PRESSURE: f64 = 0.50;
// Host replacement first shuts down the old process, then performs the same
// bounded cold start and rebinds its Lanes.
const HOST_RESTART_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(75);
const HOST_SHUTDOWN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_RETIREMENT_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_FINALIZATION_WAITER_TIMEOUT: Duration = Duration::from_secs(7);
const PENDING_LANE_START_WAIT_TIMEOUT: Duration = Duration::from_secs(6);
const PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);
const ANONYMOUS_PROFILE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const PRIMARY_PROFILE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Footprint limits for the stable Primary profile.
///
/// Primary identity data is intentionally persistent. Crossing this boundary
/// therefore fences Primary browsing for the rest of the application
/// lifetime and stops the exact Host, but never rotates or deletes the
/// profile. The user can then clean site data or sign in again deliberately.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PrimaryProfilePolicy {
    pub max_bytes: u64,
    pub max_entries: u64,
    pub sample_interval_ms: u64,
    pub sample_navigation_interval: u64,
}

impl Default for PrimaryProfilePolicy {
    fn default() -> Self {
        Self {
            max_bytes: 2 * 1024 * 1024 * 1024,
            max_entries: 100_000,
            sample_interval_ms: 15_000,
            // A navigation can grow multiple origin stores at once. Sample
            // every navigation while still avoiding per-action filesystem
            // walks for ordinary observe/click/type operations.
            sample_navigation_interval: 1,
        }
    }
}

impl PrimaryProfilePolicy {
    fn normalize(&mut self) {
        self.max_bytes = self.max_bytes.clamp(1, 2 * 1024 * 1024 * 1024);
        self.max_entries = self.max_entries.clamp(1, 100_000);
        self.sample_interval_ms = self.sample_interval_ms.clamp(1, 15_000);
        self.sample_navigation_interval = self.sample_navigation_interval.clamp(1, 1);
    }
}

/// Lifecycle and footprint limits for the one shared Anonymous Host.
///
/// This is deliberately Host-scoped rather than task-scoped: Anonymous Lanes
/// from sibling task families share one Chromium profile. Crossing a limit
/// fences and rotates that exact Host, never an arbitrary sibling Lane.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AnonymousProfilePolicy {
    pub max_bytes: u64,
    pub max_entries: u64,
    pub max_age_ms: u64,
    pub max_navigations: u64,
    pub sample_interval_ms: u64,
    pub sample_navigation_interval: u64,
}

impl Default for AnonymousProfilePolicy {
    fn default() -> Self {
        Self {
            // Keep the live admission ceiling comfortably below the profile
            // recovery walk's entry budget. Bytes include all normal Chromium
            // cache and origin-storage files visible in the exact profile.
            max_bytes: 512 * 1024 * 1024,
            max_entries: 50_000,
            max_age_ms: 30 * 60_000,
            max_navigations: 256,
            sample_interval_ms: 15_000,
            sample_navigation_interval: 8,
        }
    }
}

impl AnonymousProfilePolicy {
    fn normalize(&mut self) {
        // These are installation safety maxima, not user-tunable aggregate
        // limits. A malformed trusted config must not turn the lifecycle
        // boundary into `u64::MAX` and silently restore unbounded growth.
        self.max_bytes = self.max_bytes.clamp(1, 512 * 1024 * 1024);
        self.max_entries = self.max_entries.clamp(1, 50_000);
        self.max_age_ms = self.max_age_ms.clamp(1, 30 * 60_000);
        self.max_navigations = self.max_navigations.clamp(1, 256);
        self.sample_interval_ms = self.sample_interval_ms.clamp(1, 15_000);
        self.sample_navigation_interval = self.sample_navigation_interval.clamp(1, 8);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubConfig {
    pub resource_policy: ResourcePolicy,
    #[serde(default)]
    pub anonymous_profile_policy: AnonymousProfilePolicy,
    #[serde(default)]
    pub primary_profile_policy: PrimaryProfilePolicy,
    pub owner_lease_ttl_ms: u64,
    pub headful: bool,
    /// The user's visibility *policy*, which governs whether the platform may
    /// resolve visibility per Lane at all.
    ///
    /// `headful` above remains the launch mechanism. This is the separate axis
    /// that decides who owns that mechanism: pinned by the user, or delegated to
    /// the trusted host. Defaults to `Auto` so a config predating the field
    /// behaves like a fresh install.
    #[serde(default)]
    pub visibility_policy: BrowserVisibilityPolicy,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            resource_policy: ResourcePolicy::default(),
            anonymous_profile_policy: AnonymousProfilePolicy::default(),
            primary_profile_policy: PrimaryProfilePolicy::default(),
            owner_lease_ttl_ms: 5 * 60_000,
            headful: false,
            visibility_policy: BrowserVisibilityPolicy::default(),
        }
    }
}

fn operation_admission_limits(policy: &ResourcePolicy) -> (usize, usize, usize) {
    let global = policy
        .max_active_operations
        .max(1)
        .saturating_mul(OPERATION_ADMISSION_MULTIPLIER);
    let task = policy
        .max_task_active_operations
        .min(policy.max_active_operations)
        .max(1)
        .saturating_mul(OPERATION_ADMISSION_MULTIPLIER)
        .min(global);
    let lane = task.min(MAX_LANE_OPERATION_ADMISSIONS).max(1);
    (global, task, lane)
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
struct HostCleanupAuthorityKey {
    host_key: HostKey,
    browser_epoch: u64,
}

type HubCleanupBudget =
    CleanupBudget<BrowserLaneId, HostCleanupAuthorityKey, HostKey>;
type LaneCleanupBudgetToken =
    CleanupBudgetToken<BrowserLaneId, HostCleanupAuthorityKey>;
type HostCleanupBudgetToken =
    CleanupBudgetToken<BrowserLaneId, HostCleanupAuthorityKey>;

#[derive(Clone, Debug)]
struct ExactLaneCleanupAuthority {
    user_id: String,
    owner_lease_id: OwnerLeaseId,
    task_id: String,
    family_id: String,
}

#[derive(Default)]
struct ExactLaneCleanupLedger {
    entries: HashMap<BrowserLaneId, ExactLaneCleanupAuthority>,
    task_counts: HashMap<String, usize>,
    family_counts: HashMap<String, usize>,
}

impl ExactLaneCleanupLedger {
    fn insert(
        &mut self,
        lane_id: BrowserLaneId,
        authority: ExactLaneCleanupAuthority,
    ) -> Result<(), BrowserPlatformError> {
        if let Some(existing) = self.entries.get(&lane_id) {
            if existing.owner_lease_id == authority.owner_lease_id
                && existing.task_id == authority.task_id
                && existing.family_id == authority.family_id
            {
                return Ok(());
            }
            return Err(exact_lane_cleanup_authority_mismatch_error(&lane_id));
        }
        let task_count = self.task_counts.get(&authority.task_id).copied().unwrap_or(0);
        let family_count = self
            .family_counts
            .get(&authority.family_id)
            .copied()
            .unwrap_or(0);
        if self.entries.len() >= MAX_EXACT_LANE_CLEANUP_HANDOFFS
            || task_count >= MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS
            || family_count >= MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS
        {
            return Err(exact_lane_cleanup_capacity_error(
                &lane_id,
                self.entries.len(),
                task_count,
                family_count,
            ));
        }
        *self.task_counts.entry(authority.task_id.clone()).or_default() += 1;
        *self
            .family_counts
            .entry(authority.family_id.clone())
            .or_default() += 1;
        self.entries.insert(lane_id, authority);
        Ok(())
    }

    fn remove(&mut self, lane_id: &BrowserLaneId) {
        let Some(authority) = self.entries.remove(lane_id) else {
            return;
        };
        decrement_exact_cleanup_count(&mut self.task_counts, &authority.task_id);
        decrement_exact_cleanup_count(&mut self.family_counts, &authority.family_id);
    }
}

fn decrement_exact_cleanup_count(counts: &mut HashMap<String, usize>, key: &str) {
    if let Some(count) = counts.get_mut(key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            counts.remove(key);
        }
    }
}

#[derive(Clone, Debug)]
struct OwnerCleanupTarget {
    user_id: String,
    /// Exact runtime cleanup attribution.
    task_id: String,
    /// User-visible task-family quota attribution.
    family_id: String,
    lane_id: BrowserLaneId,
    host_key: HostKey,
    browser_epoch: u64,
    /// No Lane driver was ever returned, so target-local close cannot prove
    /// that a possibly-created physical target is gone. Only exact Host stop
    /// may discharge this authority.
    requires_host_stop: bool,
}

impl PartialEq for OwnerCleanupTarget {
    fn eq(&self, other: &Self) -> bool {
        self.user_id == other.user_id
            && self.task_id == other.task_id
            && self.family_id == other.family_id
            && self.lane_id == other.lane_id
            && self.host_key == other.host_key
            && self.browser_epoch == other.browser_epoch
    }
}

impl Eq for OwnerCleanupTarget {}

impl Hash for OwnerCleanupTarget {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.user_id.hash(state);
        self.task_id.hash(state);
        self.family_id.hash(state);
        self.lane_id.hash(state);
        self.host_key.hash(state);
        self.browser_epoch.hash(state);
    }
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

    fn deterministic_key(&self) -> (u8, u64, &str) {
        let identity_rank = match self.identity_mode {
            BrowserIdentityMode::Primary => 0,
            BrowserIdentityMode::AuthenticatedReplica => 1,
            BrowserIdentityMode::Anonymous => 2,
            BrowserIdentityMode::Isolated => 3,
        };
        (
            identity_rank,
            self.identity_generation,
            self.isolation_lane_id
                .as_ref()
                .map(BrowserLaneId::as_str)
                .unwrap_or(""),
        )
    }
}

#[derive(Clone, Copy)]
enum PressureCloseFilter {
    AnyIdle,
    FrozenPressureReclaim,
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

/// Hub-owned completion for one physical Host launch attempt. Callers may time
/// out or be cancelled while waiting, but the launch future itself keeps
/// running and publishes either a concrete driver or a terminal factory error.
/// Exact Host cleanup consumes this same flight before it may prove that a
/// driver-less slot owns no process.
struct HostLaunchFlight {
    result: OnceLock<Result<(), BrowserPlatformError>>,
    changed: Notify,
}

/// One Host-owned managed-profile sample. The Hub task, rather than the
/// request which happened to make sampling due, owns the driver future. This
/// prevents request cancellation from releasing admission to a second native
/// walk while the first walk is still running.
struct ProfileSampleFlight {
    result: OnceLock<Result<Option<crate::BrowserProfileFootprint>, BrowserPlatformError>>,
    changed: Notify,
}

impl ProfileSampleFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            changed: Notify::new(),
        }
    }

    fn complete(
        &self,
        result: Result<Option<crate::BrowserProfileFootprint>, BrowserPlatformError>,
    ) {
        if self.result.set(result).is_ok() {
            self.changed.notify_waiters();
        }
    }

    async fn wait(
        &self,
    ) -> Result<Option<crate::BrowserProfileFootprint>, BrowserPlatformError> {
        loop {
            let changed = self.changed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            changed.await;
        }
    }
}

impl HostLaunchFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            changed: Notify::new(),
        }
    }

    fn complete(&self, result: Result<(), BrowserPlatformError>) {
        if self.result.set(result).is_ok() {
            self.changed.notify_waiters();
        }
    }

    async fn wait(&self) -> Result<(), BrowserPlatformError> {
        loop {
            let changed = self.changed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            changed.await;
        }
    }
}

struct HostSlot {
    driver: OnceCell<Arc<dyn BrowserHostDriver>>,
    /// At most one Hub-owned launch task for this slot. Unlike the caller's
    /// wait, this authority is never aborted at the cold-start timeout.
    launch_flight: StdMutex<Option<Arc<HostLaunchFlight>>>,
    /// Exact process/profile cleanup proof paired with the launch request.
    /// Factory/engine code owns counted leases; the Hub retains this observer
    /// until their final authority proves cleanup complete.
    launch_cleanup_ticket: StdMutex<Option<HostLaunchCleanupTicket>>,
    initialization_gate: Mutex<()>,
    shutdown_gate: Mutex<()>,
    /// Read-held across target/operation admission. Anonymous profile hygiene
    /// takes the write side after publishing its durable fence, draining work
    /// that won admission before exact Host shutdown begins.
    admission_gate: Arc<RwLock<()>>,
    shutdown_complete: AtomicBool,
    retired: AtomicBool,
    headful: AtomicBool,
    epoch: u64,
    created_at_ms: u64,
    profile_navigation_count: AtomicU64,
    profile_sample_completed: AtomicBool,
    last_profile_sample_ms: AtomicU64,
    last_profile_sample_navigation: AtomicU64,
    /// At most one active or completed sample. A completed flight remains as
    /// a one-item mailbox until a waiter consumes it, so aborting every waiter
    /// cannot lose a limit/error result or start an overlapping scan.
    profile_sample_flight: StdMutex<Option<Arc<ProfileSampleFlight>>>,
}

impl HostSlot {
    fn new(epoch: u64, headful: bool, created_at_ms: u64) -> Self {
        Self {
            driver: OnceCell::new(),
            launch_flight: StdMutex::new(None),
            launch_cleanup_ticket: StdMutex::new(None),
            initialization_gate: Mutex::new(()),
            shutdown_gate: Mutex::new(()),
            admission_gate: Arc::new(RwLock::new(())),
            shutdown_complete: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            headful: AtomicBool::new(headful),
            epoch,
            created_at_ms,
            profile_navigation_count: AtomicU64::new(0),
            profile_sample_completed: AtomicBool::new(false),
            last_profile_sample_ms: AtomicU64::new(created_at_ms),
            last_profile_sample_navigation: AtomicU64::new(0),
            profile_sample_flight: StdMutex::new(None),
        }
    }

    fn is_headful(&self) -> bool {
        self.headful.load(Ordering::Acquire)
    }

    fn get(&self) -> Option<&Arc<dyn BrowserHostDriver>> {
        self.driver.get()
    }

    fn claim_profile_sample_if_due(
        &self,
        sample_interval_ms: u64,
        sample_navigation_interval: u64,
        now_ms: u64,
        navigation_count: u64,
        force_initial_sample: bool,
    ) -> Option<(Arc<ProfileSampleFlight>, bool)> {
        let mut current = self
            .profile_sample_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(flight) = current.as_ref() {
            return Some((Arc::clone(flight), false));
        }
        // Recheck while holding the same synchronous authority used to publish
        // a new flight. A just-completed sample may have updated these atomics
        // after this caller's optimistic check; that must not create a
        // redundant successor scan.
        let sample_due = (force_initial_sample
            && !self.profile_sample_completed.load(Ordering::Acquire))
            || now_ms
            .saturating_sub(self.last_profile_sample_ms.load(Ordering::Acquire))
            >= sample_interval_ms
            || navigation_count.saturating_sub(
                self.last_profile_sample_navigation.load(Ordering::Acquire),
            ) >= sample_navigation_interval;
        if !sample_due {
            return None;
        }
        let flight = Arc::new(ProfileSampleFlight::new());
        *current = Some(Arc::clone(&flight));
        Some((flight, true))
    }

    fn consume_profile_sample(&self, flight: &Arc<ProfileSampleFlight>) {
        let mut current = self
            .profile_sample_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, flight))
        {
            current.take();
        }
    }

    async fn get_or_try_init<F, Fut>(
        self: &Arc<Self>,
        init: F,
    ) -> Result<&Arc<dyn BrowserHostDriver>, BrowserPlatformError>
    where
        F: FnOnce(HostLaunchCleanupLease) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError>>
            + Send
            + 'static,
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
        if let Some(host) = self.driver.get() {
            return Ok(host);
        }
        if let Some(ticket) = self.current_launch_cleanup_ticket() {
            if !ticket.is_complete() {
                self.retire();
                return Err(host_launch_cleanup_pending_error(self.epoch, None));
            }
            self.clear_completed_launch_cleanup_ticket();
        }

        // The caller's cold-start timeout must never be the owner of the
        // factory future. A timeout used to drop that future and then let
        // `shutdown_retired` treat `driver == None` as proof that no Chromium
        // existed. The Hub now owns one detached flight until the factory has
        // either published the exact driver or returned a terminal error.
        let flight = {
            let mut launch_flight = self
                .launch_flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(flight) = launch_flight.as_ref() {
                Arc::clone(flight)
            } else {
                let flight = Arc::new(HostLaunchFlight::new());
                let (cleanup_ticket, cleanup_lease) = HostLaunchCleanupTicket::new();
                *self
                    .launch_cleanup_ticket
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(cleanup_ticket);
                *launch_flight = Some(Arc::clone(&flight));
                let slot = Arc::clone(self);
                let task_flight = Arc::clone(&flight);
                tokio::spawn(async move {
                    let result = AssertUnwindSafe(init(cleanup_lease)).catch_unwind().await;
                    match result {
                        Ok(Ok(host)) => {
                            // Publish the physical driver before publishing
                            // flight completion. Cleanup observes these in the
                            // same order and can therefore never accept a
                            // successful flight with `driver == None`.
                            if let Err(error) = slot.driver.set(host) {
                                let unexpected_host = match error {
                                    SetError::AlreadyInitializedError(host)
                                    | SetError::InitializingError(host) => host,
                                };
                                // This branch is an invariant defense only:
                                // the initialization gate and single stored
                                // flight make a duplicate publication
                                // unreachable. Still shut the exact duplicate
                                // down before reporting the invariant failure.
                                let _ = unexpected_host.shutdown().await;
                                task_flight.complete(Err(
                                    host_launch_publication_invariant_error(slot.epoch),
                                ));
                            } else {
                                task_flight.complete(Ok(()));
                            }
                        }
                        Ok(Err(error)) => task_flight.complete(Err(error)),
                        Err(_) => task_flight
                            .complete(Err(host_launch_task_panicked_error(slot.epoch))),
                    }
                });
                flight
            }
        };

        // Gate contention must not consume the factory's own cold-start
        // budget. This wait is independently bounded, but timing out retires
        // the slot without aborting its Hub-owned flight.
        match tokio::time::timeout(HOST_INITIALIZATION_LAUNCH_TIMEOUT, flight.wait()).await {
            Ok(Ok(())) => {
                self.clear_launch_flight(&flight);
            }
            Ok(Err(error)) => {
                self.clear_launch_flight(&flight);
                if self
                    .current_launch_cleanup_ticket()
                    .is_some_and(|ticket| !ticket.is_complete())
                {
                    self.retire();
                    return Err(host_launch_cleanup_pending_error(
                        self.epoch,
                        Some(error),
                    ));
                }
                self.clear_completed_launch_cleanup_ticket();
                return Err(error);
            }
            Err(_) => {
                // Keep the flight stored. Exact cleanup must observe its
                // terminal result before it may release this HostKey/epoch
                // fence, so a forever-pending factory cannot amplify into an
                // unbounded series of new Chromium launches.
                self.retire();
                return Err(host_initialization_timeout_error(
                    self.epoch,
                    "launch",
                    HOST_INITIALIZATION_LAUNCH_TIMEOUT,
                ));
            }
        }
        let host = self
            .driver
            .get()
            .ok_or_else(|| host_launch_publication_invariant_error(self.epoch))?;
        // Retirement is published before cleanup waits for the initialization
        // gate. If shutdown/sweep selected this slot while launch was in
        // flight, never hand the late Host back to a Lane; the cleanup waiter
        // will shut it down as soon as this guard is released.
        if self.retired.load(Ordering::Acquire) {
            return Err(host_slot_retired_error());
        }
        Ok(host)
    }

    fn clear_launch_flight(&self, completed: &Arc<HostLaunchFlight>) {
        let mut launch_flight = self
            .launch_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if launch_flight
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, completed))
        {
            *launch_flight = None;
        }
    }

    fn current_launch_flight(&self) -> Option<Arc<HostLaunchFlight>> {
        self.launch_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn current_launch_cleanup_ticket(&self) -> Option<HostLaunchCleanupTicket> {
        self.launch_cleanup_ticket
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn clear_completed_launch_cleanup_ticket(&self) {
        let mut ticket = self
            .launch_cleanup_ticket
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ticket
            .as_ref()
            .is_some_and(HostLaunchCleanupTicket::is_complete)
        {
            *ticket = None;
        }
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
        if let Some(flight) = self.current_launch_flight() {
            let launch_result = tokio::time::timeout_at(deadline, flight.wait())
                .await
                .map_err(|_| {
                    host_cleanup_timeout_error(
                        self.epoch,
                        "launch_completion",
                        HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                    )
                })?;
            self.clear_launch_flight(&flight);
            if launch_result.is_ok() && self.driver.get().is_none() {
                return Err(host_launch_publication_invariant_error(self.epoch));
            }
            // A terminal factory error means the driver handoff never
            // occurred, but does not by itself prove process absence. The
            // counted cleanup ticket below remains pending while a factory or
            // engine relay still owns pre-handoff process/profile authority.
            // A pending/cancelled flight never even reaches this branch.
        }
        let host = self.driver.get();
        if let Some(host) = host {
            tokio::time::timeout_at(deadline, host.shutdown())
                .await
                .map_err(|_| {
                    host_cleanup_timeout_error(
                        self.epoch,
                        "driver_shutdown",
                        HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                    )
                })??;
        }
        if let Some(ticket) = self.current_launch_cleanup_ticket() {
            tokio::time::timeout_at(deadline, ticket.wait())
                .await
                .map_err(|_| {
                    host_cleanup_timeout_error(
                        self.epoch,
                        "launch_cleanup_proof",
                        HOST_SHUTDOWN_ATTEMPT_TIMEOUT,
                    )
                })?;
            self.clear_completed_launch_cleanup_ticket();
        }
        self.shutdown_complete.store(true, Ordering::Release);
        Ok(host.is_some())
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
    /// Visibility escalations already spent on this Lane.
    ///
    /// Each one replaces the Chromium Host process, so the count is bounded by
    /// [`MAX_LANE_VISIBILITY_ESCALATIONS`]. It lives on the Lane rather than the
    /// task so a single runaway page cannot spend a sibling Lane's allowance.
    visibility_escalations: AtomicU32,
    /// Set just before the platform closes this Lane to stay inside the task's
    /// memory budget, so an in-flight operation reports the honest, retryable
    /// [`BrowserErrorCode::TaskMemoryReclaimed`] instead of claiming the user
    /// closed the browser.
    memory_reclaimed: AtomicBool,
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
            visibility_escalations: AtomicU32::new(0),
            memory_reclaimed: AtomicBool::new(false),
            workspace_hint,
        }
    }

    async fn current_snapshot(&self) -> BrowserLaneSnapshot {
        let mut snapshot = self.snapshot.read().await.clone();
        snapshot.active_operation_count =
            self.active_operation_count.load(Ordering::Acquire);
        snapshot
    }

    /// The close reason for a Lane that is shutting down, so waiters report why.
    fn closed_error(&self, lane_id: BrowserLaneId) -> BrowserPlatformError {
        if self.memory_reclaimed.load(Ordering::Acquire) {
            BrowserPlatformError::task_memory_reclaimed(lane_id)
        } else {
            lane_closed_error(lane_id)
        }
    }
}

struct PendingLaneCleanup {
    cleanup_id: u64,
    lane_id: BrowserLaneId,
    user_id: String,
    task_id: String,
    family_id: String,
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
    task_id: String,
    family_id: String,
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

#[derive(Clone, Copy, Debug, Default)]
struct TaskMemoryAttribution {
    shared_rss_estimate_bytes: u64,
    /// True only when every measured Host contributing to this task has no
    /// other task attached. In that case Host RSS is an exact task-local
    /// upper bound (including Chromium base overhead), so sustained overage
    /// may safely close the noisy task without guessing among siblings.
    exclusive_hosts_only: bool,
}

#[derive(Clone)]
struct ResourceEmergencyHostCandidate {
    key: HostKey,
    browser_epoch: u64,
    headful: bool,
    rss_bytes: u64,
    attribution_rank: u8,
}

#[derive(Clone)]
struct ResourceEmergencyCpuHostCandidate {
    key: HostKey,
    browser_epoch: u64,
    headful: bool,
    cpu_pressure: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct TaskHostActivity {
    lane_count: u64,
    tab_count: u64,
}

impl TaskHostActivity {
    fn variable_weight(self) -> u64 {
        // A Lane carries renderer/session overhead even when its visible tab
        // list is empty. Tabs add signal but cannot erase that Lane floor.
        self.lane_count
            .saturating_mul(4)
            .saturating_add(self.tab_count)
            .max(1)
    }
}

#[derive(Clone)]
struct PolicyLaneCandidate {
    lane_id: BrowserLaneId,
    task_id: String,
    priority: LanePriority,
    lifecycle_state: LaneLifecycleState,
    created_at_ms: u64,
}

struct PolicyHostTabTarget {
    task_id: String,
    host_key: HostKey,
    lane_count: usize,
    reserved_count: usize,
    driver: Arc<dyn BrowserHostDriver>,
}

impl PolicyLaneCandidate {
    fn survivor_key(&self) -> (bool, u8, u64, BrowserLaneId) {
        let lifecycle_rank = match self.lifecycle_state {
            LaneLifecycleState::Running => 0,
            LaneLifecycleState::Frozen => 1,
            LaneLifecycleState::Starting => 2,
            LaneLifecycleState::Queued => 3,
            LaneLifecycleState::Stopping | LaneLifecycleState::Failed => 4,
        };
        (
            self.priority == LanePriority::Expansion,
            lifecycle_rank,
            self.created_at_ms,
            self.lane_id.clone(),
        )
    }
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

type CloseAllFlightResult = Result<CloseResult, BrowserPlatformError>;

struct CloseAllFlight {
    result: OnceLock<CloseAllFlightResult>,
    completed: Notify,
}

type PolicyUpdateFlightResult = Result<(), BrowserPlatformError>;

struct PolicyUpdateFlight {
    result: OnceLock<PolicyUpdateFlightResult>,
    completed: Notify,
}

type ShutdownFlightResult = Result<(), BrowserPlatformError>;

struct ShutdownFlight {
    result: OnceLock<ShutdownFlightResult>,
    completed: Notify,
}

/// Cancellation-safe ownership of one replacement target while it has not
/// yet been published either as the live Lane driver or as pending cleanup.
/// An armed Drop conservatively fences the entire exact replacement Host;
/// the independent cleanup supervisor then uses Host shutdown as proof.
struct PreparedRebindAuthorityGuard {
    inner: Weak<BrowserSessionHubInner>,
    lane_id: BrowserLaneId,
    host_authority: HostCleanupAuthorityKey,
    armed: bool,
}

impl PreparedRebindAuthorityGuard {
    fn complete(mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner
                .prepared_rebind_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&(self.lane_id.clone(), self.host_authority.clone()));
        }
        self.armed = false;
    }
}

impl Drop for PreparedRebindAuthorityGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        inner
            .host_stop_required_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(self.host_authority.clone());
        BrowserSessionHub { inner }.ensure_cleanup_retry_worker();
    }
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

impl CloseAllFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: CloseAllFlightResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> CloseAllFlightResult {
        loop {
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }
}

impl PolicyUpdateFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: PolicyUpdateFlightResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> PolicyUpdateFlightResult {
        loop {
            let notified = self.completed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            notified.await;
        }
    }
}

impl ShutdownFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            completed: Notify::new(),
        }
    }

    fn complete(&self, result: ShutdownFlightResult) {
        let _ = self.result.set(result);
        self.completed.notify_waiters();
    }

    async fn wait(&self) -> ShutdownFlightResult {
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

#[derive(Default)]
struct OperationAdmissionState {
    total: usize,
    by_lane: HashMap<BrowserLaneId, usize>,
    by_task: HashMap<String, usize>,
}

struct OperationAdmissionPermit {
    inner: Arc<BrowserSessionHubInner>,
    lane_id: BrowserLaneId,
    task_id: String,
}

#[derive(Default)]
struct TaskTabReservationState {
    by_task: HashMap<String, HashMap<(String, String), Weak<HubTaskTabReservation>>>,
}

struct HubTaskTabAuthorityShared {
    max_task_tabs: AtomicUsize,
    reservations: StdMutex<TaskTabReservationState>,
    changed: Notify,
}

#[derive(Clone)]
struct HubTaskTabAuthority {
    shared: Arc<HubTaskTabAuthorityShared>,
}

impl std::fmt::Debug for HubTaskTabAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubTaskTabAuthority")
            .field(
                "max_task_tabs",
                &self.shared.max_task_tabs.load(Ordering::Acquire),
            )
            .finish_non_exhaustive()
    }
}

impl HubTaskTabAuthority {
    fn new(max_task_tabs: usize) -> Self {
        Self {
            shared: Arc::new(HubTaskTabAuthorityShared {
                max_task_tabs: AtomicUsize::new(max_task_tabs.max(1)),
                reservations: StdMutex::new(TaskTabReservationState::default()),
                changed: Notify::new(),
            }),
        }
    }

    fn set_limit(&self, max_task_tabs: usize) {
        // Share the exact transaction lock with reserve. A lowering must not
        // return while a reservation which read the old ceiling is still able
        // to publish itself afterwards.
        let _state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.shared
            .max_task_tabs
            .store(max_task_tabs.max(1), Ordering::Release);
        self.shared.changed.notify_waiters();
    }

    fn limit(&self) -> usize {
        self.shared.max_task_tabs.load(Ordering::Acquire).max(1)
    }

    #[cfg(test)]
    fn count_for(&self, task_resource_key: &str) -> usize {
        let mut state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(reservations) = state.by_task.get_mut(task_resource_key) else {
            return 0;
        };
        reservations.retain(|_, reservation| reservation.strong_count() != 0);
        let count = reservations.len();
        if reservations.is_empty() {
            state.by_task.remove(task_resource_key);
        }
        count
    }

    fn task_counts(&self) -> HashMap<String, usize> {
        let mut state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.by_task.retain(|_, reservations| {
            reservations.retain(|_, reservation| reservation.strong_count() != 0);
            !reservations.is_empty()
        });
        state
            .by_task
            .iter()
            .map(|(task, reservations)| (task.clone(), reservations.len()))
            .collect()
    }

    fn lane_counts_for(&self, task_resource_key: &str) -> HashMap<String, usize> {
        let mut state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(reservations) = state.by_task.get_mut(task_resource_key) else {
            return HashMap::new();
        };
        reservations.retain(|_, reservation| reservation.strong_count() != 0);
        let mut counts = HashMap::new();
        for reservation in reservations.values().filter_map(Weak::upgrade) {
            *counts.entry(reservation.lane_id.clone()).or_default() += 1;
        }
        if reservations.is_empty() {
            state.by_task.remove(task_resource_key);
        }
        counts
    }
}

struct HubTaskTabReservation {
    shared: Arc<HubTaskTabAuthorityShared>,
    task_resource_key: String,
    lane_id: String,
    reservation_key: String,
}

impl BrowserTaskTabReservation for HubTaskTabReservation {}

impl Drop for HubTaskTabReservation {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(reservations) = state.by_task.get_mut(&self.task_resource_key) {
            let reservation_key = (
                self.lane_id.clone(),
                self.reservation_key.clone(),
            );
            let is_this_reservation = reservations
                .get(&reservation_key)
                .is_some_and(|reservation| {
                    std::ptr::eq(reservation.as_ptr(), self as *const Self)
                });
            if is_this_reservation {
                reservations.remove(&reservation_key);
            }
            if reservations.is_empty() {
                state.by_task.remove(&self.task_resource_key);
            }
        }
        drop(state);
        self.shared.changed.notify_waiters();
    }
}

#[async_trait::async_trait]
impl BrowserTaskTabAuthority for HubTaskTabAuthority {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        reservation_key: &str,
    ) -> Result<Arc<dyn BrowserTaskTabReservation>, BrowserPlatformError> {
        if task_resource_key.trim().is_empty()
            || lane_id.trim().is_empty()
            || reservation_key.trim().is_empty()
        {
            return Err(invalid_tab_reservation_error());
        }
        let mut state = self
            .shared
            .reservations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let reservations = state
            .by_task
            .entry(task_resource_key.to_owned())
            .or_default();
        reservations.retain(|_, reservation| reservation.strong_count() != 0);
        let scoped_key = (lane_id.to_owned(), reservation_key.to_owned());
        if let Some(existing) = reservations
            .get(&scoped_key)
            .and_then(Weak::upgrade)
        {
            return Ok(existing);
        }
        let limit = self.shared.max_task_tabs.load(Ordering::Acquire).max(1);
        if reservations.len() >= limit {
            return Err(task_tab_capacity_error(task_resource_key, limit));
        }
        let reservation = Arc::new(HubTaskTabReservation {
            shared: Arc::clone(&self.shared),
            task_resource_key: task_resource_key.to_owned(),
            lane_id: lane_id.to_owned(),
            reservation_key: reservation_key.to_owned(),
        });
        reservations.insert(scoped_key, Arc::downgrade(&reservation));
        Ok(reservation)
    }
}

#[derive(Default)]
struct TaskDownloadAuthorityState {
    families: HashMap<String, TaskDownloadFamilyUsage>,
    owner_families: HashMap<OwnerLeaseId, String>,
}

#[derive(Default)]
struct TaskDownloadFamilyUsage {
    owners: HashSet<OwnerLeaseId>,
    completed_bytes: u64,
    completed_files: usize,
    active: HashMap<(String, String), ActiveTaskDownload>,
}

struct ActiveTaskDownload {
    reservation: Weak<HubTaskDownloadReservation>,
    accounted_bytes: u64,
    completion_prepared: bool,
}

struct HubTaskDownloadAuthorityShared {
    state: StdMutex<TaskDownloadAuthorityState>,
}

#[derive(Clone)]
struct HubTaskDownloadAuthority {
    shared: Arc<HubTaskDownloadAuthorityShared>,
}

impl std::fmt::Debug for HubTaskDownloadAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HubTaskDownloadAuthority")
            .field("max_active", &MAX_TASK_ACTIVE_DOWNLOADS)
            .field("max_single_bytes", &MAX_TASK_SINGLE_DOWNLOAD_BYTES)
            .field(
                "max_completed_bytes",
                &MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            )
            .field("max_completed_files", &MAX_TASK_COMPLETED_DOWNLOAD_FILES)
            .finish()
    }
}

impl HubTaskDownloadAuthority {
    fn new() -> Self {
        Self {
            shared: Arc::new(HubTaskDownloadAuthorityShared {
                state: StdMutex::new(TaskDownloadAuthorityState::default()),
            }),
        }
    }

    /// Register a bound owner under the same Hub lifecycle gate used by
    /// revoke. This is the task-lifetime anchor which prevents a Host or
    /// runtime generation from resetting completed output usage.
    fn register_owner(
        &self,
        task_resource_key: &str,
        owner_lease_id: &OwnerLeaseId,
    ) -> Result<(), BrowserPlatformError> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = state.owner_families.get(owner_lease_id) {
            return if existing == task_resource_key {
                Ok(())
            } else {
                Err(task_download_owner_binding_error())
            };
        }
        state
            .owner_families
            .insert(owner_lease_id.clone(), task_resource_key.to_owned());
        state
            .families
            .entry(task_resource_key.to_owned())
            .or_default()
            .owners
            .insert(owner_lease_id.clone());
        Ok(())
    }

    /// Retire one exact owner only after its Lane/Host cleanup proof.
    ///
    /// A family which has ever completed output is sticky for the Hub process
    /// lifetime. Owner TTL/runtime rotation is not trusted task-finalization
    /// authority and therefore cannot reset consumed quota. Empty families
    /// which never completed output remain reclaimable.
    fn retire_owner(&self, owner_lease_id: &OwnerLeaseId) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(task_resource_key) = state.owner_families.remove(owner_lease_id) else {
            return;
        };
        let remove_family = if let Some(family) = state.families.get_mut(&task_resource_key) {
            family.owners.remove(owner_lease_id);
            family
                .active
                .retain(|_, active| active.reservation.strong_count() != 0);
            family.owners.is_empty()
                && family.active.is_empty()
                && family.completed_files == 0
                && family.completed_bytes == 0
        } else {
            false
        };
        if remove_family {
            state.families.remove(&task_resource_key);
        }
    }

    fn clear(&self) {
        let mut state = self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.families.clear();
        state.owner_families.clear();
    }

    #[cfg(test)]
    fn usage_for(&self, task_resource_key: &str) -> Option<(usize, u64, usize, usize)> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let family = state.families.get_mut(task_resource_key)?;
        family
            .active
            .retain(|_, active| active.reservation.strong_count() != 0);
        Some((
            family.owners.len(),
            family.completed_bytes,
            family.completed_files,
            family.active.len(),
        ))
    }
}

struct HubTaskDownloadReservation {
    shared: Arc<HubTaskDownloadAuthorityShared>,
    task_resource_key: String,
    lane_id: String,
    download_key: String,
    completed: AtomicBool,
}

impl BrowserTaskDownloadReservation for HubTaskDownloadReservation {
    fn update_progress(
        &self,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), BrowserPlatformError> {
        let proposed = received_bytes.max(total_bytes.unwrap_or(0));
        if proposed > MAX_TASK_SINGLE_DOWNLOAD_BYTES {
            return Err(task_download_capacity_error(
                "single_file_bytes",
                proposed,
                MAX_TASK_SINGLE_DOWNLOAD_BYTES,
            ));
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let family = state
            .families
            .get_mut(&self.task_resource_key)
            .ok_or_else(task_download_authority_retired_error)?;
        if family.owners.is_empty() {
            return Err(task_download_authority_retired_error());
        }
        let scoped_key = (self.lane_id.clone(), self.download_key.clone());
        let Some(current) = family.active.get(&scoped_key) else {
            return Err(task_download_authority_retired_error());
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self) {
            return Err(task_download_authority_retired_error());
        }
        if current.completion_prepared {
            return Err(task_download_authority_retired_error());
        }
        let proposed = proposed.max(current.accounted_bytes);
        let other_active = family
            .active
            .iter()
            .filter(|(key, _)| *key != &scoped_key)
            .fold(0u64, |total, (_, active)| {
                total.saturating_add(active.accounted_bytes)
            });
        let total = family
            .completed_bytes
            .saturating_add(other_active)
            .saturating_add(proposed);
        if total > MAX_TASK_COMPLETED_DOWNLOAD_BYTES {
            return Err(task_download_capacity_error(
                "task_cumulative_bytes",
                total,
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }
        family
            .active
            .get_mut(&scoped_key)
            .expect("validated active download remains under the authority lock")
            .accounted_bytes = proposed;
        Ok(())
    }

    fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserPlatformError> {
        if self.completed.load(Ordering::Acquire) {
            return Ok(());
        }
        if actual_bytes > MAX_TASK_SINGLE_DOWNLOAD_BYTES {
            return Err(task_download_capacity_error(
                "single_file_bytes",
                actual_bytes,
                MAX_TASK_SINGLE_DOWNLOAD_BYTES,
            ));
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let completed_family_count = state
            .families
            .values()
            .filter(|usage| usage.completed_files != 0 || usage.completed_bytes != 0)
            .count();
        let family = state
            .families
            .get_mut(&self.task_resource_key)
            .ok_or_else(task_download_authority_retired_error)?;
        if family.owners.is_empty() {
            return Err(task_download_authority_retired_error());
        }
        let scoped_key = (self.lane_id.clone(), self.download_key.clone());
        let Some(current) = family.active.get(&scoped_key) else {
            return Err(task_download_authority_retired_error());
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self) {
            return Err(task_download_authority_retired_error());
        }
        if current.completion_prepared {
            return if current.accounted_bytes == actual_bytes {
                Ok(())
            } else {
                Err(task_download_authority_retired_error())
            };
        }
        if family.completed_files >= MAX_TASK_COMPLETED_DOWNLOAD_FILES {
            return Err(task_download_file_capacity_error(family.completed_files + 1));
        }
        if family.completed_files == 0
            && completed_family_count >= MAX_RETAINED_COMPLETED_DOWNLOAD_FAMILIES
        {
            return Err(task_download_family_capacity_error());
        }
        let other_active = family
            .active
            .iter()
            .filter(|(key, _)| *key != &scoped_key)
            .fold(0u64, |total, (_, active)| {
                total.saturating_add(active.accounted_bytes)
            });
        let total = family
            .completed_bytes
            .saturating_add(other_active)
            .saturating_add(actual_bytes);
        if total > MAX_TASK_COMPLETED_DOWNLOAD_BYTES {
            return Err(task_download_capacity_error(
                "task_cumulative_bytes",
                total,
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }

        let current = family
            .active
            .get_mut(&scoped_key)
            .expect("validated completion remains active under the authority lock");
        current.accounted_bytes = actual_bytes;
        current.completion_prepared = true;
        Ok(())
    }

    fn finalize_complete(&self) {
        if self.completed.load(Ordering::Acquire) {
            return;
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(family) = state.families.get_mut(&self.task_resource_key) else {
            return;
        };
        let scoped_key = (self.lane_id.clone(), self.download_key.clone());
        let Some(current) = family.active.get(&scoped_key) else {
            return;
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self)
            || !current.completion_prepared
        {
            return;
        }
        let actual_bytes = current.accounted_bytes;
        family.completed_bytes = family.completed_bytes.saturating_add(actual_bytes);
        family.completed_files = family.completed_files.saturating_add(1);
        family.active.remove(&scoped_key);
        self.completed.store(true, Ordering::Release);
    }
}

impl Drop for HubTaskDownloadReservation {
    fn drop(&mut self) {
        if self.completed.load(Ordering::Acquire) {
            return;
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let scoped_key = (self.lane_id.clone(), self.download_key.clone());
        let remove_family = if let Some(family) = state.families.get_mut(&self.task_resource_key) {
            let is_current = family
                .active
                .get(&scoped_key)
                .is_some_and(|active| {
                    std::ptr::eq(active.reservation.as_ptr(), self as *const Self)
                });
            if is_current {
                family.active.remove(&scoped_key);
            }
            family.owners.is_empty()
                && family.active.is_empty()
                && family.completed_files == 0
                && family.completed_bytes == 0
        } else {
            false
        };
        if remove_family {
            state.families.remove(&self.task_resource_key);
        }
    }
}

#[async_trait::async_trait]
impl BrowserTaskDownloadAuthority for HubTaskDownloadAuthority {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        download_key: &str,
    ) -> Result<Arc<dyn BrowserTaskDownloadReservation>, BrowserPlatformError> {
        const MAX_DOWNLOAD_AUTHORITY_KEY_BYTES: usize = 4 * 1024;
        if task_resource_key.trim().is_empty()
            || lane_id.trim().is_empty()
            || download_key.trim().is_empty()
            || task_resource_key.len() > MAX_DOWNLOAD_AUTHORITY_KEY_BYTES
            || lane_id.len() > MAX_DOWNLOAD_AUTHORITY_KEY_BYTES
            || download_key.len() > MAX_DOWNLOAD_AUTHORITY_KEY_BYTES
        {
            return Err(task_download_invalid_reservation_error());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let family = state
            .families
            .get_mut(task_resource_key)
            .filter(|family| !family.owners.is_empty())
            .ok_or_else(task_download_authority_retired_error)?;
        family
            .active
            .retain(|_, active| active.reservation.strong_count() != 0);
        let scoped_key = (lane_id.to_owned(), download_key.to_owned());
        if let Some(existing) = family
            .active
            .get(&scoped_key)
            .and_then(|active| active.reservation.upgrade())
        {
            return Ok(existing);
        }
        if family.active.len() >= MAX_TASK_ACTIVE_DOWNLOADS {
            return Err(task_download_active_capacity_error(family.active.len() + 1));
        }
        if family.completed_files >= MAX_TASK_COMPLETED_DOWNLOAD_FILES {
            return Err(task_download_file_capacity_error(family.completed_files + 1));
        }
        if family.completed_bytes >= MAX_TASK_COMPLETED_DOWNLOAD_BYTES {
            return Err(task_download_capacity_error(
                "task_cumulative_bytes",
                family.completed_bytes,
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }
        let reservation = Arc::new(HubTaskDownloadReservation {
            shared: Arc::clone(&self.shared),
            task_resource_key: task_resource_key.to_owned(),
            lane_id: lane_id.to_owned(),
            download_key: download_key.to_owned(),
            completed: AtomicBool::new(false),
        });
        family.active.insert(
            scoped_key,
            ActiveTaskDownload {
                reservation: Arc::downgrade(&reservation),
                accounted_bytes: 0,
                completion_prepared: false,
            },
        );
        Ok(reservation)
    }
}

impl Drop for OperationAdmissionPermit {
    fn drop(&mut self) {
        let mut state = self
            .inner
            .operation_admissions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.total = state.total.saturating_sub(1);
        decrement_count(&mut state.by_lane, &self.lane_id);
        decrement_count(&mut state.by_task, &self.task_id);
        drop(state);
        self.inner.operation_capacity_changed.notify_waiters();
    }
}

fn decrement_count<K: Eq + std::hash::Hash>(counts: &mut HashMap<K, usize>, key: &K) {
    if let Some(count) = counts.get_mut(key) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            counts.remove(key);
        }
    }
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
    operation_admissions: StdMutex<OperationAdmissionState>,
    operation_admission_global_limit: AtomicUsize,
    operation_admission_task_limit: AtomicUsize,
    operation_admission_lane_limit: AtomicUsize,
    operation_weight_limit: AtomicU64,
    task_operation_weight_limit: AtomicU64,
    active_operation_weight: AtomicU64,
    task_active_operation_weight: StdMutex<HashMap<String, u64>>,
    active_regular_operations: AtomicU64,
    operation_capacity_changed: Notify,
    active_heavy_operations: AtomicU64,
    owner_leases: OwnerLeaseService,
    /// Serializes the synchronous owner bind/revoke boundary with download
    /// family membership publication. It is never held across an await.
    task_lifecycle_gate: StdMutex<()>,
    task_download_authority: Arc<HubTaskDownloadAuthority>,
    identity_generations: IdentityGenerationCoordinator,
    identity_refresh_gate: Mutex<()>,
    lanes: RwLock<HashMap<BrowserLaneId, Arc<LaneRecord>>>,
    lane_keys: RwLock<HashMap<LaneKey, BrowserLaneId>>,
    /// Latest instantaneous Host-RSS share for each live Lane. This is a
    /// conservative attribution signal for task-local watchdog decisions; it
    /// is not presented as exact per-target RSS.
    lane_memory_samples: RwLock<HashMap<BrowserLaneId, u64>>,
    task_memory_samples: RwLock<HashMap<String, TaskMemoryAttribution>>,
    task_over_budget_samples: StdMutex<HashMap<String, u8>>,
    task_tab_authority: Arc<HubTaskTabAuthority>,
    // Retirement lock order is contractual for every path that touches more
    // than one of these structures:
    //   open_gate -> retiring_host_keys -> host_slots
    //     -> retiring_host_slots -> orphaned_host_slots
    // Never acquire an earlier authority while holding a later one.
    host_slots: RwLock<HashMap<HostKey, Arc<HostSlot>>>,
    /// Durable in-process admission fence and retry record for an exact
    /// Anonymous Host generation whose profile crossed a hygiene boundary.
    anonymous_profile_retirements: StdMutex<HashMap<HostKey, u64>>,
    anonymous_profile_rotation_workers: StdMutex<HashSet<HostKey>>,
    #[cfg(test)]
    anonymous_profile_rotation_panics_remaining: AtomicUsize,
    /// Permanent, process-lifetime Primary admission fence. This is separate
    /// from exact cleanup debt: after the Host is proven stopped the fence
    /// remains, while the independent cleanup supervisor may go idle.
    primary_profile_fence: OnceLock<PrimaryProfileFence>,
    primary_profile_cleanup_epochs: StdMutex<HashSet<u64>>,
    primary_profile_cleanup_workers: StdMutex<HashSet<u64>>,
    #[cfg(test)]
    primary_profile_cleanup_panics_remaining: AtomicUsize,
    host_empty_since_ms: RwLock<HashMap<HostKey, u64>>,
    retiring_host_slots: Mutex<Vec<(HostKey, Arc<HostSlot>)>>,
    retiring_host_keys: RwLock<HashSet<HostKey>>,
    retiring_hosts_changed: Notify,
    orphaned_host_slots: Mutex<Vec<(HostKey, Arc<HostSlot>)>>,
    pending_lane_cleanups: Mutex<Vec<Arc<PendingLaneCleanup>>>,
    pending_host_retirements: Mutex<Vec<PendingHostRetirement>>,
    owner_cleanup_targets: Mutex<HashMap<OwnerLeaseId, HashSet<OwnerCleanupTarget>>>,
    host_finalizations: Mutex<HashMap<HostKey, Arc<HostFinalizationFlight>>>,
    cleanup_budget: HubCleanupBudget,
    lane_cleanup_budget_tokens: StdMutex<HashMap<BrowserLaneId, LaneCleanupBudgetToken>>,
    host_cleanup_budget_tokens:
        StdMutex<HashMap<HostCleanupAuthorityKey, HostCleanupBudgetToken>>,
    /// Synchronous publication authority for a replacement Host between its
    /// insertion into `host_slots` and completion of every Lane/route rebind.
    /// An aborted restart marks the exact entry abandoned in Drop; the
    /// independent cleanup worker then transfers that slot to the durable
    /// orphan queue without ever selecting a different epoch by key alone.
    published_restart_slots:
        StdMutex<HashMap<HostCleanupAuthorityKey, Arc<PublishedRestartAuthority>>>,
    prepared_rebind_authority_gate: Mutex<()>,
    prepared_rebind_authorities:
        StdMutex<HashSet<(BrowserLaneId, HostCleanupAuthorityKey)>>,
    host_stop_required_authorities: StdMutex<HashSet<HostCleanupAuthorityKey>>,
    /// O(1) nudge for the independent supervisor to reconcile only retained,
    /// exact cleanup authority after cleanup-budget backpressure. It never
    /// carries a dynamic live-Lane predicate and therefore cannot broaden a
    /// delayed retry into task, Host, or installation cleanup.
    cleanup_ledger_reconcile_requested: AtomicBool,
    /// O(1) wakeup used only when a panic rolls back an admission before the
    /// Lane is published. The scheduler entry itself is removed synchronously;
    /// this flag asks the fixed supervisor to reconsider already-queued work.
    scheduler_reconcile_requested: AtomicBool,
    #[cfg(test)]
    lane_admission_publication_panics_remaining: AtomicUsize,
    #[cfg(test)]
    promotion_publication_blocked: AtomicBool,
    #[cfg(test)]
    promotion_publication_panics_remaining: AtomicUsize,
    #[cfg(test)]
    promotion_publication_attempts: AtomicUsize,
    #[cfg(test)]
    promotion_publication_changed: Notify,
    #[cfg(test)]
    promotion_publication_release: tokio::sync::Semaphore,
    exact_lane_cleanup_admission_gate: StdMutex<()>,
    exact_lane_cleanup_handoffs: StdMutex<ExactLaneCleanupLedger>,
    abandoned_lane_starts: StdMutex<HashSet<BrowserLaneId>>,
    lane_cleanup_retry_gate: Mutex<()>,
    host_cleanup_retry_gate: Mutex<()>,
    cleanup_retry_worker_running: AtomicBool,
    cleanup_sequence: AtomicU64,
    host_epoch_sequence: AtomicU64,
    // Last pressure state broadcast by the telemetry sampler, encoded for the
    // idle-sample suppression in `update_resource_telemetry`.
    // 0 = never sampled, 1 = Normal, 2 = Pressured, 3 = Critical.
    last_sampled_pressure_state: AtomicU64,
    // Temporal hysteresis for the last-resort physical convergence path.  The
    // counter advances only on consecutive real managed-Host RSS breaches for
    // which task-local reclaim made no progress.
    critical_browser_rss_streak: AtomicU64,
    // CPU cannot be attributed precisely inside a shared Chromium Host. This
    // streak therefore guards an installation-level, exact-managed-Host
    // convergence path rather than pretending to be a per-task CPU quota.
    critical_browser_cpu_streak: AtomicU64,
    resource_emergency_gate: Mutex<()>,
    host_restarts: PerKeyHostRestartSingleFlight<HostKey>,
    host_circuits: Mutex<HashMap<HostKey, Arc<HostCircuitBreaker>>>,
    // Primary process visibility is a Host-wide property. Serialize explicit
    // display transitions with Primary Host selection/start so opposite
    // headful/headless requests cannot join the same restart flight or launch
    // a process from a stale default in the middle of a transition.
    primary_visibility_gate: Mutex<()>,
    drain_gate: Mutex<()>,
    close_all_flight: StdMutex<Option<Arc<CloseAllFlight>>>,
    policy_update_flight: StdMutex<Option<(String, Arc<PolicyUpdateFlight>)>>,
    draining: AtomicBool,
    policy_reconciling: AtomicBool,
    policy_reconciled: Notify,
    open_gate: Mutex<()>,
    shutdown_gate: Mutex<()>,
    shutdown_flight: StdMutex<Option<Arc<ShutdownFlight>>>,
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
    task_id: String,
    resource_class: DriverResourceClass,
    acquired_weight: u64,
}

struct HubDrainGuard {
    inner: Arc<BrowserSessionHubInner>,
}

struct PolicyReconciliationGuard {
    inner: Arc<BrowserSessionHubInner>,
}

impl Drop for HubDrainGuard {
    fn drop(&mut self) {
        self.inner.draining.store(false, Ordering::Release);
    }
}


impl Drop for PolicyReconciliationGuard {
    fn drop(&mut self) {
        self.inner
            .policy_reconciling
            .store(false, Ordering::Release);
        self.inner.policy_reconciled.notify_waiters();
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
        let mut task_weights = self
            .inner
            .task_active_operation_weight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = task_weights.get_mut(&self.task_id) {
            debug_assert!(
                *current >= self.acquired_weight,
                "task operation weight underflow"
            );
            *current = current.saturating_sub(self.acquired_weight);
            if *current == 0 {
                task_weights.remove(&self.task_id);
            }
        }
        drop(task_weights);
        self.inner.operation_capacity_changed.notify_waiters();
    }
}

#[derive(Clone)]
pub struct BrowserSessionHub {
    inner: Arc<BrowserSessionHubInner>,
}

#[derive(Clone, Copy, Debug)]
struct PrimaryProfileFence {
    trigger_epoch: u64,
    reason: &'static str,
}

/// Exact membership authority for one Anonymous rotation worker. The guard is
/// created before the task is spawned, so panic, abort, or a never-polled task
/// all remove the worker-set key. If the sticky retirement fence still exists,
/// Drop transfers the still-sticky epoch to the independent cleanup supervisor,
/// which safely rearms a successor worker outside the dropping task's stack.
struct AnonymousProfileRotationWorkerGuard {
    inner: Arc<BrowserSessionHubInner>,
    key: HostKey,
}

/// Exact membership authority for one Primary cleanup epoch. The permanent
/// fence is never removed; this guard only rearms process cleanup while the
/// exact epoch still has retained physical authority.
struct PrimaryProfileCleanupWorkerGuard {
    inner: Arc<BrowserSessionHubInner>,
    epoch: u64,
}

struct PublishedRestartAuthority {
    slot: Arc<HostSlot>,
    abandoned: AtomicBool,
}

struct PublishedRestartGuard {
    inner: Arc<BrowserSessionHubInner>,
    authority_key: HostCleanupAuthorityKey,
    authority: Arc<PublishedRestartAuthority>,
    armed: bool,
}

impl PublishedRestartGuard {
    fn disarm(mut self) {
        let mut published = self
            .inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if published
            .get(&self.authority_key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.authority))
        {
            published.remove(&self.authority_key);
        }
        self.armed = false;
    }
}

impl Drop for PublishedRestartGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let still_provisional = self
            .inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&self.authority_key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.authority));
        // Another path may already have atomically transferred this exact
        // slot into the durable orphan queue. In that case the provisional
        // guard has nothing left to publish.
        if !still_provisional {
            return;
        }
        self.authority.slot.retire();
        self.authority.abandoned.store(true, Ordering::Release);
        BrowserSessionHub {
            inner: Arc::clone(&self.inner),
        }
        .ensure_cleanup_retry_worker();
    }
}

impl Drop for AnonymousProfileRotationWorkerGuard {
    fn drop(&mut self) {
        self.inner
            .anonymous_profile_rotation_workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return;
        }
        let pending = self
            .inner
            .anonymous_profile_retirements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&self.key);
        if pending {
            // Re-arm on the independent cleanup runtime instead of spawning a
            // successor recursively from Drop. During Tokio runtime teardown,
            // a newly spawned never-polled task is immediately dropped; doing
            // another spawn from its guard would recurse until stack overflow.
            BrowserSessionHub {
                inner: Arc::clone(&self.inner),
            }
            .ensure_cleanup_retry_worker();
        }
    }
}

impl Drop for PrimaryProfileCleanupWorkerGuard {
    fn drop(&mut self) {
        self.inner
            .primary_profile_cleanup_workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.epoch);
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return;
        }
        let pending = self
            .inner
            .primary_profile_cleanup_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&self.epoch);
        if pending {
            BrowserSessionHub {
                inner: Arc::clone(&self.inner),
            }
            .ensure_cleanup_retry_worker();
        }
    }
}

struct LaneStartWaiter {
    hub: BrowserSessionHub,
    lane_id: BrowserLaneId,
    lane: Arc<LaneRecord>,
    flight: Arc<LaneStartFlight>,
}

/// Panic-safe bridge between scheduler admission and Lane inventory
/// publication.
///
/// The guarded production section is intentionally synchronous: once
/// `scheduler.admit` succeeds there must be no cancellation point before both
/// Lane maps contain the exact id. Drop is therefore a panic backstop, not a
/// detached cleanup path.
struct UnpublishedLaneAdmissionGuard<'a> {
    hub: BrowserSessionHub,
    lane_id: BrowserLaneId,
    lane_key: LaneKey,
    lanes: &'a mut HashMap<BrowserLaneId, Arc<LaneRecord>>,
    lane_keys: &'a mut HashMap<LaneKey, BrowserLaneId>,
    published: bool,
}

impl<'a> UnpublishedLaneAdmissionGuard<'a> {
    fn new(
        hub: BrowserSessionHub,
        lane_id: BrowserLaneId,
        lane_key: LaneKey,
        lanes: &'a mut HashMap<BrowserLaneId, Arc<LaneRecord>>,
        lane_keys: &'a mut HashMap<LaneKey, BrowserLaneId>,
    ) -> Self {
        Self {
            hub,
            lane_id,
            lane_key,
            lanes,
            lane_keys,
            published: false,
        }
    }

    fn publish(&mut self, lane: Arc<LaneRecord>) {
        self.lanes.insert(self.lane_id.clone(), lane);
        #[cfg(test)]
        if self
            .hub
            .inner
            .lane_admission_publication_panics_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                (remaining != 0).then(|| remaining - 1)
            })
            .is_ok()
        {
            panic!("synthetic Lane admission publication panic");
        }
        self.lane_keys
            .insert(self.lane_key.clone(), self.lane_id.clone());
        self.published = true;
    }
}

impl Drop for UnpublishedLaneAdmissionGuard<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // Both maps are already exclusively borrowed by this guard, so a
        // panic after the first insertion can be rolled back synchronously.
        // Lane ids are never reused; remove the key only when it still names
        // this exact unpublished id.
        self.lanes.remove(&self.lane_id);
        if self.lane_keys.get(&self.lane_key) == Some(&self.lane_id) {
            self.lane_keys.remove(&self.lane_key);
        }
        if self
            .hub
            .inner
            .scheduler
            .discard_unpublished(&self.lane_id)
        {
            self.hub
                .inner
                .scheduler_reconcile_requested
                .store(true, Ordering::Release);
            self.hub.ensure_cleanup_retry_worker();
        }
    }
}

/// Exact rollback for a queued Lane selected by the scheduler but not yet
/// published with a Hub-owned start flight.
///
/// Owner validation and async inventory locks necessarily happen after the
/// fairness decision. If that caller-owned future is cancelled or panics, the
/// guard returns only this opaque Lane id to the queue and nudges the one fixed
/// supervisor to retry. It never selects work by name, runtime, task or Host.
struct UnpublishedLanePromotionGuard {
    hub: BrowserSessionHub,
    lane_id: Option<BrowserLaneId>,
}

impl UnpublishedLanePromotionGuard {
    fn new(hub: BrowserSessionHub, lane_id: BrowserLaneId) -> Self {
        Self {
            hub,
            lane_id: Some(lane_id),
        }
    }

    fn publish(&mut self) {
        self.lane_id = None;
    }
}

impl Drop for UnpublishedLanePromotionGuard {
    fn drop(&mut self) {
        let Some(lane_id) = self.lane_id.take() else {
            return;
        };
        if self
            .hub
            .inner
            .scheduler
            .defer_active_to_queue(&lane_id, "browser_promotion_interrupted")
            .is_some()
        {
            self.hub
                .inner
                .scheduler_reconcile_requested
                .store(true, Ordering::Release);
            self.hub.ensure_cleanup_retry_worker();
        }
    }
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
            self.hub
                .request_abandon_unclaimed_lane_start(self.lane_id.clone());
        }
    }
}

impl BrowserSessionHub {
    fn recover_host_failure_owned(
        self,
        key: HostKey,
        observed_epoch: u64,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<HostRestartTransition, BrowserPlatformError>>
                + Send
                + 'static,
        >,
    > {
        Box::pin(async move {
            self.recover_host_failure(key, observed_epoch).await
        })
    }

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
        config.anonymous_profile_policy.normalize();
        config.primary_profile_policy.normalize();
        let scheduler_config = SchedulerConfig {
            max_open_lanes: config.resource_policy.max_open_lanes,
            max_owner_active: config
                .resource_policy
                .max_task_open_lanes
                .min(config.resource_policy.max_open_lanes),
            max_global_queue: config.resource_policy.max_global_queue,
            max_owner_queue: config.resource_policy.max_owner_queue,
            ..SchedulerConfig::default()
        };
        let operation_admission_limits =
            operation_admission_limits(&config.resource_policy);
        let task_tab_authority = Arc::new(HubTaskTabAuthority::new(
            config.resource_policy.max_task_tabs,
        ));
        let task_download_authority = Arc::new(HubTaskDownloadAuthority::new());
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            inner: Arc::new(BrowserSessionHubInner {
                factory,
                scheduler: BrowserLaneScheduler::new(scheduler_config, Arc::clone(&clock)),
                operation_budget_gate: Mutex::new(()),
                operation_admissions: StdMutex::new(OperationAdmissionState::default()),
                operation_admission_global_limit: AtomicUsize::new(
                    operation_admission_limits.0,
                ),
                operation_admission_task_limit: AtomicUsize::new(
                    operation_admission_limits.1,
                ),
                operation_admission_lane_limit: AtomicUsize::new(
                    operation_admission_limits.2,
                ),
                operation_weight_limit: AtomicU64::new(
                    config.resource_policy.max_active_operations.max(1) as u64,
                ),
                task_operation_weight_limit: AtomicU64::new(
                    config
                        .resource_policy
                        .max_task_active_operations
                        .min(config.resource_policy.max_active_operations)
                        .max(1) as u64,
                ),
                active_operation_weight: AtomicU64::new(0),
                task_active_operation_weight: StdMutex::new(HashMap::new()),
                active_regular_operations: AtomicU64::new(0),
                operation_capacity_changed: Notify::new(),
                active_heavy_operations: AtomicU64::new(0),
                owner_leases: OwnerLeaseService::new(
                    Arc::clone(&clock),
                    config.owner_lease_ttl_ms,
                ),
                task_lifecycle_gate: StdMutex::new(()),
                task_download_authority,
                identity_generations: IdentityGenerationCoordinator::new(Arc::clone(&clock)),
                identity_refresh_gate: Mutex::new(()),
                clock,
                config: RwLock::new(config),
                telemetry: RwLock::new(ResourceTelemetry::default()),
                lanes: RwLock::new(HashMap::new()),
                lane_keys: RwLock::new(HashMap::new()),
                lane_memory_samples: RwLock::new(HashMap::new()),
                task_memory_samples: RwLock::new(HashMap::new()),
                task_over_budget_samples: StdMutex::new(HashMap::new()),
                task_tab_authority,
                host_slots: RwLock::new(HashMap::new()),
                anonymous_profile_retirements: StdMutex::new(HashMap::new()),
                anonymous_profile_rotation_workers: StdMutex::new(HashSet::new()),
                #[cfg(test)]
                anonymous_profile_rotation_panics_remaining: AtomicUsize::new(0),
                primary_profile_fence: OnceLock::new(),
                primary_profile_cleanup_epochs: StdMutex::new(HashSet::new()),
                primary_profile_cleanup_workers: StdMutex::new(HashSet::new()),
                #[cfg(test)]
                primary_profile_cleanup_panics_remaining: AtomicUsize::new(0),
                host_empty_since_ms: RwLock::new(HashMap::new()),
                retiring_host_slots: Mutex::new(Vec::new()),
                retiring_host_keys: RwLock::new(HashSet::new()),
                retiring_hosts_changed: Notify::new(),
                orphaned_host_slots: Mutex::new(Vec::new()),
                pending_lane_cleanups: Mutex::new(Vec::new()),
                pending_host_retirements: Mutex::new(Vec::new()),
                owner_cleanup_targets: Mutex::new(HashMap::new()),
                host_finalizations: Mutex::new(HashMap::new()),
                cleanup_budget: HubCleanupBudget::new(),
                lane_cleanup_budget_tokens: StdMutex::new(HashMap::new()),
                host_cleanup_budget_tokens: StdMutex::new(HashMap::new()),
                published_restart_slots: StdMutex::new(HashMap::new()),
                prepared_rebind_authority_gate: Mutex::new(()),
                prepared_rebind_authorities: StdMutex::new(HashSet::new()),
                host_stop_required_authorities: StdMutex::new(HashSet::new()),
                cleanup_ledger_reconcile_requested: AtomicBool::new(false),
                scheduler_reconcile_requested: AtomicBool::new(false),
                #[cfg(test)]
                lane_admission_publication_panics_remaining: AtomicUsize::new(0),
                #[cfg(test)]
                promotion_publication_blocked: AtomicBool::new(false),
                #[cfg(test)]
                promotion_publication_panics_remaining: AtomicUsize::new(0),
                #[cfg(test)]
                promotion_publication_attempts: AtomicUsize::new(0),
                #[cfg(test)]
                promotion_publication_changed: Notify::new(),
                #[cfg(test)]
                promotion_publication_release: tokio::sync::Semaphore::new(0),
                exact_lane_cleanup_admission_gate: StdMutex::new(()),
                exact_lane_cleanup_handoffs: StdMutex::new(ExactLaneCleanupLedger::default()),
                abandoned_lane_starts: StdMutex::new(HashSet::new()),
                lane_cleanup_retry_gate: Mutex::new(()),
                host_cleanup_retry_gate: Mutex::new(()),
                cleanup_retry_worker_running: AtomicBool::new(false),
                cleanup_sequence: AtomicU64::new(0),
                host_epoch_sequence: AtomicU64::new(0),
                last_sampled_pressure_state: AtomicU64::new(0),
                critical_browser_rss_streak: AtomicU64::new(0),
                critical_browser_cpu_streak: AtomicU64::new(0),
                resource_emergency_gate: Mutex::new(()),
                host_restarts: PerKeyHostRestartSingleFlight::default(),
                host_circuits: Mutex::new(HashMap::new()),
                primary_visibility_gate: Mutex::new(()),
                drain_gate: Mutex::new(()),
                close_all_flight: StdMutex::new(None),
                policy_update_flight: StdMutex::new(None),
                draining: AtomicBool::new(false),
                policy_reconciling: AtomicBool::new(false),
                policy_reconciled: Notify::new(),
                open_gate: Mutex::new(()),
                shutdown_gate: Mutex::new(()),
                shutdown_flight: StdMutex::new(None),
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
        self.managed_host_process_identities()
            .await
            .into_iter()
            .map(|identity| identity.process_id)
            .collect()
    }

    pub async fn managed_host_process_identities(&self) -> Vec<crate::BrowserProcessIdentity> {
        let slots = self.managed_host_slots().await;
        let mut identities = slots
            .iter()
            .filter_map(|slot| slot.get().and_then(|host| host.process_identity()))
            .filter(|identity| identity.process_id != 0)
            .collect::<Vec<_>>();
        identities.sort_unstable_by_key(|identity| {
            (
                identity.process_id,
                identity.started_at_epoch_seconds,
                identity.platform_start_key,
            )
        });
        identities.dedup();
        identities
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
        let inventory_lane_count = self.inner.lanes.read().await.len();
        // The two counts are equal at every published boundary. Taking the
        // larger value makes installation drains fail honestly if a future
        // regression ever strands scheduler authority outside inventory,
        // rather than reporting a false clean shutdown.
        let lane_count = inventory_lane_count.max(self.inner.scheduler.retained_lane_count());
        let pending_lane_cleanups = self.inner.pending_lane_cleanups.lock().await.len();
        let pending_host_retirements =
            self.inner.pending_host_retirements.lock().await.len();
        let retiring_host_slots = self.inner.retiring_host_slots.lock().await.len();
        let orphaned_host_slots = self.inner.orphaned_host_slots.lock().await.len();
        let host_finalizations = self.inner.host_finalizations.lock().await.len();
        let prepared_rebind_authorities = self
            .inner
            .prepared_rebind_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let exact_lane_cleanup_handoffs = self
            .inner
            .exact_lane_cleanup_handoffs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .len();
        RemainingResources {
            lane_count,
            cleanup_count: pending_lane_cleanups
                .saturating_add(pending_host_retirements)
                .saturating_add(retiring_host_slots)
                .saturating_add(orphaned_host_slots)
                .saturating_add(host_finalizations)
                .saturating_add(prepared_rebind_authorities)
                .saturating_add(exact_lane_cleanup_handoffs),
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
        let exact_lane_cleanup_handoffs = self
            .inner
            .exact_lane_cleanup_handoffs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .values()
            .filter(|authority| authority.user_id == user_id)
            .count();
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
            .saturating_add(exact_lane_cleanup_handoffs)
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
        let _task_lifecycle = self
            .inner
            .task_lifecycle_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The owner lease is the mutable authority for a runtime capability.
        // Binding may only establish or narrow its policy, never broaden it.
        self.inner.owner_leases.bind_policy(&caller)?;
        self.inner.task_download_authority.register_owner(
            caller.task_resource_family_key().as_str(),
            &caller.owner_lease_id,
        )?;
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
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
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
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
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

    /// Close only turn-scoped Lanes. A Lane explicitly marked keep-alive is
    /// user-pinned work (for example media playback) and survives the normal
    /// turn boundary; owner teardown still uses `close_owner_lease` and
    /// therefore always reclaims it.
    pub async fn close_turn_lanes(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.close_matching(|lane| {
            &lane.caller.owner_lease_id == lease_id && !lane.keep_alive
        })
        .await
    }

    /// Revokes one exact owner lease and closes only the lanes that carry that
    /// lease. This capability-scoped path is how runtime lifecycle teardown
    /// reclaims Lanes (runtime kill/drop revokes the owner lease).
    pub async fn close_owner_lease(
        &self,
        lease_id: &crate::OwnerLeaseId,
    ) -> Result<CloseResult, BrowserPlatformError> {
        // Revoke first so no new operation can validate this owner while its
        // resources are being detached. `renew` removes an already-expired
        // lease before returning its error, so Lane cleanup must not depend on
        // this boolean: an expired/revoked lease may still have orphaned Lane
        // records that require authoritative cleanup.
        {
            let _task_lifecycle = self
                .inner
                .task_lifecycle_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.inner.owner_leases.revoke(lease_id);
        }

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
        let result = self.close_owner_lanes(lease_id).await;
        if result.is_ok() {
            let _task_lifecycle = self
                .inner
                .task_lifecycle_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.inner.task_download_authority.retire_owner(lease_id);
        }
        result
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
            if (!target.requires_host_stop && shared_by_sibling)
                || target.browser_epoch == 0
            {
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
            let completed_lane_ids = completed
                .iter()
                .filter(|target| {
                    // A logical Lane may already have rebound to a replacement
                    // Host epoch. Its exact cleanup token follows that live
                    // Lane and must not be released by late proof for the old
                    // epoch.
                    !snapshots
                        .iter()
                        .any(|snapshot| snapshot.lane_id == target.lane_id)
                })
                .map(|target| target.lane_id.clone())
                .collect::<HashSet<_>>();
            let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
            if let Some(targets) = owner_targets.get_mut(owner_lease_id) {
                targets.retain(|target| !completed.contains(target));
                if targets.is_empty() {
                    owner_targets.remove(owner_lease_id);
                }
            }
            drop(owner_targets);
            // `shared_by_sibling` is an exact Lane cleanup proof even though
            // the shared Primary Host intentionally stays alive. Releasing
            // only at Host shutdown would leak one token per ordinary
            // open/close cycle and permanently fence a task after 128 cycles.
            for lane_id in completed_lane_ids {
                self.release_lane_cleanup_budget_if_unowned(&lane_id)
                    .await;
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
        task_id: &str,
        operation: &BrowserOperation,
        cancellation: &CancellationToken,
        wait_deadline: Instant,
    ) -> Result<HubDriverPermit, BrowserPlatformError> {
        let resource_class = DriverResourceClass::for_operation(operation);
        let acquired_weight = self
            .acquire_operation_weight(
                task_id,
                resource_class.weight(),
                cancellation,
                wait_deadline,
            )
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
            task_id: task_id.to_owned(),
            resource_class,
            acquired_weight,
        })
    }

    async fn acquire_operation_weight(
        &self,
        task_id: &str,
        weight: u64,
        cancellation: &CancellationToken,
        wait_deadline: Instant,
    ) -> Result<u64, BrowserPlatformError> {
        loop {
            let notified = self.inner.operation_capacity_changed.notified();
            let acquired = {
                // Serialize admissions with pressure/policy limit updates.
                // Releases remain lock-free and can only create more room.
                let _budget_guard = self.inner.operation_budget_gate.lock().await;
                let limit = self.inner.operation_weight_limit.load(Ordering::Acquire);
                let current = self.inner.active_operation_weight.load(Ordering::Acquire);
                let task_limit = self
                    .inner
                    .task_operation_weight_limit
                    .load(Ordering::Acquire);
                let mut task_weights = self
                    .inner
                    .task_active_operation_weight
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let task_current = task_weights.get(task_id).copied().unwrap_or_default();
                // A weighted operation must still make progress when a user
                // policy or Critical pressure lowers the entire budget below
                // its nominal weight. Admit an oversized operation only into
                // an empty budget, and retain its nominal weight so it stays
                // exclusive even if the policy limit rises while it runs.
                let oversized_exclusive = current == 0 && weight > limit;
                let task_oversized_exclusive = task_current == 0 && weight > task_limit;
                let global_available =
                    oversized_exclusive || current.saturating_add(weight) <= limit;
                let task_available = task_oversized_exclusive
                    || task_current.saturating_add(weight) <= task_limit;
                if global_available && task_available {
                    self.inner
                        .active_operation_weight
                        .fetch_add(weight, Ordering::AcqRel);
                    task_weights.insert(
                        task_id.to_owned(),
                        task_current.saturating_add(weight),
                    );
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
                _ = tokio::time::sleep_until(wait_deadline) => {
                    return Err(operation_queue_wait_timeout_error(None));
                }
            }
        }
    }

    fn try_acquire_operation_admission(
        &self,
        lane_id: &BrowserLaneId,
        task_id: &str,
    ) -> Result<OperationAdmissionPermit, BrowserPlatformError> {
        let global_limit = self
            .inner
            .operation_admission_global_limit
            .load(Ordering::Acquire)
            .max(1);
        let task_limit = self
            .inner
            .operation_admission_task_limit
            .load(Ordering::Acquire)
            .max(1);
        let lane_limit = self
            .inner
            .operation_admission_lane_limit
            .load(Ordering::Acquire)
            .max(1);
        let mut state = self
            .inner
            .operation_admissions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let task_count = state.by_task.get(task_id).copied().unwrap_or_default();
        let lane_count = state.by_lane.get(lane_id).copied().unwrap_or_default();
        let saturated_scope = if state.total >= global_limit {
            Some("global")
        } else if task_count >= task_limit {
            Some("task")
        } else if lane_count >= lane_limit {
            Some("lane")
        } else {
            None
        };
        if let Some(scope) = saturated_scope {
            return Err(operation_admission_busy_error(
                lane_id.clone(),
                scope,
                global_limit,
                task_limit,
                lane_limit,
            ));
        }
        state.total = state.total.saturating_add(1);
        *state.by_task.entry(task_id.to_owned()).or_default() += 1;
        *state.by_lane.entry(lane_id.clone()).or_default() += 1;
        Ok(OperationAdmissionPermit {
            inner: Arc::clone(&self.inner),
            lane_id: lane_id.clone(),
            task_id: task_id.to_owned(),
        })
    }

    fn apply_operation_admission_limits(&self, policy: &ResourcePolicy) {
        let (global, task, lane) = operation_admission_limits(policy);
        self.inner
            .operation_admission_global_limit
            .store(global, Ordering::Release);
        self.inner
            .operation_admission_task_limit
            .store(task, Ordering::Release);
        self.inner
            .operation_admission_lane_limit
            .store(lane, Ordering::Release);
        self.inner.operation_capacity_changed.notify_waiters();
    }

    async fn apply_operation_weight_limits(&self, global_limit: usize, task_limit: usize) {
        let _budget_guard = self.inner.operation_budget_gate.lock().await;
        self.inner
            .operation_weight_limit
            .store(global_limit.max(1) as u64, Ordering::Release);
        self.inner
            .task_operation_weight_limit
            .store(task_limit.max(1) as u64, Ordering::Release);
        drop(_budget_guard);
        self.inner.operation_capacity_changed.notify_waiters();
    }

    fn reserve_cleanup_lane_for_existing_host(
        &self,
        task_id: &str,
        family_id: &str,
        host_key: &HostKey,
        lane_id: &BrowserLaneId,
    ) -> Result<(), BrowserPlatformError> {
        let token = self
            .inner
            .cleanup_budget
            .reserve_lane_for_family(
                task_id,
                family_id,
                host_key.clone(),
                lane_id.clone(),
            )
            .map_err(|error| {
                self.cleanup_budget_error(error, task_id, family_id, host_key)
            })?;
        self.inner
            .lane_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(lane_id.clone(), token);
        Ok(())
    }

    fn reserve_cleanup_lane_and_host(
        &self,
        task_id: &str,
        family_id: &str,
        host_key: &HostKey,
        browser_epoch: u64,
        lane_id: &BrowserLaneId,
    ) -> Result<(), BrowserPlatformError> {
        let host_authority = HostCleanupAuthorityKey {
            host_key: host_key.clone(),
            browser_epoch,
        };
        let (lane_token, host_token) = self
            .inner
            .cleanup_budget
            .reserve_lane_and_host_for_family(
                task_id,
                family_id,
                host_key.clone(),
                lane_id.clone(),
                host_authority.clone(),
            )
            .map_err(|error| {
                self.cleanup_budget_error(error, task_id, family_id, host_key)
            })?;
        self.inner
            .lane_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(lane_id.clone(), lane_token);
        self.inner
            .host_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(host_authority, host_token);
        Ok(())
    }

    fn reserve_cleanup_host(
        &self,
        task_id: &str,
        family_id: &str,
        host_key: &HostKey,
        browser_epoch: u64,
    ) -> Result<(), BrowserPlatformError> {
        let host_authority = HostCleanupAuthorityKey {
            host_key: host_key.clone(),
            browser_epoch,
        };
        let token = self
            .inner
            .cleanup_budget
            .reserve_host_for_family(
                task_id,
                family_id,
                host_key.clone(),
                host_authority.clone(),
            )
            .map_err(|error| {
                self.cleanup_budget_error(error, task_id, family_id, host_key)
            })?;
        self.inner
            .host_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(host_authority, token);
        Ok(())
    }

    fn release_lane_cleanup_budget(&self, lane_id: &BrowserLaneId) {
        let token = self
            .inner
            .lane_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(lane_id);
        if let Some(token) = token {
            self.inner.cleanup_budget.release(&token);
        }
    }

    fn release_host_cleanup_budget(&self, host_key: &HostKey, browser_epoch: u64) {
        let authority = HostCleanupAuthorityKey {
            host_key: host_key.clone(),
            browser_epoch,
        };
        let token = self
            .inner
            .host_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&authority);
        if let Some(token) = token {
            self.inner.cleanup_budget.release(&token);
        }
    }

    fn mark_prepared_rebind_authority(
        &self,
        lane_id: BrowserLaneId,
        host_key: &HostKey,
        browser_epoch: u64,
    ) -> PreparedRebindAuthorityGuard {
        let host_authority = HostCleanupAuthorityKey {
            host_key: host_key.clone(),
            browser_epoch,
        };
        self.inner
            .prepared_rebind_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert((lane_id.clone(), host_authority.clone()));
        PreparedRebindAuthorityGuard {
            inner: Arc::downgrade(&self.inner),
            lane_id,
            host_authority,
            armed: true,
        }
    }

    fn has_prepared_rebind_authority(
        &self,
        lane_id: &BrowserLaneId,
        host_key: &HostKey,
        browser_epoch: u64,
    ) -> bool {
        self.inner
            .prepared_rebind_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&(
                lane_id.clone(),
                HostCleanupAuthorityKey {
                    host_key: host_key.clone(),
                    browser_epoch,
                },
            ))
    }

    fn require_exact_host_stop(&self, host_key: &HostKey, browser_epoch: u64) {
        self.inner
            .host_stop_required_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(HostCleanupAuthorityKey {
                host_key: host_key.clone(),
                browser_epoch,
            });
        self.ensure_cleanup_retry_worker();
    }

    fn cleanup_budget_error(
        &self,
        error: CleanupBudgetError,
        task_id: &str,
        _family_id: &str,
        host_key: &HostKey,
    ) -> BrowserPlatformError {
        if let Some(saturation) = error.saturation().copied() {
            self.request_cleanup_ledger_reconcile();
            return cleanup_budget_capacity_error(
                task_id,
                host_key,
                saturation.scope,
                saturation,
            );
        }
        cleanup_budget_invariant_error(error, task_id, host_key)
    }

    fn cleanup_budget_fence_error(
        &self,
        task_id: &str,
        family_id: &str,
        host_key: &HostKey,
    ) -> Option<BrowserPlatformError> {
        let snapshot = self.inner.cleanup_budget.snapshot();
        let (scope, blocked) = if snapshot.global.latched {
            (CleanupBudgetScope::Global, snapshot.global)
        } else if let Some(blocked) = snapshot
            .tasks
            .get(task_id)
            .copied()
            .filter(|scope| scope.latched)
        {
            (CleanupBudgetScope::Task, blocked)
        } else if let Some(blocked) = snapshot
            .families
            .get(family_id)
            .copied()
            .filter(|scope| scope.latched)
        {
            (CleanupBudgetScope::Family, blocked)
        } else if let Some(blocked) = snapshot
            .hosts
            .get(host_key)
            .copied()
            .filter(|scope| scope.latched)
        {
            (CleanupBudgetScope::Host, blocked)
        } else {
            return None;
        };
        self.request_cleanup_ledger_reconcile();
        Some(cleanup_budget_capacity_error(
            task_id,
            host_key,
            scope,
            CleanupBudgetSaturation {
                scope,
                count: blocked.count,
                requested_units: 1,
                hard_max: blocked.hard_max,
                low_water: blocked.low_water,
                latched: blocked.latched,
            },
        ))
    }

    /// Ask the fixed cleanup supervisor for one bounded ledger pass. Every
    /// concrete cleanup target remains in the exact Lane/Host/debt ledgers;
    /// this flag deliberately stores no user-controlled scope keys.
    fn request_cleanup_ledger_reconcile(&self) {
        self.inner
            .cleanup_ledger_reconcile_requested
            .store(true, Ordering::Release);
        self.ensure_cleanup_retry_worker();
    }

    /// Durably hands one already-admitted Lane to the independent cleanup
    /// supervisor. The request is sealed to the caller's owner generation and
    /// never broadens to a task/family/global predicate.
    fn handoff_exact_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
        authority: ExactLaneCleanupAuthority,
    ) -> Result<(), BrowserPlatformError> {
        let _admission_guard = self
            .inner
            .exact_lane_cleanup_admission_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A detached Lane is already either clean or represented by the Hub's
        // pending driver/Host authority. Refusing arbitrary ids here also
        // makes the ledger cardinality a direct function of scheduler limits.
        if !self.inner.scheduler.contains_lane(&lane_id) {
            return Ok(());
        }
        self.inner
            .exact_lane_cleanup_handoffs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(lane_id, authority)?;
        self.ensure_cleanup_retry_worker();
        Ok(())
    }

    fn exact_lane_cleanup_admission_fence_error(
        &self,
        lane_id: &BrowserLaneId,
        task_id: &str,
        family_id: &str,
    ) -> Option<BrowserPlatformError> {
        let retained_global = self.inner.scheduler.retained_lane_count();
        let retained_family = self.inner.scheduler.retained_lane_count_for(family_id);
        let ledger = self
            .inner
            .exact_lane_cleanup_handoffs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let global_required = ledger
            .entries
            .len()
            .saturating_add(retained_global.saturating_add(1).saturating_mul(2));
        let task_required = ledger
            .task_counts
            .get(task_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(retained_family.saturating_add(1).saturating_mul(2));
        let family_required = ledger
            .family_counts
            .get(family_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(retained_family.saturating_add(1).saturating_mul(2));
        if global_required <= MAX_EXACT_LANE_CLEANUP_HANDOFFS
            && task_required <= MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS
            && family_required <= MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS
        {
            return None;
        }
        Some(exact_lane_cleanup_capacity_error(
            lane_id,
            ledger.entries.len(),
            ledger.task_counts.get(task_id).copied().unwrap_or(0),
            ledger
                .family_counts
                .get(family_id)
                .copied()
                .unwrap_or(0),
        ))
    }

    async fn process_exact_lane_cleanup_handoffs(
        &self,
    ) -> Result<(), BrowserPlatformError> {
        let requests = self
            .inner
            .exact_lane_cleanup_handoffs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .iter()
            .map(|(lane_id, authority)| (lane_id.clone(), authority.clone()))
            .collect::<Vec<_>>();
        let mut first_error = None;
        for (lane_id, authority) in requests {
            let lane = self.inner.lanes.read().await.get(&lane_id).cloned();
            let matches_authority = if let Some(lane) = lane {
                let snapshot = lane.current_snapshot().await;
                snapshot.caller.user_id == authority.user_id
                    && snapshot.caller.owner_lease_id == authority.owner_lease_id
                    && snapshot.caller.task_resource_key() == authority.task_id
                    && snapshot.caller.task_resource_family_key().as_str() == authority.family_id
            } else {
                false
            };
            if !matches_authority {
                // Lane ids are UUIDv7 and are never reused. Absence means an
                // earlier exact close already transferred physical authority;
                // a mismatch is stale/invalid authority and must not broaden.
                self.inner
                    .exact_lane_cleanup_handoffs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&lane_id);
                continue;
            }

            let result = self.close_lane(&lane_id).await;
            if !self.inner.scheduler.contains_lane(&lane_id) {
                self.inner
                    .exact_lane_cleanup_handoffs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&lane_id);
            }
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn resource_workload(&self, lane_cold_start_bytes: u64) -> ResourceWorkload {
        let active_requests = self.inner.scheduler.active_requests();
        // Workload accounting is order-independent; the unordered snapshot
        // avoids running the O(queue^2) promotion simulation under the
        // scheduler mutex on every admission/release decision.
        let queued_requests = self.inner.scheduler.queued_requests_unordered();
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
        // Detached Lanes disappear from both scheduler and live inventory,
        // but their target close or still-running Host.open_lane can continue
        // owning a renderer/process share. Count each retained Lane exactly
        // once so a stream of new tasks cannot make unresolved starts and
        // cleanup debt invisible to the admission predictor.
        let mut retained_lanes = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .map(|pending| pending.lane_id.clone())
            .collect::<HashSet<_>>();
        retained_lanes.extend(
            self.inner
                .pending_host_retirements
                .lock()
                .await
                .iter()
                .map(|pending| pending.lane_id.clone()),
        );
        let retained_count = retained_lanes.len();
        let mut retained_hosts = self
            .inner
            .retiring_host_slots
            .lock()
            .await
            .iter()
            .map(|(_, slot)| Arc::as_ptr(slot) as usize)
            .collect::<HashSet<_>>();
        retained_hosts.extend(
            self.inner
                .orphaned_host_slots
                .lock()
                .await
                .iter()
                .map(|(_, slot)| Arc::as_ptr(slot) as usize),
        );
        let retained_count = retained_count.saturating_add(retained_hosts.len());
        workload.queued_lanes = workload.queued_lanes.saturating_add(retained_count);
        workload.queued_lane_estimate_bytes = workload.queued_lane_estimate_bytes.saturating_add(
            lane_cold_start_bytes
                .saturating_mul(u64::try_from(retained_count).unwrap_or(u64::MAX)),
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

    fn promotion_policy(
        decision: &ResourceDecision,
        blocked_tasks: BTreeSet<String>,
    ) -> PromotionPolicy {
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
        .with_blocked_owners(blocked_tasks, "task_resource_limit")
    }

    async fn task_cleanup_debt_owners(&self) -> BTreeSet<String> {
        // Owner cleanup targets are durable revoke authority, not by
        // themselves evidence that cleanup is outstanding. A target may
        // legitimately outlive a Lane start error or an epoch transition
        // until the exact Host proof is reconciled. Only an actual retained
        // cleanup flight fences new task expansion.
        let mut blocked = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .map(|pending| pending.task_id.clone())
            .collect::<BTreeSet<_>>();
        blocked.extend(
            self.inner
                .pending_host_retirements
                .lock()
                .await
                .iter()
                .filter(|pending| pending.start_flight.result.get().is_none())
                .map(|pending| pending.task_id.clone()),
        );
        blocked
    }

    async fn task_cleanup_debt_families(&self) -> BTreeSet<String> {
        let mut blocked = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .map(|pending| pending.family_id.clone())
            .collect::<BTreeSet<_>>();
        blocked.extend(
            self.inner
                .pending_host_retirements
                .lock()
                .await
                .iter()
                .filter(|pending| pending.start_flight.result.get().is_none())
                .map(|pending| pending.family_id.clone()),
        );
        blocked
    }

    async fn task_memory_attributions(
        &self,
        policy: &ResourcePolicy,
    ) -> HashMap<String, TaskMemoryAttribution> {
        let mut attributions = self.inner.task_memory_samples.read().await.clone();
        let sampled_lanes = self.inner.lane_memory_samples.read().await.clone();
        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        for lane in records {
            let snapshot = lane.current_snapshot().await;
            if lane.closing.load(Ordering::Acquire)
                || !matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Starting
                        | LaneLifecycleState::Running
                        | LaneLifecycleState::Frozen
                )
                || sampled_lanes.contains_key(&snapshot.lane_id)
            {
                continue;
            }
            let entry = attributions
                .entry(snapshot.caller.task_resource_family_key().into_string())
                .or_default();
            entry.shared_rss_estimate_bytes = entry.shared_rss_estimate_bytes.saturating_add(
                snapshot
                    .resource_estimate_bytes
                    .max(policy.lane_cold_start_bytes),
            );
            // Missing Host telemetry is not sufficient evidence for a hard
            // task-local kill on a potentially shared Host.
            entry.exclusive_hosts_only = false;
        }

        let mut retained = HashSet::new();
        for pending in self.inner.pending_lane_cleanups.lock().await.iter() {
            retained.insert((pending.family_id.clone(), pending.lane_id.clone()));
        }
        for pending in self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .filter(|pending| pending.start_flight.result.get().is_none())
        {
            retained.insert((pending.family_id.clone(), pending.lane_id.clone()));
        }
        for (task_id, _) in retained {
            let entry = attributions.entry(task_id).or_default();
            entry.shared_rss_estimate_bytes = entry
                .shared_rss_estimate_bytes
                .saturating_add(policy.lane_cold_start_bytes);
            entry.exclusive_hosts_only = false;
        }
        attributions
    }

    async fn blocked_task_owners(&self, policy: &ResourcePolicy) -> BTreeSet<String> {
        // Scheduler owner ids are quota families. Exact runtime cleanup debt
        // is rejected before scheduler admission in `open_lane`; it must not
        // block or close a healthy sibling runtime in the same conversation.
        // A family with retained physical cleanup debt may continue using its
        // healthy existing Lanes, but may not expand and accumulate another
        // generation of resources until exact cleanup converges.
        let mut blocked = self.task_cleanup_debt_families().await;
        let cleanup_snapshot = self.inner.cleanup_budget.snapshot();
        if cleanup_snapshot.global.latched {
            blocked.extend(
                self.inner
                    .scheduler
                    .queued_requests_unordered()
                    .into_iter()
                    .map(|request| request.owner_id),
            );
        } else {
            blocked.extend(
                cleanup_snapshot
                    .families
                    .iter()
                    .filter(|(_, scope)| scope.latched)
                    .map(|(family_id, _)| family_id.clone()),
            );
            let fenced_hosts = cleanup_snapshot
                .hosts
                .iter()
                .filter(|(_, scope)| scope.latched)
                .map(|(host_key, _)| host_key.clone())
                .collect::<HashSet<_>>();
            if !fenced_hosts.is_empty() {
                let queued = self.inner.scheduler.queued_requests_unordered();
                let records = self.inner.lanes.read().await;
                for request in queued {
                    let Some(lane) = records.get(&request.lane_id) else {
                        continue;
                    };
                    let snapshot = lane.snapshot.read().await;
                    let host_key = HostKey::for_lane(
                        snapshot.identity_mode,
                        snapshot.identity_generation,
                        &snapshot.lane_id,
                    );
                    if fenced_hosts.contains(&host_key) {
                        blocked.insert(request.owner_id);
                    }
                }
            }
        }
        blocked.extend(
            self.task_memory_attributions(policy)
                .await
                .into_iter()
                .filter(|(_, usage)| {
                    usage
                        .shared_rss_estimate_bytes
                        .saturating_add(policy.lane_cold_start_bytes)
                        > policy.max_task_memory_bytes
                })
                .map(|(task_id, _)| task_id),
        );
        blocked
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
        if identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            return Err(error);
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
        let runtime_cleanup_id = caller.runtime_cleanup_key().into_string();
        let family_id = caller.task_resource_family_key().into_string();
        let prospective_host_key = HostKey::for_lane(
            identity_mode,
            identity_generation,
            &lane_id,
        );
        if let Some(error) =
            self.cleanup_budget_fence_error(
                &runtime_cleanup_id,
                &family_id,
                &prospective_host_key,
            )
        {
            return Err(error);
        }
        if self
            .task_cleanup_debt_owners()
            .await
            .contains(&runtime_cleanup_id)
        {
            return Err(owner_cleanup_pending_error());
        }
        let existing_records: Vec<_> =
            self.inner.lanes.read().await.values().cloned().collect();
        let mut first_lane = true;
        for lane in existing_records {
            if lane
                .snapshot
                .read()
                .await
                .caller
                .task_resource_family_key()
                .as_str()
                == family_id
            {
                first_lane = false;
                break;
            }
        }
        let priority = if first_lane {
            LanePriority::First
        } else {
            LanePriority::Expansion
        };
        let policy = self.inner.config.read().await.resource_policy.clone();
        let task_blocked = self
            .blocked_task_owners(&policy)
            .await
            .contains(&family_id);
        let decision = {
            let telemetry = self.inner.telemetry.read().await.clone();
            self.decide_resources(&policy, &telemetry).await
        };
        self.inner
            .scheduler
            .update_recommended_concurrency(decision.recommended_concurrency);
        let allow_immediate = !task_blocked && match priority {
            LanePriority::First => decision.admit_first_lane,
            LanePriority::Expansion => decision.admit_expansion_lane,
        };
        let reason_code = if task_blocked {
            "task_resource_limit"
        } else { match priority {
            LanePriority::First => decision.first_lane_reason_code,
            LanePriority::Expansion => decision.expansion_lane_reason_code,
        }
        .unwrap_or("browser_capacity_queued") };
        let lane_cold_start_bytes = policy.lane_cold_start_bytes;
        // Acquire every async publication lock before scheduler admission.
        // From `admit` through both map inserts below there must be no `.await`:
        // cancellation may drop a future only at a poll boundary, so an
        // admitted Lane can never exist outside the authoritative inventory.
        let mut lanes = self.inner.lanes.write().await;
        let mut lane_keys = self.inner.lane_keys.write().await;
        // Install rollback authority before calling the scheduler. This also
        // covers an unwinding panic inside `admit` after it has mutated state,
        // not only panics in the subsequent inventory construction.
        let mut unpublished_admission = UnpublishedLaneAdmissionGuard::new(
            self.clone(),
            lane_id.clone(),
            lane_key.clone(),
            &mut lanes,
            &mut lane_keys,
        );
        let admission = {
            let _exact_cleanup_guard = self
                .inner
                .exact_lane_cleanup_admission_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(error) = self.exact_lane_cleanup_admission_fence_error(
                &lane_id,
                &runtime_cleanup_id,
                &family_id,
            ) {
                return Err(error);
            }
            self.inner.scheduler.admit(
                family_id,
                lane_id.clone(),
                priority,
                allow_immediate,
                reason_code,
            )?
        };

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
            resource_estimate_bytes: lane_cold_start_bytes,
            active_operation_count: 0,
            last_active_at_ms: now,
            created_at_ms: now,
            error_code: None,
            error_message: None,
            recoverable: true,
            keep_alive: false,
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
        unpublished_admission.publish(Arc::clone(&lane));
        drop(unpublished_admission);
        drop(lane_keys);
        drop(lanes);
        // The creation event precedes the start task so subscribers can never
        // observe lane_running before lane_created.
        self.emit("lane_created", Some(&snapshot));
        // Register the creating caller's start waiter while `open_gate` is
        // still held. `abandon_unclaimed_lane_start` serializes its
        // zero-waiter decision only with registrations performed under this
        // gate; registering after the gate drop would let a cancelled
        // duplicate open observe a 1->0 waiter transition and detach the Lane
        // its creator is about to wait on (spurious lane_closed for an open
        // nobody closed).
        let action = match admission {
            Admission::Ready => {
                let flight = start_flight.expect("Ready Lane must have a start flight");
                self.spawn_lane_start(
                    lane_id.clone(),
                    Arc::clone(&lane),
                    Arc::clone(&flight),
                    false,
                );
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
        drop(_open_guard);
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
        promoted_from_queue: bool,
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
            let deferred = promoted_from_queue
                && result.as_ref().err().is_some_and(|error| {
                    error.metadata["reason_code"]
                        == "browser_cleanup_budget_saturated"
                })
                && hub
                    .defer_promoted_lane_to_queue(
                        &lane_id,
                        &lane,
                        "browser_cleanup_budget_saturated",
                    )
                    .await;
            if failed && !deferred {
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
            if hub
                .inner
                .abandoned_lane_starts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&lane_id)
            {
                hub.process_abandoned_lane_starts().await;
            }
            if failed && !deferred {
                hub.finalize_hosts_ready_after_cleanup().await;
                hub.promote_released_capacity().await;
            }
        });
    }

    async fn defer_promoted_lane_to_queue(
        &self,
        lane_id: &BrowserLaneId,
        lane: &Arc<LaneRecord>,
        reason_code: &'static str,
    ) -> bool {
        let Some(request) = self
            .inner
            .scheduler
            .defer_active_to_queue(lane_id, reason_code)
        else {
            return false;
        };
        let queue = match self.inner.scheduler.metadata(&request.request_id) {
            Ok(queue) => queue,
            Err(_) => return false,
        };
        if lane.closing.load(Ordering::Acquire)
            || !self
                .inner
                .lanes
                .read()
                .await
                .get(lane_id)
                .is_some_and(|current| Arc::ptr_eq(current, lane))
        {
            self.inner.scheduler.cancel_lane(lane_id);
            return false;
        }
        let snapshot = {
            let mut snapshot = lane.snapshot.write().await;
            snapshot.lifecycle_state = LaneLifecycleState::Queued;
            snapshot.queue = Some(queue);
            snapshot.error_code = None;
            snapshot.error_message = None;
            snapshot.recoverable = true;
            snapshot.clone()
        };
        self.emit("lane_queued", Some(&snapshot));
        true
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

    fn request_abandon_unclaimed_lane_start(&self, lane_id: BrowserLaneId) {
        self.inner
            .abandoned_lane_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(lane_id);
        // `ensure_cleanup_retry_worker` uses Handle::try_current and retains
        // the request when Drop runs without a Tokio runtime. There is no
        // per-waiter spawned task and no runtime-shutdown panic.
        self.ensure_cleanup_retry_worker();
    }

    async fn process_abandoned_lane_starts(&self) {
        let lane_ids = self
            .inner
            .abandoned_lane_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        for lane_id in lane_ids {
            let lane = self.inner.lanes.read().await.get(&lane_id).cloned();
            if let Some(lane) = lane {
                self.abandon_unclaimed_lane_start(&lane_id, &lane).await;
            }
            self.inner
                .abandoned_lane_starts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&lane_id);
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
        let (
            identity_mode,
            identity_generation,
            runtime_cleanup_key,
            task_resource_family_key,
        ) = {
            let snapshot = lane.snapshot.read().await;
            (
                snapshot.identity_mode,
                snapshot.identity_generation,
                snapshot.caller.runtime_cleanup_key().into_string(),
                snapshot.caller.task_resource_family_key().into_string(),
            )
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
            .get_or_launch_host(
                identity_mode,
                identity_generation,
                &lane_id,
                &runtime_cleanup_key,
                &task_resource_family_key,
            )
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
                .entry(caller.owner_lease_id.clone())
                .or_default()
                .insert(OwnerCleanupTarget {
                    user_id: caller.user_id.clone(),
                    task_id: caller.task_resource_key(),
                    family_id: caller.task_resource_family_key().into_string(),
                    lane_id: lane_id.clone(),
                    host_key: host.key.clone(),
                    browser_epoch: host.slot.epoch,
                    requires_host_stop: true,
                });
        }
        let host_driver = Arc::clone(&host.driver);
        let host_slot = Arc::clone(&host.slot);
        let host_admission_gate = Arc::clone(&host.slot.admission_gate);
        let admitted_host_key = host.key.clone();
        let max_task_tabs = self.inner.config.read().await.resource_policy.max_task_tabs;
        let request = LaneLaunchRequest {
            lane_id: lane_id.clone(),
            identity_mode,
            workspace_hint: lane.workspace_hint.clone(),
            // Engine reliable-event and target reservations are quota-family
            // scoped; exact cleanup remains sealed separately in the Hub.
            task_resource_key: task_resource_family_key,
            max_task_tabs,
            task_tab_authority: Arc::clone(&self.inner.task_tab_authority)
                as Arc<dyn BrowserTaskTabAuthority>,
            task_download_authority: Arc::clone(&self.inner.task_download_authority)
                as Arc<dyn BrowserTaskDownloadAuthority>,
        };
        let mut open_lane_task = tokio::spawn(async move {
            let host_admission = host_admission_gate.read_owned().await;
            let result = if host_slot.retired.load(Ordering::Acquire) {
                Err(anonymous_profile_hygiene_error(
                    &admitted_host_key,
                    "host_admission_fenced",
                ))
            } else {
                host_driver.open_lane(request).await
            };
            // Return the admission authority with the driver result. The Hub
            // keeps it until the exact driver and epoch are either published
            // together or handed to retained cleanup authority.
            (result, host_admission)
        });
        let open_lane = match tokio::time::timeout(
            HOST_LANE_OPEN_TIMEOUT,
            &mut open_lane_task,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                open_lane_task.abort();
                let _ = open_lane_task.await;
                let error = host_open_lane_timeout_error(
                    lane_id.clone(),
                    host.slot.epoch,
                );
                self.require_exact_host_stop(&host.key, host.slot.epoch);
                self.mark_lane_failed(&lane, &error).await;
                // Release the Primary transition gate before entering the
                // normal restart path, which acquires the same gate. Recovery
                // stops the exact old Host and rebinds healthy siblings; the
                // timed-out Starting Lane is discarded by the flight owner.
                drop(_primary_visibility_guard);
                if let Err(recovery_error) = self
                    .recover_host_failure(host.key.clone(), host.slot.epoch)
                    .await
                {
                    tracing::warn!(
                        browser_epoch = host.slot.epoch,
                        code = ?recovery_error.code,
                        "timed-out browser Lane Host recovery remains pending"
                    );
                }
                return Err(error);
            }
        };
        let (open_lane, host_admission_guard) = match open_lane {
            Ok(joined) => joined,
            Err(join_error) => {
                tracing::error!(
                    lane_id = %lane_id,
                    browser_epoch = host.slot.epoch,
                    cancelled = join_error.is_cancelled(),
                    panic = join_error.is_panic(),
                    "browser Host open_lane task terminated unexpectedly"
                );
                let error = host_open_lane_task_failed_error(lane_id.clone(), &join_error);
                self.require_exact_host_stop(&host.key, host.slot.epoch);
                self.mark_lane_failed(&lane, &error).await;
                drop(_primary_visibility_guard);
                if let Err(recovery_error) = self
                    .recover_host_failure(host.key.clone(), host.slot.epoch)
                    .await
                {
                    tracing::warn!(
                        browser_epoch = host.slot.epoch,
                        code = ?recovery_error.code,
                        "failed browser Lane open task left exact Host recovery pending"
                    );
                }
                return Err(error);
            }
        };
        let driver = match open_lane {
            Ok(driver) => driver,
            Err(error) => {
                drop(host_admission_guard);
                self.require_exact_host_stop(&host.key, host.slot.epoch);
                self.mark_lane_failed(&lane, &error).await;
                drop(_primary_visibility_guard);
                if let Err(recovery_error) = self
                    .recover_host_failure(host.key.clone(), host.slot.epoch)
                    .await
                {
                    tracing::warn!(
                        browser_epoch = host.slot.epoch,
                        code = ?recovery_error.code,
                        "failed browser Lane open left exact Host recovery pending"
                    );
                }
                return Err(error.for_lane(lane_id));
            }
        };
        // A concrete Lane driver is now available, so its close can become an
        // exact target-local proof. Replace (rather than duplicate) the
        // pre-open unknown-side-effect authority in the owner's HashSet.
        {
            let caller = lane.snapshot.read().await.caller.clone();
            self.inner
                .owner_cleanup_targets
                .lock()
                .await
                .entry(caller.owner_lease_id.clone())
                .or_default()
                .replace(OwnerCleanupTarget {
                    user_id: caller.user_id.clone(),
                    task_id: caller.task_resource_key(),
                    family_id: caller.task_resource_family_key().into_string(),
                    lane_id: lane_id.clone(),
                    host_key: host.key.clone(),
                    browser_epoch: host.slot.epoch,
                    requires_host_stop: false,
                });
        }
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
                    lane.snapshot.read().await.caller.task_resource_key(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .task_resource_family_key()
                        .into_string(),
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
            drop(host_admission_guard);
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
                    lane.snapshot.read().await.caller.task_resource_key(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .task_resource_family_key()
                        .into_string(),
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
            drop(host_admission_guard);
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
        drop(host_admission_guard);
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
            // A caller-cancellation path may have detached the Starting Lane
            // before the Hub-owned start flight publishes its factory error.
            // Re-run the exact no-driver/no-target proof after the flight has
            // settled so that ordering cannot strand the admission token.
            self.release_lane_cleanup_budget_if_unowned(lane_id)
                .await;
            return;
        };
        if let Some(cleanup_id) = detached.cleanup_id {
            let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
            return;
        }

        self.release_lane_cleanup_budget_if_unowned(lane_id)
            .await;
    }

    async fn release_lane_cleanup_budget_if_unowned(
        &self,
        lane_id: &BrowserLaneId,
    ) {
        // A clean Host-factory error happens before `start_lane_once` can
        // register an exact owner target or publish a Lane driver. Another
        // caller may subsequently initialize the same shared HostSlot, so we
        // cannot wait for Host shutdown to reclaim this failed Lane's token.
        // Conversely, Host.open_lane failures do publish an owner target; its
        // token stays authoritative until shared-Lane or exact-Host proof.
        let lane_is_live = self.inner.lanes.read().await.contains_key(lane_id);
        let has_pending_driver = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .any(|entry| entry.lane_id == *lane_id);
        let has_exact_target = self
            .inner
            .owner_cleanup_targets
            .lock()
            .await
            .values()
            .any(|targets| targets.iter().any(|target| target.lane_id == *lane_id));
        let has_prepared_rebind = self
            .inner
            .prepared_rebind_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|(prepared_lane_id, _)| prepared_lane_id == lane_id);
        if !lane_is_live
            && !has_pending_driver
            && !has_exact_target
            && !has_prepared_rebind
        {
            self.release_lane_cleanup_budget(lane_id);
        }
    }

    async fn retain_pending_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
        user_id: String,
        task_id: String,
        family_id: String,
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
                task_id: task_id.clone(),
                family_id: family_id.clone(),
                lane_id: lane_id.clone(),
                host_key: host_key.clone(),
                browser_epoch,
                requires_host_stop: false,
            });
        pending.push(Arc::new(PendingLaneCleanup {
            cleanup_id,
            lane_id,
            user_id,
            task_id,
            family_id,
            owner_lease_id,
            host_key,
            browser_epoch,
            driver,
            flight: Mutex::new(None),
        }));
        drop(owner_targets);
        drop(pending);
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
        runtime_cleanup_key: &str,
        task_resource_family_key: &str,
    ) -> Result<HostHandle, BrowserPlatformError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(BrowserPlatformError::shutting_down());
        }
        let key = HostKey::for_lane(identity_mode, identity_generation, lane_id);
        if identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            return Err(error);
        }
        if let Some(fenced_epoch) = self.anonymous_profile_fence_epoch(&key) {
            self.spawn_anonymous_profile_rotation(key.clone(), fenced_epoch);
            return Err(anonymous_profile_hygiene_error(&key, "rotation_pending"));
        }
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
            if identity_mode == BrowserIdentityMode::Primary
                && let Some(error) = self.primary_profile_fence_error()
            {
                return Err(error);
            }
            if let Some(fenced_epoch) = self.anonymous_profile_fence_epoch(&key) {
                self.spawn_anonymous_profile_rotation(key.clone(), fenced_epoch);
                return Err(anonymous_profile_hygiene_error(&key, "rotation_pending"));
            }
            let current = { self.inner.host_slots.read().await.get(&key).cloned() };
            if let Some(slot) = current {
                self.reserve_cleanup_lane_for_existing_host(
                    runtime_cleanup_key,
                    task_resource_family_key,
                    &key,
                    lane_id,
                )?;
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
                let epoch =
                    self.inner.host_epoch_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                self.reserve_cleanup_lane_and_host(
                    runtime_cleanup_key,
                    task_resource_family_key,
                    &key,
                    epoch,
                    lane_id,
                )?;
                let slot = Arc::new(HostSlot::new(
                    epoch,
                    false,
                    self.inner.clock.now_ms(),
                ));
                slots.insert(key.clone(), Arc::clone(&slot));
                slot
            }
        };
        // Do not allocate per-Host circuit state until physical cleanup
        // authority has been reserved successfully. In particular, a stream
        // of unique Isolated Lane ids rejected by a saturated cleanup ledger,
        // shutdown/drain fencing, or a retirement wait must not grow this map
        // without any Host/Lane resource that can later evict the entry.
        let circuit = self.host_circuit(&key).await;
        let circuit_attempt = circuit.acquire_attempt()?;
        let half_open_probe = circuit_attempt.is_half_open();
        match self.initialize_host_slot(&key, Arc::clone(&slot)).await {
            Ok(driver) => {
                circuit_attempt.succeed();
                Ok(HostHandle { key, slot, driver })
            }
            Err(error) => {
                // A cold-start timeout or a factory error with deferred exact
                // cleanup retires the slot. The Hub-owned launch flight keeps
                // running, and its process/profile lease keeps this HostKey
                // fenced until physical absence is proven. Only then may a
                // retry receive a fresh slot and epoch.
                if host_launch_requires_retirement(&error)
                    && self
                        .retire_host_slot_for_cleanup(&key, slot.epoch, &slot)
                        .await
                {
                    let _ = self
                        .attempt_orphaned_host_slot_cleanup(&key, &slot)
                        .await;
                }
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
        if identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            return Err(error);
        }
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
            .get_or_try_init(move |cleanup_lease| async move {
                factory
                    .launch(HostLaunchRequest {
                        host_id: BrowserHostId::new(),
                        browser_epoch,
                        identity_mode,
                        identity_generation: host_identity_generation,
                        identity_snapshot_payload,
                        headful,
                        cleanup_lease,
                    })
                    .await
            })
            .await?;
        if identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            // A fence may be published while the Hub-owned factory future is
            // in flight. Register this late exact epoch before returning so a
            // restart/visibility transition cannot rebind the new process.
            self.publish_admitted_primary_profile_fence(
                slot.epoch,
                &slot,
                "late_host_launch_after_fence",
            );
            return Err(error);
        }
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

    fn primary_profile_fence(&self) -> Option<PrimaryProfileFence> {
        self.inner.primary_profile_fence.get().copied()
    }

    fn primary_profile_fence_error(&self) -> Option<BrowserPlatformError> {
        self.primary_profile_fence()
            .map(primary_profile_storage_limit_error)
    }

    /// Publishes a process-lifetime Primary fence and exact cleanup debt with
    /// no await point. A concurrently finishing replacement Host registers its
    /// own epoch here as well, so the first trigger can never leave a late
    /// process outside retained cleanup authority.
    fn publish_admitted_primary_profile_fence(
        &self,
        observed_epoch: u64,
        observed_slot: &Arc<HostSlot>,
        reason: &'static str,
    ) -> bool {
        observed_slot.retire();
        let inserted = self
            .inner
            .primary_profile_fence
            .set(PrimaryProfileFence {
                trigger_epoch: observed_epoch,
                reason,
            })
            .is_ok();
        self.inner
            .primary_profile_cleanup_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(observed_epoch);
        if inserted {
            tracing::warn!(
                browser_epoch = observed_epoch,
                reason,
                "fenced the stable Primary profile while preserving identity data"
            );
        }
        self.spawn_primary_profile_cleanup(observed_epoch);
        inserted
    }

    fn spawn_primary_profile_cleanup(&self, observed_epoch: u64) {
        if !self
            .inner
            .primary_profile_cleanup_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&observed_epoch)
        {
            return;
        }
        {
            let mut workers = self
                .inner
                .primary_profile_cleanup_workers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !workers.insert(observed_epoch) {
                return;
            }
        }
        let hub = self.clone();
        let worker_guard = PrimaryProfileCleanupWorkerGuard {
            inner: Arc::clone(&self.inner),
            epoch: observed_epoch,
        };
        tokio::spawn(async move {
            let result = AssertUnwindSafe(async {
                #[cfg(test)]
                if hub
                    .inner
                    .primary_profile_cleanup_panics_remaining
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    panic!("synthetic Primary profile cleanup worker panic");
                }
                loop {
                    if hub.inner.shutting_down.load(Ordering::Acquire)
                        || !hub
                            .inner
                            .primary_profile_cleanup_epochs
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .contains(&observed_epoch)
                    {
                        break;
                    }
                    let key = HostKey {
                        identity_mode: BrowserIdentityMode::Primary,
                        identity_generation: 0,
                        isolation_lane_id: None,
                    };
                    let active_slot = hub
                        .inner
                        .host_slots
                        .read()
                        .await
                        .get(&key)
                        .filter(|slot| slot.epoch == observed_epoch)
                        .cloned();
                    let retained_slot = if active_slot.is_some() {
                        active_slot.clone()
                    } else if let Some(slot) = hub
                        .inner
                        .orphaned_host_slots
                        .lock()
                        .await
                        .iter()
                        .find(|(pending_key, slot)| {
                            pending_key == &key && slot.epoch == observed_epoch
                        })
                        .map(|(_, slot)| Arc::clone(slot))
                    {
                        Some(slot)
                    } else {
                        hub.inner
                            .retiring_host_slots
                            .lock()
                            .await
                            .iter()
                            .find(|(pending_key, slot)| {
                                pending_key == &key && slot.epoch == observed_epoch
                            })
                            .map(|(_, slot)| Arc::clone(slot))
                    };
                    // Drain every operation which won admission before the
                    // sticky publication. No later Primary dispatch can take
                    // this read side because execute checks the fence first.
                    let admission_drain = match &retained_slot {
                        Some(slot) => {
                            Some(Arc::clone(&slot.admission_gate).write_owned().await)
                        }
                        None => None,
                    };

                    // Detach every logical Primary Lane before target cleanup.
                    // This releases task/global scheduler capacity and ensures
                    // a target-close timeout cannot restart a Host to preserve
                    // sibling Lanes: there are no live Primary siblings left.
                    let primary_lanes = hub.lanes_for_host_key(&key).await;
                    let mut lane_ids = Vec::with_capacity(primary_lanes.len());
                    for lane in primary_lanes {
                        lane_ids.push(lane.snapshot.read().await.lane_id.clone());
                    }
                    let mut detached = Vec::with_capacity(lane_ids.len());
                    for lane_id in lane_ids {
                        if let Some(record) = hub.detach_lane_for_close(&lane_id).await {
                            detached.push(record);
                        }
                    }
                    hub.promote_released_capacity().await;

                    if let (Some(active), Some(slot)) = (&active_slot, &retained_slot)
                        && Arc::ptr_eq(active, slot)
                    {
                        hub.retire_host_slot_for_cleanup(&key, observed_epoch, slot)
                            .await;
                    }

                    let mut first_error = None;
                    for record in detached {
                        if let Some(cleanup_id) = record.cleanup_id
                            && let Err(error) = hub
                                .attempt_pending_lane_cleanup(cleanup_id)
                                .await
                            && first_error.is_none()
                        {
                            first_error = Some(error);
                        }
                    }
                    hub.finalize_hosts_ready_after_cleanup().await;

                    if let Some(slot) = &retained_slot {
                        let cleanup = AssertUnwindSafe(
                            hub.attempt_orphaned_host_slot_cleanup(&key, slot),
                        )
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| {
                            Err(BrowserPlatformError::new(
                                BrowserErrorCode::BrowserUnavailable,
                                "Primary browser Host cleanup stopped unexpectedly.",
                                true,
                                "Wait for exact Primary Host cleanup to retry.",
                            ))
                        });
                        if let Err(error) = cleanup
                            && first_error.is_none()
                        {
                            first_error = Some(error);
                        }
                    }
                    drop(admission_drain);

                    if !hub
                        .managed_host_exists_for_key_epoch(&key, observed_epoch)
                        .await
                    {
                        hub.inner
                            .primary_profile_cleanup_epochs
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .remove(&observed_epoch);
                        break;
                    }
                    if let Some(error) = first_error {
                        tracing::warn!(
                            browser_epoch = observed_epoch,
                            code = ?error.code,
                            "Primary profile remains fenced while exact Host cleanup retries"
                        );
                    }
                    hub.ensure_cleanup_retry_worker();
                    tokio::time::sleep(PRIMARY_PROFILE_RETRY_INTERVAL).await;
                }
            })
            .catch_unwind()
            .await;
            if result.is_err() {
                tracing::error!(
                    browser_epoch = observed_epoch,
                    "Primary profile cleanup worker panicked; exact cleanup will be re-armed"
                );
            }
            drop(worker_guard);
        });
    }

    fn rearm_pending_primary_profile_cleanup(&self) {
        let pending = self
            .inner
            .primary_profile_cleanup_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for epoch in pending {
            self.spawn_primary_profile_cleanup(epoch);
        }
    }

    fn anonymous_profile_fence_epoch(&self, key: &HostKey) -> Option<u64> {
        self.inner
            .anonymous_profile_retirements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .copied()
    }

    fn spawn_anonymous_profile_rotation(&self, key: HostKey, observed_epoch: u64) {
        if key.identity_mode != BrowserIdentityMode::Anonymous {
            return;
        }
        {
            let mut workers = self
                .inner
                .anonymous_profile_rotation_workers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !workers.insert(key.clone()) {
                return;
            }
        }
        let hub = self.clone();
        let worker_guard = AnonymousProfileRotationWorkerGuard {
            inner: Arc::clone(&self.inner),
            key: key.clone(),
        };
        tokio::spawn(async move {
            let result = AssertUnwindSafe(async {
                #[cfg(test)]
                if hub
                    .inner
                    .anonymous_profile_rotation_panics_remaining
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_ok()
                {
                    panic!("synthetic Anonymous profile rotation worker panic");
                }
                loop {
                if hub.inner.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                let current_epoch = hub.anonymous_profile_fence_epoch(&key);
                if current_epoch != Some(observed_epoch) {
                    break;
                }
                let active_old_slot = hub
                    .inner
                    .host_slots
                    .read()
                    .await
                    .get(&key)
                    .filter(|slot| slot.epoch == observed_epoch)
                    .cloned();
                let old_slot = if active_old_slot.is_some() {
                    active_old_slot.clone()
                } else {
                    hub.inner
                        .orphaned_host_slots
                        .lock()
                        .await
                        .iter()
                        .find(|(pending_key, slot)| {
                            pending_key == &key && slot.epoch == observed_epoch
                        })
                        .map(|(_, slot)| Arc::clone(slot))
                };
                // Drain target creation and driver dispatch that won admission
                // before the sticky fence was published. The guard belongs to
                // the exact old slot; replacement Lane preparation uses the
                // new slot and cannot deadlock on it.
                let admission_drain = match &old_slot {
                    Some(slot) => Some(Arc::clone(&slot.admission_gate).write_owned().await),
                    None => None,
                };
                // Transfer the exact old slot out of the active map and into
                // the established orphan cleanup queue before shutdown. A
                // failed or cancelled shutdown therefore retains both the
                // physical authority and `retiring_host_keys`; no replacement
                // can be launched until exact cleanup succeeds.
                if let (Some(active), Some(slot)) = (&active_old_slot, &old_slot)
                    && Arc::ptr_eq(active, slot)
                {
                    hub.mark_host_restarting(&key, observed_epoch).await;
                    hub.retire_host_slot_for_cleanup(&key, observed_epoch, slot)
                        .await;
                }
                let cleanup = if let Some(slot) = &old_slot {
                    AssertUnwindSafe(hub.attempt_orphaned_host_slot_cleanup(&key, slot))
                        .catch_unwind()
                        .await
                        .unwrap_or_else(|_| {
                            Err(BrowserPlatformError::new(
                                BrowserErrorCode::BrowserUnavailable,
                                "Anonymous browser Host cleanup stopped unexpectedly.",
                                true,
                                "Wait for exact Host cleanup to retry.",
                            ))
                        })
                        .map(|_| ())
                } else if hub
                    .managed_host_exists_for_key_epoch(&key, observed_epoch)
                    .await
                {
                    Err(rebind_cleanup_pending_error(&key))
                } else {
                    Ok(())
                };
                drop(admission_drain);
                let attempt = match cleanup {
                    Err(error) => Ok(Err(error)),
                    Ok(()) => {
                        let has_live_lane = hub
                            .lanes_for_host_key(&key)
                            .await
                            .into_iter()
                            .any(|lane| !lane.closing.load(Ordering::Acquire));
                        if !has_live_lane {
                            Ok(Ok(HostRestartTransition {
                                old_epoch: observed_epoch,
                                new_epoch: observed_epoch,
                            }))
                        } else {
                            AssertUnwindSafe(hub.restart_host_for_resource_emergency(
                                key.clone(),
                                observed_epoch,
                                false,
                                "host_rotated_anonymous_profile_hygiene",
                            ))
                            .catch_unwind()
                            .await
                        }
                    }
                };
                match attempt {
                    Ok(Ok(_)) => {
                        let mut pending = hub
                            .inner
                            .anonymous_profile_retirements
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if pending.get(&key) == Some(&observed_epoch) {
                            pending.remove(&key);
                        }
                        break;
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(
                            browser_epoch = observed_epoch,
                            code = ?error.code,
                            "Anonymous browser profile rotation remains fenced for exact retry"
                        );
                    }
                    Err(_) => {
                        tracing::error!(
                            browser_epoch = observed_epoch,
                            "Anonymous browser profile rotation panicked; exact fence retained"
                        );
                    }
                }
                hub.ensure_cleanup_retry_worker();
                tokio::time::sleep(ANONYMOUS_PROFILE_RETRY_INTERVAL).await;
                }
            })
            .catch_unwind()
            .await;
            if result.is_err() {
                tracing::error!(
                    browser_epoch = observed_epoch,
                    "Anonymous browser profile rotation worker panicked; exact fence will be re-armed"
                );
            }
            drop(worker_guard);
        });
    }

    fn rearm_pending_anonymous_profile_rotations(&self) {
        let pending = self
            .inner
            .anonymous_profile_retirements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(key, epoch)| (key.clone(), *epoch))
            .collect::<Vec<_>>();
        for (key, epoch) in pending {
            self.spawn_anonymous_profile_rotation(key, epoch);
        }
    }

    /// Publishes the sticky fence without an await point. Callers that already
    /// hold the exact slot's admission read authority use this so cancellation
    /// immediately after threshold detection cannot discard retirement
    /// ownership before the Hub-owned worker exists.
    fn publish_admitted_anonymous_profile_fence(
        &self,
        key: HostKey,
        observed_epoch: u64,
        observed_slot: &Arc<HostSlot>,
        reason: &'static str,
    ) -> bool {
        if key.identity_mode != BrowserIdentityMode::Anonymous {
            return false;
        }
        observed_slot.retire();
        let inserted = {
            let mut pending = self
                .inner
                .anonymous_profile_retirements
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match pending.get(&key).copied() {
                Some(_) => false,
                None => {
                    pending.insert(key.clone(), observed_epoch);
                    true
                }
            }
        };
        if inserted {
            tracing::warn!(
                browser_epoch = observed_epoch,
                reason,
                "fenced the shared Anonymous Host after its profile reached a hygiene boundary"
            );
        }
        self.spawn_anonymous_profile_rotation(key, observed_epoch);
        inserted
    }

    async fn fence_anonymous_profile_host(
        &self,
        key: HostKey,
        observed_epoch: u64,
        observed_slot: &Arc<HostSlot>,
        reason: &'static str,
    ) -> bool {
        if key.identity_mode != BrowserIdentityMode::Anonymous {
            return false;
        }
        {
            let _open_guard = self.inner.open_gate.lock().await;
            let exact = self
                .inner
                .host_slots
                .read()
                .await
                .get(&key)
                .is_some_and(|current| {
                    current.epoch == observed_epoch && Arc::ptr_eq(current, observed_slot)
                });
            if !exact {
                return false;
            }
        }
        self.publish_admitted_anonymous_profile_fence(
            key,
            observed_epoch,
            observed_slot,
            reason,
        )
    }

    async fn anonymous_profile_limit_reason(
        &self,
        slot: &Arc<HostSlot>,
        policy: &AnonymousProfilePolicy,
        now_ms: u64,
        count_navigation: bool,
    ) -> Option<&'static str> {
        let navigation_count = if count_navigation {
            slot.profile_navigation_count
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1)
        } else {
            slot.profile_navigation_count.load(Ordering::Acquire)
        };
        if navigation_count > policy.max_navigations {
            return Some("navigation_limit");
        }
        if now_ms.saturating_sub(slot.created_at_ms) >= policy.max_age_ms {
            return Some("age_limit");
        }
        let navigation_count = slot.profile_navigation_count.load(Ordering::Acquire);
        let Some((flight, start)) =
            slot.claim_profile_sample_if_due(
                policy.sample_interval_ms,
                policy.sample_navigation_interval,
                now_ms,
                navigation_count,
                false,
            )
        else {
            return None;
        };
        if start {
            let Some(driver) = slot.get().cloned() else {
                flight.complete(Ok(None));
                let result = flight.wait().await;
                slot.consume_profile_sample(&flight);
                return match result {
                    Ok(_) => Some("footprint_measurement_unavailable"),
                    Err(_) => Some("footprint_measurement_failed"),
                };
            };
            let sample_slot = Arc::clone(slot);
            let worker_flight = Arc::clone(&flight);
            let worker_clock = Arc::clone(&self.inner.clock);
            let max_bytes = policy.max_bytes;
            let max_entries = policy.max_entries;
            tokio::spawn(async move {
                let result = AssertUnwindSafe(async move {
                    let result = driver.profile_footprint(max_bytes, max_entries).await;
                    if result.is_ok() {
                        // Publish the sample watermark before waking waiters.
                        // The Hub-owned worker does this even when every
                        // request which made the sample due was cancelled.
                        sample_slot
                            .profile_sample_completed
                            .store(true, Ordering::Release);
                        sample_slot
                            .last_profile_sample_ms
                            .store(worker_clock.now_ms(), Ordering::Release);
                        sample_slot
                            .last_profile_sample_navigation
                            .store(
                                sample_slot
                                    .profile_navigation_count
                                    .load(Ordering::Acquire),
                                Ordering::Release,
                            );
                    }
                    result
                })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(BrowserPlatformError::new(
                            BrowserErrorCode::BrowserUnavailable,
                            "The Anonymous profile footprint worker stopped unexpectedly.",
                            true,
                            "Retry after the exact Anonymous browser Host is retired.",
                        ))
                    });
                worker_flight.complete(result);
                // Do not clear the exact flight here. Its completed result is
                // a bounded one-item mailbox which only an observing waiter
                // consumes, preventing cancellation from losing a boundary.
            });
        }
        let result = flight.wait().await;
        // No await separates result observation from removing the exact
        // mailbox and evaluating it below. An aborted waiter leaves the
        // completed flight installed for its successor.
        slot.consume_profile_sample(&flight);
        let footprint = match result {
            Ok(footprint) => footprint,
            Err(error) => {
                tracing::warn!(
                    browser_epoch = slot.epoch,
                    code = ?error.code,
                    "Anonymous profile footprint measurement failed closed"
                );
                return Some("footprint_measurement_failed");
            }
        };
        match footprint {
            Some(footprint) => (footprint.limit_reached
                || footprint.bytes >= policy.max_bytes
                || footprint.entries >= policy.max_entries)
                .then_some("footprint_limit"),
            None => Some("footprint_measurement_unavailable"),
        }
    }

    async fn primary_profile_limit_reason(
        &self,
        key: &HostKey,
        slot: &Arc<HostSlot>,
        policy: &PrimaryProfilePolicy,
        now_ms: u64,
        count_navigation: bool,
    ) -> Option<&'static str> {
        if key.identity_mode != BrowserIdentityMode::Primary {
            return None;
        }
        let navigation_count = if count_navigation {
            slot.profile_navigation_count
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1)
        } else {
            slot.profile_navigation_count.load(Ordering::Acquire)
        };
        let Some((flight, start)) = slot.claim_profile_sample_if_due(
            policy.sample_interval_ms,
            policy.sample_navigation_interval,
            now_ms,
            navigation_count,
            true,
        ) else {
            return None;
        };
        if start {
            let Some(driver) = slot.get().cloned() else {
                self.publish_admitted_primary_profile_fence(
                    slot.epoch,
                    slot,
                    "footprint_measurement_unavailable",
                );
                flight.complete(Ok(None));
                let result = flight.wait().await;
                slot.consume_profile_sample(&flight);
                return match result {
                    Ok(_) => Some("footprint_measurement_unavailable"),
                    Err(_) => Some("footprint_measurement_failed"),
                };
            };
            let worker_hub = self.clone();
            let worker_key = key.clone();
            let worker_slot = Arc::clone(slot);
            let sample_slot = Arc::clone(slot);
            let worker_flight = Arc::clone(&flight);
            let worker_clock = Arc::clone(&self.inner.clock);
            let max_bytes = policy.max_bytes;
            let max_entries = policy.max_entries;
            tokio::spawn(async move {
                let result = AssertUnwindSafe(async move {
                    let result = driver.profile_footprint(max_bytes, max_entries).await;
                    if result.is_ok() {
                        sample_slot
                            .profile_sample_completed
                            .store(true, Ordering::Release);
                        sample_slot
                            .last_profile_sample_ms
                            .store(worker_clock.now_ms(), Ordering::Release);
                        sample_slot
                            .last_profile_sample_navigation
                            .store(
                                sample_slot
                                    .profile_navigation_count
                                    .load(Ordering::Acquire),
                                Ordering::Release,
                            );
                    }
                    result
                })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(BrowserPlatformError::new(
                        BrowserErrorCode::BrowserUnavailable,
                        "The Primary profile footprint worker stopped unexpectedly.",
                        false,
                        "Clean the managed Primary site data or sign in again, then restart the application.",
                    ))
                });
                let reason = match &result {
                    Ok(Some(footprint))
                        if footprint.limit_reached
                            || footprint.bytes >= max_bytes
                            || footprint.entries >= max_entries =>
                    {
                        Some("footprint_limit")
                    }
                    Ok(Some(_)) => None,
                    Ok(None) => Some("footprint_measurement_unavailable"),
                    Err(_) => Some("footprint_measurement_failed"),
                };
                if worker_key.identity_mode == BrowserIdentityMode::Primary
                    && let Some(reason) = reason
                {
                    // The Hub-owned sampler publishes before its mailbox. A
                    // cancelled request therefore cannot lose the sticky
                    // fence after the native walk has detected a boundary.
                    worker_hub.publish_admitted_primary_profile_fence(
                        worker_slot.epoch,
                        &worker_slot,
                        reason,
                    );
                }
                worker_flight.complete(result);
            });
        }
        let result = flight.wait().await;
        slot.consume_profile_sample(&flight);
        match result {
            Ok(Some(footprint)) => (footprint.limit_reached
                || footprint.bytes >= policy.max_bytes
                || footprint.entries >= policy.max_entries)
                .then_some("footprint_limit"),
            Ok(None) => Some("footprint_measurement_unavailable"),
            Err(error) => {
                tracing::warn!(
                    browser_epoch = slot.epoch,
                    code = ?error.code,
                    "Primary profile footprint measurement failed closed"
                );
                Some("footprint_measurement_failed")
            }
        }
    }

    async fn sweep_anonymous_profile_hygiene(&self) {
        let pending = self
            .inner
            .anonymous_profile_retirements
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(key, epoch)| (key.clone(), *epoch))
            .collect::<Vec<_>>();
        for (key, epoch) in pending {
            self.spawn_anonymous_profile_rotation(key, epoch);
        }
        let policy = self
            .inner
            .config
            .read()
            .await
            .anonymous_profile_policy
            .clone();
        let now_ms = self.inner.clock.now_ms();
        let slots = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .filter(|(key, _)| key.identity_mode == BrowserIdentityMode::Anonymous)
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect::<Vec<_>>();
        for (key, slot) in slots {
            if self.anonymous_profile_fence_epoch(&key).is_some() {
                continue;
            }
            if let Some(reason) = self
                .anonymous_profile_limit_reason(&slot, &policy, now_ms, false)
                .await
            {
                self.fence_anonymous_profile_host(key, slot.epoch, &slot, reason)
                    .await;
            }
        }
    }

    async fn sweep_primary_profile_hygiene(&self) {
        self.rearm_pending_primary_profile_cleanup();
        if self.primary_profile_fence().is_some() {
            return;
        }
        let policy = self
            .inner
            .config
            .read()
            .await
            .primary_profile_policy
            .clone();
        let now_ms = self.inner.clock.now_ms();
        let slots = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .filter(|(key, slot)| {
                key.identity_mode == BrowserIdentityMode::Primary
                    && slot.get().is_some()
                    && !slot.retired.load(Ordering::Acquire)
            })
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect::<Vec<_>>();
        for (key, slot) in slots {
            let admission = Arc::clone(&slot.admission_gate).read_owned().await;
            let is_exact = self
                .inner
                .host_slots
                .read()
                .await
                .get(&key)
                .is_some_and(|current| {
                    current.epoch == slot.epoch && Arc::ptr_eq(current, &slot)
                });
            if !is_exact
                || slot.retired.load(Ordering::Acquire)
                || self.primary_profile_fence().is_some()
            {
                drop(admission);
                continue;
            }
            if let Some(reason) = self
                .primary_profile_limit_reason(&key, &slot, &policy, now_ms, false)
                .await
            {
                self.publish_admitted_primary_profile_fence(
                    slot.epoch,
                    &slot,
                    reason,
                );
            }
            drop(admission);
        }
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
                snapshot.lifecycle_state = LaneLifecycleState::Failed;
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
        let authority_key = HostCleanupAuthorityKey {
            host_key: key.clone(),
            browser_epoch: observed_epoch,
        };
        {
            let mut published = self
                .inner
                .published_restart_slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if published
                .get(&authority_key)
                .is_some_and(|authority| Arc::ptr_eq(&authority.slot, observed_slot))
            {
                published.remove(&authority_key);
            }
        }
        self.inner.host_empty_since_ms.write().await.remove(key);
        self.ensure_cleanup_retry_worker();
        true
    }

    async fn process_abandoned_restart_slots(&self) {
        let abandoned = self
            .inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|(_, authority)| authority.abandoned.load(Ordering::Acquire))
            .map(|(key, authority)| (key.clone(), Arc::clone(authority)))
            .collect::<Vec<_>>();
        for (authority_key, authority) in abandoned {
            let key = &authority_key.host_key;
            let slot = &authority.slot;
            let still_abandoned = self
                .inner
                .published_restart_slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&authority_key)
                .is_some_and(|current| {
                    Arc::ptr_eq(current, &authority)
                        && current.abandoned.load(Ordering::Acquire)
                });
            if !still_abandoned {
                continue;
            }

            // Unlike ordinary retirement, this path already owns the exact
            // slot by value and must remain valid if another active epoch has
            // replaced it. Remove only a pointer-identical active entry, then
            // durably queue this exact slot under the HostKey fence.
            let _open_guard = self.inner.open_gate.lock().await;
            let mut retiring_keys = self.inner.retiring_host_keys.write().await;
            let mut slots = self.inner.host_slots.write().await;
            if slots.get(key).is_some_and(|current| {
                current.epoch == authority_key.browser_epoch
                    && Arc::ptr_eq(current, slot)
            }) {
                slots.remove(key);
            }
            slot.retire();
            retiring_keys.insert(key.clone());
            let mut orphaned = self.inner.orphaned_host_slots.lock().await;
            if !orphaned.iter().any(|(pending_key, pending_slot)| {
                pending_key == key && Arc::ptr_eq(pending_slot, slot)
            }) {
                orphaned.push((key.clone(), Arc::clone(slot)));
            }
            drop(orphaned);
            drop(slots);
            drop(retiring_keys);
            drop(_open_guard);

            // Remove provisional authority only after the durable orphan
            // queue owns the exact Arc. Pointer comparison prevents a late
            // cleanup pass from deleting a newer publication record.
            {
                let mut published = self
                    .inner
                    .published_restart_slots
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if published
                    .get(&authority_key)
                    .is_some_and(|current| Arc::ptr_eq(current, &authority))
                {
                    published.remove(&authority_key);
                }
            }
            self.inner.host_empty_since_ms.write().await.remove(key);
        }
        if !self.inner.orphaned_host_slots.lock().await.is_empty() {
            self.ensure_cleanup_retry_worker();
        }
    }

    fn recover_host_failure(
        &self,
        key: HostKey,
        observed_epoch: u64,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<HostRestartTransition, BrowserPlatformError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
        let hub = self.clone();
        let restart_key = key.clone();
        let terminal_hub = self.clone();
        let terminal_key = key.clone();
        // LOCK ORDER: primary_visibility_gate is always acquired BEFORE
        // joining or leading a per-key restart flight. The visibility paths
        // (set_primary_visibility_once / set_lane_visibility_and_maybe_focus
        // -> transition_primary_visibility_locked) hold the gate and then
        // enter host_restarts.run_bounded; acquiring the gate inside the
        // flight's leader closure instead would deadlock those callers
        // against crash recovery until the attempt timeout aborts the leader.
        let _primary_visibility_guard = if key.identity_mode == BrowserIdentityMode::Primary {
            Some(self.inner.primary_visibility_gate.lock().await)
        } else {
            None
        };
        let flight = self
            .inner
            .host_restarts
            .run_bounded_with_terminal_callback(
                key.clone(),
                observed_epoch,
                HOST_RESTART_ATTEMPT_TIMEOUT,
                move || async move {
                    hub.mark_host_restarting(&restart_key, observed_epoch)
                        .await;
                    hub.restart_host_once(restart_key, observed_epoch).await
                },
                move |result| async move {
                    if let Err(error) = result {
                        terminal_hub
                            .mark_host_recovery_failed(
                                &terminal_key,
                                observed_epoch,
                                &error,
                            )
                            .await;
                    }
                },
            )
            .await;
        flight.result
        })
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
        if key.identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            return Err(error);
        }
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
                if let Err(error) = self.rebind_host_lanes(
                    &key,
                    observed_epoch,
                    transition,
                    host,
                    requested_headful.is_some(),
                )
                .await
                {
                    self.retire_failed_rebind_host(&key, &current).await;
                    return Err(error);
                }
                return Ok(transition);
            }
        }

        // A failed replacement shutdown retains the exact Host in the
        // orphan queue and publishes this reopen fence. Starting yet another
        // replacement while that process is still authoritative would make
        // physical Host count grow with every recovery request.
        if self.inner.retiring_host_keys.read().await.contains(&key) {
            self.ensure_cleanup_retry_worker();
            return Err(rebind_cleanup_pending_error(&key));
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
        let mut recovery_resource_scope = None;
        for lane in self.lanes_for_host_key(&key).await {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = lane.snapshot.read().await;
            // A Lane whose Host target never finished opening still carries
            // browser_epoch=0. It needs exact old-Host stop proof, but it must
            // not cause a replacement Host to be launched and rebound just
            // before the failed start is discarded. Only Lanes actually
            // attached to the observed epoch are recovery beneficiaries.
            if snapshot.browser_epoch != observed_epoch {
                continue;
            }
            has_live_lane = true;
            recovery_resource_scope = Some((
                snapshot.caller.runtime_cleanup_key().into_string(),
                snapshot.caller.task_resource_family_key().into_string(),
            ));
            break;
        }
        if !has_live_lane {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser Host changed during recovery.",
                true,
                "Refresh browser status and retry.",
            ));
        }
        let (recovery_task_id, recovery_family_id) = recovery_resource_scope
            .unwrap_or_else(|| {
                (
                    "recovery-without-task".to_owned(),
                    "recovery-without-family".to_owned(),
                )
            });

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
            // A restart keeps the stopped slot in the active map until the
            // replacement is atomically installed, so the generic
            // "no-managed-host" check cannot infer this exact epoch is gone.
            // Successful shutdown is itself authoritative proof: clear only
            // the stopped epoch before publishing its successor.
            self.clear_completed_cleanup_authority_for_stopped_host(
                &key,
                observed_epoch,
            )
            .await;
        }

        let new_epoch = self.inner.host_epoch_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let new_slot = Arc::new(HostSlot::new(
            new_epoch,
            requested_headful.unwrap_or(false),
            self.inner.clock.now_ms(),
        ));
        let new_authority_key = HostCleanupAuthorityKey {
            host_key: key.clone(),
            browser_epoch: new_epoch,
        };
        let published_authority = Arc::new(PublishedRestartAuthority {
            slot: Arc::clone(&new_slot),
            abandoned: AtomicBool::new(false),
        });
        {
            let mut slots = self.inner.host_slots.write().await;
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::shutting_down());
            }
            if self.inner.draining.load(Ordering::Acquire) {
                return Err(platform_drain_in_progress_error());
            }
            if key.identity_mode == BrowserIdentityMode::Primary
                && let Some(error) = self.primary_profile_fence_error()
            {
                return Err(error);
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
            self.reserve_cleanup_host(
                &recovery_task_id,
                &recovery_family_id,
                &key,
                new_epoch,
            )?;
            self.inner
                .published_restart_slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    new_authority_key.clone(),
                    Arc::clone(&published_authority),
                );
            slots.insert(key.clone(), Arc::clone(&new_slot));
        }
        // No await is allowed between active-map publication and guard
        // construction. From the next cancellation point onward, Drop marks
        // this exact epoch abandoned and hands it to independent cleanup.
        let published_restart_guard = PublishedRestartGuard {
            inner: Arc::clone(&self.inner),
            authority_key: new_authority_key,
            authority: published_authority,
            armed: true,
        };

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
        if let Err(error) = self.rebind_host_lanes(
            &key,
            observed_epoch,
            transition,
            host,
            requested_headful.is_some(),
        )
        .await
        {
            self.retire_failed_rebind_host(&key, &new_slot).await;
            return Err(error);
        }
        // `rebind_host_lanes` returns only after every surviving Lane driver
        // and snapshot route has published the replacement epoch. The new Host
        // is now ordinarily owned by `host_slots`, so the provisional exact
        // retirement authority can be removed.
        published_restart_guard.disarm();
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
        let residual_prepared_lane = self
            .inner
            .prepared_rebind_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|(_, authority)| {
                authority.host_key == *key
                    && authority.browser_epoch == transition.new_epoch
            })
            .map(|(lane_id, _)| lane_id.clone());
        if let Some(lane_id) = residual_prepared_lane {
            // BrowserHostDriver does not promise open_lane idempotence. A
            // cancelled prior attempt must retire this exact Host before a
            // later attempt may create the same logical target again.
            return Err(rebind_lane_cleanup_pending_error(
                lane_id,
                transition.new_epoch,
            ));
        }
        let pending_lane_ids = self
            .inner
            .pending_lane_cleanups
            .lock()
            .await
            .iter()
            .map(|entry| entry.lane_id.clone())
            .collect::<HashSet<_>>();
        if !pending_lane_ids.is_empty() {
            for lane in self.lanes_for_host_key(key).await {
                let snapshot = lane.snapshot.read().await;
                if snapshot.browser_epoch == observed_epoch
                    && pending_lane_ids.contains(&snapshot.lane_id)
                {
                    return Err(rebind_lane_cleanup_pending_error(
                        snapshot.lane_id.clone(),
                        observed_epoch,
                    ));
                }
            }
        }
        let policy = self.inner.config.read().await.resource_policy.clone();
        self.close_policy_excess_lanes(&policy, Some(key)).await?;
        let max_task_tabs = policy.max_task_tabs;
        let mut prepared = Vec::new();
        for lane in self.lanes_for_host_key(key).await {
            // Establish replacement authority while serialized with explicit
            // Lane detach. This closes the live-read -> target-open window.
            let close_guard = lane.close_gate.lock().await;
            let lane_is_current = {
                let snapshot = lane.snapshot.read().await;
                self.inner
                    .lanes
                    .read()
                    .await
                    .get(&snapshot.lane_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &lane))
            };
            if lane.closing.load(Ordering::Acquire) || !lane_is_current {
                drop(close_guard);
                continue;
            }
            let (
                lane_id,
                identity_mode,
                workspace_hint,
                epoch,
                task_resource_family_key,
            ) = {
                let snapshot = lane.snapshot.read().await;
                (
                    snapshot.lane_id.clone(),
                    snapshot.identity_mode,
                    lane.workspace_hint.clone(),
                    snapshot.browser_epoch,
                    snapshot.caller.task_resource_family_key().into_string(),
                )
            };
            if epoch != observed_epoch {
                drop(close_guard);
                continue;
            }
            let authority_gate = self.inner.prepared_rebind_authority_gate.lock().await;
            let exact_host_is_active = self
                .inner
                .host_slots
                .read()
                .await
                .get(key)
                .is_some_and(|slot| {
                    slot.epoch == transition.new_epoch
                        && !slot.retired.load(Ordering::Acquire)
                });
            if !exact_host_is_active {
                drop(authority_gate);
                drop(close_guard);
                return Err(rebind_cleanup_pending_error(key));
            }
            let prepared_authority = self.mark_prepared_rebind_authority(
                lane_id.clone(),
                key,
                transition.new_epoch,
            );
            drop(authority_gate);
            drop(close_guard);
            let driver = match host
                .open_lane(LaneLaunchRequest {
                    lane_id: lane_id.clone(),
                    identity_mode,
                    workspace_hint,
                    task_resource_key: task_resource_family_key,
                    max_task_tabs,
                    task_tab_authority: Arc::clone(&self.inner.task_tab_authority)
                        as Arc<dyn BrowserTaskTabAuthority>,
                    task_download_authority: Arc::clone(&self.inner.task_download_authority)
                        as Arc<dyn BrowserTaskDownloadAuthority>,
                })
                .await
            {
                Ok(driver) => driver,
                Err(error) => {
                    // Armed Drop makes exact Host shutdown the proof for any
                    // target created before the adapter returned its error.
                    drop(prepared_authority);
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
            prepared.push((lane, driver, prepared_authority));
        }

        // Route publication is the other side of policy reconciliation's
        // fence. A recovery that began with cap 4 may finish after a 4->8
        // update. Wait for the update, then hold `open_gate` across a final
        // Host-local reconcile and Lane epoch publication; a new policy
        // update cannot begin in the gap between those two steps.
        let publish_guard = loop {
            let reconciled = self.inner.policy_reconciled.notified();
            let guard = self.inner.open_gate.lock().await;
            if !self.inner.policy_reconciling.load(Ordering::Acquire) {
                break guard;
            }
            drop(guard);
            reconciled.await;
        };
        let committed_tab_cap = self.inner.config.read().await.resource_policy.max_task_tabs;
        let mut prepared_tasks = BTreeSet::new();
        for (lane, _, _) in &prepared {
            prepared_tasks.insert(
                lane.snapshot
                    .read()
                    .await
                    .caller
                    .task_resource_family_key()
                    .into_string(),
            );
        }
        // The Hub authority already reserved every target opened during this
        // rebind against `max_task_tabs`. A Host-local reconcile is required
        // only if policy narrowed while the replacement was in flight. Making
        // an unchanged/raised cap depend on this optional defense-in-depth
        // hook breaks otherwise-safe first-Lane visibility transitions.
        if committed_tab_cap < max_task_tabs {
            for task_id in prepared_tasks {
                if let Err(error) = host
                    .reconcile_task_tab_limit(&task_id, committed_tab_cap)
                    .await
                {
                    drop(publish_guard);
                    let cleanup_error = self
                        .cleanup_prepared_rebind_drivers(
                            std::mem::take(&mut prepared),
                            transition.new_epoch,
                        )
                        .await;
                    if let Some(cleanup_error) = cleanup_error {
                        tracing::warn!(
                            code = ?cleanup_error.code,
                            "browser Host route reconciliation failed and prepared Lane cleanup remains pending"
                        );
                    }
                    return Err(policy_tab_driver_reconciliation_error(error, &task_id));
                }
            }
        }

        let mut late_cleanup_ids = Vec::new();
        for (lane, driver, prepared_authority) in prepared {
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
            let authority_gate = self.inner.prepared_rebind_authority_gate.lock().await;
            if !self.has_prepared_rebind_authority(
                &lane_id,
                &host_key,
                transition.new_epoch,
            ) {
                drop(authority_gate);
                drop(close_guard);
                // Exact Host-stop proof won this race. Publishing the stale
                // driver now would create authority after its proof passed.
                return Err(rebind_cleanup_pending_error(key));
            }
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
                let cleanup_id = self
                    .retain_pending_lane_cleanup(
                        lane_id,
                        lane.snapshot.read().await.caller.user_id.clone(),
                        lane.snapshot.read().await.caller.task_resource_key(),
                        lane.snapshot
                            .read()
                            .await
                            .caller
                            .task_resource_family_key()
                            .into_string(),
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
                // Publish pending driver + owner target first. Marker removal
                // is serialized with stopped-Host proof by authority_gate.
                prepared_authority.complete();
                drop(authority_gate);
                drop(close_guard);
                late_cleanup_ids.push(cleanup_id);
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
            prepared_authority.complete();
            drop(authority_gate);
            drop(close_guard);
            self.emit("lane_rebound_after_host_restart", Some(&snapshot));
        }
        drop(publish_guard);
        for cleanup_id in late_cleanup_ids {
            let _ = self.attempt_pending_lane_cleanup(cleanup_id).await;
        }
        Ok(())
    }

    async fn retire_failed_rebind_host(
        &self,
        key: &HostKey,
        slot: &Arc<HostSlot>,
    ) {
        if !self
            .retire_host_slot_for_cleanup(key, slot.epoch, slot)
            .await
        {
            return;
        }
        if let Err(cleanup_error) = self
            .attempt_orphaned_host_slot_cleanup(key, slot)
            .await
        {
            tracing::warn!(
                identity_mode = ?key.identity_mode,
                browser_epoch = slot.epoch,
                code = ?cleanup_error.code,
                "failed browser Host rebind left its exact replacement retirement pending"
            );
        }
    }

    async fn cleanup_prepared_rebind_drivers(
        &self,
        prepared: Vec<(
            Arc<LaneRecord>,
            Arc<dyn BrowserLaneDriver>,
            PreparedRebindAuthorityGuard,
        )>,
        browser_epoch: u64,
    ) -> Option<BrowserPlatformError> {
        let mut cleanup_ids = Vec::with_capacity(prepared.len());
        for (lane, driver, prepared_authority) in prepared {
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
            let authority_gate = self.inner.prepared_rebind_authority_gate.lock().await;
            if !self.has_prepared_rebind_authority(
                &lane_id,
                &host_key,
                browser_epoch,
            ) {
                drop(authority_gate);
                // Exact Host-stop proof already covered this target.
                continue;
            }
            let cleanup_id = self
                .retain_pending_lane_cleanup(
                    lane_id,
                    lane.snapshot.read().await.caller.user_id.clone(),
                    lane.snapshot.read().await.caller.task_resource_key(),
                    lane.snapshot
                        .read()
                        .await
                        .caller
                        .task_resource_family_key()
                        .into_string(),
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
            prepared_authority.complete();
            drop(authority_gate);
            cleanup_ids.push(cleanup_id);
        }
        let deadline = Instant::now() + CLEANUP_BATCH_WAIT_TIMEOUT;
        let mut attempts = tokio::task::JoinSet::new();
        let mut cleanup_ids = cleanup_ids.into_iter();
        let mut first_error = None;
        loop {
            while attempts.len() < MAX_CONCURRENT_LANE_CLEANUPS {
                let Some(cleanup_id) = cleanup_ids.next() else {
                    break;
                };
                let hub = self.clone();
                attempts.spawn(async move {
                    hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                        .await
                });
            }
            let Some(attempt) = attempts.join_next().await else {
                break;
            };
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
        let task_id = caller.task_resource_family_key().into_string();
        // Count an operation before it is allowed to wait on the per-Lane
        // serialization gate. Without this admission, arbitrarily many calls
        // to one Lane remain invisible to both task and global operation
        // limits while retaining their full request payloads.
        let _operation_admission =
            self.try_acquire_operation_admission(lane_id, &task_id)?;
        let wait_deadline = Instant::now() + OPERATION_QUEUE_WAIT_TIMEOUT;

        // Correctness is serialized per Lane.  Other Lane gates remain free,
        // and the global semaphore is only a resource bound.
        let _lane_guard = tokio::select! {
            guard = lane.operation_gate.lock() => guard,
            _ = lane.cancellation.cancelled() => return Err(lane.closed_error(lane_id.clone())),
            _ = tokio::time::sleep_until(wait_deadline) => {
                return Err(operation_queue_wait_timeout_error(Some(lane_id.clone())));
            }
        };
        // Tokio select is intentionally unbiased: gate release and close
        // cancellation may become ready in the same poll. Recheck the
        // terminal fence after winning the gate so no queued waiter can enter
        // a Stopping Lane and surface a misleading BrowserUnavailable error.
        if lane.closing.load(Ordering::Acquire) || lane.cancellation.is_cancelled() {
            return Err(lane_closed_error(lane_id.clone()));
        }
        self.execute_lane_driver(
            &lane,
            lane_id,
            operation,
            trusted_out_of_band_confirmation,
            wait_deadline,
        )
        .await
    }

    async fn execute_lane_driver(
        &self,
        lane: &LaneRecord,
        lane_id: &BrowserLaneId,
        operation: BrowserOperation,
        trusted_out_of_band_confirmation: bool,
        wait_deadline: Instant,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let should_refresh_identity = identity_operation_needs_refresh(&operation);
        let is_fresh_observe = operation.kind == BrowserOperationKind::Observe;
        let pending_recovery = {
            let snapshot = lane.snapshot.read().await;
            (lane.fresh_observe_required.load(Ordering::Acquire)
                && matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Starting | LaneLifecycleState::Failed
                ))
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
        let (context, identity_mode, host_key, task_id) = {
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
                snapshot.caller.task_resource_family_key().into_string(),
            )
        };
        let dispatch_epoch = context.browser_epoch;
        if let Err(error) = require_lane_operation(identity_mode, &operation) {
            return Err(error.for_lane(lane_id.clone()));
        }
        if identity_mode == BrowserIdentityMode::Primary
            && let Some(error) = self.primary_profile_fence_error()
        {
            return Err(error.for_lane(lane_id.clone()));
        }
        let permit = self
            .acquire_driver_permit(
                &task_id,
                &operation,
                &lane.cancellation,
                wait_deadline,
            )
            .await
            .map_err(|error| {
                if lane.cancellation.is_cancelled() {
                    lane_closed_error(lane_id.clone())
                } else {
                    error.for_lane(lane_id.clone())
                }
            })?;
        let driver = lane.driver.read().await.clone().ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser lane driver is unavailable.",
                true,
                "Retry after the lane recovers.",
            )
            .for_lane(lane_id.clone())
        })?;
        // The resource permit is intentionally acquired before this Host gate:
        // queued work must not pin the old Host and delay a hygiene rotation.
        // Once the permit is granted, however, the exact-slot read authority
        // stays held through driver dispatch and result publication. A profile
        // fence takes the write side, drains operations already admitted here,
        // and prevents every later dispatch from reaching the retired process.
        let host_admission_guard = if matches!(
            identity_mode,
            BrowserIdentityMode::Anonymous | BrowserIdentityMode::Primary
        ) {
            let observed_slot = self
                .inner
                .host_slots
                .read()
                .await
                .get(&host_key)
                .filter(|slot| slot.epoch == dispatch_epoch)
                .cloned()
                .ok_or_else(|| {
                    let error = if identity_mode == BrowserIdentityMode::Primary {
                        self.primary_profile_fence_error().unwrap_or_else(|| {
                            BrowserPlatformError::new(
                                BrowserErrorCode::BrowserUnavailable,
                                "The Primary browser Host changed before profile admission.",
                                true,
                                "Refresh browser status and retry.",
                            )
                        })
                    } else {
                        anonymous_profile_hygiene_error(
                            &host_key,
                            "host_epoch_changed",
                        )
                    };
                    error.for_lane(lane_id.clone())
                })?;
            let admission = Arc::clone(&observed_slot.admission_gate)
                .read_owned()
                .await;
            let is_exact = self
                .inner
                .host_slots
                .read()
                .await
                .get(&host_key)
                .is_some_and(|current| {
                    current.epoch == dispatch_epoch && Arc::ptr_eq(current, &observed_slot)
                });
            if !is_exact
                || observed_slot.retired.load(Ordering::Acquire)
                || (identity_mode == BrowserIdentityMode::Anonymous
                    && self.anonymous_profile_fence_epoch(&host_key).is_some())
                || (identity_mode == BrowserIdentityMode::Primary
                    && self.primary_profile_fence().is_some())
            {
                drop(admission);
                let error = if identity_mode == BrowserIdentityMode::Primary {
                    self.primary_profile_fence_error().unwrap_or_else(|| {
                        BrowserPlatformError::new(
                            BrowserErrorCode::BrowserUnavailable,
                            "The Primary browser Host is no longer available.",
                            true,
                            "Refresh browser status and retry.",
                        )
                    })
                } else {
                    anonymous_profile_hygiene_error(
                        &host_key,
                        "host_admission_fenced",
                    )
                };
                return Err(error.for_lane(lane_id.clone()));
            }
            let is_navigation = operation.action == "navigate"
                && matches!(
                    operation.kind,
                    BrowserOperationKind::Navigate | BrowserOperationKind::Crawl
                );
            let reason = if identity_mode == BrowserIdentityMode::Primary {
                let policy = self
                    .inner
                    .config
                    .read()
                    .await
                    .primary_profile_policy
                    .clone();
                self.primary_profile_limit_reason(
                    &host_key,
                    &observed_slot,
                    &policy,
                    self.inner.clock.now_ms(),
                    is_navigation,
                )
                .await
            } else {
                let policy = self
                    .inner
                    .config
                    .read()
                    .await
                    .anonymous_profile_policy
                    .clone();
                self.anonymous_profile_limit_reason(
                    &observed_slot,
                    &policy,
                    self.inner.clock.now_ms(),
                    is_navigation,
                )
                .await
            };
            if let Some(reason) = reason {
                // This publication contains no await point. The exact Host
                // read authority is still held, so the first N admitted
                // navigations may finish while attempt N+1 atomically fences
                // every later dispatch and hands rotation to Hub ownership.
                if identity_mode == BrowserIdentityMode::Primary {
                    self.publish_admitted_primary_profile_fence(
                        dispatch_epoch,
                        &observed_slot,
                        reason,
                    );
                } else {
                    self.publish_admitted_anonymous_profile_fence(
                        host_key.clone(),
                        dispatch_epoch,
                        &observed_slot,
                        reason,
                    );
                }
                drop(admission);
                let error = if identity_mode == BrowserIdentityMode::Primary {
                    self.primary_profile_fence_error().unwrap_or_else(|| {
                        primary_profile_storage_limit_error(PrimaryProfileFence {
                            trigger_epoch: dispatch_epoch,
                            reason,
                        })
                    })
                } else {
                    anonymous_profile_hygiene_error(&host_key, reason)
                };
                return Err(error.for_lane(lane_id.clone()));
            }
            // A sibling admission may have detected a boundary while this
            // call was waiting on the single profile sampler. Such a call had
            // a read permit already, but had not yet reached the browser and
            // therefore must observe the now-published fence before dispatch.
            if observed_slot.retired.load(Ordering::Acquire)
                || (identity_mode == BrowserIdentityMode::Anonymous
                    && self.anonymous_profile_fence_epoch(&host_key).is_some())
                || (identity_mode == BrowserIdentityMode::Primary
                    && self.primary_profile_fence().is_some())
            {
                drop(admission);
                let error = if identity_mode == BrowserIdentityMode::Primary {
                    self.primary_profile_fence_error().unwrap_or_else(|| {
                        BrowserPlatformError::new(
                            BrowserErrorCode::BrowserUnavailable,
                            "The Primary browser Host is no longer available.",
                            true,
                            "Refresh browser status and retry.",
                        )
                    })
                } else {
                    anonymous_profile_hygiene_error(
                        &host_key,
                        "host_admission_fenced",
                    )
                };
                return Err(error.for_lane(lane_id.clone()));
            }
            Some(admission)
        } else {
            None
        };
        // All actual driver work participates in this read side so pressure
        // lifecycle work can prove a Lane is idle before freezing or closing
        // it. Host admission is acquired first to match hygiene rotation's
        // Host-before-Lane lock order.
        let activity_guard = lane.activity_gate.read().await;
        {
            let snapshot = lane.snapshot.read().await;
            if snapshot.browser_epoch != dispatch_epoch
                || snapshot.lifecycle_state != LaneLifecycleState::Running
            {
                return Err(lane_restart_notice(lane, &snapshot).for_lane(lane_id.clone()));
            }
        }
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
            _ = lane.cancellation.cancelled() => Err(lane.closed_error(lane_id.clone())),
        };
        if result
            .as_ref()
            .err()
            .is_some_and(is_host_fatal_error)
        {
            drop(host_admission_guard);
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
        drop(host_admission_guard);
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
        AssertUnwindSafe(self.set_primary_visibility_once(visibility))
            .catch_unwind()
            .await
            .map_err(|_| visibility_operation_panicked_error("primary"))?
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

    /// Returns the installation's visibility policy.
    pub async fn visibility_policy(&self) -> BrowserVisibilityPolicy {
        self.inner.config.read().await.visibility_policy
    }

    /// Sets the installation's visibility policy.
    ///
    /// Policy only: this does not move a running Host. The caller pairs it with
    /// [`Self::set_primary_visibility`] when the new policy also pins a
    /// mechanism, so a live change takes effect without waiting for a restart.
    pub async fn set_visibility_policy(&self, policy: BrowserVisibilityPolicy) {
        self.inner.config.write().await.visibility_policy = policy;
    }

    /// Acts on an Agent's declared presentation intent for one Lane.
    ///
    /// This is the Agent-facing half of the visibility design: the Agent says
    /// *what kind of moment this is* and the trusted host decides whether that
    /// warrants a window. The Agent cannot request `Headful` directly, so a
    /// confused or compromised model cannot pin the browser into a state the
    /// user did not allow — the same split as
    /// [`crate::MODEL_IDENTITY_INPUT_FIELDS`] for identity.
    ///
    /// Advisory by design: declining to escalate is a normal outcome and returns
    /// the unchanged snapshot rather than an error, so an Agent that reports an
    /// attended moment under a pinned-silent policy simply continues headless.
    /// The only errors are real failures (unknown Lane, revoked owner, a
    /// transition that could not be applied).
    ///
    /// Escalation is one-way and bounded; see
    /// [`may_escalate_lane_to_headful`] for the rules and why de-escalation is
    /// deliberately absent.
    pub async fn apply_lane_presentation_intent(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
        intent: BrowserPresentationIntent,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.validate_caller(caller)?;
        let lane = self
            .inner
            .lanes
            .read()
            .await
            .get(lane_id)
            .cloned()
            .ok_or_else(|| BrowserPlatformError::lane_not_found(lane_id.clone()))?;
        let snapshot = lane.current_snapshot().await;
        let policy = self.inner.config.read().await.visibility_policy;
        // Read the *Host's* actual visibility, not `config.headful`. A per-Lane
        // transition deliberately leaves the installation default alone (see
        // `set_lane_visibility_for_user`), so using the default here would treat
        // an already-visible Host as silent and escalate again on every report
        // until the bound was exhausted.
        let host_key = HostKey::for_lane(
            snapshot.identity_mode,
            snapshot.identity_generation,
            &snapshot.lane_id,
        );
        let current = match self.inner.host_slots.read().await.get(&host_key) {
            Some(slot) if slot.is_headful() => BrowserVisibility::Headful,
            // No slot yet means no Host has been published for this Lane, so
            // there is nothing visible to the user.
            _ => BrowserVisibility::Headless,
        };

        // Claim an escalation slot before doing any Host work, so a burst of
        // concurrent attended reports cannot each pass the bound check and queue
        // several process replacements.
        let claimed = lane
            .visibility_escalations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                may_escalate_lane_to_headful(
                    policy,
                    intent,
                    snapshot.identity_mode,
                    current,
                    used,
                )
                .then(|| used.saturating_add(1))
            })
            .is_ok();
        if !claimed {
            return Ok(snapshot);
        }

        // Bring the window to the front: the whole point of escalating is that
        // the user is expected to look at it and possibly take over.
        match self
            .set_lane_visibility_and_maybe_focus_once(
                &caller.user_id,
                lane_id,
                BrowserVisibility::Headful,
                true,
            )
            .await
        {
            Ok(updated) => {
                self.emit("lane_presentation_escalated", Some(&updated));
                Ok(updated)
            }
            Err(error) => {
                // Return the slot; a transition that never happened must not
                // consume the Lane's small allowance.
                lane.visibility_escalations
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                        Some(used.saturating_sub(1))
                    })
                    .ok();
                Err(error)
            }
        }
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
        AssertUnwindSafe(
            self.set_lane_visibility_and_maybe_focus_once(
                user_id,
                lane_id,
                visibility,
                false,
            ),
        )
        .catch_unwind()
        .await
        .map_err(|_| visibility_operation_panicked_error("lane"))?
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

    /// Replaces the live Primary Host with the requested visibility.
    ///
    /// LOCK ORDER: the caller must already hold `primary_visibility_gate`
    /// before this method enters the per-key restart single-flight. Crash
    /// recovery (`recover_host_failure`) follows the same order — gate first,
    /// flight second — so the two can never wait on each other.
    async fn transition_primary_visibility_locked(
        &self,
        host_key: &HostKey,
        observed_epoch: u64,
        desired_headful: bool,
    ) -> Result<(), BrowserPlatformError> {
        let hub = self.clone();
        let restart_key = host_key.clone();
        let terminal_hub = self.clone();
        let terminal_key = host_key.clone();
        let flight = self
            .inner
            .host_restarts
            .run_bounded_with_terminal_callback(
                host_key.clone(),
                observed_epoch,
                HOST_RESTART_ATTEMPT_TIMEOUT,
                move || async move {
                    // State invalidation and terminal failure publication are
                    // owned by the same bounded flight as Host replacement.
                    // Aborting a foreground/visibility waiter therefore
                    // cannot strand Lanes in Starting or suppress the final
                    // recoverable failure state.
                    hub.mark_host_restarting(&restart_key, observed_epoch)
                        .await;
                    let result = hub
                        .restart_host_once_with_visibility(
                            restart_key.clone(),
                            observed_epoch,
                            Some(desired_headful),
                        )
                        .await;
                    result
                },
                move |result| async move {
                    if let Err(error) = result {
                        terminal_hub
                            .mark_host_recovery_failed(
                                &terminal_key,
                                observed_epoch,
                                &error,
                            )
                            .await;
                    }
                },
            )
            .await;
        flight.result?;
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
        AssertUnwindSafe(
            self.set_lane_visibility_and_maybe_focus_once(
                user_id,
                lane_id,
                BrowserVisibility::Headful,
                true,
            ),
        )
        .catch_unwind()
        .await
        .map_err(|_| visibility_operation_panicked_error("foreground"))?
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
                global_memory_pressure_threshold_bytes: decision
                    .effective_browser_memory_limit_bytes,
                max_task_memory_bytes: config.resource_policy.max_task_memory_bytes,
                max_task_active_operations: config
                    .resource_policy
                    .max_task_active_operations
                    .min(config.resource_policy.max_active_operations),
                max_task_open_lanes: config
                    .resource_policy
                    .max_task_open_lanes
                    .min(config.resource_policy.max_open_lanes),
                max_task_tabs: config.resource_policy.max_task_tabs,
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
                        Ok(true) => {
                            self.promote_released_capacity().await;
                            return Ok(Self::scoped_close_result(1, false));
                        }
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
        self.promote_released_capacity().await;
        Ok(Self::scoped_close_result(1, false))
    }

    pub async fn set_keep_alive(
        &self,
        caller: &CallerIdentity,
        lane_id: &BrowserLaneId,
        keep_alive: bool,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.require_operation(caller, BrowserOperationKind::Manage)?;
        let lane = self.authorized_lane(caller, lane_id).await?;
        let mut snapshot = lane.snapshot.write().await;
        snapshot.keep_alive = keep_alive;
        let updated = snapshot.clone();
        drop(snapshot);
        self.emit(
            if keep_alive {
                "lane_keep_alive_enabled"
            } else {
                "lane_keep_alive_disabled"
            },
            Some(&updated),
        );
        Ok(updated)
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
            let retained_driver = driver.take();
            // Epoch zero means Host selection has not been published to this
            // Lane. If selection already happened, start_lane_once registered
            // the exact Host epoch before calling Host.open_lane; adding a
            // synthetic epoch-zero target here can never prove a real process
            // stopped and would survive forever after a timeout. A retained
            // driver is published only together with a non-zero snapshot
            // epoch while close_gate is held.
            if snapshot.browser_epoch != 0 {
                owner_targets
                    .entry(snapshot.caller.owner_lease_id.clone())
                    .or_default()
                    .insert(OwnerCleanupTarget {
                        user_id: snapshot.caller.user_id.clone(),
                        task_id: snapshot.caller.task_resource_key(),
                        family_id: snapshot
                            .caller
                            .task_resource_family_key()
                            .into_string(),
                        lane_id: lane_id.clone(),
                        host_key: host_key.clone(),
                        browser_epoch: snapshot.browser_epoch,
                        requires_host_stop: false,
                    });
            }
            let cleanup_id = retained_driver.map(|driver| {
                let cleanup_id =
                    self.inner.cleanup_sequence.fetch_add(1, Ordering::AcqRel) + 1;
                pending.push(Arc::new(PendingLaneCleanup {
                    cleanup_id,
                    lane_id: lane_id.clone(),
                    user_id: snapshot.caller.user_id.clone(),
                    task_id: snapshot.caller.task_resource_key(),
                    family_id: snapshot
                        .caller
                        .task_resource_family_key()
                        .into_string(),
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
        let cleanup_pending = cleanup_id.is_some();
        let pending_start_flight = start_flight
            .as_ref()
            .filter(|flight| flight.result.get().is_none())
            .cloned();
        let start_pending = pending_start_flight.is_some();
        if let Some(start_flight) = pending_start_flight {
            self.inner
                .pending_host_retirements
                .lock()
                .await
                .push(PendingHostRetirement {
                    key: host_key.clone(),
                    lane_id: lane_id.clone(),
                    user_id: snapshot.caller.user_id.clone(),
                    task_id: snapshot.caller.task_resource_key(),
                    family_id: snapshot
                        .caller
                        .task_resource_family_key()
                        .into_string(),
                    owner_lease_id: snapshot.caller.owner_lease_id.clone(),
                    start_flight,
                });
        }
        if cleanup_pending || start_pending {
            self.ensure_cleanup_retry_worker();
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
                    // Keep completion publication outside the cleanup body so
                    // an adapter panic, an invariant panic, or cancellation of
                    // the inner task can never strand waiters on an unset
                    // OnceLock or leave the active-flight slot permanently
                    // occupied.
                    let cleanup_hub = hub.clone();
                    let cleanup_entry = Arc::clone(&entry_for_task);
                    let result = match tokio::spawn(async move {
                        cleanup_hub.run_pending_lane_cleanup(cleanup_entry).await
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(join_error) => Err(cleanup_batch_task_failed_error(
                            "lane",
                            &join_error,
                        )
                        .for_lane(entry_for_task.lane_id.clone())),
                    };
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
                        drop(active);
                        hub.ensure_cleanup_retry_worker();
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
        // Isolate driver panics in an inner task. A soft caller deadline does
        // not cancel this Hub-owned flight, but the flight itself has an
        // absolute bound: a permanently hung adapter must not retain the Hub,
        // driver and Chromium process forever. At the hard deadline exact Host
        // retirement (or shared-Host restart/rebind) becomes the cleanup proof.
        let driver = Arc::clone(&entry.driver);
        let mut driver_cleanup = tokio::spawn(async move { driver.close().await });
        let result = match tokio::time::timeout(
            LANE_CLEANUP_HARD_TIMEOUT,
            &mut driver_cleanup,
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                driver_cleanup.abort();
                let _ = driver_cleanup.await;
                tracing::warn!(
                    lane_id = %entry.lane_id,
                    timeout_ms = LANE_CLEANUP_HARD_TIMEOUT.as_millis() as u64,
                    "browser Lane target cleanup hit its hard deadline; escalating to exact Host retirement"
                );
                let live_siblings = self.lanes_for_host_key(&entry.host_key).await;
                let escalation = if live_siblings.is_empty() {
                    self.retire_empty_host_authoritatively(
                        &entry.host_key,
                        entry.browser_epoch,
                    )
                    .await
                    .map(|_| ())
                } else {
                    self.clone()
                        .recover_host_failure_owned(
                            entry.host_key.clone(),
                            entry.browser_epoch,
                        )
                        .await
                        .map(|_| ())
                };
                match escalation {
                    Ok(()) => {
                        self.inner
                            .pending_lane_cleanups
                            .lock()
                            .await
                            .retain(|pending| !Arc::ptr_eq(pending, &entry));
                        self.clear_completed_cleanup_authority_for_stopped_host(
                            &entry.host_key,
                            entry.browser_epoch,
                        )
                        .await;
                        self.emit("lane_cleanup_escalated", None);
                        return Ok(());
                    }
                    Err(error) => {
                        return Err(lane_cleanup_hard_timeout_error(
                            entry.lane_id.clone(),
                            error,
                        ));
                    }
                }
            }
        };
        match result {
            Ok(Ok(())) => {
                let host_key = entry.host_key.clone();
                self.inner
                    .pending_lane_cleanups
                    .lock()
                    .await
                    .retain(|pending| !Arc::ptr_eq(pending, &entry));
                self.clear_lane_cleanup_target_if_host_shared(&entry).await;
                self.emit("lane_cleanup_finished", None);
                // This cleanup flight is already Hub-owned. Complete Host
                // retirement inline instead of spawning follower tasks: this
                // preserves cancellation safety, joins the per-Host flight,
                // and prevents a completed installation drain from racing a
                // just-scheduled ghost finalization entry.
                self.finalize_hosts_ready_after_cleanup().await;
                if let Err(error) = self.finalize_host_once(host_key.clone()).await {
                    tracing::warn!(
                        identity_mode = ?host_key.identity_mode,
                        code = ?error.code,
                        "browser Host finalization remains pending after Lane cleanup"
                    );
                }
                self.clear_lane_cleanup_target_if_host_shared(&entry).await;
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

    async fn clear_lane_cleanup_target_if_host_shared(
        &self,
        entry: &PendingLaneCleanup,
    ) {
        let live_lanes = self.lanes_for_host_key(&entry.host_key).await;
        if live_lanes.is_empty() {
            return;
        }
        let removed = {
            let mut owner_targets = self.inner.owner_cleanup_targets.lock().await;
            let mut removed = false;
            if let Some(targets) = owner_targets.get_mut(&entry.owner_lease_id) {
                let previous_len = targets.len();
                targets.retain(|target| {
                target.lane_id != entry.lane_id
                    || target.host_key != entry.host_key
                    || target.browser_epoch != entry.browser_epoch
                });
                removed = targets.len() != previous_len;
                if targets.is_empty() {
                    owner_targets.remove(&entry.owner_lease_id);
                }
            }
            removed
        };
        // Closing the exact target is sufficient proof for the Lane token;
        // the Host token intentionally remains while sibling Lanes keep the
        // shared process alive. Preserve a token if this logical Lane already
        // rebound to another Host epoch.
        if removed {
            self.release_lane_cleanup_budget_if_unowned(&entry.lane_id)
                .await;
        }
    }

    /// Starts the one weakly-owned cleanup supervisor for this Hub.
    ///
    /// Publishing retained authority and starting this worker are kept in the
    /// same Hub method paths. A failed target close, a late Lane start, or a
    /// failed Host shutdown therefore progresses without waiting for another
    /// browser request or relying exclusively on the application's periodic
    /// sweep. The worker drops its strong Hub reference before every delay.
    fn ensure_cleanup_retry_worker(&self) {
        if self
            .inner
            .cleanup_retry_worker_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        let worker_weak = weak.clone();
        let spawn = std::thread::Builder::new()
            .name("nomifun-browser-cleanup".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        if let Some(inner) = worker_weak.upgrade() {
                            inner
                                .cleanup_retry_worker_running
                                .store(false, Ordering::Release);
                        }
                        tracing::error!(
                            %error,
                            "failed to build the independent browser cleanup runtime"
                        );
                        return;
                    }
                };
                let run_weak = worker_weak.clone();
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(Self::run_cleanup_retry_worker(run_weak));
                }));
                if result.is_err() {
                    if let Some(inner) = worker_weak.upgrade() {
                        inner
                            .cleanup_retry_worker_running
                            .store(false, Ordering::Release);
                    }
                    tracing::error!(
                        "independent browser cleanup supervisor terminated unexpectedly"
                    );
                }
            });
        if let Err(error) = spawn {
            self.inner
                .cleanup_retry_worker_running
                .store(false, Ordering::Release);
            tracing::error!(
                %error,
                "failed to start the independent browser cleanup supervisor"
            );
        }
    }

    async fn run_cleanup_retry_worker(weak: Weak<BrowserSessionHubInner>) {
        let mut delay = AUTONOMOUS_CLEANUP_RETRY_INITIAL;
        loop {
            tokio::time::sleep(delay).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let hub = Self { inner };
            // A panic in Hub bookkeeping is fenced just like an adapter panic:
            // the outer supervisor remains alive and retries the still-retained
            // authority instead of silently disappearing.
            let pass_hub = hub.clone();
            let _pass_failed = match tokio::spawn(async move {
                pass_hub.autonomous_cleanup_pass().await
            })
            .await
            {
                Ok(Ok(())) => false,
                Ok(Err(error)) => {
                    tracing::warn!(
                        code = ?error.code,
                        "autonomous browser cleanup pass remains pending"
                    );
                    true
                }
                Err(join_error) => {
                    tracing::error!(
                        cancelled = join_error.is_cancelled(),
                        panic = join_error.is_panic(),
                        "autonomous browser cleanup pass terminated unexpectedly"
                    );
                    true
                }
            };

            if !hub.cleanup_authority_pending().await {
                hub.inner
                    .cleanup_retry_worker_running
                    .store(false, Ordering::Release);
                // Close the publish-vs-exit race. If new authority appeared
                // after the empty check, either this worker reclaims the flag
                // or the publisher has already started its successor.
                if hub.cleanup_authority_pending().await
                    && hub
                        .inner
                        .cleanup_retry_worker_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    delay = AUTONOMOUS_CLEANUP_RETRY_INITIAL;
                    drop(hub);
                    continue;
                }
                return;
            }

            // Retained work may be an intentionally slow Host/Lane start, so
            // back off even when a pass itself returned cleanly. New cleanup
            // publication is still handled synchronously by its caller; this
            // worker is the leak-proof fallback, not the latency fast path.
            delay = delay.saturating_mul(2).min(AUTONOMOUS_CLEANUP_RETRY_MAX);
            drop(hub);
        }
    }

    async fn process_host_stop_required_authorities(
        &self,
    ) -> Result<(), BrowserPlatformError> {
        let authorities = self
            .inner
            .host_stop_required_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for authority in authorities {
            let marker_remains = self
                .inner
                .prepared_rebind_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .any(|(_, current)| current == &authority);
            let unknown_open_target_remains = self
                .inner
                .owner_cleanup_targets
                .lock()
                .await
                .values()
                .any(|targets| {
                    targets.iter().any(|target| {
                        target.requires_host_stop
                            && target.host_key == authority.host_key
                            && target.browser_epoch == authority.browser_epoch
                    })
                });
            if !marker_remains && !unknown_open_target_remains {
                self.inner
                    .host_stop_required_authorities
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&authority);
                continue;
            }
            if !self
                .managed_host_exists_for_key_epoch(
                    &authority.host_key,
                    authority.browser_epoch,
                )
                .await
            {
                self.clear_completed_cleanup_authority_for_stopped_host(
                    &authority.host_key,
                    authority.browser_epoch,
                )
                .await;
                continue;
            }
            let exact_live_lane_exists = {
                let lanes = self.lanes_for_host_key(&authority.host_key).await;
                let mut exists = false;
                for lane in lanes {
                    if lane.snapshot.read().await.browser_epoch
                        == authority.browser_epoch
                    {
                        exists = true;
                        break;
                    }
                }
                exists
            };
            let result = if exact_live_lane_exists {
                self.clone()
                    .recover_host_failure_owned(
                        authority.host_key.clone(),
                        authority.browser_epoch,
                    )
                    .await
                    .map(|_| true)
            } else {
                self.retire_empty_host_authoritatively(
                    &authority.host_key,
                    authority.browser_epoch,
                )
                .await
            };
            match result {
                Ok(_) => {
                    if !self
                        .managed_host_exists_for_key_epoch(
                            &authority.host_key,
                            authority.browser_epoch,
                        )
                        .await
                    {
                        self.clear_completed_cleanup_authority_for_stopped_host(
                            &authority.host_key,
                            authority.browser_epoch,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn cleanup_authority_pending(&self) -> bool {
        self.inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .any(|authority| authority.abandoned.load(Ordering::Acquire))
            || !self
            .inner
            .abandoned_lane_starts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
            || !self
                .inner
                .host_stop_required_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
            || self
                .inner
                .cleanup_ledger_reconcile_requested
                .load(Ordering::Acquire)
            || self
                .inner
                .scheduler_reconcile_requested
                .load(Ordering::Acquire)
            || !self
                .inner
                .exact_lane_cleanup_handoffs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entries
                .is_empty()
            || !self
                .inner
                .primary_profile_cleanup_epochs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
            || !self.inner.pending_lane_cleanups.lock().await.is_empty()
            || !self.inner.pending_host_retirements.lock().await.is_empty()
            || !self.inner.retiring_host_slots.lock().await.is_empty()
            || !self.inner.orphaned_host_slots.lock().await.is_empty()
            || !self.inner.host_finalizations.lock().await.is_empty()
    }

    async fn autonomous_cleanup_pass(&self) -> Result<(), BrowserPlatformError> {
        // Consume only the O(1) wakeup. The exact Lane/Host/debt ledgers below
        // remain the sole cleanup authority, including when more than 64
        // unrelated tasks hit cleanup-budget backpressure concurrently.
        self.inner
            .cleanup_ledger_reconcile_requested
            .swap(false, Ordering::AcqRel);
        self.inner
            .scheduler_reconcile_requested
            .swap(false, Ordering::AcqRel);
        self.process_abandoned_restart_slots().await;
        self.rearm_pending_anonymous_profile_rotations();
        self.rearm_pending_primary_profile_cleanup();
        self.process_abandoned_lane_starts().await;
        let mut first_error = self.process_exact_lane_cleanup_handoffs().await.err();
        if let Err(error) = self.process_host_stop_required_authorities().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = self.retry_pending_lane_cleanups().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        self.finalize_hosts_ready_after_cleanup().await;

        // A failed finalization result was already published to its explicit
        // waiters. Retire that settled flight before opening the autonomous
        // retry, while preserving genuinely in-flight attempts.
        let retry_keys = {
            let mut flights = self.inner.host_finalizations.lock().await;
            let keys = flights
                .iter()
                .filter(|(_, flight)| matches!(flight.result.get(), Some(Err(_))))
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in &keys {
                flights.remove(key);
            }
            keys
        };
        for key in retry_keys {
            if let Err(error) = self.finalize_host_once(key).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
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
        // Cleanup debt is a family admission fence. Once an autonomous pass
        // proves that debt is gone, actively revisit queued work instead of
        // waiting for an unrelated telemetry, policy, or close event. No
        // cleanup retry/finalization gate is held at this boundary.
        self.promote_released_capacity().await;
        first_error.map_or(Ok(()), Err)
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
        let mut cleanup_ids = cleanup_ids.into_iter();
        let mut first_error = None;
        loop {
            while attempts.len() < MAX_CONCURRENT_LANE_CLEANUPS {
                let Some(cleanup_id) = cleanup_ids.next() else {
                    break;
                };
                let hub = self.clone();
                attempts.spawn(async move {
                    hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                        .await
                });
            }
            let Some(result) = attempts.join_next().await else {
                break;
            };
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
                let blocked_tasks = self.blocked_task_owners(&policy).await;
                self.inner
                    .scheduler
                    .update_recommended_concurrency(decision.recommended_concurrency);
                self.inner
                    .scheduler
                    .promote_one_with_policy(&Self::promotion_policy(
                        &decision,
                        blocked_tasks,
                    ))
            };
            let Some(request) = promoted else {
                return;
            };
            // Promotion changes scheduler capacity before the caller-owned
            // future can acquire inventory/start-flight locks. Keep exact
            // rollback authority across every such cancellation point.
            let mut unpublished_promotion = UnpublishedLanePromotionGuard::new(
                self.clone(),
                request.lane_id.clone(),
            );
            #[cfg(test)]
            self.test_after_scheduler_promotion().await;
            if self.inner.shutting_down.load(Ordering::Acquire) {
                // Guard Drop returns the exact Lane to its queue. The
                // installation drain will detach it from authoritative
                // inventory; do not leave an unrepresented scheduler hole.
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
            let start_guard = self.inner.open_gate.lock().await;
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return;
            }
            if self.inner.draining.load(Ordering::Acquire) {
                let _ = self
                    .defer_promoted_lane_to_queue(
                        &request.lane_id,
                        &lane,
                        "browser_policy_reconciliation",
                    )
                    .await;
                return;
            }
            if !self
                .inner
                .lanes
                .read()
                .await
                .get(&request.lane_id)
                .is_some_and(|current| Arc::ptr_eq(current, &lane))
            {
                self.inner
                    .scheduler
                    .release_without_promotion(&request.lane_id);
                continue;
            }
            // Acquire every async publication lock before mutating inventory.
            // From the snapshot transition through start-flight installation,
            // task spawn and guard publication there is no `.await`; caller
            // cancellation can therefore only observe either a queued Lane or
            // a Hub-owned start flight, never an active scheduler ghost.
            let mut snapshot = lane.snapshot.write().await;
            let mut active_flight = lane.start_flight.lock().await;
            let (flight, spawn_start) = if let Some(flight) = active_flight.clone() {
                (flight, false)
            } else {
                let flight = Arc::new(LaneStartFlight::new());
                *active_flight = Some(Arc::clone(&flight));
                (flight, true)
            };
            snapshot.lifecycle_state = LaneLifecycleState::Failed;
            snapshot.queue = None;
            lane.start_claimed.store(true, Ordering::Release);
            if spawn_start {
                self.spawn_lane_start(
                    request.lane_id.clone(),
                    Arc::clone(&lane),
                    Arc::clone(&flight),
                    true,
                );
            }
            unpublished_promotion.publish();
            drop(active_flight);
            drop(snapshot);
            drop(start_guard);
            if let Err(error) = flight.wait().await {
                tracing::warn!(
                    lane_id = %request.lane_id,
                    code = ?error.code,
                    "queued browser lane failed to start"
                );
            }
        }
    }

    #[cfg(test)]
    async fn test_after_scheduler_promotion(&self) {
        self.inner
            .promotion_publication_attempts
            .fetch_add(1, Ordering::AcqRel);
        self.inner.promotion_publication_changed.notify_waiters();
        if self
            .inner
            .promotion_publication_panics_remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                (remaining != 0).then(|| remaining - 1)
            })
            .is_ok()
        {
            panic!("synthetic queued Lane promotion panic");
        }
        if self
            .inner
            .promotion_publication_blocked
            .load(Ordering::Acquire)
        {
            self.inner
                .promotion_publication_release
                .acquire()
                .await
                .expect("promotion publication test semaphore closed")
                .forget();
        }
    }

    pub async fn close_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.close_matching(|lane| lane.conversation_id() == Some(conversation_id))
            .await
    }

    pub async fn close_all(&self) -> Result<CloseResult, BrowserPlatformError> {
        let (flight, leader) = {
            let mut active = self
                .inner
                .close_all_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(flight) = active.clone() {
                (flight, false)
            } else {
                let flight = Arc::new(CloseAllFlight::new());
                *active = Some(Arc::clone(&flight));
                (flight, true)
            }
        };
        if leader {
            let hub = self.clone();
            let run_flight = Arc::clone(&flight);
            tokio::spawn(async move {
                let result = AssertUnwindSafe(hub.close_all_once())
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(drain_operation_panicked_error()));
                {
                    let mut active = hub
                        .inner
                        .close_all_flight
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if active
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &run_flight))
                    {
                        *active = None;
                    }
                }
                // Publication order is part of the single-flight contract:
                // any waiter that can observe completion must also be able to
                // install the next flight immediately.  Clearing after notify
                // leaves a narrow stale-slot window for back-to-back drains.
                run_flight.complete(result);
            });
        }
        flight.wait().await
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
                retiring.push((key.clone(), Arc::clone(&slot)));
            }
            let authority_key = HostCleanupAuthorityKey {
                host_key: key,
                browser_epoch: slot.epoch,
            };
            let mut published = self
                .inner
                .published_restart_slots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if published
                .get(&authority_key)
                .is_some_and(|authority| Arc::ptr_eq(&authority.slot, &slot))
            {
                // `retiring_host_slots` now owns the exact Arc, so a restart
                // guard that is concurrently cancelled must not publish a
                // second orphan authority for the same physical Host.
                published.remove(&authority_key);
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
        let mut cleanup_requests = detached_lanes
            .iter()
            .filter(|detached| detached.cleanup_id.is_some())
            .map(|detached| {
                (
                    detached.cleanup_id.expect("filtered cleanup id"),
                    detached.host_key.clone(),
                    detached.browser_epoch,
                )
            });
        let mut first_error = None;
        let mut terminal_cleanup_errors =
            HashMap::<(HostKey, u64), BrowserPlatformError>::new();
        let mut running_cleanup_targets = HashSet::<(HostKey, u64)>::new();
        let mut unknown_cleanup_task_failure = false;
        loop {
            while attempts.len() < MAX_CONCURRENT_LANE_CLEANUPS {
                let Some((cleanup_id, host_key, browser_epoch)) =
                    cleanup_requests.next()
                else {
                    break;
                };
                let hub = self.clone();
                attempts.spawn(async move {
                    (
                        host_key,
                        browser_epoch,
                        hub.attempt_pending_lane_cleanup_until(cleanup_id, deadline)
                            .await,
                    )
                });
            }
            let Some(attempt) = attempts.join_next().await else {
                break;
            };
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
        drop(cleanup_requests);
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
        // Serialize exact Host proof against prepared-driver publication.
        // Either pending/live authority is visible first, or this proof
        // removes the marker first and the publisher refuses the stale driver.
        let _authority_gate = self.inner.prepared_rebind_authority_gate.lock().await;
        let stopped_authority = HostCleanupAuthorityKey {
            host_key: key.clone(),
            browser_epoch,
        };
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
        let mut stopped_lane_ids = HashSet::new();
        for entry in entries {
            let flight = entry.flight.lock().await.clone();
            if flight
                .as_ref()
                .is_none_or(|flight| flight.result.get().is_some())
            {
                removable.insert(entry.cleanup_id);
                stopped_lane_ids.insert(entry.lane_id.clone());
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
                    let stopped = &target.host_key == key
                        && target.browser_epoch == browser_epoch;
                    if stopped {
                        stopped_lane_ids.insert(target.lane_id.clone());
                    }
                    !stopped
                });
                !targets.is_empty()
            });
        }
        {
            let mut prepared = self
                .inner
                .prepared_rebind_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            prepared.retain(|(lane_id, authority)| {
                let stopped = authority == &stopped_authority;
                if stopped {
                    stopped_lane_ids.insert(lane_id.clone());
                }
                !stopped
            });
        }
        self.inner
            .host_stop_required_authorities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&stopped_authority);
        for lane_id in stopped_lane_ids {
            // The helper also checks concurrent old/new-epoch pending drivers
            // for the same logical Lane, not merely live inventory.
            self.release_lane_cleanup_budget_if_unowned(&lane_id)
                .await;
        }
        self.release_host_cleanup_budget(key, browser_epoch);
        if !self.managed_host_exists_for_key(key).await {
            self.inner
                .host_finalizations
                .lock()
                .await
                .retain(|pending_key, flight| {
                    pending_key != key || flight.result.get().is_none()
                });
            // No process remains for this key; a retained restart gate could
            // only leak (isolated-lane UUID and replica-generation keys never
            // repeat). Gates with an in-flight attempt are kept.
            self.inner.host_restarts.evict_settled(key).await;
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
                let finalize_hub = hub.clone();
                let finalize_key = run_key.clone();
                let result = match tokio::spawn(async move {
                    finalize_hub.finalize_empty_host(finalize_key).await
                })
                .await
                {
                    Ok(result) => result,
                    Err(join_error) => {
                        Err(cleanup_batch_task_failed_error("host-finalizer", &join_error))
                    }
                };
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
                } else {
                    hub.ensure_cleanup_retry_worker();
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
        self.evict_host_circuit_if_unowned(key).await;
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
        self.evict_host_circuit_if_unowned(key).await;
        self.inner.retiring_hosts_changed.notify_waiters();
    }

    async fn evict_host_circuit_if_unowned(&self, key: &HostKey) {
        let _open_guard = self.inner.open_gate.lock().await;
        if self.inner.host_slots.read().await.contains_key(key)
            || self.inner.retiring_host_keys.read().await.contains(key)
        {
            return;
        }
        for lane in self.inner.lanes.read().await.values().cloned() {
            let snapshot = lane.snapshot.read().await;
            if HostKey::for_lane(
                snapshot.identity_mode,
                snapshot.identity_generation,
                &snapshot.lane_id,
            ) == *key
            {
                return;
            }
        }
        if self
            .inner
            .pending_host_retirements
            .lock()
            .await
            .iter()
            .any(|pending| pending.key == *key)
            || self
                .inner
                .pending_lane_cleanups
                .lock()
                .await
                .iter()
                .any(|pending| pending.host_key == *key)
        {
            return;
        }
        self.inner.host_circuits.lock().await.remove(key);
    }

    async fn task_family_live_lane_count(&self, family_id: &str) -> usize {
        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut count = 0;
        for lane in records {
            let snapshot = lane.snapshot.read().await;
            if snapshot.caller.task_resource_family_key().as_str() == family_id
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
            || snapshot.keep_alive
            || snapshot.lifecycle_state != LaneLifecycleState::Running
            || snapshot.active_operation_count != 0
            || now.saturating_sub(snapshot.last_active_at_ms) < idle_limit_ms
            || !(lane.priority == LanePriority::Expansion
                || is_crawl_identity(snapshot.identity_mode))
            || self
                .task_family_live_lane_count(
                    snapshot.caller.task_resource_family_key().as_str(),
                )
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
            PressureCloseFilter::FrozenPressureReclaim => {
                snapshot.lifecycle_state == LaneLifecycleState::Frozen
                    && (lane.priority == LanePriority::Expansion
                        || is_crawl_identity(snapshot.identity_mode))
            }
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
            || snapshot.keep_alive
            || !lifecycle_matches
            || snapshot.active_operation_count != 0
            || now.saturating_sub(snapshot.last_active_at_ms) < idle_limit_ms
        {
            return Ok(0);
        }
        if protect_only_owner_lane
            && self
                .task_family_live_lane_count(
                    snapshot.caller.task_resource_family_key().as_str(),
                )
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
        let mut slots = slots.into_iter();
        let mut stopped = 0;
        let mut first_error = None;
        loop {
            while attempts.len() < MAX_CONCURRENT_HOST_CLEANUPS {
                let Some((key, slot)) = slots.next() else {
                    break;
                };
                attempts.spawn(async move {
                    let result = slot.shutdown_retired().await;
                    (key, slot, result)
                });
            }
            let Some(attempt) = attempts.join_next().await else {
                break;
            };
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
        let mut slots = slots.into_iter();
        let mut stopped = 0;
        let mut first_error = None;
        loop {
            while attempts.len() < MAX_CONCURRENT_HOST_CLEANUPS {
                let Some((key, slot)) = slots.next() else {
                    break;
                };
                attempts.spawn(async move {
                    let result = slot.shutdown_retired().await;
                    (key, slot, result)
                });
            }
            let Some(attempt) = attempts.join_next().await else {
                break;
            };
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

    async fn lanes_exceeding_policy(
        &self,
        policy: &ResourcePolicy,
        host_scope: Option<&HostKey>,
    ) -> Vec<BrowserLaneId> {
        let active_lane_ids = self
            .inner
            .scheduler
            .active_requests()
            .into_iter()
            .map(|request| request.lane_id)
            .collect::<HashSet<_>>();
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut candidates = Vec::new();
        for lane in records {
            let snapshot = lane.current_snapshot().await;
            if !active_lane_ids.contains(&snapshot.lane_id)
                || lane.closing.load(Ordering::Acquire)
                || host_scope.is_some_and(|key| {
                    HostKey::for_lane(
                        snapshot.identity_mode,
                        snapshot.identity_generation,
                        &snapshot.lane_id,
                    ) != *key
                })
            {
                continue;
            }
            candidates.push(PolicyLaneCandidate {
                lane_id: snapshot.lane_id,
                task_id: snapshot.caller.task_resource_family_key().into_string(),
                priority: lane.priority,
                lifecycle_state: snapshot.lifecycle_state,
                created_at_ms: snapshot.created_at_ms,
            });
        }

        // Tab inventory is a presentation cache and may be stale. Whole Lane
        // closure is therefore driven only by Lane caps. Exact target excess
        // is handled below by the shared reservation authority plus each Host
        // reconcile executor, which preserves active tabs and the Lane itself.
        let mut close = HashSet::new();
        let task_lane_limit = policy
            .max_task_open_lanes
            .min(policy.max_open_lanes)
            .min(policy.max_task_tabs)
            .max(1);
        let mut by_task = BTreeMap::<String, Vec<PolicyLaneCandidate>>::new();
        for lane in candidates.iter().filter(|lane| !close.contains(&lane.lane_id)) {
            by_task
                .entry(lane.task_id.clone())
                .or_default()
                .push(lane.clone());
        }
        for lanes in by_task.values_mut() {
            lanes.sort_by_key(PolicyLaneCandidate::survivor_key);
            close.extend(
                lanes
                    .iter()
                    .skip(task_lane_limit)
                    .map(|lane| lane.lane_id.clone()),
            );
        }

        // The Host-scoped rebind check enforces only task-local limits. The
        // installation-wide active ceiling is reconciled by set_resource_policy
        // across all Host keys, otherwise each Host could independently keep
        // the full global allowance.
        if host_scope.is_none() {
            let mut survivors = candidates
                .iter()
                .filter(|lane| !close.contains(&lane.lane_id))
                .cloned()
                .collect::<Vec<_>>();
            survivors.sort_by_key(PolicyLaneCandidate::survivor_key);
            close.extend(
                survivors
                    .iter()
                    .skip(policy.max_open_lanes)
                    .map(|lane| lane.lane_id.clone()),
            );
        }

        let mut close = close.into_iter().collect::<Vec<_>>();
        close.sort();
        close
    }

    async fn close_policy_excess_lanes(
        &self,
        policy: &ResourcePolicy,
        host_scope: Option<&HostKey>,
    ) -> Result<usize, BrowserPlatformError> {
        let lane_ids = self.lanes_exceeding_policy(policy, host_scope).await;
        let mut closed = 0usize;
        let mut first_error = None;
        for lane_id in lane_ids {
            match self.close_lane(&lane_id).await {
                Ok(result) => closed = closed.saturating_add(result.closed),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(closed), |error| {
            Err(policy_reconciliation_pending_error(error, closed))
        })
    }

    async fn close_starting_lanes_for_tab_lowering(
        &self,
    ) -> Result<usize, BrowserPlatformError> {
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut lane_ids = Vec::new();
        for lane in records {
            let snapshot = lane.current_snapshot().await;
            if snapshot.lifecycle_state == LaneLifecycleState::Starting
                && !lane.closing.load(Ordering::Acquire)
            {
                lane_ids.push(snapshot.lane_id);
            }
        }
        lane_ids.sort();
        let mut closed = 0usize;
        let mut first_error = None;
        for lane_id in lane_ids {
            match self.close_lane(&lane_id).await {
                Ok(result) => closed = closed.saturating_add(result.closed),
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(closed), |error| {
            Err(policy_reconciliation_pending_error(error, closed))
        })
    }

    async fn settle_initial_lane_starts_for_tab_raise(
        &self,
    ) -> Result<(), BrowserPlatformError> {
        // `draining` is already installed by the caller, so this is a closed
        // set: no newly admitted Lane can enter Host.open_lane while policy
        // reconciliation is waiting. A pre-existing start may have captured
        // the old local cap; wait for that exact route to publish, then the
        // normal Host reconcile below overwrites it before the global
        // authority is raised.
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut flights = Vec::new();
        for lane in records {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = lane.snapshot.read().await;
            if snapshot.lifecycle_state != LaneLifecycleState::Starting {
                continue;
            }
            drop(snapshot);
            if let Some(flight) = lane.start_flight.lock().await.clone()
                && flight.result.get().is_none()
            {
                flights.push(flight);
            }
        }
        let deadline = Instant::now()
            + HOST_INITIALIZATION_GATE_TIMEOUT
            + HOST_LANE_OPEN_TIMEOUT;
        for flight in flights {
            if tokio::time::timeout_at(deadline, flight.wait())
                .await
                .is_err()
            {
                return Err(policy_starting_lane_wait_timeout_error());
            }
        }
        Ok(())
    }

    async fn task_host_tab_targets(
        &self,
    ) -> Result<Vec<PolicyHostTabTarget>, BrowserPlatformError> {
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut lane_counts_by_task = HashMap::<String, HashMap<String, usize>>::new();
        let mut grouped = HashMap::<(String, HostKey), (usize, usize)>::new();
        for lane in records {
            let snapshot = lane.current_snapshot().await;
            if lane.closing.load(Ordering::Acquire)
                || !matches!(
                    snapshot.lifecycle_state,
                    LaneLifecycleState::Running | LaneLifecycleState::Frozen
                )
            {
                continue;
            }
            let task_id = snapshot.caller.task_resource_family_key().into_string();
            let reserved_count = lane_counts_by_task
                .entry(task_id.clone())
                .or_insert_with(|| self.inner.task_tab_authority.lane_counts_for(&task_id))
                .get(snapshot.lane_id.as_str())
                .copied()
                .unwrap_or_default();
            let host_key = HostKey::for_lane(
                snapshot.identity_mode,
                snapshot.identity_generation,
                &snapshot.lane_id,
            );
            let entry = grouped.entry((task_id, host_key)).or_default();
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.saturating_add(reserved_count);
        }

        let slots = self.inner.host_slots.read().await.clone();
        let mut targets = Vec::with_capacity(grouped.len());
        for ((task_id, host_key), (lane_count, reserved_count)) in grouped {
            let driver = slots
                .get(&host_key)
                .and_then(|slot| slot.get())
                .cloned()
                .ok_or_else(|| task_tab_reconciliation_pending_error(&task_id, None))?;
            targets.push(PolicyHostTabTarget {
                task_id,
                host_key,
                lane_count,
                reserved_count,
                driver,
            });
        }
        targets.sort_by(|left, right| {
            left.task_id
                .cmp(&right.task_id)
                .then_with(|| {
                    left.host_key
                        .deterministic_key()
                        .cmp(&right.host_key.deterministic_key())
                })
        });
        Ok(targets)
    }

    async fn reconcile_task_tab_policy(
        &self,
        max_task_tabs: usize,
    ) -> Result<(), BrowserPlatformError> {
        let targets = self.task_host_tab_targets().await?;
        let mut temporary_caps = Vec::with_capacity(targets.len());
        let mut index = 0usize;
        while index < targets.len() {
            let task_id = targets[index].task_id.clone();
            let end = targets[index..]
                .iter()
                .position(|target| target.task_id != task_id)
                .map_or(targets.len(), |offset| index + offset);
            let task_targets = &targets[index..end];
            let live_lane_count = task_targets
                .iter()
                .map(|target| target.lane_count)
                .fold(0usize, usize::saturating_add);
            if live_lane_count > max_task_tabs {
                return Err(task_tab_reconciliation_pending_error(
                    &task_id,
                    Some(live_lane_count),
                ));
            }
            let mut remaining = max_task_tabs.saturating_sub(live_lane_count);
            for target in task_targets {
                let retained_extra = target
                    .reserved_count
                    .saturating_sub(target.lane_count)
                    .min(remaining);
                remaining = remaining.saturating_sub(retained_extra);
                temporary_caps.push(target.lane_count.saturating_add(retained_extra).max(1));
            }
            index = end;
        }

        for (target, temporary_cap) in targets.iter().zip(&temporary_caps) {
            target
                .driver
                .reconcile_task_tab_limit(&target.task_id, *temporary_cap)
                .await
                .map_err(|error| policy_tab_driver_reconciliation_error(error, &target.task_id))?;
        }

        // Reservations belonging to a detached/late-start Lane are not in the
        // live Host plan. Do not report convergence until exact cleanup drops
        // those permits as well.
        for (task_id, count) in self.inner.task_tab_authority.task_counts() {
            if count > max_task_tabs {
                return Err(task_tab_reconciliation_pending_error(
                    &task_id,
                    Some(count),
                ));
            }
            let mapped = targets
                .iter()
                .filter(|target| target.task_id == task_id)
                .map(|target| target.reserved_count)
                .fold(0usize, usize::saturating_add);
            if count > mapped {
                return Err(task_tab_reconciliation_pending_error(
                    &task_id,
                    Some(count),
                ));
            }
        }

        // Temporary Host partitions make the sum of retained targets no more
        // than the task-global cap. Once exact convergence is proven, widen
        // each local defense layer back to the global value so unused capacity
        // is not permanently stranded on another identity/Host. The shared
        // Hub authority remains the sole cross-Host admission boundary.
        for target in &targets {
            target
                .driver
                .reconcile_task_tab_limit(&target.task_id, max_task_tabs)
                .await
                .map_err(|error| policy_tab_driver_reconciliation_error(error, &target.task_id))?;
        }
        Ok(())
    }

    pub async fn set_resource_policy(
        &self,
        policy: ResourcePolicy,
    ) -> Result<(), BrowserPlatformError> {
        let policy_key = serde_json::to_string(&policy)
            .unwrap_or_else(|_| format!("{policy:?}"));
        let (flight, leader) = {
            let mut active = self
                .inner
                .policy_update_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match active.as_ref() {
                Some((active_key, flight)) if active_key == &policy_key => {
                    (Arc::clone(flight), false)
                }
                Some(_) => return Err(policy_reconciliation_busy_error()),
                None => {
                    let flight = Arc::new(PolicyUpdateFlight::new());
                    *active = Some((policy_key.clone(), Arc::clone(&flight)));
                    (flight, true)
                }
            }
        };
        if leader {
            let hub = self.clone();
            let run_flight = Arc::clone(&flight);
            tokio::spawn(async move {
                let result = AssertUnwindSafe(hub.set_resource_policy_once(policy))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(policy_operation_panicked_error()));
                {
                    let mut active = hub
                        .inner
                        .policy_update_flight
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if active.as_ref().is_some_and(|(active_key, current)| {
                        active_key == &policy_key && Arc::ptr_eq(current, &run_flight)
                    }) {
                        *active = None;
                    }
                }
                // Clear the pointer/key guard before publishing completion.
                // A waiter may submit a different policy as soon as it wakes;
                // it must never observe the completed flight as still active.
                run_flight.complete(result);
            });
        }
        flight.wait().await
    }

    async fn set_resource_policy_once(
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
        let _drain_guard = self.inner.drain_gate.lock().await;
        {
            let _open_guard = self.inner.open_gate.lock().await;
            if self.inner.shutting_down.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::shutting_down());
            }
            self.inner.draining.store(true, Ordering::Release);
            self.inner
                .policy_reconciling
                .store(true, Ordering::Release);
        }
        let drain_state = HubDrainGuard {
            inner: Arc::clone(&self.inner),
        };
        let policy_state = PolicyReconciliationGuard {
            inner: Arc::clone(&self.inner),
        };

        let authority_limit = self.inner.task_tab_authority.limit();
        let tab_limit_lowering = policy.max_task_tabs < authority_limit;
        if tab_limit_lowering {
            // This is the linearization point for every Host/identity. The
            // shared reservation lock guarantees no target can read the old
            // ceiling and publish itself after this call returns.
            self.inner
                .task_tab_authority
                .set_limit(policy.max_task_tabs);
            // A Lane already inside Host I/O may have captured an older local
            // route cap. Closing every Starting Lane transfers its late driver
            // to exact cleanup authority; a cleanup timeout keeps the policy
            // uncommitted and the stricter global tab fence installed.
            self.close_starting_lanes_for_tab_lowering().await?;
        }

        // Queue limits are hard state, not metadata. Keep the oldest requests
        // that fit the new global/per-task caps and detach every trimmed Lane
        // before the policy can be reported as installed.
        let removed_queued = self.inner.scheduler.trim_queued_to_limits(
            policy.max_global_queue,
            policy.max_owner_queue,
        );
        let mut queued_reconciliation_error = None;
        for request in removed_queued {
            if let Some(detached) = self.detach_lane_for_close(&request.lane_id).await {
                if let Some(cleanup_id) = detached.cleanup_id {
                    if let Err(error) = self.attempt_pending_lane_cleanup(cleanup_id).await
                        && queued_reconciliation_error.is_none()
                    {
                        queued_reconciliation_error = Some(error);
                    }
                }
                if let Err(error) = self.finalize_detached_host(detached).await
                    && queued_reconciliation_error.is_none()
                {
                    queued_reconciliation_error = Some(error);
                }
            }
        }
        if let Some(error) = queued_reconciliation_error {
            return Err(policy_reconciliation_pending_error(error, 0));
        }

        // Reconcile live fanout before committing configuration. This also
        // enforces max_task_tabs >= retained Lane count, ensuring the engine's
        // per-task tab route can be tightened without an impossible one-tab-
        // per-Lane state.
        if let Err(error) = self.close_policy_excess_lanes(&policy, None).await {
            drop(drain_state);
            return Err(error);
        }

        if policy.max_task_tabs > authority_limit {
            self.settle_initial_lane_starts_for_tab_raise().await?;
        }

        // The Host-local executor is deliberately given temporary partitions
        // whose sum is the task cap. This closes exact cross-Host excess; after
        // convergence each route is widened back to the global value while the
        // shared authority continues to enforce the real task-wide boundary.
        self.reconcile_task_tab_policy(policy.max_task_tabs).await?;

        let operation_weight_limits = {
            let _open_guard = self.inner.open_gate.lock().await;
            let telemetry = self.inner.telemetry.read().await.clone();
            let decision = self.decide_resources(&policy, &telemetry).await;
            self.inner
                .scheduler
                .update_policy_limits_without_promotion(
                    policy.max_open_lanes,
                    policy.max_task_open_lanes.min(policy.max_open_lanes),
                    policy.max_global_queue,
                    policy.max_owner_queue,
                    decision.recommended_concurrency,
                );
            let task_operation_limit = policy
                .max_task_active_operations
                .min(decision.operation_weight_limit);
            self.inner.config.write().await.resource_policy = policy.clone();
            (
                decision.operation_weight_limit,
                task_operation_limit,
            )
        };
        if policy.max_task_tabs > authority_limit {
            // Raising happens only after every live Host route accepts the new
            // cap and configuration is committed. The short interim remains
            // conservatively bounded by the old task-global ceiling.
            self.inner
                .task_tab_authority
                .set_limit(policy.max_task_tabs);
        }
        self.apply_operation_admission_limits(&policy);
        self.apply_operation_weight_limits(operation_weight_limits.0, operation_weight_limits.1)
            .await;
        drop(policy_state);
        drop(drain_state);
        self.promote_released_capacity().await;
        self.emit("resource_policy_changed", None);
        Ok(())
    }

    pub async fn resource_policy(&self) -> ResourcePolicy {
        self.inner.config.read().await.resource_policy.clone()
    }

    async fn resource_emergency_host_candidate(
        &self,
        policy: &ResourcePolicy,
        telemetry: &ResourceTelemetry,
    ) -> Option<ResourceEmergencyHostCandidate> {
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut tasks_by_host = HashMap::<HostKey, HashSet<String>>::new();
        for lane in records {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = lane.snapshot.read().await;
            if !matches!(
                snapshot.lifecycle_state,
                LaneLifecycleState::Starting
                    | LaneLifecycleState::Running
                    | LaneLifecycleState::Frozen
                    | LaneLifecycleState::Failed
            ) {
                continue;
            }
            tasks_by_host
                .entry(HostKey::for_lane(
                    snapshot.identity_mode,
                    snapshot.identity_generation,
                    &snapshot.lane_id,
                ))
                .or_default()
                .insert(snapshot.caller.task_resource_family_key().into_string());
        }

        let over_attribution_tasks = self
            .inner
            .task_memory_samples
            .read()
            .await
            .iter()
            .filter(|(_, sample)| {
                sample.shared_rss_estimate_bytes > policy.max_task_memory_bytes
            })
            .map(|(task_id, _)| task_id.clone())
            .collect::<HashSet<_>>();
        let slots = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect::<Vec<_>>();

        let mut best: Option<ResourceEmergencyHostCandidate> = None;
        for (key, slot) in slots {
            let Some(tasks) = tasks_by_host.get(&key).filter(|tasks| !tasks.is_empty()) else {
                continue;
            };
            let Some(rss_bytes) = slot
                .get()
                .and_then(|host| host.process_id())
                .and_then(|process_id| telemetry.host_rss_by_process_id.get(&process_id))
                .copied()
            else {
                continue;
            };
            let exclusive_over_task_budget =
                tasks.len() == 1 && rss_bytes > policy.max_task_memory_bytes;
            let contains_obviously_over_task = tasks
                .iter()
                .any(|task_id| over_attribution_tasks.contains(task_id));
            let attribution_rank = if exclusive_over_task_budget {
                2
            } else if contains_obviously_over_task {
                1
            } else {
                0
            };
            let candidate = ResourceEmergencyHostCandidate {
                key,
                browser_epoch: slot.epoch,
                headful: slot.is_headful(),
                rss_bytes,
                attribution_rank,
            };
            let replace = best.as_ref().is_none_or(|current| {
                (candidate.attribution_rank, candidate.rss_bytes)
                    > (current.attribution_rank, current.rss_bytes)
                    || ((candidate.attribution_rank, candidate.rss_bytes)
                        == (current.attribution_rank, current.rss_bytes)
                        && candidate.key.deterministic_key()
                            > current.key.deterministic_key())
            });
            if replace {
                best = Some(candidate);
            }
        }
        best
    }

    async fn resource_emergency_cpu_host_candidate(
        &self,
        telemetry: &ResourceTelemetry,
    ) -> (f64, Option<ResourceEmergencyCpuHostCandidate>) {
        let records = self
            .inner
            .lanes
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut live_host_keys = HashSet::<HostKey>::new();
        for lane in records {
            if lane.closing.load(Ordering::Acquire) {
                continue;
            }
            let snapshot = lane.snapshot.read().await;
            if !matches!(
                snapshot.lifecycle_state,
                LaneLifecycleState::Starting
                    | LaneLifecycleState::Running
                    | LaneLifecycleState::Frozen
                    | LaneLifecycleState::Failed
            ) {
                continue;
            }
            live_host_keys.insert(HostKey::for_lane(
                snapshot.identity_mode,
                snapshot.identity_generation,
                &snapshot.lane_id,
            ));
        }

        let slots = self
            .inner
            .host_slots
            .read()
            .await
            .iter()
            .map(|(key, slot)| (key.clone(), Arc::clone(slot)))
            .collect::<Vec<_>>();
        let mut total_managed_pressure = 0.0_f64;
        let mut best: Option<ResourceEmergencyCpuHostCandidate> = None;
        for (key, slot) in slots {
            if !live_host_keys.contains(&key) {
                continue;
            }
            let Some(cpu_pressure) = slot
                .get()
                .and_then(|host| host.process_id())
                .and_then(|process_id| {
                    telemetry
                        .host_cpu_pressure_by_process_id
                        .get(&process_id)
                })
                .copied()
                .filter(|pressure| pressure.is_finite() && *pressure > 0.0)
                .map(|pressure| pressure.clamp(0.0, 1.0))
            else {
                continue;
            };
            total_managed_pressure += cpu_pressure;
            let candidate = ResourceEmergencyCpuHostCandidate {
                key,
                browser_epoch: slot.epoch,
                headful: slot.is_headful(),
                cpu_pressure,
            };
            let replace = best.as_ref().is_none_or(|current| {
                candidate.cpu_pressure > current.cpu_pressure
                    || (candidate.cpu_pressure == current.cpu_pressure
                        && candidate.key.deterministic_key()
                            > current.key.deterministic_key())
            });
            if replace {
                best = Some(candidate);
            }
        }

        (total_managed_pressure.clamp(0.0, 1.0), best)
    }

    async fn restart_host_for_resource_emergency(
        &self,
        key: HostKey,
        observed_epoch: u64,
        headful: bool,
        event_name: &'static str,
    ) -> Result<HostRestartTransition, BrowserPlatformError> {
        // Preserve the exact Primary window policy. Resource convergence is a
        // healthy, intentional replacement, so it must not consume the crash
        // circuit's failure budget.
        let _primary_visibility_guard = if key.identity_mode == BrowserIdentityMode::Primary {
            Some(self.inner.primary_visibility_gate.lock().await)
        } else {
            None
        };
        let hub = self.clone();
        let restart_key = key.clone();
        let terminal_hub = self.clone();
        let terminal_key = key.clone();
        let flight = self
            .inner
            .host_restarts
            .run_bounded_with_terminal_callback(
                key,
                observed_epoch,
                HOST_RESTART_ATTEMPT_TIMEOUT,
                move || async move {
                    hub.mark_host_restarting(&restart_key, observed_epoch)
                        .await;
                    hub.restart_host_once_with_visibility(
                        restart_key,
                        observed_epoch,
                        Some(headful),
                    )
                    .await
                },
                move |result| async move {
                    if let Err(error) = result {
                        terminal_hub
                            .mark_host_recovery_failed(
                                &terminal_key,
                                observed_epoch,
                                &error,
                            )
                            .await;
                    }
                },
            )
            .await;
        let transition = flight.result?;
        self.emit(event_name, None);
        Ok(transition)
    }

    async fn converge_sustained_browser_rss_pressure(
        &self,
        telemetry: &ResourceTelemetry,
        decision: &ResourceDecision,
        task_reclaimed: usize,
    ) {
        let browser_limit = decision.effective_browser_memory_limit_bytes;
        let measured_breach = browser_limit != 0
            && telemetry.chromium_rss_bytes > browser_limit
            && !telemetry.host_rss_by_process_id.is_empty();
        if !measured_breach || task_reclaimed != 0 {
            self.inner
                .critical_browser_rss_streak
                .store(0, Ordering::Release);
            return;
        }

        let previous = self
            .inner
            .critical_browser_rss_streak
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |streak| {
                Some(streak.saturating_add(1))
            })
            .unwrap_or_else(|current| current);
        let streak = previous.saturating_add(1);
        if streak < RESOURCE_EMERGENCY_CRITICAL_SAMPLES
            || self
                .inner
                .critical_browser_rss_streak
                .compare_exchange(streak, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }

        // Concurrent telemetry publishers can reach the threshold together.
        // One exact Host replacement is sufficient; later real samples decide
        // whether another Host still needs convergence.
        let Ok(_emergency_guard) = self.inner.resource_emergency_gate.try_lock() else {
            return;
        };
        if self.inner.shutting_down.load(Ordering::Acquire)
            || self.inner.draining.load(Ordering::Acquire)
        {
            return;
        }

        // Policy and telemetry may have changed while this sample accumulated
        // its streak. Revalidate against the current hardware-derived ratio so
        // a raised policy or recovered machine never causes a stale restart.
        let current_policy = self.inner.config.read().await.resource_policy.clone();
        let current_telemetry = self.inner.telemetry.read().await.clone();
        let current_decision = self
            .decide_resources(&current_policy, &current_telemetry)
            .await;
        if current_decision.effective_browser_memory_limit_bytes == 0
            || current_telemetry.chromium_rss_bytes
                <= current_decision.effective_browser_memory_limit_bytes
            || current_telemetry.host_rss_by_process_id.is_empty()
        {
            return;
        }
        let Some(candidate) = self
            .resource_emergency_host_candidate(&current_policy, &current_telemetry)
            .await
        else {
            tracing::warn!(
                chromium_rss_bytes = current_telemetry.chromium_rss_bytes,
                browser_limit_bytes = current_decision.effective_browser_memory_limit_bytes,
                "managed Chromium RSS remains critical but no exact live Host matches the sample"
            );
            return;
        };

        let identity_mode = candidate.key.identity_mode;
        let browser_epoch = candidate.browser_epoch;
        let host_rss_bytes = candidate.rss_bytes;
        if let Err(error) = self
            .restart_host_for_resource_emergency(
                candidate.key,
                candidate.browser_epoch,
                candidate.headful,
                "host_restarted_resource_pressure",
            )
            .await
        {
            tracing::warn!(
                ?identity_mode,
                browser_epoch,
                host_rss_bytes,
                code = ?error.code,
                retryable = error.retryable,
                "sustained managed Chromium pressure Host replacement remains pending"
            );
        } else {
            tracing::warn!(
                ?identity_mode,
                browser_epoch,
                host_rss_bytes,
                browser_limit_bytes = current_decision.effective_browser_memory_limit_bytes,
                "restarted the largest attributable managed Chromium Host after sustained critical RSS"
            );
        }
    }

    async fn converge_sustained_browser_cpu_pressure(
        &self,
        telemetry: &ResourceTelemetry,
        task_reclaimed: usize,
    ) {
        if task_reclaimed != 0
            || telemetry.cpu_pressure < RESOURCE_EMERGENCY_SYSTEM_CPU_PRESSURE
            || telemetry.host_cpu_pressure_by_process_id.is_empty()
        {
            self.inner
                .critical_browser_cpu_streak
                .store(0, Ordering::Release);
            return;
        }
        let (managed_pressure, _) = self
            .resource_emergency_cpu_host_candidate(telemetry)
            .await;
        if managed_pressure < RESOURCE_EMERGENCY_MANAGED_CPU_PRESSURE {
            self.inner
                .critical_browser_cpu_streak
                .store(0, Ordering::Release);
            return;
        }

        let previous = self
            .inner
            .critical_browser_cpu_streak
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |streak| {
                Some(streak.saturating_add(1))
            })
            .unwrap_or_else(|current| current);
        let streak = previous.saturating_add(1);
        if streak < RESOURCE_EMERGENCY_CRITICAL_SAMPLES
            || self
                .inner
                .critical_browser_cpu_streak
                .compare_exchange(streak, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }

        // Serialize with the RSS endpoint: one telemetry generation may show
        // both pressures, but replacing one exact Host is enough until a new
        // real process-tree sample proves that pressure remains.
        let Ok(_emergency_guard) = self.inner.resource_emergency_gate.try_lock() else {
            return;
        };
        if self.inner.shutting_down.load(Ordering::Acquire)
            || self.inner.draining.load(Ordering::Acquire)
        {
            return;
        }

        // Require a fresh exact live-driver PID join immediately before the
        // action. This makes a stale/reused PID or unrelated Chrome process a
        // fail-closed sample, never a termination target.
        let current_telemetry = self.inner.telemetry.read().await.clone();
        if current_telemetry.cpu_pressure < RESOURCE_EMERGENCY_SYSTEM_CPU_PRESSURE
            || current_telemetry
                .host_cpu_pressure_by_process_id
                .is_empty()
        {
            return;
        }
        let (current_managed_pressure, candidate) = self
            .resource_emergency_cpu_host_candidate(&current_telemetry)
            .await;
        if current_managed_pressure < RESOURCE_EMERGENCY_MANAGED_CPU_PRESSURE {
            return;
        }
        let Some(candidate) = candidate else {
            return;
        };

        let identity_mode = candidate.key.identity_mode;
        let browser_epoch = candidate.browser_epoch;
        let host_cpu_pressure = candidate.cpu_pressure;
        if let Err(error) = self
            .restart_host_for_resource_emergency(
                candidate.key,
                candidate.browser_epoch,
                candidate.headful,
                "host_restarted_cpu_pressure",
            )
            .await
        {
            tracing::warn!(
                ?identity_mode,
                browser_epoch,
                host_cpu_pressure,
                managed_browser_cpu_pressure = current_managed_pressure,
                system_cpu_pressure = current_telemetry.cpu_pressure,
                code = ?error.code,
                retryable = error.retryable,
                "sustained managed Chromium CPU Host replacement remains pending"
            );
        } else {
            tracing::warn!(
                ?identity_mode,
                browser_epoch,
                host_cpu_pressure,
                managed_browser_cpu_pressure = current_managed_pressure,
                system_cpu_pressure = current_telemetry.cpu_pressure,
                "restarted the busiest exact managed Chromium Host after sustained critical CPU"
            );
        }
    }

    pub async fn update_resource_telemetry(&self, telemetry: ResourceTelemetry) {
        self.refresh_lane_resource_estimates(&telemetry).await;
        *self.inner.telemetry.write().await = telemetry;
        let policy = self.inner.config.read().await.resource_policy.clone();
        let task_reclaimed = match self.reclaim_over_budget_tasks(&policy).await {
            Ok(closed) => closed,
            Err(error) => {
                tracing::warn!(
                    code = ?error.code,
                    retryable = error.retryable,
                    "task-local browser memory reclaim remains pending"
                );
                0
            }
        };
        let telemetry = self.inner.telemetry.read().await.clone();
        let decision = self.decide_resources(&policy, &telemetry).await;
        self.inner
            .scheduler
            .update_recommended_concurrency(decision.recommended_concurrency);
        self.apply_operation_weight_limits(
            decision.operation_weight_limit,
            policy
                .max_task_active_operations
                .min(decision.operation_weight_limit),
        )
        .await;
        self.promote_released_capacity().await;
        // Every sample drives a client-visible inventory event, and clients
        // refresh on each one. With zero lanes and an unchanged pressure
        // state there is nothing actionable in the sample; suppress the
        // broadcast so an idle installation does not poll itself forever.
        let encoded_state = match decision.state {
            ResourcePressureState::Normal => 1,
            ResourcePressureState::Pressured => 2,
            ResourcePressureState::Critical => 3,
        };
        let previous = self
            .inner
            .last_sampled_pressure_state
            .swap(encoded_state, Ordering::AcqRel);
        let has_lanes = !self.inner.lanes.read().await.is_empty();
        if has_lanes || previous != encoded_state {
            self.emit("resource_pressure_sampled", None);
        }
        self.converge_sustained_browser_rss_pressure(
            &telemetry,
            &decision,
            task_reclaimed,
        )
        .await;
        self.converge_sustained_browser_cpu_pressure(&telemetry, task_reclaimed)
            .await;
    }

    async fn reclaim_over_budget_tasks(
        &self,
        policy: &ResourcePolicy,
    ) -> Result<usize, BrowserPlatformError> {
        let attributions = self.task_memory_attributions(policy).await;
        let active_tasks = attributions.keys().cloned().collect::<HashSet<_>>();
        let mut reclaim = Vec::new();
        {
            let mut streaks = self
                .inner
                .task_over_budget_samples
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            streaks.retain(|task_id, _| active_tasks.contains(task_id));
            for (task_id, attribution) in &attributions {
                if attribution.shared_rss_estimate_bytes <= policy.max_task_memory_bytes {
                    streaks.remove(task_id);
                    continue;
                }
                let streak = streaks.entry(task_id.clone()).or_default();
                *streak = streak.saturating_add(1);
                // Hysteresis floor: an estimated attribution must stay over
                // budget for several consecutive samples before it can reclaim
                // anything. Without this, the accelerators below reach an
                // eligible stage on the very first over-budget sample.
                if *streak < TASK_RECLAIM_MIN_SUSTAINED_SAMPLES {
                    continue;
                }
                let materially_over = attribution.shared_rss_estimate_bytes
                    > policy.max_task_memory_bytes.saturating_mul(3) / 2;
                let severely_over = attribution.shared_rss_estimate_bytes
                    > policy.max_task_memory_bytes.saturating_mul(2);
                let confidence_acceleration = u8::from(
                    attribution.exclusive_hosts_only && materially_over,
                );
                let severity_acceleration = if severely_over {
                    2
                } else if materially_over {
                    1
                } else {
                    0
                };
                let reclaim_stage = streak
                    .saturating_sub(TASK_RECLAIM_MIN_SUSTAINED_SAMPLES.saturating_sub(1))
                    .saturating_add(confidence_acceleration)
                    .saturating_add(severity_acceleration);
                if reclaim_stage >= TASK_RECLAIM_IDLE_EXPANSION_STREAK {
                    reclaim.push((task_id.clone(), reclaim_stage));
                }
            }
        }

        let records: Vec<_> = self.inner.lanes.read().await.values().cloned().collect();
        let mut closed = 0usize;
        let mut first_error = None;
        for (task_id, reclaim_stage) in reclaim {
            let mut candidates = Vec::new();
            let mut live_lane_count = 0usize;
            for lane in &records {
                let snapshot = lane.current_snapshot().await;
                if snapshot.caller.task_resource_family_key().as_str() != task_id
                    || lane.closing.load(Ordering::Acquire)
                    || !matches!(
                        snapshot.lifecycle_state,
                        LaneLifecycleState::Starting
                            | LaneLifecycleState::Running
                            | LaneLifecycleState::Frozen
                    )
                {
                    continue;
                }
                live_lane_count = live_lane_count.saturating_add(1);
                let expansion = lane.priority == LanePriority::Expansion
                    || is_crawl_identity(snapshot.identity_mode);
                let active = snapshot.active_operation_count != 0;
                let eligible = if reclaim_stage >= TASK_RECLAIM_ACTIVE_ANY_STREAK {
                    true
                } else if reclaim_stage >= TASK_RECLAIM_ACTIVE_EXPANSION_STREAK {
                    !active || expansion
                } else if reclaim_stage >= TASK_RECLAIM_IDLE_ANY_STREAK {
                    !active
                } else {
                    expansion && !active
                };
                if !eligible {
                    continue;
                }
                candidates.push((
                    active,
                    !expansion,
                    snapshot.last_active_at_ms,
                    snapshot.created_at_ms,
                    snapshot.lane_id,
                ));
            }
            // A task's last remaining Lane is its entire browser. Closing it on
            // an *estimated* attribution is what made ordinary sessions appear
            // to die at random, so it is reserved for the top of the escalation:
            // the overage must have survived every earlier stage first. Sibling
            // Lanes (a task that expanded) stay reclaimable at the normal
            // stages.
            //
            // Note this deliberately does NOT also require a *severe* overage.
            // Gating the last Lane on severity would make a single-Lane task
            // with a sustained moderate overage permanently immune, and a single
            // Lane is the common shape for an Agent task — that would be a leak
            // hole, not a protection.
            //
            // `freeze_idle_lane_for_pressure` applies the same protection via
            // `task_family_live_lane_count() <= 1`.
            if live_lane_count <= 1 && reclaim_stage < TASK_RECLAIM_ACTIVE_ANY_STREAK {
                continue;
            }
            candidates.sort();
            let Some((_, _, _, _, lane_id)) = candidates.into_iter().next() else {
                continue;
            };
            // Mark the exact Lane before closing so an operation waiting on its
            // cancellation token reports the memory-budget reason rather than
            // "closed by user". This only annotates the close reason; the close
            // itself and its exact cleanup authority are unchanged.
            if let Some(lane) = self.inner.lanes.read().await.get(&lane_id) {
                lane.memory_reclaimed.store(true, Ordering::Release);
            }
            match self.close_lane(&lane_id).await {
                Ok(result) => {
                    closed = closed.saturating_add(result.closed);
                    if result.closed > 0 {
                        self.emit("task_memory_reclaimed", None);
                    }
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(closed), Err)
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
        let mut task_activity_by_host =
            HashMap::<HostKey, HashMap<String, TaskHostActivity>>::new();
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
                let task_id = snapshot.caller.task_resource_family_key().into_string();
                let activity = task_activity_by_host
                    .entry(key.clone())
                    .or_default()
                    .entry(task_id.clone())
                    .or_default();
                activity.lane_count = activity.lane_count.saturating_add(1);
                activity.tab_count = activity.tab_count.saturating_add(
                    u64::try_from(snapshot.tabs.len()).unwrap_or(u64::MAX),
                );
                live.push((Arc::clone(&lane), key, snapshot.lane_id.clone()));
            }
        }
        let mut lane_samples = HashMap::new();
        for (lane, key, lane_id) in live {
            let Some(host_rss) = rss_by_host.get(&key).copied() else {
                continue;
            };
            let lane_count = lane_count_by_host.get(&key).copied().unwrap_or(1).max(1);
            let sample = host_rss.saturating_add(lane_count - 1) / lane_count;
            lane_samples.insert(lane_id, sample);
            let mut snapshot = lane.snapshot.write().await;
            snapshot.resource_estimate_bytes = crate::resource::next_lane_resource_ewma(
                snapshot.resource_estimate_bytes,
                sample,
                policy.lane_ewma_min_bytes,
                policy.lane_ewma_max_bytes,
            );
        }
        *self.inner.lane_memory_samples.write().await = lane_samples;

        // Attribute each measured Host exactly once across trusted tasks.
        // Native renderer memory has no exact CDP target-to-RSS mapping. Give
        // every task an equal share of half the Host before using Lane/tab
        // activity to divide the remainder. The baseline prevents a sibling
        // with many empty tabs from diluting a genuinely heavy one-tab task to
        // a negligible estimate, while exact conservation prevents the shared
        // Host from being charged more than once.
        let mut task_samples = HashMap::<String, TaskMemoryAttribution>::new();
        for (key, task_activity) in task_activity_by_host {
            let Some(host_rss) = rss_by_host.get(&key).copied() else {
                continue;
            };
            let exclusive = task_activity.len() == 1;
            let mut ordered_activity = task_activity.into_iter().collect::<Vec<_>>();
            ordered_activity.sort_by(|left, right| left.0.cmp(&right.0));
            let mut assigned = 0u64;
            let task_count = ordered_activity.len();
            let task_count_u64 = u64::try_from(task_count).unwrap_or(u64::MAX).max(1);
            let baseline_pool = host_rss / 2;
            let baseline_share = baseline_pool / task_count_u64;
            let variable_pool = host_rss.saturating_sub(baseline_pool);
            let total_weight = ordered_activity
                .iter()
                .map(|(_, activity)| activity.variable_weight())
                .fold(0u64, u64::saturating_add)
                .max(1);
            for (index, (task_id, activity)) in ordered_activity.into_iter().enumerate() {
                let variable_numerator = u128::from(variable_pool)
                    .saturating_mul(u128::from(activity.variable_weight()));
                let share = if index + 1 == task_count {
                    host_rss.saturating_sub(assigned)
                } else {
                    baseline_share.saturating_add(
                        variable_numerator
                        .saturating_div(u128::from(total_weight))
                        .min(u128::from(u64::MAX)) as u64,
                    )
                };
                assigned = assigned.saturating_add(share);
                let entry = task_samples.entry(task_id).or_insert(TaskMemoryAttribution {
                    shared_rss_estimate_bytes: 0,
                    exclusive_hosts_only: true,
                });
                entry.shared_rss_estimate_bytes =
                    entry.shared_rss_estimate_bytes.saturating_add(share);
                entry.exclusive_hosts_only &= exclusive;
            }
        }
        *self.inner.task_memory_samples.write().await = task_samples;
    }

    /// Authoritative periodic cleanup.  The application should call this every
    /// 30 seconds and also call the explicit owner/runtime cleanup methods at
    /// their lifecycle boundaries.
    pub async fn sweep(&self) -> Result<CloseResult, BrowserPlatformError> {
        let mut closed = 0;
        let mut first_error = None;
        self.process_abandoned_restart_slots().await;
        self.process_abandoned_lane_starts().await;
        // Anonymous profile age and disk growth are Host lifecycle boundaries,
        // not idle-Lane heuristics. Sample them on every authoritative sweep so
        // a silent page/profile is still fenced and rotated without requiring
        // another browser operation.
        self.sweep_anonymous_profile_hygiene().await;
        self.sweep_primary_profile_hygiene().await;
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
        // The lease registry is the current lifecycle authority. A Lane keeps
        // the immutable caller snapshot from admission for diagnostics, so
        // its embedded capability expiry may legitimately predate a later
        // renew+bind. Re-validating that historical snapshot here would tear
        // down a healthy renewed owner and churn its Host/profile.
        let expired_owner_lease_ids = self.inner.owner_leases.sweep_expired_ids();
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
                // Freezing is a cheap first response, but a frozen browser
                // target still owns renderer memory. If pressure remains on a
                // later sweep, reclaim lanes that were already frozen before
                // this observation. Newly frozen lanes get one grace sweep so
                // a short pressure spike does not destroy useful state.
                let records: Vec<_> =
                    self.inner.lanes.read().await.values().cloned().collect();
                let mut preexisting_frozen = Vec::new();
                for lane in records {
                    let snapshot = lane.current_snapshot().await;
                    if snapshot.lifecycle_state == LaneLifecycleState::Frozen
                        && (lane.priority == LanePriority::Expansion
                            || is_crawl_identity(snapshot.identity_mode))
                    {
                        preexisting_frozen.push((
                            lane.frozen_at_ms.load(Ordering::Acquire),
                            snapshot.created_at_ms,
                            snapshot.lane_id,
                        ));
                    }
                }
                preexisting_frozen.sort();
                let mut lanes = self.list_lanes().await;
                lanes.sort_by_key(|lane| (lane.last_active_at_ms, lane.created_at_ms));
                for lane in &lanes {
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
                for (_, _, lane_id) in preexisting_frozen {
                    accumulate_close_outcome(
                        self.close_idle_lane_if_eligible(
                            &lane_id,
                            now,
                            0,
                            PressureCloseFilter::FrozenPressureReclaim,
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
        let cached_result = { self.inner.shutdown_result.read().await.clone() };
        if let Some(result) = cached_result {
            return result;
        }
        let (flight, leader) = {
            let mut active = self
                .inner
                .shutdown_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(flight) = active.clone() {
                (flight, false)
            } else {
                let flight = Arc::new(ShutdownFlight::new());
                *active = Some(Arc::clone(&flight));
                (flight, true)
            }
        };
        if leader {
            let hub = self.clone();
            let run_flight = Arc::clone(&flight);
            tokio::spawn(async move {
                // The platform attempt is Hub-owned. Caller timeout or HTTP
                // cancellation only stops waiting; it never cancels physical
                // Lane/Host teardown halfway through its authority transfer.
                let result = AssertUnwindSafe(hub.shutdown_once())
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(shutdown_operation_panicked_error()));
                {
                    let mut active = hub
                        .inner
                        .shutdown_flight
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if active
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &run_flight))
                    {
                        *active = None;
                    }
                }
                run_flight.complete(result);
            });
        }
        match tokio::time::timeout(PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT, flight.wait()).await {
            Ok(result) => result,
            Err(_) => {
                self.emit("platform_shutdown_cleanup_pending", None);
                Err(platform_shutdown_timeout_error())
            }
        }
    }

    async fn shutdown_once(&self) -> Result<(), BrowserPlatformError> {
        let _shutdown_guard = self.inner.shutdown_gate.lock().await;
        if let Some(result) = self.inner.shutdown_result.read().await.clone() {
            return result;
        }
        self.inner.shutting_down.store(true, Ordering::Release);
        // Close lease issuance atomically with respect to the lease registry.
        // This is terminal even when process cleanup needs another shutdown
        // attempt: no capability may be issued or renewed behind the drain.
        {
            let _task_lifecycle = self
                .inner
                .task_lifecycle_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.inner.owner_leases.stop_accepting_and_clear();
        }
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
        // Exact process/Lane cleanup has completed and owner issuance is
        // permanently closed, so no active reservation can legitimately
        // survive this installation boundary.
        {
            let _task_lifecycle = self
                .inner
                .task_lifecycle_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.inner.task_download_authority.clear();
        }
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

fn host_launch_requires_retirement(error: &BrowserPlatformError) -> bool {
    (error
        .metadata
        .get("host_initialization_timeout")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        && error.metadata.get("phase").and_then(serde_json::Value::as_str) == Some("launch"))
        || error
            .metadata
            .get("host_launch_cleanup_pending")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
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

fn visibility_operation_panicked_error(scope: &'static str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser display transition terminated unexpectedly.",
        true,
        "Refresh browser status and retry the display-mode transition.",
    )
    .with_metadata(json!({
        "visibility_transition_failed": true,
        "scope": scope,
        "task_cancelled": false,
        "task_panicked": true,
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

    /// Stable, trusted runtime cleanup scope captured when this client was bound.
    pub fn task_resource_key(&self) -> String {
        self.caller.task_resource_key()
    }

    /// Stable user-visible task family used only for resource quotas.
    pub fn task_resource_family_key(&self) -> crate::TaskResourceFamilyKey {
        self.caller.task_resource_family_key()
    }

    /// Durable runtime-cleanup handoff for an already-bound owner.
    ///
    /// This intentionally bypasses capability-expiry and operation checks:
    /// teardown authority must remain usable after the runtime lease becomes
    /// stale. Scope cannot be broadened because the exact owner lease id was
    /// sealed into this client by [`BrowserSessionHub::bind`]. The lease is
    /// preserved so a still-live runtime can open a fresh Lane after its own
    /// cleanup completes.
    pub async fn cleanup_bound_owner_lanes(
        &self,
    ) -> Result<CloseResult, BrowserPlatformError> {
        self.hub
            .close_owner_lanes(&self.caller.owner_lease_id)
            .await
    }

    /// Synchronously transfers cleanup authority for one exact Lane to the
    /// Hub's independent supervisor. The sealed owner generation and runtime
    /// attribution are captured from this bound client; capability expiry is
    /// intentionally irrelevant to teardown.
    ///
    /// This request can never include a Lane admitted after the handoff and
    /// never broadens when the ledger is saturated.
    pub fn handoff_bound_lane_cleanup(
        &self,
        lane_id: BrowserLaneId,
    ) -> Result<(), BrowserPlatformError> {
        self.hub.handoff_exact_lane_cleanup(
            lane_id,
            ExactLaneCleanupAuthority {
                user_id: self.caller.user_id.clone(),
                owner_lease_id: self.caller.owner_lease_id.clone(),
                task_id: self.task_resource_key(),
                family_id: self.task_resource_family_key().into_string(),
            },
        )
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

    /// Close only ordinary turn-scoped Lanes. Explicitly kept-alive Lanes
    /// survive a turn boundary; owner/runtime teardown still reclaims them.
    pub async fn close_turn_lanes(&self) -> Result<CloseResult, BrowserPlatformError> {
        self.hub
            .require_operation(&self.caller, BrowserOperationKind::Manage)?;
        self.hub
            .close_turn_lanes(&self.caller.owner_lease_id)
            .await
    }

    /// Set the trusted host's long-lived Lane intent.
    pub async fn set_keep_alive(
        &self,
        lane_id: &BrowserLaneId,
        keep_alive: bool,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.hub
            .set_keep_alive(&self.caller, lane_id, keep_alive)
            .await
    }

    /// Reports this Agent's presentation intent for one of its Lanes.
    ///
    /// Gated on `Manage` like the other Lane-lifecycle operations: surfacing a
    /// window is a visible change to the user's desktop, not a page interaction.
    pub async fn apply_presentation_intent(
        &self,
        lane_id: &BrowserLaneId,
        intent: BrowserPresentationIntent,
    ) -> Result<BrowserLaneSnapshot, BrowserPlatformError> {
        self.hub
            .require_operation(&self.caller, BrowserOperationKind::Manage)?;
        self.hub
            .apply_lane_presentation_intent(&self.caller, lane_id, intent)
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
        // `reload` is deliberately absent from the safe list: the current
        // history entry may have been produced by a POST, and an empty reload
        // input cannot prove that replaying it is side-effect free.
        BrowserOperationKind::Navigate => {
            !matches!(
                operation.action.as_str(),
                "navigate" | "back" | "forward"
            ) || operation_declares_stateful_request(&operation.input)
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

fn anonymous_profile_hygiene_error(
    key: &HostKey,
    reason: &'static str,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserRestarted,
        "The shared Anonymous browser reached its bounded profile lifecycle and is being rotated.",
        true,
        "Retry after the exact Anonymous browser Host finishes cleanup.",
    )
    .with_metadata(json!({
        "identity_mode": key.identity_mode,
        "profile_hygiene": reason,
        "shared_host_boundary": true,
    }))
}

fn primary_profile_storage_limit_error(
    fence: PrimaryProfileFence,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::PrimaryProfileStorageLimit,
        "The managed Primary browser profile reached its safe storage boundary and Primary browsing was stopped. Sign-in data was preserved.",
        false,
        "Clean the managed Primary site data or sign in again, then restart the application.",
    )
    .with_metadata(json!({
        "identity_mode": BrowserIdentityMode::Primary,
        "profile_hygiene": fence.reason,
        "primary_profile_fenced": true,
        "trigger_browser_epoch": fence.trigger_epoch,
        "persistent_identity_preserved": true,
        "automatic_profile_deletion": false,
    }))
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

fn operation_admission_busy_error(
    lane_id: BrowserLaneId,
    scope: &'static str,
    global_limit: usize,
    task_limit: usize,
    lane_limit: usize,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The browser operation queue is at capacity.",
        true,
        "Wait for an in-flight browser operation to finish, then retry.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "reason_code": "browser_operation_capacity_busy",
        "capacity_scope": scope,
        "global_queued_and_active_limit": global_limit,
        "task_queued_and_active_limit": task_limit,
        "lane_queued_and_active_limit": lane_limit,
        "retry_delay_ms": 250,
    }))
}

fn operation_queue_wait_timeout_error(
    lane_id: Option<BrowserLaneId>,
) -> BrowserPlatformError {
    let error = BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The browser operation could not start before its queue deadline.",
        true,
        "Retry after earlier browser work finishes.",
    )
    .with_metadata(json!({
        "reason_code": "browser_operation_queue_timeout",
        "timeout_ms": OPERATION_QUEUE_WAIT_TIMEOUT.as_millis() as u64,
        "retry_delay_ms": 250,
    }));
    lane_id.map_or(error.clone(), |lane_id| error.for_lane(lane_id))
}

fn invalid_tab_reservation_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser target reservation was invalid.",
        false,
        "Close the affected browser Lane and open a fresh one.",
    )
    .with_metadata(json!({
        "reason_code": "browser_tab_reservation_invalid",
    }))
}

fn task_tab_capacity_error(
    task_resource_key: &str,
    max_task_tabs: usize,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "This browser task has reached its top-level tab limit.",
        true,
        "Close an existing tab in this task, then retry.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_tab_capacity",
        "capacity_scope": "task",
        "task_resource_key": task_resource_key,
        "max_task_tabs": max_task_tabs,
        "retry_delay_ms": 250,
    }))
}

fn task_download_invalid_reservation_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser download reservation was invalid.",
        false,
        "Close the affected browser Lane and open a fresh one.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_reservation_invalid",
        "capacity_scope": "task",
    }))
}

fn task_download_owner_binding_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The browser owner does not match its sealed download resource family.",
        false,
        "Request a fresh browser capability for the logical task.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_owner_binding_mismatch",
        "capacity_scope": "task",
    }))
}

fn task_download_authority_retired_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser task download authority has already retired.",
        false,
        "Stop the stale download and request a fresh browser capability.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_authority_retired",
        "capacity_scope": "task",
    }))
}

fn task_download_capacity_error(
    boundary: &'static str,
    attempted_bytes: u64,
    limit_bytes: u64,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "This browser task reached its download byte boundary.",
        false,
        "Use the files already produced by this task or start a new task.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_byte_capacity",
        "capacity_scope": "task",
        "boundary": boundary,
        "attempted_bytes": attempted_bytes,
        "limit_bytes": limit_bytes,
        "completed_bytes_are_not_refunded": true,
    }))
}

fn task_download_file_capacity_error(attempted_files: usize) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "This browser task reached its completed download file limit.",
        false,
        "Use the files already produced by this task or start a new task.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_file_capacity",
        "capacity_scope": "task",
        "attempted_files": attempted_files,
        "max_completed_files": MAX_TASK_COMPLETED_DOWNLOAD_FILES,
        "completed_files_are_not_refunded": true,
    }))
}

fn task_download_family_capacity_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The browser download ledger reached its retained task-family boundary.",
        false,
        "Restart the browser application only after active tasks have been safely finalized.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_family_capacity",
        "capacity_scope": "browser_hub",
        "max_retained_completed_families": MAX_RETAINED_COMPLETED_DOWNLOAD_FAMILIES,
        "ttl_or_lru_eviction_allowed": false,
    }))
}

fn task_download_active_capacity_error(attempted_active: usize) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "This browser task has too many downloads in progress.",
        true,
        "Wait for an in-progress download to finish or cancel, then retry.",
    )
    .with_metadata(json!({
        "reason_code": "browser_task_download_active_capacity",
        "capacity_scope": "task",
        "attempted_active": attempted_active,
        "max_active_downloads": MAX_TASK_ACTIVE_DOWNLOADS,
        "retry_delay_ms": 250,
    }))
}

fn cleanup_budget_capacity_error(
    task_id: &str,
    host_key: &HostKey,
    scope: CleanupBudgetScope,
    saturation: CleanupBudgetSaturation,
) -> BrowserPlatformError {
    let capacity_scope = match scope {
        CleanupBudgetScope::Global => "global",
        CleanupBudgetScope::Task => "task",
        CleanupBudgetScope::Family => "family",
        CleanupBudgetScope::Host => "host",
    };
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "Browser cleanup is saturated, so new physical browser work is paused.",
        true,
        "Wait for this cleanup scope to drain below its low-water mark, then retry.",
    )
    .with_metadata(json!({
        "reason_code": "browser_cleanup_budget_saturated",
        "cleanup_pending": true,
        "cleanup_ledger_reconcile_requested": true,
        "capacity_scope": capacity_scope,
        "task_resource_key": task_id,
        "identity_mode": host_key.identity_mode,
        "cleanup_authority_count": saturation.count,
        "cleanup_authority_requested_units": saturation.requested_units,
        "cleanup_authority_hard_max": saturation.hard_max,
        "cleanup_authority_low_water": saturation.low_water,
        "cleanup_fence_latched": saturation.latched,
        "retry_delay_ms": 1_000,
    }))
}

fn exact_lane_cleanup_authority_mismatch_error(
    lane_id: &BrowserLaneId,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::InvalidCallerIdentity,
        "The exact browser lane cleanup authority does not match its retained owner.",
        false,
        "Discard the stale cleanup handoff; never broaden it to another Lane.",
    )
    .for_lane(lane_id.clone())
    .with_metadata(json!({
        "reason_code": "browser_exact_lane_cleanup_authority_mismatch",
        "cleanup_pending": true,
    }))
}

fn exact_lane_cleanup_capacity_error(
    lane_id: &BrowserLaneId,
    global_count: usize,
    task_count: usize,
    family_count: usize,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The exact browser lane cleanup ledger reached its structural admission bound.",
        true,
        "Wait for exact Lane cleanup to converge before admitting more browser work.",
    )
    .for_lane(lane_id.clone())
    .with_metadata(json!({
        "reason_code": "browser_exact_lane_cleanup_capacity",
        "cleanup_pending": true,
        "cleanup_scope": "exact_lane",
        "global_count": global_count,
        "global_hard_max": MAX_EXACT_LANE_CLEANUP_HANDOFFS,
        "task_count": task_count,
        "family_count": family_count,
        "task_hard_max": MAX_TASK_EXACT_LANE_CLEANUP_HANDOFFS,
    }))
}

fn cleanup_budget_invariant_error(
    error: CleanupBudgetError,
    task_id: &str,
    host_key: &HostKey,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "Browser cleanup authority accounting rejected an inconsistent reservation.",
        false,
        "Close the affected browser task and inspect lifecycle diagnostics.",
    )
    .with_metadata(json!({
        "reason_code": "browser_cleanup_budget_invariant",
        "cleanup_pending": true,
        "task_resource_key": task_id,
        "identity_mode": host_key.identity_mode,
        "ledger_error": error.to_string(),
    }))
}

fn policy_reconciliation_pending_error(
    error: BrowserPlatformError,
    closed: usize,
) -> BrowserPlatformError {
    let mut metadata = error.metadata.as_object().cloned().unwrap_or_default();
    metadata.insert("cleanup_pending".to_owned(), json!(true));
    metadata.insert("policy_reconciliation_pending".to_owned(), json!(true));
    metadata.insert("reconciled_lane_count".to_owned(), json!(closed));
    BrowserPlatformError {
        metadata: serde_json::Value::Object(metadata),
        ..error
    }
}

fn task_tab_reconciliation_pending_error(
    task_id: &str,
    remaining_tabs: Option<usize>,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser task tab limit is still converging.",
        true,
        "Retry after retained browser target cleanup finishes.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "policy_reconciliation_pending": true,
        "reason_code": "browser_task_tab_reconciliation_pending",
        "task_resource_key": task_id,
        "remaining_task_tab_reservations": remaining_tabs,
    }))
}

fn policy_starting_lane_wait_timeout_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "A Starting browser Lane did not publish its tab route before policy reconciliation timed out.",
        true,
        "Retry the policy update after the retained Lane start settles.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "policy_reconciliation_pending": true,
        "reason_code": "browser_policy_starting_lane_pending",
        "timeout_ms": (HOST_INITIALIZATION_GATE_TIMEOUT + HOST_LANE_OPEN_TIMEOUT)
            .as_millis() as u64,
    }))
}

fn policy_tab_driver_reconciliation_error(
    error: BrowserPlatformError,
    task_id: &str,
) -> BrowserPlatformError {
    let mut metadata = error.metadata.as_object().cloned().unwrap_or_default();
    metadata.insert("cleanup_pending".to_owned(), json!(true));
    metadata.insert("policy_reconciliation_pending".to_owned(), json!(true));
    metadata.insert(
        "reason_code".to_owned(),
        json!("browser_task_tab_driver_reconciliation_pending"),
    );
    metadata.insert("task_resource_key".to_owned(), json!(task_id));
    BrowserPlatformError {
        metadata: serde_json::Value::Object(metadata),
        ..error
    }
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

fn host_open_lane_timeout_error(
    lane_id: BrowserLaneId,
    browser_epoch: u64,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host did not finish opening the lane before its deadline.",
        true,
        "Retry opening the lane after Host recovery finishes.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "host_open_lane_timeout": true,
        "browser_epoch": browser_epoch,
        "timeout_ms": HOST_LANE_OPEN_TIMEOUT.as_millis() as u64,
        "host_recovery_started": true,
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

fn lane_cleanup_hard_timeout_error(
    lane_id: BrowserLaneId,
    escalation_error: BrowserPlatformError,
) -> BrowserPlatformError {
    let mut metadata = escalation_error
        .metadata
        .as_object()
        .cloned()
        .unwrap_or_default();
    metadata.insert("cleanup_pending".to_owned(), json!(true));
    metadata.insert("cleanup_hard_timeout".to_owned(), json!(true));
    metadata.insert(
        "timeout_ms".to_owned(),
        json!(LANE_CLEANUP_HARD_TIMEOUT.as_millis() as u64),
    );
    BrowserPlatformError {
        lane_id: Some(lane_id),
        metadata: serde_json::Value::Object(metadata),
        ..escalation_error
    }
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

fn host_launch_task_panicked_error(browser_epoch: u64) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host launch task terminated unexpectedly.",
        true,
        "Retry after retained browser cleanup finishes.",
    )
    .with_metadata(json!({
        "browser_epoch": browser_epoch,
        "host_launch_task_panicked": true,
        "task_panicked": true,
        "task_cancelled": false,
    }))
}

fn host_launch_cleanup_pending_error(
    browser_epoch: u64,
    source: Option<BrowserPlatformError>,
) -> BrowserPlatformError {
    let mut error = source.unwrap_or_else(|| {
        BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "The managed browser Host launch cleanup is still pending.",
            true,
            "Retry after retained exact browser cleanup finishes.",
        )
    });
    let mut metadata = error.metadata.as_object().cloned().unwrap_or_default();
    metadata.insert("cleanup_pending".to_owned(), json!(true));
    metadata.insert("browser_epoch".to_owned(), json!(browser_epoch));
    metadata.insert("host_launch_cleanup_pending".to_owned(), json!(true));
    error.metadata = serde_json::Value::Object(metadata);
    error
}

fn host_launch_publication_invariant_error(browser_epoch: u64) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser Host launch did not publish one authoritative driver.",
        true,
        "Retry after retained browser cleanup finishes.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "browser_epoch": browser_epoch,
        "host_launch_publication_invariant": true,
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

fn rebind_cleanup_pending_error(key: &HostKey) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "The replacement browser Host is still under authoritative cleanup.",
        true,
        "Retry after the retained Host cleanup completes.",
    )
    .with_metadata(json!({
        "reason_code": "browser_host_rebind_cleanup_pending",
        "cleanup_pending": true,
        "identity_mode": key.identity_mode,
        "retry_delay_ms": 1_000,
    }))
}

fn rebind_lane_cleanup_pending_error(
    lane_id: BrowserLaneId,
    browser_epoch: u64,
) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "A previous physical driver for this browser Lane is still under cleanup.",
        true,
        "Retry after the exact Lane cleanup completes.",
    )
    .for_lane(lane_id)
    .with_metadata(json!({
        "reason_code": "browser_lane_rebind_cleanup_pending",
        "cleanup_pending": true,
        "browser_epoch": browser_epoch,
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

fn drain_operation_panicked_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The installation-wide browser cleanup task terminated unexpectedly.",
        true,
        "Retry closing all managed browser resources.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_drain_task_failed": true,
        "task_cancelled": false,
        "task_panicked": true,
    }))
}

fn policy_operation_panicked_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser resource-policy reconciliation task terminated unexpectedly.",
        true,
        "Retry the resource-policy update; retained cleanup remains fail-closed.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "policy_reconciliation_pending": true,
        "policy_task_failed": true,
        "task_cancelled": false,
        "task_panicked": true,
    }))
}

fn policy_reconciliation_busy_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserCapacityQueued,
        "Another browser resource-policy update is still reconciling.",
        true,
        "Retry this policy update after the active reconciliation completes.",
    )
    .with_metadata(json!({
        "policy_reconciliation_pending": true,
        "reason_code": "browser_policy_reconciliation_busy",
        "retry_delay_ms": 250,
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

fn shutdown_operation_panicked_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The browser platform shutdown task terminated unexpectedly.",
        true,
        "Retry shutdown; retained cleanup authority remains Hub-owned.",
    )
    .with_metadata(json!({
        "cleanup_pending": true,
        "platform_shutdown_task_panicked": true,
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
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::{Notify, Semaphore};

    use super::*;
    use crate::{
        BrowserProfileFootprint, BrowserSurface, BrowserTabSnapshot, HostLifecycleState,
        ManualClock, OwnerLeaseId,
    };

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
        host_shutdown_fail_from: AtomicUsize,
        host_shutdown_panics_remaining: AtomicUsize,
        block_host_shutdown: AtomicBool,
        host_shutdown_release: Semaphore,
        host_shutdown_changed: Notify,
        host_launch_panics_remaining: AtomicUsize,
        host_launch_failures_remaining: AtomicUsize,
        defer_failed_host_launch_cleanup: AtomicBool,
        deferred_host_launch_cleanup_leases: std::sync::Mutex<Vec<HostLaunchCleanupLease>>,
        block_host_launch: AtomicBool,
        host_launch_release: Semaphore,
        host_launch_changed: Notify,
        block_open_lane: AtomicBool,
        open_lane_panics_remaining: AtomicUsize,
        open_lane_release: Semaphore,
        open_lane_changed: Notify,
        open_lane_calls: AtomicUsize,
        open_lane_failure_at: AtomicUsize,
        lane_launch_tab_limits: std::sync::Mutex<Vec<usize>>,
        tab_reconcile_limits: std::sync::Mutex<Vec<(String, usize)>>,
        workspace_hints: std::sync::Mutex<Vec<Option<String>>>,
        host_launch_requests: std::sync::Mutex<Vec<RecordedHostLaunchRequest>>,
        profile_footprint_bytes: AtomicU64,
        profile_footprint_entries: AtomicU64,
        profile_footprint_limit_reached: AtomicBool,
        profile_footprint_none: AtomicBool,
        profile_footprint_fail: AtomicBool,
        profile_footprint_panics_remaining: AtomicUsize,
        profile_footprint_calls: AtomicUsize,
        profile_footprint_active: AtomicUsize,
        profile_footprint_maximum: AtomicUsize,
        block_profile_footprint: AtomicBool,
        profile_footprint_release: Semaphore,
        profile_footprint_changed: Notify,
        identity_capture: std::sync::Mutex<Option<CapturedIdentitySnapshot>>,
        fail_identity_capture: AtomicBool,
        agent_snapshot_release: Semaphore,
        operation_results: std::sync::Mutex<HashMap<String, BrowserOperationResult>>,
    }

    #[derive(Clone)]
    struct RecordedHostLaunchRequest {
        identity_mode: BrowserIdentityMode,
        identity_generation: u64,
        identity_snapshot_payload: Option<IdentitySnapshotPayload>,
        headful: bool,
    }

    impl From<&HostLaunchRequest> for RecordedHostLaunchRequest {
        fn from(request: &HostLaunchRequest) -> Self {
            Self {
                identity_mode: request.identity_mode,
                identity_generation: request.identity_generation,
                identity_snapshot_payload: request.identity_snapshot_payload.clone(),
                headful: request.headful,
            }
        }
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
                host_shutdown_fail_from: AtomicUsize::new(usize::MAX),
                host_shutdown_panics_remaining: AtomicUsize::new(0),
                block_host_shutdown: AtomicBool::new(false),
                host_shutdown_release: Semaphore::new(0),
                host_shutdown_changed: Notify::new(),
                host_launch_panics_remaining: AtomicUsize::new(0),
                host_launch_failures_remaining: AtomicUsize::new(0),
                defer_failed_host_launch_cleanup: AtomicBool::new(false),
                deferred_host_launch_cleanup_leases: std::sync::Mutex::new(Vec::new()),
                block_host_launch: AtomicBool::new(false),
                host_launch_release: Semaphore::new(0),
                host_launch_changed: Notify::new(),
                block_open_lane: AtomicBool::new(false),
                open_lane_panics_remaining: AtomicUsize::new(0),
                open_lane_release: Semaphore::new(0),
                open_lane_changed: Notify::new(),
                open_lane_calls: AtomicUsize::new(0),
                open_lane_failure_at: AtomicUsize::new(0),
                lane_launch_tab_limits: std::sync::Mutex::new(Vec::new()),
                tab_reconcile_limits: std::sync::Mutex::new(Vec::new()),
                workspace_hints: std::sync::Mutex::new(Vec::new()),
                host_launch_requests: std::sync::Mutex::new(Vec::new()),
                profile_footprint_bytes: AtomicU64::new(0),
                profile_footprint_entries: AtomicU64::new(0),
                profile_footprint_limit_reached: AtomicBool::new(false),
                profile_footprint_none: AtomicBool::new(false),
                profile_footprint_fail: AtomicBool::new(false),
                profile_footprint_panics_remaining: AtomicUsize::new(0),
                profile_footprint_calls: AtomicUsize::new(0),
                profile_footprint_active: AtomicUsize::new(0),
                profile_footprint_maximum: AtomicUsize::new(0),
                block_profile_footprint: AtomicBool::new(false),
                profile_footprint_release: Semaphore::new(0),
                profile_footprint_changed: Notify::new(),
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

    struct ActiveProfileFootprintCall(Arc<Probe>);

    impl ActiveProfileFootprintCall {
        fn enter(probe: &Arc<Probe>) -> Self {
            let active = probe
                .profile_footprint_active
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1);
            probe
                .profile_footprint_maximum
                .fetch_max(active, Ordering::AcqRel);
            probe.profile_footprint_changed.notify_waiters();
            Self(Arc::clone(probe))
        }
    }

    impl Drop for ActiveProfileFootprintCall {
        fn drop(&mut self) {
            self.0
                .profile_footprint_active
                .fetch_sub(1, Ordering::AcqRel);
            self.0.profile_footprint_changed.notify_waiters();
        }
    }

    struct FakeLane {
        probe: Arc<Probe>,
        _initial_tab_reservation: Arc<dyn BrowserTaskTabReservation>,
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

        async fn profile_footprint(
            &self,
            _stop_after_bytes: u64,
            _stop_after_entries: u64,
        ) -> Result<Option<BrowserProfileFootprint>, BrowserPlatformError> {
            self.probe
                .profile_footprint_calls
                .fetch_add(1, Ordering::AcqRel);
            let _active = ActiveProfileFootprintCall::enter(&self.probe);
            self.probe.profile_footprint_changed.notify_waiters();
            if self.probe.block_profile_footprint.load(Ordering::Acquire) {
                let permit = self
                    .probe
                    .profile_footprint_release
                    .acquire()
                    .await
                    .map_err(|_| BrowserPlatformError::shutting_down())?;
                permit.forget();
            }
            if self
                .probe
                .profile_footprint_panics_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("synthetic profile footprint panic");
            }
            if self.probe.profile_footprint_fail.load(Ordering::Acquire) {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic profile footprint failure.",
                    true,
                    "Rotate the synthetic Host.",
                ));
            }
            if self.probe.profile_footprint_none.load(Ordering::Acquire) {
                return Ok(None);
            }
            Ok(Some(BrowserProfileFootprint {
                bytes: self.probe.profile_footprint_bytes.load(Ordering::Acquire),
                entries: self
                    .probe
                    .profile_footprint_entries
                    .load(Ordering::Acquire),
                limit_reached: self
                    .probe
                    .profile_footprint_limit_reached
                    .load(Ordering::Acquire),
            }))
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
            self.probe
                .lane_launch_tab_limits
                .lock()
                .expect("Lane tab-limit probe poisoned")
                .push(request.max_task_tabs);
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
            let initial_tab_reservation = request
                .task_tab_authority
                .reserve(
                    &request.task_resource_key,
                    request.lane_id.as_str(),
                    "initial-tab",
                )
                .await?;
            Ok(Arc::new(FakeLane {
                probe: Arc::clone(&self.probe),
                _initial_tab_reservation: initial_tab_reservation,
            }))
        }

        async fn reconcile_task_tab_limit(
            &self,
            task_resource_key: &str,
            max_task_tabs: usize,
        ) -> Result<(), BrowserPlatformError> {
            self.probe
                .tab_reconcile_limits
                .lock()
                .expect("tab reconcile probe poisoned")
                .push((task_resource_key.to_owned(), max_task_tabs));
            Ok(())
        }

        async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
            let call = self.probe.host_shutdowns.fetch_add(1, Ordering::AcqRel) + 1;
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
                .host_shutdown_panics_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("synthetic host shutdown panic");
            }
            if call >= self.probe.host_shutdown_fail_from.load(Ordering::Acquire)
                || self
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
            request: LaneLaunchRequest,
        ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
            let initial_tab_reservation = request
                .task_tab_authority
                .reserve(
                    &request.task_resource_key,
                    request.lane_id.as_str(),
                    "initial-tab",
                )
                .await?;
            Ok(Arc::new(FakeLane {
                probe: Arc::clone(&self.probe),
                _initial_tab_reservation: initial_tab_reservation,
            }))
        }

        async fn reconcile_task_tab_limit(
            &self,
            _task_resource_key: &str,
            _max_task_tabs: usize,
        ) -> Result<(), BrowserPlatformError> {
            Ok(())
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
                .push(RecordedHostLaunchRequest::from(&request));
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
            if self
                .probe
                .host_launch_failures_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                if self
                    .probe
                    .defer_failed_host_launch_cleanup
                    .load(Ordering::Acquire)
                {
                    self.probe
                        .deferred_host_launch_cleanup_leases
                        .lock()
                        .expect("deferred Host launch cleanup probe poisoned")
                        .push(request.cleanup_lease.clone());
                }
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::BrowserUnavailable,
                    "Synthetic Host factory failure.",
                    true,
                    "Retry the synthetic Host launch.",
                ));
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

    fn client_for_conversation(
        harness: &Harness,
        conversation_id: &str,
        runtime_instance_id: &str,
    ) -> BrowserLaneClient {
        let owner = harness
            .hub
            .issue_owner_lease(
                "user-1",
                Some(conversation_id.to_owned()),
                runtime_instance_id,
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.conversation_id = Some(conversation_id.to_owned());
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

    fn saturate_task_cleanup_lanes(harness: &Harness) -> (String, Vec<BrowserLaneId>) {
        let task_id = harness.client.caller().task_resource_key();
        let family_id = harness
            .client
            .caller()
            .task_resource_family_key()
            .into_string();
        let accounting_host = HostKey::for_lane(
            BrowserIdentityMode::Primary,
            0,
            &BrowserLaneId::new(),
        );
        let mut lane_ids = Vec::with_capacity(
            crate::cleanup_budget::CLEANUP_BUDGET_TASK_HARD_MAX,
        );
        for _ in 0..crate::cleanup_budget::CLEANUP_BUDGET_TASK_HARD_MAX {
            let lane_id = BrowserLaneId::new();
            harness
                .hub
                .reserve_cleanup_lane_for_existing_host(
                    &task_id,
                    &family_id,
                    &accounting_host,
                    &lane_id,
                )
                .expect("synthetic task cleanup saturation setup failed early");
            lane_ids.push(lane_id);
        }
        (task_id, lane_ids)
    }

    fn release_synthetic_task_cleanup_lanes(
        harness: &Harness,
        lane_ids: Vec<BrowserLaneId>,
    ) {
        for lane_id in lane_ids {
            harness.hub.release_lane_cleanup_budget(&lane_id);
        }
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
    async fn task_tab_authority_is_task_global_idempotent_and_releases_at_last_drop() {
        let authority = HubTaskTabAuthority::new(2);
        let first = authority
            .reserve("task-a", "lane-primary", "same-target-id")
            .await
            .unwrap();
        let duplicate = authority
            .reserve("task-a", "lane-primary", "same-target-id")
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &duplicate));
        assert_eq!(authority.count_for("task-a"), 1);

        // A target id is only process-local. The same value in another Lane/
        // Host is a distinct page, but both still consume the same task cap.
        let cross_host = authority
            .reserve("task-a", "lane-isolated", "same-target-id")
            .await
            .unwrap();
        assert_eq!(authority.count_for("task-a"), 2);
        let overflow = authority
            .reserve("task-a", "lane-replica", "third-target")
            .await
            .err()
            .expect("the task-global N+1 reservation must be rejected");
        assert_eq!(overflow.code, BrowserErrorCode::BrowserCapacityQueued);

        drop(first);
        assert_eq!(
            authority.count_for("task-a"),
            2,
            "a duplicate Arc must keep its one logical slot alive"
        );
        drop(duplicate);
        assert_eq!(authority.count_for("task-a"), 1);
        drop(cross_host);
        assert_eq!(authority.count_for("task-a"), 0);
        assert!(authority.task_counts().is_empty());
    }

    #[tokio::test]
    async fn lowering_tab_authority_fences_every_later_reservation() {
        let authority = HubTaskTabAuthority::new(3);
        let retained = authority
            .reserve("task-a", "lane-a", "retained")
            .await
            .unwrap();
        authority.set_limit(1);

        let attempts = (0..32).map(|index| {
            let authority = authority.clone();
            tokio::spawn(async move {
                authority
                    .reserve("task-a", "lane-a", &format!("late-{index}"))
                    .await
            })
        });
        for attempt in attempts {
            assert_eq!(
                attempt
                    .await
                    .unwrap()
                    .err()
                    .expect("a reservation after lowering must be rejected")
                    .code,
                BrowserErrorCode::BrowserCapacityQueued
            );
        }
        assert_eq!(authority.count_for("task-a"), 1);
        drop(retained);
        assert_eq!(authority.count_for("task-a"), 0);
    }

    #[tokio::test]
    async fn sibling_runtimes_share_lane_and_tab_family_quotas() {
        let mut config = HubConfig::default();
        config.resource_policy = ResourcePolicy::automatic(16 * crate::resource::GIB, 8);
        config.resource_policy.max_task_open_lanes = 1;
        let harness = harness_with_config(config);
        *harness.hub.inner.telemetry.write().await = ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 16 * crate::resource::GIB,
            ..ResourceTelemetry::default()
        };
        let sibling = client_for_runtime(&harness, "runtime-family-sibling");
        let first = open(&harness.client, "family-first").await;
        let sibling_lane = sibling
            .open(Some("family-sibling"), BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        assert_eq!(
            sibling_lane.lane().lifecycle_state,
            LaneLifecycleState::Queued,
            "a sibling runtime must not receive another active Lane quota"
        );
        let family_id = harness.client.task_resource_family_key().into_string();
        assert_eq!(harness.hub.inner.task_tab_authority.count_for(&family_id), 1);
        assert_eq!(
            harness
                .hub
                .inner
                .task_tab_authority
                .count_for(&harness.client.task_resource_key()),
            0,
            "tab authority must never be keyed by runtime cleanup identity"
        );
        harness.client.close(&first).await.unwrap();
    }

    #[tokio::test]
    async fn sibling_runtimes_share_operation_budget_but_other_conversations_do_not() {
        let mut config = HubConfig::default();
        config.resource_policy = ResourcePolicy::automatic(16 * crate::resource::GIB, 8);
        config.resource_policy.max_active_operations = 4;
        config.resource_policy.max_task_active_operations = 1;
        let harness = harness_with_config(config);
        *harness.hub.inner.telemetry.write().await = ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 16 * crate::resource::GIB,
            ..ResourceTelemetry::default()
        };
        let sibling = client_for_runtime(&harness, "runtime-operation-sibling");
        let other = client_for_conversation(
            &harness,
            "conversation-operation-other",
            "runtime-operation-other",
        );
        let other_lane = open(&other, "operation-family-other").await;
        let first_lane = open(&harness.client, "operation-family-first").await;
        let sibling_lane = open(&sibling, "operation-family-sibling").await;

        let first_client = harness.client.clone();
        let first = tokio::spawn(async move {
            first_client.execute(&first_lane, navigate()).await
        });
        harness.probe.wait_for_active(1).await;
        let sibling_client = sibling.clone();
        let sibling_operation = tokio::spawn(async move {
            sibling_client.execute(&sibling_lane, navigate()).await
        });
        assert!(
            tokio::time::timeout(
                Duration::from_millis(30),
                harness.probe.wait_for_entries(2),
            )
            .await
            .is_err(),
            "a sibling runtime bypassed its conversation operation budget"
        );

        let other_operation = tokio::spawn(async move {
            other.execute(&other_lane, navigate()).await
        });
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_active(2),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(2);
        first.await.unwrap().unwrap();
        other_operation.await.unwrap().unwrap();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_entries(3),
        )
        .await
        .unwrap();
        harness.probe.releases.add_permits(1);
        sibling_operation.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn sibling_runtime_owner_cleanup_is_exact_while_memory_is_family_scoped() {
        let harness = harness();
        let sibling = client_for_runtime(&harness, "runtime-cleanup-sibling");
        let first_lane = open(&harness.client, "cleanup-family-first").await;
        let sibling_lane = open(&sibling, "cleanup-family-sibling").await;
        let family_id = harness.client.task_resource_family_key().into_string();

        harness
            .hub
            .refresh_lane_resource_estimates(&ResourceTelemetry {
                total_memory_bytes: 16 * crate::resource::GIB,
                available_memory_bytes: 8 * crate::resource::GIB,
                host_rss_by_process_id: HashMap::from([(4_242, crate::resource::GIB)]),
                ..ResourceTelemetry::default()
            })
            .await;
        let samples = harness.hub.inner.task_memory_samples.read().await;
        assert_eq!(samples.len(), 1);
        assert!(samples.contains_key(&family_id));
        drop(samples);

        let result = harness.client.cleanup_bound_owner_lanes().await.unwrap();
        assert_eq!(result.closed, 1);
        assert!(harness.client.status(&first_lane).await.is_err());
        assert_eq!(
            sibling.status(&sibling_lane).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running,
            "runtime cleanup must not close a sibling in the same family"
        );
    }

    #[tokio::test]
    async fn family_cleanup_debt_blocks_expansion_without_closing_a_sibling() {
        let harness = harness();
        let sibling = client_for_runtime(&harness, "runtime-debt-sibling");
        let leaking = open(&harness.client, "family-debt-source").await;
        let healthy = open(&sibling, "family-debt-healthy").await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        assert!(harness.client.close(&leaking).await.is_err());

        let expansion = sibling
            .open(Some("family-debt-expansion"), BrowserIdentityMode::Primary, None)
            .await
            .unwrap();
        assert_eq!(
            expansion.lane().lifecycle_state,
            LaneLifecycleState::Queued,
            "one retained cleanup must fence later family expansion before the hard ledger cap"
        );
        assert_eq!(
            sibling.status(&healthy).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running,
            "family debt fencing must not close a healthy sibling runtime"
        );

        harness.hub.autonomous_cleanup_pass().await.unwrap();
        assert_eq!(
            sibling
                .status(&expansion.lane().lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Running,
            "successful autonomous cleanup must promote the family without an unrelated wakeup"
        );
        assert_eq!(
            sibling.status(&healthy).await.unwrap().lifecycle_state,
            LaneLifecycleState::Running,
            "debt recovery must preserve the healthy sibling runtime"
        );
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
        assert!(
            harness
                .probe
                .tab_reconcile_limits
                .lock()
                .expect("tab reconcile probe poisoned")
                .is_empty(),
            "an unchanged task-family tab cap must not make visibility replacement depend on the optional Host reconcile hook"
        );
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

    /// An Agent reporting an attended moment gets a visible window under `Auto`,
    /// and the escalation is bounded so a page that keeps tripping the risk
    /// classifier cannot restart Chromium in a loop.
    #[tokio::test]
    async fn attended_intent_escalates_once_per_allowance_then_declines() {
        let harness = harness();
        let lane_id = open(&harness.client, "attended-primary").await;
        let caller = harness.client.caller().clone();
        assert_eq!(
            harness.hub.visibility_policy().await,
            BrowserVisibilityPolicy::Auto,
            "Auto is the default policy"
        );

        // Routine work must never restart the browser to show a window.
        harness
            .hub
            .apply_lane_presentation_intent(
                &caller,
                &lane_id,
                BrowserPresentationIntent::Unattended,
            )
            .await
            .unwrap();
        assert!(
            !harness.hub.overview().await.hosts[0].headful,
            "unattended work must stay silent"
        );

        // An attended moment earns the window.
        harness
            .hub
            .apply_lane_presentation_intent(
                &caller,
                &lane_id,
                BrowserPresentationIntent::Attended,
            )
            .await
            .unwrap();
        assert!(
            harness.hub.overview().await.hosts[0].headful,
            "an attended moment on Primary must surface a window"
        );
        assert_eq!(
            harness.hub.primary_visibility().await,
            BrowserVisibility::Headless,
            "escalating one Lane must not mutate the installation default"
        );

        // Already visible: further reports are no-ops, not more restarts.
        let epoch_after_first =
            harness.client.status(&lane_id).await.unwrap().browser_epoch;
        for _ in 0..4 {
            harness
                .hub
                .apply_lane_presentation_intent(
                    &caller,
                    &lane_id,
                    BrowserPresentationIntent::Attended,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            harness.client.status(&lane_id).await.unwrap().browser_epoch,
            epoch_after_first,
            "repeated attended reports must not each replace the Host"
        );
    }

    /// A user who pinned silent is never overridden by the Agent.
    #[tokio::test]
    async fn attended_intent_cannot_override_an_explicit_silent_policy() {
        let mut config = HubConfig::default();
        config.visibility_policy = BrowserVisibilityPolicy::AlwaysHeadless;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "pinned-silent-primary").await;
        let caller = harness.client.caller().clone();

        let snapshot = harness
            .hub
            .apply_lane_presentation_intent(
                &caller,
                &lane_id,
                BrowserPresentationIntent::Attended,
            )
            .await
            .expect("a declined escalation is a normal outcome, not an error");

        assert!(
            !harness.hub.overview().await.hosts[0].headful,
            "AlwaysHeadless is a promise the model does not get to override"
        );
        assert_eq!(snapshot.lane_id, lane_id);
        assert_eq!(
            snapshot.error_code, None,
            "declining to escalate must not error the Lane"
        );
    }

    /// The escalation must not be reachable through a revoked owner.
    #[tokio::test]
    async fn presentation_intent_revalidates_the_caller() {
        let harness = harness();
        let lane_id = open(&harness.client, "revoked-presentation").await;
        let mut caller = harness.client.caller().clone();
        // An expired capability is a real revocation vector: a queued or
        // long-running Agent turn can outlive its grant.
        caller.capability_expires_at_ms = 0;

        let error = harness
            .hub
            .apply_lane_presentation_intent(
                &caller,
                &lane_id,
                BrowserPresentationIntent::Attended,
            )
            .await
            .unwrap_err();

        assert!(
            !harness.hub.overview().await.hosts[0].headful,
            "an unauthorized caller must not be able to open a window"
        );
        assert_ne!(error.code, BrowserErrorCode::TaskMemoryReclaimed);
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

    #[tokio::test(start_paused = true)]
    async fn timed_out_restart_retires_the_exact_published_replacement_epoch() {
        let harness = harness();
        let lane_id = open(&harness.client, "restart-publication-timeout").await;
        let before = harness.client.status(&lane_id).await.unwrap();
        let key = HostKey {
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 0,
            isolation_lane_id: None,
        };
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);

        let transition_hub = harness.hub.clone();
        let transition = tokio::spawn(async move {
            transition_hub
                .set_primary_visibility(BrowserVisibility::Headful)
                .await
        });
        harness.probe.wait_for_host_launches(2).await;
        let published_slot = harness
            .hub
            .inner
            .host_slots
            .read()
            .await
            .get(&key)
            .cloned()
            .expect("replacement slot was not published before its launch wait");
        assert!(published_slot.epoch > before.browser_epoch);
        assert!(harness
            .hub
            .inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .any(|authority| Arc::ptr_eq(&authority.slot, &published_slot)));

        tokio::time::advance(HOST_RESTART_ATTEMPT_TIMEOUT).await;
        tokio::task::yield_now().await;
        transition
            .await
            .expect("visibility restart task panicked")
            .expect_err("the bounded restart must time out");

        // The single-flight abort drops its provisional guard. Transfer that
        // exact Arc into durable cleanup; never rediscover a slot merely by
        // HostKey, where a newer epoch could hide or replace it.
        harness.hub.process_abandoned_restart_slots().await;
        assert!(published_slot.retired.load(Ordering::Acquire));
        assert!(harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(harness
            .hub
            .inner
            .orphaned_host_slots
            .lock()
            .await
            .iter()
            .any(|(pending_key, slot)| {
                pending_key == &key && Arc::ptr_eq(slot, &published_slot)
            }));
        assert!(!harness
            .hub
            .inner
            .host_slots
            .read()
            .await
            .get(&key)
            .is_some_and(|slot| Arc::ptr_eq(slot, &published_slot)));
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);

        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);
        harness.probe.host_launch_release.add_permits(1);
        tokio::task::yield_now().await;
        harness.hub.retry_orphaned_host_slots().await.unwrap();

        assert!(harness.hub.inner.orphaned_host_slots.lock().await.is_empty());
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(harness
            .hub
            .inner
            .published_restart_slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            2,
            "both the old Host and the late exact replacement must be proven stopped"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lane_created_is_emitted_before_the_start_task_is_spawned() {
        // Ordering regression pin: `open_lane` must publish `lane_created`
        // synchronously BEFORE spawning the start task. If the emit moves back
        // after the spawn, a start task scheduled on another worker can publish
        // `lane_running` first, so inventory subscribers would observe a lane
        // running before it was ever created. Blocking the Host launch parks
        // the start task at its earliest observable step; by that point the
        // creation event must already be in the channel.
        let harness = harness();
        let mut events = harness.hub.subscribe();
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);

        let client = harness.client.clone();
        let opener = tokio::spawn(async move {
            client
                .open(
                    Some("created-before-start"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });

        // The start task has been spawned and is parked inside the blocked
        // launch; `lane_created` must already be observable and no start
        // progress may have been published ahead of it.
        tokio::time::timeout(
            Duration::from_secs(5),
            harness.probe.wait_for_host_launches(1),
        )
        .await
        .expect("the lane start task never reached the Host launch");
        assert_eq!(
            events
                .try_recv()
                .expect("lane_created must be published before the start task runs")
                .change_kind,
            "lane_created"
        );

        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);
        harness.probe.host_launch_release.add_permits(1);
        let outcome = opener.await.unwrap().unwrap();
        assert!(matches!(outcome, OpenLaneOutcome::Running { .. }));
        assert_eq!(
            events.recv().await.unwrap().change_kind,
            "lane_running",
            "lane_running must strictly follow lane_created"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn crash_recovery_and_visibility_gate_holder_never_deadlock() {
        // Lock-order regression test: crash recovery must acquire
        // primary_visibility_gate BEFORE entering the restart single-flight.
        // If it registered its flight first and acquired the gate inside the
        // leader closure, the gate-holding transition below would join that
        // flight and both sides would wait on each other until the 75s
        // attempt timeout.
        let harness = harness();
        let lane_id = open(&harness.client, "gate-order").await;
        let before = harness.client.status(&lane_id).await.unwrap();

        // Simulate the visibility caller's critical section.
        let gate = harness.hub.inner.primary_visibility_gate.lock().await;
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);
        let client = harness.client.clone();
        let op_lane = lane_id.clone();
        let recovery =
            tokio::spawn(async move { client.execute(&op_lane, navigate()).await });
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }

        let key = HostKey {
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 0,
            isolation_lane_id: None,
        };
        harness
            .hub
            .transition_primary_visibility_locked(&key, before.browser_epoch, true)
            .await
            .expect("the gate holder's transition must not deadlock against crash recovery");
        drop(gate);

        let error = tokio::time::timeout(Duration::from_secs(5), recovery)
            .await
            .expect("crash recovery did not settle after the gate was released")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted);
        let current = harness.client.status(&lane_id).await.unwrap();
        assert_ne!(current.browser_epoch, before.browser_epoch);
        assert!(harness.hub.overview().await.hosts[0].headful);
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
    async fn failed_late_renew_cannot_hide_an_expired_owner_from_sweep() {
        let harness = harness_with_config_and_owner_ttl(HubConfig::default(), 10);
        let (client, lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-expired-before-sweep");
        let lane_id = open(&client, "expired-before-sweep").await;

        harness.clock.advance(10);
        assert_eq!(
            harness
                .hub
                .renew_owner_lease(&lease_id)
                .unwrap_err()
                .code,
            BrowserErrorCode::OwnerLeaseExpired
        );

        let result = harness.hub.sweep().await.unwrap();
        assert_eq!(result.closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&lane_id).await.is_none());
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn sweep_keeps_a_renewed_owner_past_its_initial_capability_expiry() {
        let harness = harness_with_config_and_owner_ttl(HubConfig::default(), 100);
        let owner = harness
            .hub
            .issue_owner_lease(
                "user-1",
                Some("conversation-renewed".to_owned()),
                "runtime-renewed",
            )
            .unwrap();
        let mut caller = harness.client.caller().clone();
        caller.conversation_id = Some("conversation-renewed".to_owned());
        caller.runtime_instance_id = "runtime-renewed".to_owned();
        caller.owner_lease_id = owner.lease_id.clone();
        caller.capability_expires_at_ms = harness.clock.now_ms() + 50;
        let initial_client = harness.hub.bind(caller.clone()).unwrap();
        let lane_id = open(&initial_client, "renewed-owner").await;
        let initial_epoch = initial_client.status(&lane_id).await.unwrap().browser_epoch;

        harness.clock.advance(40);
        let renewed = harness.hub.renew_owner_lease(&owner.lease_id).unwrap();
        caller.capability_expires_at_ms = renewed.expires_at_ms;
        let renewed_client = harness.hub.bind(caller).unwrap();

        // Cross the capability expiry stored in the Lane's original caller,
        // while remaining inside the authoritative renewed lease window.
        harness.clock.advance(20);
        harness.hub.sweep().await.unwrap();

        let snapshot = renewed_client.status(&lane_id).await.unwrap();
        assert_eq!(snapshot.browser_epoch, initial_epoch);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 0);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
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
        // Admit the sibling lanes before cleanup debt is created. Once exact
        // runtime cleanup is pending, fail-closed admission must reject new
        // work for that runtime instead of letting it outrun teardown.
        open(&client, "owner-count-a").await;
        open(&client, "owner-count-b").await;
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
            harness.probe.wait_for_lane_close_completions(1),
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
    async fn shared_primary_lane_churn_releases_exact_cleanup_tokens() {
        let harness = harness();
        let task_id = harness.client.caller().task_resource_key();
        let anchor = open(&harness.client, "cleanup-ledger-anchor").await;

        // This deliberately crosses the task ledger's hard limit. A closed
        // shared-Host Lane must release its exact token even though the
        // Primary Host remains alive for the anchor Lane.
        for _ in 0..140 {
            let transient = open(&harness.client, "cleanup-ledger-churn").await;
            harness.client.close(&transient).await.unwrap();
            let budget = harness.hub.inner.cleanup_budget.snapshot();
            assert_eq!(budget.lane_tokens, 1);
            assert_eq!(budget.host_tokens, 1);
            assert_eq!(budget.tasks[&task_id].count, 2);
            assert!(!budget.tasks[&task_id].latched);
        }

        harness.client.close(&anchor).await.unwrap();
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 0);
        assert_eq!(budget.host_tokens, 0);
        assert!(!budget.tasks.contains_key(&task_id));
        assert!(
            harness
                .hub
                .inner
                .lane_cleanup_budget_tokens
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
        assert!(
            harness
                .hub
                .inner
                .host_cleanup_budget_tokens
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cancelled_waiter_then_factory_error_releases_exact_cleanup_tokens() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);
        harness
            .probe
            .host_launch_failures_remaining
            .store(1, Ordering::Release);

        let client = harness.client.clone();
        let open_task = tokio::spawn(async move {
            client
                .open(
                    Some("cancelled-factory-error"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_host_launches(1).await;
        open_task.abort();
        assert!(open_task.await.unwrap_err().is_cancelled());

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if harness.hub.list_lanes().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled start waiter was not detached by the Hub supervisor");
        harness.probe.host_launch_release.add_permits(1);

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let budget = harness.hub.inner.cleanup_budget.snapshot();
                if budget.lane_tokens == 0 && budget.host_tokens == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("factory failure did not release exact cleanup tokens");
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 0);
        assert_eq!(budget.host_tokens, 0);
        assert!(harness.hub.inner.pending_lane_cleanups.lock().await.is_empty());
        assert!(harness.hub.inner.owner_cleanup_targets.lock().await.is_empty());
    }

    #[tokio::test]
    async fn ten_thousand_saturated_isolated_starts_do_not_accumulate_host_circuits() {
        let harness = harness();
        let (task_id, reserved_lane_ids) = saturate_task_cleanup_lanes(&harness);
        let circuit_baseline = harness.hub.inner.host_circuits.lock().await.len();

        for _ in 0..10_000 {
            let lane_id = BrowserLaneId::new();
            let result = harness
                .hub
                .get_or_launch_host(
                    BrowserIdentityMode::Isolated,
                    0,
                    &lane_id,
                    &task_id,
                    &task_id,
                )
                .await;
            assert!(result.is_err(), "saturated cleanup admission unexpectedly launched a Host");
        }

        assert_eq!(
            harness.hub.inner.host_circuits.lock().await.len(),
            circuit_baseline,
            "rejected unique Isolated ids must not allocate unowned circuit entries"
        );
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        release_synthetic_task_cleanup_lanes(&harness, reserved_lane_ids);
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 0);
        assert_eq!(budget.host_tokens, 0);
    }

    #[tokio::test]
    async fn draining_host_starts_do_not_allocate_unowned_circuits() {
        let harness = harness();
        let task_id = harness.client.caller().task_resource_key();
        let circuit_baseline = harness.hub.inner.host_circuits.lock().await.len();
        harness.hub.inner.draining.store(true, Ordering::Release);

        for _ in 0..128 {
            let lane_id = BrowserLaneId::new();
            let result = harness
                .hub
                .get_or_launch_host(
                    BrowserIdentityMode::Isolated,
                    0,
                    &lane_id,
                    &task_id,
                    &task_id,
                )
                .await;
            assert!(result.is_err());
        }

        harness.hub.inner.draining.store(false, Ordering::Release);
        assert_eq!(
            harness.hub.inner.host_circuits.lock().await.len(),
            circuit_baseline
        );
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn retirement_wait_timeout_does_not_allocate_an_unowned_circuit() {
        let harness = harness();
        let task_id = harness.client.caller().task_resource_key();
        let lane_id = BrowserLaneId::new();
        let key = HostKey::for_lane(BrowserIdentityMode::Isolated, 0, &lane_id);
        harness
            .hub
            .inner
            .retiring_host_keys
            .write()
            .await
            .insert(key.clone());
        let circuit_baseline = harness.hub.inner.host_circuits.lock().await.len();

        let hub = harness.hub.clone();
        let waiting = tokio::spawn(async move {
            hub.get_or_launch_host(
                BrowserIdentityMode::Isolated,
                0,
                &lane_id,
                &task_id,
                &task_id,
            )
            .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(HOST_RETIREMENT_WAIT_TIMEOUT).await;
        let result = waiting.await.expect("retirement waiter task panicked");
        assert!(result.is_err());
        assert_eq!(
            harness.hub.inner.host_circuits.lock().await.len(),
            circuit_baseline
        );

        harness.hub.inner.retiring_host_keys.write().await.remove(&key);
        harness.hub.inner.retiring_hosts_changed.notify_waiters();
    }

    #[tokio::test]
    async fn existing_host_reservation_failure_does_not_create_a_missing_circuit() {
        let harness = harness();
        let (task_id, reserved_lane_ids) = saturate_task_cleanup_lanes(&harness);
        let lane_id = BrowserLaneId::new();
        let key = HostKey::for_lane(BrowserIdentityMode::Isolated, 0, &lane_id);
        harness
            .hub
            .inner
            .host_slots
            .write()
            .await
            .insert(key.clone(), Arc::new(HostSlot::new(9_999, false, 0)));
        assert!(!harness.hub.inner.host_circuits.lock().await.contains_key(&key));

        let result = harness
            .hub
            .get_or_launch_host(
                BrowserIdentityMode::Isolated,
                0,
                &lane_id,
                &task_id,
                &task_id,
            )
            .await;
        assert!(result.is_err());
        assert!(!harness.hub.inner.host_circuits.lock().await.contains_key(&key));

        harness.hub.inner.host_slots.write().await.remove(&key);
        release_synthetic_task_cleanup_lanes(&harness, reserved_lane_ids);
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
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "a failed start with no attached sibling must not launch a replacement Host"
        );
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
        assert!(harness.hub.inner.owner_cleanup_targets.lock().await.is_empty());
        assert!(
            harness
                .hub
                .inner
                .host_stop_required_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
        let cleanup_budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(cleanup_budget.lane_tokens, 0);
        assert_eq!(cleanup_budget.host_tokens, 0);

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

    #[tokio::test(start_paused = true)]
    async fn host_open_lane_timeout_converges_flight_and_exact_host_recovery() {
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
                .open(Some("host-open-timeout"), BrowserIdentityMode::Primary, None)
                .await
        });
        loop {
            let notified = harness.probe.open_lane_changed.notified();
            if harness.probe.open_lane_calls.load(Ordering::Acquire) >= 1 {
                break;
            }
            notified.await;
        }
        // The first call stays parked until its owned deadline aborts it. A
        // replacement Host may open normally, proving recovery does not reuse
        // the timed-out process/epoch.
        harness
            .probe
            .block_open_lane
            .store(false, Ordering::Release);
        tokio::time::advance(HOST_LANE_OPEN_TIMEOUT).await;
        let error = tokio::time::timeout(Duration::from_secs(5), opening)
            .await
            .expect("timed-out Host.open_lane left its start flight pending")
            .unwrap()
            .unwrap_err();

        assert_eq!(error.metadata["host_open_lane_timeout"], true);
        assert_eq!(error.metadata["host_recovery_started"], true);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(harness.hub.overview().await.capacity.active, 0);
        assert!(harness.hub.inner.pending_host_retirements.lock().await.is_empty());
        assert!(harness.hub.inner.pending_lane_cleanups.lock().await.is_empty());
        let owner_cleanup_targets = harness.hub.inner.owner_cleanup_targets.lock().await.clone();
        assert!(
            owner_cleanup_targets.is_empty(),
            "timed-out Lane retained stale owner cleanup authority: {owner_cleanup_targets:?}"
        );
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        assert!(
            harness.probe.host_shutdowns.load(Ordering::Acquire) >= 1,
            "the exact timed-out Host must enter recovery cleanup"
        );
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "a timed-out unattached start must stop its exact Host without launching a replacement"
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
            .host_shutdown_fail_from
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
        let failed_shutdown_attempts = harness.probe.host_shutdowns.load(Ordering::Acquire);
        assert!(failed_shutdown_attempts >= 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert!(harness.hub.inner.host_slots.read().await.is_empty());
        assert_eq!(
            harness.hub.inner.retiring_host_slots.lock().await.len()
                + harness.hub.inner.orphaned_host_slots.lock().await.len(),
            1,
            "failed panic cleanup must remain under Hub authority"
        );
        let retained_budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(retained_budget.lane_tokens, 1);
        assert_eq!(retained_budget.host_tokens, 1);
        assert!(!harness.hub.inner.owner_cleanup_targets.lock().await.is_empty());

        harness
            .probe
            .host_shutdown_fail_from
            .store(usize::MAX, Ordering::Release);
        harness.hub.sweep().await.unwrap();
        assert!(
            harness.probe.host_shutdowns.load(Ordering::Acquire) > failed_shutdown_attempts
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
        assert!(harness.hub.inner.retiring_host_slots.lock().await.is_empty());
        assert!(harness.hub.inner.owner_cleanup_targets.lock().await.is_empty());
        let cleanup_budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(cleanup_budget.lane_tokens, 0);
        assert_eq!(cleanup_budget.host_tokens, 0);
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
            .host_shutdown_fail_from
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
        assert_eq!(
            harness.hub.inner.retiring_host_slots.lock().await.len()
                + harness.hub.inner.orphaned_host_slots.lock().await.len(),
            1
        );
        let before = harness.hub.overview().await;
        assert_eq!(before.managed_host_count, 1);
        assert!(before.pending_cleanup_count >= 1);

        harness
            .probe
            .host_shutdown_fail_from
            .store(usize::MAX, Ordering::Release);
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

    #[test]
    fn lane_admission_to_inventory_publication_has_no_cancellation_point() {
        let source = include_str!("hub.rs");
        let open_lane = source
            .find("pub async fn open_lane(")
            .expect("open_lane must exist");
        let start = source[open_lane..]
            .find("let mut unpublished_admission = UnpublishedLaneAdmissionGuard::new(")
            .map(|offset| open_lane + offset)
            .expect("open_lane admission guard marker must exist");
        let admit_offset = source[start..]
            .find("self.inner.scheduler.admit(")
            .expect("scheduler admission marker must exist");
        let publish_offset = source[start..]
            .find("unpublished_admission.publish(Arc::clone(&lane));")
            .expect("open_lane publication marker must exist");
        let guarded = &source[start..start + publish_offset];
        assert!(
            !guarded.contains(".await"),
            "scheduler admission and both Lane inventory inserts must remain one synchronous cancellation-free section"
        );
        assert!(admit_offset > 0, "rollback guard must precede admission");
        let guard_impl = &source[source
            .find("impl<'a> UnpublishedLaneAdmissionGuard<'a>")
            .expect("admission guard implementation must exist")..start];
        assert!(guard_impl.contains("self.lanes.insert"));
        assert!(guard_impl.contains("self.lane_keys\n            .insert"));
        assert!(guard_impl.contains("self.lanes.remove"));
        assert!(guard_impl.contains("discard_unpublished"));
    }

    #[tokio::test]
    async fn admission_publication_panic_rolls_back_both_indexes_and_scheduler() {
        let harness = harness();
        harness
            .hub
            .inner
            .lane_admission_publication_panics_remaining
            .store(1, Ordering::Release);
        let client = harness.client.clone();
        let opening = tokio::spawn(async move {
            client
                .open(
                    Some("admission-publication-panic"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        let join_error = opening
            .await
            .expect_err("synthetic publication panic must unwind the open task");
        assert!(join_error.is_panic());
        assert!(harness.hub.inner.lanes.read().await.is_empty());
        assert!(harness.hub.inner.lane_keys.read().await.is_empty());
        assert_eq!(harness.hub.inner.scheduler.retained_lane_count(), 0);

        let replacement = harness
            .client
            .open(
                Some("admission-publication-panic"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(replacement, OpenLaneOutcome::Running { .. }));
    }

    #[test]
    fn queued_promotion_start_publication_has_exact_rollback_and_no_final_await() {
        let source = include_str!("hub.rs");
        let promote = source
            .find("async fn promote_released_capacity(&self)")
            .expect("promotion loop must exist");
        let guard = source[promote..]
            .find("let mut unpublished_promotion = UnpublishedLanePromotionGuard::new(")
            .map(|offset| promote + offset)
            .expect("promotion rollback guard must exist");
        let snapshot_lock = source[guard..]
            .find("let mut snapshot = lane.snapshot.write().await;")
            .map(|offset| guard + offset)
            .expect("promotion snapshot publication lock must exist");
        let final_section = source[snapshot_lock..]
            .find("let (flight, spawn_start) =")
            .map(|offset| snapshot_lock + offset)
            .expect("promotion final publication section must exist");
        let publish = source[final_section..]
            .find("unpublished_promotion.publish();")
            .map(|offset| final_section + offset)
            .expect("promotion publication marker must exist");
        let guarded = &source[final_section..publish];
        assert!(!guarded.contains(".await"));
        assert!(guarded.contains("*active_flight = Some"));
        assert!(guarded.contains("self.spawn_lane_start("));
    }

    #[tokio::test]
    async fn cancelled_post_promotion_waiter_rolls_exact_lane_back_and_retry_starts_it() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let active = open(&harness.client, "promotion-abort-active").await;
        let queued_client = client_for_conversation(
            &harness,
            "conversation-promotion-abort",
            "runtime-promotion-abort",
        );
        let queued = queued_client
            .open(
                Some("promotion-abort-queued"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(queued, OpenLaneOutcome::Queued { .. }));
        let queued_lane_id = queued.lane().lane_id.clone();

        harness
            .hub
            .inner
            .promotion_publication_blocked
            .store(true, Ordering::Release);
        let hub = harness.hub.clone();
        let closing = tokio::spawn(async move { hub.close_lane(&active).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let changed = harness
                    .hub
                    .inner
                    .promotion_publication_changed
                    .notified();
                if harness
                    .hub
                    .inner
                    .promotion_publication_attempts
                    .load(Ordering::Acquire)
                    >= 1
                {
                    break;
                }
                changed.await;
            }
        })
        .await
        .expect("queued Lane was not selected for promotion");
        closing.abort();
        assert!(closing.await.unwrap_err().is_cancelled());
        harness
            .hub
            .inner
            .promotion_publication_blocked
            .store(false, Ordering::Release);

        assert_eq!(harness.hub.inner.scheduler.active_count(), 0);
        assert_eq!(harness.hub.inner.scheduler.queued_count(), 1);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued_lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Queued
        );

        harness.hub.promote_released_capacity().await;
        assert_eq!(harness.hub.inner.scheduler.active_count(), 1);
        assert_eq!(harness.hub.inner.scheduler.queued_count(), 0);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued_lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Running
        );
        harness.hub.close_all().await.unwrap();
    }

    #[tokio::test]
    async fn panicked_post_promotion_waiter_rolls_exact_lane_back_and_retry_starts_it() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 1;
        let harness = harness_with_config(config);
        let active = open(&harness.client, "promotion-panic-active").await;
        let queued_client = client_for_conversation(
            &harness,
            "conversation-promotion-panic",
            "runtime-promotion-panic",
        );
        let queued = queued_client
            .open(
                Some("promotion-panic-queued"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(queued, OpenLaneOutcome::Queued { .. }));
        let queued_lane_id = queued.lane().lane_id.clone();
        harness
            .hub
            .inner
            .promotion_publication_panics_remaining
            .store(1, Ordering::Release);

        let hub = harness.hub.clone();
        let closing = tokio::spawn(async move { hub.close_lane(&active).await });
        let join_error = closing
            .await
            .expect_err("synthetic promotion panic must unwind the close task");
        assert!(join_error.is_panic());
        assert_eq!(harness.hub.inner.scheduler.active_count(), 0);
        assert_eq!(harness.hub.inner.scheduler.queued_count(), 1);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued_lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Queued
        );

        harness.hub.promote_released_capacity().await;
        assert_eq!(harness.hub.inner.scheduler.active_count(), 1);
        assert_eq!(harness.hub.inner.scheduler.queued_count(), 0);
        assert_eq!(
            harness
                .hub
                .lane_snapshot_unchecked(&queued_lane_id)
                .await
                .unwrap()
                .lifecycle_state,
            LaneLifecycleState::Running
        );
        harness.hub.close_all().await.unwrap();
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

        tokio::time::timeout(Duration::from_secs(3), async {
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
    async fn normal_idle_expiry_stops_empty_primary_host_at_policy_deadline() {
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
    async fn keep_alive_lane_survives_turn_cleanup_and_idle_sweep_until_owner_revoke() {
        let harness = harness();
        let keep_alive_lane = open(&harness.client, "keep-alive-media").await;
        let ordinary_lane = open(&harness.client, "turn-scoped").await;
        harness
            .client
            .set_keep_alive(&keep_alive_lane, true)
            .await
            .unwrap();

        let turn_cleanup = harness.client.close_turn_lanes().await.unwrap();
        assert_eq!(turn_cleanup.closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&ordinary_lane).await.is_none());
        assert!(harness.hub.lane_snapshot_unchecked(&keep_alive_lane).await.is_some());

        let idle_expiry_ms = harness.hub.resource_policy().await.idle_expiry_ms;
        harness.clock.advance(idle_expiry_ms + 1);
        assert_eq!(harness.hub.sweep().await.unwrap().closed, 0);
        assert!(harness.hub.lane_snapshot_unchecked(&keep_alive_lane).await.is_some());

        let owner_lease_id = harness.client.caller().owner_lease_id.clone();
        let revoked = {
            let mut result = Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "fixture",
                true,
                "retry",
            ));
            for _ in 0..3 {
                result = harness.hub.revoke_owner_lease(&owner_lease_id).await;
                if result.is_ok() {
                    break;
                }
            }
            result.unwrap()
        };
        assert_eq!(revoked.closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&keep_alive_lane).await.is_none());
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
        let mut config = HubConfig::default();
        config.resource_policy = ResourcePolicy::preset(
            crate::ResourcePolicyPreset::HighConcurrency,
            8 * crate::resource::GIB,
            4,
        );
        let harness = harness_with_config(config);
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
        let mut config = HubConfig::default();
        config.resource_policy = ResourcePolicy::preset(
            crate::ResourcePolicyPreset::HighConcurrency,
            8 * crate::resource::GIB,
            4,
        );
        let harness = harness_with_config(config);
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
        let mut config = HubConfig::default();
        config.resource_policy = ResourcePolicy::preset(
            crate::ResourcePolicyPreset::HighConcurrency,
            8 * crate::resource::GIB,
            4,
        );
        let harness = harness_with_config(config);
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
    async fn pressured_sweep_freezes_then_reclaims_idle_expansion_and_preserves_first_lane() {
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

        assert_eq!(harness.hub.sweep().await.unwrap().closed, 1);
        assert!(harness.hub.lane_snapshot_unchecked(&first).await.is_some());
        assert!(
            harness
                .hub
                .lane_snapshot_unchecked(&expansion)
                .await
                .is_none()
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
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
    async fn shared_host_task_attribution_has_a_sibling_tab_dilution_floor() {
        let harness = harness();
        let sibling = client_for_conversation(
            &harness,
            "conversation-many-empty-tabs",
            "runtime-many-empty-tabs",
        );
        let one_tab_lane = open(&harness.client, "one-heavy-tab").await;
        let many_tab_lane = open(&sibling, "many-empty-tabs").await;

        let records = harness.hub.inner.lanes.read().await;
        let one_tab = records.get(&one_tab_lane).cloned().unwrap();
        let many_tabs = records.get(&many_tab_lane).cloned().unwrap();
        drop(records);
        one_tab.snapshot.write().await.tabs = vec![BrowserTabSnapshot {
            tab_id: "heavy-tab".to_owned(),
            target_id: "heavy-target".to_owned(),
            title: None,
            url: None,
            active: true,
            crashed: false,
        }];
        many_tabs.snapshot.write().await.tabs = (0..15)
            .map(|index| BrowserTabSnapshot {
                tab_id: format!("empty-tab-{index}"),
                target_id: format!("empty-target-{index}"),
                title: None,
                url: None,
                active: index == 0,
                crashed: false,
            })
            .collect();

        harness
            .hub
            .refresh_lane_resource_estimates(&ResourceTelemetry {
                host_rss_by_process_id: HashMap::from([(4_242, crate::resource::GIB)]),
                ..Default::default()
            })
            .await;
        let samples = harness.hub.inner.task_memory_samples.read().await;
        let one_tab_share = samples
            .get(&harness.client.caller().task_resource_family_key().into_string())
            .unwrap()
            .shared_rss_estimate_bytes;
        let many_tab_share = samples
            .get(&sibling.caller().task_resource_family_key().into_string())
            .unwrap()
            .shared_rss_estimate_bytes;

        assert!(
            one_tab_share >= crate::resource::GIB / 4,
            "a sibling's empty tabs diluted the one-tab task below its Host baseline: {one_tab_share}"
        );
        assert_eq!(
            one_tab_share.saturating_add(many_tab_share),
            crate::resource::GIB,
            "shared Host attribution must conserve measured RSS exactly"
        );
    }

    #[tokio::test]
    async fn sustained_task_overage_eventually_closes_active_primary_without_harming_sibling() {
        let mut config = HubConfig::default();
        config.resource_policy.max_task_memory_bytes = crate::resource::MIN_TASK_MEMORY_BYTES;
        let harness = harness_with_config(config);
        let sibling = client_for_conversation(
            &harness,
            "conversation-memory-sibling",
            "runtime-memory-sibling",
        );
        let noisy_lane = open(&harness.client, "memory-noisy-primary").await;
        let sibling_lane = open(&sibling, "memory-sibling-primary").await;

        let noisy = harness
            .hub
            .inner
            .lanes
            .read()
            .await
            .get(&noisy_lane)
            .cloned()
            .unwrap();
        noisy.active_operation_count.store(1, Ordering::Release);
        noisy.snapshot.write().await.tabs = (0..15)
            .map(|index| BrowserTabSnapshot {
                tab_id: format!("tab-{index}"),
                target_id: format!("target-{index}"),
                title: None,
                url: None,
                active: index == 0,
                crashed: false,
            })
            .collect();

        let sample = || ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            chromium_rss_bytes: 700 * crate::resource::MIB,
            host_rss_by_process_id: HashMap::from([(4_242, 700 * crate::resource::MIB)]),
            logical_cpus: 8,
            ..Default::default()
        };

        // A severe estimate accelerates the watchdog, but it still first
        // offers idle expansion and idle-primary stages. An active Primary is
        // task-locally cancelled only after the overage remains sustained.
        // Attribution gives the noisy task ~464 MiB of the shared 700 MiB Host
        // against its 256 MiB budget: materially over (>1.5x) but not severely
        // over (>2x), and not exclusive, so the only accelerator is +1.
        //
        // The escalation is deliberately slow, because this attribution is an
        // estimate and a browser may sit at a legitimately high steady state:
        //   samples 1-2  hysteresis floor (TASK_RECLAIM_MIN_SUSTAINED_SAMPLES)
        //                is not met yet, so nothing is reclaimable at all;
        //   samples 3-5  stages 2..4 offer idle/expansion Lanes first. This
        //                task's single Lane is active, and a task's *last* Lane
        //                is reserved for the top stage, so it is protected;
        //   sample 6     stage 5 (TASK_RECLAIM_ACTIVE_ANY_STREAK) is reached and
        //                the active last Lane is finally reclaimed.
        //
        // That is ~30s of sustained overage at the default 5s sample period.
        for _ in 0..5 {
            harness.hub.update_resource_telemetry(sample()).await;
            assert!(
                harness.hub.lane_snapshot_unchecked(&noisy_lane).await.is_some(),
                "an estimated overage must not reclaim a task's only active Lane \
                 before the escalation reaches its top stage"
            );
        }
        harness.hub.update_resource_telemetry(sample()).await;

        assert!(harness.hub.lane_snapshot_unchecked(&noisy_lane).await.is_none());
        assert!(sibling.status(&sibling_lane).await.is_ok());
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "task-local reclaim must not terminate a shared Primary Host"
        );
    }

    /// A browser that is merely *expensive* must not be treated as leaking.
    ///
    /// This is the regression for the user-visible defect: an ordinary session
    /// sitting at a high but stable memory level was reclaimed within about one
    /// sampling period, because the severity/confidence accelerators reached an
    /// eligible stage with a streak of 1. A steady state must survive
    /// indefinitely as long as it stays inside the budget.
    #[tokio::test]
    async fn steady_state_memory_inside_budget_is_never_reclaimed() {
        let mut config = HubConfig::default();
        // 2 GiB budget against a 1.5 GiB single-task Host: high, but legitimate.
        config.resource_policy.max_task_memory_bytes = 2 * crate::resource::GIB;
        let harness = harness_with_config(config);
        let lane = open(&harness.client, "steady-state-primary").await;

        let sample = || ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            chromium_rss_bytes: 3 * crate::resource::GIB / 2,
            host_rss_by_process_id: HashMap::from([(
                4_242,
                3 * crate::resource::GIB / 2,
            )]),
            logical_cpus: 8,
            ..Default::default()
        };

        for _ in 0..24 {
            harness.hub.update_resource_telemetry(sample()).await;
        }

        assert!(
            harness.hub.lane_snapshot_unchecked(&lane).await.is_some(),
            "a session that stays inside its budget must never be reclaimed, \
             however long it runs"
        );
    }

    /// A task's only Lane is its entire browser, so an *estimated* attribution
    /// may not close it until the escalation reaches its top stage. An idle
    /// single-Lane task would otherwise be reclaimed at stage 3
    /// (`TASK_RECLAIM_IDLE_ANY_STREAK`) — which is what killed sessions while
    /// the Agent was waiting on the model between tool calls.
    #[tokio::test]
    async fn only_lane_of_a_task_survives_the_idle_reclaim_stages() {
        let mut config = HubConfig::default();
        config.resource_policy.max_task_memory_bytes = crate::resource::MIN_TASK_MEMORY_BYTES;
        let harness = harness_with_config(config);
        let lane = open(&harness.client, "sole-idle-primary").await;

        // Idle: no in-flight operation, exactly as an Agent looks while it waits
        // for the model to produce its next tool call.
        let record = harness
            .hub
            .inner
            .lanes
            .read()
            .await
            .get(&lane)
            .cloned()
            .unwrap();
        assert_eq!(record.active_operation_count.load(Ordering::Acquire), 0);

        // Exclusive Host and severely over budget: the fastest possible
        // escalation. Even so, stages 1..4 must not take the only Lane.
        let sample = || ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            chromium_rss_bytes: crate::resource::GIB,
            host_rss_by_process_id: HashMap::from([(4_242, crate::resource::GIB)]),
            logical_cpus: 8,
            ..Default::default()
        };

        // Samples 1-2 are below the hysteresis floor; sample 3 reaches stage
        // 1+1+2=4, still short of the top stage reserved for a last Lane.
        for _ in 0..3 {
            harness.hub.update_resource_telemetry(sample()).await;
            assert!(
                harness.hub.lane_snapshot_unchecked(&lane).await.is_some(),
                "the only Lane of a task must survive the idle reclaim stages"
            );
        }

        // Sample 4 reaches stage 5 and the last Lane finally becomes eligible,
        // so a genuine runaway is still bounded rather than immune.
        harness.hub.update_resource_telemetry(sample()).await;
        assert!(
            harness.hub.lane_snapshot_unchecked(&lane).await.is_none(),
            "a sustained severe overage must still converge"
        );
    }

    /// Reclaim must not tell the caller the *user* closed the browser.
    #[tokio::test]
    async fn reclaimed_lane_reports_an_honest_retryable_memory_error() {
        let mut config = HubConfig::default();
        config.resource_policy.max_task_memory_bytes = crate::resource::MIN_TASK_MEMORY_BYTES;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "reclaim-error-primary").await;
        let record = harness
            .hub
            .inner
            .lanes
            .read()
            .await
            .get(&lane_id)
            .cloned()
            .unwrap();

        // Before reclaim marks it, a close is an ordinary user-initiated close.
        assert_eq!(
            record.closed_error(lane_id.clone()).code,
            BrowserErrorCode::LaneClosedByUser
        );

        record.memory_reclaimed.store(true, Ordering::Release);
        let error = record.closed_error(lane_id.clone());
        assert_eq!(error.code, BrowserErrorCode::TaskMemoryReclaimed);
        assert!(
            error.retryable,
            "an Agent must be allowed to reopen a Lane after a memory reclaim"
        );
        assert_eq!(error.metadata["reason"], "task_memory_budget");
    }

    #[tokio::test]
    async fn shared_host_dilution_still_converges_after_consecutive_real_rss_breaches() {
        let harness = harness();
        let siblings = (2..=4)
            .map(|index| {
                client_for_conversation(
                    &harness,
                    &format!("conversation-dilution-{index}"),
                    &format!("runtime-dilution-{index}"),
                )
            })
            .collect::<Vec<_>>();
        open(&harness.client, "dilution-primary-1").await;
        for (index, sibling) in siblings.iter().enumerate() {
            open(sibling, &format!("dilution-primary-{}", index + 2)).await;
        }
        let crawl = client_for_conversation(
            &harness,
            "conversation-small-crawl",
            "runtime-small-crawl",
        );
        crawl
            .open(
                Some("small-crawl"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap();

        let high = || ResourceTelemetry {
            total_memory_bytes: 8 * crate::resource::GIB,
            available_memory_bytes: 4 * crate::resource::GIB,
            chromium_rss_bytes: 3_400 * crate::resource::MIB,
            logical_cpus: 4,
            host_rss_by_process_id: HashMap::from([
                (4_242, 3_300 * crate::resource::MIB),
                (4_243, 100 * crate::resource::MIB),
            ]),
            ..Default::default()
        };
        let recovered = || ResourceTelemetry {
            total_memory_bytes: 8 * crate::resource::GIB,
            available_memory_bytes: 5 * crate::resource::GIB,
            chromium_rss_bytes: 2_600 * crate::resource::MIB,
            logical_cpus: 4,
            host_rss_by_process_id: HashMap::from([
                (4_242, 2_500 * crate::resource::MIB),
                (4_243, 100 * crate::resource::MIB),
            ]),
            ..Default::default()
        };

        harness
            .hub
            .refresh_lane_resource_estimates(&high())
            .await;
        let attributed = harness.hub.inner.task_memory_samples.read().await;
        for task_id in std::iter::once(
            harness.client.caller().task_resource_family_key().into_string(),
        )
            .chain(
                siblings
                    .iter()
                    .map(|client| client.caller().task_resource_family_key().into_string()),
            )
        {
            assert!(
                attributed
                    .get(&task_id)
                    .is_some_and(|sample| sample.shared_rss_estimate_bytes < crate::resource::GIB),
                "equal activity on a shared Host can keep every task below the 1 GiB estimate even when one page caused the physical overage"
            );
        }
        drop(attributed);

        // Two critical samples are temporal hysteresis, not a restart. A real
        // recovery sample resets the streak, so another two samples still do
        // not disrupt the shared Host.
        harness.hub.update_resource_telemetry(high()).await;
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
        harness.hub.update_resource_telemetry(recovered()).await;
        harness.hub.update_resource_telemetry(high()).await;
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);

        // The third consecutive real breach cannot remain admission-only. The
        // 3.3 GiB managed Primary is replaced ahead of the unrelated 100 MiB
        // Anonymous Host, even though RSS attribution could not name a culprit.
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 3);
        assert_eq!(
            harness
                .probe
                .host_launch_requests
                .lock()
                .expect("host launch probe poisoned")
                .last()
                .map(|request| request.identity_mode),
            Some(BrowserIdentityMode::Primary)
        );
    }

    #[tokio::test]
    async fn resource_emergency_progress_resets_streak_and_failed_restart_retries() {
        let harness = harness();
        let siblings = (2..=4)
            .map(|index| {
                client_for_conversation(
                    &harness,
                    &format!("conversation-retry-{index}"),
                    &format!("runtime-retry-{index}"),
                )
            })
            .collect::<Vec<_>>();
        open(&harness.client, "retry-primary-1").await;
        for (index, sibling) in siblings.iter().enumerate() {
            open(sibling, &format!("retry-primary-{}", index + 2)).await;
        }
        let high = || ResourceTelemetry {
            total_memory_bytes: 8 * crate::resource::GIB,
            available_memory_bytes: 4 * crate::resource::GIB,
            chromium_rss_bytes: 3_300 * crate::resource::MIB,
            logical_cpus: 4,
            host_rss_by_process_id: HashMap::from([(4_242, 3_300 * crate::resource::MIB)]),
            ..Default::default()
        };

        let policy = harness.hub.resource_policy().await;
        let decision = harness.hub.decide_resources(&policy, &high()).await;
        harness
            .hub
            .inner
            .critical_browser_rss_streak
            .store(2, Ordering::Release);
        harness
            .hub
            .converge_sustained_browser_rss_pressure(&high(), &decision, 1)
            .await;
        assert_eq!(
            harness
                .hub
                .inner
                .critical_browser_rss_streak
                .load(Ordering::Acquire),
            0,
            "an exact task-local close is progress and must reset the emergency streak"
        );

        harness
            .probe
            .host_shutdown_failures_remaining
            .store(1, Ordering::Release);
        for _ in 0..RESOURCE_EMERGENCY_CRITICAL_SAMPLES {
            harness.hub.update_resource_telemetry(high()).await;
        }
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);

        // A failed exact shutdown is retained by the Host lifecycle authority;
        // it does not latch this watchdog off forever. Two new samples are
        // again hysteresis, and the third retries then replaces the same Host.
        for _ in 0..(RESOURCE_EMERGENCY_CRITICAL_SAMPLES - 1) {
            harness.hub.update_resource_telemetry(high()).await;
        }
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 2);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn resource_emergency_never_restarts_for_unmatched_process_rss() {
        let harness = harness();
        open(&harness.client, "managed-primary").await;
        let unmatched = || ResourceTelemetry {
            total_memory_bytes: 8 * crate::resource::GIB,
            available_memory_bytes: 4 * crate::resource::GIB,
            chromium_rss_bytes: 4 * crate::resource::GIB,
            logical_cpus: 4,
            // A trusted collector should never publish an unrelated root here,
            // but the Hub still requires an exact live driver-PID join.
            host_rss_by_process_id: HashMap::from([(99_999, 4 * crate::resource::GIB)]),
            ..Default::default()
        };

        for _ in 0..(RESOURCE_EMERGENCY_CRITICAL_SAMPLES * 2) {
            harness
                .hub
                .update_resource_telemetry(unmatched())
                .await;
        }
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn sustained_managed_cpu_pressure_restarts_busiest_exact_host() {
        let harness = harness();
        open(&harness.client, "cpu-primary").await;
        let crawl = client_for_runtime(&harness, "runtime-cpu-crawl");
        crawl
            .open(
                Some("cpu-crawl"),
                BrowserIdentityMode::Anonymous,
                None,
            )
            .await
            .unwrap();

        let high = || ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            logical_cpus: 8,
            cpu_pressure: 0.95,
            host_cpu_pressure_by_process_id: HashMap::from([
                (4_242, 0.60),
                (4_243, 0.10),
            ]),
            ..Default::default()
        };
        let recovered = || ResourceTelemetry {
            cpu_pressure: 0.50,
            ..high()
        };

        // A task-local close is concrete progress and resets this independent
        // CPU hysteresis just as it does for the RSS endpoint.
        harness
            .hub
            .inner
            .critical_browser_cpu_streak
            .store(2, Ordering::Release);
        harness
            .hub
            .converge_sustained_browser_cpu_pressure(&high(), 1)
            .await;
        assert_eq!(
            harness
                .hub
                .inner
                .critical_browser_cpu_streak
                .load(Ordering::Acquire),
            0
        );

        harness.hub.update_resource_telemetry(high()).await;
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
        harness.hub.update_resource_telemetry(recovered()).await;
        harness.hub.update_resource_telemetry(high()).await;
        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);

        harness.hub.update_resource_telemetry(high()).await;
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 3);
        assert_eq!(
            harness
                .probe
                .host_launch_requests
                .lock()
                .expect("host launch probe poisoned")
                .last()
                .map(|request| request.identity_mode),
            Some(BrowserIdentityMode::Primary),
            "the 60% exact managed Host must be selected ahead of the 10% Host"
        );
    }

    #[tokio::test]
    async fn cpu_emergency_ignores_unmatched_pressure_and_small_managed_share() {
        let harness = harness();
        open(&harness.client, "managed-cpu-primary").await;
        let unmatched = || ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            logical_cpus: 8,
            cpu_pressure: 0.99,
            // Even a huge unmatched sample cannot make the small exact
            // managed share look culpable or turn another Chrome into a
            // termination target.
            host_cpu_pressure_by_process_id: HashMap::from([
                (4_242, 0.10),
                (99_999, 0.90),
            ]),
            ..Default::default()
        };

        for _ in 0..(RESOURCE_EMERGENCY_CRITICAL_SAMPLES * 2) {
            harness
                .hub
                .update_resource_telemetry(unmatched())
                .await;
        }
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn pressured_release_does_not_promote_expansion_and_recovery_wakes_queue() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 2;
        let harness = harness_with_config(config);
        let other = client_for_conversation(&harness, "conversation-2", "runtime-2");
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

        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 16 * crate::resource::GIB,
                available_memory_bytes: 12 * crate::resource::GIB,
                cpu_pressure: 0.95,
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
        let other = client_for_conversation(
            &harness,
            "conversation-pressure-2",
            "runtime-pressure-2",
        );
        harness
            .hub
            .update_resource_telemetry(ResourceTelemetry {
                total_memory_bytes: 64 * crate::resource::GIB,
                available_memory_bytes: 7 * crate::resource::GIB,
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
        let other = client_for_conversation(&harness, "conversation-2", "runtime-2");
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
        assert_eq!(workload.queued_lanes, 1);

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
    async fn hub_workload_counts_detached_unresolved_lane_start() {
        let harness = harness();
        let policy = harness.hub.resource_policy().await;
        let caller = harness.client.caller().clone();
        harness
            .hub
            .inner
            .pending_host_retirements
            .lock()
            .await
            .push(PendingHostRetirement {
                key: HostKey {
                    identity_mode: BrowserIdentityMode::Primary,
                    identity_generation: 0,
                    isolation_lane_id: None,
                },
                lane_id: BrowserLaneId::new(),
                user_id: caller.user_id.clone(),
                task_id: caller.task_resource_key(),
                family_id: caller.task_resource_family_key().into_string(),
                owner_lease_id: caller.owner_lease_id,
                start_flight: Arc::new(LaneStartFlight::new()),
            });

        let workload = harness
            .hub
            .resource_workload(policy.lane_cold_start_bytes)
            .await;
        assert_eq!(workload.queued_lanes, 1);
        assert_eq!(workload.queued_lane_estimate_bytes, policy.lane_cold_start_bytes);
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
    async fn lane_operation_admission_is_bounded_and_close_releases_every_slot() {
        let mut config = HubConfig::default();
        config.resource_policy.max_active_operations = 4;
        config.resource_policy.max_task_active_operations = 4;
        let harness = harness_with_config(config);
        let lane_id = open(&harness.client, "bounded-operation-waiters").await;

        let mut operations = Vec::new();
        for _ in 0..MAX_LANE_OPERATION_ADMISSIONS {
            let client = harness.client.clone();
            let lane_id = lane_id.clone();
            operations.push(tokio::spawn(async move {
                client.execute(&lane_id, navigate()).await
            }));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let admitted = harness
                    .hub
                    .inner
                    .operation_admissions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .by_lane
                    .get(&lane_id)
                    .copied()
                    .unwrap_or_default();
                if admitted == MAX_LANE_OPERATION_ADMISSIONS {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("operation admissions did not reach the Lane hard bound");

        let overflow = harness
            .client
            .execute(&lane_id, navigate())
            .await
            .expect_err("N+1 operation must be rejected before retaining a waiter");
        assert_eq!(overflow.code, BrowserErrorCode::BrowserCapacityQueued);
        assert_eq!(
            overflow.metadata["reason_code"],
            "browser_operation_capacity_busy"
        );
        assert_eq!(overflow.metadata["capacity_scope"], "lane");

        harness.hub.close_lane(&lane_id).await.unwrap();
        for operation in operations {
            assert_eq!(
                operation.await.unwrap().unwrap_err().code,
                BrowserErrorCode::LaneClosedByUser
            );
        }
        let admissions = harness
            .hub
            .inner
            .operation_admissions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(admissions.total, 0);
        assert!(admissions.by_lane.is_empty());
        assert!(admissions.by_task.is_empty());
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
    async fn tab_policy_raise_waits_for_stale_start_then_reconciles_new_cap() {
        let mut config = HubConfig::default();
        config.resource_policy.max_task_tabs = 4;
        let harness = harness_with_config(config);
        let _anchor = open(&harness.client, "tab-raise-anchor").await;
        harness
            .probe
            .block_open_lane
            .store(true, Ordering::Release);

        let client = harness.client.clone();
        let starting = tokio::spawn(async move {
            client
                .open(
                    Some("tab-raise-stale-start"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        harness.probe.wait_for_open_lane_calls(2).await;
        assert_eq!(
            *harness
                .probe
                .lane_launch_tab_limits
                .lock()
                .expect("Lane tab-limit probe poisoned")
                .last()
                .unwrap(),
            4,
            "the in-flight Lane should have captured the old route cap"
        );

        let mut raised = harness.hub.resource_policy().await;
        raised.max_task_tabs = 8;
        let hub = harness.hub.clone();
        let update = tokio::spawn(async move { hub.set_resource_policy(raised).await });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(
            !update.is_finished(),
            "policy raise committed before the stale Starting route published"
        );

        harness.probe.open_lane_release.add_permits(1);
        assert!(matches!(
            starting.await.unwrap().unwrap(),
            OpenLaneOutcome::Running { .. }
        ));
        update.await.unwrap().unwrap();

        assert_eq!(harness.hub.inner.task_tab_authority.limit(), 8);
        assert_eq!(harness.hub.resource_policy().await.max_task_tabs, 8);
        let reconciles = harness
            .probe
            .tab_reconcile_limits
            .lock()
            .expect("tab reconcile probe poisoned");
        assert!(
            reconciles.iter().any(|(_, limit)| *limit == 8),
            "the late old-cap route was not reconciled to the raised cap: {reconciles:?}"
        );
        assert_eq!(reconciles.last().map(|(_, limit)| *limit), Some(8));
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
            0,
            "exact replacement Host shutdown is stronger proof than its failed target close"
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
            "replacement Host proof must leave no prepared-driver residue"
        );
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);

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
            old_epoch + 2
        );
        assert!(harness.client.status(&lane_c).await.is_ok());
    }

    #[tokio::test]
    async fn failed_rebind_with_permanent_cleanup_failure_cannot_amplify_drivers() {
        let harness = harness();
        let lane_a = open(&harness.client, "rebind-bounded-a").await;
        let _lane_b = open(&harness.client, "rebind-bounded-b").await;
        let calls_before_restart = harness
            .probe
            .open_lane_calls
            .load(Ordering::Acquire);
        harness
            .probe
            .open_lane_failure_at
            .store(calls_before_restart + 2, Ordering::Release);
        harness
            .probe
            .lane_close_failures_remaining
            .store(usize::MAX, Ordering::Release);
        // Call one stops the old Host. Every shutdown from call two onward
        // fails, leaving the exact replacement process in retained authority.
        harness
            .probe
            .host_shutdown_fail_from
            .store(2, Ordering::Release);
        harness
            .probe
            .host_fatal_executions_remaining
            .store(1, Ordering::Release);

        let first = harness
            .client
            .execute(&lane_a, navigate())
            .await
            .unwrap_err();
        assert_eq!(first.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
        assert_eq!(
            harness.hub.inner.pending_lane_cleanups.lock().await.len(),
            1
        );
        assert_eq!(harness.hub.inner.orphaned_host_slots.lock().await.len(), 1);
        let baseline_budget = harness.hub.inner.cleanup_budget.snapshot();

        for _ in 0..12 {
            let error = harness
                .client
                .execute(&lane_a, navigate())
                .await
                .unwrap_err();
            assert!(matches!(
                error.code,
                BrowserErrorCode::BrowserCapacityQueued
                    | BrowserErrorCode::BrowserUnavailable
            ));
        }

        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            2,
            "retained replacement cleanup must fence every later relaunch"
        );
        assert_eq!(
            harness.probe.open_lane_calls.load(Ordering::Acquire),
            calls_before_restart + 2,
            "the prepared physical Lane driver must not be duplicated"
        );
        assert_eq!(
            harness.hub.inner.pending_lane_cleanups.lock().await.len(),
            1
        );
        assert_eq!(harness.hub.inner.orphaned_host_slots.lock().await.len(), 1);
        let after_retries = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(after_retries.lane_tokens, baseline_budget.lane_tokens);
        assert_eq!(after_retries.host_tokens, baseline_budget.host_tokens);
        assert_eq!(after_retries.global.count, baseline_budget.global.count);
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
        assert_eq!(snapshot.lifecycle_state, LaneLifecycleState::Failed);
        assert!(
            snapshot.error_code.is_some(),
            "a recovery-blocked Lane must retain its terminal error"
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
        // Cleanup debt fences only the exact runtime that owns it. A distinct
        // user-visible task must still be able to consume released aggregate
        // capacity while the old target is converging in the background.
        let (replacement_client, _replacement_lease_id) =
            client_for_runtime_with_lease(&harness, "runtime-hung-cleanup-replacement");
        let replacement = tokio::time::timeout(
            Duration::from_secs(1),
            open(&replacement_client, "replacement"),
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

    #[tokio::test]
    async fn failed_shared_target_cleanup_retries_without_request_or_sweep() {
        let harness = harness();
        let target = open(&harness.client, "autonomous-cleanup-target").await;
        let sibling = open(&harness.client, "autonomous-cleanup-sibling").await;
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);

        let error = harness.hub.close_lane(&target).await.unwrap_err();
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(harness.probe.lane_closes.load(Ordering::Acquire), 1);
        assert_eq!(harness.hub.inner.pending_lane_cleanups.lock().await.len(), 1);

        tokio::time::timeout(
            Duration::from_secs(3),
            harness.probe.wait_for_lane_closes(2),
        )
        .await
        .expect("retained target cleanup was not retried autonomously");
        tokio::task::yield_now().await;

        assert!(harness.hub.inner.pending_lane_cleanups.lock().await.is_empty());
        assert!(harness.client.status(&sibling).await.is_ok());
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "autonomous task cleanup must not stop a Host shared by a sibling Lane"
        );
    }

    #[tokio::test]
    async fn host_finalizer_panic_completes_waiters_and_retries_autonomously() {
        let harness = harness();
        let lane_id = open(&harness.client, "host-finalizer-panic").await;
        harness
            .probe
            .host_shutdown_panics_remaining
            .store(1, Ordering::Release);

        let error = tokio::time::timeout(Duration::from_secs(1), harness.hub.close_lane(&lane_id))
            .await
            .expect("Host finalizer panic stranded its single-flight waiter")
            .unwrap_err();
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(error.metadata["task_panicked"], true);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(harness.hub.inner.host_finalizations.lock().await.len(), 1);

        tokio::time::timeout(
            Duration::from_secs(3),
            harness.probe.wait_for_host_shutdowns(2),
        )
        .await
        .expect("panicked Host finalization was not retried autonomously");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if harness.hub.inner.host_finalizations.lock().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("successful autonomous Host retry did not retire its flight");
        assert!(harness.hub.inner.retiring_host_slots.lock().await.is_empty());
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
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
    async fn hung_lane_cleanup_escalates_only_at_its_hard_deadline() {
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

        // The soft caller timeout must not itself kill the Host. The Hub-owned
        // cleanup continues until its separate absolute deadline.
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "the soft waiter timeout retired the Host early"
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

        // At the hard bound the hung close task is aborted and exact Host
        // shutdown becomes the stronger cleanup proof.
        tokio::time::advance(
            LANE_CLEANUP_HARD_TIMEOUT - LANE_CLEANUP_WAITER_TIMEOUT,
        )
        .await;
        tokio::task::yield_now().await;
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_shutdowns(1),
        )
        .await
        .expect("the empty Host was not retired at the Lane cleanup hard bound");
        assert_eq!(
            harness.probe.lane_close_completions.load(Ordering::Acquire),
            0,
            "the hung driver close should be aborted, not reported complete"
        );
        harness.hub.sweep().await.unwrap();
        assert_eq!(
            harness.hub.remaining_resources().await,
            RemainingResources::default()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cold_start_launch_timeout_retires_the_slot_and_retry_gets_a_fresh_epoch() {
        let harness = harness();
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);

        let error = harness
            .client
            .open(
                Some("cold-start-timeout"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .expect_err("a blocked cold start must time out");
        assert_eq!(error.metadata["host_initialization_timeout"], true);
        assert_eq!(error.metadata["phase"], "launch");
        assert!(
            harness.hub.inner.host_slots.read().await.is_empty(),
            "the timed-out slot must be retired to cleanup authority, not left active"
        );
        let key = HostKey {
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 0,
            isolation_lane_id: None,
        };
        assert!(harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert_eq!(harness.hub.inner.orphaned_host_slots.lock().await.len(), 1);
        assert!(harness
            .hub
            .inner
            .host_cleanup_budget_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&HostCleanupAuthorityKey {
                host_key: key.clone(),
                browser_epoch: 1,
            }));
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);

        // A permanently pending first flight retains the exact HostKey and its
        // cleanup-budget token. Repeated admissions may time out while waiting
        // for cleanup, but must never amplify into more physical launches.
        let runtime_key = harness
            .client
            .caller()
            .runtime_cleanup_key()
            .into_string();
        let family_key = harness
            .client
            .caller()
            .task_resource_family_key()
            .into_string();
        for _ in 0..3 {
            let retry = match harness
                .hub
                .get_or_launch_host(
                    BrowserIdentityMode::Primary,
                    0,
                    &BrowserLaneId::new(),
                    &runtime_key,
                    &family_key,
                )
                .await
            {
                Ok(_) => panic!("cleanup proof must fence every premature retry"),
                Err(error) => error,
            };
            assert_eq!(retry.metadata["cleanup_pending"], true);
            assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        }

        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);
        harness.probe.host_launch_release.add_permits(1);
        tokio::task::yield_now().await;
        harness.hub.retry_orphaned_host_slots().await.unwrap();
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert!(harness.hub.inner.orphaned_host_slots.lock().await.is_empty());
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        let lane = harness
            .client
            .open(
                Some("cold-start-retry"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .expect("retry after a cold-start timeout must succeed")
            .lane()
            .clone();
        assert_eq!(
            lane.browser_epoch, 2,
            "the retry must launch a fresh slot/epoch instead of reusing the timed-out slot"
        );
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn factory_error_with_deferred_cleanup_lease_retains_exact_host_fence() {
        let harness = harness();
        harness
            .probe
            .host_launch_failures_remaining
            .store(1, Ordering::Release);
        harness
            .probe
            .defer_failed_host_launch_cleanup
            .store(true, Ordering::Release);

        let error = harness
            .client
            .open(
                Some("factory-deferred-cleanup"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .expect_err("a synthetic factory error must fail the open");
        assert_eq!(error.metadata["host_launch_cleanup_pending"], true);
        let key = HostKey {
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 0,
            isolation_lane_id: None,
        };
        assert!(harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert_eq!(harness.hub.inner.orphaned_host_slots.lock().await.len(), 1);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert_eq!(
            harness
                .probe
                .deferred_host_launch_cleanup_leases
                .lock()
                .expect("deferred Host launch cleanup probe poisoned")
                .len(),
            1
        );

        let runtime_key = harness
            .client
            .caller()
            .runtime_cleanup_key()
            .into_string();
        let family_key = harness
            .client
            .caller()
            .task_resource_family_key()
            .into_string();
        for _ in 0..3 {
            let result = harness
                .hub
                .get_or_launch_host(
                    BrowserIdentityMode::Primary,
                    0,
                    &BrowserLaneId::new(),
                    &runtime_key,
                    &family_key,
                )
                .await;
            assert!(result.is_err());
            assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        }

        harness
            .probe
            .deferred_host_launch_cleanup_leases
            .lock()
            .expect("deferred Host launch cleanup probe poisoned")
            .clear();
        harness
            .probe
            .defer_failed_host_launch_cleanup
            .store(false, Ordering::Release);
        harness.hub.retry_orphaned_host_slots().await.unwrap();
        assert!(harness.hub.inner.orphaned_host_slots.lock().await.is_empty());
        assert!(!harness.hub.inner.retiring_host_keys.read().await.contains(&key));
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 0);

        let lane = harness
            .client
            .open(
                Some("factory-cleanup-proven-retry"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .expect("cleanup proof must allow one fresh Host epoch")
            .lane()
            .clone();
        assert_eq!(lane.browser_epoch, 2);
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn idle_telemetry_samples_do_not_broadcast_without_change_or_lanes() {
        let harness = harness();
        let mut events = harness.hub.subscribe();
        let normal = ResourceTelemetry {
            total_memory_bytes: 16 * crate::resource::GIB,
            available_memory_bytes: 12 * crate::resource::GIB,
            logical_cpus: 8,
            ..Default::default()
        };

        // The first sample establishes the pressure state and is broadcast.
        harness.hub.update_resource_telemetry(normal.clone()).await;
        assert_eq!(
            events.recv().await.unwrap().change_kind,
            "resource_pressure_sampled"
        );

        // An idle repeat (zero lanes, unchanged state) must not push a
        // client-visible event every sample period forever.
        harness.hub.update_resource_telemetry(normal.clone()).await;
        harness.hub.update_resource_telemetry(normal.clone()).await;
        assert!(
            matches!(
                events.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "idle unchanged samples must be suppressed"
        );

        // A pressure-state transition is actionable and is broadcast.
        let pressured = ResourceTelemetry {
            available_memory_bytes: 3 * crate::resource::GIB,
            ..normal.clone()
        };
        harness.hub.update_resource_telemetry(pressured.clone()).await;
        assert_eq!(
            events.recv().await.unwrap().change_kind,
            "resource_pressure_sampled"
        );

        // With live lanes every sample is broadcast so lane resource
        // estimates stay fresh in the management surface.
        let lane_id = open(&harness.client, "telemetry-live").await;
        assert!(harness.client.status(&lane_id).await.is_ok());
        while events.try_recv().is_ok() {}
        harness.hub.update_resource_telemetry(pressured).await;
        assert_eq!(
            events.recv().await.unwrap().change_kind,
            "resource_pressure_sampled"
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
        let lease_id = harness.client.caller().owner_lease_id.clone();
        harness.hub.shutdown().await.unwrap();
        harness.hub.shutdown().await.unwrap();
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert_eq!(
            harness.hub.renew_owner_lease(&lease_id).unwrap_err().code,
            BrowserErrorCode::BrowserShuttingDown
        );
        assert_eq!(
            harness
                .hub
                .issue_owner_lease("user", None, "runtime-after-shutdown")
                .unwrap_err()
                .code,
            BrowserErrorCode::BrowserShuttingDown
        );
    }

    #[tokio::test]
    async fn cancelled_shutdown_waiter_does_not_cancel_hub_owned_shutdown() {
        let harness = harness();
        let _ = open(&harness.client, "cancelled-shutdown-waiter").await;
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let waiter = tokio::spawn(async move { hub.shutdown().await });
        harness.probe.wait_for_host_shutdowns(1).await;
        let flight = harness
            .hub
            .inner
            .shutdown_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("shutdown leader did not publish its owned flight");
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        harness.probe.host_shutdown_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3), flight.wait())
            .await
            .expect("Hub-owned shutdown stopped when its caller was cancelled")
            .expect("Hub-owned shutdown failed after the blocked Host was released");
        harness.hub.shutdown().await.unwrap();

        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.list_lanes().await.is_empty());
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 0);
        assert_eq!(budget.host_tokens, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_wait_timeout_does_not_cancel_hub_owned_shutdown() {
        let harness = harness();
        let _ = open(&harness.client, "timed-out-shutdown-waiter").await;
        harness
            .probe
            .block_host_shutdown
            .store(true, Ordering::Release);

        let hub = harness.hub.clone();
        let waiter = tokio::spawn(async move { hub.shutdown().await });
        harness.probe.wait_for_host_shutdowns(1).await;
        let flight = harness
            .hub
            .inner
            .shutdown_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("shutdown leader did not publish its owned flight");

        tokio::time::advance(PLATFORM_SHUTDOWN_ATTEMPT_TIMEOUT).await;
        let error = waiter
            .await
            .expect("shutdown waiter task panicked")
            .expect_err("blocked shutdown waiter should hit its soft deadline");
        assert_eq!(error.metadata["platform_shutdown_timeout"], true);

        harness.probe.host_shutdown_release.add_permits(1);
        let first_terminal = tokio::time::timeout(Duration::from_secs(10), flight.wait())
            .await
            .expect("timed-out caller cancelled the Hub-owned shutdown flight");
        assert!(
            first_terminal.is_err(),
            "the first owned pass must preserve its earlier finalization wait error"
        );
        harness
            .hub
            .shutdown()
            .await
            .expect("a fresh explicit shutdown should confirm the completed cleanup proof");

        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            2,
            "the first adapter call hits its five-second hard deadline; the owned pass must retry the same retained Host exactly once"
        );
        assert!(harness.hub.managed_host_process_ids().await.is_empty());
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 0);
        assert_eq!(budget.host_tokens, 0);
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

    #[tokio::test]
    async fn open_lane_error_with_shared_sibling_requires_exact_host_stop() {
        let harness = harness();
        let anchor = open(&harness.client, "uncertain-open-anchor").await;
        let old_epoch = harness.client.status(&anchor).await.unwrap().browser_epoch;
        let next_call = harness.probe.open_lane_calls.load(Ordering::Acquire) + 1;
        harness
            .probe
            .open_lane_failure_at
            .store(next_call, Ordering::Release);

        let error = harness
            .client
            .open(
                Some("uncertain-open-failure"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            1,
            "a sibling Host cannot discharge an open_lane unknown side effect"
        );
        let anchor_after = harness.client.status(&anchor).await.unwrap();
        assert_ne!(anchor_after.browser_epoch, old_epoch);
        assert!(
            harness
                .hub
                .inner
                .host_stop_required_authorities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "exact old-Host shutdown must clear the sticky unknown-target fence"
        );
        assert_eq!(harness.hub.managed_host_process_ids().await.len(), 1);
        let task_id = harness.client.task_resource_key();
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        assert_eq!(budget.lane_tokens, 1);
        assert_eq!(budget.host_tokens, 1);
        assert_eq!(budget.tasks[&task_id].count, 2);
    }

    #[tokio::test]
    async fn many_cleanup_reconcile_requests_retry_only_exact_debt() {
        let harness = harness();
        let sibling = client_for_runtime(&harness, "reconcile-healthy-sibling");
        let stale = open(&harness.client, "reconcile-stale-target").await;
        let sibling_anchor = open(&sibling, "reconcile-sibling-anchor").await;

        // Keep the test in direct control of the pass. The production worker
        // uses this same flag to deduplicate requests and the same method to
        // perform reconciliation.
        harness
            .hub
            .inner
            .cleanup_retry_worker_running
            .store(true, Ordering::Release);
        // This is deliberately larger than the removed 64-key broad-scope
        // ledger. Every request collapses to one O(1) wakeup, never to a
        // Global close or a delayed task/Host predicate.
        for _ in 0..96 {
            harness.hub.request_cleanup_ledger_reconcile();
        }
        let later_same_runtime = open(&harness.client, "reconcile-later-same-runtime").await;
        let later_sibling = open(&sibling, "reconcile-later-sibling").await;

        // Publish one real exact cleanup debt only after those later Lanes
        // exist. The pass must retry that sealed target without dynamically
        // rediscovering either the same-runtime or sibling inventory.
        harness
            .probe
            .lane_close_failures_remaining
            .store(1, Ordering::Release);
        let error = harness.hub.close_lane(&stale).await.unwrap_err();
        assert_eq!(error.metadata["cleanup_pending"], true);
        assert_eq!(harness.hub.inner.pending_lane_cleanups.lock().await.len(), 1);

        harness.hub.autonomous_cleanup_pass().await.unwrap();
        harness
            .hub
            .inner
            .cleanup_retry_worker_running
            .store(false, Ordering::Release);

        assert!(harness.hub.inner.pending_lane_cleanups.lock().await.is_empty());
        assert!(harness.client.status(&later_same_runtime).await.is_ok());
        assert!(sibling.status(&sibling_anchor).await.is_ok());
        assert!(sibling.status(&later_sibling).await.is_ok());
        assert_eq!(
            harness.probe.lane_closes.load(Ordering::Acquire),
            2,
            "only the original exact target may be retried"
        );
        assert_eq!(
            harness.probe.host_shutdowns.load(Ordering::Acquire),
            0,
            "ledger reconciliation must not stop a shared healthy Host"
        );
        assert!(
            !harness
                .hub
                .inner
                .cleanup_ledger_reconcile_requested
                .load(Ordering::Acquire)
        );
    }

    #[tokio::test]
    async fn policy_completion_clears_flight_before_waking_next_update() {
        let harness = harness();
        let mut first = harness.hub.resource_policy().await;
        first.max_task_tabs = first.max_task_tabs.saturating_sub(1).max(1);
        harness.hub.set_resource_policy(first).await.unwrap();
        assert!(
            harness
                .hub
                .inner
                .policy_update_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_none(),
            "completion visibility must imply the policy slot is reusable"
        );
        let mut second = harness.hub.resource_policy().await;
        second.max_task_tabs = second.max_task_tabs.saturating_add(1);
        harness.hub.set_resource_policy(second).await.unwrap();
    }

    #[tokio::test]
    async fn thirty_two_isolated_lanes_fit_before_task_policy_queues_n_plus_one() {
        let mut config = HubConfig::default();
        config.resource_policy.max_open_lanes = 64;
        config.resource_policy.max_task_open_lanes = crate::MAX_TASK_OPEN_LANES;
        config.resource_policy.max_task_memory_bytes = crate::MAX_TASK_MEMORY_BYTES;
        config.resource_policy.max_task_tabs = crate::MAX_TASK_TABS;
        let harness = harness_with_config(config);
        let client = trusted_user_client(&harness, "runtime-isolated-bound");
        assert_eq!(
            harness.hub.resource_policy().await.max_task_open_lanes,
            crate::MAX_TASK_OPEN_LANES
        );
        let mut lane_ids = Vec::new();
        for index in 0..crate::MAX_TASK_OPEN_LANES {
            lane_ids.push(
                client
                    .open(
                        Some(&format!("isolated-bound-{index}")),
                        BrowserIdentityMode::Isolated,
                        None,
                    )
                    .await
                    .unwrap()
                    .lane()
                    .lane_id
                    .clone(),
            );
        }
        let budget = harness.hub.inner.cleanup_budget.snapshot();
        let task_id = client.task_resource_key();
        assert_eq!(budget.tasks[&task_id].count, crate::MAX_TASK_OPEN_LANES * 2);
        assert!(!budget.tasks[&task_id].latched);

        let overflow = client
            .open(
                Some("isolated-bound-overflow"),
                BrowserIdentityMode::Isolated,
                None,
            )
            .await
            .unwrap();
        assert!(matches!(overflow, OpenLaneOutcome::Queued { .. }));
        assert_eq!(
            harness.hub.inner.cleanup_budget.snapshot().tasks[&task_id].count,
            crate::MAX_TASK_OPEN_LANES * 2,
            "a policy-queued Lane must not reserve physical cleanup authority"
        );

        harness.hub.close_all().await.unwrap();
        assert!(harness.hub.inner.cleanup_budget.snapshot().tasks.is_empty());
        assert_eq!(lane_ids.len(), crate::MAX_TASK_OPEN_LANES);
    }

    #[tokio::test]
    async fn anonymous_navigation_limit_dispatches_n_and_fences_n_plus_one() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.max_navigations = 2;
        config.anonymous_profile_policy.sample_navigation_interval = 8;
        let harness = harness_with_config(config);
        let lane = open_identity(
            &harness.client,
            "anonymous-navigation-ceiling",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        harness.probe.releases.add_permits(2);

        harness.client.execute(&lane, navigate()).await.unwrap();
        harness.client.execute(&lane, navigate()).await.unwrap();
        let error = harness.client.execute(&lane, navigate()).await.unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(harness.probe.entries.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn anonymous_profile_footprint_equality_fails_closed_before_dispatch() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.max_bytes = 10;
        config.anonymous_profile_policy.sample_navigation_interval = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "anonymous-profile-byte-ceiling",
            BrowserIdentityMode::Anonymous,
        )
        .await;

        let error = harness.client.execute(&lane, navigate()).await.unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted);
        assert_eq!(harness.probe.entries.load(Ordering::Acquire), 0);
        assert_eq!(
            harness.probe.profile_footprint_calls.load(Ordering::Acquire),
            1
        );
    }

    #[tokio::test]
    async fn anonymous_shutdown_failure_retains_exact_fence_without_replacement() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.max_navigations = 1;
        config.anonymous_profile_policy.sample_navigation_interval = 8;
        let harness = harness_with_config(config);
        let lane = open_identity(
            &harness.client,
            "anonymous-cleanup-failure",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let old_epoch = harness.client.status(&lane).await.unwrap().browser_epoch;
        harness.probe.releases.add_permits(1);
        harness.client.execute(&lane, navigate()).await.unwrap();
        harness
            .probe
            .host_shutdown_fail_from
            .store(1, Ordering::Release);

        let _ = harness.client.execute(&lane, navigate()).await.unwrap_err();
        tokio::time::timeout(
            Duration::from_secs(1),
            harness.probe.wait_for_host_shutdowns(1),
        )
        .await
        .expect("Anonymous exact Host cleanup was not attempted");

        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane);
        assert_eq!(harness.hub.anonymous_profile_fence_epoch(&key), Some(old_epoch));
        assert!(
            harness
                .hub
                .inner
                .orphaned_host_slots
                .lock()
                .await
                .iter()
                .any(|(pending_key, slot)| pending_key == &key && slot.epoch == old_epoch)
        );
    }

    #[tokio::test]
    async fn anonymous_rotation_worker_panic_removes_membership_and_rearms_fence() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.max_navigations = 1;
        config.anonymous_profile_policy.sample_navigation_interval = 8;
        let harness = harness_with_config(config);
        let lane = open_identity(
            &harness.client,
            "anonymous-rotation-worker-panic",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let old_epoch = harness.client.status(&lane).await.unwrap().browser_epoch;
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane);
        harness.probe.releases.add_permits(1);
        harness.client.execute(&lane, navigate()).await.unwrap();
        harness
            .hub
            .inner
            .anonymous_profile_rotation_panics_remaining
            .store(1, Ordering::Release);

        let error = harness.client.execute(&lane, navigate()).await.unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserRestarted);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let panic_consumed = harness
                    .hub
                    .inner
                    .anonymous_profile_rotation_panics_remaining
                    .load(Ordering::Acquire)
                    == 0;
                let worker_absent = !harness
                    .hub
                    .inner
                    .anonymous_profile_rotation_workers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains(&key);
                if panic_consumed && worker_absent {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("panicked Anonymous rotation worker retained its membership key");
        harness.hub.autonomous_cleanup_pass().await.unwrap();

        let convergence = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let worker_absent = !harness
                    .hub
                    .inner
                    .anonymous_profile_rotation_workers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains(&key);
                let fence_absent = harness.hub.anonymous_profile_fence_epoch(&key).is_none();
                let replacement_published = harness
                    .client
                    .status(&lane)
                    .await
                    .is_ok_and(|snapshot| snapshot.browser_epoch > old_epoch);
                if worker_absent && fence_absent && replacement_published {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        if convergence.is_err() {
            panic!(
                "panic-safe Anonymous rotation worker did not re-arm and converge: fence={:?}, workers={}, launches={}, shutdowns={}, orphaned={}, retiring={}, status={:?}",
                harness.hub.anonymous_profile_fence_epoch(&key),
                harness
                    .hub
                    .inner
                    .anonymous_profile_rotation_workers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .len(),
                harness.factory.launches.load(Ordering::Acquire),
                harness.probe.host_shutdowns.load(Ordering::Acquire),
                harness.hub.inner.orphaned_host_slots.lock().await.len(),
                harness.hub.inner.retiring_host_keys.read().await.len(),
                harness.client.status(&lane).await,
            );
        }
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 2);
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn anonymous_profile_sample_abort_storm_remains_one_host_owned_flight() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.sample_navigation_interval = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .block_profile_footprint
            .store(true, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "anonymous-profile-abort-storm",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane);
        let slot = harness
            .hub
            .inner
            .host_slots
            .read()
            .await
            .get(&key)
            .cloned()
            .expect("Anonymous Host slot must exist");

        let first = {
            let client = harness.client.clone();
            let lane = lane.clone();
            tokio::spawn(async move { client.execute(&lane, navigate()).await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if harness
                    .probe
                    .profile_footprint_calls
                    .load(Ordering::Acquire)
                    == 1
                    && harness
                        .probe
                        .profile_footprint_active
                        .load(Ordering::Acquire)
                        == 1
                {
                    break;
                }
                harness.probe.profile_footprint_changed.notified().await;
            }
        })
        .await
        .expect("the first profile sample did not enter its driver");
        first.abort();
        let _ = first.await;

        // Every successor reaches the sample decision after the previous
        // request is fully aborted. None may release the Host-owned flight or
        // invoke the driver a second time while its first walk is blocked.
        for expected_navigation_count in 2..=33 {
            let request = {
                let client = harness.client.clone();
                let lane = lane.clone();
                tokio::spawn(async move { client.execute(&lane, navigate()).await })
            };
            tokio::time::timeout(Duration::from_secs(1), async {
                while slot.profile_navigation_count.load(Ordering::Acquire)
                    < expected_navigation_count
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("aborted successor did not reach the shared sample flight");
            request.abort();
            let _ = request.await;
            assert_eq!(
                harness
                    .probe
                    .profile_footprint_calls
                    .load(Ordering::Acquire),
                1,
                "request cancellation spawned an overlapping profile walk"
            );
            assert_eq!(
                harness
                    .probe
                    .profile_footprint_active
                    .load(Ordering::Acquire),
                1
            );
            assert_eq!(
                harness
                    .probe
                    .profile_footprint_maximum
                    .load(Ordering::Acquire),
                1
            );
        }

        harness
            .probe
            .block_profile_footprint
            .store(false, Ordering::Release);
        harness.probe.profile_footprint_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while harness
                .probe
                .profile_footprint_active
                .load(Ordering::Acquire)
                != 0
            {
                harness.probe.profile_footprint_changed.notified().await;
            }
        })
        .await
        .expect("the released Host-owned profile sample did not finish");

        // The completed one-item mailbox survives cancellation. This request
        // consumes that exact result and dispatches without creating a second
        // measurement; the next due navigation can then start a fresh sample.
        harness.probe.releases.add_permits(1);
        harness.client.execute(&lane, navigate()).await.unwrap();
        assert_eq!(
            harness
                .probe
                .profile_footprint_calls
                .load(Ordering::Acquire),
            1
        );
        harness.probe.releases.add_permits(1);
        harness.client.execute(&lane, navigate()).await.unwrap();
        assert_eq!(
            harness
                .probe
                .profile_footprint_calls
                .load(Ordering::Acquire),
            2,
            "a consumed sample flight must permit one later bounded sample"
        );
        assert_eq!(
            harness
                .probe
                .profile_footprint_maximum
                .load(Ordering::Acquire),
            1
        );
        harness.hub.close_all().await.unwrap();
    }

    #[tokio::test]
    async fn anonymous_profile_sample_panic_and_error_do_not_wedge_single_flight() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.sample_navigation_interval = 1;
        let harness = harness_with_config(config);
        let lane = open_identity(
            &harness.client,
            "anonymous-profile-sample-retry",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let key = HostKey::for_lane(BrowserIdentityMode::Anonymous, 0, &lane);
        let slot = harness
            .hub
            .inner
            .host_slots
            .read()
            .await
            .get(&key)
            .cloned()
            .expect("Anonymous Host slot must exist");
        let policy = harness
            .hub
            .inner
            .config
            .read()
            .await
            .anonymous_profile_policy
            .clone();

        harness
            .probe
            .profile_footprint_panics_remaining
            .store(1, Ordering::Release);
        assert_eq!(
            harness
                .hub
                .anonymous_profile_limit_reason(
                    &slot,
                    &policy,
                    harness.clock.now_ms(),
                    true,
                )
                .await,
            Some("footprint_measurement_failed")
        );
        assert_eq!(
            harness
                .hub
                .anonymous_profile_limit_reason(
                    &slot,
                    &policy,
                    harness.clock.now_ms(),
                    true,
                )
                .await,
            None,
            "a panicked sample must release its exact flight for retry"
        );

        harness
            .probe
            .profile_footprint_fail
            .store(true, Ordering::Release);
        assert_eq!(
            harness
                .hub
                .anonymous_profile_limit_reason(
                    &slot,
                    &policy,
                    harness.clock.now_ms(),
                    true,
                )
                .await,
            Some("footprint_measurement_failed")
        );
        harness
            .probe
            .profile_footprint_fail
            .store(false, Ordering::Release);
        assert_eq!(
            harness
                .hub
                .anonymous_profile_limit_reason(
                    &slot,
                    &policy,
                    harness.clock.now_ms(),
                    true,
                )
                .await,
            None,
            "a failed sample must release its exact flight for retry"
        );

        assert_eq!(
            harness
                .probe
                .profile_footprint_calls
                .load(Ordering::Acquire),
            4
        );
        assert_eq!(
            harness
                .probe
                .profile_footprint_active
                .load(Ordering::Acquire),
            0
        );
        assert_eq!(
            harness
                .probe
                .profile_footprint_maximum
                .load(Ordering::Acquire),
            1
        );
        harness.hub.close_all().await.unwrap();
    }

    #[tokio::test]
    async fn anonymous_profile_sample_does_not_block_exact_host_shutdown() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.sample_navigation_interval = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .block_profile_footprint
            .store(true, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "anonymous-profile-shutdown",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let request = {
            let client = harness.client.clone();
            let lane = lane.clone();
            tokio::spawn(async move { client.execute(&lane, navigate()).await })
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            while harness
                .probe
                .profile_footprint_active
                .load(Ordering::Acquire)
                != 1
            {
                harness.probe.profile_footprint_changed.notified().await;
            }
        })
        .await
        .expect("the profile sample did not enter before shutdown");
        request.abort();
        let _ = request.await;

        tokio::time::timeout(Duration::from_secs(1), harness.hub.close_all())
            .await
            .expect("exact Host shutdown waited on a detached request")
            .expect("exact Host shutdown failed");
        assert_eq!(harness.probe.host_shutdowns.load(Ordering::Acquire), 1);
        assert!(harness.hub.inner.host_slots.read().await.is_empty());

        // The bounded native walk retains only its exact flight after Host
        // shutdown; releasing it still converges without creating a successor.
        harness
            .probe
            .block_profile_footprint
            .store(false, Ordering::Release);
        harness.probe.profile_footprint_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while harness
                .probe
                .profile_footprint_active
                .load(Ordering::Acquire)
                != 0
            {
                harness.probe.profile_footprint_changed.notified().await;
            }
        })
        .await
        .expect("the bounded profile sample did not converge after shutdown");
        assert_eq!(
            harness
                .probe
                .profile_footprint_calls
                .load(Ordering::Acquire),
            1
        );
        assert_eq!(
            harness
                .probe
                .profile_footprint_maximum
                .load(Ordering::Acquire),
            1
        );
    }

    #[tokio::test]
    async fn anonymous_blocked_profile_sample_stays_behind_operation_admission() {
        let mut config = HubConfig::default();
        config.anonymous_profile_policy.sample_navigation_interval = 1;
        let harness = harness_with_config(config);
        harness
            .probe
            .block_profile_footprint
            .store(true, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "anonymous-bounded-profile-sample",
            BrowserIdentityMode::Anonymous,
        )
        .await;
        let mut operations = Vec::new();
        for _ in 0..MAX_LANE_OPERATION_ADMISSIONS {
            let client = harness.client.clone();
            let lane = lane.clone();
            operations.push(tokio::spawn(async move {
                client.execute(&lane, navigate()).await
            }));
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let admitted = harness
                    .hub
                    .inner
                    .operation_admissions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .by_lane
                    .get(&lane)
                    .copied()
                    .unwrap_or_default();
                if admitted == MAX_LANE_OPERATION_ADMISSIONS
                    && harness
                        .probe
                        .profile_footprint_calls
                        .load(Ordering::Acquire)
                        == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocked Anonymous profile sample bypassed operation admission");

        let overflow = harness.client.execute(&lane, navigate()).await.unwrap_err();
        assert_eq!(overflow.code, BrowserErrorCode::BrowserCapacityQueued);
        assert_eq!(
            overflow.metadata["reason_code"],
            "browser_operation_capacity_busy"
        );
        assert_eq!(
            harness
                .probe
                .profile_footprint_calls
                .load(Ordering::Acquire),
            1,
            "overflow request reached the blocked profile sampler"
        );

        harness
            .probe
            .block_profile_footprint
            .store(false, Ordering::Release);
        harness.probe.profile_footprint_release.add_permits(1);
        harness
            .probe
            .releases
            .add_permits(MAX_LANE_OPERATION_ADMISSIONS);
        for operation in operations {
            operation.await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn task_download_ledger_survives_owner_and_host_rotation_until_last_owner_cleanup() {
        let authority = HubTaskDownloadAuthority::new();
        let family = "family-download-rotation";
        let owner_a = OwnerLeaseId::new();
        let owner_b = OwnerLeaseId::new();
        authority.register_owner(family, &owner_a).unwrap();
        authority.register_owner(family, &owner_b).unwrap();

        let first = authority
            .reserve(family, "lane-host-a", "guid-host-a")
            .await
            .unwrap();
        first.update_progress(64, Some(64)).unwrap();
        first.complete(64).unwrap();
        drop(first);
        authority.retire_owner(&owner_a);
        assert_eq!(authority.usage_for(family), Some((1, 64, 1, 0)));

        // A replacement runtime/Host receives the same family authority and
        // sees the already-completed charge rather than a fresh ledger.
        let replacement = authority
            .reserve(family, "lane-host-b", "guid-host-b")
            .await
            .unwrap();
        replacement.update_progress(32, Some(32)).unwrap();
        replacement.complete(32).unwrap();
        drop(replacement);
        assert_eq!(authority.usage_for(family), Some((1, 96, 2, 0)));

        // Defensive late-completion race: final owner retirement fences the
        // still-held reservation, but the already-consumed family remains
        // sticky across a real zero-owner interval.
        let late = authority
            .reserve(family, "lane-host-b", "guid-late")
            .await
            .unwrap();
        authority.retire_owner(&owner_b);
        assert!(late.complete(1).is_err());
        drop(late);
        assert_eq!(authority.usage_for(family), Some((0, 96, 2, 0)));

        let owner_c = OwnerLeaseId::new();
        authority.register_owner(family, &owner_c).unwrap();
        let after_gap = authority
            .reserve(family, "lane-host-c", "guid-host-c")
            .await
            .unwrap();
        after_gap.update_progress(4, Some(4)).unwrap();
        after_gap.complete(4).unwrap();
        authority.retire_owner(&owner_c);
        assert_eq!(authority.usage_for(family), Some((0, 100, 3, 0)));

        // Hub/application shutdown is the explicit global finalization proof.
        authority.clear();
        assert_eq!(authority.usage_for(family), None);
    }

    #[tokio::test]
    async fn task_download_ledger_enforces_active_single_and_cumulative_boundaries() {
        let authority = HubTaskDownloadAuthority::new();
        let family = "family-download-bounds";
        let owner = OwnerLeaseId::new();
        authority.register_owner(family, &owner).unwrap();

        let mut active = Vec::new();
        for index in 0..MAX_TASK_ACTIVE_DOWNLOADS {
            active.push(
                authority
                    .reserve(family, "lane", &format!("active-{index}"))
                    .await
                    .unwrap(),
            );
        }
        assert!(
            authority
                .reserve(family, "lane", "active-overflow")
                .await
                .is_err()
        );
        drop(active);

        let oversized = authority
            .reserve(family, "lane", "oversized")
            .await
            .unwrap();
        assert!(
            oversized
                .update_progress(MAX_TASK_SINGLE_DOWNLOAD_BYTES + 1, None)
                .is_err()
        );
        drop(oversized);

        for index in 0..2 {
            let completed = authority
                .reserve(family, "lane", &format!("half-{index}"))
                .await
                .unwrap();
            completed
                .update_progress(
                    MAX_TASK_SINGLE_DOWNLOAD_BYTES,
                    Some(MAX_TASK_SINGLE_DOWNLOAD_BYTES),
                )
                .unwrap();
            completed
                .complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES)
                .unwrap();
        }
        assert_eq!(
            authority.usage_for(family),
            Some((1, MAX_TASK_COMPLETED_DOWNLOAD_BYTES, 2, 0))
        );
        assert!(
            authority
                .reserve(family, "lane", "cumulative-overflow")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn task_download_sticky_family_table_fails_closed_at_structural_limit() {
        let authority = HubTaskDownloadAuthority::new();
        for index in 0..MAX_RETAINED_COMPLETED_DOWNLOAD_FAMILIES {
            let family = format!("sticky-family-{index}");
            let owner = OwnerLeaseId::new();
            authority.register_owner(&family, &owner).unwrap();
            let reservation = authority
                .reserve(&family, "lane", "completed")
                .await
                .unwrap();
            reservation.update_progress(1, Some(1)).unwrap();
            reservation.complete(1).unwrap();
            drop(reservation);
            authority.retire_owner(&owner);
            assert_eq!(authority.usage_for(&family), Some((0, 1, 1, 0)));
        }

        let overflow_family = "sticky-family-overflow";
        let overflow_owner = OwnerLeaseId::new();
        authority
            .register_owner(overflow_family, &overflow_owner)
            .unwrap();
        let overflow = authority
            .reserve(overflow_family, "lane", "completed")
            .await
            .unwrap();
        overflow.update_progress(1, Some(1)).unwrap();
        let error = overflow
            .complete(1)
            .expect_err("N+1 completed family must fail closed without eviction");
        assert_eq!(
            error.metadata["reason_code"],
            "browser_task_download_family_capacity"
        );
        drop(overflow);
        authority.retire_owner(&overflow_owner);
        assert_eq!(authority.usage_for(overflow_family), None);
        assert_eq!(
            authority.usage_for("sticky-family-0"),
            Some((0, 1, 1, 0)),
            "the table must never TTL/LRU-evict an already-consumed family"
        );
    }

    #[tokio::test]
    async fn task_download_rebind_cycles_inherit_cumulative_cap_until_hub_clear() {
        let authority = HubTaskDownloadAuthority::new();
        let family = "family-download-rebind-cycles";
        let quarter = MAX_TASK_COMPLETED_DOWNLOAD_BYTES / 4;

        // Four successive owner generations, each separated by a real
        // zero-owner interval (child restart, renew failure, login reopen).
        for cycle in 0..4 {
            let owner = OwnerLeaseId::new();
            authority.register_owner(family, &owner).unwrap();
            let reservation = authority
                .reserve(family, "lane", &format!("cycle-{cycle}"))
                .await
                .unwrap();
            reservation.update_progress(quarter, Some(quarter)).unwrap();
            reservation.complete(quarter).unwrap();
            drop(reservation);
            authority.retire_owner(&owner);
            let (owners, bytes, files, active) =
                authority.usage_for(family).expect("family stays sticky");
            assert_eq!(owners, 0, "the zero-owner interval is real");
            assert_eq!(active, 0);
            assert_eq!(files, cycle + 1);
            assert_eq!(
                bytes,
                quarter * (cycle as u64 + 1),
                "each rebind inherits the accumulated family charge"
            );
        }

        // The fifth rebind inherits a saturated 1 GiB ledger: owner rotation
        // cannot wash a task's download budget.
        let owner = OwnerLeaseId::new();
        authority.register_owner(family, &owner).unwrap();
        assert_eq!(
            authority.usage_for(family),
            Some((1, MAX_TASK_COMPLETED_DOWNLOAD_BYTES, 4, 0))
        );
        assert!(
            authority
                .reserve(family, "lane", "cycle-overflow")
                .await
                .is_err(),
            "a rebind cycle must not mint fresh cumulative download bytes"
        );

        // Only Hub/application shutdown is global finalization proof.
        authority.clear();
        assert_eq!(authority.usage_for(family), None);
        let successor = OwnerLeaseId::new();
        authority.register_owner(family, &successor).unwrap();
        let fresh = authority
            .reserve(family, "lane", "post-clear")
            .await
            .expect("a new Hub lifetime starts from an empty ledger");
        fresh.update_progress(1, Some(1)).unwrap();
        fresh.complete(1).unwrap();
        assert_eq!(authority.usage_for(family), Some((1, 1, 1, 0)));
    }

    /// Primary fence harness: a footprint-limited policy whose Primary Host is
    /// launched but not yet sampled.
    fn primary_fence_harness() -> Harness {
        let mut config = HubConfig::default();
        config.primary_profile_policy.max_bytes = 10;
        config.primary_profile_policy.sample_navigation_interval = 1;
        harness_with_config(config)
    }

    fn assert_primary_fence_error(error: &BrowserPlatformError) {
        assert_eq!(error.code, BrowserErrorCode::PrimaryProfileStorageLimit);
        assert_eq!(error.metadata["primary_profile_fenced"], true);
        assert_eq!(error.metadata["persistent_identity_preserved"], true);
        assert_eq!(error.metadata["automatic_profile_deletion"], false);
    }

    #[tokio::test]
    async fn primary_first_observe_samples_and_fences_before_dispatch() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "primary-first-observe",
            BrowserIdentityMode::Primary,
        )
        .await;

        let error = harness.client.execute(&lane, observe()).await.unwrap_err();

        assert_primary_fence_error(&error);
        assert_eq!(
            harness.probe.profile_footprint_calls.load(Ordering::Acquire),
            1,
            "the first Primary operation forces one sample"
        );
        assert_eq!(
            harness.probe.entries.load(Ordering::Acquire),
            0,
            "no operation may reach the browser after the boundary is detected"
        );
        assert!(harness.hub.primary_profile_fence().is_some());
    }

    #[tokio::test]
    async fn primary_public_sweep_samples_silent_host() {
        let harness = primary_fence_harness();
        let lane = open_identity(
            &harness.client,
            "primary-silent-host",
            BrowserIdentityMode::Primary,
        )
        .await;
        // The Host is live but never dispatched an operation.
        assert!(harness.hub.primary_profile_fence().is_none());
        assert_eq!(
            harness.probe.profile_footprint_calls.load(Ordering::Acquire),
            0
        );

        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        harness.clock.advance(20_000);
        harness.hub.sweep_primary_profile_hygiene().await;

        assert!(
            harness.hub.primary_profile_fence().is_some(),
            "the periodic sweep must sample a silent Primary Host"
        );
        let error = harness.client.execute(&lane, observe()).await.unwrap_err();
        assert_primary_fence_error(&error);
    }

    #[tokio::test]
    async fn primary_cold_launch_is_never_fenced_by_the_sweep() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        harness
            .probe
            .block_host_launch
            .store(true, Ordering::Release);
        let client = harness.client.clone();
        let opening = tokio::spawn(async move {
            client
                .open(
                    Some("primary-cold-launch"),
                    BrowserIdentityMode::Primary,
                    None,
                )
                .await
        });
        // Wait until the slot exists with no published driver yet.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let cold = harness
                    .hub
                    .inner
                    .host_slots
                    .read()
                    .await
                    .iter()
                    .any(|(key, slot)| {
                        key.identity_mode == BrowserIdentityMode::Primary
                            && slot.get().is_none()
                    });
                if cold {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a cold Primary launch slot was never observed");

        harness.clock.advance(20_000);
        harness.hub.sweep_primary_profile_hygiene().await;
        assert!(
            harness.hub.primary_profile_fence().is_none(),
            "a launch whose driver is not published yet must never be fenced"
        );
        assert_eq!(
            harness.probe.profile_footprint_calls.load(Ordering::Acquire),
            0,
            "a cold slot has no driver to sample"
        );

        harness.probe.host_launch_release.add_permits(1);
        harness
            .probe
            .block_host_launch
            .store(false, Ordering::Release);
        let _ = opening.await.expect("cold launch task joins");
    }

    #[tokio::test]
    async fn primary_sticky_fence_blocks_existing_and_new_open() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let existing = open_identity(
            &harness.client,
            "primary-sticky-existing",
            BrowserIdentityMode::Primary,
        )
        .await;
        let trigger = harness
            .client
            .execute(&existing, observe())
            .await
            .unwrap_err();
        assert_primary_fence_error(&trigger);

        // An already-open Lane cannot dispatch again...
        let repeat = harness
            .client
            .execute(&existing, observe())
            .await
            .unwrap_err();
        assert_primary_fence_error(&repeat);
        // ...and a brand new Primary open is refused at admission.
        let refused = harness
            .client
            .open(
                Some("primary-sticky-new"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_primary_fence_error(&refused);

        // Exact cleanup completion clears the cleanup epoch but never the
        // process-lifetime sticky fence.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let epochs_drained = harness
                    .hub
                    .inner
                    .primary_profile_cleanup_epochs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_empty();
                if epochs_drained {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exact Primary cleanup never converged");
        let after_cleanup = harness
            .client
            .open(
                Some("primary-sticky-after-cleanup"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_primary_fence_error(&after_cleanup);
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "a fenced Primary profile is never relaunched in this Hub lifetime"
        );
    }

    #[tokio::test]
    async fn primary_shutdown_failure_retries_exact_epoch_without_replacement() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "primary-cleanup-failure",
            BrowserIdentityMode::Primary,
        )
        .await;
        let fenced_epoch = harness.client.status(&lane).await.unwrap().browser_epoch;
        harness
            .probe
            .host_shutdown_fail_from
            .store(1, Ordering::Release);

        let error = harness.client.execute(&lane, observe()).await.unwrap_err();
        assert_primary_fence_error(&error);
        tokio::time::timeout(
            Duration::from_secs(5),
            harness.probe.wait_for_host_shutdowns(1),
        )
        .await
        .expect("exact Primary Host cleanup was never attempted");

        // A failing shutdown retains the exact epoch as cleanup debt and does
        // not start a replacement Host.
        assert!(
            harness
                .hub
                .inner
                .primary_profile_cleanup_epochs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&fenced_epoch),
            "a failed exact cleanup must retain its epoch authority"
        );
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert!(harness.hub.primary_profile_fence().is_some());

        // Once the injected failure is lifted the retained epoch converges,
        // still without a replacement launch and still fenced.
        harness
            .probe
            .host_shutdown_fail_from
            .store(usize::MAX, Ordering::Release);
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let drained = harness
                    .hub
                    .inner
                    .primary_profile_cleanup_epochs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_empty();
                if drained {
                    break;
                }
                harness.hub.sweep_primary_profile_hygiene().await;
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retained exact Primary cleanup debt never converged");
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
        assert!(
            harness.hub.primary_profile_fence().is_some(),
            "cleanup completion must not lift the sticky fence"
        );
    }

    #[tokio::test]
    async fn primary_worker_panic_rearms_exact_cleanup() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "primary-worker-panic",
            BrowserIdentityMode::Primary,
        )
        .await;
        let fenced_epoch = harness.client.status(&lane).await.unwrap().browser_epoch;
        harness
            .hub
            .inner
            .primary_profile_cleanup_panics_remaining
            .store(1, Ordering::Release);

        let error = harness.client.execute(&lane, observe()).await.unwrap_err();
        assert_primary_fence_error(&error);

        // The panicking worker releases its membership without losing the
        // exact epoch, so the debt is still armed.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let panic_consumed = harness
                    .hub
                    .inner
                    .primary_profile_cleanup_panics_remaining
                    .load(Ordering::Acquire)
                    == 0;
                let worker_absent = !harness
                    .hub
                    .inner
                    .primary_profile_cleanup_workers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains(&fenced_epoch);
                if panic_consumed && worker_absent {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the panicking Primary cleanup worker retained its membership");

        // Re-arming converges the same retained debt.
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                harness.hub.rearm_pending_primary_profile_cleanup();
                let drained = harness
                    .hub
                    .inner
                    .primary_profile_cleanup_epochs
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_empty();
                if drained {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a panicked Primary cleanup worker did not re-arm and converge");
        assert!(harness.hub.primary_profile_fence().is_some());
        assert_eq!(harness.factory.launches.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn primary_visibility_restart_cannot_bypass_sticky_fence() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "primary-visibility-bypass",
            BrowserIdentityMode::Primary,
        )
        .await;
        let error = harness.client.execute(&lane, observe()).await.unwrap_err();
        assert_primary_fence_error(&error);
        let launches_when_fenced = harness.factory.launches.load(Ordering::Acquire);

        // A visibility transition is a central launch entry point; it must
        // observe the fence instead of minting a replacement Primary Host.
        let _ = harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headful)
            .await;
        let _ = harness
            .hub
            .set_primary_visibility(BrowserVisibility::Headless)
            .await;

        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            launches_when_fenced,
            "a visibility restart must not bypass the sticky Primary fence"
        );
        assert!(harness.hub.primary_profile_fence().is_some());
        let refused = harness
            .client
            .open(
                Some("primary-after-visibility"),
                BrowserIdentityMode::Primary,
                None,
            )
            .await
            .unwrap_err();
        assert_primary_fence_error(&refused);
    }

    /// The stable Primary profile is never deleted by the fence path: the Hub
    /// stops the exact Host and preserves identity data. Physical sentinel
    /// survival is asserted by the real-Chrome `integration_managed_host`
    /// ignored suite; at this layer the contract surface is the absence of any
    /// profile-deletion authority plus the honest error metadata.
    #[tokio::test]
    async fn primary_fence_preserves_identity_data_and_never_deletes_the_profile() {
        let harness = primary_fence_harness();
        harness
            .probe
            .profile_footprint_bytes
            .store(10, Ordering::Release);
        let lane = open_identity(
            &harness.client,
            "primary-profile-preserved",
            BrowserIdentityMode::Primary,
        )
        .await;

        let error = harness.client.execute(&lane, observe()).await.unwrap_err();

        assert_primary_fence_error(&error);
        assert_eq!(error.metadata["profile_hygiene"], "footprint_limit");
        assert!(
            !error.retryable,
            "a fenced Primary profile needs a deliberate user action, not a retry"
        );
        tokio::time::timeout(
            Duration::from_secs(5),
            harness.probe.wait_for_host_shutdowns(1),
        )
        .await
        .expect("the fence must stop the exact Primary Host");
        assert_eq!(
            harness.factory.launches.load(Ordering::Acquire),
            1,
            "stopping is not rotating: no replacement Primary profile is created"
        );
    }
}
