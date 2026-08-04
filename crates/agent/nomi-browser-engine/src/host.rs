//! Shared Chromium host and lane ownership primitives.
//!
//! A [`ManagedBrowserHost`] owns exactly one Chromium browser instance (one
//! operating-system process tree) and one CDP connection. Chromium normally
//! splits that instance into browser, renderer, GPU, network/utility and other
//! child processes. Each lane gets a separate [`BrowserEngine`] value backed
//! by the shared connection, with independent tab/cursor/ref/cancellation state.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::future::join_all;
use tokio::sync::Mutex;

use crate::backend::cdp::{CdpBackend, CdpHostRuntime};
use crate::{
    BrowserEngine, BrowserError, BrowserHostLaunchMode, EngineConfig, KnownSecretValues,
};

const HOST_LANE_CLOSE_GRACE: Duration = Duration::from_millis(750);

/// Structural limits for the legacy/standalone public engine path. Platform
/// Hub Hosts use their own mandatory cross-Host authority and are not charged
/// to these compatibility limits.
pub const STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE: usize = 4;
pub const STANDALONE_MAX_LIVE_LANES_PER_SCOPE: usize = 4;
pub const STANDALONE_MAX_LIVE_TABS_PER_SCOPE: usize = 16;
/// Standalone tasks use the same fixed task-lifetime output envelope as the
/// platform Hub. The scope is opaque and retained across its Host rotations.
pub const MAX_TASK_COMPLETED_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_TASK_SINGLE_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TASK_COMPLETED_DOWNLOAD_FILES: usize = 256;
pub const MAX_TASK_ACTIVE_DOWNLOADS: usize = 4;

/// Trusted opaque authority which must remain live until this Host's exact
/// process-tree and profile cleanup has completed.
///
/// The engine deliberately cannot inspect or mint the wrapped authority.  A
/// platform adapter may place its Hub cleanup-budget ticket here before the
/// first launch await; standalone callers wrap their structural Host lease in
/// the same type.  Every launch/cancellation path then moves this value with
/// the indivisible process/profile cleanup authority.
#[derive(Clone)]
pub struct HostCleanupLease {
    _authority: Arc<dyn Send + Sync>,
}

impl HostCleanupLease {
    /// Wrap one trusted, process-internal cleanup authority.  Dropping the
    /// final engine clone is the only operation the engine performs on it.
    pub fn new<T>(authority: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        Self {
            _authority: Arc::new(authority),
        }
    }
}

impl std::fmt::Debug for HostCleanupLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostCleanupLease")
            .field("opaque", &true)
            .finish()
    }
}

static NEXT_STANDALONE_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
struct StandaloneResourceState {
    live_hosts: usize,
    live_lanes: usize,
    live_tabs: usize,
    tab_reservations: HashMap<String, Weak<StandaloneTabReservation>>,
    completed_download_bytes: u64,
    completed_download_files: usize,
    active_downloads: HashMap<String, StandaloneActiveDownload>,
}

struct StandaloneActiveDownload {
    reservation: Weak<StandaloneDownloadReservation>,
    accounted_bytes: u64,
    completion_prepared: bool,
}

struct StandaloneResourceScopeInner {
    task_resource_key: String,
    state: std::sync::Mutex<StandaloneResourceState>,
}

/// Trusted, non-serializable resource scope for the standalone engine path.
///
/// Each `BrowserTool` creates one scope and threads it through `EngineConfig`.
/// Older callers which use `EngineConfig::default()` are conservatively folded
/// into one process-wide compatibility scope, so changing Lane ids cannot mint
/// fresh Host/Lane/tab budgets.
#[derive(Clone)]
pub struct StandaloneResourceScope {
    inner: Arc<StandaloneResourceScopeInner>,
}

impl std::fmt::Debug for StandaloneResourceScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandaloneResourceScope")
            .field("opaque", &true)
            .finish()
    }
}

impl Default for StandaloneResourceScope {
    fn default() -> Self {
        static COMPATIBILITY_SCOPE: OnceLock<StandaloneResourceScope> = OnceLock::new();
        COMPATIBILITY_SCOPE
            .get_or_init(|| StandaloneResourceScope::with_key("standalone:compatibility".into()))
            .clone()
    }
}

impl StandaloneResourceScope {
    /// Create a fresh trusted task scope. The opaque identity is generated in
    /// process and is never accepted from model/tool arguments.
    pub fn new() -> Self {
        let id = NEXT_STANDALONE_SCOPE_ID.fetch_add(1, Ordering::Relaxed);
        Self::with_key(format!("standalone:task:{id}"))
    }

    /// Opaque identity check for trusted facade/task factories and contract
    /// tests. This exposes no key and cannot mint or select model-controlled
    /// capacity.
    #[doc(hidden)]
    pub fn shares_budget_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn with_key(task_resource_key: String) -> Self {
        Self {
            inner: Arc::new(StandaloneResourceScopeInner {
                task_resource_key,
                state: std::sync::Mutex::new(StandaloneResourceState::default()),
            }),
        }
    }

    /// Trusted opaque task key used when an internal standalone adapter needs
    /// to wire the same Lane/tab reservation protocol as `ManagedBrowserHost`.
    /// The key is never accepted from model input and cannot mint a new scope.
    pub(crate) fn task_resource_key(&self) -> &str {
        &self.inner.task_resource_key
    }

    pub(crate) fn reserve_host(&self) -> Result<StandaloneHostLease, BrowserError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.live_hosts >= STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE {
            return Err(standalone_capacity_error(
                "Host",
                STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE,
            ));
        }
        state.live_hosts += 1;
        Ok(StandaloneHostLease::new(Arc::clone(&self.inner)))
    }

    pub(crate) fn reserve_lane(
        &self,
        lane_id: LaneId,
    ) -> Result<Arc<StandaloneLaneAuthority>, BrowserError> {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.live_lanes >= STANDALONE_MAX_LIVE_LANES_PER_SCOPE {
            return Err(standalone_capacity_error(
                "Lane",
                STANDALONE_MAX_LIVE_LANES_PER_SCOPE,
            ));
        }
        state.live_lanes += 1;
        drop(state);
        Ok(Arc::new(StandaloneLaneAuthority {
            scope: self.clone(),
            lane_id,
            released: AtomicBool::new(false),
        }))
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.live_hosts, state.live_lanes, state.live_tabs)
    }
}

fn standalone_capacity_error(kind: &str, limit: usize) -> BrowserError {
    BrowserError::Blocked {
        reason: format!(
            "standalone browser task reached its live {kind} safety limit ({limit}); reuse or close an existing resource"
        ),
    }
}

/// RAII authority held by `CdpHostRuntime`, not the temporary
/// `ManagedBrowserHost` wrapper returned during `create_engine`.
pub(crate) struct StandaloneHostLease {
    scope: Arc<StandaloneResourceScopeInner>,
    released: AtomicBool,
}

impl StandaloneHostLease {
    fn new(scope: Arc<StandaloneResourceScopeInner>) -> Self {
        Self {
            scope,
            released: AtomicBool::new(false),
        }
    }

    pub(crate) fn release(&self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut state = self
            .scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(state.live_hosts > 0, "standalone Host lease underflow");
        state.live_hosts = state.live_hosts.saturating_sub(1);
    }
}

impl Drop for StandaloneHostLease {
    fn drop(&mut self) {
        self.release();
    }
}

pub(crate) struct StandaloneLaneAuthority {
    scope: StandaloneResourceScope,
    lane_id: LaneId,
    released: AtomicBool,
}

impl StandaloneLaneAuthority {
    fn release(&self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut state = self
            .scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(state.live_lanes > 0, "standalone Lane lease underflow");
        state.live_lanes = state.live_lanes.saturating_sub(1);
    }
}

impl Drop for StandaloneLaneAuthority {
    fn drop(&mut self) {
        self.release();
    }
}

struct StandaloneTabReservation {
    scope: Weak<StandaloneResourceScopeInner>,
    reservation_key: String,
}

impl TaskTabReservation for StandaloneTabReservation {}

impl Drop for StandaloneTabReservation {
    fn drop(&mut self) {
        let Some(scope) = self.scope.upgrade() else {
            return;
        };
        let mut state = scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let is_current = state
            .tab_reservations
            .get(&self.reservation_key)
            .is_some_and(|current| current.as_ptr() == self as *const Self);
        if is_current {
            state.tab_reservations.remove(&self.reservation_key);
            debug_assert!(state.live_tabs > 0, "standalone tab lease underflow");
            state.live_tabs = state.live_tabs.saturating_sub(1);
        }
    }
}

/// Per-lane correctness gate used by the production backend.  There is no
/// host-global operation mutex: every call to `open_lane` constructs a new
/// instance of this gate.
#[derive(Default)]
pub(crate) struct LaneOperationGate(Arc<Mutex<()>>);

impl LaneOperationGate {
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }

    /// Owned form used by Host policy transactions which must lock a
    /// deterministic set of Lanes without borrowing the Lane values holding
    /// those gates.
    pub(crate) async fn lock_owned(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.0).lock_owned().await
    }
}
/// Stable caller-supplied identifier for one ownership/concurrency lane.
pub type LaneId = String;

/// Opaque task-wide top-level-tab permit. The authority's concrete permit
/// releases its slot when the last `Arc` is dropped; the engine never needs to
/// call back asynchronously from `TabRecord::drop`.
pub trait TaskTabReservation: Send + Sync {}

/// Cross-Host task tab admission authority supplied by the platform Hub.
///
/// `reservation_key` is stable for one target creation attempt (the inert
/// pending URL for explicit creates, the full target id for browser-created
/// popups). Implementations must be idempotent for the same task/key so a
/// duplicate attach event cannot consume a second slot.
#[async_trait]
pub trait TaskTabReservationAuthority: Send + Sync {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        reservation_key: &str,
    ) -> Result<Arc<dyn TaskTabReservation>, BrowserError>;

    /// Release the Lane-level structural reservation, if this authority owns
    /// one. Platform Hub authorities use the default no-op because their Lane
    /// capacity is owned by the Hub scheduler.
    fn release_lane(&self) {}
}

