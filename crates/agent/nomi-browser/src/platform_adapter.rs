//! Production bridge from the Browser Platform authority to the shared CDP
//! host.
//!
//! This module is intentionally the only place which translates the platform's
//! generic JSON operation envelope into `nomi-browser-engine` calls.  The Hub
//! remains responsible for trusted caller identity, leases, scheduling and the
//! per-lane operation gate; this adapter owns engine-specific dispatch and
//! converts every engine failure into the stable, display-safe platform error
//! taxonomy.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use nomi_browser_engine::{
    ActResult, ActSpec, BrowserEngine, BrowserError, BrowserTabInfo, EngineConfig,
    LaneEngineConfig, ManagedBrowserHost, ObserveOpts, Observation, TaskTabReservation,
    TaskDownloadReservation, TaskDownloadReservationAuthority, TaskTabReservationAuthority,
};
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserHostDriver, BrowserHostFactory, BrowserHostId,
    BrowserIdentityMode, BrowserLaneDriver, BrowserLaneId, BrowserOperation,
    BrowserOperationKind, BrowserOperationResult, BrowserPlatformError,
    BrowserProfileFootprint, BrowserTabSnapshot, BrowserTaskDownloadAuthority,
    BrowserTaskDownloadReservation, BrowserTaskTabAuthority, BrowserTaskTabReservation,
    CapturedIdentitySnapshot, DriverOperationContext,
    HostLaunchRequest, HostLifecycleState, IdentitySnapshotPayload, LaneFreezeOutcome,
    LaneLaunchRequest, SnapshotCoverage,
};
#[cfg(test)]
use nomifun_browser_platform::HostLaunchCleanupTicket;
use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct, Serializer};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::BrowserTool;

const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_ACTION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MANAGED_NAVIGATION_URL_BYTES: usize = 64 * 1024;
/// F22: per-operation bound on generation catch-up observations. Each catch-up
/// step is a full engine observation (multi-frame CDP AX snapshot) holding the
/// lane's operation gate, so one operation must never storm thousands of them
/// back-to-back. The consumed generations are kept by the engine, so a fence
/// beyond this bound fails retryable and every retry makes monotonic progress.
const MAX_OBSERVATION_GENERATION_CATCH_UP: u64 = 32;

/// Synchronous, side-effect-free resolver used immediately before launching a
/// host.  Applications which load an authenticated replica from a vault can
/// inject the resulting `storage_state` here without teaching the platform core
/// about engine configuration.
pub type EngineConfigResolver = Arc<
    dyn Fn(&HostLaunchRequest) -> Result<EngineConfig, BrowserPlatformError> + Send + Sync,
>;

type ManagedHostLaunchFuture =
    Pin<Box<dyn Future<Output = Result<ManagedBrowserHost, BrowserError>> + Send>>;
type ManagedHostLauncher = Arc<
    dyn Fn(EngineConfig, nomi_browser_engine::HostCleanupLease) -> ManagedHostLaunchFuture
        + Send
        + Sync,
>;

/// Trusted composition-root hook for adding the existing BrowserTool policy
/// locator) to each managed Lane. Model input never reaches this hook.
pub type ManagedLanePolicyDecorator =
    Arc<dyn Fn(BrowserTool) -> BrowserTool + Send + Sync>;

type IdentitySnapshotPersister =
    Arc<dyn Fn(&Value) -> Result<(), BrowserPlatformError> + Send + Sync>;

#[derive(Debug)]
struct EngineTaskTabAuthorityBridge {
    inner: Arc<dyn BrowserTaskTabAuthority>,
}

struct EngineTaskTabReservationBridge {
    _inner: Arc<dyn BrowserTaskTabReservation>,
}

impl TaskTabReservation for EngineTaskTabReservationBridge {}

#[async_trait]
impl TaskTabReservationAuthority for EngineTaskTabAuthorityBridge {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        reservation_key: &str,
    ) -> Result<Arc<dyn TaskTabReservation>, BrowserError> {
        let reservation = self
            .inner
            .reserve(task_resource_key, lane_id, reservation_key)
            .await
            .map_err(|_| BrowserError::Blocked {
                reason: "the task-wide browser tab capacity is unavailable".into(),
            })?;
        Ok(Arc::new(EngineTaskTabReservationBridge {
            _inner: reservation,
        }))
    }
}

#[derive(Debug)]
struct EngineTaskDownloadAuthorityBridge {
    inner: Arc<dyn BrowserTaskDownloadAuthority>,
}

struct EngineTaskDownloadReservationBridge {
    inner: Arc<dyn BrowserTaskDownloadReservation>,
}

impl TaskDownloadReservation for EngineTaskDownloadReservationBridge {
    fn update_progress(
        &self,
        received_bytes: u64,
        total_bytes: Option<u64>,
    ) -> Result<(), BrowserError> {
        self.inner
            .update_progress(received_bytes, total_bytes)
            .map_err(|_| BrowserError::Blocked {
                reason: "the task-wide browser download byte capacity is unavailable".into(),
            })
    }

    fn prepare_complete(&self, actual_bytes: u64) -> Result<(), BrowserError> {
        self.inner
            .prepare_complete(actual_bytes)
            .map_err(|_| BrowserError::Blocked {
                reason: "the task-wide browser completed-download capacity is unavailable".into(),
            })
    }

    fn finalize_complete(&self) {
        self.inner.finalize_complete();
    }
}

#[async_trait]
impl TaskDownloadReservationAuthority for EngineTaskDownloadAuthorityBridge {
    async fn reserve(
        &self,
        task_resource_key: &str,
        lane_id: &str,
        download_key: &str,
    ) -> Result<Arc<dyn TaskDownloadReservation>, BrowserError> {
        let reservation = self
            .inner
            .reserve(task_resource_key, lane_id, download_key)
            .await
            .map_err(|_| BrowserError::Blocked {
                reason: "the task-wide browser download capacity is unavailable".into(),
            })?;
        Ok(Arc::new(EngineTaskDownloadReservationBridge {
            inner: reservation,
        }))
    }
}

/// Production `BrowserHostFactory` backed by `ManagedBrowserHost`.
#[derive(Clone)]
pub struct ManagedEngineHostFactory {
    resolver: EngineConfigResolver,
    /// Kept as one private seam so cancellation tests exercise the exact await
    /// boundary used in production. The default launcher is always
    /// `ManagedBrowserHost::launch_platform_managed`.
    host_launcher: ManagedHostLauncher,
    lane_policy: ManagedLanePolicyDecorator,
    identity_snapshot_persister: Option<IdentitySnapshotPersister>,
}

impl ManagedEngineHostFactory {
    /// Build a safe default resolver from an engine template.
    ///
    /// Profiles are derived below `<template.data_dir>/platform-profiles`.
    /// Primary identity is stable for one identity generation. Anonymous,
    /// replica and isolated hosts receive unique ephemeral profiles.
    pub fn new(template: EngineConfig) -> Self {
        let profiles_root = template.data_dir.join("platform-profiles");
        Self::with_profiles_root(template, profiles_root)
    }

    /// Same as [`Self::new`], with an explicit application-owned profile root.
    pub fn with_profiles_root(template: EngineConfig, profiles_root: PathBuf) -> Self {
        Self::from_config_resolver(Arc::new(move |request| {
            derive_host_config(&template, &profiles_root, request)
        }))
    }

    /// Use an application resolver for identity vault and profile policy.
    ///
    /// The resolver must still return an application-owned `user_data_dir`; the
    /// adapter never points Chromium at the user's real browser profile.
    pub fn from_config_resolver(resolver: EngineConfigResolver) -> Self {
        Self {
            resolver,
            host_launcher: Arc::new(|config, cleanup_lease| {
                Box::pin(ManagedBrowserHost::launch_platform_managed_with_cleanup_lease(
                    config,
                    cleanup_lease,
                ))
            }),
            lane_policy: Arc::new(|policy| policy),
            identity_snapshot_persister: None,
        }
    }

    /// Decorate the managed policy with trusted application-owned services.
    /// The decorator cannot be supplied by model input.
    pub fn with_lane_policy(
        mut self,
        decorator: ManagedLanePolicyDecorator,
    ) -> Self {
        self.lane_policy = decorator;
        self
    }

    /// Persist each trusted Primary capture to the encrypted shared vault
    /// before the Hub commits its canonical generation.
    pub fn with_identity_vault(
        mut self,
        vault_path: PathBuf,
        encryption_key: [u8; 32],
    ) -> Self {
        self.identity_snapshot_persister = Some(Arc::new(move |payload| {
            let state = nomi_browser_engine::StorageState::from_json(payload.clone())
                .map_err(|_| identity_capture_error())?
                .into_cookie_only();
            nomi_browser_engine::save_storage_state(
                &state,
                &vault_path,
                &encryption_key,
            )
            .map_err(|_| identity_capture_error())
        }));
        self
    }
}

#[async_trait]
impl BrowserHostFactory for ManagedEngineHostFactory {
    async fn launch(
        &self,
        request: HostLaunchRequest,
    ) -> Result<Arc<dyn BrowserHostDriver>, BrowserPlatformError> {
        let config = (self.resolver)(&request)?;
        if config.user_data_dir.is_none() {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser profile is not configured.",
                false,
                "Configure an application-owned browser profile directory.",
            ));
        }

        let defaults = LaneEngineConfig {
            workspace_dir: config.workspace_dir.clone(),
            evaluate_full_power: config.evaluate_full_power,
            evaluate_persistent_login: config.evaluate_persistent_login,
            known_secret_values: Some(config.known_secret_values.clone()),
            task_resource_key: None,
            max_task_tabs: 16,
            task_tab_reservation_authority: None,
            task_download_reservation_authority: None,
        };
        let data_dir = config.data_dir.clone();
        let profile_dir = config.user_data_dir.clone();
        // Move the Hub's provisional physical-cleanup authority into the
        // engine future before its first await. If this factory future is
        // cancelled, the engine's launch guards/relay retain the same opaque
        // lease until exact process/profile cleanup completes.
        let cleanup_lease = nomi_browser_engine::HostCleanupLease::new(request.cleanup_lease);
        let host = Arc::new(
            (self.host_launcher)(config, cleanup_lease)
                .await
                .map_err(map_engine_error)?,
        );
        // Capture the root identity immediately after the engine has launched
        // and created its profile. Measurement later must match this exact
        // directory identity before and after the bounded walk.
        let profile_root = profile_dir.map(|path| {
            capture_profile_directory_identity(&path)
                .map(ManagedProfileRoot::Captured)
                .unwrap_or(ManagedProfileRoot::Unavailable)
        });
        // Record the effective mode after display-capability probing. A
        // requested Headful launch is forced Headless on machines without a
        // usable display and must not be reported as foregroundable.
        let headful = host.launch_mode().is_headful();
        Ok(Arc::new(ManagedEngineHostDriver {
            host_id: request.host_id,
            epoch: request.browser_epoch,
            identity_mode: request.identity_mode,
            host,
            defaults,
            data_dir,
            profile_root,
            headful,
            lane_policy: self.lane_policy.clone(),
            identity_snapshot_persister: self.identity_snapshot_persister.clone(),
            state: AtomicU8::new(HostState::Running as u8),
            shutdown_gate: AsyncMutex::new(()),
            profile_footprint_sampler: Arc::new(ManagedProfileFootprintSampler::default()),
        }))
    }
}

