use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::{
    BrowserHostId, BrowserIdentityMode, BrowserLaneId, BrowserOperation,
    BrowserOperationResult, BrowserPlatformError, HostLifecycleState,
    IdentitySnapshotPayload, OperationContext, SnapshotCoverage,
};

struct HostLaunchCleanupState {
    lease_count: AtomicUsize,
    complete: AtomicBool,
    changed: tokio::sync::Notify,
}

/// Sticky observer for exact cleanup of one provisional physical Host launch.
///
/// The Hub keeps this ticket while the factory owns [`HostLaunchCleanupLease`].
/// A caller timeout or any factory terminal path must not be mistaken for
/// proof that no Chromium process remains while that authority is retained.
#[derive(Clone)]
pub struct HostLaunchCleanupTicket {
    inner: Arc<HostLaunchCleanupState>,
}

impl HostLaunchCleanupTicket {
    pub fn new() -> (Self, HostLaunchCleanupLease) {
        let inner = Arc::new(HostLaunchCleanupState {
            lease_count: AtomicUsize::new(1),
            complete: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        });
        (
            Self {
                inner: Arc::clone(&inner),
            },
            HostLaunchCleanupLease { inner },
        )
    }

    pub fn is_complete(&self) -> bool {
        self.inner.complete.load(Ordering::Acquire)
    }

    pub async fn wait(&self) {
        loop {
            let changed = self.inner.changed.notified();
            if self.is_complete() {
                return;
            }
            changed.await;
        }
    }
}

impl std::fmt::Debug for HostLaunchCleanupTicket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostLaunchCleanupTicket")
            .field("complete", &self.is_complete())
            .finish()
    }
}

/// Cloneable exact cleanup authority handed to a physical Host factory.
///
/// Every clone must remain indivisible from either the published driver or the
/// factory's cancellation-safe process/profile cleanup relay. The ticket is
/// completed only when the final authority clone is dropped.
pub struct HostLaunchCleanupLease {
    inner: Arc<HostLaunchCleanupState>,
}

impl Clone for HostLaunchCleanupLease {
    fn clone(&self) -> Self {
        let previous = self.inner.lease_count.fetch_add(1, Ordering::AcqRel);
        assert!(previous > 0, "completed Host launch cleanup lease cannot be cloned");
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl std::fmt::Debug for HostLaunchCleanupLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostLaunchCleanupLease")
            .field("opaque", &true)
            .finish()
    }
}

impl Drop for HostLaunchCleanupLease {
    fn drop(&mut self) {
        let previous = self.inner.lease_count.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "Host launch cleanup lease underflow");
        if previous == 1 {
            self.inner.complete.store(true, Ordering::Release);
            self.inner.changed.notify_waiters();
        }
    }
}

#[derive(Clone, Debug)]
pub struct HostLaunchRequest {
    pub host_id: BrowserHostId,
    /// Hub-assigned logical epoch. This is monotonic across replacements of
    /// the same HostKey even if an engine implementation resets its own local
    /// counter when a new process is constructed.
    pub browser_epoch: u64,
    pub identity_mode: BrowserIdentityMode,
    pub identity_generation: u64,
    /// Present only for an AuthenticatedReplica and resolved atomically from
    /// the Hub's canonical generation store.
    pub identity_snapshot_payload: Option<IdentitySnapshotPayload>,
    pub headful: bool,
    /// Provisional exact cleanup authority installed before the factory's
    /// first await. Production adapters must move this into the engine's
    /// process/profile cleanup chain before awaiting physical launch.
    pub cleanup_lease: HostLaunchCleanupLease,
}

#[derive(Clone, Debug)]
pub struct CapturedIdentitySnapshot {
    pub payload: IdentitySnapshotPayload,
    pub coverage: SnapshotCoverage,
}

/// Exact-enough OS process identity for resource telemetry joins.
///
/// A numeric PID alone is unsafe on long-running Windows installations because
/// it may be reused after Chromium exits while Hub cleanup is converging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BrowserProcessIdentity {
    pub process_id: u32,
    pub started_at_epoch_seconds: u64,
    /// Platform-native process creation key. Zero is reserved for legacy test
    /// drivers; production drivers must provide the exact captured key.
    pub platform_start_key: u64,
}

/// Bounded, no-follow measurement of one exact managed browser profile.
///
/// The caller supplies stop limits so an already-hostile directory cannot make
/// telemetry itself retain an unbounded walk. `limit_reached` means the walk
/// stopped as soon as either supplied ceiling was crossed; the reported values
/// are therefore lower bounds in that case.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BrowserProfileFootprint {
    pub bytes: u64,
    pub entries: u64,
    pub limit_reached: bool,
}