/// One active task download. Filesystem outputs use a two-phase completion:
/// reserve the final charge, atomically publish, then finalize without await.
pub trait TaskDownloadReservation: Send + Sync {
    fn update_progress(
        &self,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), BrowserError>;

    fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserError>;

    fn finalize_complete(&self);

    fn complete(&self, actual_bytes: u64) -> Result<(), BrowserError> {
        self.prepare_complete(actual_bytes)?;
        self.finalize_complete();
        Ok(())
    }
}

/// Task-global download admission authority. Production supplies a Hub bridge;
/// standalone callers use their opaque [`StandaloneResourceScope`].
#[async_trait]
pub trait TaskDownloadReservationAuthority: Send + Sync {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError>;
}

#[async_trait]
impl TaskTabReservationAuthority for StandaloneLaneAuthority {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        reservation_key: &str,
    ) -> Result<Arc<dyn TaskTabReservation>, BrowserError> {
        if self.released.load(Ordering::Acquire)
            || task_resource_key != self.scope.task_resource_key()
            || lane_id != self.lane_id
        {
            return Err(BrowserError::Blocked {
                reason: "standalone browser tab reservation does not match its trusted task/Lane scope"
                    .into(),
            });
        }
        let reservation_key = format!("{lane_id}\0{reservation_key}");
        let mut state = self
            .scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = state
            .tab_reservations
            .get(&reservation_key)
            .and_then(Weak::upgrade)
        {
            return Ok(existing);
        }
        if state.live_tabs >= STANDALONE_MAX_LIVE_TABS_PER_SCOPE {
            return Err(standalone_capacity_error(
                "top-level tab",
                STANDALONE_MAX_LIVE_TABS_PER_SCOPE,
            ));
        }
        let reservation = Arc::new(StandaloneTabReservation {
            scope: Arc::downgrade(&self.scope.inner),
            reservation_key: reservation_key.clone(),
        });
        state.live_tabs += 1;
        state
            .tab_reservations
            .insert(reservation_key, Arc::downgrade(&reservation));
        Ok(reservation)
    }

    fn release_lane(&self) {
        self.release();
    }
}

struct StandaloneDownloadReservation {
    scope: Weak<StandaloneResourceScopeInner>,
    scoped_key: String,
    completed: AtomicBool,
}