/// Platform host driver wrapping exactly one shared Chromium/CDP connection.
struct ManagedEngineHostDriver {
    host_id: BrowserHostId,
    epoch: u64,
    identity_mode: BrowserIdentityMode,
    host: Arc<ManagedBrowserHost>,
    defaults: LaneEngineConfig,
    data_dir: PathBuf,
    /// Exact application-owned profile passed to this Chromium process. The
    /// platform never guesses this path; bounded footprint telemetry stays in
    /// the adapter that created it.
    profile_root: Option<ManagedProfileRoot>,
    headful: bool,
    lane_policy: ManagedLanePolicyDecorator,
    identity_snapshot_persister: Option<IdentitySnapshotPersister>,
    state: AtomicU8,
    shutdown_gate: AsyncMutex<()>,
    /// Host-owned single-flight authority for the blocking profile walk.
    /// Dropping a request future must not detach one blocking walk and let the
    /// next request enqueue another. A completed flight stays as a one-item
    /// mailbox until a waiter consumes it, so cancellation cannot discard a
    /// limit/error result either.
    profile_footprint_sampler: Arc<ManagedProfileFootprintSampler>,
}

fn new_lane_known_secret_values() -> nomi_browser_engine::KnownSecretValues {
    nomi_browser_engine::KnownSecretValues::default()
}

#[repr(u8)]
enum HostState {
    Running = 1,
    Stopping = 2,
    Stopped = 3,
    Failed = 4,
}

impl ManagedEngineHostDriver {
    fn lifecycle_state(&self) -> HostLifecycleState {
        match self.state.load(Ordering::Acquire) {
            value if value == HostState::Running as u8 => HostLifecycleState::Running,
            value if value == HostState::Stopping as u8 => HostLifecycleState::Stopping,
            value if value == HostState::Stopped as u8 => HostLifecycleState::Stopped,
            _ => HostLifecycleState::Failed,
        }
    }
}

const PROFILE_FOOTPRINT_MAX_DEPTH: usize = 256;
const PROFILE_FOOTPRINT_MAX_ACTIVE_PATH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct ManagedProfileFootprintSampler {
    flight: StdMutex<Option<Arc<ManagedProfileFootprintFlight>>>,
}

struct ManagedProfileFootprintFlight {
    result: OnceLock<Result<BrowserProfileFootprint, BrowserPlatformError>>,
    changed: Notify,
}

impl ManagedProfileFootprintFlight {
    fn new() -> Self {
        Self {
            result: OnceLock::new(),
            changed: Notify::new(),
        }
    }

    fn complete(&self, result: Result<BrowserProfileFootprint, BrowserPlatformError>) {
        if self.result.set(result).is_ok() {
            self.changed.notify_waiters();
        }
    }

    async fn wait(&self) -> Result<BrowserProfileFootprint, BrowserPlatformError> {
        loop {
            let changed = self.changed.notified();
            if let Some(result) = self.result.get() {
                return result.clone();
            }
            changed.await;
        }
    }
}

impl ManagedProfileFootprintSampler {
    fn claim(&self) -> (Arc<ManagedProfileFootprintFlight>, bool) {
        let mut current = self
            .flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(flight) = current.as_ref() {
            return (Arc::clone(flight), false);
        }
        let flight = Arc::new(ManagedProfileFootprintFlight::new());
        *current = Some(Arc::clone(&flight));
        (flight, true)
    }

    fn consume(&self, flight: &Arc<ManagedProfileFootprintFlight>) {
        let mut current = self
            .flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, flight))
        {
            current.take();
        }
    }
}

fn profile_footprint_worker_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser profile footprint worker stopped unexpectedly.",
        true,
        "Retry after the exact Anonymous browser Host is retired.",
    )
}

fn profile_footprint_measurement_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser profile footprint could not be measured safely.",
        true,
        "Retry after the exact Anonymous browser Host is retired.",
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ManagedProfileRoot {
    Captured(ProfileDirectoryIdentity),
    /// The engine did receive a profile path, but the adapter could not bind
    /// it to one exact directory identity immediately after launch. Anonymous
    /// hygiene treats this as unavailable telemetry and fails closed.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileDirectoryIdentity {
    canonical_path: PathBuf,
    platform_identity: ProfilePlatformIdentity,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfilePlatformIdentity {
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfilePlatformIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(any(windows, unix)))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProfilePlatformIdentity {
    creation_time_nanos: u128,
}

#[cfg(windows)]
fn profile_metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn profile_metadata_is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
#[allow(non_snake_case)]
#[repr(C)]
struct ProfileWindowsFileTime {
    dwLowDateTime: u32,
    dwHighDateTime: u32,
}

#[cfg(windows)]
#[allow(non_snake_case)]
#[repr(C)]
struct ProfileWindowsByHandleFileInformation {
    dwFileAttributes: u32,
    ftCreationTime: ProfileWindowsFileTime,
    ftLastAccessTime: ProfileWindowsFileTime,
    ftLastWriteTime: ProfileWindowsFileTime,
    dwVolumeSerialNumber: u32,
    nFileSizeHigh: u32,
    nFileSizeLow: u32,
    nNumberOfLinks: u32,
    nFileIndexHigh: u32,
    nFileIndexLow: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: *mut std::ffi::c_void,
        information: *mut ProfileWindowsByHandleFileInformation,
    ) -> i32;
}

#[cfg(windows)]
fn profile_platform_identity(
    path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<ProfilePlatformIdentity, String> {
    use std::mem::MaybeUninit;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    let directory = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "open managed browser profile identity handle".to_owned())?;
    let handle_metadata = directory
        .metadata()
        .map_err(|_| "inspect managed browser profile identity handle".to_owned())?;
    if !handle_metadata.is_dir()
        || handle_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("managed browser profile identity handle is unsafe".to_owned());
    }
    let mut information = MaybeUninit::<ProfileWindowsByHandleFileInformation>::zeroed();
    // SAFETY: the borrowed directory handle is live and the output points to
    // correctly sized writable storage; success is checked before assume_init.
    let succeeded = unsafe {
        GetFileInformationByHandle(directory.as_raw_handle(), information.as_mut_ptr())
    };
    if succeeded == 0 {
        return Err("inspect managed browser profile filesystem identity".to_owned());
    }
    // SAFETY: GetFileInformationByHandle reported full initialization.
    let information = unsafe { information.assume_init() };
    Ok(ProfilePlatformIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(unix)]