#[derive(Clone, Debug)]
pub struct LaneLaunchRequest {
    pub lane_id: BrowserLaneId,
    pub identity_mode: BrowserIdentityMode,
    pub workspace_hint: Option<String>,
    /// Trusted user-visible task-family quota key. Sibling runtimes in one
    /// conversation share it. It is never accepted from model arguments and
    /// must never be reused as runtime/owner cleanup authority.
    pub task_resource_key: String,
    pub max_task_tabs: usize,
    /// Hub-owned, task-global authority shared by every Host and identity.
    /// Engine adapters must reserve one slot before publishing any top-level
    /// target, including the Lane's initial page and opener-created popups.
    pub task_tab_authority: Arc<dyn BrowserTaskTabAuthority>,
    /// Hub-owned, task-global download authority. Unlike Host-local routing
    /// state, its completed byte/file ledger survives Host and runtime
    /// replacement until the last owner of the logical task is retired.
    pub task_download_authority: Arc<dyn BrowserTaskDownloadAuthority>,
}

/// One exact task-tab slot. The concrete Hub permit releases its reservation
/// when the last `Arc` is dropped after target absence has been proven.
pub trait BrowserTaskTabReservation: Send + Sync {}

/// Cross-Host tab authority supplied by the Hub to every managed Lane.
///
/// `reservation_key` is stable within a task (for example a create nonce or a
/// complete target id). Repeating a reservation for the same live key must be
/// idempotent and return the same logical slot rather than double charging it.
#[async_trait]
pub trait BrowserTaskTabAuthority: Send + Sync + std::fmt::Debug {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        reservation_key: &str,
    ) -> Result<Arc<dyn BrowserTaskTabReservation>, BrowserPlatformError>;
}

/// One in-flight task download reservation.
///
/// Dropping an uncompleted reservation releases only its active byte/count
/// charge. Completion is a two-phase transaction: [`Self::prepare_complete`]
/// reserves the final byte/file charge, an output is atomically published with
/// no intervening await, then [`Self::finalize_complete`] makes the charge
/// permanent. This prevents either an uncharged published file or a permanent
/// charge for a failed filesystem publication.
pub trait BrowserTaskDownloadReservation: Send + Sync {
    /// Monotonically account the largest observed received/declared size.
    /// Implementations fail closed if either the single-file or cumulative
    /// task boundary would be crossed.
    fn update_progress(
        &self,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), BrowserPlatformError>;

    /// Reserve a completion byte/file charge without making it permanent.
    fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserPlatformError>;

    /// Finalize a previously prepared charge. Implementations make this
    /// idempotent and infallible so an RAII output guard can conservatively
    /// charge an artifact which could not be rolled back.
    fn finalize_complete(&self);

    /// Convenience for non-filesystem callers and tests.
    fn complete(&self, actual_bytes: u64) -> Result<(), BrowserPlatformError> {
        self.prepare_complete(actual_bytes)?;
        self.finalize_complete();
        Ok(())
    }
}

/// Cross-Host task download admission authority supplied by the Hub.
///
/// `download_key` is a bounded, Host-generated GUID (or a process-generated
/// direct-output nonce). Repeating the same live `(lane, key)` is idempotent;
/// runtime or Host replacement cannot mint a fresh completed-byte budget.
#[async_trait]
pub trait BrowserTaskDownloadAuthority: Send + Sync + std::fmt::Debug {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        download_key: &str,
    ) -> Result<Arc<dyn BrowserTaskDownloadReservation>, BrowserPlatformError>;
}

#[derive(Clone)]
pub struct DriverOperationContext {
    pub operation: OperationContext,
    pub cancellation: CancellationToken,
    /// Set only by a trusted in-process transport after consuming a matching
    /// one-shot human approval. This authority is deliberately absent from
    /// [`BrowserOperation`] and therefore cannot be supplied by model JSON.
    pub trusted_out_of_band_confirmation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneFreezeOutcome {
    Frozen,
    Unsupported,
}

#[async_trait]
pub trait BrowserLaneDriver: Send + Sync {
    async fn execute(
        &self,
        operation: BrowserOperation,
        context: DriverOperationContext,
    ) -> Result<BrowserOperationResult, BrowserPlatformError>;

    /// Must be idempotent and must not wait for the lane operation gate.
    /// Returning `Ok(())` is an exact cleanup proof: the target and every
    /// target-local resource must no longer exist. On `Err`, panic, timeout,
    /// or cancellation the Hub conservatively retains authority and may stop
    /// the entire exact Host; implementations must never report success while
    /// cleanup is merely queued in an untracked background task.
    async fn close(&self) -> Result<(), BrowserPlatformError>;

    /// Trusted process-internal seam for foregrounding this Lane's visible
    /// browser window.
    ///
    /// This is deliberately separate from [`Self::execute`] so model JSON and
    /// ordinary browser capabilities cannot request operating-system focus.
    /// Drivers that can foreground a headful browser should override it; the
    /// fail-closed default reports that the capability is unavailable.
    async fn bring_to_front(&self) -> Result<(), BrowserPlatformError> {
        Err(BrowserPlatformError::new(
            crate::BrowserErrorCode::OperationNotAllowed,
            "This browser lane cannot be brought to the foreground.",
            false,
            "Use a running Primary browser lane with a visible browser window.",
        ))
    }

    /// Trusted process-internal Primary identity capture seam. This is not a
    /// browser operation and therefore cannot be invoked from model JSON.
    async fn capture_identity_snapshot(
        &self,
    ) -> Result<Option<CapturedIdentitySnapshot>, BrowserPlatformError> {
        Ok(None)
    }