impl TaskDownloadReservation for StandaloneDownloadReservation {
    fn update_progress(
        &self,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), BrowserError> {
        let proposed = received_bytes.max(total_bytes.unwrap_or(0));
        if proposed > MAX_TASK_SINGLE_DOWNLOAD_BYTES {
            return Err(standalone_download_capacity_error(
                "single-file byte",
                MAX_TASK_SINGLE_DOWNLOAD_BYTES,
            ));
        }
        let Some(scope) = self.scope.upgrade() else {
            return Err(standalone_download_retired_error());
        };
        let mut state = scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = state.active_downloads.get(&self.scoped_key) else {
            return Err(standalone_download_retired_error());
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self) {
            return Err(standalone_download_retired_error());
        }
        if current.completion_prepared {
            return Err(standalone_download_retired_error());
        }
        let proposed = proposed.max(current.accounted_bytes);
        let other_active = state
            .active_downloads
            .iter()
            .filter(|(key, _)| *key != &self.scoped_key)
            .fold(0u64, |total, (_, active)| {
                total.saturating_add(active.accounted_bytes)
            });
        if state
            .completed_download_bytes
            .saturating_add(other_active)
            .saturating_add(proposed)
            > MAX_TASK_COMPLETED_DOWNLOAD_BYTES
        {
            return Err(standalone_download_capacity_error(
                "cumulative byte",
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }
        state
            .active_downloads
            .get_mut(&self.scoped_key)
            .expect("validated standalone download remains under the scope lock")
            .accounted_bytes = proposed;
        Ok(())
    }

    fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserError> {
        if self.completed.load(Ordering::Acquire) {
            return Ok(());
        }
        if actual_bytes > MAX_TASK_SINGLE_DOWNLOAD_BYTES {
            return Err(standalone_download_capacity_error(
                "single-file byte",
                MAX_TASK_SINGLE_DOWNLOAD_BYTES,
            ));
        }
        let Some(scope) = self.scope.upgrade() else {
            return Err(standalone_download_retired_error());
        };
        let mut state = scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = state.active_downloads.get(&self.scoped_key) else {
            return Err(standalone_download_retired_error());
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self) {
            return Err(standalone_download_retired_error());
        }
        if current.completion_prepared {
            return if current.accounted_bytes == actual_bytes {
                Ok(())
            } else {
                Err(standalone_download_retired_error())
            };
        }
        if state.completed_download_files >= MAX_TASK_COMPLETED_DOWNLOAD_FILES {
            return Err(standalone_download_capacity_error(
                "completed file",
                MAX_TASK_COMPLETED_DOWNLOAD_FILES as u64,
            ));
        }
        let other_active = state
            .active_downloads
            .iter()
            .filter(|(key, _)| *key != &self.scoped_key)
            .fold(0u64, |total, (_, active)| {
                total.saturating_add(active.accounted_bytes)
            });
        if state
            .completed_download_bytes
            .saturating_add(other_active)
            .saturating_add(actual_bytes)
            > MAX_TASK_COMPLETED_DOWNLOAD_BYTES
        {
            return Err(standalone_download_capacity_error(
                "cumulative byte",
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }
        let current = state
            .active_downloads
            .get_mut(&self.scoped_key)
            .expect("validated standalone completion remains active");
        current.accounted_bytes = actual_bytes;
        current.completion_prepared = true;
        Ok(())
    }

    fn finalize_complete(&self) {
        if self.completed.load(Ordering::Acquire) {
            return;
        }
        let Some(scope) = self.scope.upgrade() else {
            return;
        };
        let mut state = scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = state.active_downloads.get(&self.scoped_key) else {
            return;
        };
        if !std::ptr::eq(current.reservation.as_ptr(), self as *const Self)
            || !current.completion_prepared
        {
            return;
        }
        let actual_bytes = current.accounted_bytes;
        state.completed_download_bytes = state
            .completed_download_bytes
            .saturating_add(actual_bytes);
        state.completed_download_files = state.completed_download_files.saturating_add(1);
        state.active_downloads.remove(&self.scoped_key);
        self.completed.store(true, Ordering::Release);
    }
}

impl Drop for StandaloneDownloadReservation {
    fn drop(&mut self) {
        if self.completed.load(Ordering::Acquire) {
            return;
        }
        let Some(scope) = self.scope.upgrade() else {
            return;
        };
        let mut state = scope
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let is_current = state
            .active_downloads
            .get(&self.scoped_key)
            .is_some_and(|active| {
                std::ptr::eq(active.reservation.as_ptr(), self as *const Self)
            });
        if is_current {
            state.active_downloads.remove(&self.scoped_key);
        }
    }
}

#[async_trait]
impl TaskDownloadReservationAuthority for StandaloneLaneAuthority {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        const MAX_DOWNLOAD_KEY_BYTES: usize = 4 * 1024;
        if self.released.load(Ordering::Acquire)
            || task_resource_key != self.scope.task_resource_key()
            || lane_id != self.lane_id
            || download_key.is_empty()
            || download_key.len() > MAX_DOWNLOAD_KEY_BYTES
        {
            return Err(standalone_download_retired_error());
        }
        let scoped_key = format!("{lane_id}\0{download_key}");
        let mut state = self
            .scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .active_downloads
            .retain(|_, active| active.reservation.strong_count() != 0);
        if let Some(existing) = state
            .active_downloads
            .get(&scoped_key)
            .and_then(|active| active.reservation.upgrade())
        {
            return Ok(existing);
        }
        if state.active_downloads.len() >= MAX_TASK_ACTIVE_DOWNLOADS {
            return Err(standalone_download_capacity_error(
                "active download",
                MAX_TASK_ACTIVE_DOWNLOADS as u64,
            ));
        }
        if state.completed_download_files >= MAX_TASK_COMPLETED_DOWNLOAD_FILES {
            return Err(standalone_download_capacity_error(
                "completed file",
                MAX_TASK_COMPLETED_DOWNLOAD_FILES as u64,
            ));
        }
        if state.completed_download_bytes >= MAX_TASK_COMPLETED_DOWNLOAD_BYTES {
            return Err(standalone_download_capacity_error(
                "cumulative byte",
                MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            ));
        }
        let reservation = Arc::new(StandaloneDownloadReservation {
            scope: Arc::downgrade(&self.scope.inner),
            scoped_key: scoped_key.clone(),
            completed: AtomicBool::new(false),
        });
        state.active_downloads.insert(
            scoped_key,
            StandaloneActiveDownload {
                reservation: Arc::downgrade(&reservation),
                accounted_bytes: 0,
                completion_prepared: false,
            },
        );
        Ok(reservation)
    }
}

fn standalone_download_capacity_error(kind: &str, limit: u64) -> BrowserError {
    BrowserError::Blocked {
        reason: format!(
            "standalone browser task reached its {kind} download boundary ({limit}); use existing outputs or start a new task"
        ),
    }
}

fn standalone_download_retired_error() -> BrowserError {
    BrowserError::Blocked {
        reason: "standalone browser download authority no longer matches a live task/Lane scope"
            .into(),
    }
}

#[async_trait]
trait ManagedLaneCleanup: Send + Sync {
    async fn shutdown_owned_targets(&self) -> Result<(), BrowserError>;

    /// Transfer exact-target cleanup to a bounded background authority after
    /// the synchronous close attempt fails. Implementations must not silently
    /// drop this handoff.
    fn hand_off_failed_cleanup(&self);
}

#[async_trait]
impl ManagedLaneCleanup for CdpBackend {
    async fn shutdown_owned_targets(&self) -> Result<(), BrowserError> {
        self.shutdown_lane().await
    }

    fn hand_off_failed_cleanup(&self) {
        self.hand_off_lane_cleanup();
    }
}

struct ManagedLaneEntry<L> {
    lane: Arc<L>,
    close_gate: Mutex<()>,
    /// Sticky "cleanup has started" flag (F16). Once a close begins, the lane
    /// backend has already fenced itself (every operation fails TargetClosed)
    /// and there is no un-close path, so a dying entry must never be handed
    /// out as a live lane again — even while it is still in the map mid-close.
    closing: AtomicBool,
    closed: AtomicBool,
}

impl<L> ManagedLaneEntry<L> {
    fn new(lane: Arc<L>) -> Self {
        Self {
            lane,
            close_gate: Mutex::new(()),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn is_dying(&self) -> bool {
        self.closing.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire)
    }
}

/// Coordinates lane insertion and cleanup without ever holding a host-global
/// lock across lane CDP I/O. Each entry has its own close gate, so cleanup is
/// single-flight per lane while unrelated lanes remain independent.
struct HostLaneCoordinator<L> {
    lanes: Mutex<HashMap<LaneId, Arc<ManagedLaneEntry<L>>>>,
    open_gate: Mutex<()>,
}

impl<L> Default for HostLaneCoordinator<L> {
    fn default() -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
            open_gate: Mutex::new(()),
        }
    }
}

impl<L> HostLaneCoordinator<L> {
    /// Retrieve a live lane. An entry whose close is in flight (or finished)
    /// is treated as absent (F16): returning it would hand the caller an
    /// engine whose targets are being destroyed and which is unrecoverable.
    async fn get(&self, lane_id: &str) -> Option<Arc<L>> {
        self.lanes
            .lock()
            .await
            .get(lane_id)
            .filter(|entry| !entry.is_dying())
            .map(|entry| Arc::clone(&entry.lane))
    }

    /// Publish a newly opened lane only while the host is still accepting
    /// work. The shutdown check and insertion share the map critical section,
    /// preventing a lane from appearing after successful host cleanup.
    async fn insert_if_open(
        &self,
        lane_id: LaneId,
        lane: Arc<L>,
        shutdown: &AtomicBool,
    ) -> Result<Arc<L>, BrowserError> {
        let mut lanes = self.lanes.lock().await;
        if shutdown.load(Ordering::Acquire) {
            return Err(BrowserError::SessionLost { recoverable: false });
        }
        if let Some(existing) = lanes.get(&lane_id) {
            // A dying entry must never be returned as the opened lane (F16).
            // Replace it: the in-flight close holds its own Arc to the old
            // entry and its final `remove_if_current` is ptr-guarded, so it
            // cannot remove this replacement.
            if !existing.is_dying() {
                return Ok(Arc::clone(&existing.lane));
            }
        }
        lanes.insert(
            lane_id,
            Arc::new(ManagedLaneEntry::new(Arc::clone(&lane))),
        );
        Ok(lane)
    }

    async fn snapshot(&self) -> Vec<(LaneId, Arc<ManagedLaneEntry<L>>)> {
        self.lanes
            .lock()
            .await
            .iter()
            .map(|(lane_id, entry)| (lane_id.clone(), Arc::clone(entry)))
            .collect()
    }

    /// Resolve an exact router snapshot to live Lane values under one map
    /// critical section. A closing/replaced Lane makes the policy transaction
    /// retry or fail instead of trimming an obsolete backend.
    async fn live_lanes(&self, lane_ids: &[LaneId]) -> Result<Vec<Arc<L>>, BrowserError> {
        let lanes = self.lanes.lock().await;
        lane_ids
            .iter()
            .map(|lane_id| {
                lanes
                    .get(lane_id)
                    .filter(|entry| !entry.is_dying())
                    .map(|entry| Arc::clone(&entry.lane))
                    .ok_or(BrowserError::TargetClosed)
            })
            .collect()
    }

    async fn remove_if_current(&self, lane_id: &str, entry: &Arc<ManagedLaneEntry<L>>) {
        let mut lanes = self.lanes.lock().await;
        let should_remove = lanes
            .get(lane_id)
            .is_some_and(|current| Arc::ptr_eq(current, entry));
        if should_remove {
            lanes.remove(lane_id);
        }
    }

    async fn clear(&self) {
        self.lanes.lock().await.clear();
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.lanes.lock().await.len()
    }
}

impl<L> HostLaneCoordinator<L>
where
    L: ManagedLaneCleanup + 'static,
{
    async fn close_entry(
        &self,
        lane_id: &str,
        entry: Arc<ManagedLaneEntry<L>>,
    ) -> Result<(), BrowserError> {
        let _close = entry.close_gate.lock().await;
        if entry.closed.load(Ordering::Acquire) {
            self.remove_if_current(lane_id, &entry).await;
            return Ok(());
        }

        // Sticky from the moment cleanup starts (F16): the backend fences
        // itself before any CDP I/O, so this entry can never be a valid
        // open_lane result again, even if this close attempt fails.
        entry.closing.store(true, Ordering::Release);
        let result = entry.lane.shutdown_owned_targets().await;
        if let Err(error) = result {
            // `shutdown_owned_targets` has already fenced the Lane and retains
            // exact target/session state. Move that state to the Host's
            // bounded cleanup executor before releasing the coordinator's
            // strong reference. Executor saturation fails closed by retiring
            // the whole Host, so no target cleanup authority is lost here.
            entry.lane.hand_off_failed_cleanup();
            entry.closed.store(true, Ordering::Release);
            self.remove_if_current(lane_id, &entry).await;
            return Err(error);
        }
        entry.closed.store(true, Ordering::Release);
        self.remove_if_current(lane_id, &entry).await;
        Ok(())
    }

    /// Idempotently close one lane. The only lock held across engine I/O is
    /// that lane's own close gate; sibling open/close operations stay live.
    async fn close_lane(&self, lane_id: &str) -> Result<(), BrowserError> {
        let entry = self.lanes.lock().await.get(lane_id).cloned();
        match entry {
            Some(entry) => self.close_entry(lane_id, entry).await,
            None => Ok(()),
        }
    }

    async fn close_all_with_grace(&self, grace: Duration) -> LaneCloseGraceOutcome {
        let entries = self.snapshot().await;
        if entries.is_empty() {
            return LaneCloseGraceOutcome::default();
        }

        let closes = entries.into_iter().map(|(lane_id, entry)| async move {
            self.close_entry(&lane_id, entry).await
        });
        match tokio::time::timeout(grace, join_all(closes)).await {
            Ok(results) => LaneCloseGraceOutcome {
                failed: results.iter().filter(|result| result.is_err()).count(),
                timed_out: false,
            },
            Err(_) => LaneCloseGraceOutcome {
                failed: 0,
                timed_out: true,
            },
        }
    }

    /// Give every lane a concurrent bounded graceful-close opportunity, then
    /// unconditionally advance to process-tree cleanup. Successful runtime
    /// shutdown is the final authority and permits dropping all stale entries.
    async fn shutdown_then_runtime<F>(
        &self,
        grace: Duration,
        runtime_shutdown: F,
    ) -> Result<(), BrowserError>
    where
        F: Future<Output = Result<(), BrowserError>>,
    {
        let outcome = self.close_all_with_grace(grace).await;
        if outcome.timed_out {
            tracing::warn!(
                grace_ms = grace.as_millis(),
                "lane cleanup grace expired; escalating to browser host shutdown"
            );
        } else if outcome.failed > 0 {
            tracing::warn!(
                failed_lane_count = outcome.failed,
                "lane cleanup failed; escalating to browser host shutdown"
            );
        }

        runtime_shutdown.await?;
        self.clear().await;
        Ok(())
    }
}

#[derive(Default)]
struct LaneCloseGraceOutcome {
    failed: usize,
    timed_out: bool,
}

/// Settings which are deliberately scoped to one lane rather than the shared
/// browser host.
#[derive(Clone)]
pub struct LaneEngineConfig {
    /// Upload sandbox root and the preferred destination for lane-created
    /// artifacts.  Browser-initiated downloads are staged and attributed by
    /// the host before being routed here.
    pub workspace_dir: Option<PathBuf>,
    /// Whether the lane may use the full-power evaluate action.
    pub evaluate_full_power: bool,
    /// Whether the lane is operating against a persistent live identity.
    pub evaluate_persistent_login: bool,
    /// Optional lane-local known-secret registry.  If omitted, the host
    /// configuration's registry is cloned.
    pub known_secret_values: Option<KnownSecretValues>,
    /// Trusted resource scope shared by sibling Lanes from one task.
    pub task_resource_key: Option<String>,
    pub max_task_tabs: usize,
    /// Optional platform-owned cross-Host tab authority. Standalone engine
    /// callers omit it and retain the Host-local cap only.
    pub task_tab_reservation_authority: Option<Arc<dyn TaskTabReservationAuthority>>,
    /// Optional platform-owned task-lifetime download authority. Managed
    /// platform Lanes must provide it; standalone Lanes receive the opaque
    /// scope-backed implementation during admission.
    pub task_download_reservation_authority:
        Option<Arc<dyn TaskDownloadReservationAuthority>>,
}

impl Default for LaneEngineConfig {
    fn default() -> Self {
        Self {
            workspace_dir: None,
            evaluate_full_power: false,
            evaluate_persistent_login: false,
            known_secret_values: None,
            // Standalone engine users have no cross-Lane task identity. The
            // Lane id becomes their isolated scope during registration.
            task_resource_key: None,
            max_task_tabs: 16,
            task_tab_reservation_authority: None,
            task_download_reservation_authority: None,
        }
    }
}

/// Result of applying an attached top-level target to the ownership table.
///
/// `Quarantined` is intentional: an unknown target must never be adopted by an
/// arbitrary lane.  It can later be claimed by the operation which created it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetRoute {
    Owned(LaneId),
    Inherited {
        lane_id: LaneId,
        opener_target_id: String,
    },
    Quarantined,
}

/// Pure target ownership state used by the asynchronous host router.
#[derive(Default, Debug)]
pub struct TargetOwnership {
    target_owner: HashMap<String, LaneId>,
}

impl TargetOwnership {
    /// Claim a target. Repeating the same claim is idempotent; attempting to
    /// transfer an already-owned target to another lane fails closed and
    /// returns the established owner.
    pub fn claim(
        &mut self,
        lane_id: impl Into<LaneId>,
        target_id: impl Into<String>,
    ) -> Result<(), LaneId> {
        let lane_id = lane_id.into();
        let target_id = target_id.into();
        if let Some(owner) = self.target_owner.get(&target_id) {
            return if owner == &lane_id {
                Ok(())
            } else {
                Err(owner.clone())
            };
        }
        self.target_owner.insert(target_id, lane_id);
        Ok(())
    }

