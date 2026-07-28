use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::{
    BrowserHostId, BrowserIdentityMode, BrowserLaneId, BrowserOperation,
    BrowserOperationResult, BrowserPlatformError, HostLifecycleState,
    IdentitySnapshotPayload, OperationContext, SnapshotCoverage,
};

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
}

#[derive(Clone, Debug)]
pub struct CapturedIdentitySnapshot {
    pub payload: IdentitySnapshotPayload,
    pub coverage: SnapshotCoverage,
}

#[derive(Clone, Debug)]
pub struct LaneLaunchRequest {
    pub lane_id: BrowserLaneId,
    pub identity_mode: BrowserIdentityMode,
    pub workspace_hint: Option<String>,
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

    async fn open_lane(
        &self,
        request: LaneLaunchRequest,
    ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError>;

    async fn shutdown(&self) -> Result<(), BrowserPlatformError>;
}

#[async_trait]
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