fn profile_platform_identity(
    _path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<ProfilePlatformIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(ProfilePlatformIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn profile_platform_identity(
    _path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<ProfilePlatformIdentity, String> {
    let creation_time_nanos = metadata
        .created()
        .ok()
        .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(ProfilePlatformIdentity {
        creation_time_nanos,
    })
}

#[cfg(windows)]
fn profile_os_str_storage_bytes(value: &std::ffi::OsStr) -> usize {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().count().saturating_mul(2)
}

#[cfg(unix)]
fn profile_os_str_storage_bytes(value: &std::ffi::OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().len()
}

#[cfg(not(any(windows, unix)))]
fn profile_os_str_storage_bytes(value: &std::ffi::OsStr) -> usize {
    value.to_string_lossy().len()
}

fn capture_profile_directory_identity(root: &Path) -> Result<ProfileDirectoryIdentity, String> {
    let original_metadata = std::fs::symlink_metadata(root)
        .map_err(|_| "inspect managed browser profile root metadata".to_owned())?;
    if !original_metadata.is_dir()
        || original_metadata.file_type().is_symlink()
        || profile_metadata_is_reparse_point(&original_metadata)
    {
        return Err("managed browser profile root is not an owned directory".to_owned());
    }
    let canonical_path = std::fs::canonicalize(root)
        .map_err(|_| "resolve managed browser profile root identity".to_owned())?;
    let canonical_metadata = std::fs::symlink_metadata(&canonical_path)
        .map_err(|_| "inspect resolved managed browser profile root metadata".to_owned())?;
    if !canonical_metadata.is_dir()
        || canonical_metadata.file_type().is_symlink()
        || profile_metadata_is_reparse_point(&canonical_metadata)
        || profile_platform_identity(&canonical_path, &canonical_metadata)?
            != profile_platform_identity(root, &original_metadata)?
    {
        return Err("managed browser profile root identity is unsafe".to_owned());
    }
    let platform_identity = profile_platform_identity(&canonical_path, &canonical_metadata)?;
    Ok(ProfileDirectoryIdentity {
        canonical_path,
        platform_identity,
    })
}

fn capture_profile_child_directory_identity(
    directory: &Path,
    exact_root: &ProfileDirectoryIdentity,
) -> Result<ProfileDirectoryIdentity, String> {
    let identity = capture_profile_directory_identity(directory)?;
    if !identity
        .canonical_path
        .starts_with(&exact_root.canonical_path)
    {
        return Err("managed browser profile traversal escaped its owned root".to_owned());
    }
    Ok(identity)
}

fn walk_profile_directory(
    directory: &ProfileDirectoryIdentity,
    exact_root: &ProfileDirectoryIdentity,
    stop_after_bytes: u64,
    stop_after_entries: u64,
    depth: usize,
    active_path_bytes: usize,
    footprint: &mut BrowserProfileFootprint,
) -> Result<(), String> {
    if depth > PROFILE_FOOTPRINT_MAX_DEPTH
        || active_path_bytes > PROFILE_FOOTPRINT_MAX_ACTIVE_PATH_BYTES
    {
        footprint.limit_reached = true;
        return Ok(());
    }
    let entries = std::fs::read_dir(&directory.canonical_path)
        .map_err(|_| "inspect managed browser profile footprint".to_owned())?;
    for entry in entries {
        let entry = entry.map_err(|_| "inspect managed browser profile entry".to_owned())?;
        let file_name = entry.file_name();
        let name_bytes = profile_os_str_storage_bytes(&file_name);
        if active_path_bytes.saturating_add(name_bytes)
            > PROFILE_FOOTPRINT_MAX_ACTIVE_PATH_BYTES
        {
            footprint.limit_reached = true;
            return Ok(());
        }
        // Keep only one child path at a time. A wide directory therefore
        // cannot turn the scanner itself into an unbounded PathBuf frontier.
        let child_path = directory.canonical_path.join(&file_name);
        let metadata = std::fs::symlink_metadata(&child_path)
            .map_err(|_| "inspect managed browser profile metadata".to_owned())?;
        footprint.entries = footprint.entries.saturating_add(1);
        footprint.bytes = footprint.bytes.saturating_add(metadata.len());
        if footprint.bytes >= stop_after_bytes || footprint.entries >= stop_after_entries {
            footprint.limit_reached = true;
            return Ok(());
        }
        let file_type = metadata.file_type();
        if file_type.is_dir()
            && !file_type.is_symlink()
            && !profile_metadata_is_reparse_point(&metadata)
        {
            if depth >= PROFILE_FOOTPRINT_MAX_DEPTH {
                footprint.limit_reached = true;
                return Ok(());
            }
            let child_identity =
                capture_profile_child_directory_identity(&child_path, exact_root)?;
            let child_path_bytes =
                profile_os_str_storage_bytes(child_identity.canonical_path.as_os_str());
            walk_profile_directory(
                &child_identity,
                exact_root,
                stop_after_bytes,
                stop_after_entries,
                depth.saturating_add(1),
                active_path_bytes.saturating_add(child_path_bytes),
                footprint,
            )?;
            if footprint.limit_reached {
                return Ok(());
            }
        }
    }
    let after = capture_profile_child_directory_identity(&directory.canonical_path, exact_root)?;
    if &after != directory {
        return Err("managed browser profile directory identity changed during scan".to_owned());
    }
    Ok(())
}

fn bounded_profile_footprint(
    exact_root: &ProfileDirectoryIdentity,
    stop_after_bytes: u64,
    stop_after_entries: u64,
) -> Result<BrowserProfileFootprint, String> {
    let before = capture_profile_directory_identity(&exact_root.canonical_path)?;
    if &before != exact_root {
        return Err("managed browser profile root identity changed before scan".to_owned());
    }
    let mut footprint = BrowserProfileFootprint::default();
    let root_path_bytes = profile_os_str_storage_bytes(exact_root.canonical_path.as_os_str());
    if root_path_bytes > PROFILE_FOOTPRINT_MAX_ACTIVE_PATH_BYTES {
        footprint.limit_reached = true;
    } else {
        walk_profile_directory(
            exact_root,
            exact_root,
            stop_after_bytes,
            stop_after_entries,
            0,
            root_path_bytes,
            &mut footprint,
        )?;
    }
    let after = capture_profile_directory_identity(&exact_root.canonical_path)?;
    if &after != exact_root {
        return Err("managed browser profile root identity changed during scan".to_owned());
    }
    Ok(footprint)
}

#[async_trait]
impl BrowserHostDriver for ManagedEngineHostDriver {
    fn host_id(&self) -> BrowserHostId {
        self.host_id.clone()
    }

    fn epoch(&self) -> u64 {
        self.epoch
    }

    fn state(&self) -> HostLifecycleState {
        self.lifecycle_state()
    }

    fn is_headful(&self) -> bool {
        // This is the effective engine launch mode, not a user preference.
        // The Hub uses it to distinguish a real native window from a Host
        // that must be replaced before an explicit foreground request.
        self.headful
    }

    fn process_id(&self) -> Option<u32> {
        self.host.process_id()
    }

    fn process_identity(&self) -> Option<nomifun_browser_platform::BrowserProcessIdentity> {
        self.host.process_identity().map(
            |(process_id, started_at_epoch_seconds, platform_start_key)| {
                nomifun_browser_platform::BrowserProcessIdentity {
                    process_id,
                    started_at_epoch_seconds,
                    platform_start_key,
                }
            },
        )
    }

    async fn profile_footprint(
        &self,
        stop_after_bytes: u64,
        stop_after_entries: u64,
    ) -> Result<Option<BrowserProfileFootprint>, BrowserPlatformError> {
        let Some(profile_root) = self.profile_root.clone() else {
            return Ok(None);
        };
        let ManagedProfileRoot::Captured(profile_root) = profile_root else {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The managed browser profile identity is unavailable.",
                true,
                "Retry after the exact Anonymous browser Host is retired.",
            ));
        };
        let sampler = Arc::clone(&self.profile_footprint_sampler);
        let (flight, start) = sampler.claim();
        if start {
            let worker_flight = Arc::clone(&flight);
            tokio::spawn(async move {
                let measured = tokio::task::spawn_blocking(move || {
                    bounded_profile_footprint(
                        &profile_root,
                        stop_after_bytes.max(1),
                        stop_after_entries.max(1),
                    )
                })
                .await
                .map_err(|_| profile_footprint_worker_error())
                .and_then(|result| result.map_err(|_| profile_footprint_measurement_error()));
                // Publish before dropping the worker. The sampler deliberately
                // keeps this completed flight until a caller consumes it.
                worker_flight.complete(measured);
            });
        }
        let measured = flight.wait().await;
        // There is no await between consuming the exact flight and returning
        // its result. If this caller was cancelled before observing completion,
        // the completed one-item mailbox remains for the next caller.
        sampler.consume(&flight);
        let measured = measured?;
        Ok(Some(measured))
    }

    async fn open_lane(
        &self,
        request: LaneLaunchRequest,
    ) -> Result<Arc<dyn BrowserLaneDriver>, BrowserPlatformError> {
        if self.lifecycle_state() != HostLifecycleState::Running {
            return Err(BrowserPlatformError::shutting_down());
        }
        if request.identity_mode != self.identity_mode {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The lane identity does not match its browser host.",
                false,
                "Open the lane on a host with the requested identity mode.",
            ));
        }

        // Exact-redaction values belong to this Lane/task, not the shared
        // Chromium Host.  Reusing the Host template would retain task A's
        // plaintext until every sibling task closed the Host and would turn
        // the registry's hard bound into a global concurrency cap.
        let known_secret_values = new_lane_known_secret_values();
        let config = LaneEngineConfig {
            workspace_dir: request
                .workspace_hint
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| self.defaults.workspace_dir.clone()),
            evaluate_full_power: self.defaults.evaluate_full_power,
            evaluate_persistent_login: self.defaults.evaluate_persistent_login,
            known_secret_values: Some(known_secret_values.clone()),
            task_resource_key: Some(request.task_resource_key),
            max_task_tabs: request.max_task_tabs,
            task_tab_reservation_authority: Some(Arc::new(EngineTaskTabAuthorityBridge {
                inner: request.task_tab_authority,
            })),
            task_download_reservation_authority: Some(Arc::new(
                EngineTaskDownloadAuthorityBridge {
                    inner: request.task_download_authority,
                },
            )),
        };
        let engine = self
            .host
            .open_lane(request.lane_id.to_string(), config.clone())
            .await
            .map_err(map_engine_error)?;
        let policy = BrowserTool::with_managed_engine(
            engine.clone(),
            self.data_dir.clone(),
            config.workspace_dir,
            self.headful,
            config.evaluate_full_power,
            config.evaluate_persistent_login,
            known_secret_values,
        );
        let policy = (self.lane_policy)(policy);
        Ok(Arc::new(ManagedEngineLaneDriver {
            lane_id: request.lane_id,
            epoch: self.epoch,
            engine,
            policy,
            host: Arc::downgrade(&self.host),
            identity_mode: self.identity_mode,
            identity_snapshot_persister: self.identity_snapshot_persister.clone(),
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            close_gate: AsyncMutex::new(()),
        }))
    }

    async fn reconcile_task_tab_limit(
        &self,
        task_resource_key: &str,
        max_task_tabs: usize,
    ) -> Result<(), BrowserPlatformError> {
        if self.lifecycle_state() != HostLifecycleState::Running {
            return Err(BrowserPlatformError::shutting_down());
        }
        self.host
            .reconcile_task_tab_limit(task_resource_key, max_task_tabs)
            .await
            .map_err(map_engine_error)
    }

    async fn shutdown(&self) -> Result<(), BrowserPlatformError> {
        let _shutdown_guard = self.shutdown_gate.lock().await;
        if self.state.load(Ordering::Acquire) == HostState::Stopped as u8 {
            return Ok(());
        }
        // A previous explicit shutdown failure is retryable. Keep the wrapper
        // authoritative and transition Failed -> Stopping for this attempt.
        self.state
            .store(HostState::Stopping as u8, Ordering::Release);
        match self.host.shutdown().await {
            Ok(()) => {
                self.state
                    .store(HostState::Stopped as u8, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.state
                    .store(HostState::Failed as u8, Ordering::Release);
                Err(map_engine_error(error))
            }
        }
    }
}

/// Per-lane engine adapter. The Hub owns serialization; the engine also keeps
/// its own lane-local correctness gate as defense in depth.
pub struct ManagedEngineLaneDriver {
    lane_id: BrowserLaneId,
    epoch: u64,
    engine: Arc<dyn BrowserEngine>,
    policy: BrowserTool,
    host: Weak<ManagedBrowserHost>,
    identity_mode: BrowserIdentityMode,
    identity_snapshot_persister: Option<IdentitySnapshotPersister>,
    closing: AtomicBool,
    closed: AtomicBool,
    close_gate: AsyncMutex<()>,
}

