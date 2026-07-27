//! Production bridge from the Browser Platform authority to the shared CDP
//! host.
//!
//! This module is intentionally the only place which translates the platform's
//! generic JSON operation envelope into `nomi-browser-engine` calls.  The Hub
//! remains responsible for trusted caller identity, leases, scheduling and the
//! per-lane operation gate; this adapter owns engine-specific dispatch and
//! converts every engine failure into the stable, display-safe platform error
//! taxonomy.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use nomi_browser_engine::{
    ActResult, ActSpec, BrowserEngine, BrowserError, BrowserTabInfo, EngineConfig,
    LaneEngineConfig, ManagedBrowserHost, ObserveOpts, Observation,
};
use nomifun_browser_platform::{
    BrowserErrorCode, BrowserHostDriver, BrowserHostFactory, BrowserHostId,
    BrowserIdentityMode, BrowserLaneDriver, BrowserLaneId, BrowserOperation,
    BrowserOperationKind, BrowserOperationResult, BrowserPlatformError, BrowserTabSnapshot,
    CapturedIdentitySnapshot, DriverOperationContext, HostLaunchRequest, HostLifecycleState,
    IdentitySnapshotPayload, LaneFreezeOutcome, LaneLaunchRequest, SnapshotCoverage,
};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex as AsyncMutex;

use crate::BrowserTool;

const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_ACTION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OBSERVATION_GENERATION_ADVANCE: u64 = 65_536;

/// Synchronous, side-effect-free resolver used immediately before launching a
/// host.  Applications which load an authenticated replica from a vault can
/// inject the resulting `storage_state` here without teaching the platform core
/// about engine configuration.
pub type EngineConfigResolver = Arc<
    dyn Fn(&HostLaunchRequest) -> Result<EngineConfig, BrowserPlatformError> + Send + Sync,
>;

/// Trusted composition-root hook for adding the existing BrowserTool policy
/// services (secret source, approval gate, extract model, site memory, visual
/// locator) to each managed Lane. Model input never reaches this hook.
pub type ManagedLanePolicyDecorator =
    Arc<dyn Fn(BrowserTool) -> BrowserTool + Send + Sync>;

type IdentitySnapshotPersister =
    Arc<dyn Fn(&Value) -> Result<(), BrowserPlatformError> + Send + Sync>;