    /// Best-effort resource-pressure freeze.
    ///
    /// Returning [`LaneFreezeOutcome::Frozen`] is only valid when the complete
    /// production stack also exposes a reliable path that resumes the same
    /// Lane before its next operation. Adapters with a one-way lifecycle API,
    /// or no lifecycle API, must return `Unsupported`; the Hub then falls back
    /// to closing the idle lane instead of retaining a fake frozen Lane.
    async fn freeze(&self) -> Result<LaneFreezeOutcome, BrowserPlatformError> {
        Ok(LaneFreezeOutcome::Unsupported)
    }
}

#[async_trait]
pub trait BrowserHostDriver: Send + Sync {
    fn host_id(&self) -> BrowserHostId;
    fn epoch(&self) -> u64;
    fn state(&self) -> HostLifecycleState;
    /// Whether this Host was launched with a real native browser window.
    ///
    /// A headless Host cannot be made visible through CDP window commands;
    /// callers must perform an explicit, trusted Host replacement instead.
    fn is_headful(&self) -> bool {
        false
    }
    fn process_id(&self) -> Option<u32> {
        None
    }
    fn process_identity(&self) -> Option<BrowserProcessIdentity> {
        self.process_id().map(|process_id| BrowserProcessIdentity {
            process_id,
            // Zero is an explicit legacy/unknown marker. Production Chromium
            // drivers must override this method with a verified start time.
            started_at_epoch_seconds: 0,
            platform_start_key: 0,
        })
    }

    /// Measure the exact application-owned profile backing this Host.
    ///
    /// Production managed Chromium drivers override this with a bounded,
    /// no-follow filesystem walk. Other drivers return `None`; the Hub never
    /// guesses a path or scans outside driver-owned storage.
    async fn profile_footprint(
        &self,
        _stop_after_bytes: u64,
        _stop_after_entries: u64,
    ) -> Result<Option<BrowserProfileFootprint>, BrowserPlatformError> {
        Ok(None)
    }

    async fn open_lane(
        &self,
        request: LaneLaunchRequest,
    ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError>;

    /// Atomically installs a Host-local defense-in-depth ceiling for one
    /// trusted task and closes excess targets before returning. The Hub owns
    /// the cross-Host reservation authority; this seam is the executor used
    /// while lowering policy and during Host rebind.
    async fn reconcile_task_tab_limit(
        &self,
        _task_resource_key: &str,
        _max_task_tabs: usize,
    ) -> Result<(), BrowserPlatformError> {
        Err(BrowserPlatformError::new(
            crate::BrowserErrorCode::BrowserUnavailable,
            "This browser Host cannot reconcile a live task tab limit.",
            false,
            "Restart the affected browser Host before applying a lower tab limit.",
        ))
    }

    /// Stops this exact Host process tree and all of its targets.
    ///
    /// Returning `Ok(())` is the proof used by the Hub to release Host and
    /// residual Lane cleanup authority. Implementations must therefore be
    /// idempotent and return success only after exact process-tree absence is
    /// established. A deferred retry must retain its own durable authority and
    /// return `Err` until that proof is available.
    async fn shutdown(&self) -> Result<(), BrowserPlatformError>;
}

#[async_trait]
/// Factory boundary for one physical browser Host.
///
/// [`Self::launch`] **must be cancellation-safe**. Before spawning any OS
/// process, an implementation must install durable, exact cleanup authority
/// that survives this future being dropped, returning `Err`, or panicking.
/// Those terminal paths must not leave an untracked process or profile behind.
/// The Hub assumes responsibility for calling [`BrowserHostDriver::shutdown`]
/// only after `launch` successfully returns the driver; before that handoff,
/// cleanup remains the factory implementation's responsibility.
pub trait BrowserHostFactory: Send + Sync {
    async fn launch(
        &self,
        request: HostLaunchRequest,
    ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserErrorCode, BrowserOperation};

    #[tokio::test]
    async fn host_launch_cleanup_ticket_completes_only_after_final_lease_drop() {
        let (ticket, lease) = HostLaunchCleanupTicket::new();
        let sibling = lease.clone();
        drop(lease);
        assert!(!ticket.is_complete());

        let waiting = {
            let ticket = ticket.clone();
            tokio::spawn(async move { ticket.wait().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        drop(sibling);
        waiting.await.unwrap();
        assert!(ticket.is_complete());

        // Completion is sticky for late waiters.
        ticket.wait().await;
    }

    struct DriverWithoutForegroundSupport;

    #[async_trait]
    impl BrowserLaneDriver for DriverWithoutForegroundSupport {
        async fn execute(
            &self,
            _operation: BrowserOperation,
            _context: DriverOperationContext,
        ) -> Result<BrowserOperationResult, BrowserPlatformError> {
            Ok(BrowserOperationResult::default())
        }

        async fn close(&self) -> Result<(), BrowserPlatformError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn foreground_seam_defaults_to_fail_closed() {
        let error = DriverWithoutForegroundSupport
            .bring_to_front()
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(!error.retryable);
        assert!(error.lane_id.is_none());
    }
}