impl ManagedEngineLaneDriver {
    async fn execute_inner(
        &self,
        operation: BrowserOperation,
        context: &DriverOperationContext,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(lane_closed_error());
        }
        if context.operation.browser_epoch != self.epoch {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::StaleBrowserEpoch,
                "The browser host restarted after this operation was prepared.",
                true,
                "Refresh the lane and retry with its current browser epoch.",
            ));
        }
        if context.operation.lane_id != self.lane_id {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The operation was issued for a different browser lane.",
                false,
                "Use the lane handle issued for this operation.",
            ));
        }

        let action = operation.action.as_str();
        authorize_action_shape(operation.kind, action)?;
        // `OperationContext.target_id` is the full CDP target id. Platform
        // snapshots expose a distinct short `tab_id`, so never echo the target
        // id into `active_tab_id`; the structured inventory below supplies the
        // authoritative mapping.
        let active_tab_id: Option<String> = None;
        let refresh_tab_inventory = true;
        let active_frame_follows_active_tab = matches!(action, "switch_tab")
            || (action == "switch_frame"
                && operation
                    .input
                    .get("ref")
                    .and_then(Value::as_str)
                    .is_some_and(|reference| {
                        reference.trim().eq_ignore_ascii_case("main")
                            || reference.trim().eq_ignore_ascii_case("top")
                    }));

        let result = match (operation.kind, action) {
            (BrowserOperationKind::Navigate, "navigate")
            | (BrowserOperationKind::Crawl, "navigate") => {
                self.select_target_if_requested(&operation, context).await?;
                let url = required_string(&operation.input, "url", "navigate")?;
                if url.len() > MAX_MANAGED_NAVIGATION_URL_BYTES {
                    return Err(navigation_url_capacity_error());
                }
                let new_tab = operation
                    .input
                    .get("new_tab")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let nav = self
                    .engine
                    .navigate(url, new_tab)
                    .await
                    .map_err(map_engine_error)?;
                let final_url_slice = nomi_browser_engine::actions::utf8_prefix_at_most(
                    &nav.final_url,
                    MAX_MANAGED_NAVIGATION_URL_BYTES,
                );
                let final_url_truncated = final_url_slice.len() < nav.final_url.len();
                let final_url = final_url_slice.to_owned();
                Ok(BrowserOperationResult {
                    output: json!({
                        "final_url": final_url,
                        "final_url_truncated": final_url_truncated,
                        "http_status": nav.http_status,
                        "redirected": nav.redirected,
                        "load_state": nav.load_state.to_string(),
                    }),
                    active_tab_id,
                    ..Default::default()
                })
            }
            (BrowserOperationKind::Navigate, "back") => {
                self.execute_act("back", &operation.input, context, active_tab_id)
                    .await
            }
            (BrowserOperationKind::Navigate, "forward") => {
                self.execute_act("forward", &operation.input, context, active_tab_id)
                    .await
            }
            (BrowserOperationKind::Navigate, "reload") => {
                self.execute_act("reload", &operation.input, context, active_tab_id)
                    .await
            }
            (BrowserOperationKind::Observe, "observe")
            | (BrowserOperationKind::Crawl, "observe") => {
                self.select_target_if_requested(&operation, context).await?;
                let options = observe_options(&operation.input);
                let observation = self.observe_with_generation_fence(&options, context).await?;
                let generation = observation.generation.0;
                let output = match serialize_observation(&observation) {
                    Ok(output) => output,
                    Err(error) => {
                        self.policy.invalidate_cached_observation();
                        return Err(error);
                    }
                };
                self.policy
                    .cache_managed_observation(observation)
                    .map_err(|_| observation_capacity_error())?;
                Ok(BrowserOperationResult {
                    output,
                    active_tab_id,
                    ref_generation: Some(generation),
                    ..Default::default()
                })
            }
            (BrowserOperationKind::Screenshot, "screenshot") => {
                self.select_target_if_requested(&operation, context).await?;
                let png = self
                    .engine
                    .screenshot()
                    .await
                    .map_err(map_engine_error)?;
                Ok(BrowserOperationResult {
                    output: json!({
                        "media_type": "image/png",
                        "data": base64::engine::general_purpose::STANDARD.encode(png),
                    }),
                    active_tab_id,
                    ..Default::default()
                })
            }
            (BrowserOperationKind::Debug, "rendered_html")
            | (BrowserOperationKind::Crawl, "rendered_html") => {
                self.select_target_if_requested(&operation, context).await?;
                let html = self
                    .engine
                    .rendered_html()
                    .await
                    .map_err(map_engine_error)?;
                let (html, html_truncated) = bound_rendered_html(html);
                Ok(BrowserOperationResult {
                    output: json!({
                        "html": html,
                        "html_truncated": html_truncated,
                    }),
                    active_tab_id,
                    ..Default::default()
                })
            }
            (BrowserOperationKind::Crawl, "get_page_text")
            | (BrowserOperationKind::Crawl, "extract") => {
                self.select_target_if_requested(&operation, context).await?;
                self.execute_act(action, &operation.input, context, active_tab_id)
                    .await
            }
            (BrowserOperationKind::Manage, "capabilities") => {
                let caps = self.engine.capabilities();
                Ok(BrowserOperationResult {
                    output: json!({
                        "browser_ready": caps.browser_ready,
                        "headful": caps.headful,
                        "display_available": caps.display_available,
                        "engine": caps.engine,
                    }),
                    active_tab_id,
                    ..Default::default()
                })
            }
            (BrowserOperationKind::Manage, "device_pixel_ratio") => {
                let dpr = self
                    .engine
                    .device_pixel_ratio()
                    .await
                    .map_err(map_engine_error)?;
                Ok(BrowserOperationResult {
                    output: json!({ "device_pixel_ratio": dpr }),
                    active_tab_id,
                    ..Default::default()
                })
            }
            (
                BrowserOperationKind::Act
                | BrowserOperationKind::Observe
                | BrowserOperationKind::Tabs
                | BrowserOperationKind::Download
                | BrowserOperationKind::Debug,
                _,
            ) => {
                self.select_target_if_requested(&operation, context).await?;
                self.execute_act(action, &operation.input, context, active_tab_id)
                    .await
            }
            _ => Err(invalid_operation(
                "This action is not available for the requested operation kind.",
            )),
        }?;
        if refresh_tab_inventory {
            self.attach_tab_inventory(result, active_frame_follows_active_tab)
                .await
        } else {
            Ok(result)
        }
    }

    async fn observe_with_generation_fence(
        &self,
        options: &ObserveOpts,
        context: &DriverOperationContext,
    ) -> Result<Observation, BrowserPlatformError> {
        // The Hub advances `ref_generation` before rebinding a restarted Host.
        // A fresh engine starts its local RefTable at generation 1, so consume
        // reset generations until the real observation reaches that Hub fence.
        // Never synthesize or rewrite the generation: the value returned to
        // the Hub remains exactly `Observation::generation`.
        //
        // F54: always attempt the first observe before judging the fence — the
        // bound below caps the catch-up DELTA, never the absolute canonical
        // generation, so a long-lived in-sync lane keeps observing forever.
        let required = context.operation.ref_generation.max(1);
        let observation = self
            .engine
            .observe(options)
            .await
            .map_err(map_engine_error)?;
        if observation.generation.0 >= required {
            return Ok(observation);
        }

        // F22: keep the catch-up burst small. Beyond the bound, fail with the
        // retryable exhaustion error instead of storming full observations for
        // minutes on one serialized operation; the generations consumed here
        // are kept by the engine, so each retry resumes closer to the fence.
        let catch_up = (required - observation.generation.0)
            .min(MAX_OBSERVATION_GENERATION_CATCH_UP);
        for _ in 0..catch_up {
            let observation = self
                .engine
                .observe(options)
                .await
                .map_err(map_engine_error)?;
            if observation.generation.0 >= required {
                return Ok(observation);
            }
        }

        Err(observation_generation_exhausted())
    }

    async fn attach_tab_inventory(
        &self,
        mut result: BrowserOperationResult,
        active_frame_follows_active_tab: bool,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let tabs = match self.engine.tabs().await {
            Ok(tabs) => tabs,
            // Keep test/fallback engines source-compatible. The production CDP
            // backend implements this seam and never relies on parsing LLM text.
            Err(BrowserError::Unsupported { .. }) => return Ok(result),
            Err(error) => return Err(map_engine_error(error)),
        };
        let active_tab = tabs.iter().find(|tab| tab.active);
        result.active_tab_id = active_tab.map(|tab| tab.tab_id.clone());
        if active_frame_follows_active_tab {
            // A top-level tab switch (or an explicit switch back to main/top)
            // makes the selected tab's full target id the authoritative main
            // frame id. Never expose the short tab id in the frame field.
            result.active_frame_id = active_tab.map(|tab| tab.target_id.clone());
        }
        result.tabs = tabs.into_iter().map(serialize_tab).collect();
        Ok(result)
    }

    async fn select_target_if_requested(
        &self,
        operation: &BrowserOperation,
        context: &DriverOperationContext,
    ) -> Result<(), BrowserPlatformError> {
        let Some(target_id) = operation.target_id.as_deref() else {
            return Ok(());
        };
        if matches!(operation.action.as_str(), "switch_tab" | "close_tab") {
            return Ok(());
        }
        let progress = operation_progress(&operation.input, context);
        self.engine
            .act(
                &ActSpec::SwitchTab {
                    tab_id: target_id.to_string(),
                },
                &progress,
            )
            .await
            .map_err(map_engine_error)?;
        Ok(())
    }

    async fn execute_act(
        &self,
        action: &str,
        input: &Value,
        context: &DriverOperationContext,
        active_tab_id: Option<String>,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        let spec = self
            .policy
            .prepare_managed_act(action, input)
            .await
            .map_err(|_rejection| {
                BrowserPlatformError::new(
                    BrowserErrorCode::OperationNotAllowed,
                    "The browser action was rejected by the managed browser policy.",
                    false,
                    "Review the action parameters or request explicit user control.",
                )
            })?;
        let progress = operation_progress(input, context);
        let result = self
            .engine
            .act(&spec, &progress)
            .await
            .map_err(map_engine_error)?;
        if matches!(action, "get_page_text" | "extract")
            && (result.message.len()
                > nomi_browser_engine::actions::MAX_PAGE_READ_RESULT_BYTES
                || result.message.capacity()
                    > nomi_browser_engine::actions::MAX_PAGE_READ_RESULT_BYTES)
        {
            return Err(page_read_capacity_error());
        }
        let active_frame_id = active_frame_id_from_act_result(
            &spec,
            &result,
            context.operation.target_id.as_deref(),
        );
        // The structured tab inventory is the only authoritative short-id/full
        // target mapping. In particular, do not echo a full SwitchTab target
        // into `active_tab_id`. Tab-switch generation invalidation remains Hub
        // owned; this action intentionally emits no adapter-local ref fence.
        Ok(serialize_act_result(
            result,
            active_tab_id,
            active_frame_id,
        ))
    }

}

#[async_trait]
impl BrowserLaneDriver for ManagedEngineLaneDriver {
    async fn execute(
        &self,
        operation: BrowserOperation,
        context: DriverOperationContext,
    ) -> Result<BrowserOperationResult, BrowserPlatformError> {
        tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => Err(lane_closed_error()),
            result = self.execute_inner(operation, &context) => result,
        }
    }

    async fn close(&self) -> Result<(), BrowserPlatformError> {
        let _close_guard = self.close_gate.lock().await;
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        // Fence new adapter work before entering engine cleanup. This remains
        // sticky if cleanup fails so retries keep cleanup authority without
        // allowing operations back into a half-closed Lane.
        self.closing.store(true, Ordering::Release);
        if let Some(host) = self.host.upgrade() {
            host.close_lane(self.lane_id.as_str())
                .await
                .map_err(map_engine_error)?;
        }
        self.closed.store(true, Ordering::Release);
        Ok(())
    }

    async fn bring_to_front(&self) -> Result<(), BrowserPlatformError> {
        // This trait method is the platform's trusted process-internal seam. It
        // deliberately bypasses the model-visible JSON operation dispatcher,
        // while retaining the same close fence and safe error mapping.
        if self.closing.load(Ordering::Acquire) {
            return Err(lane_closed_error());
        }
        self.engine
            .bring_to_front()
            .await
            .map_err(map_engine_error)
    }

    async fn freeze(&self) -> Result<LaneFreezeOutcome, BrowserPlatformError> {
        // The managed engine currently has no paired Page lifecycle
        // freeze/resume contract, and the platform cannot transition a Frozen
        // Lane back to Running. Be explicit so resource pressure closes and
        // recreates this idle Lane instead of retaining a one-way fake freeze.
        Ok(LaneFreezeOutcome::Unsupported)
    }

    async fn capture_identity_snapshot(
        &self,
    ) -> Result<Option<CapturedIdentitySnapshot>, BrowserPlatformError> {
        if self.identity_mode != BrowserIdentityMode::Primary {
            return Ok(None);
        }
        let state = self
            .engine
            .capture_cookie_state()
            .await
            .map_err(map_engine_error)?;
        state.validate_bounds().map_err(|_| identity_capture_error())?;
        // Managed Browser Use has a strict cookie-only identity contract. This avoids running
        // page scripts after every successful operation and makes cancellation non-amplifying.
        let coverage = SnapshotCoverage::cookies_only();
        let payload = state.to_json().map_err(|_| identity_capture_error())?;
        if let Some(persist) = &self.identity_snapshot_persister {
            persist(&payload)?;
        }
        Ok(Some(CapturedIdentitySnapshot {
            payload: IdentitySnapshotPayload::from_json(payload),
            coverage,
        }))
    }
}

fn derive_host_config(
    template: &EngineConfig,
    profiles_root: &Path,
    request: &HostLaunchRequest,
) -> Result<EngineConfig, BrowserPlatformError> {
    let mut config = template.clone();
    let host_component = format!("host-{}", request.host_id.as_str());
    let (profile, ephemeral) = match request.identity_mode {
        BrowserIdentityMode::Primary => (
            profiles_root
                .join("primary")
                .join(format!("generation-{}", request.identity_generation)),
            false,
        ),
        BrowserIdentityMode::Anonymous => (
            profiles_root.join("anonymous").join(host_component),
            true,
        ),
        BrowserIdentityMode::AuthenticatedReplica => (
            profiles_root
                .join("replica")
                .join(format!("generation-{}", request.identity_generation))
                .join(host_component),
            true,
        ),
        BrowserIdentityMode::Isolated => (
            profiles_root.join("isolated").join(host_component),
            true,
        ),
    };
    config.user_data_dir = Some(profile);
    config.ephemeral_profile = ephemeral;
    config.headful = request.headful;
    // A host serves multiple lanes, so downloads are attributed at lane-open.
    config.workspace_dir = None;

    match request.identity_mode {
        BrowserIdentityMode::Primary => {}
        BrowserIdentityMode::AuthenticatedReplica => {
            // Replica payload is resolved by the Hub for this exact canonical
            // generation. Never fall back to the process-start template.
            let payload = request
                .identity_snapshot_payload
                .as_ref()
                .ok_or_else(|| {
                    BrowserPlatformError::new(
                        BrowserErrorCode::NeedsPrimaryIdentity,
                        "The authenticated browser identity payload is unavailable.",
                        true,
                        "Capture and publish the Primary browser identity again.",
                    )
                })?;
            let cookie_only = nomi_browser_engine::StorageState::from_json(
                payload.as_json().clone(),
            )
            .map_err(|_| identity_capture_error())?
            .into_cookie_only()
            .to_json()
            .map_err(|_| identity_capture_error())?;
            config.storage_state = Some(cookie_only);
            config.evaluate_persistent_login = false;
        }
        BrowserIdentityMode::Anonymous | BrowserIdentityMode::Isolated => {
            config.storage_state = None;
            config.evaluate_persistent_login = false;
            config.known_secret_values = nomi_browser_engine::KnownSecretValues::default();
        }
    }
    Ok(config)
}