    pub fn release(&mut self, target_id: &str) -> Option<LaneId> {
        self.target_owner.remove(target_id)
    }

    pub fn release_lane(&mut self, lane_id: &str) -> Vec<String> {
        let mut released = Vec::new();
        self.target_owner.retain(|target_id, owner| {
            if owner == lane_id {
                released.push(target_id.clone());
                false
            } else {
                true
            }
        });
        released
    }

    pub fn owner(&self, target_id: &str) -> Option<&str> {
        self.target_owner.get(target_id).map(String::as_str)
    }

    /// Snapshot every top-level target still owned by one Lane.
    ///
    /// Ownership intentionally outlives the live tab record until Lane
    /// teardown, so this is the authoritative cleanup inventory even when a
    /// detach/destroy event has already removed the target from the UI-facing
    /// tab registry.
    pub fn targets_for_lane(&self, lane_id: &str) -> Vec<String> {
        self.target_owner
            .iter()
            .filter_map(|(target_id, owner)| {
                (owner == lane_id).then_some(target_id.clone())
            })
            .collect()
    }

    pub fn route_attached(
        &mut self,
        target_id: &str,
        opener_target_id: Option<&str>,
    ) -> TargetRoute {
        if let Some(owner) = self.target_owner.get(target_id) {
            return TargetRoute::Owned(owner.clone());
        }
        if let Some(opener_target_id) = opener_target_id {
            if let Some(owner) = self.target_owner.get(opener_target_id).cloned() {
                self.target_owner
                    .insert(target_id.to_string(), owner.clone());
                return TargetRoute::Inherited {
                    lane_id: owner,
                    opener_target_id: opener_target_id.to_string(),
                };
            }
        }
        TargetRoute::Quarantined
    }
}

const MAX_TASK_TAB_RECONCILE_FLIGHTS_PER_HOST: usize = 64;

struct TaskTabReconcileFlightState {
    desired_generation: u64,
    desired_limit: usize,
    completed_generation: u64,
    outcome: Option<Result<(), BrowserError>>,
}

struct TaskTabReconcileFlight {
    state: std::sync::Mutex<TaskTabReconcileFlightState>,
    changed: tokio::sync::Notify,
}

impl TaskTabReconcileFlight {
    fn new(desired_limit: usize) -> Self {
        Self {
            state: std::sync::Mutex::new(TaskTabReconcileFlightState {
                desired_generation: 1,
                desired_limit,
                completed_generation: 0,
                outcome: None,
            }),
            changed: tokio::sync::Notify::new(),
        }
    }

    fn update(&self, desired_limit: usize) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.desired_limit != desired_limit {
            state.desired_generation = state.desired_generation.wrapping_add(1).max(1);
            state.desired_limit = desired_limit;
        }
        state.desired_generation
    }

    fn desired(&self) -> (u64, usize) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.desired_generation, state.desired_limit)
    }

    async fn wait_for(&self, generation: u64) -> Result<(), BrowserError> {
        loop {
            let changed = self.changed.notified();
            let outcome = {
                let state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (state.completed_generation >= generation)
                    .then(|| state.outcome.clone())
                    .flatten()
            };
            if let Some(outcome) = outcome {
                return outcome;
            }
            changed.await;
        }
    }
}

struct TaskTabReconcileRequest {
    flight: Arc<TaskTabReconcileFlight>,
    generation: u64,
    starts_worker: bool,
}

#[derive(Default)]
struct TaskTabReconcileCoordinator {
    flights: Mutex<HashMap<String, Arc<TaskTabReconcileFlight>>>,
}

impl TaskTabReconcileCoordinator {
    async fn request(
        &self,
        task_resource_key: &str,
        desired_limit: usize,
    ) -> Result<TaskTabReconcileRequest, BrowserError> {
        let mut flights = self.flights.lock().await;
        if let Some(flight) = flights.get(task_resource_key) {
            let generation = flight.update(desired_limit);
            return Ok(TaskTabReconcileRequest {
                flight: Arc::clone(flight),
                generation,
                starts_worker: false,
            });
        }
        if flights.len() >= MAX_TASK_TAB_RECONCILE_FLIGHTS_PER_HOST {
            return Err(BrowserError::Blocked {
                reason: format!(
                    "browser Host already has {} distinct task-tab reconciliations in flight",
                    MAX_TASK_TAB_RECONCILE_FLIGHTS_PER_HOST
                ),
            });
        }
        let flight = Arc::new(TaskTabReconcileFlight::new(desired_limit));
        flights.insert(task_resource_key.to_owned(), Arc::clone(&flight));
        Ok(TaskTabReconcileRequest {
            flight,
            generation: 1,
            starts_worker: true,
        })
    }

    /// Publish one attempt only if it still represents the latest request for
    /// this task. `false` tells the sole worker to rerun with the coalesced
    /// latest value; no caller ever creates a second worker.
    async fn complete_attempt(
        &self,
        task_resource_key: &str,
        flight: &Arc<TaskTabReconcileFlight>,
        attempted_generation: u64,
        outcome: Result<(), BrowserError>,
    ) -> bool {
        let mut flights = self.flights.lock().await;
        if !flights
            .get(task_resource_key)
            .is_some_and(|current| Arc::ptr_eq(current, flight))
        {
            return true;
        }
        let mut state = flight
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.desired_generation != attempted_generation {
            return false;
        }
        state.completed_generation = attempted_generation;
        state.outcome = Some(outcome);
        flights.remove(task_resource_key);
        drop(state);
        drop(flights);
        flight.changed.notify_waiters();
        true
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.flights.lock().await.len()
    }
}

/// One managed Chromium/connection serving multiple independently serialized
/// lanes.
pub struct ManagedBrowserHost {
    runtime: Arc<CdpHostRuntime>,
    lanes: HostLaneCoordinator<CdpBackend>,
    resource_mode: HostResourceMode,
    /// Serializes rare live task-policy changes. Ordinary browser operations
    /// remain Lane-local and never take this gate.
    task_tab_policy_gate: Mutex<()>,
    task_tab_reconciliations: TaskTabReconcileCoordinator,
    shutdown_gate: Mutex<()>,
    epoch: u64,
    shutdown: AtomicBool,
    default_lane_config: LaneEngineConfig,
}

#[derive(Clone)]
enum HostResourceMode {
    Standalone(StandaloneResourceScope),
    PlatformManaged,
}

impl ManagedBrowserHost {
    /// Launch exactly one managed Chromium browser instance/process tree and
    /// establish its single CDP connection. No page/lane is created until
    /// [`Self::open_lane`].
    pub async fn launch(config: EngineConfig) -> Result<Self, BrowserError> {
        let mode = BrowserHostLaunchMode::from_headful(config.headful);
        Self::launch_in_mode(config, mode).await
    }

    /// Launch one Host in an explicit process presentation mode.
    ///
    /// [`BrowserHostLaunchMode::Headless`] is the safe default for ordinary
    /// Agent work. A trusted foreground coordinator may replace a stopped
    /// Headless Host by calling this with `Headful` and the same
    /// application-owned stable profile. It **must** first await
    /// [`Self::shutdown`] on the old Host: Chromium forbids two live processes
    /// owning the same profile. The returned Host has a new epoch, so all old
    /// target/frame/ref handles are stale and callers must rebuild lanes and
    /// perform a fresh observe.
    ///
    /// This primitive intentionally does not pretend to migrate live targets;
    /// lane URL reconstruction and logical epoch publication belong to the
    /// authoritative platform layer.
    pub async fn launch_in_mode(
        config: EngineConfig,
        mode: BrowserHostLaunchMode,
    ) -> Result<Self, BrowserError> {
        let scope = config.standalone_resource_scope.clone();
        let cleanup_lease = HostCleanupLease::new(scope.reserve_host()?);
        Self::launch_with_resource_mode(
            config,
            mode,
            HostResourceMode::Standalone(scope),
            cleanup_lease,
        )
        .await
    }

    /// Trusted platform-Hub constructor. Unlike the standalone public path it
    /// does not install a process-global fixed cap; every Lane must provide the
    /// Hub's task key and external cross-Host tab authority.
    pub async fn launch_platform_managed(
        config: EngineConfig,
    ) -> Result<Self, BrowserError> {
        Self::launch_platform_managed_with_cleanup_lease(
            config,
            HostCleanupLease::new(()),
        )
        .await
    }

    /// Trusted platform-Hub constructor with provisional physical cleanup
    /// authority installed before any launch await.  Cancellation, factory
    /// errors, deferred relay dispatch, and runtime Drop all retain this lease
    /// until the same exact process/profile cleanup ticket completes.
    pub async fn launch_platform_managed_with_cleanup_lease(
        config: EngineConfig,
        cleanup_lease: HostCleanupLease,
    ) -> Result<Self, BrowserError> {
        let mode = BrowserHostLaunchMode::from_headful(config.headful);
        Self::launch_with_resource_mode(
            config,
            mode,
            HostResourceMode::PlatformManaged,
            cleanup_lease,
        )
        .await
    }

    async fn launch_with_resource_mode(
        mut config: EngineConfig,
        mode: BrowserHostLaunchMode,
        resource_mode: HostResourceMode,
        cleanup_lease: HostCleanupLease,
    ) -> Result<Self, BrowserError> {
        static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
        config.headful = mode.is_headful();
        let default_lane_config = LaneEngineConfig {
            workspace_dir: config.workspace_dir.clone(),
            evaluate_full_power: config.evaluate_full_power,
            evaluate_persistent_login: config.evaluate_persistent_login,
            known_secret_values: Some(config.known_secret_values.clone()),
            task_resource_key: None,
            max_task_tabs: 16,
            task_tab_reservation_authority: None,
            task_download_reservation_authority: None,
        };
        let runtime = CdpHostRuntime::launch_in_mode(config, mode, cleanup_lease).await?;
        Ok(Self {
            runtime,
            lanes: HostLaneCoordinator::default(),
            resource_mode,
            task_tab_policy_gate: Mutex::new(()),
            task_tab_reconciliations: TaskTabReconcileCoordinator::default(),
            shutdown_gate: Mutex::new(()),
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            shutdown: AtomicBool::new(false),
            default_lane_config,
        })
    }

