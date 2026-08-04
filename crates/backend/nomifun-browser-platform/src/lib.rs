//! Main-process browser ownership, scheduling, and lifecycle authority.
//!
//! This crate deliberately does not launch Chromium itself.  A host-specific
//! adapter implements [`BrowserHostFactory`], while [`BrowserSessionHub`]
//! supplies the stable ownership, isolation, scheduling, lease, inventory, and
//! cleanup contract shared by Native, Gateway, ACP, remote, and cluster callers.

mod clock;
mod cleanup_budget;
mod driver;
mod error;
mod hub;
mod identity;
mod lease;
mod lifecycle;
mod model;
mod resource;
mod scheduler;

pub use clock::{Clock, ManualClock, SystemClock};
pub use driver::{
    BrowserHostDriver, BrowserHostFactory, BrowserLaneDriver, BrowserProcessIdentity,
    BrowserProfileFootprint, BrowserTaskDownloadAuthority, BrowserTaskDownloadReservation,
    BrowserTaskTabAuthority, BrowserTaskTabReservation, CapturedIdentitySnapshot,
    DriverOperationContext, HostLaunchCleanupLease, HostLaunchCleanupTicket, HostLaunchRequest,
    LaneFreezeOutcome, LaneLaunchRequest,
};
pub use error::{BrowserErrorCode, BrowserPlatformError};
pub use hub::{
    AnonymousProfilePolicy, BrowserLaneClient, BrowserSessionHub, HubConfig, OpenLaneOutcome,
    PrimaryProfilePolicy, MAX_RETAINED_COMPLETED_DOWNLOAD_FAMILIES, MAX_TASK_ACTIVE_DOWNLOADS,
    MAX_TASK_COMPLETED_DOWNLOAD_BYTES, MAX_TASK_COMPLETED_DOWNLOAD_FILES,
    MAX_TASK_SINGLE_DOWNLOAD_BYTES,
};
pub use identity::{
    CanonicalIdentitySnapshot, IdentitySnapshotPayload, SnapshotComponentCoverage,
    SnapshotCoverage,
};
pub use lease::{OwnerLease, OwnerLeaseService};
pub use lifecycle::{
    HOST_FAILURE_THRESHOLD, HOST_FAILURE_WINDOW_MS, HostCircuitBreaker, HostCircuitPolicy,
    HostCircuitSnapshot, HostRestartFlightResult, HostRestartSingleFlight,
    HostRestartTransition, PerKeyHostRestartSingleFlight, stale_browser_epoch_error,
};
pub use model::*;
pub use resource::{
    AUTOMATIC_TASK_MEMORY_BYTES, HIGH_CONCURRENCY_TASK_MEMORY_BYTES, MAX_ACTIVE_OPERATIONS,
    MAX_BROWSER_MEMORY_RATIO, MAX_GLOBAL_QUEUE, MAX_OPEN_LANES, MAX_OWNER_QUEUE,
    MAX_RESERVED_MEMORY_BYTES, MAX_TASK_ACTIVE_OPERATIONS, MAX_TASK_MEMORY_BYTES,
    MAX_TASK_OPEN_LANES, MAX_TASK_TABS, MIN_BROWSER_MEMORY_RATIO, MIN_RESERVED_MEMORY_BYTES,
    MIN_TASK_MEMORY_BYTES, RESOURCE_SAVING_TASK_MEMORY_BYTES,
    ResourceDecision, ResourcePolicy, ResourcePolicyPreset, ResourcePolicyValidationError,
    ResourceTelemetry, ResourceWorkload,
};
pub use scheduler::{
    Admission, BrowserLaneScheduler, LanePriority, PromotionPolicy, QueueRequest, SchedulerConfig,
};