fn identity_capture_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The Primary browser identity could not be captured safely.",
        true,
        "Retry after navigating the Primary browser to the signed-in site.",
    )
}

fn authorize_action_shape(
    kind: BrowserOperationKind,
    action: &str,
) -> Result<(), BrowserPlatformError> {
    let allowed = match kind {
        BrowserOperationKind::Navigate => matches!(
            action,
            "navigate" | "back" | "forward" | "reload"
        ),
        BrowserOperationKind::Observe => matches!(
            action,
            "observe"
                | "get_page_text"
                | "search_page"
                | "find_elements"
                | "get_dropdown_options"
                | "cursor"
        ),
        BrowserOperationKind::Screenshot => matches!(action, "screenshot"),
        BrowserOperationKind::Tabs => matches!(
            action,
            "tabs" | "switch_tab" | "close_tab" | "open_link_new_tab"
        ),
        BrowserOperationKind::Download => matches!(action, "download" | "save_as_pdf"),
        BrowserOperationKind::Debug => matches!(
            action,
            "get_console_logs"
                | "get_page_errors"
                | "get_network_log"
                | "rendered_html"
                | "evaluate"
        ),
        BrowserOperationKind::Manage => matches!(action, "capabilities" | "device_pixel_ratio"),
        BrowserOperationKind::Crawl => matches!(
            action,
            "navigate" | "observe" | "get_page_text" | "extract" | "rendered_html"
        ),
        BrowserOperationKind::Act => {
            !matches!(
                action,
                "" | "navigate"
                    | "observe"
                    | "screenshot"
                    | "capabilities"
                    | "tabs"
                    | "download"
                    | "save_as_pdf"
                    | "get_console_logs"
                    | "get_page_errors"
                    | "get_network_log"
                    | "rendered_html"
            )
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(invalid_operation(
            "This action does not match the authorized browser operation kind.",
        ))
    }
}

fn operation_progress(
    input: &Value,
    context: &DriverOperationContext,
) -> nomi_browser_engine::progress::Progress {
    let requested = input
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ACTION_TIMEOUT);
    let timeout = requested.min(MAX_ACTION_TIMEOUT);
    nomi_browser_engine::progress::Progress::child(timeout, &context.cancellation)
}

fn observe_options(input: &Value) -> ObserveOpts {
    let mut options = ObserveOpts::default();
    if let Some(depth) = input.get("max_depth").and_then(Value::as_u64) {
        options.max_depth = depth.min(u32::MAX as u64) as u32;
    }
    if let Some(diff) = input.get("diff").and_then(Value::as_bool) {
        options.diff = diff;
    }
    if let Some(include_screenshot) = input.get("include_screenshot").and_then(Value::as_bool) {
        options.include_screenshot = include_screenshot;
    }
    if let Some(include_boxes) = input.get("include_boxes").and_then(Value::as_bool) {
        options.include_boxes = include_boxes;
    }
    options
}

struct ObservationJson<'a>(&'a Observation);

struct ObservationEntriesJson<'a>(&'a [nomi_browser_engine::ElementEntry]);

struct ObservationEntryJson<'a>(&'a nomi_browser_engine::ElementEntry);

struct ObservationBoxesJson<'a>(
    &'a std::collections::HashMap<String, nomi_browser_engine::CssRect>,
);

struct ObservationRectJson(nomi_browser_engine::CssRect);

impl Serialize for ObservationEntryJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ObservationEntry", 4)?;
        state.serialize_field("ref", &self.0.r#ref)?;
        state.serialize_field("role", &self.0.role)?;
        state.serialize_field("name", &self.0.name)?;
        state.serialize_field("frame_seq", &self.0.frame_seq)?;
        state.end()
    }
}

impl Serialize for ObservationEntriesJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            sequence.serialize_element(&ObservationEntryJson(entry))?;
        }
        sequence.end()
    }
}

impl Serialize for ObservationRectJson {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ObservationRect", 4)?;
        state.serialize_field("x", &self.0.x)?;
        state.serialize_field("y", &self.0.y)?;
        state.serialize_field("width", &self.0.width)?;
        state.serialize_field("height", &self.0.height)?;
        state.end()
    }
}

impl Serialize for ObservationBoxesJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (reference, rect) in self.0 {
            map.serialize_entry(reference, &ObservationRectJson(*rect))?;
        }
        map.end()
    }
}

impl Serialize for ObservationJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let observation = self.0;
        let mut state = serializer.serialize_struct("Observation", 7)?;
        state.serialize_field("generation", &observation.generation.0)?;
        state.serialize_field("yaml", &observation.yaml)?;
        state.serialize_field(
            "entries",
            &ObservationEntriesJson(observation.entries.as_slice()),
        )?;
        state.serialize_field("url", &observation.url)?;
        state.serialize_field("truncated", &observation.truncated)?;
        state.serialize_field("current_page_is_post", &observation.current_page_is_post)?;
        state.serialize_field("boxes", &ObservationBoxesJson(&observation.boxes))?;
        state.end()
    }
}

fn observation_capacity_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "The browser observation exceeded the per-task byte limit.",
        false,
        "Simplify the page or reduce observe depth, then run a fresh observe.",
    )
}

fn page_read_capacity_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "The browser page read exceeded the per-operation retained-byte limit.",
        false,
        "Request a smaller extraction or split the page into narrower reads.",
    )
}

fn navigation_url_capacity_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        "The browser navigation URL exceeded the per-operation byte limit.",
        false,
        "Use a shorter HTTP(S) URL.",
    )
}

fn bound_rendered_html(mut html: String) -> (String, bool) {
    let marker = nomi_browser_engine::RENDERED_HTML_TRUNCATION_MARKER;
    if html.len() <= nomi_browser_engine::MAX_RENDERED_HTML_BYTES {
        let truncated = html.ends_with(marker);
        return (html, truncated);
    }

    let content_limit = nomi_browser_engine::MAX_RENDERED_HTML_BYTES.saturating_sub(marker.len());
    let prefix = nomi_browser_engine::actions::utf8_prefix_at_most(&html, content_limit).len();
    html.truncate(prefix);
    html.push_str(marker);
    (html, true)
}

fn serialize_observation(
    observation: &Observation,
) -> Result<Value, BrowserPlatformError> {
    observation
        .validate_retained_bytes()
        .map_err(|_| observation_capacity_error())?;
    // Count the exact escaped UTF-8 JSON without allocating a serialized copy.
    // Only materialize the Value after the output is known to fit.
    nomi_browser_engine::observe::serialized_json_bytes_bounded(&ObservationJson(observation))
        .map_err(|_| observation_capacity_error())?;
    let entries = observation
        .entries
        .iter()
        .map(|entry| {
            json!({
                "ref": entry.r#ref,
                "role": entry.role,
                "name": entry.name,
                "frame_seq": entry.frame_seq,
            })
        })
        .collect::<Vec<_>>();
    let boxes = observation
        .boxes
        .iter()
        .map(|(reference, rect)| {
            (
                reference.clone(),
                json!({
                    "x": rect.x,
                    "y": rect.y,
                    "width": rect.width,
                    "height": rect.height,
                }),
            )
        })
        .collect::<Map<String, Value>>();
    Ok(json!({
        "generation": observation.generation.0,
        "yaml": observation.yaml,
        "entries": entries,
        "url": observation.url,
        "truncated": observation.truncated,
        "current_page_is_post": observation.current_page_is_post,
        "boxes": boxes,
    }))
}

fn serialize_tab(tab: BrowserTabInfo) -> BrowserTabSnapshot {
    BrowserTabSnapshot {
        tab_id: tab.tab_id,
        target_id: tab.target_id,
        title: tab.title,
        url: tab.url,
        active: tab.active,
        crashed: tab.crashed,
    }
}

fn serialize_act_result(
    result: ActResult,
    active_tab_id: Option<String>,
    active_frame_id: Option<String>,
) -> BrowserOperationResult {
    BrowserOperationResult {
        output: json!({
            "success": result.success,
            "message": result.message,
            "effect": {
                "changed": result.effect.changed,
                "before_anchor": result.effect.before_anchor,
                "after_anchor": result.effect.after_anchor,
            },
        }),
        active_tab_id,
        active_frame_id,
        ..Default::default()
    }
}

fn active_frame_id_from_act_result(
    spec: &ActSpec,
    result: &ActResult,
    main_target_id: Option<&str>,
) -> Option<String> {
    if !result.success || !matches!(spec, ActSpec::SwitchFrame { .. }) {
        return None;
    }
    let frame_id = result
        .effect
        .after_anchor
        .as_ref()?
        .get("active_frame")?
        .as_str()?;
    if frame_id.eq_ignore_ascii_case("main") || frame_id.eq_ignore_ascii_case("top") {
        main_target_id.map(str::to_owned)
    } else {
        Some(frame_id.to_owned())
    }
}

fn required_string<'a>(
    input: &'a Value,
    field: &str,
    action: &str,
) -> Result<&'a str, BrowserPlatformError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| invalid_operation(&format!("{action} requires `{field}`.")))
}

fn invalid_operation(message: &str) -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::OperationNotAllowed,
        message,
        false,
        "Correct the browser operation and retry.",
    )
}

fn lane_closed_error() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::LaneClosedByUser,
        "The browser lane was closed before the operation completed.",
        false,
        "Open a new lane before retrying.",
    )
}

fn observation_generation_exhausted() -> BrowserPlatformError {
    BrowserPlatformError::new(
        BrowserErrorCode::BrowserUnavailable,
        "The managed browser could not establish a fresh observation generation.",
        true,
        "Retry once; if it persists, open a new browser lane.",
    )
}

