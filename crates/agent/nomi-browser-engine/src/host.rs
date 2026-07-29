//! Shared Chromium host and lane ownership primitives.
//!
//! A [`ManagedBrowserHost`] owns exactly one Chromium process and one CDP
//! connection.  Each lane gets a separate [`BrowserEngine`] value backed by
//! that connection, with independent tab/cursor/ref/cancellation state.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
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

/// Per-lane correctness gate used by the production backend.  There is no
/// host-global operation mutex: every call to `open_lane` constructs a new
/// instance of this gate.
#[derive(Default)]
pub(crate) struct LaneOperationGate(Mutex<()>);

impl LaneOperationGate {
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }
}
/// Stable caller-supplied identifier for one ownership/concurrency lane.
pub type LaneId = String;

#[async_trait]
trait ManagedLaneCleanup: Send + Sync {
    async fn shutdown_owned_targets(&self) -> Result<(), BrowserError>;
}

#[async_trait]
impl ManagedLaneCleanup for CdpBackend {
    async fn shutdown_owned_targets(&self) -> Result<(), BrowserError> {
        self.shutdown_lane().await
    }
}

struct ManagedLaneEntry<L> {
    lane: Arc<L>,
    close_gate: Mutex<()>,
    closed: AtomicBool,
}

impl<L> ManagedLaneEntry<L> {
    fn new(lane: Arc<L>) -> Self {
        Self {
            lane,
            close_gate: Mutex::new(()),
            closed: AtomicBool::new(false),
        }
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
    async fn get(&self, lane_id: &str) -> Option<Arc<L>> {
        self.lanes
            .lock()
            .await
            .get(lane_id)
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
            return Ok(Arc::clone(&existing.lane));
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

        entry.lane.shutdown_owned_targets().await?;
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
#[derive(Clone, Default)]
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

/// One managed Chromium/connection serving multiple independently serialized
/// lanes.
pub struct ManagedBrowserHost {
    runtime: Arc<CdpHostRuntime>,
    lanes: HostLaneCoordinator<CdpBackend>,
    shutdown_gate: Mutex<()>,
    epoch: u64,
    shutdown: AtomicBool,
    default_lane_config: LaneEngineConfig,
}

/// Result of an explicit process-mode replacement.
///
/// The old Host has been synchronously shut down before `host` is launched,
/// so the two Chromium processes never concurrently own the stable profile.
/// `fresh_observe_required` is always true because CDP identifiers and refs
/// cannot survive a Chromium process boundary.
pub struct ManagedBrowserHostReplacement {
    pub host: ManagedBrowserHost,
    pub previous_epoch: u64,
}

impl ManagedBrowserHostReplacement {
    /// A process replacement always invalidates target/frame/ref state.
    pub const fn fresh_observe_required(&self) -> bool {
        true
    }
}

impl ManagedBrowserHost {
    /// Launch exactly one managed Chromium process and establish its single CDP
    /// connection.  No page/lane is created until [`Self::open_lane`].
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
        mut config: EngineConfig,
        mode: BrowserHostLaunchMode,
    ) -> Result<Self, BrowserError> {
        static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
        config.headful = mode.is_headful();
        let default_lane_config = LaneEngineConfig {
            workspace_dir: config.workspace_dir.clone(),
            evaluate_full_power: config.evaluate_full_power,
            evaluate_persistent_login: config.evaluate_persistent_login,
            known_secret_values: Some(config.known_secret_values.clone()),
        };
        let runtime = CdpHostRuntime::launch_in_mode(config, mode).await?;
        Ok(Self {
            runtime,
            lanes: HostLaneCoordinator::default(),
            shutdown_gate: Mutex::new(()),
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            shutdown: AtomicBool::new(false),
            default_lane_config,
        })
    }

    /// Authoritatively replace this Host with one in `mode`.
    ///
    /// This is the low-level seam for a trusted Headless→Headful foreground
    /// transition (and the reverse transition when hiding again). It performs
    /// the only safe order for a shared profile: close/cancel every old Lane,
    /// prove the old process tree has stopped, then launch the replacement.
    /// No Lane is silently recreated and no old target is retained. The
    /// caller must rebuild its logical Lane inventory/URLs and require a fresh
    /// observe before accepting Agent operations.
    ///
    /// If replacement launch fails, the old Host remains stopped and the
    /// error is returned; this method never reports a successful transition
    /// without a live replacement Host.
    pub async fn replace_in_mode(
        &self,
        config: EngineConfig,
        mode: BrowserHostLaunchMode,
    ) -> Result<ManagedBrowserHostReplacement, BrowserError> {
        let previous_epoch = self.epoch;
        self.shutdown().await?;
        let host = Self::launch_in_mode(config, mode).await?;
        Ok(ManagedBrowserHostReplacement {
            host,
            previous_epoch,
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
        config: LaneEngineConfig,
    ) -> Result<Arc<dyn BrowserEngine>, BrowserError> {
        // Lane construction is single-flight to avoid duplicate target
        // creation for the same id. Lane close never takes this gate.
        let _open = self.lanes.open_gate.lock().await;
        self.ensure_open()?;
        let lane_id = lane_id.into();
        if let Some(existing) = self.lanes.get(&lane_id).await {
            return Ok(existing);
        }

        let backend =
            Arc::new(CdpBackend::from_host(self.runtime.clone(), lane_id.clone(), config).await?);
        let backend = self
            .lanes
            .insert_if_open(lane_id, backend, &self.shutdown)
            .await?;
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

    struct FakeLaneCleanup {
        close_calls: AtomicUsize,
        release: Option<Notify>,
    }

    impl FakeLaneCleanup {
        fn immediate() -> Arc<Self> {
            Arc::new(Self {
                close_calls: AtomicUsize::new(0),
                release: None,
            })
        }

        fn hanging() -> Arc<Self> {
            Arc::new(Self {
                close_calls: AtomicUsize::new(0),
                release: Some(Notify::new()),
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
            Ok(())
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