    /// Monotonic process epoch.  A newly launched host always has a different
    /// epoch, so callers can invalidate old target/frame/ref handles.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Root Chromium process id for in-process resource telemetry.
    ///
    /// This deliberately exposes no CDP endpoint, profile path, or child
    /// process detail and returns `None` after the host has stopped.
    pub fn process_id(&self) -> Option<u32> {
        self.runtime.process_id()
    }

    /// Exact root-process telemetry identity captured at launch.
    pub fn process_identity(&self) -> Option<(u32, u64, u64)> {
        self.runtime.process_identity()
    }

    /// The effective process presentation mode after display capability
    /// probing. This is telemetry only; changing it requires a new Host.
    pub fn launch_mode(&self) -> BrowserHostLaunchMode {
        if self.runtime.is_headful() {
            BrowserHostLaunchMode::Headful
        } else {
            BrowserHostLaunchMode::Headless
        }
    }

    /// Open (or retrieve) one lane.  The returned engine has lane-local tabs,
    /// active target/frame, ref generations, operation mutex and cancellation.
    pub async fn open_lane(
        &self,
        lane_id: impl Into<LaneId>,
        mut config: LaneEngineConfig,
    ) -> Result<Arc<dyn BrowserEngine>, BrowserError> {
        // Lane construction is single-flight to avoid duplicate target
        // creation for the same id. Lane close never takes this gate.
        let _open = self.lanes.open_gate.lock().await;
        self.ensure_open()?;
        let lane_id = lane_id.into();
        if let Some(existing) = self.lanes.get(&lane_id).await {
            return Ok(existing);
        }

        let standalone_lane_authority = match &self.resource_mode {
            HostResourceMode::Standalone(scope) => {
                // Standalone task identity comes only from the trusted Host
                // constructor. Caller-supplied task keys and Lane ids cannot
                // mint an independent quota.
                let authority = scope.reserve_lane(lane_id.clone())?;
                config.task_resource_key = Some(scope.task_resource_key().to_owned());
                config.max_task_tabs = config
                    .max_task_tabs
                    .min(STANDALONE_MAX_LIVE_TABS_PER_SCOPE)
                    .max(1);
                config.task_tab_reservation_authority = Some(authority.clone());
                config.task_download_reservation_authority = Some(authority.clone());
                Some(authority)
            }
            HostResourceMode::PlatformManaged => {
                if config
                    .task_resource_key
                    .as_deref()
                    .is_none_or(|key| key.trim().is_empty())
                    || config.task_tab_reservation_authority.is_none()
                    || config.task_download_reservation_authority.is_none()
                {
                    return Err(BrowserError::Blocked {
                        reason: "platform-managed browser Lanes require a trusted task key plus external tab and download authorities"
                            .into(),
                    });
                }
                if config.max_task_tabs == 0 {
                    return Err(BrowserError::Blocked {
                        reason: "platform-managed browser task tab limit must be at least one"
                            .into(),
                    });
                }
                None
            }
        };

        let backend = Arc::new(
            CdpBackend::from_host(self.runtime.clone(), lane_id.clone(), config).await?,
        );
        let backend = self
            .lanes
            .insert_if_open(lane_id, backend, &self.shutdown)
            .await?;
        // `CdpBackend` now owns the same authority via its reservation scope.
        // Dropping this local Arc preserves the Lane lease until exact close or
        // final backend Drop.
        drop(standalone_lane_authority);
        Ok(backend)
    }

    /// Open a lane using the lane-scoped values inherited from [`EngineConfig`].
    pub async fn open_default_lane(
        &self,
        lane_id: impl Into<LaneId>,
    ) -> Result<Arc<dyn BrowserEngine>, BrowserError> {
        self.open_lane(lane_id, self.default_lane_config.clone()).await
    }

    /// Idempotently close only the named lane and its owned targets.
    pub async fn close_lane(&self, lane_id: &str) -> Result<(), BrowserError> {
        self.lanes.close_lane(lane_id).await
    }

    /// Atomically install a live task tab cap inside this shared Host and
    /// deterministically close every excess top-level page.
    ///
    /// The actual reconciliation runs in an owned task. Dropping the caller's
    /// future therefore cannot abandon a half-applied policy after the stricter
    /// admission cap has been committed. The Hub remains the cross-Host quota
    /// authority; this is its defense-in-depth Host-local enforcement seam.
    pub async fn reconcile_task_tab_limit(
        self: &Arc<Self>,
        task_resource_key: &str,
        max_task_tabs: usize,
    ) -> Result<(), BrowserError> {
        let request = self
            .task_tab_reconciliations
            .request(task_resource_key, max_task_tabs)
            .await?;
        if request.starts_worker {
            let host = Arc::clone(self);
            let task_resource_key = task_resource_key.to_owned();
            let flight = Arc::clone(&request.flight);
            tokio::spawn(async move {
                host.run_task_tab_reconcile_flight(task_resource_key, flight)
                    .await;
            });
        }
        request.flight.wait_for(request.generation).await
    }

    async fn run_task_tab_reconcile_flight(
        self: Arc<Self>,
        task_resource_key: String,
        flight: Arc<TaskTabReconcileFlight>,
    ) {
        loop {
            let (generation, max_task_tabs) = flight.desired();
            let host = Arc::clone(&self);
            let task_for_attempt = task_resource_key.clone();
            let outcome = match tokio::spawn(async move {
                host.reconcile_task_tab_limit_inner(&task_for_attempt, max_task_tabs)
                    .await
            })
            .await
            {
                Ok(outcome) => outcome,
                Err(error) => Err(BrowserError::Other(format!(
                    "browser task tab reconciliation worker failed: {error}"
                ))),
            };
            if self
                .task_tab_reconciliations
                .complete_attempt(&task_resource_key, &flight, generation, outcome)
                .await
            {
                return;
            }
        }
    }