fn map_engine_error(error: BrowserError) -> BrowserPlatformError {
    match error {
        BrowserError::Unsupported { .. } => BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            "This browser capability is not available in the current engine.",
            false,
            "Use a supported browser action or change the browser configuration.",
        ),
        BrowserError::SessionLost { recoverable } => {
            let error = BrowserPlatformError::new(
                BrowserErrorCode::BrowserRestarted,
                "The managed browser connection was lost.",
                recoverable,
                if recoverable {
                    "Refresh the lane after the browser restarts, then retry."
                } else {
                    "Open a new browser lane."
                },
            );
            if recoverable {
                error
            } else {
                // Safe, machine-readable scope used by the Hub. Generic
                // BrowserUnavailable/timeout errors must never relaunch a Host.
                error.with_metadata(json!({ "failure_scope": "host" }))
            }
        }
        BrowserError::Blocked { .. } => BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            "The browser security policy blocked this operation.",
            false,
            "Change the operation or request explicit user control.",
        ),
        BrowserError::NodeStale { .. }
        | BrowserError::NotConnected
        | BrowserError::Detached { .. }
        | BrowserError::TargetClosed => BrowserPlatformError::new(
            BrowserErrorCode::StaleLaneRef,
            "The browser target or element reference is no longer current.",
            true,
            "Observe the lane again and retry with a fresh reference.",
        ),
        BrowserError::TargetCrashed => BrowserPlatformError::new(
            BrowserErrorCode::TargetCrashed,
            "The active browser target crashed.",
            true,
            "Open or select another tab, then retry.",
        ),
        BrowserError::Timeout { .. } => BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "The browser operation timed out.",
            true,
            "Retry the operation or use a shorter, more specific action.",
        ),
        BrowserError::NavigationInterrupted | BrowserError::NavFailed { .. } => {
            BrowserPlatformError::new(
                BrowserErrorCode::BrowserUnavailable,
                "The browser navigation did not complete.",
                true,
                "Observe the current page and retry the navigation if needed.",
            )
        }
        // Never surface `Other` text. It can contain CDP endpoints, profile
        // paths, transport diagnostics, URLs or page-controlled text.
        BrowserError::Other(_) => BrowserPlatformError::new(
            BrowserErrorCode::BrowserUnavailable,
            "The managed browser operation failed.",
            true,
            "Retry once; if it persists, open a new browser lane.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use image::ImageFormat;
    use nomi_browser_engine::{
        Capabilities, DebugSnapshot, Effect, ElementEntry, IndexedDbDump, LoadState, NavResult,
        OriginStorage, SnapshotGen, StorageState,
    };
    use nomifun_browser_platform::OperationContext;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[test]
    fn profile_footprint_is_bounded_and_treats_equality_as_limit() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = temporary.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        std::fs::write(profile.join("cache.bin"), [7_u8; 10]).unwrap();
        let identity = capture_profile_directory_identity(&profile).unwrap();

        let measured = bounded_profile_footprint(&identity, 10, 100).unwrap();

        assert_eq!(measured.bytes, 10);
        assert_eq!(measured.entries, 1);
        assert!(measured.limit_reached);
    }

    #[test]
    fn profile_footprint_rejects_replaced_root_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = temporary.path().join("profile");
        let moved = temporary.path().join("profile-old");
        std::fs::create_dir(&profile).unwrap();
        let identity = capture_profile_directory_identity(&profile).unwrap();
        std::fs::rename(&profile, &moved).unwrap();
        std::fs::create_dir(&profile).unwrap();

        assert!(bounded_profile_footprint(&identity, 100, 100).is_err());
    }

    #[test]
    fn platform_rejects_oversized_observation_before_value_materialization() {
        let observation = Observation {
            generation: SnapshotGen(1),
            yaml: "x".repeat(
                nomi_browser_engine::observe::MAX_OBSERVATION_RETAINED_BYTES + 1,
            ),
            entries: vec![],
            url: None,
            truncated: false,
            current_page_is_post: false,
            boxes: HashMap::new(),
        };
        let error = serialize_observation(&observation)
            .expect_err("platform must not materialize an oversized output Value");
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(!error.message.contains(&"x".repeat(64)));
    }

    #[test]
    fn platform_rejects_json_escape_expansion_before_value_materialization() {
        let observation = Observation {
            generation: SnapshotGen(1),
            // Retained bytes fit, but JSON encodes every NUL as six bytes.
            yaml: "\0".repeat(
                nomi_browser_engine::observe::MAX_OBSERVATION_RETAINED_BYTES / 5,
            ),
            entries: vec![],
            url: None,
            truncated: false,
            current_page_is_post: false,
            boxes: HashMap::new(),
        };
        assert!(observation.validate_retained_bytes().is_ok());
        assert!(serialize_observation(&observation).is_err());
    }

    #[derive(Debug)]
    struct TestExactLaunchAuthority;

    struct TestDurableLaunchGuard {
        authority: Option<Arc<TestExactLaunchAuthority>>,
        cleanup_lease: Option<nomi_browser_engine::HostCleanupLease>,
        reaper: tokio::sync::mpsc::UnboundedSender<TestReapedLaunch>,
    }

    struct TestReapedLaunch {
        authority: Arc<TestExactLaunchAuthority>,
        _cleanup_lease: nomi_browser_engine::HostCleanupLease,
    }

    impl Drop for TestDurableLaunchGuard {
        fn drop(&mut self) {
            if let (Some(authority), Some(cleanup_lease)) =
                (self.authority.take(), self.cleanup_lease.take())
            {
                let _ = self.reaper.send(TestReapedLaunch {
                    authority,
                    _cleanup_lease: cleanup_lease,
                });
            }
        }
    }

    #[tokio::test]
    async fn managed_factory_cancellation_before_driver_publication_retains_exact_authority() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = EngineConfig::default();
        config.data_dir = temp.path().join("data");
        config.user_data_dir = Some(temp.path().join("profile"));
        config.ephemeral_profile = true;
        let mut factory = ManagedEngineHostFactory::from_config_resolver(Arc::new(move |_| {
            Ok(config.clone())
        }));

        let (created_tx, mut created_rx) = tokio::sync::mpsc::unbounded_channel();
        let (reaper_tx, mut reaper_rx) = tokio::sync::mpsc::unbounded_channel();
        factory.host_launcher = Arc::new(move |_config, cleanup_lease| {
            let created_tx = created_tx.clone();
            let reaper_tx = reaper_tx.clone();
            Box::pin(async move {
                // This models the production engine's pre-publication launch
                // guards. The companion engine tests
                // `abort_before_marker_commit_keeps_process_and_ephemeral_cleanup_indivisible`
                // and `abort_after_marker_commit_retains_exact_cleanup_authority`
                // exercise the same handoff with real managed child processes.
                let authority = Arc::new(TestExactLaunchAuthority);
                let weak = Arc::downgrade(&authority);
                created_tx.send(weak).unwrap();
                let _guard = TestDurableLaunchGuard {
                    authority: Some(authority),
                    cleanup_lease: Some(cleanup_lease),
                    reaper: reaper_tx,
                };
                std::future::pending::<Result<ManagedBrowserHost, BrowserError>>().await
            })
        });

        let (cleanup_ticket, cleanup_lease) = HostLaunchCleanupTicket::new();
        let launching = tokio::spawn(async move {
            factory
                .launch(HostLaunchRequest {
                    host_id: BrowserHostId::parse("host-cancel-before-publication").unwrap(),
                    browser_epoch: 1,
                    identity_mode: BrowserIdentityMode::Isolated,
                    identity_generation: 0,
                    identity_snapshot_payload: None,
                    headful: false,
                    cleanup_lease,
                })
                .await
        });
        let exact = tokio::time::timeout(Duration::from_secs(2), created_rx.recv())
            .await
            .expect("injected production launch boundary was not entered")
            .expect("launch authority sender disappeared");
        assert!(
            exact.upgrade().is_some(),
            "pre-publication launch future must own exact cleanup authority"
        );

        // This is the state observed by Hub when its factory timeout drops the
        // future: no BrowserHostDriver was returned, but the engine-side guard
        // must transfer authority to a durable reaper rather than destroy it.
        launching.abort();
        let join_result = launching.await;
        assert!(matches!(join_result, Err(error) if error.is_cancelled()));
        let retained = tokio::time::timeout(Duration::from_secs(2), reaper_rx.recv())
            .await
            .expect("cancelled launch did not publish durable cleanup authority")
            .expect("durable cleanup receiver disappeared");
        assert!(exact.ptr_eq(&Arc::downgrade(&retained.authority)));
        assert!(
            !cleanup_ticket.is_complete(),
            "the cancelled factory's durable reaper must retain the Hub lease"
        );
        drop(retained);
        assert!(
            exact.upgrade().is_none(),
            "the durable receiver was the final exact cleanup authority"
        );
        cleanup_ticket.wait().await;
        assert!(cleanup_ticket.is_complete());
    }

    #[tokio::test]
    async fn managed_factory_error_releases_platform_cleanup_lease_after_engine_future_settles() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = EngineConfig::default();
        config.data_dir = temp.path().join("data");
        config.user_data_dir = Some(temp.path().join("profile"));
        let mut factory = ManagedEngineHostFactory::from_config_resolver(Arc::new(move |_| {
            Ok(config.clone())
        }));
        factory.host_launcher = Arc::new(move |_config, cleanup_lease| {
            Box::pin(async move {
                let _cleanup_lease = cleanup_lease;
                Err(BrowserError::Other("synthetic managed launch failure".into()))
            })
        });

        let (ticket, cleanup_lease) = HostLaunchCleanupTicket::new();
        let result = factory
            .launch(HostLaunchRequest {
                host_id: BrowserHostId::parse("host-factory-error").unwrap(),
                browser_epoch: 1,
                identity_mode: BrowserIdentityMode::Isolated,
                identity_generation: 0,
                identity_snapshot_payload: None,
                headful: false,
                cleanup_lease,
            })
            .await;
        let error = match result {
            Ok(_) => panic!("injected engine launch failure must reach the platform"),
            Err(error) => error,
        };
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        ticket.wait().await;
        assert!(ticket.is_complete());
    }

    #[test]
    fn known_secret_registry_is_lane_local_not_host_global() {
        let lane_a = new_lane_known_secret_values();
        let lane_b = new_lane_known_secret_values();
        lane_a.try_insert("lane-a-secret").unwrap();
        assert_eq!(lane_a.len(), 1);
        assert!(lane_b.is_empty(), "a sibling task must not retain lane A's plaintext");
    }

    struct FakeEngine {
        act_calls: AtomicUsize,
        bring_to_front_calls: AtomicUsize,
        navigate_calls: AtomicUsize,
        observe_calls: AtomicUsize,
        next_observation_generation: AtomicU64,
        oversized_observation: AtomicBool,
        fail_with_private_error: AtomicBool,
        storage_origin: Mutex<Option<String>>,
        act_result: Mutex<Option<ActResult>>,
        rendered_html: Mutex<String>,
        navigation_result_url: Mutex<Option<String>>,
        tabs: Mutex<Vec<BrowserTabInfo>>,
    }

    impl FakeEngine {
        fn new() -> Self {
            Self::with_observation_generation(7)
        }

        fn with_observation_generation(generation: u64) -> Self {
            Self {
                act_calls: AtomicUsize::new(0),
                bring_to_front_calls: AtomicUsize::new(0),
                navigate_calls: AtomicUsize::new(0),
                observe_calls: AtomicUsize::new(0),
                next_observation_generation: AtomicU64::new(generation),
                oversized_observation: AtomicBool::new(false),
                fail_with_private_error: AtomicBool::new(false),
                storage_origin: Mutex::new(Some("https://example.test".to_owned())),
                act_result: Mutex::new(None),
                rendered_html: Mutex::new("<html></html>".to_owned()),
                navigation_result_url: Mutex::new(None),
                tabs: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl BrowserEngine for FakeEngine {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                browser_ready: true,
                headful: false,
                display_available: true,
                engine: "fake".to_string(),
            }
        }

        async fn navigate(&self, url: &str, _new_tab: bool) -> Result<NavResult, BrowserError> {
            self.navigate_calls.fetch_add(1, Ordering::SeqCst);
            Ok(NavResult {
                final_url: self
                    .navigation_result_url
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| url.to_string()),
                http_status: Some(200),
                redirected: false,
                load_state: LoadState::Load,
            })
        }

        async fn screenshot(&self) -> Result<Vec<u8>, BrowserError> {
            if self.fail_with_private_error.load(Ordering::SeqCst) {
                return Err(BrowserError::Other(
                    "ws://127.0.0.1:9222 profile=C:\\secret\\profile".to_string(),
                ));
            }
            let image = image::DynamicImage::new_rgb8(4, 3);
            let mut png = Cursor::new(Vec::new());
            image.write_to(&mut png, ImageFormat::Png).unwrap();
            Ok(png.into_inner())
        }

        async fn rendered_html(&self) -> Result<String, BrowserError> {
            Ok(self.rendered_html.lock().unwrap().clone())
        }

        async fn observe(&self, _opts: &ObserveOpts) -> Result<Observation, BrowserError> {
            self.observe_calls.fetch_add(1, Ordering::SeqCst);
            let generation = self
                .next_observation_generation
                .fetch_add(1, Ordering::SeqCst);
            let yaml = if self.oversized_observation.load(Ordering::SeqCst) {
                "x".repeat(nomi_browser_engine::observe::MAX_OBSERVATION_RETAINED_BYTES + 1)
            } else {
                "<data>Pay now</data>".to_string()
            };
            Ok(Observation {
                generation: SnapshotGen(generation),
                yaml,
                entries: vec![ElementEntry {
                    r#ref: "f0e1".to_string(),
                    role: "button".to_string(),
                    name: "Pay now".to_string(),
                    frame_seq: 0,
                }],
                url: Some("https://example.test/checkout".to_string()),
                truncated: false,
                current_page_is_post: false,
                boxes: HashMap::new(),
            })
        }

        async fn act(
            &self,
            _spec: &ActSpec,
            _progress: &nomi_browser_engine::progress::Progress,
        ) -> Result<ActResult, BrowserError> {
            self.act_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(result) = self.act_result.lock().unwrap().clone() {
                return Ok(result);
            }
            Ok(ActResult {
                message: "ok".to_string(),
                effect: Effect {
                    changed: true,
                    before_anchor: None,
                    after_anchor: None,
                },
                success: true,
            })
        }

        async fn tabs(&self) -> Result<Vec<BrowserTabInfo>, BrowserError> {
            Ok(self.tabs.lock().unwrap().clone())
        }

        async fn debug_snapshot(&self) -> Result<DebugSnapshot, BrowserError> {
            Ok(DebugSnapshot {
                console: Vec::new(),
                errors: Vec::new(),
                network: Vec::new(),
            })
        }

        async fn bring_to_front(&self) -> Result<(), BrowserError> {
            self.bring_to_front_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn capture_cookie_state(&self) -> Result<StorageState, BrowserError> {
            Ok(StorageState::default())
        }

        async fn capture_storage_state(&self) -> Result<StorageState, BrowserError> {
            let local_storage = self
                .storage_origin
                .lock()
                .unwrap()
                .clone()
                .map(|origin| {
                    let mut storage = OriginStorage::new_local_storage(
                        origin,
                        [("session".to_owned(), "active".to_owned())],
                    );
                    storage.index_db = Some(IndexedDbDump::default());
                    storage
                })
                .into_iter()
                .collect();
            Ok(StorageState {
                cookies: Vec::new(),
                local_storage,
            })
        }

        async fn click_at_css_point(&self, _x: f64, _y: f64) -> Result<(), BrowserError> {
            Ok(())
        }
    }

    fn test_driver(engine: Arc<FakeEngine>) -> ManagedEngineLaneDriver {
        let lane_id = BrowserLaneId::parse("lane-test").unwrap();
        let policy = BrowserTool::with_managed_engine(
            engine.clone(),
            std::env::temp_dir().join("nomifun-platform-adapter-test"),
            None,
            false,
            false,
            false,
            nomi_browser_engine::KnownSecretValues::default(),
        );
        ManagedEngineLaneDriver {
            lane_id,
            epoch: 42,
            engine,
            policy,
            host: Weak::new(),
            identity_mode: BrowserIdentityMode::Primary,
            identity_snapshot_persister: None,
            closing: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            close_gate: AsyncMutex::new(()),
        }
    }

    fn context() -> DriverOperationContext {
        DriverOperationContext {
            operation: OperationContext {
                browser_epoch: 42,
                lane_id: BrowserLaneId::parse("lane-test").unwrap(),
                target_id: None,
                frame_id: None,
                ref_generation: 0,
                cancellation_id: "cancel-test".to_string(),
            },
            cancellation: CancellationToken::new(),
            trusted_out_of_band_confirmation: false,
        }
    }

    fn operation(kind: BrowserOperationKind, action: &str, input: Value) -> BrowserOperation {
        BrowserOperation {
            kind,
            action: action.to_string(),
            input,
            expected_browser_epoch: None,
            target_id: None,
            frame_id: None,
            ref_generation: None,
            may_modify_identity: false,
        }
    }

    #[tokio::test]
    async fn trusted_foreground_seam_calls_engine_without_json_operation() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());

        BrowserLaneDriver::bring_to_front(&driver)
            .await
            .expect("trusted foreground seam succeeds");

        assert_eq!(engine.bring_to_front_calls.load(Ordering::SeqCst), 1);
        assert_eq!(engine.act_calls.load(Ordering::SeqCst), 0);
        assert_eq!(engine.navigate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(engine.observe_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn trusted_foreground_seam_respects_lane_close_fence() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());
        driver.closing.store(true, Ordering::Release);

        let error = BrowserLaneDriver::bring_to_front(&driver)
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::LaneClosedByUser);
        assert_eq!(engine.bring_to_front_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn model_json_cannot_request_trusted_foregrounding() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());

        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Manage,
                    "bring_to_front",
                    Value::Object(Map::new()),
                ),
                context(),
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(engine.bring_to_front_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn navigate_maps_to_structured_result() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());
        let result = driver
            .execute(
                operation(
                    BrowserOperationKind::Navigate,
                    "navigate",
                    json!({"url": "https://example.test"}),
                ),
                context(),
            )
            .await
            .unwrap();
        assert_eq!(result.output["final_url"], "https://example.test");
        assert_eq!(result.output["http_status"], 200);
        assert_eq!(engine.navigate_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn navigate_bounds_input_and_redirect_url_before_value_retention() {
        let engine = Arc::new(FakeEngine::new());
        *engine.navigation_result_url.lock().unwrap() =
            Some("界".repeat(MAX_MANAGED_NAVIGATION_URL_BYTES / 3 + 32));
        let driver = test_driver(engine.clone());
        let result = driver
            .execute(
                operation(
                    BrowserOperationKind::Crawl,
                    "navigate",
                    json!({"url": "https://example.test"}),
                ),
                context(),
            )
            .await
            .unwrap();
        let final_url = result.output["final_url"].as_str().unwrap();
        assert!(final_url.len() <= MAX_MANAGED_NAVIGATION_URL_BYTES);
        assert_eq!(result.output["final_url_truncated"], true);
        assert!(std::str::from_utf8(final_url.as_bytes()).is_ok());

        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Crawl,
                    "navigate",
                    json!({"url": "x".repeat(MAX_MANAGED_NAVIGATION_URL_BYTES + 1)}),
                ),
                context(),
            )
            .await
            .expect_err("oversized input URL must be rejected before engine dispatch");
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(engine.navigate_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn crawl_many_read_actions_reach_the_real_managed_adapter() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());

        let text = driver
            .execute(
                operation(
                    BrowserOperationKind::Crawl,
                    "get_page_text",
                    json!({}),
                ),
                context(),
            )
            .await
            .unwrap();
        assert_eq!(text.output["message"], "ok");

        let extracted = driver
            .execute(
                operation(
                    BrowserOperationKind::Crawl,
                    "extract",
                    json!({"schema": {"title": "string"}}),
                ),
                context(),
            )
            .await
            .unwrap();
        assert_eq!(extracted.output["message"], "ok");
        assert_eq!(engine.act_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rendered_html_is_utf8_bounded_again_at_the_platform_adapter() {
        let engine = Arc::new(FakeEngine::new());
        *engine.rendered_html.lock().unwrap() =
            "😀".repeat(nomi_browser_engine::MAX_RENDERED_HTML_BYTES / 4 + 64);
        let driver = test_driver(engine);
        let result = driver
            .execute(
                operation(BrowserOperationKind::Crawl, "rendered_html", json!({})),
                context(),
            )
            .await
            .expect("oversized renderer product is safely truncated");
        let html = result.output["html"].as_str().unwrap();
        assert!(html.len() <= nomi_browser_engine::MAX_RENDERED_HTML_BYTES);
        assert!(html.ends_with(nomi_browser_engine::RENDERED_HTML_TRUNCATION_MARKER));
        assert_eq!(result.output["html_truncated"], true);
        assert!(std::str::from_utf8(html.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn crawl_page_read_rejects_oversized_retained_message_before_value_copy() {
        let engine = Arc::new(FakeEngine::new());
        *engine.act_result.lock().unwrap() = Some(ActResult {
            message: "x".repeat(
                nomi_browser_engine::actions::MAX_PAGE_READ_RESULT_BYTES + 1,
            ),
            effect: Effect {
                changed: false,
                before_anchor: None,
                after_anchor: None,
            },
            success: true,
        });
        let driver = test_driver(engine);
        let error = driver
            .execute(
                operation(BrowserOperationKind::Crawl, "get_page_text", json!({})),
                context(),
            )
            .await
            .expect_err("oversized message must fail before serde_json::Value retention");
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(error.message.contains("retained-byte limit"));
    }

    #[tokio::test]
    async fn mismatched_epoch_is_rejected_before_engine_dispatch() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());
        let mut stale = context();
        stale.operation.browser_epoch = 41;
        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Navigate,
                    "navigate",
                    json!({"url": "https://example.test"}),
                ),
                stale,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::StaleBrowserEpoch);
        assert_eq!(engine.navigate_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn managed_engine_lane_refuses_a_one_way_freeze() {
        let driver = test_driver(Arc::new(FakeEngine::new()));
        assert_eq!(
            driver.freeze().await.unwrap(),
            LaneFreezeOutcome::Unsupported
        );
        assert!(!driver.closing.load(Ordering::Acquire));
        assert!(!driver.closed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn lane_close_is_idempotent_and_fences_future_adapter_work() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(Arc::clone(&engine));

        driver.close().await.unwrap();
        driver.close().await.unwrap();

        assert!(driver.closing.load(Ordering::Acquire));
        assert!(driver.closed.load(Ordering::Acquire));
        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Navigate,
                    "navigate",
                    json!({"url": "https://example.test"}),
                ),
                context(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::LaneClosedByUser);
        assert_eq!(engine.navigate_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn operation_kind_cannot_be_used_to_smuggle_another_action() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine);
        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "screenshot",
                    Value::Object(Map::new()),
                ),
                context(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
    }




    #[tokio::test]
    async fn oversized_platform_observation_returns_no_output_and_clears_cache() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());
        driver
            .policy
            .cache_managed_observation(Observation {
                generation: SnapshotGen(1),
                yaml: "<data>old</data>".into(),
                entries: vec![],
                url: Some("https://old.example.test".into()),
                truncated: false,
                current_page_is_post: false,
                boxes: HashMap::new(),
            })
            .expect("small seed observation fits");
        engine.oversized_observation.store(true, Ordering::SeqCst);

        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                context(),
            )
            .await
            .expect_err("oversized observation must not produce a platform output");

        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert!(driver.policy.last_snapshot.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn replacement_first_observe_fences_reset_engine_generation() {
        let old_engine = Arc::new(FakeEngine::with_observation_generation(7));
        let old_driver = test_driver(old_engine);
        let old = old_driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                context(),
            )
            .await
            .unwrap();
        assert_eq!(old.ref_generation, Some(7));

        // The Hub increments its canonical lane generation before rebinding
        // the replacement driver. The replacement engine itself starts at 1.
        let replacement_engine = Arc::new(FakeEngine::with_observation_generation(1));
        let replacement_driver = test_driver(replacement_engine.clone());
        let mut replacement_context = context();
        replacement_context.operation.ref_generation = 8;
        let replacement = replacement_driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                replacement_context,
            )
            .await
            .unwrap();

        assert_eq!(replacement.ref_generation, Some(8));
        assert_eq!(replacement.output["generation"], 8);
        assert_eq!(
            replacement.ref_generation,
            replacement.output["generation"].as_u64()
        );
        assert_eq!(
            replacement_engine.observe_calls.load(Ordering::SeqCst),
            8,
            "the adapter should consume reset generations until it reaches the Hub fence"
        );
    }

    #[tokio::test]
    async fn long_lived_lane_observe_is_never_bricked_by_absolute_generation() {
        // F54: the cap bounds the catch-up delta, not the absolute canonical
        // generation. A lane that legitimately accumulated far more than the
        // old 65,536 cap — while staying in sync with its engine — must still
        // observe with a single engine call.
        let engine = Arc::new(FakeEngine::with_observation_generation(70_000));
        let driver = test_driver(engine.clone());
        let mut long_lived = context();
        long_lived.operation.ref_generation = 70_000;

        let observed = driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                long_lived,
            )
            .await
            .unwrap();

        assert_eq!(observed.ref_generation, Some(70_000));
        assert_eq!(engine.observe_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn generation_catch_up_is_bounded_and_retries_make_progress() {
        // F22: a huge fence gap (host crash after a long-lived lane) must not
        // storm thousands of back-to-back observations inside one operation.
        // The bounded attempt fails retryable, and because the engine keeps the
        // consumed generations, each retry resumes closer to the fence.
        let engine = Arc::new(FakeEngine::with_observation_generation(1));
        let driver = test_driver(engine.clone());
        let mut stale = context();
        stale.operation.ref_generation = 10_000;

        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                stale.clone(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(error.retryable, "bounded catch-up must stay recoverable");
        let first_attempt = engine.observe_calls.load(Ordering::SeqCst);
        assert_eq!(
            first_attempt as u64,
            1 + MAX_OBSERVATION_GENERATION_CATCH_UP,
            "one operation performs at most 1 + cap observations"
        );

        let retry_error = driver
            .execute(
                operation(
                    BrowserOperationKind::Observe,
                    "observe",
                    Value::Object(Map::new()),
                ),
                stale,
            )
            .await
            .unwrap_err();
        assert_eq!(retry_error.code, BrowserErrorCode::BrowserUnavailable);
        let second_attempt = engine.observe_calls.load(Ordering::SeqCst) - first_attempt;
        assert_eq!(
            second_attempt as u64,
            1 + MAX_OBSERVATION_GENERATION_CATCH_UP,
            "a retry is bounded identically and resumes from the kept generations"
        );
    }

    #[tokio::test]
    async fn switch_frame_projects_the_structured_frame_cursor() {
        let engine = Arc::new(FakeEngine::new());
        *engine.act_result.lock().unwrap() = Some(ActResult {
            message: "frame switched".to_owned(),
            effect: Effect {
                changed: true,
                before_anchor: None,
                after_anchor: Some(json!({ "active_frame": "frame-child-7" })),
            },
            success: true,
        });
        *engine.tabs.lock().unwrap() = vec![BrowserTabInfo {
            tab_id: "tab-a".to_owned(),
            target_id: "target-a".to_owned(),
            title: Some("Tab A".to_owned()),
            url: Some("https://example.test".to_owned()),
            active: true,
            crashed: false,
        }];
        let driver = test_driver(engine);
        let result = driver
            .execute(
                operation(
                    BrowserOperationKind::Act,
                    "switch_frame",
                    json!({ "ref": "f0e1" }),
                ),
                context(),
            )
            .await
            .unwrap();

        assert_eq!(result.active_tab_id.as_deref(), Some("tab-a"));
        assert_eq!(result.active_frame_id.as_deref(), Some("frame-child-7"));
    }

    #[tokio::test]
    async fn switching_to_main_frame_and_another_tab_use_full_target_ids() {
        let engine = Arc::new(FakeEngine::new());
        *engine.act_result.lock().unwrap() = Some(ActResult {
            message: "switched".to_owned(),
            effect: Effect {
                changed: true,
                before_anchor: None,
                after_anchor: Some(json!({ "active_frame": "main" })),
            },
            success: true,
        });
        *engine.tabs.lock().unwrap() = vec![BrowserTabInfo {
            tab_id: "tab-main".to_owned(),
            target_id: "target-main-full".to_owned(),
            title: Some("Main".to_owned()),
            url: Some("https://example.test/main".to_owned()),
            active: true,
            crashed: false,
        }];
        let driver = test_driver(engine.clone());
        let mut frame_context = context();
        frame_context.operation.target_id = Some("target-main-full".to_owned());
        let main_frame = driver
            .execute(
                operation(
                    BrowserOperationKind::Act,
                    "switch_frame",
                    json!({ "ref": "main" }),
                ),
                frame_context,
            )
            .await
            .unwrap();
        assert_eq!(main_frame.active_tab_id.as_deref(), Some("tab-main"));
        assert_eq!(
            main_frame.active_frame_id.as_deref(),
            Some("target-main-full")
        );

        *engine.act_result.lock().unwrap() = None;
        *engine.tabs.lock().unwrap() = vec![BrowserTabInfo {
            tab_id: "tab-next".to_owned(),
            target_id: "target-next-full".to_owned(),
            title: Some("Next".to_owned()),
            url: Some("https://example.test/next".to_owned()),
            active: true,
            crashed: false,
        }];
        let switched_tab = driver
            .execute(
                operation(
                    BrowserOperationKind::Tabs,
                    "switch_tab",
                    json!({ "tab_id": "tab-next" }),
                ),
                context(),
            )
            .await
            .unwrap();
        assert_eq!(switched_tab.active_tab_id.as_deref(), Some("tab-next"));
        assert_eq!(
            switched_tab.active_frame_id.as_deref(),
            Some("target-next-full")
        );
        assert_eq!(switched_tab.ref_generation, None);
    }

    #[tokio::test]
    async fn private_engine_diagnostics_never_cross_platform_boundary() {
        let engine = Arc::new(FakeEngine::new());
        engine.fail_with_private_error.store(true, Ordering::SeqCst);
        let driver = test_driver(engine);
        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Screenshot,
                    "screenshot",
                    Value::Object(Map::new()),
                ),
                context(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::BrowserUnavailable);
        assert!(!error.message.contains("9222"));
        assert!(!error.message.contains("profile"));
    }

    #[tokio::test]
    async fn primary_capture_is_strictly_cookie_only() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(Arc::clone(&engine));

        let captured_with_origin_storage = driver
            .capture_identity_snapshot()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            captured_with_origin_storage.coverage,
            SnapshotCoverage::cookies_only()
        );
        assert_eq!(
            captured_with_origin_storage.payload.as_json()["localStorage"],
            json!([])
        );

        *engine.storage_origin.lock().unwrap() = None;
        let cookies_only = driver
            .capture_identity_snapshot()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cookies_only.coverage, SnapshotCoverage::cookies_only());
        assert_eq!(
            cookies_only.payload.as_json()["localStorage"],
            json!([])
        );
    }

    #[test]
    fn identity_vault_persister_strips_legacy_page_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.vault");
        let key = [17_u8; 32];
        let factory = ManagedEngineHostFactory::new(EngineConfig::default())
            .with_identity_vault(path.clone(), key);
        let state = StorageState {
            cookies: Vec::new(),
            local_storage: vec![OriginStorage::new_local_storage(
                "https://example.test",
                [("session".to_owned(), "active".to_owned())],
            )],
        };
        let payload = state.to_json().unwrap();

        factory
            .identity_snapshot_persister
            .as_ref()
            .expect("vault persister missing")(&payload)
            .unwrap();

        assert_eq!(
            nomi_browser_engine::load_storage_state(&path, &key),
            Some(state.into_cookie_only())
        );
    }

    #[test]
    fn identity_profiles_and_replica_payloads_are_generation_bound() {
        let template = EngineConfig {
            data_dir: PathBuf::from("C:/app/browser"),
            storage_state: Some(json!({"cookies": []})),
            evaluate_persistent_login: true,
            ..Default::default()
        };
        let root = PathBuf::from("C:/app/browser/profiles");
        let primary_a = HostLaunchRequest {
            host_id: BrowserHostId::parse("host-a").unwrap(),
            browser_epoch: 1,
            identity_mode: BrowserIdentityMode::Primary,
            identity_generation: 9,
            identity_snapshot_payload: None,
            headful: true,
            cleanup_lease: HostLaunchCleanupTicket::new().1,
        };
        let mut primary_b = primary_a.clone();
        primary_b.host_id = BrowserHostId::parse("host-b").unwrap();
        let a = derive_host_config(&template, &root, &primary_a).unwrap();
        let b = derive_host_config(&template, &root, &primary_b).unwrap();
        assert_eq!(a.user_data_dir, b.user_data_dir);
        assert!(!a.ephemeral_profile);
        assert!(a.storage_state.is_some());

        let anonymous = derive_host_config(
            &template,
            &root,
            &HostLaunchRequest {
                host_id: BrowserHostId::parse("host-anon").unwrap(),
                browser_epoch: 2,
                identity_mode: BrowserIdentityMode::Anonymous,
                identity_generation: 9,
                identity_snapshot_payload: None,
                headful: false,
                cleanup_lease: HostLaunchCleanupTicket::new().1,
            },
        )
        .unwrap();
        assert!(anonymous.ephemeral_profile);
        assert!(anonymous.storage_state.is_none());
        assert!(!anonymous.evaluate_persistent_login);
        assert_ne!(anonymous.user_data_dir, a.user_data_dir);

        let replica_payload = IdentitySnapshotPayload::from_json(json!({
            "cookies": [],
            "localStorage": [{
                "origin": "https://legacy.example",
                "localStorage": [{"name": "token", "value": "legacy"}]
            }]
        }));
        let replica = derive_host_config(
            &template,
            &root,
            &HostLaunchRequest {
                host_id: BrowserHostId::parse("host-replica").unwrap(),
                browser_epoch: 3,
                identity_mode: BrowserIdentityMode::AuthenticatedReplica,
                identity_generation: 10,
                identity_snapshot_payload: Some(replica_payload.clone()),
                headful: false,
                cleanup_lease: HostLaunchCleanupTicket::new().1,
            },
        )
        .unwrap();
        assert_eq!(
            replica.storage_state.as_ref(),
            Some(&json!({"cookies": [], "localStorage": []}))
        );
        assert_ne!(replica.storage_state, template.storage_state);
        assert!(!replica.evaluate_persistent_login);

        let missing = derive_host_config(
            &template,
            &root,
            &HostLaunchRequest {
                host_id: BrowserHostId::parse("host-missing").unwrap(),
                browser_epoch: 4,
                identity_mode: BrowserIdentityMode::AuthenticatedReplica,
                identity_generation: 11,
                identity_snapshot_payload: None,
                headful: false,
                cleanup_lease: HostLaunchCleanupTicket::new().1,
            },
        )
        .unwrap_err();
        assert_eq!(missing.code, BrowserErrorCode::NeedsPrimaryIdentity);
    }


    #[test]
    fn isolated_reliable_queue_ceiling_is_tied_to_platform_lane_admission() {
        use nomi_browser_engine::session::{
            RELIABLE_HOST_EVENT_BYTE_CAPACITY, RELIABLE_TASK_EVENT_BYTE_CAPACITY,
        };
        use nomifun_browser_platform::{MAX_TASK_OPEN_LANES, ResourcePolicy};

        // Isolated mode maps each Lane to its own HostKey. This cross-crate
        // contract prevents the engine's per-Host bound and the Hub's hard
        // per-task Lane/Host maximum from drifting apart unnoticed.
        assert_eq!(
            MAX_TASK_OPEN_LANES * RELIABLE_HOST_EVENT_BYTE_CAPACITY,
            RELIABLE_TASK_EVENT_BYTE_CAPACITY,
            "32 fully isolated Host-owned queues are capped at 128 MiB"
        );
        assert_eq!(
            ResourcePolicy::default().max_task_open_lanes
                * RELIABLE_HOST_EVENT_BYTE_CAPACITY,
            16 * 1024 * 1024,
            "the default four isolated Hosts contribute at most 16 MiB"
        );
        assert_eq!(
            MAX_TASK_OPEN_LANES * RELIABLE_HOST_EVENT_BYTE_CAPACITY
                + RELIABLE_TASK_EVENT_BYTE_CAPACITY,
            256 * 1024 * 1024,
            "conservative fixed Host plus dynamic task ceiling is 256 MiB"
        );
    }
}