/// Production `BrowserHostFactory` backed by `ManagedBrowserHost`.
#[derive(Clone)]
pub struct ManagedEngineHostFactory {
    resolver: EngineConfigResolver,
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
            lane_policy: Arc::new(|policy| policy),
            identity_snapshot_persister: None,
        }
    }

    /// Decorate the fail-closed managed policy with trusted application
    /// services. For example, the desktop composition root can add a
    /// `BrowserSecretSource` and `BrowserApprovalGate` here.
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
                .map_err(|_| identity_capture_error())?;
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
        };
        let data_dir = config.data_dir.clone();
        let host = Arc::new(
            ManagedBrowserHost::launch(config)
                .await
                .map_err(map_engine_error)?,
        );
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
            headful,
            lane_policy: self.lane_policy.clone(),
            identity_snapshot_persister: self.identity_snapshot_persister.clone(),
            state: AtomicU8::new(HostState::Running as u8),
            shutdown_gate: AsyncMutex::new(()),
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
    headful: bool,
    lane_policy: ManagedLanePolicyDecorator,
    identity_snapshot_persister: Option<IdentitySnapshotPersister>,
    state: AtomicU8,
    shutdown_gate: AsyncMutex<()>,
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

        let known_secret_values = self
            .defaults
            .known_secret_values
            .clone()
            .unwrap_or_else(|| Arc::new(std::sync::Mutex::new(HashSet::new())));
        let config = LaneEngineConfig {
            workspace_dir: request
                .workspace_hint
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| self.defaults.workspace_dir.clone()),
            evaluate_full_power: self.defaults.evaluate_full_power,
            evaluate_persistent_login: self.defaults.evaluate_persistent_login,
            known_secret_values: Some(known_secret_values.clone()),
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
                Ok(BrowserOperationResult {
                    output: json!({
                        "final_url": nav.final_url,
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
                let output = serialize_observation(&observation);
                self.policy.cache_managed_observation(observation);
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
                Ok(BrowserOperationResult {
                    output: json!({ "html": html }),
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
        let required = context.operation.ref_generation.max(1);
        if required > MAX_OBSERVATION_GENERATION_ADVANCE {
            return Err(observation_generation_exhausted());
        }

        for _ in 0..required {
            let observation = self
                .engine
                .observe(options)
                .await
                .map_err(map_engine_error)?;
            let generation = observation.generation.0;
            if generation >= required {
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
            .prepare_managed_act(
                action,
                input,
                context.trusted_out_of_band_confirmation,
            )
            .await
            .map_err(|_| {
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
            .capture_storage_state()
            .await
            .map_err(map_engine_error)?;
        // Keep the complete captured payload inside the trusted identity
        // boundary, but only advertise state that the current Replica startup
        // path can prove is restored before the first page script runs.
        // Cookies satisfy that contract today; localStorage and IndexedDB do
        // not, even when capture happened to include them.
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
            config.storage_state = Some(payload.as_json().clone());
            config.evaluate_persistent_login = false;
        }
        BrowserIdentityMode::Anonymous | BrowserIdentityMode::Isolated => {
            config.storage_state = None;
            config.evaluate_persistent_login = false;
            config.known_secret_values = Arc::new(std::sync::Mutex::new(HashSet::new()));
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

fn serialize_observation(observation: &Observation) -> Value {
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
    json!({
        "generation": observation.generation.0,
        "yaml": observation.yaml,
        "entries": entries,
        "url": observation.url,
        "truncated": observation.truncated,
        "current_page_is_post": observation.current_page_is_post,
        "boxes": boxes,
    })
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
    use crate::OUT_OF_BAND_CONFIRMED_KEY;

    struct FakeEngine {
        act_calls: AtomicUsize,
        bring_to_front_calls: AtomicUsize,
        navigate_calls: AtomicUsize,
        observe_calls: AtomicUsize,
        next_observation_generation: AtomicU64,
        fail_with_private_error: AtomicBool,
        storage_origin: Mutex<Option<String>>,
        act_result: Mutex<Option<ActResult>>,
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
                fail_with_private_error: AtomicBool::new(false),
                storage_origin: Mutex::new(Some("https://example.test".to_owned())),
                act_result: Mutex::new(None),
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
                final_url: url.to_string(),
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
            Ok("<html></html>".to_string())
        }

        async fn observe(&self, _opts: &ObserveOpts) -> Result<Observation, BrowserError> {
            self.observe_calls.fetch_add(1, Ordering::SeqCst);
            let generation = self
                .next_observation_generation
                .fetch_add(1, Ordering::SeqCst);
            Ok(Observation {
                generation: SnapshotGen(generation),
                yaml: "<data>Pay now</data>".to_string(),
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
            Arc::new(std::sync::Mutex::new(HashSet::new())),
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
    async fn observation_generation_is_preserved_and_dangerous_click_fails_closed() {
        let engine = Arc::new(FakeEngine::new());
        let driver = test_driver(engine.clone());
        let observed = driver
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
        assert_eq!(observed.ref_generation, Some(7));
        assert_eq!(observed.output["entries"][0]["name"], "Pay now");

        let error = driver
            .execute(
                operation(
                    BrowserOperationKind::Act,
                    "click",
                    json!({"ref": "f0e1", OUT_OF_BAND_CONFIRMED_KEY: true}),
                ),
                context(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(engine.act_calls.load(Ordering::SeqCst), 0);
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
    async fn primary_capture_only_claims_provable_cookies_coverage() {
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
            captured_with_origin_storage.payload.as_json()["localStorage"][0]["origin"],
            "https://example.test"
        );
        assert_eq!(
            captured_with_origin_storage.payload.as_json()["localStorage"][0]["indexDb"],
            json!({"databases": []})
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
    fn identity_vault_persister_round_trips_the_exact_payload() {
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
            Some(state)
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
            },
        )
        .unwrap();
        assert!(anonymous.ephemeral_profile);
        assert!(anonymous.storage_state.is_none());
        assert!(!anonymous.evaluate_persistent_login);
        assert_ne!(anonymous.user_data_dir, a.user_data_dir);

        let replica_payload =
            IdentitySnapshotPayload::from_json(json!({"cookies": [{"name": "fresh"}]}));
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
            },
        )
        .unwrap();
        assert_eq!(
            replica.storage_state.as_ref(),
            Some(replica_payload.as_json())
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
            },
        )
        .unwrap_err();
        assert_eq!(missing.code, BrowserErrorCode::NeedsPrimaryIdentity);
    }
}