    async fn reconcile_task_tab_limit_inner(
        &self,
        task_resource_key: &str,
        max_task_tabs: usize,
    ) -> Result<(), BrowserError> {
        if max_task_tabs == 0 {
            return Err(BrowserError::Blocked {
                reason: "a browser task tab limit must retain at least one page".into(),
            });
        }
        let _policy = self.task_tab_policy_gate.lock().await;
        self.ensure_open()?;

        // Do not hold the Host open gate while waiting for in-flight Lane
        // operations. Once every selected Lane is fenced, take the open gate,
        // verify the router snapshot is unchanged, and commit the lower cap.
        let (lanes, plan) = loop {
            let lane_ids = self.runtime.task_lane_ids(task_resource_key).await;
            if lane_ids.is_empty() {
                return Ok(());
            }
            let lanes = self.lanes.live_lanes(&lane_ids).await?;
            let mut operation_guards = Vec::with_capacity(lanes.len());
            for lane in &lanes {
                operation_guards.push(lane.lock_operations_for_task_policy().await);
            }

            let open_guard = self.lanes.open_gate.lock().await;
            self.ensure_open()?;
            if self.runtime.task_lane_ids(task_resource_key).await != lane_ids {
                drop(open_guard);
                drop(operation_guards);
                continue;
            }
            let plan = self
                .runtime
                .prepare_task_tab_limit_reconciliation(task_resource_key, max_task_tabs)
                .await?;
            drop(open_guard);
            break ((lane_ids, lanes, operation_guards), plan);
        };

        // Keep every affected Lane operation gate through exact close and
        // verification. Popup/attach publication is independently serialized
        // by the router and sees the stricter cap immediately.
        let (lane_ids, lanes, _operation_guards) = lanes;
        let lane_by_id = lane_ids
            .into_iter()
            .zip(lanes)
            .collect::<HashMap<_, _>>();
        let mut first_error = None;
        for (lane_id, targets) in plan.excess_tabs {
            let Some(lane) = lane_by_id.get(&lane_id) else {
                first_error.get_or_insert(BrowserError::TargetClosed);
                continue;
            };
            for target_id in targets {
                if let Err(error) = lane.close_tab_for_task_policy(&target_id).await {
                    first_error.get_or_insert(error);
                }
            }
        }

        let remaining = self.runtime.task_tab_count(task_resource_key).await;
        if remaining > max_task_tabs {
            first_error.get_or_insert_with(|| {
                BrowserError::Other(format!(
                    "browser task remains cleanup-pending with {remaining} top-level tabs after lowering its Host cap to {max_task_tabs}"
                ))
            });
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Explicit, idempotent host shutdown.  Lanes are cancelled/closed before
    /// the process tree and profile are released.
    pub async fn shutdown(&self) -> Result<(), BrowserError> {
        // Fence opens before waiting for another shutdown caller. An open that
        // was already constructing a target re-checks this flag while holding
        // the lane map lock and cannot publish after this point.
        self.shutdown.store(true, Ordering::Release);
        let _shutdown = self.shutdown_gate.lock().await;
        if self.runtime.is_stopped() {
            self.lanes.clear().await;
            return Ok(());
        }
        self.lanes
            .shutdown_then_runtime(HOST_LANE_CLOSE_GRACE, self.runtime.shutdown())
            .await
    }

    fn ensure_open(&self) -> Result<(), BrowserError> {
        if self.shutdown.load(Ordering::Acquire) {
            Err(BrowserError::SessionLost { recoverable: false })
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[test]
    fn standalone_host_n_plus_one_is_rejected_and_drop_returns_capacity() {
        let scope = StandaloneResourceScope::new();
        let mut leases = Vec::new();
        for _ in 0..STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE {
            leases.push(scope.reserve_host().expect("Host slot within cap"));
        }
        assert!(scope.reserve_host().is_err(), "Host N+1 must fail closed");
        assert_eq!(scope.counts().0, STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE);

        drop(leases.pop());
        let replacement = scope
            .reserve_host()
            .expect("dropping the exact Host lease returns its slot");
        assert_eq!(scope.counts().0, STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE);
        drop(replacement);
        drop(leases);
        assert_eq!(scope.counts(), (0, 0, 0));
    }

    #[tokio::test]
    async fn cancelled_or_failed_lane_admission_returns_its_raii_slot() {
        let scope = StandaloneResourceScope::new();
        let cancelled_scope = scope.clone();
        let task = tokio::spawn(async move {
            let _lease = cancelled_scope
                .reserve_lane("cancelled".into())
                .expect("Lane reserve");
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        assert_eq!(scope.counts().1, 1);
        task.abort();
        let _ = task.await;
        assert_eq!(scope.counts().1, 0, "cancelled future must return Lane slot");

        // Models an error after admission but before CdpBackend publication.
        let failed = (|| -> Result<(), BrowserError> {
            let _lease = scope.reserve_lane("failed".into())?;
            Err(BrowserError::Other("synthetic open failure".into()))
        })();
        assert!(failed.is_err());
        assert_eq!(scope.counts().1, 0, "failed open must return Lane slot");
    }

    #[tokio::test]
    async fn standalone_scope_caps_tabs_across_different_lane_ids() {
        let scope = StandaloneResourceScope::new();
        let lanes = (0..STANDALONE_MAX_LIVE_LANES_PER_SCOPE)
            .map(|index| {
                scope
                    .reserve_lane(format!("lane-{index}"))
                    .expect("Lane within task cap")
            })
            .collect::<Vec<_>>();
        assert!(
            scope.reserve_lane("different-id-cannot-bypass".into()).is_err(),
            "different Lane ids must share one task Lane budget"
        );

        let mut tabs = Vec::<Arc<dyn TaskTabReservation>>::new();
        for index in 0..STANDALONE_MAX_LIVE_TABS_PER_SCOPE {
            let lane = &lanes[index % lanes.len()];
            tabs.push(
                TaskTabReservationAuthority::reserve(
                    lane.as_ref(),
                    scope.task_resource_key(),
                    &lane.lane_id,
                    &format!("target-{index}"),
                )
                .await
                .expect("tab within shared task cap"),
            );
        }
        assert_eq!(scope.counts().2, STANDALONE_MAX_LIVE_TABS_PER_SCOPE);
        assert!(
            TaskTabReservationAuthority::reserve(
                lanes[0].as_ref(),
                scope.task_resource_key(),
                &lanes[0].lane_id,
                "target-n-plus-one",
            )
                .await
                .is_err(),
            "a different Lane cannot bypass the task tab cap"
        );
        let duplicate = TaskTabReservationAuthority::reserve(
            lanes[0].as_ref(),
            scope.task_resource_key(),
            &lanes[0].lane_id,
            "target-0",
        )
            .await
            .expect("duplicate target reservation is idempotent");
        assert_eq!(scope.counts().2, STANDALONE_MAX_LIVE_TABS_PER_SCOPE);
        drop(duplicate);
        drop(tabs);
        assert_eq!(scope.counts().2, 0);
        drop(lanes);
        assert_eq!(scope.counts(), (0, 0, 0));
    }

    #[test]
    fn legacy_default_scopes_share_one_compatibility_budget() {
        let first = StandaloneResourceScope::default();
        let second = StandaloneResourceScope::default();
        let mut leases = Vec::new();
        for _ in 0..STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE {
            leases.push(first.reserve_host().expect("legacy Host within cap"));
        }
        assert!(
            second.reserve_host().is_err(),
            "legacy callers without an explicit scope must share compatibility capacity"
        );
        drop(leases);
        assert_eq!(first.counts(), (0, 0, 0));
    }

    #[test]
    fn trusted_task_scopes_are_independent_but_clones_share_one_budget() {
        let task_a = StandaloneResourceScope::new();
        let task_a_other_facade = task_a.clone();
        let task_b = StandaloneResourceScope::new();
        let mut task_a_hosts = Vec::new();
        let mut task_b_hosts = Vec::new();

        for _ in 0..STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE {
            task_a_hosts.push(task_a.reserve_host().expect("task A Host within cap"));
            task_b_hosts.push(task_b.reserve_host().expect("task B Host within cap"));
        }
        assert!(
            task_a_other_facade.reserve_host().is_err(),
            "a second facade for task A must consume the same aggregate budget"
        );
        assert_eq!(
            task_b.counts().0,
            STANDALONE_MAX_LIVE_HOSTS_PER_SCOPE,
            "task B has an independent trusted scope"
        );

        drop(task_a_hosts);
        drop(task_b_hosts);
        assert_eq!(task_a.counts(), (0, 0, 0));
        assert_eq!(task_b.counts(), (0, 0, 0));
    }

    #[test]
    fn lane_cleanup_debt_retains_capacity_until_exact_finish() {
        let scope = StandaloneResourceScope::new();
        let mut live = (0..STANDALONE_MAX_LIVE_LANES_PER_SCOPE)
            .map(|index| {
                scope
                    .reserve_lane(format!("lane-{index}"))
                    .expect("Lane within cap")
            })
            .collect::<Vec<_>>();
        let cleanup_debt: Arc<dyn TaskTabReservationAuthority> =
            live.pop().expect("one Lane authority");

        assert!(
            scope.reserve_lane("n-plus-one".into()).is_err(),
            "a dropped facade whose exact cleanup still owns the authority must remain charged"
        );
        cleanup_debt.release_lane();
        let replacement = scope
            .reserve_lane("replacement-after-proof".into())
            .expect("exact cleanup completion returns the Lane slot");

        drop(replacement);
        drop(cleanup_debt);
        drop(live);
        assert_eq!(scope.counts(), (0, 0, 0));
    }

    async fn reserve_download(
        scope: &StandaloneResourceScope,
        lane: &Arc<StandaloneLaneAuthority>,
        download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        TaskDownloadReservationAuthority::reserve(
            lane.as_ref(),
            scope.task_resource_key(),
            &lane.lane_id,
            download_key,
        )
        .await
    }

    /// Snapshot of the scope-level standalone download ledger:
    /// `(completed bytes, completed files, live active-download slots)`.
    fn download_ledger(scope: &StandaloneResourceScope) -> (u64, usize, usize) {
        let state = scope
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            state.completed_download_bytes,
            state.completed_download_files,
            state.active_downloads.len(),
        )
    }

    fn blocked_reason(error: BrowserError) -> String {
        match error {
            BrowserError::Blocked { reason } => reason,
            other => panic!("standalone download admission must fail closed as Blocked: {other:?}"),
        }
    }

    #[tokio::test]
    async fn standalone_download_active_slots_cap_at_four_and_drop_returns_capacity() {
        let scope = StandaloneResourceScope::new();
        let lane = scope.reserve_lane("lane-dl".into()).expect("Lane within cap");

        let mut active = Vec::new();
        for index in 0..MAX_TASK_ACTIVE_DOWNLOADS {
            active.push(
                reserve_download(&scope, &lane, &format!("download-{index}"))
                    .await
                    .expect("active download within cap"),
            );
        }
        assert_eq!(download_ledger(&scope).2, MAX_TASK_ACTIVE_DOWNLOADS);

        let overflow = match reserve_download(&scope, &lane, "download-n-plus-one").await {
            Ok(_) => panic!("active download N+1 must fail closed"),
            Err(error) => error,
        };
        assert!(
            blocked_reason(overflow).contains("active download"),
            "N+1 must be rejected by the active-slot boundary, not another gate"
        );

        drop(active.pop());
        let replacement = reserve_download(&scope, &lane, "download-n-plus-one")
            .await
            .expect("dropping an unfinished reservation returns its exact active slot");
        assert_eq!(download_ledger(&scope).2, MAX_TASK_ACTIVE_DOWNLOADS);

        drop(replacement);
        drop(active);
        assert_eq!(
            download_ledger(&scope),
            (0, 0, 0),
            "abandoned (never-finalized) downloads must not leave sticky charges"
        );
    }

    #[tokio::test]
    async fn standalone_download_single_file_and_cumulative_byte_boundaries_fail_closed() {
        let scope = StandaloneResourceScope::new();
        let lane = scope.reserve_lane("lane-dl".into()).expect("Lane within cap");

        let first = reserve_download(&scope, &lane, "first")
            .await
            .expect("first download admitted");
        assert!(
            first
                .update_progress(MAX_TASK_SINGLE_DOWNLOAD_BYTES + 1, None)
                .is_err(),
            "received bytes past the single-file cap must fail closed"
        );
        assert!(
            first
                .update_progress(0, Some(MAX_TASK_SINGLE_DOWNLOAD_BYTES + 1))
                .is_err(),
            "a total-size hint past the single-file cap must fail closed before bytes arrive"
        );
        assert!(
            first
                .prepare_complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES + 1)
                .is_err(),
            "completion past the single-file cap must fail closed"
        );
        first
            .update_progress(MAX_TASK_SINGLE_DOWNLOAD_BYTES, None)
            .expect("exactly the single-file cap is admitted");
        first
            .complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES)
            .expect("first completion fits the cumulative envelope");
        assert_eq!(
            download_ledger(&scope),
            (MAX_TASK_SINGLE_DOWNLOAD_BYTES, 1, 0)
        );

        // In-flight bytes of *other* active downloads are charged against the
        // cumulative envelope before anything is finalized.
        let second = reserve_download(&scope, &lane, "second")
            .await
            .expect("second download admitted");
        second
            .update_progress(MAX_TASK_SINGLE_DOWNLOAD_BYTES, None)
            .expect("completed + in-flight may reach exactly the cumulative cap");
        let third = reserve_download(&scope, &lane, "third")
            .await
            .expect("admission gates on finalized bytes, so a third slot still opens");
        let cross = third
            .update_progress(1, None)
            .expect_err("one more in-flight byte would overrun the cumulative cap");
        assert!(blocked_reason(cross).contains("cumulative byte"));

        second
            .complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES)
            .expect("second completion reaches exactly the cumulative envelope");
        assert_eq!(
            download_ledger(&scope).0,
            MAX_TASK_COMPLETED_DOWNLOAD_BYTES,
            "two half-envelope completions saturate the task byte budget"
        );
        let saturated = match reserve_download(&scope, &lane, "fourth").await {
            Ok(_) => panic!("a saturated cumulative ledger must reject new reservations"),
            Err(error) => error,
        };
        assert!(blocked_reason(saturated).contains("cumulative byte"));
        drop(third);
    }

    #[tokio::test]
    async fn standalone_download_two_phase_prepare_finalize_and_idempotency() {
        let scope = StandaloneResourceScope::new();
        let lane = scope.reserve_lane("lane-dl".into()).expect("Lane within cap");
        let download = reserve_download(&scope, &lane, "report")
            .await
            .expect("download admitted");

        download
            .update_progress(1_000, Some(2_048))
            .expect("in-flight progress within caps");
        download
            .prepare_complete(2_048)
            .expect("phase one reserves the final charge");
        download
            .prepare_complete(2_048)
            .expect("re-preparing the same final size is idempotent");
        assert!(
            download.prepare_complete(1_024).is_err(),
            "a duplicate completion with a different size is an inconsistency and must fail"
        );
        assert!(
            download.update_progress(4_096, None).is_err(),
            "progress after a prepared completion must be rejected"
        );
        assert_eq!(
            download_ledger(&scope),
            (0, 0, 1),
            "phase one must not charge the completed ledger yet"
        );

        download.finalize_complete();
        assert_eq!(
            download_ledger(&scope),
            (2_048, 1, 0),
            "finalize charges bytes+file exactly once and frees the active slot"
        );
        download.finalize_complete();
        download
            .complete(999_999)
            .expect("complete() on an already-completed reservation is an idempotent no-op");
        assert_eq!(download_ledger(&scope), (2_048, 1, 0), "no double charge");

        drop(download);
        assert_eq!(
            download_ledger(&scope),
            (2_048, 1, 0),
            "dropping a completed reservation must not refund the completed quota"
        );
        let _next = reserve_download(&scope, &lane, "next")
            .await
            .expect("a new key is admitted while prior completed charges stay on the ledger");
        assert_eq!(download_ledger(&scope), (2_048, 1, 1));
    }

    #[tokio::test]
    async fn standalone_download_same_key_reservation_is_idempotent_and_key_length_bounded() {
        let scope = StandaloneResourceScope::new();
        let lane = scope.reserve_lane("lane-dl".into()).expect("Lane within cap");

        let first = reserve_download(&scope, &lane, "guid-1")
            .await
            .expect("download admitted");
        let duplicate = reserve_download(&scope, &lane, "guid-1")
            .await
            .expect("re-reserving the same (Lane, key) is idempotent");
        assert!(
            std::ptr::eq(
                Arc::as_ptr(&first) as *const (),
                Arc::as_ptr(&duplicate) as *const (),
            ),
            "the duplicate must reuse the same logical reservation, not a second slot"
        );
        assert_eq!(
            download_ledger(&scope).2,
            1,
            "one logical download holds exactly one active slot"
        );

        let bounded_key = "k".repeat(4 * 1024);
        let _bounded = reserve_download(&scope, &lane, &bounded_key)
            .await
            .expect("a key of exactly 4 KiB is admitted");
        assert!(
            reserve_download(&scope, &lane, &"k".repeat(4 * 1024 + 1))
                .await
                .is_err(),
            "a key longer than 4 KiB must be rejected before touching capacity state"
        );
        assert!(
            reserve_download(&scope, &lane, "").await.is_err(),
            "an empty key must be rejected"
        );
        assert_eq!(
            download_ledger(&scope).2,
            2,
            "rejected keys must not leak active slots"
        );
    }

    #[tokio::test]
    async fn standalone_download_completed_charge_survives_lane_drop() {
        // The completed-download ledger lives on the opaque task scope
        // (`StandaloneResourceState`), not on any Lane authority, so finalized
        // charges are sticky for the task lifetime: rotating or dropping Lanes
        // must never mint a fresh byte budget.
        let scope = StandaloneResourceScope::new();

        let first_lane = scope
            .reserve_lane("lane-first".into())
            .expect("Lane within cap");
        let download = reserve_download(&scope, &first_lane, "big-artifact")
            .await
            .expect("download admitted");
        download
            .complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES)
            .expect("first completion within the envelope");
        drop(download);
        drop(first_lane);
        assert_eq!(
            download_ledger(&scope),
            (MAX_TASK_SINGLE_DOWNLOAD_BYTES, 1, 0),
            "dropping the Lane authority must not refund task-level completed charges"
        );

        let second_lane = scope
            .reserve_lane("lane-second".into())
            .expect("replacement Lane within cap");
        let successor = reserve_download(&scope, &second_lane, "big-artifact-2")
            .await
            .expect("a fresh Lane still reserves against the surviving ledger");
        successor
            .complete(MAX_TASK_SINGLE_DOWNLOAD_BYTES)
            .expect("second completion saturates the task envelope");
        let saturated = match reserve_download(&scope, &second_lane, "big-artifact-3").await {
            Ok(_) => panic!("Lane rotation must not mint a fresh cumulative byte budget"),
            Err(error) => error,
        };
        assert!(blocked_reason(saturated).contains("cumulative byte"));
    }

    struct FakeLaneCleanup {
        close_calls: AtomicUsize,
        handoff_calls: AtomicUsize,
        release: Option<Notify>,
        fail: AtomicBool,
    }

    impl FakeLaneCleanup {
        fn immediate() -> Arc<Self> {
            Arc::new(Self {
                close_calls: AtomicUsize::new(0),
                handoff_calls: AtomicUsize::new(0),
                release: None,
                fail: AtomicBool::new(false),
            })
        }

        fn hanging() -> Arc<Self> {
            Arc::new(Self {
                close_calls: AtomicUsize::new(0),
                handoff_calls: AtomicUsize::new(0),
                release: Some(Notify::new()),
                fail: AtomicBool::new(false),
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                close_calls: AtomicUsize::new(0),
                handoff_calls: AtomicUsize::new(0),
                release: None,
                fail: AtomicBool::new(true),
            })
        }

        fn release(&self) {
            if let Some(release) = &self.release {
                release.notify_one();
            }
        }
    }

    #[async_trait]
    impl ManagedLaneCleanup for FakeLaneCleanup {
        async fn shutdown_owned_targets(&self) -> Result<(), BrowserError> {
            self.close_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(release) = &self.release {
                release.notified().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(BrowserError::Other("fake lane close failure".into()));
            }
            Ok(())
        }

        fn hand_off_failed_cleanup(&self) {
            self.handoff_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn wait_for_close_calls(lane: &FakeLaneCleanup, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while lane.close_calls.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("fake lane close should start");
    }

    #[tokio::test]
    async fn task_tab_reconcile_cancel_flood_keeps_one_latest_wins_flight() {
        let coordinator = Arc::new(TaskTabReconcileCoordinator::default());
        let first = coordinator
            .request("one-task", 4)
            .await
            .expect("first reconcile request is admitted");
        assert!(first.starts_worker);
        let mut worker_starts = usize::from(first.starts_worker);

        for index in 0..1_000usize {
            let request = coordinator
                .request("one-task", index % 8 + 1)
                .await
                .expect("same-task requests coalesce without extra admission");
            worker_starts += usize::from(request.starts_worker);
            assert!(Arc::ptr_eq(&first.flight, &request.flight));
            let waiter = tokio::spawn(async move {
                request.flight.wait_for(request.generation).await
            });
            waiter.abort();
            let _ = waiter.await;
        }

        assert_eq!(worker_starts, 1, "caller flood may start only one worker");
        assert_eq!(coordinator.len().await, 1);
        let (latest_generation, latest_limit) = first.flight.desired();
        assert_eq!(latest_limit, 8, "the last requested cap wins");
        assert!(
            !coordinator
                .complete_attempt(
                    "one-task",
                    &first.flight,
                    first.generation,
                    Err(BrowserError::Other("stale attempt".into())),
                )
                .await,
            "a stale attempt must rerun instead of publishing or removing the flight"
        );
        assert_eq!(coordinator.len().await, 1);
        assert!(
            coordinator
                .complete_attempt(
                    "one-task",
                    &first.flight,
                    latest_generation,
                    Ok(()),
                )
                .await
        );
        first
            .flight
            .wait_for(first.generation)
            .await
            .expect("an earlier surviving caller observes latest completion");
        assert_eq!(coordinator.len().await, 0, "completed flight is removed");
    }

    #[test]
    fn popup_inherits_opener_lane() {
        let mut ownership = TargetOwnership::default();
        ownership.claim("lane-a", "opener").unwrap();
        assert_eq!(
            ownership.route_attached("popup", Some("opener")),
            TargetRoute::Inherited {
                lane_id: "lane-a".into(),
                opener_target_id: "opener".into(),
            }
        );
        assert_eq!(ownership.owner("popup"), Some("lane-a"));
    }

    #[test]
    fn unknown_target_is_quarantined_and_never_adopted() {
        let mut ownership = TargetOwnership::default();
        ownership.claim("lane-a", "a").unwrap();
        ownership.claim("lane-b", "b").unwrap();
        assert_eq!(
            ownership.route_attached("unknown", None),
            TargetRoute::Quarantined
        );
        assert_eq!(ownership.owner("unknown"), None);
    }

    #[test]
    fn unknown_opener_is_quarantined() {
        let mut ownership = TargetOwnership::default();
        assert_eq!(
            ownership.route_attached("popup", Some("not-owned")),
            TargetRoute::Quarantined
        );
        assert_eq!(ownership.owner("popup"), None);
    }

    #[test]
    fn quarantined_popup_inherits_when_opener_is_claimed_later() {
        let mut ownership = TargetOwnership::default();
        assert_eq!(
            ownership.route_attached("popup", Some("opener")),
            TargetRoute::Quarantined
        );
        ownership.claim("lane-a", "opener").unwrap();
        assert_eq!(
            ownership.route_attached("popup", Some("opener")),
            TargetRoute::Inherited {
                lane_id: "lane-a".into(),
                opener_target_id: "opener".into(),
            }
        );
    }

    #[test]
    fn releasing_lane_does_not_release_other_lane_targets() {
        let mut ownership = TargetOwnership::default();
        ownership.claim("lane-a", "a1").unwrap();
        ownership.claim("lane-a", "a2").unwrap();
        ownership.claim("lane-b", "b1").unwrap();
        let mut released = ownership.release_lane("lane-a");
        released.sort();
        assert_eq!(released, vec!["a1", "a2"]);
        assert_eq!(ownership.owner("b1"), Some("lane-b"));
    }

    #[test]
    fn lane_target_snapshot_is_scoped_and_keeps_cleanup_tombstones() {
        let mut ownership = TargetOwnership::default();
        ownership.claim("lane-a", "a2").unwrap();
        ownership.claim("lane-b", "b1").unwrap();
        ownership.claim("lane-a", "a1").unwrap();

        let mut lane_a = ownership.targets_for_lane("lane-a");
        lane_a.sort();
        assert_eq!(lane_a, vec!["a1", "a2"]);
        assert_eq!(ownership.targets_for_lane("lane-b"), vec!["b1"]);
        assert!(ownership.targets_for_lane("unknown").is_empty());
    }

    #[test]
    fn claim_cannot_transfer_target_between_lanes() {
        let mut ownership = TargetOwnership::default();
        ownership.claim("lane-a", "target").unwrap();
        assert_eq!(
            ownership.claim("lane-b", "target"),
            Err("lane-a".to_string())
        );
        assert_eq!(ownership.owner("target"), Some("lane-a"));
    }

    #[tokio::test]
    async fn same_lane_is_serial_but_different_lanes_overlap() {
        async fn run(
            gate: Arc<LaneOperationGate>,
            entered: Arc<AtomicUsize>,
            release: Arc<Notify>,
        ) {
            let _guard = gate.lock().await;
            entered.fetch_add(1, Ordering::SeqCst);
            release.notified().await;
        }

        let lane_a = Arc::new(LaneOperationGate::default());
        let lane_b = Arc::new(LaneOperationGate::default());
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());

        let a1 = tokio::spawn(run(lane_a.clone(), entered.clone(), release.clone()));
        while entered.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
        let a2 = tokio::spawn(run(lane_a, entered.clone(), release.clone()));
        let b1 = tokio::spawn(run(lane_b, entered.clone(), release.clone()));

        while entered.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            entered.load(Ordering::SeqCst),
            2,
            "lane B must enter while lane A1 holds its gate, but lane A2 must wait"
        );

        release.notify_waiters();
        tokio::task::yield_now().await;
        release.notify_waiters();
        a1.await.unwrap();
        a2.await.unwrap();
        b1.await.unwrap();
        assert_eq!(entered.load(Ordering::SeqCst), 3);
    }

    /// **F16 回归**：close 在飞行中（close_gate 已持、closed 尚未置位、entry 仍在
    /// map）时，open/retrieve 路径**绝不能**把这条垂死 lane 交出去——那是一个每个
    /// 操作都恒 TargetClosed 的死引擎，且 close 完成后 map 槽位清空，调用方无从修复。
    #[tokio::test]
    async fn inflight_close_hides_the_dying_lane_from_open_paths() {
        let coordinator = Arc::new(HostLaneCoordinator::<FakeLaneCleanup>::default());
        let shutdown = AtomicBool::new(false);
        let dying = FakeLaneCleanup::hanging();
        coordinator
            .insert_if_open("lane-a".into(), Arc::clone(&dying), &shutdown)
            .await
            .unwrap();
        assert!(
            coordinator.get("lane-a").await.is_some(),
            "a live entry is retrievable before close starts"
        );

        let close = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.close_lane("lane-a").await })
        };
        wait_for_close_calls(&dying, 1).await;

        assert!(
            coordinator.get("lane-a").await.is_none(),
            "a mid-close lane must be treated as absent, not returned"
        );

        // A concurrent reopen publishes a replacement, never the dying lane.
        let replacement = FakeLaneCleanup::immediate();
        let opened = coordinator
            .insert_if_open("lane-a".into(), Arc::clone(&replacement), &shutdown)
            .await
            .unwrap();
        assert!(
            Arc::ptr_eq(&opened, &replacement),
            "reopen during an in-flight close must not hand back the dying lane"
        );

        // The old close finishing must not evict the replacement entry.
        dying.release();
        close.await.unwrap().unwrap();
        let survivor = coordinator
            .get("lane-a")
            .await
            .expect("the replacement lane survives the old generation's close");
        assert!(Arc::ptr_eq(&survivor, &replacement));
        assert_eq!(coordinator.len().await, 1);
    }

    /// **F16/F21**：close 失败后 backend 已自我 fence，精确 cleanup authority
    /// 必须移交给有界后台执行器，同时 coordinator 立即释放强引用。
    #[tokio::test]
    async fn failed_close_hands_off_cleanup_and_removes_the_entry() {
        let coordinator = Arc::new(HostLaneCoordinator::<FakeLaneCleanup>::default());
        let shutdown = AtomicBool::new(false);
        let lane = FakeLaneCleanup::failing();
        coordinator
            .insert_if_open("lane-a".into(), Arc::clone(&lane), &shutdown)
            .await
            .unwrap();

        assert!(coordinator.close_lane("lane-a").await.is_err());
        assert!(
            coordinator.get("lane-a").await.is_none(),
            "an entry whose close failed is never re-issued"
        );
        assert_eq!(coordinator.len().await, 0);
        assert_eq!(lane.handoff_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lane.close_calls.load(Ordering::SeqCst), 1);
        coordinator.close_lane("lane-a").await.unwrap();
        assert_eq!(lane.handoff_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_distinct_failed_closes_do_not_accumulate_coordinator_entries() {
        let coordinator = HostLaneCoordinator::<FakeLaneCleanup>::default();
        let shutdown = AtomicBool::new(false);

        for index in 0..256 {
            let lane = FakeLaneCleanup::failing();
            let lane_id = format!("failing-lane-{index}");
            coordinator
                .insert_if_open(lane_id.clone(), Arc::clone(&lane), &shutdown)
                .await
                .unwrap();
            assert!(coordinator.close_lane(&lane_id).await.is_err());
            assert_eq!(
                coordinator.len().await,
                0,
                "failed Lane generations must not form a linear tombstone table"
            );
            assert_eq!(lane.handoff_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn same_lane_close_is_single_flight_and_idempotent() {
        let coordinator = Arc::new(HostLaneCoordinator::<FakeLaneCleanup>::default());
        let shutdown = AtomicBool::new(false);
        let lane = FakeLaneCleanup::hanging();
        coordinator
            .insert_if_open("lane-a".into(), Arc::clone(&lane), &shutdown)
            .await
            .unwrap();

        let first = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.close_lane("lane-a").await })
        };
        wait_for_close_calls(&lane, 1).await;
        let second = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.close_lane("lane-a").await })
        };
        tokio::task::yield_now().await;
        assert_eq!(
            lane.close_calls.load(Ordering::SeqCst),
            1,
            "the second close must join the lane-local cleanup flight"
        );

        lane.release();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        coordinator.close_lane("lane-a").await.unwrap();
        assert_eq!(lane.close_calls.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.len().await, 0);
    }

    #[tokio::test]
    async fn hanging_lane_close_does_not_block_sibling_open_or_close() {
        let coordinator = Arc::new(HostLaneCoordinator::<FakeLaneCleanup>::default());
        let shutdown = AtomicBool::new(false);
        let hanging = FakeLaneCleanup::hanging();
        let sibling = FakeLaneCleanup::immediate();
        coordinator
            .insert_if_open("lane-hung".into(), Arc::clone(&hanging), &shutdown)
            .await
            .unwrap();
        coordinator
            .insert_if_open("lane-sibling".into(), Arc::clone(&sibling), &shutdown)
            .await
            .unwrap();

        let hung_close = {
            let coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move { coordinator.close_lane("lane-hung").await })
        };
        wait_for_close_calls(&hanging, 1).await;

        let opened = FakeLaneCleanup::immediate();
        tokio::time::timeout(
            Duration::from_millis(100),
            coordinator.insert_if_open(
                "lane-opened-while-close-hangs".into(),
                Arc::clone(&opened),
                &shutdown,
            ),
        )
        .await
        .expect("a hung lane close must not block sibling open")
        .unwrap();
        tokio::time::timeout(
            Duration::from_millis(100),
            coordinator.close_lane("lane-sibling"),
        )
        .await
        .expect("a hung lane close must not block sibling close")
        .unwrap();
        assert_eq!(sibling.close_calls.load(Ordering::SeqCst), 1);

        hanging.release();
        hung_close.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn host_shutdown_escalates_past_hung_lane_and_clears_entries() {
        let coordinator = Arc::new(HostLaneCoordinator::<FakeLaneCleanup>::default());
        let shutdown = AtomicBool::new(true);
        let hanging = FakeLaneCleanup::hanging();
        let sibling = FakeLaneCleanup::immediate();
        // Tests model entries which were published before the host shutdown
        // flag became sticky.
        let accepting = AtomicBool::new(false);
        coordinator
            .insert_if_open("lane-hung".into(), Arc::clone(&hanging), &accepting)
            .await
            .unwrap();
        coordinator
            .insert_if_open("lane-sibling".into(), Arc::clone(&sibling), &accepting)
            .await
            .unwrap();
        assert!(shutdown.load(Ordering::Acquire));

        let runtime_shutdowns = Arc::new(AtomicUsize::new(0));
        let task = {
            let coordinator = Arc::clone(&coordinator);
            let runtime_shutdowns = Arc::clone(&runtime_shutdowns);
            tokio::spawn(async move {
                coordinator
                    .shutdown_then_runtime(Duration::from_secs(1), async move {
                        runtime_shutdowns.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .await
            })
        };
        wait_for_close_calls(&hanging, 1).await;
        wait_for_close_calls(&sibling, 1).await;

        tokio::time::advance(Duration::from_secs(1)).await;
        task.await.unwrap().unwrap();
        assert_eq!(runtime_shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(
            coordinator.len().await,
            0,
            "successful process-tree cleanup is authoritative over a hung lane close"
        );
    }

    #[test]
    fn shutdown_state_is_sticky_after_cleanup_failure() {
        let shutdown = AtomicBool::new(false);
        shutdown.store(true, Ordering::Release);
        assert!(
            shutdown.load(Ordering::Acquire),
            "once shutdown begins, open_lane must remain fenced even when cleanup needs retry"
        );
    }

}
